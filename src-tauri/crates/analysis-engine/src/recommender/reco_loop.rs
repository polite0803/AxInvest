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
//!
//! ## 数据面列名（阶段 3 权重合成的**全部**输入；不新建表、不改 migration）
//!
//! - `decision_validations`：`pick_id` / `style` / `period` / `t_plus_n` /
//!   `confidence` / `entry_price` / `t_plus_n_price` / `hit_outcome` /
//!   `generated_at` / `validated_at` / `created_at` / `as_of_date`。
//!   胜负与收益一律从这些列现算（`hit_outcome_to_binary_outcome` + entry→T+N 收益），
//!   不读任何预聚合胜率表 —— 口径只有一处。
//! - `reco_picks`：`id` / `reco_version`（样本代际归属，`load_pick_versions`）。
//! - `strategy_weight_history`：`strategy_id` / `period` / `old_weight` /
//!   `new_weight` / `sample_size` / `win_rate` / `trigger` / `applied_at` /
//!   `rationale`。读侧按 `trigger='reco-loop'` 取每格最新一行（`evolution_drift`），
//!   写侧**只追加**，不改历史行。
//! - `reco_scan_audit`：`style` / `period` / `stock_name` / `rank`（mover 漏检线）。
//! - `data_quality_observations`：经 `dao` 的熔断时间线（`dqi_fused_at_prediction`）。
//!
//! 权重纪律（阶段 3 的两条硬判据，均有单测钉在文件末尾）：
//! - **样本门**：窗口内可用样本 < `IC_MIN_SAMPLE` 或 IC 方差退化 ⇒ 胜率/IC 线**不动**，
//!   保留该格当前权重（不是回退基线 1.0 —— 回退会把上一轮的降权洗掉）；
//! - **只降不升**：最终权重 = `min(本轮计算值, 当前权重)`，任何分支都不得把权重抬高。
//!
//! 唯一还有独立降权权的是 mover（漏检）与熔断（数据质量）两条外乘线：它们各有自己的
//! 样本门（`MOVER_MIN_EVIDENCE` / `DQI_MIN_SAMPLES`），且乘数恒 ≤ 1。

use std::collections::HashMap;

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::Serialize;

use axagent_entities::decision_validations;
use axagent_entities::stock_analyses;
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
    /// 来源 pick 的主键 —— 代际标注轴（[`load_reco_loop_samples`] 据此回查
    /// `reco_picks.reco_version`）。
    pub pick_id: String,
    /// 产出这条 pick 时的**荐股算法代际**（`reco_picks.reco_version`）。
    ///
    /// 由 [`load_reco_loop_samples`] 标注，纯函数入口默认 `None`（= 归属未知）。
    /// `None` 与 `Some(旧代)` 在 IC 筛样时都**不进分母**，但两类排除分别计数 ——
    /// 前者该去补写侧盖章，后者是换代后的正常积累（同分析链 `filter_by_generation_floor` 裁定）。
    pub reco_version: Option<i32>,
}

/// 逐格闭环态（Phase B 计算、Phase D 呈现共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCellStatus {
    /// 样本不足（n < IC_MIN_SAMPLE）⇒ 胜率/IC 线不动，**保留该格当前权重**
    InsufficientSamples,
    /// IC 不可测（置信一侧方差退化）⇒ 同「样本门」：保留当前权重
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
        // 代际闸门：回放样本（`as_of_date` 非空）不进**实盘**权重闭环。读侧 SQL 已过滤，
        // 这里是同一判据在纯函数层的副本 —— 只有 SQL 过滤时单测查不到（要 create_test_pool
        // 才碰得到），且未来若有调用方漏带谓词，这里不会静默混码。
        if r.as_of_date.is_some() {
            continue;
        }
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
            pick_id: r.pick_id.clone(),
            // 代际同 `dqi`：纯函数不查库 ⇒ 由 `load_reco_loop_samples` 回查 pick 后标注
            reco_version: None,
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
            // 代际跟着样本走：IC 筛样判据在 `aggregate_reco_ic`，本处只负责把归属带过去
            algorithm_version: self.reco_version,
        }
    }
}

/// ── 证据分桶校准（2026-10-18，v4 语义重构的「真概率最后一公里」）──
///
/// `confidence` 自 v4 起是**证据强度**（策略条件满足度 × 风格权重），不是概率。
/// 真概率的唯一正路：按 (style, period, 证据分桶) 统计**闭环真实 T+N 胜率**
/// （`decision_validations.hit_outcome`），样本足够的桶才允许输出「校准置信度」，
/// 不足的桶显式返回 `None`（前端标「样本不足」）——样本不足时任何「区分个股的
/// 概率」都是新的造假。
///
/// 口径事实：`decision_validations.confidence` = 出票时的 `confidence`（证据 × 该格
/// 风格权重）。同格 (style, period) 内权重是**同一常数** ⇒ 它对桶只是整体平移，
/// 不影响桶内胜率纯度，无需反推原值。
/// 代际隔离：旧代（≤v3）的 confidence 是伪概率常数（60/56/53/51），混桶即污染
/// ⇒ `min_reco_version` 以下的样本不计。
///
/// 证据桶宽（20 分一桶，0-100 共 5 桶）。桶宽取粗不取细：桶越细样本越稀，
/// 「校准了个寂寞」比「校准得粗」更糟。
pub const EVIDENCE_BUCKET_WIDTH: f64 = 20.0;

/// 桶达标的最小样本数（与 `ic::IC_MIN_SAMPLE` 同量纲精神：没有下限的统计都是噪声）。
pub const EVIDENCE_BUCKET_MIN_SAMPLES: usize = 20;

/// 证据分 → 桶下标（0..=4）；100 落最后一桶。
pub fn evidence_bucket_index(confidence: f64) -> usize {
    ((confidence / EVIDENCE_BUCKET_WIDTH) as usize).min(4)
}

/// 桶校准（纯函数）：统计 (style, period, 证据桶) 格内的闭环真实胜率。
///
/// 返回 `Some((胜率 0~1, 桶内样本数))` 当且仅当样本数 ≥ `min_samples`；
/// 否则 `None`（调用方显式标「样本不足」，不得回落到任何合成值）。
pub fn bucket_calibrated_win_rate(
    samples: &[RecoLoopSample],
    style_key: &str,
    period: Period,
    confidence: f64,
    min_reco_version: i32,
    min_samples: usize,
) -> Option<(f64, usize)> {
    let bucket = evidence_bucket_index(confidence);
    let mut n = 0usize;
    let mut wins = 0usize;
    for s in samples {
        if s.style_key != style_key || s.period != period {
            continue;
        }
        // 代际闸门：旧代 confidence 是伪概率常数，混桶即污染（与 IC 的
        // `filter_by_generation_floor` 同一裁定；未知代（None）同样不计）。
        if s.reco_version.unwrap_or(0) < min_reco_version {
            continue;
        }
        if evidence_bucket_index(s.confidence) != bucket {
            continue;
        }
        n += 1;
        if s.win {
            wins += 1;
        }
    }
    if n < min_samples {
        return None;
    }
    Some((wins as f64 / n as f64, n))
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

/// 回查一批 pick 的算法代际：`pick_id → reco_version`，**只收录有代际的行**。
///
/// 为什么返回空 map 而不是报错：取数失败 ⇒ 所有样本都成「代际未知」⇒ IC 分母归零并
/// 如实报 `excluded_unknown_generation`，**不**退回「当作当代」（那会把「不知道」写成「有把握」）。
/// 面板侧（`reco_ic_stats`）与闭环侧（[`load_reco_loop_samples`]）共用本函数，
/// 保证两处看到的是同一套归属（禁区 12）。
pub async fn load_pick_versions(
    db: &DatabaseConnection,
    pick_ids: &[String],
) -> HashMap<String, i32> {
    use axagent_entities::reco_picks;
    if pick_ids.is_empty() {
        return HashMap::new();
    }
    match reco_picks::Entity::find()
        .filter(reco_picks::Column::Id.is_in(pick_ids.to_vec()))
        .all(db)
        .await
    {
        Ok(picks) => picks.into_iter().filter_map(|p| p.reco_version.map(|v| (p.id, v))).collect(),
        Err(e) => {
            tracing::warn!("[reco_loop] 代际回查失败 ⇒ 本轮样本一律按「代际未知」剔除: {e}");
            HashMap::new()
        },
    }
}

/// 从库中读取全部已验证的荐股样本（薄封装；归一化与判定全在纯函数里）。
pub async fn load_reco_loop_samples(
    db: &DatabaseConnection,
) -> Result<Vec<RecoLoopSample>, String> {
    // 代际过滤：实盘权重闭环只吃**实盘**样本。回放的验证行（`as_of_date` 非空）是另一套
    // 计数域，混进来会让 IC 把「历史回放的预测」与「实盘的预测」配在同一格里降权
    // （与分析链伪风格行同表混码是同一类错误，见本文件头 D1）。
    // 回放样本的用途是代际对照/证伪，走显式入口，不在这里默认并入。
    let rows = decision_validations::Entity::find()
        .filter(decision_validations::Column::AsOfDate.is_null())
        .all(db)
        .await
        .map_err(|e| format!("读取 decision_validations 失败: {e}"))?;
    // B3 窗口换代（拍板 6/7）：本闭环的分母只吃**当前四档窗口集**里的 T+N 行。
    // 判据与分析链两处读点同源（`Period::is_validation_window`），三处不同源就会出现
    // 「报表里有、闭环里没有」这类无法裁决的差异。旧窗口集 [5,20,60] 产的 20/60 行
    // 留在库里不删不回填，只是不计票。
    let (window_rows, pre_window_rows): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .partition(|r| axagent_harness::holding_period::Period::is_validation_window(r.t_plus_n));
    if !pre_window_rows.is_empty() {
        tracing::info!(
            "[reco_loop] {} 条 T+N 属上线前窗口集 ⇒ 未进闭环分母（当前窗口集 = {:?}）",
            pre_window_rows.len(),
            axagent_harness::holding_period::Period::validation_windows()
        );
    }
    let mut samples = samples_from_validation_rows(window_rows);
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
    // 代际归属：回查来源 pick 的 `reco_version` 标到样本上（IC 筛样的输入）。
    let pick_ids: Vec<String> = samples.iter().map(|s| s.pick_id.clone()).collect();
    let versions = load_pick_versions(db, &pick_ids).await;
    for s in samples.iter_mut() {
        s.reco_version = versions.get(&s.pick_id).copied();
    }
    Ok(samples)
}

// ── Phase B：胜率 × IC 合成逐格权重（PLAN-reco-reflection-closure）──

/// 闭环留痕的 trigger 标记（`strategy_weight_history.trigger`）。
/// 与分析链演化的 `"cron"|"manual"|"rule"` 分键空间消费（Q3 归属分离）。
///
/// 权威定义在 `evolution_drift`（分析链的当前权重/仪表盘也要用它做排除），
/// 这里 `pub use` 保持既有调用路径不变。
pub use crate::evolution_drift::RECO_LOOP_TRIGGER;

/// 基线权重：闭环**从未降过权**的格的取值，也是「只降不升」（Q1）的绝对上限。
///
/// ⚠ 它**不是**「未校准格的取值」：样本门未通过时保留该格当前权重（可能已被降到 0.16），
/// 回退基线会把历史降权洗掉（见 `keep_prior_weight` / `ratchet_weight`）。
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

/// 一条样本是否可参与权重合成（纯函数，**损坏行跳过**的唯一判据）。
///
/// 为什么需要它：`samples_from_validation_rows` 已挡住「价格非有限 / entry ≤ 0」，
/// 但那条防线只在**落库归一化**时生效；`compute_loop_cell_weights` 也接受夹具与
/// 未来其它入口直接构造的样本。NaN 一旦进入 `rank IC` 或 EWMA，会把整格权重写成 NaN，
/// 而 NaN 落库后 `clamp` 不再有意义（NaN 比较恒 false），下一轮读回仍是 NaN。
///
/// 判据刻意只收「可机械判定的损坏」，不按风格/档位筛样（那是调用方的事）：
/// - `confidence` 必须有限且在 0..=100（越界只可能来自损坏行，不是真实证据）；
/// - `realized_return_pct` 必须有限；
/// - `exit_at_ms` / `predicted_at_ms` 必须非负（时间戳损坏）。
pub fn sample_is_usable(s: &RecoLoopSample) -> bool {
    s.confidence.is_finite()
        && (0.0..=100.0).contains(&s.confidence)
        && s.realized_return_pct.is_finite()
        && s.exit_at_ms >= 0
        && s.predicted_at_ms >= 0
}

/// 只降不升棘轮（纯函数）：本轮计算值永远不得高于该格**当前**权重。
///
/// 为什么不能只做 `min(computed, BASELINE_WEIGHT)`：基线是 1.0，而该格上一轮可能已被
/// 降到 0.16 —— 只封顶基线会让下一轮「样本不足 / 方差退化」分支把它抬回 1.0，
/// 等于每轮把历史降权洗掉一次（现网 `strategy_weight_history` 实测：
/// `trend×ultra_short` 0.164→1.0、`reversion×short` 0.916→1.0，样本分别 0 / 3 条）。
///
/// 输入防御：`old` 非有限 ⇒ 视为基线；`computed` 非有限 ⇒ 保持 `old`（不写 NaN）；
/// 两者都先归一进 `[WEIGHT_FLOOR, BASELINE_WEIGHT]` 再比，避免 `clamp` 的 min > max 崩溃。
pub fn ratchet_weight(computed: f64, old: f64) -> f64 {
    let old = if old.is_finite() {
        old.clamp(WEIGHT_FLOOR, BASELINE_WEIGHT)
    } else {
        BASELINE_WEIGHT
    };
    if !computed.is_finite() {
        return old;
    }
    computed.clamp(WEIGHT_FLOOR, old)
}

/// 胜率/IC 线未校准时的权重（纯函数）：保留当前权重，只允许独立外乘线继续降。
///
/// 「样本门」的准确语义是**这条线不动**，不是「整格一律不动」：mover 漏检线与熔断线
/// 各有自己的样本门与证据源，它们在本格样本不足时仍可独立成立（例如该格 T+N 还没
/// 攒够，但 `reco_scan_audit` 已记录 9 次「算出了却没要」）。
pub fn keep_prior_weight(old_weight: f64, outer_penalty: f64) -> f64 {
    let outer = if outer_penalty.is_finite() {
        outer_penalty.clamp(WEIGHT_FLOOR, 1.0)
    } else {
        1.0
    };
    ratchet_weight(old_weight * outer, old_weight)
}

/// 逐格合成（纯函数）：胜率线（`weight_decay` 基元）× 力度线（`ic` 基元）
/// × mover 线（`reco_scan_audit` 截断留痕，Phase 4）× 熔断线（`data_quality_observations`，#8 P5）。
///
/// 规则（对齐已批方案 Q1/P2/Q5，阶段 3 补两条硬判据）：
/// - 矩阵不成立格 ⇒ `NotInMatrix`，不产权重；
/// - 该档窗口（`default_holding_days`，Q5 按档对齐）内**可用**样本 < `IC_MIN_SAMPLE`
///   ⇒ `InsufficientSamples`，胜率/IC 线不动（`keep_prior_weight`，保留当前权重）；
/// - IC 可测但一侧方差退化（含 rankIC 非有限）⇒ `IcUnmeasurable`，同上保留当前权重；
/// - IC ≥ 0 ⇒ 只吃胜率降权（`min(aw, 1.0)`），最终再过棘轮 `min(本轮, 当前)`；
/// - IC < 0 ⇒ 胜率权重再乘 `NEGATIVE_IC_PENALTY`，下限 `WEIGHT_FLOOR`；
/// - 最后（无论 IC 线是否校准）叠乘 `mover` 线：只降不升，且**永不提权**
///   （`PLAN-mover-recall-attribution.md` §Phase 4 明文禁止用 mover 数据提权）。
/// - 再叠乘 `熔断` 线（#8 P5）：本格窗口里「出生时数据质量已熔断」的样本占比 ⇒ 同一条只降不升。
///   它问的是**证据的可信度**而不是策略的预测力：胜率 80% 却全部产自坏数据期，
///   这个 80% 本身不可用；不动它就等于让"取数一直在坏"这件事对权重零成本。
/// - **只降不升**对所有分支统一生效（`ratchet_weight`）：任何情况下 `new_weight ≤ old_weight`。
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
                    s.style_key == style_key
                        && s.period == *period
                        && s.exit_at_ms >= cutoff
                        // 损坏行（NaN/越界置信度/坏时间戳）一律不进分母 —— 见 `sample_is_usable`
                        && sample_is_usable(s)
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
                    // 样本门：胜率/IC 线不动 ⇒ 保留当前权重（不是回退基线 1.0）。
                    //   只允许 mover/熔断两条独立外乘线继续降（它们有自己的样本门）。
                    new_weight: keep_prior_weight(old_weight, outer_penalty),
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
            // rankIC 非有限（NaN/∞）与「没样本」同处置：测不出力度就不校准，
            // 不让 NaN 进入 `< 0.0` 的比较（NaN 恒 false ⇒ 会被误判成「非负 IC」）。
            let rank_ic = rank_ic.filter(|v| v.is_finite());
            let ic_status = if ic_status == "ok" && rank_ic.is_none() {
                "degenerate_variance"
            } else {
                ic_status
            };

            let win = aw.map(|a| (a.new_weight.min(BASELINE_WEIGHT), a.win_rate));
            let (win_w, win_rate) = match win {
                Some((w, wr)) => (Some(w), Some(wr)),
                None => (None, None),
            };
            let (status, new_weight) = if ic_status != "ok" {
                // 样本门闸三态（P2）：测不出力度就不校准 ⇒ 保留当前权重；
                //   outer_penalty 在下方统一叠乘，这里不得重复乘一次。
                (LoopCellStatus::IcUnmeasurable, old_weight)
            } else if rank_ic.unwrap_or(0.0) < 0.0 {
                (
                    LoopCellStatus::DemotedNegativeIc,
                    win_w.unwrap_or(old_weight) * NEGATIVE_IC_PENALTY,
                )
            } else {
                (LoopCellStatus::IcNonNegative, win_w.unwrap_or(old_weight))
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
                // 只降不升棘轮：无论哪条线算出什么，都不得高于该格当前权重
                new_weight: ratchet_weight(blended, old_weight),
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
                // 样本门未通过时**不是**「回退基线」：那是旧实现把历史降权洗掉的写法。
                // 现在的语义是保留当前权重（只允许 mover/熔断线继续降）。
                "闭环未校准({}): 窗口内 {} 样本, 保持当前权重 {:.2}→{:.2}",
                r.status.as_str(),
                r.samples,
                r.old_weight,
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
    // 心跳与写库解耦：|Δ|<1% 的轮次不写权重行，但「确实算过」也要留证据 ——
    // 否则面板的「最近重算」会停在几天前，被误读成反思已停。
    persist_last_recalc_at(db, applied_at).await;
    Ok((written, results))
}

/// 「最近一次计算时刻」的 settings 键（KV 表，`settings`）。
///
/// 与 `strategy_weight_history.applied_at` 的区别：后者只在**有权重变化**时推进。
/// 面板要回答的是「反思还在不在跑」，所以必须按计算落点记录。
pub const LAST_RECALC_SETTING_KEY: &str = "reco_loop_last_recalc_at";

/// 记录一次闭环计算（成功写完/跳过权重行都算）。失败只 warn：心跳丢一次不该让
/// 整轮重算失败，面板退化为读权重留痕时刻（`reco_loop_view` 的回退分支）。
async fn persist_last_recalc_at(db: &DatabaseConnection, at_ms: i64) {
    use axagent_entities::settings;
    use sea_orm::{ActiveModelTrait, Set};

    let value = at_ms.to_string();
    match settings::Entity::find_by_id(LAST_RECALC_SETTING_KEY).one(db).await {
        Ok(Some(row)) => {
            let mut active: settings::ActiveModel = row.into();
            active.value = Set(value);
            if let Err(e) = active.update(db).await {
                tracing::warn!("[reco_loop] 记录最近计算时刻失败: {e}");
            }
        },
        Ok(None) => {
            let active = settings::ActiveModel {
                key: Set(LAST_RECALC_SETTING_KEY.to_string()),
                value: Set(value),
            };
            if let Err(e) = active.insert(db).await {
                tracing::warn!("[reco_loop] 记录最近计算时刻失败: {e}");
            }
        },
        Err(e) => tracing::warn!("[reco_loop] 读取最近计算时刻失败: {e}"),
    }
}

/// 读「最近一次计算时刻」（ms）。`None` = 心跳从未写过（首次/旧库）。
pub async fn load_last_recalc_at(db: &DatabaseConnection) -> Option<i64> {
    use axagent_entities::settings;
    settings::Entity::find_by_id(LAST_RECALC_SETTING_KEY)
        .one(db)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.value.parse::<i64>().ok())
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

    let mut vars: Vec<(String, serde_json::Value)> = match template {
        Some(t) => extract_template_vars(&t),
        None => Vec::new(),
    };

    // ── 分析回环否决（P1，2026-10-18，002696 案例复盘）──────────────────────
    // 与下方 IC 权重闭环**相互独立**（所以放在 gate != "on" 早返回之前）：IC 闭环管
    // 「该风格该不该降权」，这里管「这只票最近被判过卖出，别再出」。判定源是
    // `stock_analyses.horizon_decisions` 既有列（不新建表），消费方是
    // `gates::apply_exit_gates`（mid/long 出票口）。加载失败 fail-open（warn 留痕），
    // 与财报门同一纪律：宁可放过不可错杀，但「门没生效」必须可归因。
    // ⚠ 回放过滤**不在加载侧做**：served_vars 在命令层 as_of scope 之外加载，
    //   此刻 current_as_of() 恒为 None，加载侧守卫是假守卫。唯一权威守卫在消费侧
    //   （`scan_period`：回放不消费否决表 ⇒ 回测语义 =「当时的判定」）。
    match load_analysis_veto_var(db).await {
        Ok(Some(v)) => vars.push(("reco_analysis_vetoes".to_string(), v)),
        Ok(None) => {},
        Err(e) => {
            tracing::warn!("[reco_loop] 分析否决表加载失败（fail-open，回环门不生效）: {e}")
        },
    }

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
    //
    // **逐格覆盖、不是整表替换**：闭环只在 |Δ|≥1% 时写行，所以 `looped` 只含「被闭环
    // 降过的格」。若整表替换，用户在「⚡ 校准」里写入、但闭环从未管的格会整条键消失，
    // 静默回落到策略默认值 —— 校准按钮看起来成功、实际配置丢了。
    let static_weights = static_strategy_weights(&vars);
    let merged = merge_static_and_loop_weights(static_weights, &looped);
    let mut v = vars.into_iter().filter(|(k, _)| k != "reco_strategy_weights").collect::<Vec<_>>();
    v.push(("reco_strategy_weights".to_string(), serde_json::Value::Object(merged)));
    Ok(v)
}

/// 模板静态权重（`reco_strategy_weights`）的 object 形态；缺失/损坏 ⇒ 空表。
fn static_strategy_weights(
    vars: &[(String, serde_json::Value)],
) -> serde_json::Map<String, serde_json::Value> {
    vars.iter()
        .find(|(k, _)| k == "reco_strategy_weights")
        .and_then(|(_, v)| v.as_object().cloned())
        .unwrap_or_default()
}

/// 静态权重表与闭环权重的合并（纯函数，供单测直接喂表）。
///
/// 合并规则：
/// - 静态表里的格**全部保留**（闭环没管过的格不能丢）；
/// - 闭环算过的格覆盖同名键；
/// - 闭环值在读取侧夹到 `≤ 1.0`：历史上被旧实现抬高到 >1 的行不再生效（不回写历史，
///   真正的历史最小值回滚是单独的数据订正）。
pub fn merge_static_and_loop_weights(
    static_weights: serde_json::Map<String, serde_json::Value>,
    looped: &HashMap<(String, String), f64>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut merged = static_weights;
    for ((s, p), w) in looped {
        merged.insert(format!("{s}_{p}"), serde_json::json!(w.min(1.0)));
    }
    merged
}

// ── P1 分析回环否决表（2026-10-18）────────────────────────────────────────

/// 判定源窗口（天）：更早的分析不构成否决。见 `gates::ANALYSIS_VETO_WINDOW_DAYS`。
const ANALYSIS_VETO_WINDOW_DAYS: i64 = 30;

/// 否决查询的投影行（不取 `blackboard_snapshot` 大列 —— 那是整份黑板 JSON，
/// 拉它会把这个本该廉价的查询变成 IO 事故）。
#[derive(Debug, FromQueryResult)]
struct AnalysisVetoRow {
    stock_code: String,
    horizon_decisions: String,
    #[allow(dead_code)]
    created_at: i64,
}

/// 从 `stock_analyses` 加载 30 天内的分析否决表（纯读，无写入）。
///
/// 判定口径（三条过滤器缺一不可）：
/// - `status = 'completed'`：failed/timeout/interrupted 的记录没有可信结论；
/// - `analysis_kind = 'live'` + `synthetic = 0`：回放/补分析样本不构成实盘否决
///   （与 `reco_loop` 样本层的代际闸门同一裁定）；
/// - `horizon_decisions IS NOT NULL`：**只用四周期独立决策**判档位动作 —— 主档
///   `decision_action` 可能来自超短档（选档规则），拿它否决长线出票就是张冠李戴。
///   存量记录该列为 NULL ⇒ 天然无否决（采集时点没有该信息，不得脑补）。
///
/// 同一 code 只保留**最新**一条分析的表态（按 `created_at` 降序取首见）：
/// 最新分析判「观望」⇒ 旧「卖出」不得翻案 —— 回环跟人走，改判即解锁。
pub async fn load_analysis_veto_var(
    db: &DatabaseConnection,
) -> Result<Option<serde_json::Value>, String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let since = now_ms - ANALYSIS_VETO_WINDOW_DAYS * 24 * 60 * 60 * 1000;
    let rows = stock_analyses::Entity::find()
        .select_only()
        .column(stock_analyses::Column::StockCode)
        .column(stock_analyses::Column::HorizonDecisions)
        .column(stock_analyses::Column::CreatedAt)
        .filter(stock_analyses::Column::Status.eq("completed"))
        .filter(stock_analyses::Column::AnalysisKind.eq("live"))
        .filter(stock_analyses::Column::Synthetic.eq(0))
        .filter(stock_analyses::Column::HorizonDecisions.is_not_null())
        .filter(stock_analyses::Column::CreatedAt.gte(since))
        .order_by_desc(stock_analyses::Column::CreatedAt)
        .into_model::<AnalysisVetoRow>()
        .all(db)
        .await
        .map_err(|e| format!("查询 stock_analyses 否决投影失败: {e}"))?;
    Ok(veto_var_from_rows(rows))
}

/// 行集 → 否决变量（纯函数，供单测直接喂构造行）。
///
/// 输出形态：`[{"code":"002696","mid":"减持","long":"卖出"}, ...]` —— **只含**有
/// 卖出/减持档位的条目；一条否决都没有 ⇒ `None`（消费侧对 `None` 与空数组同判不拦）。
fn veto_var_from_rows(rows: Vec<AnalysisVetoRow>) -> Option<serde_json::Value> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for r in rows {
        // 行已按 created_at 降序 ⇒ 首见即最新。
        if !seen.insert(r.stock_code.clone()) {
            continue;
        }
        let Ok(hd) = serde_json::from_str::<serde_json::Value>(&r.horizon_decisions) else {
            // 损坏快照不是否决证据：跳过（与 blackboard_snapshot 三态表同一处置取向）。
            continue;
        };
        let mut entry = serde_json::Map::new();
        entry.insert("code".to_string(), serde_json::Value::String(r.stock_code));
        for tier in ["mid", "long"] {
            let Some(action) = hd.get(tier).and_then(|t| t.get("action")).and_then(|v| v.as_str())
            else {
                continue;
            };
            if action == "卖出" || action == "减持" {
                entry.insert(tier.to_string(), serde_json::Value::String(action.to_string()));
            }
        }
        if entry.len() > 1 {
            entries.push(serde_json::Value::Object(entry));
        }
    }
    if entries.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(entries))
    }
}

// ── 分析侧自动定档 → 荐股（2026-10-19，用户裁定）────────────────────────────
//
// 事实依据：002696 在 `stock_analyses` 里有一条 as_of=2026-09-28 的 completed 分析，
// `decision_horizon_source=branch_pick`、`decision_time_horizon=mid`、mid 档结论「持有」——
// 但荐股侧从不消费它（唯一消费是 `gates` 的**否决**），于是这只票永远进不了任何一档的候选。
// 本变量把「该票归哪一档 + 该档结论」交给 `WatchlistStrategy` 这类消费方按档归位。
//
// ⚠ 代际过滤必须由调用方**显式传日期**：served_vars 在 as_of scope 之外加载，
//   那一刻 `current_as_of()` 恒为 None ⇒ 加载侧自判代际是假守卫（与 veto 那条同一教训）。
//   live 传 None（`as_of_date IS NULL` + 30 天窗口）；回放传截止日（`as_of_date = 该日`）。

#[derive(Debug, FromQueryResult)]
struct AnalysisTierRow {
    stock_code: String,
    horizon_decisions: Option<String>,
    as_of_date: Option<String>,
}

/// 加载「逐档结论」变量：`{ "<code>": { "ultra_short"|"short"|"mid"|"long":
/// {action, confidence}, "asOfDate": "YYYY-MM-DD" } }`。
///
/// **形态为什么是「每档一份」而不是「选一个主档」**（2026-10-20 改正）：四周期是四个
/// **相互独立**的预测 —— 同一只票完全可以「超短判卖出、中线判持有」。旧实现只取
/// `decision_time_horizon`（按置信度选出的“主档”）并把它当成该票的归属档，
/// 于是另外三档即使自己判了「买入」也永远进不去 ⇒ 这不叫按档归位，叫单档指派。
/// 现在把四档结论整份交给消费方，由**本档**自己的结论决定去留（`watchlist::apply_analysis_tier`）。
///
/// 口径：`completed` 行、取每个 code 最新一条（`created_at` 降序首见），
/// **不过滤 `synthetic`** —— 回放代的分析正是这一代唯一正确的证据；实盘代按
/// `analysis_kind='live'` + 30 天窗口收口。一条可用结论都没有 ⇒ `None`。
pub async fn load_analysis_tier_var(
    db: &DatabaseConnection,
    generation: Option<&str>,
) -> Result<Option<serde_json::Value>, String> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

    let mut q = stock_analyses::Entity::find()
        .select_only()
        .column(stock_analyses::Column::StockCode)
        .column(stock_analyses::Column::HorizonDecisions)
        .column(stock_analyses::Column::AsOfDate)
        .filter(stock_analyses::Column::Status.eq("completed"))
        // ⚠ **不要**在这里加 `synthetic = 0`：定档与 veto 是两种口径。
        //   veto（上面那条）问的是「今天的实盘荐股该不该被一份回放样本否决」⇒ 必须排除
        //   replay/synthetic 行；定档问的是「**这一代**这只票各档被判定成什么」⇒ 该代的
        //   replay 分析正是唯一正确的证据。2026-10-19 实测：002696 的 as_of=09-28 分析是
        //   `analysis_kind='replay'`、`synthetic=1`，复制 veto 的 `synthetic=0` 把它滤掉后，
        //   定档变量里根本没有这只票 ⇒「有定档的先扫」与「按档归位」对它是空转，
        //   库里连一行 pick 都没有。实盘代那支在下面按 `analysis_kind='live'` 收口即可。
        .filter(stock_analyses::Column::HorizonDecisions.is_not_null());
    q = match generation {
        Some(day) => q.filter(stock_analyses::Column::AsOfDate.eq(day)),
        None => {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let since = now_ms - ANALYSIS_VETO_WINDOW_DAYS * 24 * 60 * 60 * 1000;
            q.filter(stock_analyses::Column::AsOfDate.is_null())
                .filter(stock_analyses::Column::AnalysisKind.eq("live"))
                .filter(stock_analyses::Column::CreatedAt.gte(since))
        },
    };
    let rows = q
        .order_by_desc(stock_analyses::Column::CreatedAt)
        .into_model::<AnalysisTierRow>()
        .all(db)
        .await
        .map_err(|e| format!("查询 stock_analyses 定档投影失败: {e}"))?;
    Ok(tier_var_from_rows(rows))
}

/// 行集 → 逐档结论变量（纯函数，供单测直接喂构造行）。行已按 created_at 降序 ⇒ 首见即最新。
///
/// 每只票输出**本行里出现的全部档位**（可能 1~4 个），消费方按自己那一档取用 ——
/// 不再按「主档」二选一（见 `load_analysis_tier_var` 的文档）。
fn tier_var_from_rows(rows: Vec<AnalysisTierRow>) -> Option<serde_json::Value> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut map = serde_json::Map::new();
    for r in rows {
        if !seen.insert(r.stock_code.clone()) {
            continue;
        }
        let Ok(hd) = serde_json::from_str::<serde_json::Value>(
            r.horizon_decisions.as_deref().unwrap_or("null"),
        ) else {
            continue;
        };
        let mut tiers = serde_json::Map::new();
        for period in Period::ALL {
            let Some(tier) = horizon_tier_row(&hd, period.as_str()) else {
                continue;
            };
            let action = tier.get("action").and_then(|v| v.as_str()).unwrap_or("");
            if action.is_empty() {
                continue;
            }
            let confidence = tier.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
            tiers.insert(
                period.as_str().to_string(),
                serde_json::json!({ "action": action, "confidence": confidence }),
            );
        }
        if tiers.is_empty() {
            continue;
        }
        tiers.insert(
            "asOfDate".to_string(),
            serde_json::Value::String(r.as_of_date.unwrap_or_default()),
        );
        map.insert(r.stock_code, serde_json::Value::Object(tiers));
    }
    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map))
    }
}

/// 按**两种键族**取该档的决策行：snake（历史存档与 `Period::as_str()`）与
/// camelCase（v125 起 `horizon_decisions` 的生产键）。
///
/// 必须两侧都试，因为四档里**只有超短档两族不同形**（`ultra_short` vs `ultraShort`），
/// short/mid/long 恰好同名 ⇒ 单按 snake 读不会在另外三档露馅。实测（2026-10-20）：
/// 库里 48 条 `decision_time_horizon='ultra_short'` 的 completed 分析，
/// 48/48 的结论挂在 `ultraShort` 下、snake 键 0 条 ⇒ 只读 snake 会把超短档结论
/// **全部**读成缺席，而超短恰是现网占比最高的一档（10-19 那天 15/20）。
/// 键域权威 = [`axagent_harness::holding_period::Period::camel_key`]，与本仓其余
/// 逐档读点（`leg_weight_overlay`、`decision.rs`）同源。
fn horizon_tier_row<'a>(hd: &'a serde_json::Value, horizon: &str) -> Option<&'a serde_json::Value> {
    hd.get(horizon).filter(|t| !t.is_null()).or_else(|| {
        horizon.parse::<Period>().ok().and_then(|p| hd.get(p.camel_key()).filter(|t| !t.is_null()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflection_stats::IC_MIN_SAMPLE;

    fn loop_sample(
        style_key: &'static str,
        period: Period,
        confidence: f64,
        win: bool,
    ) -> RecoLoopSample {
        RecoLoopSample {
            style_key,
            period,
            exit_at_ms: 0,
            predicted_at_ms: 0,
            win,
            confidence,
            realized_return_pct: 0.0,
            holding_days: period.default_holding_days(),
            dqi_fused_at_prediction: None,
            pick_id: "test-pick".to_string(),
            reco_version: Some(5),
        }
    }

    #[test]
    fn bucket_index_maps_scores_to_five_buckets() {
        assert_eq!(evidence_bucket_index(0.0), 0);
        assert_eq!(evidence_bucket_index(19.9), 0);
        assert_eq!(evidence_bucket_index(45.0), 2);
        assert_eq!(evidence_bucket_index(100.0), 4);
        // 越界防御：>100 不落出桶集
        assert_eq!(evidence_bucket_index(130.0), 4);
    }

    #[test]
    fn bucket_calibration_requires_min_samples() {
        let mut samples: Vec<RecoLoopSample> = (0..19)
            .map(|i| loop_sample("trend", Period::Long, 50.0 + f64::from(i % 3), true))
            .collect();
        // 19 条同桶（45~59）< 20 ⇒ None（显式样本不足，不回落合成值）
        assert_eq!(
            bucket_calibrated_win_rate(
                &samples,
                "trend",
                Period::Long,
                50.0,
                5,
                EVIDENCE_BUCKET_MIN_SAMPLES
            ),
            None
        );
        // 第 20 条补足 ⇒ 出真统计
        samples.push(loop_sample("trend", Period::Long, 50.0, false));
        let (wr, n) = bucket_calibrated_win_rate(
            &samples,
            "trend",
            Period::Long,
            50.0,
            5,
            EVIDENCE_BUCKET_MIN_SAMPLES,
        )
        .expect("20 条达标");
        assert_eq!(n, 20);
        // 19 win + 1 loss ⇒ 0.95
        assert!((wr - 0.95).abs() < 1e-9, "实得 {wr}");
    }

    #[test]
    fn bucket_calibration_filters_generation_and_cell() {
        let mut samples = vec![
            // 旧代（v4 伪概率常数 60）同格 ⇒ 代际闸门必须排除（60 落桶 3，即便同桶也拦）
            loop_sample("trend", Period::Long, 60.0, true),
        ];
        samples[0].reco_version = Some(4);
        // 新代同桶同格 1 条
        samples.push(loop_sample("trend", Period::Long, 50.0, true));
        // 旧代样本 < min（1 < 20）即便混入也不计票 ⇒ None
        assert_eq!(bucket_calibrated_win_rate(&samples, "trend", Period::Long, 55.0, 5, 2), None);
        // 补足 2 条新代同桶样本 ⇒ 计票 2
        samples.push(loop_sample("trend", Period::Long, 55.0, false));
        let (wr, n) = bucket_calibrated_win_rate(&samples, "trend", Period::Long, 55.0, 5, 2)
            .expect("2 条新代达标");
        assert_eq!(n, 2, "旧代样本不计入");
        assert!((wr - 0.5).abs() < 1e-9, "实得 {wr}");
        // 其它风格/档位不同格 ⇒ 不计入
        samples.push(loop_sample("value", Period::Long, 50.0, true));
        samples.push(loop_sample("trend", Period::Mid, 50.0, true));
        let (_, n2) = bucket_calibrated_win_rate(&samples, "trend", Period::Long, 55.0, 5, 2)
            .expect("仍达标");
        assert_eq!(n2, 2, "跨格样本不计入");
    }

    fn veto_row(code: &str, horizon_json: &str) -> AnalysisVetoRow {
        AnalysisVetoRow {
            stock_code: code.to_string(),
            horizon_decisions: horizon_json.to_string(),
            created_at: 0,
        }
    }

    #[test]
    fn veto_var_keeps_latest_per_code_and_only_exit_tiers() {
        // 行按 created_at 降序传入（调用方保证）；首见即最新。
        let rows = vec![
            veto_row("002696", r#"{"long":{"action":"观望"},"mid":{"action":"观望"}}"#),
            // 旧账「卖出」不得翻新判「观望」的案 —— 首见（更新）已表态。
            veto_row("002696", r#"{"long":{"action":"卖出"}}"#),
            veto_row("600519", r#"{"long":{"action":"卖出"},"mid":{"action":"持有"}}"#),
            veto_row("300750", r#"{"long":{"action":"买入"}}"#),
            veto_row("000001", "not-json"),
        ];
        let var = veto_var_from_rows(rows).expect("有否决条目");
        let arr = var.as_array().expect("数组形态");
        assert_eq!(
            arr.len(),
            1,
            "只应有 600519 一条（002696 最新无否决/300750 无卖出/000001 损坏）"
        );
        assert_eq!(arr[0]["code"], "600519");
        assert_eq!(arr[0]["long"], "卖出");
        assert!(arr[0].get("mid").is_none(), "「持有」档不进否决表");
    }

    #[test]
    fn veto_var_none_when_no_exit_verdicts() {
        let rows = vec![veto_row("002696", r#"{"long":{"action":"观望"}}"#)];
        assert!(veto_var_from_rows(rows).is_none());
        assert!(veto_var_from_rows(Vec::new()).is_none());
    }

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
            // 夹具默认是实盘样本（代际为空）；回放样本另造，见本模块的代际负控测试
            as_of_date: None,
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
            // 夹具默认「归属当代且可追认」：权重线测试不关心代际，代际筛样由 ic.rs 的
            // `cross_generation_samples_are_screened_and_counted_separately` 单独锁住。
            pick_id: format!("p-{style}-{exit_at_ms}"),
            reco_version: Some(crate::recommender::RECO_GENERATION_FLOOR),
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

    /// 样本 < IC_MIN_SAMPLE ⇒ **保留当前权重**（本夹具 `current` 为空 ⇒ 当前=基线 1.0）
    /// 且显式「未校准」（不冒充校准过）。
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

    /// conf 一侧恒等（方差退化）⇒ IC 测不出力度 ⇒ 保留当前权重（夹具=基线 1.0），
    /// 标 IcUnmeasurable。
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

    /// 代际闸门：`as_of_date` 非空的回放样本不得进实盘权重闭环。
    #[test]
    fn replay_samples_do_not_enter_live_loop() {
        let live = dv_row("p-live", "trend", "mid", 28, "hit");
        let mut replay = dv_row("p-replay", "trend", "mid", 28, "hit");
        replay.as_of_date = Some("2026-05-27".to_string());

        // 前置：夹具本身进得了闭环（否则下面两条恒真，测的是别的 continue）
        assert_eq!(
            samples_from_validation_rows(vec![live.clone()]).len(),
            1,
            "前置不成立：实盘样本未被采纳"
        );
        // 混入一条回放样本 ⇒ 仍 1 条（被吃进就变 2）
        assert_eq!(
            samples_from_validation_rows(vec![live, replay.clone()]).len(),
            1,
            "回放样本混进了实盘闭环"
        );
        // 另一头：只给回放样本 ⇒ 0 条
        assert_eq!(samples_from_validation_rows(vec![replay]).len(), 0);
    }
}

#[cfg(test)]
mod tier_var_tests {
    use super::*;

    fn row(code: &str, hd: Option<&str>, as_of: Option<&str>) -> AnalysisTierRow {
        AnalysisTierRow {
            stock_code: code.into(),
            horizon_decisions: hd.map(str::to_string),
            as_of_date: as_of.map(str::to_string),
        }
    }

    /// 三格：同一只票取**最新**一行（行序即 created_at 降序）、损坏/四档全缺的行跳过、
    /// 四档结论**整份携带**（不再二选一成“主档”）、输出键名与消费方
    /// （`watchlist::apply_analysis_tier`）逐字一致（snake）。
    #[test]
    fn tier_var_carries_every_horizon_of_the_newest_row() {
        let rows = vec![
            row(
                "002696",
                Some(
                    r#"{"ultraShort":{"action":"卖出","confidence":19.9},
                       "short":{"action":"卖出","confidence":22.3},
                       "mid":{"action":"持有","confidence":60.5},
                       "long":{"action":"卖出","confidence":24.5}}"#,
                ),
                Some("2026-09-28"),
            ),
            // 更旧的一行（行序在后 = created_at 更低）不得覆盖上面那条
            row(
                "002696",
                Some(r#"{"mid":{"action":"卖出","confidence":50.0}}"#),
                Some("2026-10-18"),
            ),
            row("600001", Some("{not json"), Some("2026-09-28")),
            row("600002", Some(r#"{"mid":{"confidence":60.0}}"#), Some("2026-09-28")),
            row("600003", Some("{}"), None),
        ];
        let v = tier_var_from_rows(rows).expect("至少 002696 一条可用");
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 1, "损坏/无动作/四档全缺的行不得进变量");
        let e = &obj["002696"];
        assert_eq!(e["ultra_short"]["action"], "卖出", "超短档结论必须带上（camel→snake 归一）");
        assert_eq!(e["short"]["action"], "卖出");
        assert_eq!(e["mid"]["action"], "持有", "同一只票必须以最新一行为准");
        assert_eq!(e["long"]["action"], "卖出");
        assert!((e["mid"]["confidence"].as_f64().unwrap() - 60.5).abs() < 1e-9);
        assert_eq!(e["asOfDate"], "2026-09-28");
        assert!(e.get("horizon").is_none(), "不再输出“主档”字段：四档独立，消费方按自己那一档取用");

        // 负控：全不可用 ⇒ None（消费侧据此保持原行为，而不是把所有票当成已有结论）
        assert!(tier_var_from_rows(vec![row("600001", Some("{not json"), None)]).is_none());
    }

    /// 生产键族是 **camelCase**（v125 起）：`{"ultraShort":{…}}` 必须读得到。
    /// 现网实测 48/48 条超短结论就是这个形态、snake 键 0 条 —— 只按 snake 读会让
    /// 超短档结论整批消失（四档里只有超短两族拼写不同，另外三档从不露馅）。
    #[test]
    fn camel_keyed_ultra_short_verdict_is_readable() {
        let rows = vec![row(
            "600123",
            Some(r#"{"ultraShort":{"action":"买入","confidence":71.2}}"#),
            Some("2026-10-19"),
        )];
        let v = tier_var_from_rows(rows).expect("camel 键的超短结论必须被认");
        let e = &v["600123"];
        assert_eq!(
            e["ultra_short"]["action"], "买入",
            "输出侧用 snake：消费方按 Period::as_str 比对"
        );
        assert!((e["ultra_short"]["confidence"].as_f64().unwrap() - 71.2).abs() < 1e-9);

        // 负控：该档两族键下都没有 ⇒ 不得拿别的档的结论凑一格
        let only_mid = tier_var_from_rows(vec![row(
            "600124",
            Some(r#"{"mid":{"action":"买入","confidence":70.0},"long":{"action":"买入"}}"#),
            None,
        )])
        .expect("mid/long 两档仍应进变量");
        assert!(
            only_mid["600124"].get("ultra_short").is_none(),
            "只有 mid/long 结论的行不能凭空补出超短档"
        );
    }
}

/// 阶段 3 的三条硬判据（样本门 / 只降不升 / 损坏行跳过）。
///
/// 刻意独立成 mod 而不是塞进上面的 `tests`：那三组测的是样本归一化与 IC 分桶，
/// 本组只测**权重合成**，判据互不牵连，失败时一眼能归因。
#[cfg(test)]
mod weight_ratchet_tests {
    use super::*;
    use crate::reflection_stats::IC_MIN_SAMPLE;

    const NOW: i64 = 1_700_000_000_000;

    fn sample(
        style: &'static str,
        period: Period,
        confidence: f64,
        ret: f64,
        win: bool,
        exit_at_ms: i64,
    ) -> RecoLoopSample {
        RecoLoopSample {
            style_key: style,
            period,
            exit_at_ms,
            predicted_at_ms: exit_at_ms - 86_400_000,
            win,
            confidence,
            realized_return_pct: ret,
            holding_days: period.default_holding_days(),
            dqi_fused_at_prediction: None,
            pick_id: format!("p-{style}-{exit_at_ms}-{confidence}"),
            reco_version: Some(crate::recommender::RECO_GENERATION_FLOOR),
        }
    }

    fn cell_of<'a>(res: &'a [LoopCellResult], style: &str, period: &str) -> &'a LoopCellResult {
        res.iter().find(|r| r.style == style && r.period == period).expect("缺格")
    }

    /// 样本门：窗口内可用样本 < `IC_MIN_SAMPLE` ⇒ 胜率/IC 线不动。
    /// 关键断言是 `new == old`（**不是** 1.0）：旧实现回退基线会把历史降权洗掉。
    #[test]
    fn insufficient_samples_keep_current_weight_not_baseline() {
        let samples: Vec<_> = (0..IC_MIN_SAMPLE - 1)
            .map(|i| sample("trend", Period::Short, 60.0 + i as f64, i as f64, i % 2 == 0, NOW))
            .collect();
        let mut current = HashMap::new();
        current.insert(("trend".to_string(), "short".to_string()), 0.42);
        let res = compute_loop_cell_weights(&samples, &current, NOW, &MoverCellSignal::default());
        let c = cell_of(&res, "trend", "short");
        assert_eq!(c.status, LoopCellStatus::InsufficientSamples, "{c:?}");
        assert_eq!(c.samples, IC_MIN_SAMPLE - 1);
        assert_eq!(c.old_weight, 0.42);
        assert!(
            (c.new_weight - 0.42).abs() < 1e-12,
            "样本不足必须保持当前权重 0.42，实际 {}（回退 1.0 即旧缺陷复现）",
            c.new_weight
        );
    }

    /// 零样本 + 当前已被降权 ⇒ 仍保持该权重（不得被抬回基线）。
    #[test]
    fn empty_cell_never_lifts_previous_downgrade() {
        let current = HashMap::from([(("trend".to_string(), "short".to_string()), 0.16_f64)]);
        let res = compute_loop_cell_weights(&[], &current, NOW, &MoverCellSignal::default());
        let c = cell_of(&res, "trend", "short");
        assert_eq!(c.status, LoopCellStatus::InsufficientSamples);
        assert!((c.new_weight - 0.16).abs() < 1e-12, "实际 {}", c.new_weight);
    }

    /// IC 方差退化（conf 恒等）⇒ `IcUnmeasurable`，同样保留当前权重。
    #[test]
    fn unmeasurable_ic_keeps_current_weight() {
        let samples: Vec<_> = (0..IC_MIN_SAMPLE)
            .map(|i| sample("trend", Period::Mid, 60.0, i as f64, true, NOW))
            .collect();
        let current = HashMap::from([(("trend".to_string(), "mid".to_string()), 0.31_f64)]);
        let res = compute_loop_cell_weights(&samples, &current, NOW, &MoverCellSignal::default());
        let c = cell_of(&res, "trend", "mid");
        assert_eq!(c.status, LoopCellStatus::IcUnmeasurable, "{c:?}");
        assert!((c.new_weight - 0.31).abs() < 1e-12, "实际 {}", c.new_weight);
    }

    /// 只降不升：胜率/IC 线算出的权重高于当前值 ⇒ 棘轮必须压回当前值。
    ///
    /// 夹具用「全 win + conf/ret 同向」把 `compute_adjusted_weights` 的 EWMA 结果推到
    /// 当前值之上（raw ≈ 0.5 > 0.2），否则这条断言测不到棘轮分支。
    #[test]
    fn ratchet_never_raises_above_current_weight() {
        let samples: Vec<_> = (0..IC_MIN_SAMPLE)
            .map(|i| sample("trend", Period::Short, 10.0 + i as f64 * 10.0, i as f64, true, NOW))
            .collect();
        let current = HashMap::from([(("trend".to_string(), "short".to_string()), 0.2_f64)]);
        let res = compute_loop_cell_weights(&samples, &current, NOW, &MoverCellSignal::default());
        let c = cell_of(&res, "trend", "short");
        assert_eq!(c.status, LoopCellStatus::IcNonNegative, "{c:?}");
        assert!(c.new_weight <= 0.2 + 1e-12, "只降不升：不得从 0.20 抬高到 {}", c.new_weight);

        // 纯函数边界：非有限输入、越界输入都不得产生 NaN / 越过当前值
        assert_eq!(ratchet_weight(0.99, 0.2), 0.2);
        assert_eq!(ratchet_weight(0.2, 0.2), 0.2);
        assert_eq!(ratchet_weight(0.01, 0.2), WEIGHT_FLOOR, "下限钳位");
        assert_eq!(ratchet_weight(f64::NAN, 0.2), 0.2, "NaN 计算值 ⇒ 保持当前权重");
        assert_eq!(ratchet_weight(0.3, f64::NAN), 0.3, "NaN 当前值 ⇒ 按基线处理");
    }

    /// 损坏行跳过：NaN 置信度 / NaN 收益 / 越界置信度 / 负时间戳一律不进分母，
    /// 且不得把整格权重污染成 NaN。
    #[test]
    fn corrupt_samples_are_skipped_not_counted() {
        let mut samples: Vec<_> = (0..IC_MIN_SAMPLE)
            .map(|i| sample("trend", Period::Short, 10.0 + i as f64 * 10.0, i as f64, true, NOW))
            .collect();
        let mut nan_conf = sample("trend", Period::Short, 50.0, 1.0, true, NOW);
        nan_conf.confidence = f64::NAN;
        let mut nan_ret = sample("trend", Period::Short, 50.0, 1.0, true, NOW);
        nan_ret.realized_return_pct = f64::NAN;
        let mut out_of_range = sample("trend", Period::Short, 50.0, 1.0, true, NOW);
        out_of_range.confidence = 150.0;
        let mut bad_time = sample("trend", Period::Short, 50.0, 1.0, true, NOW);
        bad_time.exit_at_ms = -1;

        for (i, bad) in [nan_conf, nan_ret, out_of_range, bad_time].into_iter().enumerate() {
            assert!(!sample_is_usable(&bad), "第 {i} 条损坏样本未被判为不可用");
            samples.push(bad);
        }

        let res =
            compute_loop_cell_weights(&samples, &HashMap::new(), NOW, &MoverCellSignal::default());
        let c = cell_of(&res, "trend", "short");
        assert_eq!(c.samples, IC_MIN_SAMPLE, "损坏行不得进分母");
        assert!(c.new_weight.is_finite(), "权重被污染成 {}", c.new_weight);
        assert!((0.0..=BASELINE_WEIGHT).contains(&c.new_weight), "越界 {}", c.new_weight);
    }

    /// 样本不足时，mover 线（有自己的样本门）仍可独立降权；但不得提权。
    #[test]
    fn mover_line_still_lowers_an_insufficient_cell() {
        let samples: Vec<_> = (0..IC_MIN_SAMPLE - 1)
            .map(|i| sample("trend", Period::Short, 60.0 + i as f64, i as f64, true, NOW))
            .collect();
        let current = HashMap::from([(("trend".to_string(), "short".to_string()), 0.9_f64)]);
        let mut sig = MoverCellSignal::default();
        sig.cells.insert(("trend".into(), "short".into()), (9, 0));
        let res = compute_loop_cell_weights(&samples, &current, NOW, &sig);
        let c = cell_of(&res, "trend", "short");
        assert_eq!(c.status, LoopCellStatus::InsufficientSamples);
        assert_eq!(c.mover_penalty, Some(0.1));
        // 9 次全推出去 ⇒ Laplace 转化率 0.1 ⇒ 0.9×0.1 = 0.09
        assert!((c.new_weight - 0.09).abs() < 1e-9, "实际 {}", c.new_weight);
        assert!(c.new_weight < c.old_weight, "独立降权线必须生效");
    }
}

/// 静态权重表 × 闭环权重的合并判据（2026-10-20：整表替换会静默删掉用户校准格）。
#[cfg(test)]
mod merge_weights_tests {
    use super::*;

    fn static_map(pairs: &[(&str, f64)]) -> serde_json::Map<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), serde_json::json!(v))).collect()
    }

    /// 闭环只管它算过的格：静态表里其余格必须原样保留。
    /// 负控：如果实现退回「整表替换」，`value_mid` 会消失 —— 本测试当场红。
    #[test]
    fn loop_overlay_keeps_static_cells_it_never_touched() {
        let statics = static_map(&[("trend_short", 0.7), ("value_mid", 0.8)]);
        let looped = HashMap::from([(("trend".to_string(), "short".to_string()), 0.4_f64)]);
        let merged = merge_static_and_loop_weights(statics, &looped);

        assert_eq!(merged["trend_short"], serde_json::json!(0.4), "闭环写过的格必须覆盖");
        assert_eq!(
            merged["value_mid"],
            serde_json::json!(0.8),
            "闭环没写过的静态格被删掉了（整表替换缺陷）"
        );
        assert_eq!(merged.len(), 2, "不得新增/丢失键: {merged:?}");
    }

    /// 闭环值读取侧夹到 ≤1.0；静态值本身不做夹取（模板里原有的 >1 值不在本判据的管辖内）。
    #[test]
    fn loop_overlay_clamps_values_above_baseline() {
        let statics = static_map(&[("capital_long", 0.3)]);
        let looped = HashMap::from([(("capital".to_string(), "long".to_string()), 1.2_f64)]);
        let merged = merge_static_and_loop_weights(statics, &looped);
        assert_eq!(merged["capital_long"], serde_json::json!(1.0), ">1.0 的闭环行必须在读取侧夹回");
    }

    /// 闭环为空 ⇒ 静态表逐字保留（调用方虽然会早返回，纯函数自身也必须正确）。
    #[test]
    fn empty_loop_leaves_static_weights_untouched() {
        let statics = static_map(&[("trend_short", 0.7)]);
        let merged = merge_static_and_loop_weights(statics.clone(), &HashMap::new());
        assert_eq!(merged, statics);
    }
}
