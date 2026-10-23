// SPDX-License-Identifier: AGPL-3.0-only

//! 反思命中率聚合（PLAN-stock-decision-hitrate-validation M2）——
//! 把已落库的决策验证数据聚合成可读的命中率基线。
//!
//! ## 数据源分工（避免与 hit_rate_backtest.rs 重复）
//! - [`crate::hit_rate_backtest`]（V55）：消费 `reco_picks` + 实时行情 API，对**未落库**的决策做回溯验证。
//! - 本模块：消费**已落库**的 `strategy_performance`（M1 确定性判定 + 复盘 win/loss 写回），
//!   按决策方向（action）与时间维度（horizon）聚合，产出方向命中率 / 价位命中率 / 平均收益 / alpha。
//!
//! ## 诚实性铁律
//! 可判样本 < [`MIN_SAMPLE`] 时 `direction_hit_rate` 返回 `None`（前端显示「样本不足」），不产出伪结论。
//! 无法判定的记录（无行情 / 中性档 / 期中观察）在落库层**不写行**，天然不进命中率分子。

use std::collections::HashMap;

use crate::decision_action::normalize_action;
use axagent_entities::{stock_analyses, stock_reflections, strategy_performance};
use axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR;
use axagent_harness::Period;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};

/// 命中率的最低样本数：低于此值不产出命中率数值（诚实性铁律）
pub const MIN_SAMPLE: usize = 5;

/// rank IC 的最低样本数。
///
/// 比 [`MIN_SAMPLE`] 高一档是**刻意的**：命中率是两个计数的比，5 条已能给出可读区间；
/// 相关系数是四阶矩量，n=5 的抽样噪声就能把 ρ 推到 ±0.8，报出来比不报更误导。
/// 阈值宁可提供「样本不足」这一诚实缺席，也不产出无法与噪声区分的数字。
pub const IC_MIN_SAMPLE: usize = 8;

/// 置信度**校准可得性**的最低本档样本数（P1-3，`PLAN-stock-decision-reliability.md` R3/R7）。
///
/// 为什么与上面两个分列：那两个管「某个统计量算不算得出」，本常量管「`confidence → 实际胜率`
/// 这条映射可不可信」。个位数样本上拟合出来的可靠性曲线是一条过拟合阶梯，拿它去压 Kelly
/// 输入比不校准更误导 —— 所以未达标时不做映射，改由消费端按最保守上限封顶（诚实缺席）。
///
/// ⚠ 达标后该标志不会自动带来校准：映射本身（isotonic/Platt）仍待实施，
/// 落点见 [`crate::horizon_prior::horizon_prior_map`] 的 `calibrationReady` 与 PLAN 的 P1-3 行。
pub const CALIBRATION_MIN_SAMPLES: usize = 30;

/// 判定口径水印的锚定天数（与 `portfolio-mgr.rhai` 的 `SNR_ANCHOR_DAYS` 同值）。
/// 引擎侧只把它当「有没有」的水印用，不参与任何数值计算。
pub const SNR_ANCHOR: i64 = 28;

/// 四周期反思状态。`Immature` 与 `Unavailable` 不得产生确定性胜负结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizonStatus {
    Mature,
    Immature,
    Unavailable,
    Legacy,
}

/// 从 `stock_analyses.horizon_decisions` 解析出的单周期决策。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonDecision {
    pub action: String,
    #[serde(alias = "position_pct")]
    pub position_pct: Option<f64>,
    pub confidence: Option<f64>,
    #[serde(alias = "expected_holding_days")]
    pub expected_holding_days: Option<i64>,
    #[serde(alias = "target_price")]
    pub target_price: Option<f64>,
    #[serde(alias = "stop_loss")]
    pub stop_loss: Option<f64>,
    #[serde(alias = "conf_lower_bound")]
    pub conf_lower_bound: Option<f64>,
    /// **判定口径水印**（Phase D-2 起才有，值 = 该次决策 SNR 折算的锚定天数）。
    ///
    /// 为什么必须一路带进反思 JSON：v104 之前逐档 `confidence` = 生效后验，v104 起
    /// = `0.5+(生效后验−0.5)·√(h/28)`。两者**跨版本混在同一档里会改变排序**（同一档内
    /// 各自单调，但两代的刻度不同），rank IC 因此被污染。有水印才允许进 IC 分母，
    /// 没水印就是「上一代口径的记录」，如实标出来而不是默默混算。
    #[serde(alias = "snr_anchor_days", default)]
    pub snr_anchor_days: Option<i64>,
}

/// 兼容主周期所需的最小反思结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonResult {
    pub status: HorizonStatus,
    pub horizon: String,
    pub expected_holding_days: i64,
    pub action: String,
}

/// 单周期客观评价的最小结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonEvaluation {
    pub status: HorizonStatus,
    pub was_correct: Option<i32>,
    pub return_pct: Option<f64>,
}

/// 解析四周期决策 JSON 对象。
pub fn parse_horizon_decisions(
    source: &serde_json::Map<String, serde_json::Value>,
) -> Result<HashMap<String, HorizonDecision>, String> {
    source
        .iter()
        .map(|(horizon, value)| {
            let decision = serde_json::from_value::<HorizonDecision>(value.clone())
                .map_err(|error| format!("解析 horizon_decisions.{horizon} 失败: {error}"))?;
            Ok((horizon.clone(), decision))
        })
        .collect()
}

/// 为没有四周期输出的历史记录构造 legacy 主周期结果。
pub fn legacy_horizon_result(
    action: Option<&str>,
    horizon: Option<&str>,
    expected_holding_days: Option<i64>,
) -> Option<HorizonResult> {
    let action = action?.to_string();
    let horizon = horizon?.to_string();
    let expected_holding_days = expected_holding_days?;
    Some(HorizonResult { status: HorizonStatus::Legacy, horizon, expected_holding_days, action })
}

/// 四周期缺省期望持有期（交易日）。
///
/// 天数从 `Period::default_holding_days` 取（唯一权威源，见
/// `axagent_harness::holding_period` 模块头）；本函数只做「周期字符串 → 档位」的解析，
/// 不再手抄一份天数表。未知/缺失周期沿用中线档（与既有行为一致）。
pub fn default_expected_holding_days(horizon: &str) -> i64 {
    horizon
        .parse::<Period>()
        .map(|p| p.default_holding_days() as i64)
        .unwrap_or(Period::Mid.default_holding_days() as i64)
}

/// 构造行情不可用或未成熟等状态的评价，禁止用 0% 收益冒充事实。
pub fn build_horizon_evaluation(
    status: HorizonStatus,
    was_correct: Option<i64>,
    return_pct: Option<f64>,
) -> HorizonEvaluation {
    let is_mature = status == HorizonStatus::Mature || status == HorizonStatus::Legacy;
    let normalized_was_correct = if is_mature {
        was_correct.and_then(|value| i32::try_from(value).ok())
    } else {
        None
    };
    let normalized_return = if is_mature { return_pct } else { None };
    HorizonEvaluation { status, was_correct: normalized_was_correct, return_pct: normalized_return }
}

/// 按已成熟周期的 action 与净收益符号确定方向命中情况。
pub fn deterministic_horizon_was_correct(
    action: &str,
    return_pct: f64,
    within_expected_horizon: bool,
) -> Option<i32> {
    if within_expected_horizon || !return_pct.is_finite() {
        return None;
    }
    // 方向命中判定走权威谓词。旧写法自写四值表（中英各二），漏 `加仓` / `减仓` /
    // `clear` / `强烈卖出` 等别名 ⇒ 那些记录的 was_correct 恒 None，样本被静默丢弃
    let kind = normalize_action(action)?;
    if kind.implies_buy() {
        Some(i32::from(return_pct > 0.0))
    } else if kind.implies_reduce() {
        Some(i32::from(return_pct < 0.0))
    } else {
        None
    }
}

/// 样本数据源标识 —— 命令层标注该样本来自四周期 JSON 还是旧单周期字段回退。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleDataSource {
    /// 四周期 `horizon_results_json` 中已成熟周期展开而来
    #[default]
    Reflection,
    /// 旧记录无四周期 JSON，经 `original_analysis_id` 回退主周期字段
    Legacy,
}

/// 单条决策表现样本 —— 命令层跨三表（strategy_performance / stock_reflections /
/// stock_analyses）查询并关联好维度后传入，本模块只做纯函数聚合（方便单元测试）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionPerformanceSample {
    /// 决策方向（stock_analyses.decision_action；未识别归 None）
    pub action: Option<String>,
    /// 时间维度（stock_analyses.decision_time_horizon；未识别归 None）
    pub horizon: Option<String>,
    /// 决策是否正确：1 对 / 0 错（M1 确定性判定或复盘 win/loss 写回）
    pub was_correct: i32,
    /// 实际净收益（%）
    pub return_pct: f64,
    /// 相对基准超额收益（%）；仅反思行有
    pub alpha_pct: Option<f64>,
    /// 是否达目标价；仅反思行有（解析自 stock_reflections.blackboard_snapshot）
    pub target_reached: Option<bool>,
    /// 该档决策的置信度 —— Phase E rank IC 的**预测侧**。
    /// 口径与 `portfolio-mgr.rhai` 输出一致：**0–100**（`(hconf*100)` 保留一位小数），
    /// 不是 0–1 概率；IC 是秩相关，量纲只要全体一致就不影响结果。
    /// 只有四周期 JSON 展开的样本有（`horizon_results_json.decision.confidence`）；
    /// legacy 回退路径拿不到逐档置信度，归 None ⇒ 进不了 IC 分母，不冒充「0 置信」。
    #[serde(default)]
    pub confidence: Option<f64>,
    /// 该样本所属的**判定口径水印**（见 [`HorizonDecision::snr_anchor_days`]）。
    /// `None` = 上一代口径（v104 前）的记录 ⇒ 其 confidence 与本档其余样本不同刻度，
    /// **不进 IC 分母**，并被计入 [`HitrateGroup::ic_regime_excluded`] 如实报出。
    #[serde(default)]
    pub snr_anchor_days: Option<i64>,
    /// 该样本所属的**算法代际**（= 被复盘那条分析的 `template_version`，§五十一-②/§七十三）。
    /// 筛选规则是**下限** [`HORIZON_BRANCH_GENERATION_FLOOR`]：`>= floor` 进分母；
    /// 早于 floor 的（`generation_pre_floor`）与 `None`（代际未知）都**排除**并分别计数 ——
    /// 两者处置不同：旧代样本永久不可比，未知代的要等写侧补章/重跑。
    /// 与 `snr_anchor_days` 的区别：那个水印管的是**IC 的置信度刻度**，这个管的是
    /// **全部统计量的分母**（命中率/IC/先验/权重），两者不可互相顶替。
    #[serde(default)]
    pub template_version: Option<i32>,
    /// 数据源标识（四周期展开 / legacy 回退）
    pub data_source: SampleDataSource,
    /// B3（§18）：该档 **LLM 侧**动作原文，产端 = `decision_json.agreementByHorizon[].llmAction`。
    /// 只做溯源/呈现与 `llm_was_correct` 的输入，不参与命中率判定。
    #[serde(default)]
    pub llm_action: Option<String>,
    /// B3：把 [`deterministic_horizon_was_correct`] 用**同一窗口、同一 return_pct、同一分母**
    /// 再跑一次到 LLM 该档动作上的结果。
    /// `None` = 该档 LLM 缺席或窗口未满 ⇒ **不进 2×2 的任何一格**（不得当作「LLM 判错」）。
    #[serde(default)]
    pub llm_was_correct: Option<i32>,
    /// B3：该档三维归一分（65 制 → 100，产端 `agreementByHorizon[].total`）。
    /// ⚠ 与主档五维分（落 `strategy_performance.agreement_score`）不同计数域，不可并排比大小。
    #[serde(default)]
    pub horizon_agreement: Option<i32>,
}

/// 按维度分组的方向命中率与分周期指标
///
/// ⚠ 跨 IPC 边界（`reflection_stats` 命令返回值），字段必须 camelCase 输出 —— 本结构
/// 定义在 `analysis-engine`，而 `check-serde-annotations` 原先只扫 `harness/`+`commands/`，
/// 缺注解不会被报红，前端按 camelCase 读**恒为 undefined**（表现为「暂无数据」而非报错）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitrateGroup {
    /// 分组键：action / horizon 的原始值，缺失归 "unknown"
    pub key: String,
    /// 已判定（成熟）样本数（was_correct ∈ {0,1}）
    pub samples: usize,
    /// 方向命中率；样本 < [`MIN_SAMPLE`] → None（样本不足）
    pub direction_hit_rate: Option<f64>,
    /// 目标价命中率 = target_reached=true / 有目标价判定样本；样本不足 → None
    pub target_hit_rate: Option<f64>,
    /// 该组平均净收益（%）；无样本 → None
    pub avg_raw_return_pct: Option<f64>,
    /// 该组平均超额收益（%）；无 alpha 样本 → None
    pub avg_alpha_pct: Option<f64>,
    /// 该组 rank IC（Spearman ρ：决策置信度 vs 实际净收益）—— Phase E 观测面。
    /// 可判样本 < [`IC_MIN_SAMPLE`] → None（样本不足，不产出与噪声难分的系数）。
    pub rank_ic: Option<f64>,
    /// 参与 IC 计算的样本数（同时有置信度与有限收益者）；与 `samples` 不同口径，
    /// 单独报出是为了让「IC 为 None 是因为缺置信度」与「是因为缺样本」可分辨。
    pub ic_samples: usize,
    /// IC 计算的**口径标注**（Phase F 的结构性缺席原则：不可得必须说清为什么）。
    /// `"ok"` = 正常产出；`"insufficient_ic_samples"` = 有预测值的样本太少；
    /// `"no_confidence"` = 该组一条置信度都没有（legacy 回退样本）；
    /// `"degenerate_variance"` = 样本够但秩无方差（ρ 无定义）；
    /// `"pre_snr_regime"` = 有置信度但**全部缺判定口径水印**（v104 前的旧记录），
    /// 与上一类「没有置信度」是两件事：那要补字段，这要等样本换代累积。
    pub ic_status: String,
    /// 因「缺判定口径水印」被排除出 IC 分母的样本数（换代进度可见：它归零即说明
    /// 该档样本已全部属于当前口径）。
    pub ic_regime_excluded: usize,
    /// 该档的期望持有交易日（半衰期拟合的自变量）；未知档 → None
    pub holding_days: Option<i64>,
}

/// 阶段 3 / 决-3：反思落地层级看板 —— **结构层待裁清单的数据源**。
///
/// 为什么必须有一个活的落点：L2/L3 被 WFO 与定时聚合挡在参数网格之外之后，如果只「排除」
/// 而不「落到某处」，这些反思行就消失了 —— 排除没有出口等于把根因藏得更深。
/// 本看板住在 `HitrateStats`（面板每次打开都读，是**活路径**），而不是
/// `stock_evolution_history.trigger_sample_json`：现网实测 89 行进化计划里该列
/// **0 行非空、且无一行 `pending_approval`**（2026-10-19 取数）⇒ 那条通道当前没有产出，
/// 把待裁项只写在那里就是惰性接线。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TierBoard {
    /// 参与统计的 completed 反思行全集
    pub total_rows: usize,
    /// L1（改参数）——唯一可进参数网格的层级
    pub l1: usize,
    /// L2（改 .rhai 公式 / 算法约束）
    pub l2: usize,
    /// L3（改 .md 提示词）
    pub l3: usize,
    /// NULL = 本次反思**没判**层级（含本列引入前的存量行）。
    /// 不得折算进 `l1` —— 那正是本轴引入前要消除的那类伪造。
    pub unlabelled: usize,
    /// 结构层明细（按更新时间倒序，上限 [`STRUCTURAL_ROWS_CAP`] 条）
    pub structural_rows: Vec<StructuralReflectionRow>,
}

/// 结构层待裁项的一行明细。文本字段在装配处按字符截断（IPC 不带整段快照）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuralReflectionRow {
    pub reflection_id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 该反思复盘的档（NULL = 未记录）
    pub horizon: Option<String>,
    /// `"L2"` / `"L3"`
    pub tier: String,
    /// 建议动的文件/段落描述（反思的 `code_diff_proposal`）
    pub proposal: Option<String>,
    /// 错因原文（截断）
    pub what_went_wrong: Option<String>,
    pub updated_at: i64,
}

/// 明细上限：待裁清单是给人逐条看的，超过这个数就该先看计数而不是往下滚。
const STRUCTURAL_ROWS_CAP: usize = 30;

/// 按字符数截断（中文一字符＝一个字，不按字节切，避免把一个汉字切成半个）。
fn clip_chars(src: &str, max_chars: usize) -> String {
    let mut out: String = src.chars().take(max_chars).collect();
    if src.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// 从 completed 反思行装配层级看板（纯函数，入参已筛过状态）。
pub fn build_tier_board(rows: &[&axagent_entities::stock_reflections::Model]) -> TierBoard {
    use crate::reflection_tier::parse_reflection_tier;
    let mut b = TierBoard { total_rows: rows.len(), ..Default::default() };
    for r in rows {
        match parse_reflection_tier(r.implementation_tier.as_deref()) {
            Some(crate::reflection_tier::ReflectionTier::L1) => b.l1 += 1,
            Some(t) => {
                // L2/L3 ⇒ 既计数也进明细（这就是「待裁」的那一摞）
                if t == crate::reflection_tier::ReflectionTier::L2 {
                    b.l2 += 1;
                } else {
                    b.l3 += 1;
                }
                b.structural_rows.push(StructuralReflectionRow {
                    reflection_id: r.id.clone(),
                    stock_code: r.stock_code.clone(),
                    stock_name: r.stock_name.clone(),
                    horizon: r.horizon.clone(),
                    tier: t.as_str().to_string(),
                    proposal: r
                        .code_diff_proposal
                        .as_deref()
                        .map(|s| clip_chars(s, STRUCTURAL_PROPOSAL_CHARS)),
                    what_went_wrong: r
                        .what_went_wrong
                        .as_deref()
                        .map(|s| clip_chars(s, STRUCTURAL_WRONG_CHARS)),
                    updated_at: r.updated_at,
                });
            },
            None => b.unlabelled += 1,
        }
    }
    // 最近的先处理；同刻按 id 稳定排序（否则同一批数据两次打开顺序不同）
    b.structural_rows.sort_by(|a, c| {
        c.updated_at.cmp(&a.updated_at).then_with(|| a.reflection_id.cmp(&c.reflection_id))
    });
    b.structural_rows.truncate(STRUCTURAL_ROWS_CAP);
    b
}

/// 明细文本截断长度：一屏能读完，又能看出建议到底指向哪个文件。
const STRUCTURAL_PROPOSAL_CHARS: usize = 240;
const STRUCTURAL_WRONG_CHARS: usize = 160;

/// 命中率聚合结果（DTO；前端按 camelCase 消费，见 src/types/stock-analysis.ts）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HitrateStats {
    /// P2-b 失败模式看板（按 completed 反思行的 `failure_modes` 列聚合）。
    /// 计数域与 `miss_layer`（漏检链机械归因）不同，面板不得并列成可比数字。
    pub failure_modes: crate::failure_mode::FailureModeBoard,
    /// 阶段 3 / 决-3：落地层级看板（结构层待裁清单）。
    /// 与 `failure_modes`（错成什么样）、`miss_layer`（漏在哪一层）是三套计数域，
    /// 面板不得把三者并排成可比数字。
    pub tiers: TierBoard,
    /// B4：逐档**结局 2×2**（公式判对否 × LLM 判对否，同窗口、同判定函数）。
    /// 四档恒各一行 —— 没样本的档是 `judged == 0`，**不是**「没有这一档」。
    /// 计数域与 `by_horizon`（只统计公式侧）不同，两者不得并成同一个分母读。
    pub horizon_confusion: Vec<HorizonConfusionRow>,
    /// 参与统计的已结算样本总数
    pub total_samples: usize,
    /// 方向命中率 = was_correct=1 / 已判定样本；样本 < [`MIN_SAMPLE`] → None
    pub direction_hit_rate: Option<f64>,
    /// 价位命中率 = target_reached=true / 有目标价判定样本；样本 < [`MIN_SAMPLE`] → None
    pub target_hit_rate: Option<f64>,
    /// 平均净收益（%）
    pub avg_raw_return_pct: Option<f64>,
    /// 平均超额收益（%）
    pub avg_alpha_pct: Option<f64>,
    /// 样本中来自旧单周期字段回退（`legacy`）的条数
    pub legacy_samples: usize,
    /// 按决策方向分组（含 "unknown"）
    pub by_action: Vec<HitrateGroup>,
    /// 按时间维度分组（含 "unknown"）
    pub by_horizon: Vec<HitrateGroup>,
    /// 预测半衰期（交易日）—— 由逐档 rank IC 的指数衰减拟合导出（[`signal_half_life_days`]）。
    /// None 的三种真实原因由 `usable_ic_tiers` 判别：不足 3 档 / IC 非正 / 拟合不达标。
    /// **观测面**：Phase E 只报数，不回写任何权重。
    pub signal_half_life_days: Option<f64>,
    /// 参与半衰期拟合的档数（有持有期且 rank IC 有值的档）
    pub usable_ic_tiers: usize,
    /// 本次统计采用的**起算代际** = [`HORIZON_BRANCH_GENERATION_FLOOR`]（§五十一-②）。
    /// 回传它是为了让面板能说清「分母为什么变小」——这个数是判据参数，不是运行时状态。
    #[serde(default)]
    pub generation_floor: i32,
    /// 因「早于起算代」被排除出**全部统计分母**的样本数。
    /// 与 `ic_regime_excluded` 的区别：那个只管 IC 这一项，这个管命中率/收益/IC/先验全套。
    #[serde(default)]
    pub excluded_pre_floor_generation: usize,
    /// 代际未知（NULL）的样本数 —— 与上一项分开计数，因为处置不同：
    /// 旧代样本**永远不会**再进分母（算法不可比）；代际未知的
    /// **P2-f 起按收缩纳入**（不再排除，见 `filter_by_generation_floor` 的文档），
    /// 本字段即「其中有多少条是代际未知的」供面板如实报出。
    #[serde(default)]
    pub unknown_generation_samples: usize,
}

/// 命中率：分母为 0 或样本不足 → None
fn hit_rate_or_none(good: usize, total: usize) -> Option<f64> {
    if total == 0 || total < MIN_SAMPLE {
        None
    } else {
        Some(good as f64 / total as f64)
    }
}

/// 均值：空序列 → None
fn mean(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        None
    } else {
        Some(xs.iter().sum::<f64>() / xs.len() as f64)
    }
}

/// 分组聚合中间态（一次遍历同时累加方向 / 目标价 / 收益 / alpha / IC 五类指标）
#[derive(Default)]
struct GroupAggregate {
    /// 已判定样本（was_correct ∈ {0,1}）
    judged: usize,
    /// 方向命中样本（was_correct = 1）
    correct: usize,
    /// 有目标价判定的样本
    target_judged: usize,
    /// 目标价命中样本
    target_hit: usize,
    /// 净收益序列（与顶层同口径，组内全样本）
    returns: Vec<f64>,
    /// 超额收益序列（仅含 alpha_pct 有值的样本）
    alphas: Vec<f64>,
    /// IC 样本对：(决策置信度, 实际净收益%)。缺任一不进此列 —— 宁缺毋伪。
    /// ⚠ 还要求该样本**带判定口径水印**（见 [`DecisionPerformanceSample::snr_anchor_days`]）：
    ///   v104 前后的 confidence 不是同一把尺，混在一档里会把秩排错，进而把 IC 算歪。
    ic_pairs: Vec<(f64, f64)>,
    /// 有置信度但缺水印 ⇒ 因口径换代被排除的条数（计入 `ic_regime_excluded`）。
    ic_regime_excluded: usize,
}

/// rank IC 的唯一实现来自 [`axagent_harness::indicators::spearman_rank_ic`]（秩相关 =
/// `pearson(average_ranks(x), average_ranks(y))`）—— 本仓已有两份同族实现
/// （`hit_rate_backtest` 的 9 因子 IC、`portfolio_monitor` 的 Pearson），
/// 再抄第三份就是禁区 12。此处只 re-export，本模块负责的是**门槛与缺席标注**：
/// 样本数门槛见 [`IC_MIN_SAMPLE`]，缺席原因见 [`HitrateGroup::ic_status`]。
pub use axagent_harness::indicators::spearman_rank_ic;

/// 由逐档 (持有天数, rank IC) 序列拟合**预测半衰期**。
///
/// 模型：IC(h) ≈ IC₀ · e^(−λh) ⇒ 半衰期 t½ = ln2 / λ。对 IC>0 的档取 ln|IC| 对 h
/// 做最小二乘直线拟合（斜率 −λ）。
/// 诚实性（三重，任一不满足即 None，绝不外推）：
/// 1. 至少 3 个有效点 —— 两点总能连出一条线，把「样本噪声」卖成「衰减规律」；
/// 2. 拟合斜率必须 < 0 —— 没有衰减就不该报半衰期（报 None 而不是「无限长」）；
/// 3. 决定系数 R² 必须 ≥ [`HALF_LIFE_MIN_R2`] —— 拟合本身不可信时，给出的天数是
///    装饰而非信息。调用方按 None 显示「无法拟合」。
pub fn signal_half_life_days(points: &[(f64, f64)]) -> Option<f64> {
    let pts: Vec<(f64, f64)> = points
        .iter()
        .filter(|(h, ic)| h.is_finite() && ic.is_finite() && *h > 0.0 && *ic > 0.0)
        .map(|(h, ic)| (*h, ic.ln()))
        .collect();
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    let mut syy = 0.0;
    for (x, y) in &pts {
        sxx += (x - mx) * (x - mx);
        sxy += (x - mx) * (y - my);
        syy += (y - my) * (y - my);
    }
    if sxx <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    if slope >= 0.0 {
        return None;
    }
    // R² = 1 − SSE/SST；一元回归下 SSE = SST − slope²·SXX ⇒ R² = slope²·SXX/SST
    if syy <= 0.0 {
        return None;
    }
    let r2 = (slope * slope * sxx) / syy;
    if r2 < HALF_LIFE_MIN_R2 {
        return None;
    }
    Some(std::f64::consts::LN_2 / -slope)
}

/// 半衰期拟合的最低拟合优度：低于此值说明「天数 → lnIC」根本不成直线，
/// 报出来的半衰期是拟合噪声而非衰减。
pub const HALF_LIFE_MIN_R2: f64 = 0.5;

/// 按 key 分组统计方向命中率、分周期指标与 rank IC（保留首次出现的顺序，避免 HashMap 无序）。
/// 只统计已判定样本（was_correct ∈ {0,1}）；未判定样本不进任何分组。
///
/// `days_of` 只在 horizon 分组下有值（半衰期拟合的自变量）；action 分组传 `|_| None`。
fn group_hit_rates<F, G>(
    samples: &[DecisionPerformanceSample],
    key_of: F,
    days_of: G,
) -> Vec<HitrateGroup>
where
    F: Fn(&DecisionPerformanceSample) -> String,
    G: Fn(&str) -> Option<i64>,
{
    let mut order: Vec<String> = Vec::new();
    let mut agg: HashMap<String, GroupAggregate> = HashMap::new();
    for s in samples {
        if s.was_correct != 0 && s.was_correct != 1 {
            continue;
        }
        let key = key_of(s);
        let entry = agg.entry(key.clone()).or_default();
        entry.judged += 1;
        if s.was_correct == 1 {
            entry.correct += 1;
        }
        if let Some(reached) = s.target_reached {
            entry.target_judged += 1;
            if reached {
                entry.target_hit += 1;
            }
        }
        entry.returns.push(s.return_pct);
        if let Some(alpha) = s.alpha_pct {
            entry.alphas.push(alpha);
        }
        if let Some(conf) = s.confidence {
            // 水印缺失 = 上一代口径的 confidence，与同档其他样本不同尺 ⇒ 不进 IC，
            // 但要计数报出去（「样本还不够新」和「根本没有预测值」是两件不同的事）。
            if s.snr_anchor_days.is_some() {
                entry.ic_pairs.push((conf, s.return_pct));
            } else {
                entry.ic_regime_excluded += 1;
            }
        }
        if !order.contains(&key) {
            order.push(key);
        }
    }
    order
        .into_iter()
        .map(|key| {
            let a = agg.remove(&key).unwrap_or_default();
            let ic_status = if a.ic_pairs.is_empty() {
                // 有预测值但全缺水印 ⇒ 是「口径换代、新样本还没攒够」，不是「没有预测值」
                if a.ic_regime_excluded > 0 {
                    "pre_snr_regime"
                } else {
                    "no_confidence"
                }
            } else if a.ic_pairs.len() < IC_MIN_SAMPLE {
                "insufficient_ic_samples"
            } else {
                "ok"
            };
            let rank_ic = if a.ic_pairs.len() >= IC_MIN_SAMPLE {
                spearman_rank_ic(&a.ic_pairs)
            } else {
                None
            };
            // 秩全同（置信度或收益一侧无方差）时 ρ 无定义 → None，状态跟着改口径，
            // 不能让前端把「样本够了却算不出」误读成「样本不足」。
            let ic_status = if rank_ic.is_none() && ic_status == "ok" {
                "degenerate_variance"
            } else {
                ic_status
            };
            HitrateGroup {
                key: key.clone(),
                samples: a.judged,
                direction_hit_rate: hit_rate_or_none(a.correct, a.judged),
                target_hit_rate: hit_rate_or_none(a.target_hit, a.target_judged),
                avg_raw_return_pct: mean(&a.returns),
                avg_alpha_pct: mean(&a.alphas),
                rank_ic,
                ic_samples: a.ic_pairs.len(),
                ic_status: ic_status.to_string(),
                ic_regime_excluded: a.ic_regime_excluded,
                holding_days: days_of(&key),
            }
        })
        .collect()
}

/// 聚合命中率（纯函数，无 I/O）。
///
/// 判定口径（与落库层一致）：
/// - 方向命中率分子 = was_correct=1；分母 = was_correct ∈ {0,1} 的已判定样本。
/// - 价位命中率分子 = target_reached=true；分母 = 有 target_reached 判定的样本。
/// - 平均收益只统计已结算样本（pending / 未判定不在落库层写行，天然排除）。
pub fn compute_reflection_stats(samples: &[DecisionPerformanceSample]) -> HitrateStats {
    let judged_total = samples.iter().filter(|s| s.was_correct == 0 || s.was_correct == 1).count();
    let correct_total = samples.iter().filter(|s| s.was_correct == 1).count();

    let target_judged: Vec<bool> = samples.iter().filter_map(|s| s.target_reached).collect();
    let target_hit = target_judged.iter().filter(|&&t| t).count();

    let returns: Vec<f64> = samples.iter().map(|s| s.return_pct).collect();
    let alphas: Vec<f64> = samples.iter().filter_map(|s| s.alpha_pct).collect();
    let legacy_samples =
        samples.iter().filter(|s| s.data_source == SampleDataSource::Legacy).count();

    let by_action = group_hit_rates(
        samples,
        |s| s.action.clone().unwrap_or_else(|| "unknown".to_string()),
        |_| None,
    );
    let by_horizon = group_hit_rates(
        samples,
        |s| s.horizon.clone().unwrap_or_else(|| "unknown".to_string()),
        |key| {
            // "unknown" 没有持有期可言；其余走权威天数表（不手抄）。
            if key == "unknown" {
                None
            } else {
                Some(default_expected_holding_days(key))
            }
        },
    );
    let ic_points: Vec<(f64, f64)> = by_horizon
        .iter()
        .filter(|g| g.key != "unknown")
        .filter_map(|g| match (g.holding_days, g.rank_ic) {
            (Some(d), Some(ic)) => Some((d as f64, ic)),
            _ => None,
        })
        .collect();
    let usable_ic_tiers = ic_points.len();
    let signal_half_life_days = signal_half_life_days(&ic_points);

    HitrateStats {
        total_samples: samples.len(),
        direction_hit_rate: hit_rate_or_none(correct_total, judged_total),
        target_hit_rate: hit_rate_or_none(target_hit, target_judged.len()),
        avg_raw_return_pct: mean(&returns),
        avg_alpha_pct: mean(&alphas),
        legacy_samples,
        by_action,
        by_horizon,
        signal_half_life_days,
        usable_ic_tiers,
        // 代际三项由取数侧（`build_hitrate_stats`）填 —— 纯函数不做筛样。
        generation_floor: 0,
        excluded_pre_floor_generation: 0,
        unknown_generation_samples: 0,
        failure_modes: Default::default(),
        tiers: Default::default(),
        // 2×2 与代际三项同理：由取数侧按**筛完代的样本**填（`build_hitrate_stats`），纯函数不筛。
        horizon_confusion: Vec::new(),
    }
}

// ── 落库行 → 统计样本的取数 + 展开（DB 边界）─────────────────────────────
//
// 以下四项原本住在 `commands/stock_workflow/reflection_stats.rs`，2026-09-29 搬到
// 这里：`reflection_stats` 命令、`stock_workflow/hooks.rs` 的先验注入、
// `stock_analysis.rs` 的荐股先验三处共用同一实现。放命令层会导致后两处跨模块
// 调用命令（分层门禁规则 2 `commands-no-sibling-call`），放这里则三处都只是
// 「调 engine 纯函数」，依赖方向保持 consumer→implementor（见 AGENTS.md 分层表）。
//
// 跨三表聚合口径：
// - `strategy_performance`：主表（was_correct 确定性判定 + 复盘 win/loss 写回、return_pct）
// - `stock_reflections`：反思行（alpha_return、blackboard_snapshot 里递归找 target_reached）
// - `stock_analyses`：维度（decision_action / decision_time_horizon）
//
// join 口径：
// 1. **反思行**（strategy_id="reflection_verdict"）：(stock_code, created_at)
//    精确匹配 stock_reflections **同批记录** → 经 original_analysis_id 精确取
//    stock_analyses 维度；alpha / target_reached 取反思行。
// 2. **分析回测行**（复盘 cron 写回，strategy_id=trend/value/...）：stock_code +
//    decision_at 与 stock_analyses.created_at 时间窗（±3 天）匹配最近分析行取维度。
//
// 诚实性：target_reached 未独立落库（反思时仅在内存 MarketSnapshot），故从
// blackboard_snapshot 递归查找；找不到 → None（前端按「样本不足/暂无数据」展示，不伪报）。

/// 时间窗匹配阈值：decision_at 与分析 created_at 允许的最大偏差（3 天）
const JOIN_WINDOW_MS: i64 = 3 * 86_400_000;

/// 在 JSON 值中递归查找指定 key 的布尔值（blackboard_snapshot 结构不确定，
/// 兼容「一层嵌套 / 多层嵌套 / 数组内」三种形态；找不到 → None）。
fn find_bool_recursive(value: &serde_json::Value, key: &str) -> Option<bool> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(b) = map.get(key).and_then(|v| v.as_bool()) {
                return Some(b);
            }
            for v in map.values() {
                if let Some(b) = find_bool_recursive(v, key) {
                    return Some(b);
                }
            }
            None
        },
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| find_bool_recursive(v, key)),
        _ => None,
    }
}

/// 从四周期反思 JSON（`stock_reflections.horizon_results_json`）展开统计样本。
///
/// 展开口径（PLAN-stock-reflection-four-horizon 批次 4 第 11 条）：
/// - 仅 `mature` / `legacy` 周期可成样本；`immature` / `unavailable` **不进分母**。
/// - 还要求 `evaluation.wasCorrect ∈ {0,1}`（中性档 = 不可判定）且 `market.returnPct` 为有限数，
///   否则该周期整体跳过 —— 不伪造 0% 收益，也不把「未到期」写成「判错」。
/// - 已成熟周期的 action / 收益 / alpha / 目标价全部取自该周期自身条目，不与其他周期串线。
///
/// 返回 `None` 表示该 JSON 不是四周期对象（无法解析 / 非对象），调用方回退 legacy 单周期字段。
pub(crate) fn expand_horizon_samples(
    json: &str,
    template_version: Option<i32>,
) -> Option<Vec<DecisionPerformanceSample>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let map = value.as_object()?;
    let mut samples = Vec::new();
    for (horizon, entry) in map {
        let Some(entry) = entry.as_object() else {
            continue; // schemaVersion 等标量键
        };
        let status = entry.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status != "mature" && status != "legacy" {
            continue;
        }
        let was_correct = entry
            .get("evaluation")
            .and_then(|e| e.get("wasCorrect"))
            .and_then(|v| v.as_i64())
            .and_then(|v| i32::try_from(v).ok())
            .filter(|v| *v == 0 || *v == 1);
        let Some(was_correct) = was_correct else { continue };
        let market = entry.get("market");
        let return_pct = market
            .and_then(|m| m.get("returnPct"))
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite());
        let Some(return_pct) = return_pct else { continue };

        samples.push(DecisionPerformanceSample {
            action: entry
                .get("decision")
                .and_then(|d| d.get("action"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            horizon: Some(horizon.clone()),
            was_correct,
            return_pct,
            alpha_pct: market
                .and_then(|m| m.get("alphaPct"))
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite()),
            target_reached: market.and_then(|m| m.get("targetReached")).and_then(|v| v.as_bool()),
            // Phase E：IC 的预测侧。旧记录无 confidence 字段 → None（不进 IC 分母），
            // 不做「用 action 反推置信度」这类填充 —— 那是把缺失伪装成测量。
            confidence: entry
                .get("decision")
                .and_then(|d| d.get("confidence"))
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite()),
            // 判定口径水印：v104 前逐档 confidence = 生效后验，v104 起 = SNR √h 折算值，
            // 两代混在一档里会打乱池化秩 ⇒ 引擎按此位决定是否进 IC 分母。
            snr_anchor_days: entry
                .get("decision")
                .and_then(|d| d.get("snrAnchorDays").or_else(|| d.get("snr_anchor_days")))
                .and_then(|v| v.as_i64()),
            template_version,
            data_source: if status == "legacy" {
                SampleDataSource::Legacy
            } else {
                SampleDataSource::Reflection
            },
            // B3 三键：档级兄弟键（产端 = reflection.rs 的 build_horizon_results_json）。
            // 旧记录没这些键 ⇒ 全 None；None 与 0 的差别在这里是结论级的：
            // 0 会被读成「LLM 这一档判错了」或「这一档一致性 0 分」。
            llm_action: entry.get("llmAction").and_then(|v| v.as_str()).map(str::to_string),
            llm_was_correct: entry
                .get("llmWasCorrect")
                .and_then(|v| v.as_i64())
                .and_then(|v| i32::try_from(v).ok())
                .filter(|v| *v == 0 || *v == 1),
            horizon_agreement: entry
                .get("horizonAgreement")
                .and_then(|v| v.as_i64())
                .and_then(|v| i32::try_from(v).ok()),
        });
    }
    Some(samples)
}

/// 按 (stock_code, decision_at) 在分析行中找时间窗内最近的一条，返回其维度。
fn nearest_analysis_dimensions<'a>(
    analyses: &'a [stock_analyses::Model],
    stock_code: &str,
    decision_at: i64,
) -> Option<&'a stock_analyses::Model> {
    analyses
        .iter()
        .filter(|a| a.stock_code == stock_code)
        .filter(|a| (a.created_at - decision_at).abs() <= JOIN_WINDOW_MS)
        .min_by_key(|a| (a.created_at - decision_at).abs())
}

/// sp 行 → 反思行的匹配：**首选显式外键**，外键缺失/悬空时退回旧的时间戳批键。
///
/// 返回 `(与 sp_rows 同序的匹配结果, 走旧键回退的行数)`，纯函数（不碰 DB），
/// 便于把「外键命中」与「旧键回退」两种情形各打一条样本直接锁住。
///
/// 为什么要这一层（2026-10-14，PLAN-reflection-optimization-closure §二）：旧键
/// `(stock_code, created_at, 复盘档)` 要求两侧 `created_at` 相等，而 Gap1 写入的 sp 行用
/// **反思完成时刻**、反思行用**建点时刻**（pending 行由决策落盘时建）⇒ 批量反思路径上
/// 永远配不上（现网实测 8 行 0 命中），逐档展开分支从未生效、统计恒落 legacy。
/// 现在写侧当场盖 `reflection_id`，读侧据此直连；NULL/悬空只可能是本列引入前的存量行。
///
/// 旧键里为什么仍要带档：〇-B v2 起**一行反思 = 一个周期档**，同一条分析产出的四行由建点用
/// **同一个 now_ms** 写 `created_at` ⇒ 只用 (stock_code, created_at) 做键会四行同键、
/// `.or_insert` 随机丢三档（命中率统计的档间差异被抹平）。sp 侧的档来自 `period` 的
/// `reflection:{档}` 后缀（写入点：`commands/stock_workflow/reflection.rs` 的 Gap1 构造）；
/// 老数据两侧同构：sp.period 无后缀 ⇔ 反思行 horizon NULL ⇒ 仍按 None 相配。
pub fn match_sp_rows_to_reflections<'a>(
    sp_rows: &'a [strategy_performance::Model],
    ref_rows: &'a [stock_reflections::Model],
) -> (Vec<Option<&'a stock_reflections::Model>>, usize) {
    let ref_by_id: HashMap<&str, &'a stock_reflections::Model> =
        ref_rows.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut ref_by_batch: HashMap<(String, i64, Option<String>), &'a stock_reflections::Model> =
        HashMap::new();
    for r in ref_rows {
        ref_by_batch.entry((r.stock_code.clone(), r.created_at, r.horizon.clone())).or_insert(r);
    }

    let mut out = Vec::with_capacity(sp_rows.len());
    let mut legacy_key_rows = 0usize;
    for sp in sp_rows {
        let by_fk = sp.reflection_id.as_deref().and_then(|rid| ref_by_id.get(rid).copied());
        let matched = by_fk.or_else(|| {
            // 只有 Gap1 旧格（写侧字面量 `"reflection_verdict"`，见
            // `commands/stock_workflow/reflection.rs` 的 Gap1 构造）才该有反思行归属。
            // 其它通道（#31 四档 / 分析回测 / 对话直执行）本来就不参与这个匹配，
            // 把它们的 NULL 也计入回退数会让告警常年长亮、失去信号。
            if sp.strategy_id == "reflection_verdict" {
                legacy_key_rows += 1;
            }
            let sp_horizon = sp.period.strip_prefix("reflection:").map(str::to_string);
            ref_by_batch.get(&(sp.stock_code.clone(), sp.created_at, sp_horizon)).copied()
        });
        out.push(matched);
    }
    (out, legacy_key_rows)
}

/// 取数 + 逐周期展开 + 聚合的**唯一实现**（三处调用方共用，禁区 12：不重复实现）。
///
/// 2026-09-29 从命令体抽出：四周期科学化 Phase C 要在**工作流启动时**读同一份统计
/// （`hooks.rs` 据此注入 `horizon_prior_json` 逐档先验）。若留在命令体里，hooks 只能
/// 再抄一遍取数，且 `stock_analysis.rs` 的荐股先验也要调 —— 两处跨模块调命令 =
/// 分层门禁规则 2 违规。落到 engine 后三处都只是调本函数。
pub async fn build_hitrate_stats(db: &DatabaseConnection) -> Result<HitrateStats, String> {
    let sp_rows = strategy_performance::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 strategy_performance 失败: {e}"))?;
    let ref_rows = stock_reflections::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 stock_reflections 失败: {e}"))?;
    // P2-b：失败模式看板复用同一批反思行（零额外 IO）—— 只统计 completed 行，
    // pending/failed 行还没判定，计入会把「未产出」虚报成「未标注」。
    let fm_rows: Vec<Option<String>> = ref_rows
        .iter()
        .filter(|r| r.status.starts_with("completed"))
        .map(|r| r.failure_modes.clone())
        .collect();
    let failure_modes = crate::failure_mode::build_failure_mode_board(&fm_rows);
    // 阶段 3：层级看板复用同一批 completed 行（零额外 IO，与上面同一判据：
    // pending/failed 行还没判定，计入会把「未产出」虚报成「未标注」）。
    let completed_ref_rows: Vec<&stock_reflections::Model> =
        ref_rows.iter().filter(|r| r.status.starts_with("completed")).collect();
    let tiers = build_tier_board(&completed_ref_rows);
    let ana_rows = stock_analyses::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 stock_analyses 失败: {e}"))?;

    // 索引 1：stock_analyses.id → Model
    let ana_by_id: HashMap<&str, &stock_analyses::Model> =
        ana_rows.iter().map(|a| (a.id.as_str(), a)).collect();
    // 索引 2：sp 行 → 反思行的匹配（首选显式外键 `reflection_id`，退回旧时间戳批键）。
    let (ref_matches, legacy_key_rows) = match_sp_rows_to_reflections(&sp_rows, &ref_rows);
    if legacy_key_rows > 0 {
        tracing::warn!(
            "[hitrate] {} 行 strategy_performance 没有可用的 reflection_id（缺失或悬空）⇒ 退回旧时间戳批键匹配。\
             该旧键要求两侧 created_at 相等，而 sp 行写的是反思**完成**时刻、反思行写的是**建点**时刻，\
             配不上的行会静默落 legacy 分支并丢掉逐档结果",
            legacy_key_rows
        );
    }

    let mut samples = Vec::with_capacity(sp_rows.len());
    for (idx, sp) in sp_rows.iter().enumerate() {
        let matched = ref_matches[idx];
        // §七十三 代际归属：优先取**反思行的建点盖章**（#5 落的列），配不上反思行的 sp 行
        // 退到分析行的时间窗匹配（`nearest_analysis_dimensions` 与上面 action/horizon 同源）。
        // 两条都拿不到 ⇒ None = 代际未知，**不退回编译期版本**（那会把「不知道」写成「就是这一代」）。
        let sp_template_version: Option<i32> =
            matched.and_then(|r| r.template_version).or_else(|| {
                nearest_analysis_dimensions(&ana_rows, &sp.stock_code, sp.decision_at)
                    .and_then(|a| a.template_version)
            });

        // 路径 1：反思行有四周期结果 JSON → 按周期独立展开（每周期一条样本）
        if let Some(expanded) = matched
            .and_then(|r| r.horizon_results_json.as_deref())
            .and_then(|j| expand_horizon_samples(j, sp_template_version))
        {
            samples.extend(expanded);
            continue;
        }

        // 路径 2：旧记录无四周期 JSON → 回退 legacy 主周期字段，并带 legacy 数据源标识
        let mut action = None;
        let mut horizon = None;
        let mut alpha_pct = None;
        let mut target_reached = None;

        if let Some(r) = matched {
            alpha_pct = r.alpha_return;
            target_reached = r
                .blackboard_snapshot
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .and_then(|v| find_bool_recursive(&v, "target_reached"));
            // `original_analysis_id` 是**非空 String**（entity `stock_reflections.rs:14`），
            // 不是 Option ⇒ 直接取 `&str` 查索引，没有 Option 链可 `.and_then`。
            if let Some(a) = ana_by_id.get(r.original_analysis_id.as_str()) {
                action = a.decision_action.clone();
                horizon = a.decision_time_horizon.clone();
            }
        }

        // 时间窗匹配（分析回测行 / 反思批匹配失败兜底）
        if action.is_none() {
            if let Some(a) = nearest_analysis_dimensions(&ana_rows, &sp.stock_code, sp.decision_at)
            {
                action = a.decision_action.clone();
                horizon = a.decision_time_horizon.clone();
            }
        }

        samples.push(DecisionPerformanceSample {
            action,
            horizon,
            was_correct: sp.was_correct,
            return_pct: sp.return_pct,
            alpha_pct,
            target_reached,
            // legacy 回退路径没有逐档决策，置信度只能是 None —— 该样本进命中率
            // 分母，但**不进 IC 分母**（IC 缺席由 `ic_status="no_confidence"` 点名）。
            confidence: None,
            snr_anchor_days: None,
            template_version: sp_template_version,
            data_source: SampleDataSource::Legacy,
            // B3 三键在 legacy 回退路径**就从本行自己带回**（这四列正是本表的列）：
            // 同一行的公式侧与 LLM 侧成对出现，去 `agreementByHorizon` 另取反而会与
            // 落库时那次判定不一致（同一决策两次读数）。
            llm_action: sp.llm_action.clone(),
            llm_was_correct: sp.llm_was_correct,
            horizon_agreement: sp.horizon_agreement,
        });
    }

    // §七十三 按代筛样：把**全部统计量的分母**收窄到当前代。
    // 为什么必须筛：跨代混池算出来的命中率/IC/先验是「两套判据的加权平均」，
    // 既不是旧代的结论也不是新代的结论（本仓已为此付过代价：换算法后指标的含义悄悄变了）。
    // 为什么两类要**分开计数**：旧代样本是永久性剔除（不可比，只报数）；
    // 代际未知样本 P2-f 起按收缩纳入（`unknown_generation_samples` 报其中未知的条数）——
    // 合成一个数字就看不出「样本为什么变少」到底是不是真少了。
    let (kept, pre_floor, unknown) =
        filter_by_generation_floor(samples, HORIZON_BRANCH_GENERATION_FLOOR);
    let mut stats = compute_reflection_stats(&kept);
    stats.generation_floor = HORIZON_BRANCH_GENERATION_FLOOR;
    stats.excluded_pre_floor_generation = pre_floor;
    stats.unknown_generation_samples = unknown;
    // P2-b：失败模式看板（同一次读库的 completed 反思行聚合，不额外查询）。
    stats.failure_modes = failure_modes;
    // 阶段 3：层级看板随行装配（面板的结构层待裁清单读它）
    stats.tiers = tiers;
    // B4：逐档 2×2 用**同一批筛完代的样本**（`kept`）——分母必须与上面的命中率同源，
    // 否则「命中率按代、2×2 跨代」就成了同一张面板上两种口径。
    stats.horizon_confusion = build_horizon_confusion(&kept);
    Ok(stats)
}

/// 代际归属的**单格判定**（全 crate 唯一载体）。
///
/// 为什么要有这一层而不是各处自己比 `>= floor`：按代筛样的三分（本代 / 旧代 / 代际未知）
/// 已经在 P2-f 改过一次口径（未知从「整批丢弃」改成「收缩纳入」）。改一次要同步改所有
/// 抄了字面比较的地方 —— 漏一处就出现两份判据（本仓为这类裂缝付过多次代价，禁区 12）。
/// 新增的腿级预测力检验（[`crate::leg_predictiveness`]）与命中率筛样必须同一把尺。
pub(crate) enum GenerationBucket {
    /// `>= floor` 的本代样本
    Current,
    /// 早于起算代 —— 算法口径不可比，永久排除（不会再回到分母）
    PreFloor,
    /// 代际未知（`None`）—— P2-f 起纳入分母，但仍单独计数
    Unknown,
}

pub(crate) fn generation_bucket(template_version: Option<i32>, floor: i32) -> GenerationBucket {
    match template_version {
        Some(v) if v >= floor => GenerationBucket::Current,
        Some(_) => GenerationBucket::PreFloor,
        None => GenerationBucket::Unknown,
    }
}

/// 按**起算代际**筛样 + 代际未知样本的计数（§五十一-②/§七十三；P2-f 改口径）。
///
/// 规则是下限 `>= floor`（不是等号，理由见常量注释）。三类样本的处置：
/// - `>= floor`：正常纳入；
/// - 早于 floor（**已知是旧代**）：永久性剔除（算法口径不可比，不会再回分母）；
/// - `None`（**代际未知**）：P2-f（`PLAN-stock-decision-reliability.md` R4/R12）起
///   **纳入统计**而非整批丢弃 —— 唯一存在的 8 条 legacy 样本（均收益 -2.82%、
///   方向胜率 3/8）曾被「代际未知 ⇒ 排除」规则整批压掉，系统对唯一的不利证据完全
///   不可见。小样本过拟合由先验侧收缩（`shrink_prior` 向 0.5 收缩、强度随样本量）
///   自然抑制，无须在筛样层一刀切。unknown 仍**单独计数**
///   （`unknown_generation_samples`）供面板如实报出。
fn filter_by_generation_floor(
    samples: Vec<DecisionPerformanceSample>,
    floor: i32,
) -> (Vec<DecisionPerformanceSample>, usize, usize) {
    let mut kept = Vec::with_capacity(samples.len());
    let mut pre_floor = 0usize;
    let mut unknown = 0usize;
    for s in samples {
        match generation_bucket(s.template_version, floor) {
            GenerationBucket::Current => kept.push(s),
            GenerationBucket::PreFloor => pre_floor += 1,
            // P2-f：代际未知 ≠ 旧代 —— 「不知道」不得被当成「不可比」整批丢弃。
            GenerationBucket::Unknown => {
                kept.push(s);
                unknown += 1;
            },
        }
    }
    (kept, pre_floor, unknown)
}

/// B4（`PLAN-llm-horizon-decision-and-parity` §18/§21）：**逐档结局 2×2**的一行。
///
/// 每一格回答的是「公式这一档判对/判错 × LLM 这一档判对/判错」——
/// 两侧用的是**同一份 `net_return_pct`、同一个判定函数、同一个成熟窗口**（判定发生在
/// `reflection.rs` 落 `horizon_results_json` 时，本模块只做聚合），所以四格可直接对比。
///
/// ⚠ 两个「不计入任何一格」的分母必须单列，别折叠成 0：
/// - `llm_undecided`：LLM 侧无从判定（该档缺席 / 窗口未满 / 动作不是方向性的（观望/持有））——
///   把这类记成「LLM 判错」会让 LLM 的胜率**凭空变差**；
/// - 公式侧无对应行（该档树没长出来）时整档缺席，由 `judged == 0` 表达。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HorizonConfusionRow {
    /// 档位键（snake，与 `strategy_performance.horizon_results_json` 同族）
    pub tier: String,
    /// 档位中文名（与 `HorizonAgreement.tier_label` 同口径；前端显示名仍走自己的 i18n）
    pub tier_label: String,
    pub both_correct: usize,
    /// 公式判对而 LLM 判错
    pub formula_only: usize,
    /// 公式判错而 LLM 判对
    pub llm_only: usize,
    pub both_wrong: usize,
    /// LLM 侧无从判定的条数（**不进四格**，见结构体文档）
    pub llm_undecided: usize,
    /// 四格合计（= 上式四项之和）。它就是这档 2×2 的分母。
    pub judged: usize,
    /// 该档三维分均值（只收 `horizon_agreement` 有值的行）；一条都没有 → None（不是 0 分）
    pub avg_horizon_agreement: Option<f64>,
    /// `judged < MIN_SAMPLE` ⇒ 面板只能说「样本不足」，**不得**把这四个计数当结论读
    pub sufficient: bool,
}

/// 逐档 2×2 的**纯函数**聚合（四档恒各出一行，缺样本的档用 `judged == 0` 表达）。
///
/// 判定口径（为什么不在这里重算）：`was_correct` / `llm_was_correct` 都是产端用
/// [`deterministic_horizon_was_correct`] 在**同一份档快照**上算好落库的；这里重算需要
/// 快照与 `within_expected_horizon`，那是给同一件事立第二个真相。
pub fn build_horizon_confusion(samples: &[DecisionPerformanceSample]) -> Vec<HorizonConfusionRow> {
    let mut out = Vec::with_capacity(Period::ALL.len());
    for period in Period::ALL {
        let tier = period.as_str();
        let mut row = HorizonConfusionRow {
            tier: tier.to_string(),
            tier_label: period.label_zh().to_string(),
            ..Default::default()
        };
        let mut agr_sum = 0.0_f64;
        let mut agr_n = 0usize;
        for s in samples.iter().filter(|s| s.horizon.as_deref() == Some(tier)) {
            if let Some(a) = s.horizon_agreement {
                agr_sum += a as f64;
                agr_n += 1;
            }
            // 两侧都必须可判才进四格：公式侧理论恒为 0/1（`expand_horizon_samples` 已过滤），
            // 但这里按元组匹配 ⇒ 万一出现越界值也落「无从判定」而**不是**被算成某一格。
            match (s.was_correct, s.llm_was_correct) {
                (1, Some(1)) => row.both_correct += 1,
                (1, Some(0)) => row.formula_only += 1,
                (0, Some(1)) => row.llm_only += 1,
                (0, Some(0)) => row.both_wrong += 1,
                // 含 LLM 侧 None（缺席/未满/非方向性动作）⇒ 单列，不进四格
                _ => {
                    row.llm_undecided += 1;
                    continue;
                },
            }
            row.judged += 1;
        }
        if agr_n > 0 {
            row.avg_horizon_agreement = Some(agr_sum / agr_n as f64);
        }
        row.sufficient = row.judged >= MIN_SAMPLE;
        out.push(row);
    }
    out
}

// ── 单元测试 ── 追加在文件末尾（防 clippy::items_after_test_module）
#[cfg(test)]
mod reflection_stats_tests {
    use super::*;

    const FOUR_HORIZON_FIXTURE: &str =
        include_str!("../tests/fixtures/stock_reflection_four_horizon_complete.json");
    const LEGACY_FIXTURE: &str =
        include_str!("../tests/fixtures/stock_reflection_legacy_single_horizon.json");
    const UNAVAILABLE_FIXTURE: &str =
        include_str!("../tests/fixtures/stock_reflection_market_unavailable.json");

    fn sample(
        action: Option<&str>,
        horizon: Option<&str>,
        was_correct: i32,
        return_pct: f64,
    ) -> DecisionPerformanceSample {
        sample_gen(action, horizon, was_correct, return_pct, None)
    }

    /// 同 `sample`，但指定样本所属代际（§五十一-② 的按代筛样用）。
    fn sample_gen(
        action: Option<&str>,
        horizon: Option<&str>,
        was_correct: i32,
        return_pct: f64,
        template_version: Option<i32>,
    ) -> DecisionPerformanceSample {
        DecisionPerformanceSample {
            action: action.map(str::to_string),
            horizon: horizon.map(str::to_string),
            was_correct,
            return_pct,
            alpha_pct: None,
            target_reached: None,
            confidence: None,
            snr_anchor_days: None,
            template_version,
            data_source: SampleDataSource::Reflection,
            llm_action: None,
            llm_was_correct: None,
            horizon_agreement: None,
        }
    }

    #[test]
    fn five_plus_samples_produce_hit_rate() {
        let samples = vec![
            sample(Some("买入"), Some("short"), 1, 5.0),
            sample(Some("买入"), Some("short"), 1, 3.0),
            sample(Some("买入"), Some("short"), 1, 1.0),
            sample(Some("买入"), Some("mid"), 0, -2.0),
            sample(Some("卖出"), Some("mid"), 1, 4.0),
        ];
        let stats = compute_reflection_stats(&samples);
        assert_eq!(stats.total_samples, 5);
        // 4 对 / 5 已判定 = 0.8
        assert_eq!(stats.direction_hit_rate, Some(0.8));
        // 全部样本都有 return_pct → 均值 (5+3+1-2+4)/5 = 2.2
        assert_eq!(stats.avg_raw_return_pct, Some(2.2));
        // 分组：买入 3/4、卖出 1/1
        let buy = stats.by_action.iter().find(|g| g.key == "买入").unwrap();
        assert_eq!(buy.samples, 4);
        assert_eq!(buy.direction_hit_rate, None); // 4 < MIN_SAMPLE
        let sell = stats.by_action.iter().find(|g| g.key == "卖出").unwrap();
        assert_eq!(sell.samples, 1);
        assert_eq!(sell.direction_hit_rate, None); // 1 < MIN_SAMPLE
    }

    #[test]
    fn below_min_sample_returns_none() {
        let samples = vec![sample(Some("买入"), Some("short"), 1, 1.0)];
        let stats = compute_reflection_stats(&samples);
        assert_eq!(stats.direction_hit_rate, None);
        assert_eq!(stats.avg_raw_return_pct, Some(1.0));
    }

    #[test]
    fn empty_input_returns_defaults() {
        let stats = compute_reflection_stats(&[]);
        assert_eq!(stats.total_samples, 0);
        assert_eq!(stats.direction_hit_rate, None);
        assert_eq!(stats.target_hit_rate, None);
        assert!(stats.by_action.is_empty());
        assert!(stats.by_horizon.is_empty());
    }

    #[test]
    fn target_hit_rate_counts_only_judged_samples() {
        let mut s1 = sample(Some("买入"), Some("short"), 1, 5.0);
        s1.target_reached = Some(true);
        let mut s2 = sample(Some("买入"), Some("short"), 0, -5.0);
        s2.target_reached = Some(false);
        let mut s3 = sample(Some("买入"), Some("short"), 1, 1.0);
        s3.target_reached = Some(true);
        let mut s4 = sample(Some("买入"), Some("short"), 1, 2.0);
        s4.target_reached = Some(true);
        let mut s5 = sample(Some("买入"), Some("short"), 1, 3.0);
        s5.target_reached = Some(false);
        let stats = compute_reflection_stats(&[s1, s2, s3, s4, s5]);
        // 3 达目标 / 5 判定 = 0.6
        assert_eq!(stats.target_hit_rate, Some(0.6));
    }

    #[test]
    fn alpha_mean_only_over_samples_with_alpha() {
        let mut s1 = sample(Some("买入"), Some("short"), 1, 5.0);
        s1.alpha_pct = Some(1.0);
        let mut s2 = sample(Some("买入"), Some("short"), 1, 3.0);
        s2.alpha_pct = Some(2.0);
        let s3 = sample(Some("买入"), Some("short"), 0, -2.0);
        let s4 = sample(Some("买入"), Some("short"), 1, 4.0);
        let s5 = sample(Some("买入"), Some("short"), 1, 6.0);
        let stats = compute_reflection_stats(&[s1, s2, s3, s4, s5]);
        // alpha 只取有值两项：(1+2)/2 = 1.5
        assert_eq!(stats.avg_alpha_pct, Some(1.5));
    }

    #[test]
    fn unknown_group_for_missing_dimension() {
        let samples = vec![
            sample(None, None, 1, 5.0),
            sample(None, None, 1, 3.0),
            sample(None, None, 1, 1.0),
            sample(None, None, 0, -2.0),
            sample(None, None, 1, 4.0),
        ];
        let stats = compute_reflection_stats(&samples);
        let act = stats.by_action.iter().find(|g| g.key == "unknown").unwrap();
        assert_eq!(act.samples, 5);
        assert_eq!(act.direction_hit_rate, Some(0.8));
        let hor = stats.by_horizon.iter().find(|g| g.key == "unknown").unwrap();
        assert_eq!(hor.samples, 5);
    }

    #[test]
    fn parses_four_horizon_decisions_from_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(FOUR_HORIZON_FIXTURE).unwrap();
        let decisions = parse_horizon_decisions(
            fixture["stockAnalysis"]["horizonDecisions"].as_object().unwrap(),
        )
        .unwrap();

        assert_eq!(decisions.len(), 4);
        assert_eq!(decisions["ultra_short"].action, "买入");
        assert_eq!(decisions["short"].expected_holding_days, Some(5));
        assert_eq!(decisions["mid"].target_price, Some(1850.0));
        assert_eq!(decisions["long"].stop_loss, Some(1450.0));
    }

    #[test]
    fn legacy_fixture_falls_back_to_single_horizon() {
        let fixture: serde_json::Value = serde_json::from_str(LEGACY_FIXTURE).unwrap();
        let result = legacy_horizon_result(
            fixture["stockAnalysis"]["decisionAction"].as_str(),
            fixture["stockAnalysis"]["decisionTimeHorizon"].as_str(),
            fixture["stockAnalysis"]["decisionExpectedHoldingDays"].as_i64(),
        )
        .unwrap();

        assert_eq!(result.status, HorizonStatus::Legacy);
        assert_eq!(result.horizon, "short");
        assert_eq!(result.expected_holding_days, 5);
        assert_eq!(result.action, "卖出");
    }

    #[test]
    fn unavailable_fixture_never_becomes_zero_return() {
        let fixture: serde_json::Value = serde_json::from_str(UNAVAILABLE_FIXTURE).unwrap();
        let result = build_horizon_evaluation(
            HorizonStatus::Unavailable,
            fixture["expected"]["wasCorrect"].as_i64(),
            fixture["expected"]["returnPct"].as_f64(),
        );

        assert_eq!(result.status, HorizonStatus::Unavailable);
        assert_eq!(result.was_correct, None);
        assert_eq!(result.return_pct, None);
    }

    #[test]
    fn mature_horizon_direction_uses_action_and_return_sign() {
        assert_eq!(deterministic_horizon_was_correct("买入", 3.2, false), Some(1));
        assert_eq!(deterministic_horizon_was_correct("增持", -0.1, false), Some(0));
        assert_eq!(deterministic_horizon_was_correct("卖出", -3.2, false), Some(1));
        assert_eq!(deterministic_horizon_was_correct("减持", 0.1, false), Some(0));
        assert_eq!(deterministic_horizon_was_correct("持有", 3.2, false), None);
        assert_eq!(deterministic_horizon_was_correct("买入", -3.2, true), None);
        assert_eq!(deterministic_horizon_was_correct("买入", f64::NAN, false), None);
    }

    #[test]
    fn parses_snake_case_decision_fields_and_uses_horizon_default() {
        let source = serde_json::json!({
            "ultra_short": {
                "action": "BUY",
                "position_pct": 0.2,
                "expected_holding_days": null,
                "target_price": 102.0,
                "stop_loss": 98.0,
                "conf_lower_bound": 0.45
            }
        });
        let decisions = parse_horizon_decisions(source.as_object().unwrap()).unwrap();

        assert_eq!(decisions["ultra_short"].position_pct, Some(0.2));
        assert_eq!(decisions["ultra_short"].target_price, Some(102.0));
        assert_eq!(default_expected_holding_days("ultra_short"), 2);
        assert_eq!(default_expected_holding_days("short"), 5);
        assert_eq!(default_expected_holding_days("mid"), 28);
        assert_eq!(default_expected_holding_days("long"), 90);
    }

    #[test]
    fn horizon_group_carries_target_return_and_alpha() {
        // 5 条 horizon=short 样本：4 命中目标、净收益 1..5、alpha 仅 2 条有值
        let mut samples = Vec::new();
        for (idx, correct) in [1, 1, 1, 1, 0].into_iter().enumerate() {
            let mut s = sample(Some("买入"), Some("short"), correct, idx as f64 + 1.0);
            s.target_reached = Some(correct == 1);
            s.alpha_pct = if idx < 2 {
                Some(0.5 + idx as f64)
            } else {
                None
            };
            samples.push(s);
        }
        let stats = compute_reflection_stats(&samples);
        let group = stats.by_horizon.iter().find(|g| g.key == "short").unwrap();

        assert_eq!(group.samples, 5);
        assert_eq!(group.direction_hit_rate, Some(0.8));
        assert_eq!(group.target_hit_rate, Some(0.8));
        assert_eq!(group.avg_raw_return_pct, Some(3.0)); // (1+2+3+4+5)/5
        assert_eq!(group.avg_alpha_pct, Some(1.0)); // (0.5+1.5)/2
    }

    #[test]
    fn horizon_group_treats_missing_target_and_alpha_as_none() {
        // 全部样本无目标价判定 / 无 alpha → 两项指标必须是 None，不得伪报为 0
        let samples: Vec<_> =
            (0..5).map(|idx| sample(Some("买入"), Some("mid"), 1, idx as f64)).collect();
        let stats = compute_reflection_stats(&samples);
        let group = stats.by_horizon.iter().find(|g| g.key == "mid").unwrap();

        assert_eq!(group.samples, 5);
        assert_eq!(group.target_hit_rate, None);
        assert_eq!(group.avg_alpha_pct, None);
        assert_eq!(group.avg_raw_return_pct, Some(2.0));
    }

    #[test]
    fn horizon_min_sample_applied_per_horizon_independently() {
        // ultra_short 5 条、long 4 条：仅前者产出命中率，long 必须 None（按 horizon 独立门槛）
        let mut samples: Vec<_> =
            (0..5).map(|_| sample(Some("买入"), Some("ultra_short"), 1, 1.0)).collect();
        samples.extend((0..4).map(|_| sample(Some("买入"), Some("long"), 1, 1.0)));

        let stats = compute_reflection_stats(&samples);
        let ultra = stats.by_horizon.iter().find(|g| g.key == "ultra_short").unwrap();
        let long = stats.by_horizon.iter().find(|g| g.key == "long").unwrap();

        assert_eq!(ultra.direction_hit_rate, Some(1.0));
        assert_eq!(long.samples, 4);
        assert_eq!(long.direction_hit_rate, None);
    }

    #[test]
    fn legacy_samples_are_counted_without_changing_hit_rate() {
        let mut legacy = sample(Some("卖出"), Some("short"), 1, -3.0);
        legacy.data_source = SampleDataSource::Legacy;
        let samples = vec![
            legacy,
            sample(Some("买入"), Some("short"), 1, 1.0),
            sample(Some("买入"), Some("short"), 0, -1.0),
        ];
        let stats = compute_reflection_stats(&samples);

        assert_eq!(stats.total_samples, 3);
        assert_eq!(stats.legacy_samples, 1);
        // legacy 样本照常进分子分母：2 对 / 3 判定
        assert_eq!(stats.direction_hit_rate, None); // 3 < MIN_SAMPLE
    }

    // ── Phase E：rank IC 与预测半衰期 ──

    /// 当代（v104+）带水印样本：IC 分母只收这种。
    fn conf_sample(horizon: &str, conf: f64, ret: f64) -> DecisionPerformanceSample {
        let mut s = sample(Some("买入"), Some(horizon), 1, ret);
        s.confidence = Some(conf);
        s.snr_anchor_days = Some(SNR_ANCHOR);
        s
    }

    /// 上一代记录：有 confidence 但**没有口径水印** ⇒ 不得进 IC 分母。
    fn legacy_conf_sample(horizon: &str, conf: f64, ret: f64) -> DecisionPerformanceSample {
        let mut s = conf_sample(horizon, conf, ret);
        s.snr_anchor_days = None;
        s
    }

    #[test]
    fn spearman_is_perfectly_monotone_regardless_of_scaling() {
        // 秩相关只看顺序：把 y 换成任意严格递增变换，ρ 必须仍是 1
        let pairs: Vec<(f64, f64)> =
            (0..10).map(|i| (i as f64, (i as f64).powi(3) + 7.0)).collect();
        assert_eq!(spearman_rank_ic(&pairs), Some(1.0));
        let rev: Vec<(f64, f64)> = pairs.iter().map(|(x, y)| (*x, -y)).collect();
        assert_eq!(spearman_rank_ic(&rev), Some(-1.0));
    }

    #[test]
    fn spearman_uses_mid_ranks_for_ties() {
        // x 有并列：(1,1,2,3) → 秩 (1.5,1.5,3,4)；y 严格递增 ⇒ ρ 是确定值而非 NaN
        let pairs = vec![(1.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, 4.0)];
        let ic = spearman_rank_ic(&pairs).unwrap();
        // 手算：rx=(1.5,1.5,3,4)、ry=(1,2,3,4) ⇒ cov=4.5, vx=4.5, vy=5.0
        // ⇒ ρ = 4.5/√22.5 = 0.9486833
        assert!((ic - 0.9486833).abs() < 1e-6, "并列未取平均秩：{ic}");
    }

    #[test]
    fn spearman_returns_none_when_undefined_not_fake_zero() {
        // 一侧无方差（全同分）⇒ ρ 无定义，必须 None（返回 0 会被读成「无相关」）
        assert_eq!(spearman_rank_ic(&[(0.5, 1.0), (0.5, 2.0), (0.5, 3.0)]), None);
        // 样本 < 2 / NaN / Inf 同样 None，不产出系数
        assert_eq!(spearman_rank_ic(&[(0.5, 1.0)]), None);
        assert_eq!(spearman_rank_ic(&[(f64::NAN, 1.0), (0.5, 2.0)]), None);
        assert_eq!(spearman_rank_ic(&[(0.5, f64::INFINITY), (0.6, 2.0)]), None);
    }

    #[test]
    fn ic_gate_is_eight_and_labels_why_it_is_missing() {
        // 7 条带置信度的样本：命中率照出（≥5），IC 必须 None 且状态点名「样本不足」
        let samples: Vec<_> =
            (0..7).map(|i| conf_sample("short", 0.5 + i as f64 * 0.01, i as f64)).collect();
        let stats = compute_reflection_stats(&samples);
        let g = stats.by_horizon.iter().find(|g| g.key == "short").unwrap();
        assert_eq!(g.direction_hit_rate, Some(1.0));
        assert_eq!(g.rank_ic, None);
        assert_eq!(g.ic_samples, 7);
        assert_eq!(g.ic_status, "insufficient_ic_samples");

        // 补到 8 条 ⇒ 同一批口径下 IC 出数
        let mut more = samples.clone();
        more.push(conf_sample("short", 0.58, 7.0));
        let stats = compute_reflection_stats(&more);
        let g = stats.by_horizon.iter().find(|g| g.key == "short").unwrap();
        assert_eq!(g.ic_status, "ok");
        assert_eq!(g.rank_ic, Some(1.0));
    }

    /// 判定口径换代（v104 给逐档 confidence 加了 SNR √h 折算）不能让上一代记录混进 IC。
    ///
    /// 我第一版在这里写错过一句论证：「同一档内换算是单调 ⇒ 秩不变 ⇒ IC 不变」。
    /// **那是错的**：单调性只在同一代刻度内成立，跨代混在一档里时，两代的相对顺序本身
    /// 会被各自不同的系数打乱 ⇒ 池化秩被改，IC 也随之偏。故按水印筛，而不是靠性质免疫。
    #[test]
    fn pre_watermark_records_are_excluded_and_named_separately() {
        let find = |stats: &HitrateStats, key: &str| -> HitrateGroup {
            stats.by_horizon.iter().find(|g| g.key == key).unwrap().clone()
        };

        // ① 全上一代：有 confidence 但无水印 ⇒ 不产 IC，状态必须说「换代」而非「没置信度」
        let stale: Vec<_> = (0..8).map(|i| legacy_conf_sample("mid", i as f64, i as f64)).collect();
        let g = find(&compute_reflection_stats(&stale), "mid");
        assert_eq!(g.rank_ic, None, "缺水印的样本不得进 IC");
        assert_eq!(g.ic_samples, 0);
        assert_eq!(g.ic_regime_excluded, 8, "排除条数必须报出来（换代进度可见）");
        assert_eq!(g.ic_status, "pre_snr_regime");

        // ② 与「根本没有置信度」区分开：那条状态仍是 no_confidence
        let noconf: Vec<_> =
            (0..8).map(|i| sample(Some("买入"), Some("mid"), 1, i as f64)).collect();
        let g2 = find(&compute_reflection_stats(&noconf), "mid");
        assert_eq!(g2.ic_status, "no_confidence");
        assert_eq!(g2.ic_regime_excluded, 0);

        // ③ 混代：只数当代的 3 条 ⇒ insufficient_ic_samples（门槛 8），排除数 5
        let mut mixed: Vec<_> =
            (0..5).map(|i| legacy_conf_sample("mid", i as f64, i as f64)).collect();
        mixed.extend((5..8).map(|i| conf_sample("mid", i as f64, i as f64)));
        let g3 = find(&compute_reflection_stats(&mixed), "mid");
        assert_eq!(g3.ic_samples, 3);
        assert_eq!(g3.ic_regime_excluded, 5);
        assert_eq!(g3.ic_status, "insufficient_ic_samples");
        assert_eq!(g3.rank_ic, None);

        // ④ 换代攒够 8 条后同一形态就该出数（证明排除按水印而非按数量）
        let mut fresh: Vec<_> =
            mixed.iter().filter(|s| s.snr_anchor_days.is_some()).cloned().collect();
        fresh.extend((8..13).map(|i| conf_sample("mid", i as f64, i as f64)));
        let g4 = find(&compute_reflection_stats(&fresh), "mid");
        assert_eq!(g4.ic_samples, 8);
        assert_eq!(g4.ic_regime_excluded, 0);
        assert_eq!(g4.ic_status, "ok");
        assert_eq!(g4.rank_ic, Some(1.0));
    }

    #[test]
    fn missing_confidence_is_labeled_apart_from_missing_samples() {
        // 全部无置信度（legacy 回退的真实形态）：缺席原因必须是 no_confidence，
        // 不能与「有置信度但太少」混成同一句话 —— 前者要补字段、后者要攒样本。
        let samples: Vec<_> =
            (0..6).map(|i| sample(Some("买入"), Some("long"), 1, i as f64)).collect();
        let stats = compute_reflection_stats(&samples);
        let g = stats.by_horizon.iter().find(|g| g.key == "long").unwrap();
        assert_eq!(g.rank_ic, None);
        assert_eq!(g.ic_samples, 0);
        assert_eq!(g.ic_status, "no_confidence");
    }

    #[test]
    fn horizon_groups_carry_holding_days_from_the_single_authority() {
        let samples: Vec<_> = ["ultra_short", "short", "mid", "long"]
            .iter()
            .flat_map(|h| (0..8).map(move |i| conf_sample(h, 0.5 + i as f64 * 0.01, i as f64)))
            .chain((0..8).map(|i| sample(None, None, 1, i as f64)))
            .collect();
        let stats = compute_reflection_stats(&samples);
        for (key, days) in [("ultra_short", 2_i64), ("short", 5), ("mid", 28), ("long", 90)] {
            let g = stats.by_horizon.iter().find(|g| g.key == key).unwrap();
            assert_eq!(g.holding_days, Some(days), "{key} 持有期未取自权威天数表");
        }
        // 维度缺失档：没有持有期可言，必须是 None 而不是中线兜底
        let unk = stats.by_horizon.iter().find(|g| g.key == "unknown").unwrap();
        assert_eq!(unk.holding_days, None);
    }

    #[test]
    fn half_life_fits_exponential_ic_decay() {
        // 构造 IC(h)=0.6·e^(−h/20)：h=2/5/28/90 ⇒ λ=0.05 ⇒ t½=ln2/0.05≈13.86
        let pts: Vec<(f64, f64)> = [2.0_f64, 5.0, 28.0, 90.0]
            .iter()
            .map(|h| (*h, 0.6_f64 * (-h / 20.0_f64).exp()))
            .collect();
        let t_half = signal_half_life_days(&pts).unwrap();
        assert!((t_half - 13.8629).abs() < 0.01, "拟合半衰期偏离解析值：{t_half}");
    }

    #[test]
    fn half_life_refuses_to_invent_a_number() {
        // ① 不足 3 个点：两点连线必然完美，但不能卖成规律
        assert_eq!(signal_half_life_days(&[(2.0, 0.4), (5.0, 0.3)]), None);
        // ② IC 随持有期**上升**（无衰减）⇒ None，不是「半衰期无限长」
        let rising = vec![(2.0, 0.1), (5.0, 0.2), (28.0, 0.3), (90.0, 0.4)];
        assert_eq!(signal_half_life_days(&rising), None);
        // ③ 非正 IC 不进拟合（ln 无定义），只剩两点 ⇒ None
        let mixed = vec![(2.0, 0.4), (5.0, -0.1), (28.0, 0.1), (90.0, -0.2)];
        assert_eq!(signal_half_life_days(&mixed), None);
        // ④ 有衰减但噪声压过趋势（R² < 0.5）⇒ None
        let noisy = vec![(2.0, 0.02), (5.0, 0.5), (28.0, 0.01), (90.0, 0.4)];
        assert_eq!(signal_half_life_days(&noisy), None);
    }

    #[test]
    fn half_life_is_reported_only_from_tier_ic_that_actually_exists() {
        // 三档各 8 条，逐档 IC 由「置换距离」构造（无并列 ⇒ ρ = 1 − 6Σd²/(n(n²−1))）：
        // ultra_short 全对 ⇒ 1.0；short 三处相邻互换 Σd²=6 ⇒ 0.928571；
        // mid 四处互换 Σd²=24 ⇒ 0.714286。IC 随持有期衰减 ⇒ 拟合应给出正半衰期。
        let perms: [(&str, [usize; 8]); 3] = [
            ("ultra_short", [0, 1, 2, 3, 4, 5, 6, 7]),
            ("short", [1, 0, 3, 2, 5, 4, 6, 7]),
            ("mid", [1, 0, 3, 2, 7, 6, 5, 4]),
        ];
        let mut samples = Vec::new();
        for (h, perm) in perms {
            for (i, &ret) in perm.iter().enumerate() {
                let mut s = sample(Some("买入"), Some(h), 1, ret as f64);
                s.confidence = Some(i as f64);
                s.snr_anchor_days = Some(SNR_ANCHOR);
                samples.push(s);
            }
        }
        let stats = compute_reflection_stats(&samples);
        let want = [
            ("ultra_short", 1.0),
            ("short", 1.0 - 6.0 * 6.0 / 504.0),
            ("mid", 1.0 - 6.0 * 24.0 / 504.0),
        ];
        for (key, ic) in want {
            let g = stats.by_horizon.iter().find(|g| g.key == key).unwrap();
            let got = g.rank_ic.expect("该档 IC 应产出");
            assert!((got - ic).abs() < 1e-9, "{key} 档 IC 与闭式解不符：{got} vs {ic}");
        }
        assert_eq!(stats.usable_ic_tiers, 3);
        let t_half = stats.signal_half_life_days.expect("三档递减 IC 应拟合出半衰期");
        assert!(t_half > 0.0 && t_half < 200.0, "半衰期数量级不合理：{t_half}");

        // 只有一档够样本 ⇒ 拟合无从谈起，两个字段必须一起沉默
        let single: Vec<_> = (0..8).map(|i| conf_sample("short", i as f64, i as f64)).collect();
        let stats = compute_reflection_stats(&single);
        assert_eq!(stats.usable_ic_tiers, 1);
        assert_eq!(stats.signal_half_life_days, None);
    }

    #[test]
    fn action_groups_never_carry_holding_days() {
        // 半衰期的自变量是「持有期」，action 分组没有这个维度 ⇒ 逐条 None，
        // 防止把 horizon 的天数串味到 action 表上。
        let samples: Vec<_> = (0..8)
            .map(|i| {
                let mut s = conf_sample("short", i as f64 * 0.01, i as f64);
                s.action = Some("买入".into());
                s
            })
            .collect();
        let stats = compute_reflection_stats(&samples);
        let g = stats.by_action.iter().find(|g| g.key == "买入").unwrap();
        assert_eq!(g.holding_days, None);
        assert_eq!(g.rank_ic, Some(1.0));
    }

    /// IPC 契约：命中率 DTO 跨边界必须输出 camelCase（禁区 13）。
    /// 这两个 DTO 定义在 `analysis-engine`，而 `check-serde-annotations` 只扫
    /// `harness/` 与 `commands/` ⇒ 注解缺失不会有门禁报红，前端按 camelCase 读**恒为
    /// undefined**（表现为命中率卡永远「暂无数据」，而不是报错）。因此把键名逐条锁在
    /// 这里 —— 先于修复本测试必红，它就是那个缺失的检法。
    #[test]
    fn hitrate_dto_serializes_camel_case_for_ipc() {
        let stats = HitrateStats {
            total_samples: 9,
            direction_hit_rate: Some(0.5),
            target_hit_rate: Some(0.4),
            avg_raw_return_pct: Some(1.2),
            avg_alpha_pct: Some(0.3),
            legacy_samples: 2,
            by_action: vec![HitrateGroup {
                key: "买入".into(),
                samples: 5,
                direction_hit_rate: Some(0.6),
                rank_ic: Some(0.2),
                ic_samples: 8,
                ic_status: "ok".into(),
                ..Default::default()
            }],
            by_horizon: vec![HitrateGroup {
                key: "short".into(),
                samples: 5,
                holding_days: Some(5),
                ..Default::default()
            }],
            signal_half_life_days: Some(13.86),
            usable_ic_tiers: 3,
            // §五十一-②/§七十三 三字段：同样进 IPC ⇒ 必须一起锁 camelCase 键（见下方断言）
            generation_floor: 125,
            excluded_pre_floor_generation: 4,
            unknown_generation_samples: 7,
            failure_modes: Default::default(),
            // 阶段 3：层级看板同样进 IPC ⇒ 齐备态给生产形态（四类计数都有值、明细非空），
            // 只数键名不给值的话「行字段改名」这类断裂照样查不出来。
            tiers: TierBoard {
                total_rows: 12,
                l1: 4,
                l2: 3,
                l3: 1,
                unlabelled: 4,
                structural_rows: vec![StructuralReflectionRow {
                    reflection_id: "r-1".into(),
                    stock_code: "002696".into(),
                    stock_name: "百洋股份".into(),
                    horizon: Some("ultra_short".into()),
                    tier: "L2".into(),
                    proposal: Some("portfolio-mgr-h-ultra-short.rhai 进门门加相对强度分支".into()),
                    what_went_wrong: Some("近 5 日未涨停 ⇒ 进门门否掉，方向本身没判错".into()),
                    updated_at: 1_760_000_000_000,
                }],
            },
            // B4：齐备态值给**生产形态**（四格都有数、均值有值）—— 只在断言里数键、
            // 值给 Default 的话，「格子字段改了名/漏了」这类断裂照样查不出来。
            horizon_confusion: vec![HorizonConfusionRow {
                tier: "short".into(),
                tier_label: "短线".into(),
                both_correct: 3,
                formula_only: 2,
                llm_only: 1,
                both_wrong: 4,
                llm_undecided: 5,
                judged: 10,
                avg_horizon_agreement: Some(61.5),
                sufficient: true,
            }],
        };
        let value = serde_json::to_value(&stats).unwrap();
        let top = value.as_object().unwrap();
        for key in [
            "totalSamples",
            "directionHitRate",
            "targetHitRate",
            "avgRawReturnPct",
            "avgAlphaPct",
            "legacySamples",
            "byAction",
            "byHorizon",
            "signalHalfLifeDays",
            "usableIcTiers",
            "generationFloor",
            "excludedPreFloorGeneration",
            // P2-f：代际未知样本改「收缩纳入」后字段更名（未知条数如实报出）。
            "unknownGenerationSamples",
            // P2-b：失败模式看板随命中率一并出 IPC（面板零新增 invoke）。
            "failureModes",
            // B4：逐档结局 2×2 同批出 IPC（与命中率同源分母，面板零新增 invoke）。
            "horizonConfusion",
            // 阶段 3：层级看板（结构层待裁清单）同批出 IPC，面板同样零新增 invoke。
            "tiers",
        ] {
            assert!(top.contains_key(key), "HitrateStats 顶层缺 camelCase 键 {key}：{top:?}");
        }
        // 阶段 3：层级看板四类计数 + 明细行键逐条锁 —— `unlabelled` 必须是**独立一格**，
        // 被折进 `l1` 就是本轴引入前要消除的那类伪造（现网 79/80 条属此形态）。
        let tb = value["tiers"].as_object().unwrap();
        for key in ["totalRows", "l1", "l2", "l3", "unlabelled", "structuralRows"] {
            assert!(tb.contains_key(key), "TierBoard 缺 camelCase 键 {key}：{tb:?}");
        }
        assert_eq!(tb["unlabelled"].as_u64(), Some(4), "未标注条数必须单独报出，不得折进 L1");
        let sr = value["tiers"]["structuralRows"][0].as_object().unwrap();
        for key in [
            "reflectionId",
            "stockCode",
            "stockName",
            "horizon",
            "tier",
            "proposal",
            "whatWentWrong",
            "updatedAt",
        ] {
            assert!(sr.contains_key(key), "StructuralReflectionRow 缺键 {key}：{sr:?}");
        }
        // B4：2×2 的行级键同样逐条锁（少一个字段前端就是 undefined，格子会静默显示 0）
        let cell = value["horizonConfusion"][0].as_object().unwrap();
        for key in [
            "tier",
            "tierLabel",
            "bothCorrect",
            "formulaOnly",
            "llmOnly",
            "bothWrong",
            "llmUndecided",
            "judged",
            "avgHorizonAgreement",
            "sufficient",
        ] {
            assert!(cell.contains_key(key), "HorizonConfusionRow 缺 camelCase 键 {key}：{cell:?}");
        }
        assert!(!cell.contains_key("both_correct"), "2×2 行又出现 snake_case 键");
        let group = value["byHorizon"][0].as_object().unwrap();
        for key in [
            "key",
            "samples",
            "directionHitRate",
            "targetHitRate",
            "avgRawReturnPct",
            "avgAlphaPct",
            "rankIc",
            "icSamples",
            "icStatus",
            "icRegimeExcluded",
            "holdingDays",
        ] {
            assert!(group.contains_key(key), "HitrateGroup 缺 camelCase 键 {key}：{group:?}");
        }
        // 反向锁：snake_case 旧键名不得回来（那正是前端读不到的形态）
        assert!(!top.contains_key("direction_hit_rate"), "顶层又出现 snake_case 键");
        assert!(!group.contains_key("rank_ic"), "分组又出现 snake_case 键");
    }

    // ── 取数展开 helpers 的纯逻辑测试（2026-09-29 随实现一起从 commands 搬入）──
    #[test]
    fn find_bool_recursive_walks_nested_and_arrays() {
        let v: serde_json::Value = serde_json::json!({
            "report": {
                "items": [
                    { "name": "a" },
                    { "snapshot": { "target_reached": true } }
                ]
            }
        });
        assert_eq!(find_bool_recursive(&v, "target_reached"), Some(true));
        assert_eq!(find_bool_recursive(&v, "target_price"), None);
        let flat: serde_json::Value = serde_json::json!({ "target_reached": false });
        assert_eq!(find_bool_recursive(&flat, "target_reached"), Some(false));
        let non_bool: serde_json::Value = serde_json::json!({ "target_reached": "yes" });
        assert_eq!(find_bool_recursive(&non_bool, "target_reached"), None);
    }

    /// 四周期 JSON：ultra_short 成熟、short 未到期、mid 无行情、long 缺失（null）、
    /// 以及一个标量 `schemaVersion` 键（不是周期条目）。
    fn four_horizon_json() -> String {
        serde_json::json!({
            "ultra_short": {
                "status": "mature",
                "decision": { "action": "买入", "confidence": 62.0, "snrAnchorDays": 28 },
                "market": { "returnPct": 3.1, "alphaPct": 1.4, "targetReached": true },
                "evaluation": { "wasCorrect": 1 }
            },
            "short": {
                "status": "immature",
                "decision": { "action": "买入" },
                "market": { "returnPct": 0.4, "alphaPct": 0.1, "targetReached": false },
                "evaluation": { "wasCorrect": null }
            },
            "mid": {
                "status": "unavailable",
                "decision": { "action": "卖出" },
                "market": null,
                "evaluation": null
            },
            "long": null,
            "schemaVersion": 1
        })
        .to_string()
    }

    #[test]
    fn expand_only_mature_and_legacy_horizons() {
        let samples = expand_horizon_samples(&four_horizon_json(), None).unwrap();
        // 未到期 / 无行情 / 缺失周期一律不进样本，也不进命中率分母
        assert_eq!(samples.len(), 1);
        let s = &samples[0];
        assert_eq!(s.horizon.as_deref(), Some("ultra_short"));
        assert_eq!(s.action.as_deref(), Some("买入"));
        assert_eq!(s.was_correct, 1);
        assert_eq!(s.return_pct, 3.1);
        assert_eq!(s.alpha_pct, Some(1.4));
        assert_eq!(s.target_reached, Some(true));
        // IC 的预测侧：键名与产出方（`reflection.rs::build_horizon_results_json` 的
        // `"confidence": decision.confidence`）逐字对齐，口径是 0–100 不是 0–1。
        assert_eq!(s.confidence, Some(62.0));
        // 判定口径水印必须一路带到统计层（v104 前后 confidence 不同尺，IC 靠它筛样本）
        assert_eq!(s.snr_anchor_days, Some(28));
        assert_eq!(s.data_source, SampleDataSource::Reflection);
    }

    /// 旧记录 / 产出方漏写 confidence 时必须是 None ⇒ 该样本进命中率但**不进 IC 分母**，
    /// 由 `ic_status="no_confidence"` 点名。若这里兜底成 0，IC 会被一批假 0 拉成负相关 ——
    /// 把「拿不到」伪装成「测到 0」正是本轮要避免的形态。
    #[test]
    fn missing_confidence_stays_none_instead_of_zero_fill() {
        let json = serde_json::json!({
            "short": {
                "status": "mature",
                "decision": { "action": "买入" },
                "market": { "returnPct": 2.0 },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        let samples = expand_horizon_samples(&json, None).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].confidence, None);
        assert_eq!(samples[0].was_correct, 1);

        // 非有限值同样归 None（产出方写出 NaN/INF 时不进 IC）
        let nan = serde_json::json!({
            "short": {
                "status": "mature",
                "decision": { "action": "买入", "confidence": null },
                "market": { "returnPct": 2.0 },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        assert_eq!(expand_horizon_samples(&nan, None).unwrap()[0].confidence, None);
    }

    /// 水印的两种键名形态都要认（现网 camelCase、旧快照/手拼 snake_case），
    /// 而**缺水印**必须原样落 None —— 那是「上一代口径」的判据本身，兜底成 28 等于
    /// 把所有旧记录都说成当代样本，正是本轮要挡住的混算。
    #[test]
    fn snr_watermark_is_read_verbatim_in_both_key_forms_and_never_invented() {
        let camel = serde_json::json!({
            "mid": {
                "status": "mature",
                "decision": { "action": "买入", "confidence": 55.0, "snrAnchorDays": 28 },
                "market": { "returnPct": 1.5 },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        assert_eq!(expand_horizon_samples(&camel, None).unwrap()[0].snr_anchor_days, Some(28));

        let snake = camel.replace("snrAnchorDays", "snr_anchor_days");
        assert_eq!(expand_horizon_samples(&snake, None).unwrap()[0].snr_anchor_days, Some(28));

        let none = serde_json::json!({
            "mid": {
                "status": "mature",
                "decision": { "action": "买入", "confidence": 55.0 },
                "market": { "returnPct": 1.5 },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        let s = &expand_horizon_samples(&none, None).unwrap()[0];
        assert_eq!(s.confidence, Some(55.0), "没有水印不等于没有置信度，两者要分开表达");
        assert_eq!(s.snr_anchor_days, None);
    }

    #[test]
    fn expand_skips_neutral_action_and_missing_return() {
        let json = serde_json::json!({
            "ultra_short": {
                "status": "mature",
                "decision": { "action": "持有" },
                "market": { "returnPct": 1.0 },
                "evaluation": { "wasCorrect": null }
            },
            "short": {
                "status": "mature",
                "decision": { "action": "买入" },
                "market": { "returnPct": null },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        // 中性档不可判定、收益缺失不可伪报 0% → 两者都不成样本
        assert!(expand_horizon_samples(&json, None).unwrap().is_empty());
    }

    #[test]
    fn expand_marks_legacy_entries_and_rejects_non_object_json() {
        let json = serde_json::json!({
            "short": {
                "status": "legacy",
                "decision": { "action": "卖出" },
                "market": { "returnPct": -2.5, "targetReached": false },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        let samples = expand_horizon_samples(&json, None).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].data_source, SampleDataSource::Legacy);
        assert_eq!(samples[0].alpha_pct, None);
        assert_eq!(samples[0].target_reached, Some(false));

        assert!(expand_horizon_samples("[]", None).is_none());
        assert!(expand_horizon_samples("not-json", None).is_none());
    }

    /// §五十一-② 统计分母按**起算代际**筛样：`>= floor` 进分母，旧代与代际未知**分开计数**。
    ///
    /// 为什么这条必须有：分母变小是**静默**的（面板只看到样本数少了）。丢掉「被排除多少」，
    /// 就会出现「面板样本 0、没人知道为什么」—— 而「样本早于起算代」与「存量样本还没盖章」
    /// 的处置完全不同（前者等新样本积累或抬 floor，后者要补章/重跑）。
    ///
    /// ⚠ 判据必须是**下限**：夹具里专门放了 `floor - 1`、`floor`、`floor + 4` 三个值，
    /// 若有人把实现改成等号，`floor + 4` 那条会掉出分母 ⇒ 本用例当场红。
    #[test]
    fn generation_floor_keeps_at_or_after_and_counts_the_rest_separately() {
        let f = HORIZON_BRANCH_GENERATION_FLOOR;
        let samples = vec![
            sample_gen(Some("买入"), Some("short"), 1, 5.0, Some(f)), // 恰在起算代
            sample_gen(Some("买入"), Some("short"), 1, 3.0, Some(f + 4)), // 起算代之后（等号实现会误杀）
            sample_gen(Some("卖出"), Some("mid"), 0, -2.0, Some(f - 1)),  // 旧代
            sample_gen(Some("卖出"), Some("mid"), 1, 1.0, None),          // 代际未知
        ];
        let (kept, pre_floor, unknown) = filter_by_generation_floor(samples, f);
        // P2-f：代际未知样本**纳入分母**（收缩而非丢弃）⇒ kept = 2（>= floor）+ 1（None）。
        assert_eq!(kept.len(), 3, "floor 及其之后、以及代际未知的都必须留在分母里");
        assert_eq!((pre_floor, unknown), (1, 1), "两类计数仍如实分开");
        assert!(
            kept.iter()
                .filter(|s| s.template_version.is_some())
                .all(|s| s.template_version.is_some_and(|v| v >= f)),
            "留下的**已知代**样本都必须 >= 起算代"
        );
        assert!(
            kept.iter().any(|s| s.template_version.is_none()),
            "P2-f：代际未知样本必须进分母（收缩而非整批丢弃）"
        );

        // 负控：全部旧代 ⇒ 分母归 0，但**排除计数必须等于原样本数**（「为什么是 0」的唯一解释来源）。
        let all_old = vec![
            sample_gen(Some("买入"), Some("short"), 1, 5.0, Some(f - 5)),
            sample_gen(Some("买入"), Some("short"), 1, 3.0, Some(f - 1)),
        ];
        let (kept, pre_floor, unknown) = filter_by_generation_floor(all_old, f);
        assert!(kept.is_empty(), "旧代样本不得进分母");
        assert_eq!((pre_floor, unknown), (2, 0), "排除量必须如实等于被剔掉的样本数");
    }
    // ── B4：逐档结局 2×2 的纯函数判据 ──

    fn b4_sample(
        horizon: &str,
        formula: i32,
        llm: Option<i32>,
        agr: Option<i32>,
    ) -> DecisionPerformanceSample {
        let mut s = sample_gen(Some("买入"), Some(horizon), formula, 1.0, Some(200));
        s.llm_was_correct = llm;
        s.horizon_agreement = agr;
        s
    }

    /// 四格各归各位 + 「无从判定」单列（**不得**混进任何一格，尤其不得算成 LLM 判错）。
    #[test]
    fn confusion_quarters_are_distinct_and_undecided_is_not_a_wrong_cell() {
        let rows = build_horizon_confusion(&[
            b4_sample("mid", 1, Some(1), Some(60)),
            b4_sample("mid", 1, Some(0), None),
            b4_sample("mid", 0, Some(1), None),
            b4_sample("mid", 0, Some(0), None),
            // LLM 观望/持有 ⇒ 产端判 None（不是方向判断）⇒ 只能落 undecided
            b4_sample("mid", 1, None, None),
        ]);
        let mid = rows.iter().find(|r| r.tier == "mid").unwrap();
        assert_eq!(
            (mid.both_correct, mid.formula_only, mid.llm_only, mid.both_wrong),
            (1, 1, 1, 1)
        );
        assert_eq!(mid.llm_undecided, 1, "无从判定必须单列");
        assert_eq!(mid.judged, 4, "分母只数四格");
        assert_eq!(mid.tier_label, "中线", "档名口径跟随 harness::Period::label_zh");
        // 负控向：undecided 若被折进「LLM 判错」那两格，下面这条会红
        assert_eq!(
            mid.formula_only + mid.llm_only + mid.both_wrong + mid.both_correct,
            4,
            "四格相加不得含 undecided"
        );
    }

    /// 四档恒各出一行；「样本不足」按 `MIN_SAMPLE` 判，且边界值（恰 5 条）算够。
    #[test]
    fn confusion_always_has_four_rows_and_sufficiency_uses_min_sample_boundary() {
        let empty = build_horizon_confusion(&[]);
        assert_eq!(empty.len(), 4, "四档恒各一行，缺样本不是「没有这一档」");
        for r in &empty {
            assert_eq!(r.judged, 0);
            assert!(!r.sufficient, "0 条可判样本不得判成「够」");
            assert_eq!(r.avg_horizon_agreement, None, "无值给 None，不是 0 分");
        }
        let five: Vec<_> = (0..MIN_SAMPLE).map(|_| b4_sample("long", 1, Some(1), None)).collect();
        let rows = build_horizon_confusion(&five);
        let long = rows.iter().find(|r| r.tier == "long").unwrap();
        assert_eq!(long.judged, 5);
        assert!(long.sufficient, "恰达 MIN_SAMPLE({MIN_SAMPLE}) 即算够 —— 边界不许写成 >");
        assert!(!rows.iter().find(|r| r.tier == "short").unwrap().sufficient);
    }

    /// 三维分均值只收有值的行；一条都没有 ⇒ None（不是 0 分）。
    #[test]
    fn confusion_avg_agreement_divides_only_by_rows_that_have_a_value() {
        let rows = build_horizon_confusion(&[
            b4_sample("short", 1, Some(1), Some(60)),
            b4_sample("short", 0, Some(0), Some(70)),
            b4_sample("short", 1, None, None),
        ]);
        let short = rows.iter().find(|r| r.tier == "short").unwrap();
        assert_eq!(short.avg_horizon_agreement, Some(65.0), "分母只能是有值的那两条");
        assert_eq!(short.judged, 2, "第三条无从判定不进四格");
    }
}

/// §二 显式外键匹配的两种情形 + 两条负控
/// （2026-10-14，PLAN-reflection-optimization-closure）。
///
/// 关键前提：反思行的 `created_at` 是**建点时刻**、Gap1 sp 行的是**完成时刻**，
/// 二者在同一条反思路径上也就不相等 —— 旧键因此在现网 8 行里 0 命中。
/// 本套件把「外键命中」与「旧键回退」各钉一条，并锁住回退计数。
#[cfg(test)]
mod hitrate_join_tests {
    use super::match_sp_rows_to_reflections;
    // 阶段 3：层级看板的装配面（本 mod 按符号逐个 use，不加 `super::*`）
    use super::{build_tier_board, STRUCTURAL_PROPOSAL_CHARS, STRUCTURAL_ROWS_CAP};
    use axagent_entities::{stock_reflections, strategy_performance};

    fn sp_row(
        id: &str,
        code: &str,
        completed_at: i64,
        period: &str,
        reflection_id: Option<&str>,
    ) -> strategy_performance::Model {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "strategyId": "reflection_verdict",
            "period": period,
            "stockCode": code,
            "stockName": "测试股",
            "decisionAt": 1_700_000_000_000_i64,
            "exitAt": completed_at,
            "holdingDays": 28,
            "returnPct": 3.5,
            "wasCorrect": 1,
            "decisionConfidence": 60,
            "reflectionId": reflection_id,
            "createdAt": completed_at,
        }))
        .expect("strategy_performance 夹具必须能反序列化")
    }

    fn reflection_row(
        id: &str,
        code: &str,
        built_at: i64,
        horizon: Option<&str>,
    ) -> stock_reflections::Model {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "stockCode": code,
            "stockName": "测试股",
            "originalAnalysisId": "ana-1",
            "asOfDate": "2026-09-01",
            "hindsightDate": "2026-09-29",
            "minConfidenceThreshold": 50,
            "reflectionDepth": "light",
            "actualOutcome": "win",
            "source": "due",
            "status": "completed",
            "horizon": horizon,
            "createdAt": built_at,
            "updatedAt": built_at,
        }))
        .expect("stock_reflections 夹具必须能反序列化")
    }

    /// 阶段 3：层级看板装配 —— 四类计数互不折算、明细按新→旧、脏值按缺席处理。
    #[test]
    fn tier_board_counts_four_states_without_folding_absence_into_l1() {
        let mk = |id: &str, tier: Option<&str>, proposal: Option<&str>, at: i64| {
            let mut r = reflection_row(id, "002696", at, Some("ultra_short"));
            r.implementation_tier = tier.map(str::to_string);
            r.code_diff_proposal = proposal.map(str::to_string);
            r
        };
        let rows = [
            mk("r-l1", Some("L1"), None, 3_000),
            mk("r-l2", Some("L2"), Some("改 .rhai 进门门"), 5_000),
            mk("r-l3", Some("L3"), Some("改 reflection.md"), 4_000),
            mk("r-none", None, None, 1_000),
            // 脏值（模型自造 L9）在读侧同样走白名单 ⇒ 与缺席同桶，不得蒙进任何一层
            mk("r-junk", Some("L9"), None, 2_000),
        ];
        let refs: Vec<&stock_reflections::Model> = rows.iter().collect();
        let b = build_tier_board(&refs);
        assert_eq!(b.total_rows, 5);
        assert_eq!((b.l1, b.l2, b.l3), (1, 1, 1));
        // 缺席(NULL) 与脏值(L9) 各算一条 ⇒ 2，不是 3（5 行里 3 行有层级）
        assert_eq!(b.unlabelled, 2, "缺席与脏值都算未标注，不得折进 L1: {b:?}");
        assert_eq!(
            b.l1 + b.l2 + b.l3 + b.unlabelled,
            b.total_rows,
            "四格之和必须等于全集，有行被漏计: {b:?}"
        );
        assert_eq!(b.structural_rows.len(), 2, "只有 L2/L3 进待裁明细");
        assert_eq!(b.structural_rows[0].reflection_id, "r-l2", "明细按更新时间倒序");
        assert_eq!(b.structural_rows[0].tier, "L2");
    }

    /// 截断按**字符**（按字节会把一个汉字切成两半），且明细有上限而计数没有。
    #[test]
    fn tier_board_clips_by_chars_and_caps_rows() {
        let long: String = "测".repeat(400);
        let mut r = reflection_row("r-long", "002696", 9_000, Some("mid"));
        r.implementation_tier = Some("L2".into());
        r.code_diff_proposal = Some(long);
        let b = build_tier_board(&[&r]);
        let p = b.structural_rows[0].proposal.clone().expect("L2 行必须带提案");
        assert!(p.starts_with("测测") && p.ends_with('…'), "截断须以省略号收尾: {p}");
        assert_eq!(
            p.chars().count(),
            STRUCTURAL_PROPOSAL_CHARS + 1,
            "截断后应恰为上限字符数 + 省略号，实得 {}",
            p.chars().count()
        );

        let many: Vec<stock_reflections::Model> = (0..(STRUCTURAL_ROWS_CAP + 5))
            .map(|i| {
                let mut r = reflection_row(&format!("r-{i}"), "000001", i as i64, Some("mid"));
                r.implementation_tier = Some("L3".into());
                r
            })
            .collect();
        let refs: Vec<&stock_reflections::Model> = many.iter().collect();
        let b2 = build_tier_board(&refs);
        assert_eq!(b2.l3, STRUCTURAL_ROWS_CAP + 5, "计数不受明细上限影响");
        assert_eq!(b2.structural_rows.len(), STRUCTURAL_ROWS_CAP);
    }

    #[test]
    fn explicit_fk_matches_even_when_created_at_differs() {
        // 建点 1_000、完成 9_000 —— 旧时间戳键在这个情形下必然配不上
        let refs = vec![reflection_row("r-1", "600000", 1_000, Some("mid"))];
        let sps = vec![sp_row("s-1", "600000", 9_000, "reflection:mid", Some("r-1"))];
        let (matched, legacy) = match_sp_rows_to_reflections(&sps, &refs);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].map(|r| r.id.as_str()), Some("r-1"), "显式外键必须命中反思行");
        assert_eq!(legacy, 0, "走外键的行不得计入旧键回退数");
    }

    #[test]
    fn legacy_timestamp_key_still_matches_pre_fk_rows_but_is_counted() {
        // 存量行：reflection_id 为 NULL，两侧 created_at 恰好相同（老形态同批写入）
        let refs = vec![reflection_row("r-old", "600001", 5_000, Some("short"))];
        let sps = vec![sp_row("s-old", "600001", 5_000, "reflection:short", None)];
        let (matched, legacy) = match_sp_rows_to_reflections(&sps, &refs);
        assert_eq!(
            matched[0].map(|r| r.id.as_str()),
            Some("r-old"),
            "存量行仍要按旧键配上，否则历史命中率凭空清零"
        );
        assert_eq!(legacy, 1, "回退行必须被计数，供调用方告警");
    }

    /// 负控 A：外键悬空 ⇒ 走回退，且回退仍要按档区分，不得随机拿同键的第一行。
    #[test]
    fn dangling_fk_falls_back_and_still_distinguishes_horizon() {
        let refs = vec![
            reflection_row("r-a", "600002", 7_000, Some("mid")),
            reflection_row("r-b", "600002", 7_000, Some("long")),
        ];
        let sps = vec![sp_row("s-x", "600002", 7_000, "reflection:long", Some("nope"))];
        let (matched, legacy) = match_sp_rows_to_reflections(&sps, &refs);
        assert_eq!(legacy, 1, "悬空外键要走回退并计数");
        assert_eq!(matched[0].map(|r| r.id.as_str()), Some("r-b"), "回退须按档筛到对应行");
    }

    /// 负控 B：旧键丢档 = 命中率档间差异被抹平，这条必须拦得住。
    #[test]
    fn legacy_key_distinguishes_horizon_rows() {
        let refs = vec![
            reflection_row("r-mid", "600003", 8_000, Some("mid")),
            reflection_row("r-long", "600003", 8_000, Some("long")),
        ];
        let sps = vec![sp_row("s-long", "600003", 8_000, "reflection:long", None)];
        let (matched, legacy) = match_sp_rows_to_reflections(&sps, &refs);
        assert_eq!(legacy, 1);
        assert_eq!(matched[0].map(|r| r.id.as_str()), Some("r-long"));
    }
}
