//! 荐股链反思优化闭环 —— 样本层（Phase A，见 `PLAN-reco-reflection-closure.md`）
//!
//! ## 为什么荐股链要有自己的样本通路
//!
//! 既有「胜率线」（evolution_drift B1）的输入 `strategy_performance` 只有分析链写入：
//! `backtest_analysis` 用 `map_action_to_strategy_id` 把**决策 action** 映射成与荐股
//! Style 同名但语义无关的 strategy_id，再被 B1 覆盖进 `reco_strategy_weights` ——
//! 荐股消费到的权重实际是「分析链买卖观望的胜率」（归属错位 D1）。本层从荐股链
//! **自己**的实现结果 `decision_validations`（逐 pick 带真 style×period）构造样本，
//! 供 Phase B 的逐格权重合成消费；不落 `strategy_performance`，避免与分析链伪风格行同表混码。
//!
//! ## 口径（全部复用唯一判定源，禁区 12）
//!
//! - 胜负判定：[`crate::hit_rate_backtest::hit_outcome_to_binary_outcome`]
//!   （hit/partial ⇒ win；miss/false_hit ⇒ loss；insufficient/None ⇒ 剔除）——
//!   与命中率统计分母同口径，不另写一份「partial 算不算对」。
//! - 风格名目：[`crate::recommender::style_matrix`]（serenity ⇔ bottleneck 等别名经
//!   `db_style_aliases` 合并，一名两写不得各算一套）。
//! - 档位：`Period::from_str` 同时读 `ultra_short` 与历史存档 `ultrashort`。
//! - **一 pick 一样本**：同一 pick 在多个 T+N 窗口（默认 5/20/60）各有一行验证时，
//!   只取 `t_plus_n` 最接近该档 `default_holding_days` 的一行（Q5 按档对齐；并列取更短窗，
//!   保守）。否则同一决策会被重复计入胜率分母，且实现收益口径混档。

use std::collections::HashMap;

use sea_orm::{DatabaseConnection, EntityTrait, Set};
use serde::Serialize;

use axagent_entities::decision_validations;
use axagent_entities::workflow_template;
use axagent_harness::workflow_types::Variable;

use crate::hit_rate_backtest::hit_outcome_to_binary_outcome;
use crate::recommender::style_matrix::{db_style_aliases, style_keys};
use crate::recommender::types::Period;
use crate::weight_decay::StrategyPerformanceRow;

/// 一条荐股链自己的已验证决策（胜率行 + IC 配对的统一样本）。
#[derive(Debug, Clone, PartialEq)]
pub struct RecoLoopSample {
    /// 矩阵名目（`serenity`，不是落库写法 `bottleneck`）——逐格权重的行键
    pub style_key: &'static str,
    pub period: Period,
    /// 验证时刻（ms）——按档 lookback 的过滤轴
    pub exit_at_ms: i64,
    /// 该荐股**产出**时刻（ms）——熔断降权线的归因轴（不是验证时刻）
    pub predicted_at_ms: i64,
    /// 胜负（hit/partial ⇒ true）
    pub win: bool,
    /// 预测时的置信度 0-100（IC 的 x 轴）
    pub confidence: f64,
    /// 实现收益（%），按所选验证窗口（IC 的 y 轴）
    pub realized_return_pct: f64,
    /// 实际使用的验证窗口（天），半衰期拟合横轴
    pub holding_days: u32,
    /// 该样本**产出时刻**（`generated_at`）数据质量是否处于跨轮熔断态。
    ///
    /// 三态（`None` / `Some(false)` / `Some(true)`）而不是布尔：`None` 说的是
    /// 「观测面还没给出过熔断证据」（表为空，或这条样本比**首个熔断段**还早），
    /// 压成 `false` 就等于让「设施没跑过」给策略发一张清白证明。
    /// 由 [`load_reco_loop_samples`] 用 dao 的熔断时间线标注；纯函数入口默认 `None`。
    pub dqi_fused_at_prediction: Option<bool>,
}

/// 逐格闭环态（Phase B 计算、Phase D 呈现共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCellStatus {
    /// 样本不足（n < IC_MIN_SAMPLE）⇒ 基线 1.0，显式「未校准」
    InsufficientSamples,
    /// IC 不可测（置信一侧方差退化）⇒ 基线 1.0，显式「未校准」
    IcUnmeasurable,
    /// 按设计不出票（矩阵不成立格）⇒ 不参与闭环
    NotInMatrix,
    /// IC 非负 ⇒ 仅胜率线降权通路（无提权）
    IcNonNegative,
    /// IC < 0 ⇒ 胜率权重再乘负 IC 惩罚（只降不升）
    DemotedNegativeIc,
}

impl LoopCellStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LoopCellStatus::InsufficientSamples => "insufficient_samples",
            LoopCellStatus::IcUnmeasurable => "ic_unmeasurable",
            LoopCellStatus::NotInMatrix => "not_in_matrix",
            LoopCellStatus::IcNonNegative => "ic_non_negative",
            LoopCellStatus::DemotedNegativeIc => "demoted_negative_ic",
        }
    }
}

/// 把 decision_validations 的 Model 行归一化成闭环样本（纯函数，供单测直接喂构造行）。
///
/// 丢弃规则（每条都会在此注释处对应一个测试）：
/// - 风格不在矩阵名目（含别名）⇒ 丢（策略链之外的写入方不进闭环）
/// - period 解析失败 ⇒ 丢
/// - hit_outcome 无 win/loss 判定 ⇒ 丢
/// - 同一 pick_id 多窗口 ⇒ 只留最贴近该档持有天数的一行
pub fn samples_from_validation_rows(rows: Vec<decision_validations::Model>) -> Vec<RecoLoopSample> {
    // (pick_id, 候选样本, 窗口与该档默认天数的距离) —— 同 pick 取距离最小、并列取短窗
    let mut best: std::collections::HashMap<String, (RecoLoopSample, i64)> =
        std::collections::HashMap::new();
    let keys = style_keys();

    for r in rows {
        let Some(style_key) =
            keys.iter().copied().find(|k| db_style_aliases(k).contains(&r.style.as_str()))
        else {
            continue;
        };
        let Ok(period) = r.period.parse::<Period>() else { continue };
        let Some(binary) = hit_outcome_to_binary_outcome(r.hit_outcome.as_deref()) else {
            continue;
        };
        let Some(after) = r.t_plus_n_price else { continue };
        if !after.is_finite() || !r.entry_price.is_finite() || r.entry_price <= 0.0 {
            continue;
        }
        let Some(exit_at_ms) = parse_ms(&r.validated_at).or_else(|| parse_ms(&r.created_at)) else {
            continue;
        };
        let t_plus_n = r.t_plus_n.max(0) as u32;
        // 产出时刻：荐股**生成**那一刻（不是验证时刻）—— 熔断降权问的是
        // 「这条样本出生时证据面好不好」，用 exit 时刻会把「持有期内才坏」算到它头上。
        let predicted_at_ms = parse_ms(&r.generated_at).unwrap_or(exit_at_ms);
        let sample = RecoLoopSample {
            style_key,
            period,
            exit_at_ms,
            win: binary == "win",
            confidence: r.confidence as f64,
            realized_return_pct: (after / r.entry_price - 1.0) * 100.0,
            holding_days: t_plus_n,
            predicted_at_ms,
            // 标注在 `load_reco_loop_samples` 里按时间线做（本纯函数不碰库）
            dqi_fused_at_prediction: None,
        };
        let dist = (t_plus_n as i64 - period.default_holding_days() as i64).abs();
        match best.get(&r.pick_id) {
            Some((_, d)) if *d <= dist => {},
            _ => {
                best.insert(r.pick_id.clone(), (sample, dist));
            },
        }
    }
    best.into_values().map(|(s, _)| s).collect()
}

impl RecoLoopSample {
    /// 喂给 `weight_decay::compute_adjusted_weights` 的胜率行形态。
    pub fn to_performance_row(&self) -> StrategyPerformanceRow {
        StrategyPerformanceRow {
            strategy_id: self.style_key.to_string(),
            period: self.period.as_str().to_string(),
            was_correct: i32::from(self.win),
            exit_at: self.exit_at_ms,
        }
    }

    /// 喂给 `ic::aggregate_reco_ic` 的 IC 配对行形态（holding_days 即所选验证窗口）。
    pub fn to_ic_row(&self) -> crate::recommender::ic::RecoIcRow {
        crate::recommender::ic::RecoIcRow {
            style: self.style_key.to_string(),
            period: self.period.as_str().to_string(),
            confidence: self.confidence,
            realized_return_pct: self.realized_return_pct,
            holding_days: self.holding_days,
        }
    }
}

/// rfc3339 与「YYYY-MM-DDTHH:MM:SS.fff」（reco_picks.generated_at 形态）两种时间写法都读。
fn parse_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|dt| dt.and_utc().timestamp_millis())
}

/// 从库中读取全部已验证的荐股样本（薄封装；归一化与判定全在纯函数里）。
pub async fn load_reco_loop_samples(
    db: &DatabaseConnection,
) -> Result<Vec<RecoLoopSample>, String> {
    let rows = decision_validations::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 decision_validations 失败: {e}"))?;
    let mut samples = samples_from_validation_rows(rows);
    // #8 P5 第三生效面：把每条样本按**产出时刻**标成「当时是否处于熔断态」。
    // 判据（什么算异常、几轮熔断、去重口径）全在 dao，本处只取时间线做归因。
    // 取数失败 ⇒ 时间线为空 ⇒ 全部 `None` ⇒ 降权线不参与（不是参与并按正常处理）。
    let intervals = match axagent_dao::repo::data_quality_fuse::load_fused_intervals(db).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[reco_loop] 熔断时间线取数失败 ⇒ 本轮回闭不做数据质量降权: {e}");
            Vec::new()
        },
    };
    for s in samples.iter_mut() {
        s.dqi_fused_at_prediction =
            axagent_dao::repo::data_quality_fuse::fused_at(&intervals, s.predicted_at_ms);
    }
    Ok(samples)
}

// ── Phase B：胜率 × IC 合成逐格权重（PLAN-reco-reflection-closure）──

/// 闭环留痕的 trigger 标记（`strategy_weight_history.trigger`）。
/// 与分析链演化的 `"cron"|"manual"|"rule"` 分键空间消费（Q3 归属分离）。
pub const RECO_LOOP_TRIGGER: &str = "reco-loop";

/// 基线权重：闭环未校准格的取值，也是「只降不升」（Q1）的上限。
pub const BASELINE_WEIGHT: f64 = 1.0;

/// 负 IC 格的额外惩罚乘数（rank IC < 0 ⇒ 该格预测力为反向，胜率权重再乘此系数）。
/// 之所以是常数而不是可调变量：只降不升 + shadow 起步下它只影响「降多少」，
/// 转正判据（Phase E A/B）看的是相对次序而不是该系数。
pub const NEGATIVE_IC_PENALTY: f64 = 0.5;

/// 权重下限（与 `weight_decay` 的 clamp 下限同值，不另造常量）。
const WEIGHT_FLOOR: f64 = 0.05;

/// 一个 (风格, 档位) 格的闭环计算结果（Phase D 呈现层的行来源）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopCellResult {
    /// 矩阵名目（`serenity`，非落库写法）
    pub style: &'static str,
    pub period: &'static str,
    pub status: LoopCellStatus,
    pub rank_ic: Option<f64>,
    /// 该档验证窗口 lookback 内的成对样本数
    pub samples: usize,
    pub win_rate: Option<f64>,
    pub old_weight: f64,
    pub new_weight: f64,
    /// mover 漏检线（Phase 4）的降权乘数（`None` = 该格证据不足，未参与合成）。
    /// 只降不升：取值域 `(WEIGHT_FLOOR, 1]`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mover_penalty: Option<f64>,
    /// 数据质量熔断线（#8 P5）的降权乘数（`None` = 本格窗口内可判定样本不足，未参与合成）。
    /// 与 `mover_penalty` 同一条纪律：只降不升、永不提权，取值域 `(WEIGHT_FLOOR, 1]`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dqi_penalty: Option<f64>,
    /// 该格在核查窗口内「算出了它却被 top-N 淘汰」的事件数（留痕缺失时恒 0）
    pub scored_out: usize,
}

/// 逐格 mover 证据（Phase 4）：该格在核查窗口里「看到了却没要」与「要了」的事件数。
///
/// 口径与生成侧：[[`crate::mover_recall::load_trim_evidence`]] 给「看到了却没要」
/// （`reco_scan_audit` 有行 = 策略算出过却排在组内 top-N 之外），
/// `reco_picks` 给「要了」。**无留痕 ≠ 没淘汰**，因此 `scored_out=0` 时本线不参与合成。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MoverCellSignal {
    /// (矩阵名目, 档位键) → (scored_out, hits)
    pub cells: HashMap<(String, String), (usize, usize)>,
}

/// 逐个参数的推导（不写死「每漏一次降 5%」这类无出处常数）：
/// 惩罚 = Laplace 平滑后的**转化率** `(hits+1)/(hits+scored_out+1)`，
/// 即「该格看到的达标机会里，要下来而不是推出去的比例」——
/// 全要中 ⇒ 1.0（不降）；一半推出去 ⇒ ≈0.5。样本不足（见 `MOVER_MIN_EVIDENCE`）
/// 或没有留痕证据 ⇒ `None`（不动权重，宁可不动也不猜）。
pub const MOVER_MIN_EVIDENCE: usize = 3;

impl MoverCellSignal {
    /// 该格的降权乘数；`None` = 证据不足，保持原权重。
    pub fn penalty(&self, style_key: &str, period: &str) -> Option<f64> {
        let (scored_out, hits) =
            self.cells.get(&(style_key.to_string(), period.to_string())).copied().unwrap_or((0, 0));
        // 没有「看到了却没要」的证据 ⇒ 本线无从判断（不是「表现完美」）
        if scored_out == 0 || scored_out + hits < MOVER_MIN_EVIDENCE {
            return None;
        }
        let rate = (hits as f64 + 1.0) / ((hits + scored_out) as f64 + 1.0);
        Some(rate.clamp(WEIGHT_FLOOR, 1.0))
    }
}

/// 熔断降权线（#8 P5）参与合成所需的最少「可判定」样本数。
///
/// 取 3 的出处就是 `MOVER_MIN_EVIDENCE` 那条同一纪律：少于三次观测就动手，等于用一次
/// 抖动给整格定调。两线各自独立成常量而不是共用 —— 它们判的是不同的证据源，
/// 将来其中一条要改门槛，不应连带改到另一条。
pub const DQI_MIN_SAMPLES: usize = 3;

/// 逐格熔断降权乘数（纯函数，输入是已经过 lookback 过滤的本格样本）。
///
/// 分母只数**可判定**样本（`Some(_)`）：`None` 既不进分子也不进分母 ——
/// 观测面覆盖不到的样本不能白送一个「清白」，也不能算成「有罪」。
/// 推导与 mover 线同一条（Laplace 平滑转化率）：`(可判定且未熔断 + 1) / (可判定 + 1)`，
/// 全清白 ⇒ 1.0（不降）；全在熔断期产出 ⇒ 1/(n+1)（越多样本越确定该降）。
fn dqi_penalty_of(cell: &[&RecoLoopSample]) -> Option<f64> {
    let mut fused = 0usize;
    let mut clear = 0usize;
    for s in cell {
        match s.dqi_fused_at_prediction {
            Some(true) => fused += 1,
            Some(false) => clear += 1,
            None => {},
        }
    }
    if fused + clear < DQI_MIN_SAMPLES {
        return None;
    }
    Some((((clear + 1) as f64) / ((clear + fused + 1) as f64)).clamp(WEIGHT_FLOOR, 1.0))
}

/// 逐格合成（纯函数）：胜率线（`weight_decay` 基元）× 力度线（`ic` 基元）
/// × mover 线（`reco_scan_audit` 截断留痕，Phase 4）× 熔断线（`data_quality_observations`，#8 P5）。
///
/// 规则（对齐已批方案 Q1/P2/Q5）：
/// - 矩阵不成立格 ⇒ `NotInMatrix`，不产权重；
/// - 该档窗口（`default_holding_days`，Q5 按档对齐）内样本 < `IC_MIN_SAMPLE` ⇒ 基线 1.0（`InsufficientSamples`）；
/// - IC 可测但一侧方差退化 ⇒ 基线 1.0（`IcUnmeasurable`），不在测不出力度的格上做校准；
/// - IC ≥ 0 ⇒ 只吃胜率降权（`min(aw, 1.0)`，上限即基线 ⇒ 只降不升）；
/// - IC < 0 ⇒ 胜率权重再乘 `NEGATIVE_IC_PENALTY`，下限 `WEIGHT_FLOOR`；
/// - 最后（无论 IC 线是否校准）叠乘 `mover` 线：只降不升，且**永不提权**
///   （`PLAN-mover-recall-attribution.md` §Phase 4 明文禁止用 mover 数据提权）。
/// - 再叠乘 `熔断` 线（#8 P5）：本格窗口里「出生时数据质量已熔断」的样本占比 ⇒ 同一条只降不升。
///   它问的是**证据的可信度**而不是策略的预测力：胜率 80% 却全部产自坏数据期，
///   这个 80% 本身不可用；不动它就等于让"取数一直在坏"这件事对权重零成本。
pub fn compute_loop_cell_weights(
    samples: &[RecoLoopSample],
    current: &HashMap<(String, String), f64>,
    now_ms: i64,
    mover: &MoverCellSignal,
) -> Vec<LoopCellResult> {
    let mut out = Vec::new();
    for style_key in style_keys() {
        for period in Period::ALL.iter() {
            let mover_penalty = mover.penalty(style_key, period.as_str());
            let scored_out = mover
                .cells
                .get(&(style_key.to_string(), period.as_str().to_string()))
                .map_or(0, |(out, _)| *out);
            let reason = crate::recommender::style_matrix::reason_code_by_key(style_key, *period);
            if reason != "cell_is_active" {
                out.push(LoopCellResult {
                    style: style_key,
                    period: period.as_str(),
                    status: LoopCellStatus::NotInMatrix,
                    rank_ic: None,
                    samples: 0,
                    win_rate: None,
                    old_weight: BASELINE_WEIGHT,
                    new_weight: BASELINE_WEIGHT,
                    // 按设计不出票的格不适用 mover 线（也就不该出现在降权清单里）
                    mover_penalty: None,
                    // 同理不适用熔断线
                    dqi_penalty: None,
                    scored_out: 0,
                });
                continue;
            }
            let days = period.default_holding_days();
            let cutoff = now_ms - (days as i64) * 86_400_000;
            let cell: Vec<&RecoLoopSample> = samples
                .iter()
                .filter(|s| {
                    s.style_key == style_key && s.period == *period && s.exit_at_ms >= cutoff
                })
                .collect();
            let key = (style_key.to_string(), period.as_str().to_string());
            let old_weight = current.get(&key).copied().unwrap_or(BASELINE_WEIGHT);
            // 两条只降不升的外乘线合成一个乘数（都缺席 ⇒ 1.0 = 不动）。
            //   刻意不在这里比较阈值：判据分别是 `MOVER_MIN_EVIDENCE` 与 `DQI_MIN_SAMPLES`，
            //   各自的 `None` 已经表达「证据不足」。
            let dqi_penalty = dqi_penalty_of(&cell);
            let outer_penalty = mover_penalty.unwrap_or(1.0) * dqi_penalty.unwrap_or(1.0);

            if cell.len() < crate::reflection_stats::IC_MIN_SAMPLE {
                out.push(LoopCellResult {
                    style: style_key,
                    period: period.as_str(),
                    status: LoopCellStatus::InsufficientSamples,
                    rank_ic: None,
                    samples: cell.len(),
                    win_rate: None,
                    old_weight,
                    new_weight: (BASELINE_WEIGHT * outer_penalty)
                        .clamp(WEIGHT_FLOOR, BASELINE_WEIGHT),
                    mover_penalty,
                    dqi_penalty,
                    scored_out,
                });
                continue;
            }

            let perf_rows: Vec<StrategyPerformanceRow> =
                cell.iter().map(|s| s.to_performance_row()).collect();
            let cfg = crate::weight_decay::WeightDecayConfig {
                lookback_days: days,
                ..Default::default()
            };
            let mut cur1 = HashMap::new();
            cur1.insert(key.clone(), old_weight);
            let aw =
                crate::weight_decay::compute_adjusted_weights_at(&perf_rows, &cfg, &cur1, now_ms)
                    .remove(&key);

            let ic_rows: Vec<crate::recommender::ic::RecoIcRow> =
                cell.iter().map(|s| s.to_ic_row()).collect();
            let stats = crate::recommender::ic::aggregate_reco_ic(&ic_rows);
            let cell_ic = stats
                .styles
                .iter()
                .flat_map(|st| st.cells.iter())
                .find(|c| c.period == period.as_str());
            let (ic_status, rank_ic) = match cell_ic {
                Some(c) => (c.ic_status, c.rank_ic),
                None => ("insufficient_ic_samples", None),
            };

            let win = aw.map(|a| (a.new_weight.min(BASELINE_WEIGHT), a.win_rate));
            let (win_w, win_rate) = match win {
                Some((w, wr)) => (Some(w), Some(wr)),
                None => (None, None),
            };
            let (status, new_weight) = if ic_status != "ok" {
                // 样本门闸三态（P2）：测不出力度就不校准
                (LoopCellStatus::IcUnmeasurable, BASELINE_WEIGHT)
            } else if rank_ic.unwrap_or(0.0) < 0.0 {
                (
                    LoopCellStatus::DemotedNegativeIc,
                    win_w.unwrap_or(BASELINE_WEIGHT) * NEGATIVE_IC_PENALTY,
                )
            } else {
                (LoopCellStatus::IcNonNegative, win_w.unwrap_or(BASELINE_WEIGHT))
            };
            // mover 线与熔断线都在 IC/胜率线之后叠乘：只降不升，任一条样本不足时不动它自己那一份
            let blended = new_weight * outer_penalty;
            out.push(LoopCellResult {
                style: style_key,
                period: period.as_str(),
                status,
                rank_ic,
                samples: cell.len(),
                win_rate,
                old_weight,
                new_weight: blended.clamp(WEIGHT_FLOOR, BASELINE_WEIGHT),
                mover_penalty,
                dqi_penalty,
                scored_out,
            });
        }
    }
    out
}

/// 闭环整体视图（随 `reco_ic_stats` 一起出，Phase D 呈现层的表头来源）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoLoopView {
    /// 生效闸（`reco_ic_gate`）：`off` | `shadow` | `on`
    pub gate: String,
    pub cells: Vec<LoopCellResult>,
    /// 最近一次闭环重算时刻（ms）；0 = 从未重算
    pub last_recalc_at: i64,
}

impl Default for RecoLoopView {
    fn default() -> Self {
        Self { gate: "shadow".to_string(), cells: Vec::new(), last_recalc_at: 0 }
    }
}

/// 重算荐股闭环权重并留痕（写入键空间 = 矩阵名目，trigger = `reco-loop`）。
///
/// `as_of_date` 为时间旅行右端：样本按 `exit_at_ms <= as_of` 截断、窗口以 as_of 为
/// 「今天」，用于 Phase E 的 A/B 回放；写库的 `applied_at` 仍是真实时刻（与分析链
/// `recalc_and_persist` 同形态）。极小抖动（|Δ| < 1%）不写库，与既有权重线一致。
pub async fn recalc_and_persist_reco_loop(
    db: &DatabaseConnection,
    as_of_date: Option<&str>,
    mover: &MoverCellSignal,
) -> Result<(usize, Vec<LoopCellResult>), String> {
    use axagent_entities::strategy_weight_history;

    let now_ms = match as_of_date {
        Some(d) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map_err(|e| format!("as_of_date 格式错误: {e}"))?
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "无效日期".to_string())?
            .and_utc()
            .timestamp_millis(),
        None => chrono::Utc::now().timestamp_millis(),
    };

    let mut samples = load_reco_loop_samples(db).await?;
    samples.retain(|s| s.exit_at_ms <= now_ms);
    let current =
        crate::evolution_drift::load_current_weights_by_trigger(db, Some(RECO_LOOP_TRIGGER))
            .await?;
    let results = compute_loop_cell_weights(&samples, &current, now_ms, mover);

    let applied_at = chrono::Utc::now().timestamp_millis();
    let mut written = 0usize;
    for r in results.iter().filter(|r| r.status != LoopCellStatus::NotInMatrix) {
        let delta_pct = if r.old_weight.abs() > f64::EPSILON {
            (r.new_weight - r.old_weight) / r.old_weight * 100.0
        } else {
            0.0
        };
        if delta_pct.abs() < 1.0 {
            continue;
        }
        let rationale = match r.status {
            LoopCellStatus::DemotedNegativeIc => format!(
                "闭环降权: 窗口内 {} 样本, 负 rankIC {:.2}, 胜率 {:.0}%, 惩罚×{} ⇒ 权重 {:.2}→{:.2}",
                r.samples,
                r.rank_ic.unwrap_or(0.0),
                r.win_rate.unwrap_or(0.0) * 100.0,
                NEGATIVE_IC_PENALTY,
                r.old_weight,
                r.new_weight
            ),
            LoopCellStatus::IcNonNegative => format!(
                "闭环校准: 窗口内 {} 样本, rankIC {:.2}≥0, 胜率 {:.0}% ⇒ 权重 {:.2}→{:.2}",
                r.samples,
                r.rank_ic.unwrap_or(0.0),
                r.win_rate.unwrap_or(0.0) * 100.0,
                r.old_weight,
                r.new_weight
            ),
            _ => format!(
                "闭环未校准({}): 窗口内 {} 样本, 回退基线 {:.2}",
                r.status.as_str(),
                r.samples,
                r.new_weight
            ),
        };
        // 留痕必须能反解：胜率/IC 之外还有两条外乘线（mover 漏检线、数据质量熔断线），
        //   不进句子的话「窗口样本一样、权重却更低」在审计时无从归因。缺席的线不写，
        //   写了的就代表它真的参与了本轮合成。
        let mut outer_lines: Vec<String> = Vec::new();
        if let Some(p) = r.mover_penalty {
            outer_lines.push(format!("漏检线×{p:.2}"));
        }
        if let Some(p) = r.dqi_penalty {
            outer_lines.push(format!("熔断线×{p:.2}"));
        }
        let rationale = if outer_lines.is_empty() {
            rationale
        } else {
            format!("{rationale}; {}", outer_lines.join("; "))
        };
        let am = strategy_weight_history::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            strategy_id: Set(r.style.to_string()),
            period: Set(r.period.to_string()),
            old_weight: Set(r.old_weight),
            new_weight: Set(r.new_weight),
            delta_pct: Set(delta_pct),
            trigger: Set(RECO_LOOP_TRIGGER.to_string()),
            source_reflection_id: Set(None),
            sample_size: Set(r.samples as i32),
            win_rate: Set(r.win_rate.unwrap_or(0.0)),
            rationale: Set(Some(rationale)),
            applied_at: Set(applied_at),
        };
        strategy_weight_history::Entity::insert(am)
            .exec(db)
            .await
            .map_err(|e| format!("写入闭环权重留痕失败: {e}"))?;
        written += 1;
    }
    Ok((written, results))
}

/// 从 `workflow_template.variables` JSON 解析出 `(name, value)` 对。
///
/// `variables` 存的是 `Vec<Variable>` 序列；解析失败 ⇒ 空（调用方按缺省闸处理，
/// 不把"模板损坏"升级成"参数缺失"的静默行为）。
pub fn extract_template_vars(t: &workflow_template::Model) -> Vec<(String, serde_json::Value)> {
    let raw = match t.variables.as_ref() {
        Some(s) => s,
        None => return Vec::new(),
    };
    match serde_json::from_str::<Vec<Variable>>(raw) {
        Ok(vs) => vs.into_iter().map(|v| (v.name, v.value)).collect(),
        Err(_) => Vec::new(),
    }
}

/// 读取荐股实际消费的模板变量（含闭环权重覆盖）。
///
/// 单档 `recommend_stocks`、多档 `recommend_stocks_all_periods`、cron 定时扫描与
/// 漏检核查四条入口**必须**走同一个函数（缺陷 D2 的教训：曾有入口读裸模板变量，
/// 闭环权重只对手动刷新生效，多条链算多套分）。cron/核查拿不到 `State`，故实现抽在
/// db 层。本函数 2026-09-30 从 `commands/stock_analysis.rs` 搬入：放命令层会导致
/// 跨模块调命令（分层门禁规则 2 `commands-no-sibling-call`），放这里四处都只是调库。
///
/// 生效闸（Q2 shadow 起步）：`reco_ic_gate`（模板可调变量，缺省 `shadow`）——
/// - `off`／`shadow` ⇒ 不覆盖（闭环照算照留痕，只是不进评分）；
/// - `on` ⇒ 用 reco-loop 权重覆盖模板静态值。
///   变量读不到按 `shadow`（新装/模板缺失时保持现状，不假装闭环已转正）。
pub async fn load_reco_served_vars_db(
    db: &DatabaseConnection,
) -> Result<Vec<(String, serde_json::Value)>, String> {
    // 读取 workflow template 变量用于 vendor 启用检测
    let template = workflow_template::Entity::find_by_id("stock-analysis")
        .one(db)
        .await
        .map_err(|e| format!("查询模板失败: {e}"))?;

    let vars: Vec<(String, serde_json::Value)> = match template {
        Some(t) => extract_template_vars(&t),
        None => Vec::new(),
    };

    let gate = vars
        .iter()
        .find(|(k, _)| k == "reco_ic_gate")
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("shadow");
    if gate != "on" {
        return Ok(vars);
    }

    let looped =
        crate::evolution_drift::load_current_weights_by_trigger(db, Some(RECO_LOOP_TRIGGER))
            .await?;
    if looped.is_empty() {
        return Ok(vars);
    }
    // load_current_weights_by_trigger 返回 ((strategy, period), weight)，
    // 需换算成 parse_strategy_weights 约定的 "{style}_{period}" key 形态。
    let mut obj = serde_json::Map::new();
    for ((s, p), w) in looped {
        obj.insert(format!("{s}_{p}"), serde_json::json!(w));
    }
    let mut v = vars.into_iter().filter(|(k, _)| k != "reco_strategy_weights").collect::<Vec<_>>();
    v.push(("reco_strategy_weights".to_string(), serde_json::Value::Object(obj)));
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflection_stats::IC_MIN_SAMPLE;

    fn dv_row(
        pick_id: &str,
        style: &str,
        period: &str,
        t_plus_n: i32,
        outcome: &str,
    ) -> decision_validations::Model {
        decision_validations::Model {
            id: format!("v-{pick_id}-{t_plus_n}"),
            pick_id: pick_id.to_string(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            style: style.into(),
            period: period.into(),
            t_plus_n,
            generated_at: "2026-09-01T10:00:00.000".into(),
            validated_at: "2026-09-10T10:00:00+00:00".into(),
            entry_price: 100.0,
            target_price: 110.0,
            stop_loss: 95.0,
            position_pct: 5.0,
            confidence: 70,
            inferred_action: "buy".into(),
            t_plus_n_price: Some(105.0),
            max_price: Some(108.0),
            min_price: Some(98.0),
            max_return_pct: Some(8.0),
            max_drawdown_pct: Some(2.0),
            final_return_pct: Some(5.0),
            hit_stop_loss: Some(0),
            hit_target: Some(0),
            hit_outcome: Some(outcome.into()),
            factor_snapshot: None,
            data_source: "astock_client".into(),
            created_at: "2026-09-10T10:00:00+00:00".into(),
        }
    }

    /// 别名合并：落库两写（serenity/bottleneck）必须归到同一矩阵名目行。
    #[test]
    fn db_style_aliases_merge_into_matrix_key() {
        let rows = vec![
            dv_row("p1", "bottleneck", "mid", 20, "hit"),
            dv_row("p2", "serenity", "mid", 20, "hit"),
        ];
        let samples = samples_from_validation_rows(rows);
        assert_eq!(samples.len(), 2);
        assert!(samples.iter().all(|s| s.style_key == "serenity"), "{samples:?}");
    }

    /// 矩阵外风格（分析链伪写入方/未知来源）不进闭环。
    #[test]
    fn unknown_style_is_dropped() {
        let samples =
            samples_from_validation_rows(vec![dv_row("p1", "momentum_x", "mid", 20, "hit")]);
        assert!(samples.is_empty());
    }

    /// 历史存档 period 写法 ultrashort 必须被读成 UltraShort（与 Period::from_str 契约一致）。
    #[test]
    fn legacy_period_spelling_is_parsed() {
        let samples =
            samples_from_validation_rows(vec![dv_row("p1", "trend", "ultrashort", 5, "hit")]);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].period, Period::UltraShort);
    }

    /// partial 计 win（与命中率分母同口径）；insufficient 整行剔除。
    #[test]
    fn outcome_uses_single_binary_source() {
        let samples = samples_from_validation_rows(vec![
            dv_row("p1", "trend", "short", 5, "partial"),
            dv_row("p2", "trend", "short", 5, "insufficient"),
            dv_row("p3", "value", "short", 5, "false_hit"),
        ]);
        assert_eq!(samples.len(), 2, "insufficient 整行剔除: {samples:?}");
        assert!(samples.iter().any(|s| s.style_key == "trend" && s.win), "partial 必须计 win");
        assert!(samples.iter().any(|s| s.style_key == "value" && !s.win), "false_hit 必须计 loss");
    }

    /// 一 pick 一样本：mid 档（28 天）三窗口 5/20/60 中取 20（距离 8 < 32）。
    #[test]
    fn one_pick_yields_one_sample_at_nearest_window() {
        let rows = vec![
            dv_row("p1", "trend", "mid", 5, "hit"),
            dv_row("p1", "trend", "mid", 20, "hit"),
            dv_row("p1", "trend", "mid", 60, "miss"),
        ];
        let samples = samples_from_validation_rows(rows);
        assert_eq!(samples.len(), 1, "同 pick 多窗口必须收敛为一行");
        assert_eq!(samples[0].holding_days, 20);
        assert!(samples[0].win, "应取 T+20 那行的判定");
    }

    /// 并列距离取更短窗（保守）：short 档 5 天，窗口 0/10 距离都是 5 ⇒ 取 0 不行，实际取更小 t_plus_n。
    #[test]
    fn tie_break_prefers_shorter_window() {
        let rows = vec![
            dv_row("p1", "trend", "short", 0, "miss"),
            dv_row("p1", "trend", "short", 10, "hit"),
        ];
        let samples = samples_from_validation_rows(rows);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].holding_days, 0, "并列时取更短窗（先入且距离相等不覆盖）");
        assert!(!samples[0].win);
    }

    /// 坏时间戳/坏价格的行不得混进样本（NaN 与不可解析时间都防御）。
    #[test]
    fn corrupt_rows_are_dropped() {
        let mut bad_time = dv_row("p1", "trend", "short", 5, "hit");
        bad_time.validated_at = "not-a-time".into();
        bad_time.created_at = "not-a-time".into();
        let mut bad_price = dv_row("p2", "trend", "short", 5, "hit");
        bad_price.entry_price = 0.0;
        let mut nan_price = dv_row("p3", "trend", "short", 5, "hit");
        nan_price.t_plus_n_price = Some(f64::NAN);
        let samples = samples_from_validation_rows(vec![bad_time, bad_price, nan_price]);
        assert!(samples.is_empty());
    }

    /// 胜率行形态：strategy_id 用矩阵名目、period 用规范键（与 reco_strategy_weights 的
    /// "{style}_{period}" 键空间往返一致）。
    #[test]
    fn performance_row_round_trips_matrix_naming() {
        let samples =
            samples_from_validation_rows(vec![dv_row("p1", "bottleneck", "mid", 20, "hit")]);
        let row = samples[0].to_performance_row();
        assert_eq!(row.strategy_id, "serenity");
        assert_eq!(row.period, "mid");
        assert_eq!(row.was_correct, 1);
    }

    // ── Phase B：逐格合成 ──

    fn mk(
        style: &'static str,
        period: Period,
        conf: f64,
        ret: f64,
        win: bool,
        exit_at_ms: i64,
    ) -> RecoLoopSample {
        RecoLoopSample {
            style_key: style,
            period,
            exit_at_ms,
            win,
            confidence: conf,
            realized_return_pct: ret,
            holding_days: period.default_holding_days(),
            // 夹具默认「产出即验证前一刻」：lookback 过滤用 exit_at_ms，本字段只喂熔断线
            predicted_at_ms: exit_at_ms - 86_400_000,
            dqi_fused_at_prediction: None,
        }
    }

    /// 只改熔断标注，其余沿用 `mk` —— 熔断线的测试要能分别造出三态样本。
    fn mk_dqi(
        style: &'static str,
        period: Period,
        conf: f64,
        ret: f64,
        win: bool,
        exit_at_ms: i64,
        fused: Option<bool>,
    ) -> RecoLoopSample {
        let mut s = mk(style, period, conf, ret, win, exit_at_ms);
        s.dqi_fused_at_prediction = fused;
        s
    }

    fn cell<'a>(res: &'a [LoopCellResult], style: &str, period: &str) -> &'a LoopCellResult {
        res.iter().find(|r| r.style == style && r.period == period).expect("缺格")
    }

    /// 负 IC（conf 与收益反向）⇒ 触发惩罚降权，且低于同规模的无 IC 校准基线。
    #[test]
    fn negative_ic_demotes_below_baseline() {
        let now = 1_700_000_000_000i64;
        // 8 样本，conf 10..80 递增、ret 反向递减 ⇒ Spearman ρ = -1
        let samples: Vec<_> = (0..8)
            .map(|i| mk("trend", Period::Short, 10.0 + i as f64 * 10.0, -i as f64, i < 5, now))
            .collect();
        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let c = cell(&res, "trend", "short");
        assert_eq!(c.status, LoopCellStatus::DemotedNegativeIc, "{c:?}");
        assert!(c.new_weight < BASELINE_WEIGHT, "负 IC 必须降权，实际 {}", c.new_weight);
        assert!(c.new_weight >= WEIGHT_FLOOR, "不得越过下限，实际 {}", c.new_weight);
    }

    /// 正 IC ⇒ 只走胜率线，权重封顶基线 1.0（只降不升，绝不因高胜率提权超过基线）。
    #[test]
    fn non_negative_ic_never_boosts_above_baseline() {
        let now = 1_700_000_000_000i64;
        // conf 与 ret 同向 ⇒ ρ = +1，且全 win（高胜率）
        let samples: Vec<_> = (0..8)
            .map(|i| mk("trend", Period::Short, 10.0 + i as f64 * 10.0, i as f64, true, now))
            .collect();
        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let c = cell(&res, "trend", "short");
        assert_eq!(c.status, LoopCellStatus::IcNonNegative, "{c:?}");
        assert!(
            c.new_weight <= BASELINE_WEIGHT + 1e-9,
            "正 IC 高胜率也不得提权超基线，实际 {}",
            c.new_weight
        );
    }

    /// 样本 < IC_MIN_SAMPLE ⇒ 基线 1.0 且显式「未校准」（不冒充校准过）。
    #[test]
    fn insufficient_samples_stay_baseline_and_mark_uncalibrated() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> = (0..3)
            .map(|i| mk("trend", Period::Mid, 60.0 + i as f64, i as f64, true, now))
            .collect();
        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let c = cell(&res, "trend", "mid");
        assert_eq!(c.status, LoopCellStatus::InsufficientSamples, "{c:?}");
        assert_eq!(c.new_weight, BASELINE_WEIGHT);
    }

    /// conf 一侧恒等（方差退化）⇒ IC 测不出力度 ⇒ 回退基线，标 IcUnmeasurable。
    #[test]
    fn degenerate_variance_marks_ic_unmeasurable() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> =
            (0..8).map(|i| mk("trend", Period::Mid, 60.0, i as f64, true, now)).collect();
        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let c = cell(&res, "trend", "mid");
        assert_eq!(c.status, LoopCellStatus::IcUnmeasurable, "{c:?}");
        assert_eq!(c.new_weight, BASELINE_WEIGHT);
    }

    /// 超短档 lookback 只 2 天：3 天前的样本必须落在窗口外 ⇒ 视作样本不足。
    #[test]
    fn stale_samples_fall_outside_short_lookback() {
        let now = 1_700_000_000_000i64;
        let day = 86_400_000i64;
        let samples: Vec<_> = (0..8)
            .map(|i| {
                mk("trend", Period::UltraShort, 10.0 + i as f64, -i as f64, true, now - 3 * day)
            })
            .collect();
        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let c = cell(&res, "trend", "ultra_short");
        assert_eq!(c.status, LoopCellStatus::InsufficientSamples, "3 天前样本应出超短 2 天窗");
        assert_eq!(c.new_weight, BASELINE_WEIGHT);
    }

    /// 矩阵不成立格（reversion×ultra_short）⇒ NotInMatrix，不产权重。
    #[test]
    fn inactive_cell_is_not_in_matrix() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> = (0..8)
            .map(|i| mk("reversion", Period::UltraShort, 10.0 + i as f64, -i as f64, false, now))
            .collect();
        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let c = cell(&res, "reversion", "ultra_short");
        assert_eq!(c.status, LoopCellStatus::NotInMatrix, "{c:?}");
        assert_eq!(c.new_weight, BASELINE_WEIGHT);
    }

    /// IPC 契约（禁区 13）：LoopCellResult 跨边界必须 camelCase，status 用 snake_case 词表。
    #[test]
    fn loop_cell_result_serializes_camel_case() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> = (0..8)
            .map(|i| mk("trend", Period::Short, 10.0 + i as f64 * 10.0, -i as f64, true, now))
            .collect();
        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let value = serde_json::to_value(cell(&res, "trend", "short")).unwrap();
        let obj = value.as_object().unwrap();
        for key in
            ["style", "period", "status", "rankIc", "samples", "winRate", "oldWeight", "newWeight"]
        {
            assert!(obj.contains_key(key), "LoopCellResult 缺 camelCase 键 {key}: {obj:?}");
        }
        assert!(!obj.contains_key("rank_ic"));
        assert_eq!(obj["status"].as_str(), Some("demoted_negative_ic"), "status 须 snake_case");
    }

    // ── Phase 4：mover 漏检线 ──

    /// Laplace 转化率 + 两条守卫：无截断留痕（scored_out=0）不算证据；
    /// 总样本 < `MOVER_MIN_EVIDENCE` 不猜。
    #[test]
    fn mover_penalty_requires_evidence_and_uses_laplace_ratio() {
        let empty = MoverCellSignal::default();
        assert_eq!(empty.penalty("trend", "short"), None, "无任何证据必须 None");

        let mut s = MoverCellSignal::default();
        // 只有命中、没有任何截断留痕 ⇒ 仍算无证据（不是「表现完美」）
        s.cells.insert(("trend".into(), "short".into()), (0, 5));
        assert_eq!(s.penalty("trend", "short"), None);

        // 有留痕但总样本不足 ⇒ None
        s.cells.insert(("trend".into(), "short".into()), (2, 0));
        assert_eq!(s.penalty("trend", "short"), None);

        // 9 个推出去、0 个要中 ⇒ (0+1)/(9+1) = 0.1
        s.cells.insert(("trend".into(), "short".into()), (9, 0));
        let p = s.penalty("trend", "short").unwrap();
        assert!((p - 0.1).abs() < 1e-9, "得 {p}");

        // 3 个推出去、1 个要中 ⇒ (1+1)/(3+1+1) = 0.4
        s.cells.insert(("trend".into(), "short".into()), (3, 1));
        let p = s.penalty("trend", "short").unwrap();
        assert!((p - 0.4).abs() < 1e-9, "得 {p}");

        // 永不提权：几乎全要中时也 ≤ 1.0
        s.cells.insert(("trend".into(), "short".into()), (1, 100));
        let p = s.penalty("trend", "short").unwrap();
        assert!(p <= 1.0 && p > 0.98, "得 {p}");
    }

    /// mover 线叠乘在 IC/胜率线之后：比无惩罚更低，下限不破 `WEIGHT_FLOOR`，
    /// 且惩罚值与留痕条数都进 DTO（呈现层要显示「为什么降」）。
    #[test]
    fn mover_line_demotes_after_ic_line() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> = (0..8)
            .map(|i| mk("trend", Period::Short, 10.0 + i as f64 * 10.0, i as f64, true, now))
            .collect();
        let base =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let base_w = cell(&base, "trend", "short").new_weight;
        assert!(base_w > WEIGHT_FLOOR, "前提：无 mover 证据时该格已校准出 {base_w}");

        let mut sig = MoverCellSignal::default();
        sig.cells.insert(("trend".into(), "short".into()), (9, 0));
        let with_mover = compute_loop_cell_weights(&samples, &HashMap::new(), now, &sig);
        let c = cell(&with_mover, "trend", "short");
        let expected = (base_w * 0.1).clamp(WEIGHT_FLOOR, BASELINE_WEIGHT);
        assert!(
            (c.new_weight - expected).abs() < 1e-9,
            "mover 线须以乘数叠加：{} vs 期望 {expected}",
            c.new_weight
        );
        assert!(c.new_weight < base_w, "降权必须真实生效");
        assert_eq!(c.mover_penalty, Some(0.1), "乘数须进 DTO");
        assert_eq!(c.scored_out, 9, "留痕条数须进 DTO");

        // 按设计不出票的格不吃 mover 线（也就不该出现在降权清单里）
        let sig2 = MoverCellSignal {
            cells: vec![(("reversion".into(), "ultra_short".into()), (9, 0))].into_iter().collect(),
        };
        let res = compute_loop_cell_weights(&samples, &HashMap::new(), now, &sig2);
        let c = cell(&res, "reversion", "ultra_short");
        assert_eq!(c.status, LoopCellStatus::NotInMatrix);
        assert_eq!(c.mover_penalty, None);
        assert_eq!(c.scored_out, 0);
    }

    // ── #8 P5：数据质量熔断降权线 ──

    /// 造一批同格样本，前 `fused_n` 条标成「出生于熔断态」，其余标成清白。
    fn dqi_batch(fused_n: usize, clear_n: usize) -> Vec<RecoLoopSample> {
        let now = 1_700_000_000_000i64;
        (0..fused_n)
            .map(|i| mk_dqi("trend", Period::Short, 20.0 + i as f64, -1.0, true, now, Some(true)))
            .chain((0..clear_n).map(|i| {
                mk_dqi("trend", Period::Short, 20.0 + i as f64, -1.0, true, now, Some(false))
            }))
            .collect()
    }

    /// 分母只数可判定样本：全清白 ⇒ 乘数 1.0（不降）；全熔断 ⇒ 1/(n+1)（Laplace 转化率）。
    #[test]
    fn dqi_line_is_laplace_ratio_of_clear_among_judgeable() {
        let now = 1_700_000_000_000i64;

        let all_clear = compute_loop_cell_weights(
            &dqi_batch(0, IC_MIN_SAMPLE),
            &HashMap::new(),
            now,
            &MoverCellSignal::default(),
        );
        let c = cell(&all_clear, "trend", "short");
        assert_eq!(c.dqi_penalty, Some(1.0), "全部出生于正常期 ⇒ 乘数 1.0，不该动权重");

        let all_fused = compute_loop_cell_weights(
            &dqi_batch(IC_MIN_SAMPLE, 0),
            &HashMap::new(),
            now,
            &MoverCellSignal::default(),
        );
        let c = cell(&all_fused, "trend", "short");
        let expect = 1.0 / (IC_MIN_SAMPLE as f64 + 1.0);
        let c_dqi = c.dqi_penalty.expect("全熔断应有乘数");
        assert!((c_dqi - expect).abs() < 1e-9, "全熔断 ⇒ (0+1)/(n+1)：{c_dqi} vs 期望 {expect}");
        assert!(c_dqi > WEIGHT_FLOOR, "夹具不该只测到钳位边界");
    }

    /// 同一批样本，标熔断的那一半必须真的把权重压下去，且**只降不升**。
    #[test]
    fn dqi_line_only_lowers_and_is_visible_in_new_weight() {
        let now = 1_700_000_000_000i64;
        let base = compute_loop_cell_weights(
            &dqi_batch(0, 2 * IC_MIN_SAMPLE),
            &HashMap::new(),
            now,
            &MoverCellSignal::default(),
        );
        let mixed = compute_loop_cell_weights(
            &dqi_batch(IC_MIN_SAMPLE, IC_MIN_SAMPLE),
            &HashMap::new(),
            now,
            &MoverCellSignal::default(),
        );
        let b = cell(&base, "trend", "short");
        let m = cell(&mixed, "trend", "short");
        assert_eq!(b.dqi_penalty, Some(1.0), "全清白 ⇒ 乘数恰为 1.0（不提权）");
        // (8+1)/(8+8+1) = 9/17 —— 分子分母都带 Laplace 平滑，期望值按同式现算而不是手敲小数
        let expect_mixed = (IC_MIN_SAMPLE as f64 + 1.0) / (2.0 * IC_MIN_SAMPLE as f64 + 1.0);
        let m_penalty = m.dqi_penalty.expect("一半样本出生于熔断期 ⇒ 本线必须参与");
        assert!(
            (m_penalty - expect_mixed).abs() < 1e-12,
            "(clear+1)/(total+1)：{m_penalty} vs 期望 {expect_mixed}"
        );
        assert!(
            m.new_weight < b.new_weight,
            "一半样本出生于熔断期 ⇒ 权重必须更低：{:?} vs {:?}",
            m.new_weight,
            b.new_weight
        );
        assert!(m.new_weight <= BASELINE_WEIGHT, "永不提权");
    }

    /// `None`（观测覆盖不到）既不进分子也不进分母，更不让本线参与。
    #[test]
    fn unjudgeable_samples_leave_the_dqi_line_out_entirely() {
        let now = 1_700_000_000_000i64;
        let unobserved: Vec<_> = (0..2 * IC_MIN_SAMPLE)
            .map(|i| mk("trend", Period::Short, 20.0 + i as f64, -1.0, true, now))
            .collect();
        let res = compute_loop_cell_weights(
            &unobserved,
            &HashMap::new(),
            now,
            &MoverCellSignal::default(),
        );
        let c = cell(&res, "trend", "short");
        assert_eq!(c.dqi_penalty, None, "没有任何可判定样本 ⇒ 本线不参与（不是「按正常处理」）");

        // 可判定样本不足 DQI_MIN_SAMPLES ⇒ 同样不参与（一次抖动不给整格定调）。
        // ⚠ 只有前 `DQI_MIN_SAMPLES-1` 条带标注，其余必须是 `None`：本线数的是
        //   「可判定」条数，全标成 Some 就变成 8 条可判定 ⇒ 夹具测不到那条守卫分支。
        let few: Vec<_> = (0..IC_MIN_SAMPLE)
            .map(|i| {
                mk_dqi(
                    "trend",
                    Period::Short,
                    20.0 + i as f64,
                    -1.0,
                    true,
                    now,
                    if i < DQI_MIN_SAMPLES - 1 {
                        Some(true)
                    } else {
                        None
                    },
                )
            })
            .collect();
        let res2 =
            compute_loop_cell_weights(&few, &HashMap::new(), now, &MoverCellSignal::default());
        assert_eq!(
            cell(&res2, "trend", "short").dqi_penalty,
            None,
            "可判定 {} 条 < DQI_MIN_SAMPLES ⇒ 不猜",
            DQI_MIN_SAMPLES - 1
        );
    }

    /// IPC 契约：新增字段跨边界必须是 `dqiPenalty`，且缺席时不出现在 JSON 里（与 mover 线同形）。
    #[test]
    fn dqi_penalty_serializes_camel_case_and_skips_when_absent() {
        let now = 1_700_000_000_000i64;
        let present = compute_loop_cell_weights(
            &dqi_batch(IC_MIN_SAMPLE, 0),
            &HashMap::new(),
            now,
            &MoverCellSignal::default(),
        );
        let obj = serde_json::to_value(cell(&present, "trend", "short")).unwrap();
        let obj = obj.as_object().unwrap();
        assert!(obj.contains_key("dqiPenalty"), "缺 camelCase 键: {obj:?}");
        assert!(!obj.contains_key("dqi_penalty"));

        let absent = compute_loop_cell_weights(
            &dqi_batch(0, 0),
            &HashMap::new(),
            now,
            &MoverCellSignal::default(),
        );
        let obj2 = serde_json::to_value(cell(&absent, "trend", "short")).unwrap();
        assert!(
            !obj2.as_object().unwrap().contains_key("dqiPenalty"),
            "未参与合成的线不该在 JSON 里留一个 null 让前端猜"
        );
    }

    /// 按设计不出票的格不吃熔断线（与 mover 线同一条纪律）。
    #[test]
    fn not_in_matrix_cell_never_carries_dqi_penalty() {
        let now = 1_700_000_000_000i64;
        let samples: Vec<_> = (0..2 * IC_MIN_SAMPLE)
            .map(|i| {
                mk_dqi(
                    "reversion",
                    Period::UltraShort,
                    20.0 + i as f64,
                    -1.0,
                    true,
                    now,
                    Some(true),
                )
            })
            .collect();
        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), now, &MoverCellSignal::default());
        let c = cell(&res, "reversion", "ultra_short");
        assert_eq!(c.status, LoopCellStatus::NotInMatrix);
        assert_eq!(c.dqi_penalty, None);
    }

    /// 纯函数入口的默认值必须是 `None` —— 夹具/回放里忘了标注时应「不参与」而不是「算清白」。
    #[test]
    fn mk_default_leaves_samples_unjudgeable() {
        let s = mk("trend", Period::Short, 50.0, 1.0, true, 1_700_000_000_000);
        assert_eq!(s.dqi_fused_at_prediction, None);
        assert_eq!(dqi_penalty_of(&[&s]), None);
    }
}
