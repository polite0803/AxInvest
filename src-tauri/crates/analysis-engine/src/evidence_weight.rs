//! 证据质量驱动的决策权重系统 (P0-1)
//!
//! 借鉴 TradingAgents-AShare 的"证据质量驱动决策"理念，废弃简单阈值投票，
//! 根据市场环境(regime)、投资周期(horizon)、历史表现(weight_decay)动态分配分析师权重。
//!
//! ## 核心设计
//!
//! 1. **三层权重融合**:
//!    - 市场周期层 (RegimeLayer): 牛市→技术面+资金面权重↑, 熊市→基本面+宏观权重↑
//!    - 时间维度层 (HorizonLayer): 短线→情绪+动量权重↑, 长线→价值权重↑
//!    - 历史表现层 (HistoryLayer): 从 weight_decay 模块获取的贝叶斯平滑后胜率权重
//!
//! 2. **BUY/SELL/HOLD 对称化门控**:
//!    - HOLD 必须满足: 技术面无趋势 + 资金面无方向 + 基本面/新闻面无催化剂
//!    - BUY/SELL 统一门槛: 任一维度有明确信号即必须选方向
//!
//! 3. **输出结构**:
//!    - `EvidenceWeightReport`: 包含每个分析师的最终权重、决策方向、置信度、门控条件

use axagent_harness::Period;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

// ── 常量定义 ──

/// 共识方向判定阈值：净得分占总权重比例超过该值才判定为明确方向（bullish/bearish）
pub const CONSENSUS_DIRECTION_THRESHOLD: f64 = 0.15;
/// 置信度映射基准（中性/分歧场景的基础置信度）
pub const CONSENSUS_CONFIDENCE_BASE: f64 = 30.0;
/// 方向明确时置信度缩放系数（净占比映射到 [BASE, DIR_MAX]）
pub const CONSENSUS_CONFIDENCE_DIR_SCALE: f64 = 70.0;
/// 方向明确时置信度上限
pub const CONSENSUS_CONFIDENCE_DIR_MAX: f64 = 95.0;
/// 分歧时置信度缩放系数（较强方占比映射到 [0, DIVIDED_MAX]）
pub const CONSENSUS_CONFIDENCE_DIVIDED_SCALE: f64 = 50.0;
/// 分歧时置信度上限
pub const CONSENSUS_CONFIDENCE_DIVIDED_MAX: f64 = 60.0;

/// **产出证据的分析师节点 id —— 全仓唯一权威清单。**
///
/// 为什么需要它（2026-10-03 实测）：本仓一度并存**三套**分析师 id 写法 ——
/// 逐档权重表用 `a-market`/`a-technical`/`capital`/`macro`/`fundamental`/`sentiment`、
/// 腿桥表用 `a-technical`/`capital`、`stock_workflow/decision.rs` 的 `expert_mapping` 用
/// 专家 id（`market-analyst` …），而运行时 `reports` 的键是**图里的节点 id**。
/// 逐档权重是 `horizon_weights.get(analyst_id)` **精确查表、无兜底** ⇒ 表里那些
/// 不存在的名字一律静默退 1.0：技术面与研报两档最要紧的权重差从未生效，
/// 而 `classify_role` 有 `contains` 后缀兜底，域分类看着正常，反而把查表落空盖住了。
///
/// 判据 = 「seed 里真有这样的 Agent 节点，且其输出进黑版 `report.*`」
/// （键形态来自 `crates/analysis-engine/src/blackboard.rs` 的
/// `id.starts_with("a-") => format!("report.{id}")`；`value-investor`/`research-mgr`
/// 两个证据方节点无 `a-` 前缀）。
///
/// ⚠ **不含 `a-macro`** —— 图里没有宏观分析师节点（宏观进决策流 = PLAN 的 P9-2，未做）。
/// 留着一个「有权重、无证据来源」的键，正是本清单要消灭的形态。
pub const EVIDENCE_ANALYST_IDS: &[&str] = &[
    "a-market-analyst",
    "a-fundamentals",
    "value-investor",
    "a-research",
    "research-mgr",
    "a-news",
    "a-catalyst",
    "a-sentiment",
    "a-hot-money",
    "a-lockup",
    "a-sector",
    "a-policy",
];

/// 历史名，内容等价于 [`EVIDENCE_ANALYST_IDS`]；新代码请用后者。
pub const ANALYST_IDS: &[&str] = EVIDENCE_ANALYST_IDS;

/// 把黑版报告键归一成裸节点 id（`report.a-fundamentals` → `a-fundamentals`）。
///
/// 权重表、腿桥表、前端表都按裸节点 id 索引；只有黑版键带 `report.` 前缀
/// （见 `crates/analysis-engine/src/blackboard.rs` 的 `report.{id}` 规则）。
/// 归一放在查表入口这一处，而不是让每个消费端各自 `replace`（历史上
/// `HistoricalAnalysisPanel` 就是自己 replace 了一次，别处全忘）。
fn analyst_key(id: &str) -> &str {
    id.strip_prefix("report.").unwrap_or(id)
}

/// 分析师**角色**分类（**不是**能力域 `CapabilityDomain`）——基本面/宏观/技术面/情绪/裁决，
/// 是投资域内部的分工粒度，不可复用为通用能力轴。划界见 `PLAN-domain-single-source.md` §5。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnalystRole {
    /// 基本面/价值
    Fundamental,
    /// 宏观/行业
    Macro,
    /// 技术面/市场
    Technical,
    /// 情绪/新闻/资金
    Sentiment,
    /// 综合裁决(Research Manager)
    Research,
}

fn classify_role(analyst_id: &str) -> AnalystRole {
    // B2-1 接线：先剥档位后缀再分类。裸 base id 与 `value-investor` / `research-mgr`
    // 这类无分隔符的名目会原样通过（`analyst_base_of` 对它们返回 `None`），所以今天的行为
    // 逐字不变；等带档分析师节点（`a-sentiment--mid`）出现后，这里不会静默落到后缀推断。
    let analyst_id =
        axagent_harness::holding_period::analyst_base_of(analyst_id).unwrap_or(analyst_id);
    match analyst_id {
        "a-fundamentals" | "value-investor" => AnalystRole::Fundamental,
        "a-sector" | "a-policy" => AnalystRole::Macro,
        "a-market-analyst" => AnalystRole::Technical,
        "a-sentiment" | "a-news" | "a-hot-money" | "a-lockup" | "a-catalyst" => {
            AnalystRole::Sentiment
        },
        "research-mgr" | "a-research" => AnalystRole::Research,
        _ => {
            // 未登记名：仍按关键词后缀推断（保持既有分类结果），但**必须留声** ——
            // 静默兜底正是「权重表里的幽灵 id」能活这么久的原因之一。
            tracing::warn!(
                "[evidence_weight] 分析师 '{analyst_id}' 不在 EVIDENCE_ANALYST_IDS 里，按后缀推断角色（新增分析师请同时登记该清单）"
            );
            if analyst_id.contains("fundamental") || analyst_id.contains("value") {
                AnalystRole::Fundamental
            } else if analyst_id.contains("macro") || analyst_id.contains("sector") {
                AnalystRole::Macro
            } else if analyst_id.contains("market") || analyst_id.contains("technical") {
                AnalystRole::Technical
            } else {
                AnalystRole::Sentiment
            }
        },
    }
}

// ── 输入结构 ──

/// 市场环境信息（来自 stock-analysis 的 market_regime.rs 或 astock-data 的 regime.rs）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketRegimeInfo {
    /// "bull" / "bear" / "sideways" / "volatile"
    pub regime: String,
    /// 置信度 0-1
    pub confidence: f64,
    /// "high" / "low" / "normal"
    pub volatility: String,
    /// 可读描述
    pub description: String,
    /// 20 日年化波动率(%)
    pub volatility_pct: Option<f64>,
    /// 连续上涨天数（前端可选，缺失时视为 0）
    #[serde(default)]
    pub consecutive_up: Option<i32>,
    /// 连续下跌天数（前端可选，缺失时视为 0）
    #[serde(default)]
    pub consecutive_down: Option<i32>,
}

/// 单个分析师的运行时信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalystInput {
    /// 分析师 ID
    pub analyst_id: String,
    /// 该分析师的原始报告文本（用于情感分类）
    pub report_text: Option<String>,
    /// 结构化输出的立场（如果有）
    pub stance: Option<String>,
    /// 该分析师的 bull_score (0-10)
    pub bull_score: Option<f64>,
    /// 该分析师的 bear_score (0-10)
    pub bear_score: Option<f64>,
    /// 建议仓位 (0-100)
    pub position_pct: Option<f64>,
    /// 不可信输出标记（来自上游解析失败/幻觉检测）
    #[serde(default)]
    pub untrusted: Option<bool>,
}

/// 证据权重计算请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceWeightRequest {
    /// 市场环境
    pub market_regime: MarketRegimeInfo,
    /// 投资周期: "ultra_short" | "short" | "mid" | "long"
    pub time_horizon: String,
    /// 各分析师输入
    pub analysts: Vec<AnalystInput>,
    /// 历史表现权重（来自 weight_decay 模块）(analyst_id, period) → adjusted_weight
    pub historical_weights: Option<HashMap<String, f64>>,
}

// ── 输出结构 ──

/// 单个分析师的最终权重
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalystWeight {
    pub analyst_id: String,
    /// 领域分类
    pub domain: String,
    /// 时间维度权重
    pub horizon_weight: f64,
    /// 市场周期调节系数
    pub regime_modifier: f64,
    /// 历史表现系数
    pub history_modifier: f64,
    /// 最终合成权重
    pub final_weight: f64,
    /// 该分析师的立场方向
    pub stance_direction: String,
    /// 该分析师的分析置信度（基于报告内容）
    pub stance_confidence: f64,
    /// 是否为不可信输出（解析失败/幻觉/上游标记）
    pub is_untrusted: bool,
}

/// 共识结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceConsensus {
    /// 加权 bullish 总分
    pub bullish_score: f64,
    /// 加权 bearish 总分
    pub bearish_score: f64,
    /// 加权 neutral 总分
    pub neutral_score: f64,
    /// 总权重
    pub total_weight: f64,
    /// 净得分 (bullish - bearish)
    pub net_score: f64,
    /// "bullish" | "bearish" | "neutral" | "divided"
    pub consensus: String,
    /// 置信度 0-100
    pub confidence: f64,
}

/// BUY/SELL/HOLD 门控条件检查结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldGateResult {
    /// HOLD 是否被允许
    pub hold_allowed: bool,
    /// 原因
    pub reason: String,
    /// 技术面是否有趋势
    pub technical_has_trend: bool,
    /// 资金面是否有方向
    pub moneyflow_has_direction: bool,
    /// 基本面/新闻面是否有催化剂
    pub fundamental_has_catalyst: bool,
    /// 建议动作: "BUY" | "SELL" | "HOLD" | "FORCE_DIRECTION"
    pub suggested_action: String,
}

/// 完整证据权重报告
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceWeightReport {
    /// 市场环境
    pub market_regime: MarketRegimeInfo,
    /// 投资周期
    pub time_horizon: String,
    /// 各分析师权重详情
    pub analyst_weights: Vec<AnalystWeight>,
    /// 加权共识
    pub consensus: EvidenceConsensus,
    /// HOLD 门控
    pub hold_gate: HoldGateResult,
    /// 推荐决策
    pub recommended_action: String,
    /// 推荐仓位百分比
    pub recommended_position_pct: f64,
    /// 整体置信度
    pub overall_confidence: f64,
}

// ── 核心计算 ──

/// 时间维度基础权重表 (与前端 ANALYST_TIME_HORIZON_WEIGHT 对应)
///
/// ⚠ 四档必须**逐档显式**：`mid` 曾靠 `_` 兜底命中，
/// 新增档位或脏值都会静默套用中线权重（〇-B G4「退化即声明」）。
fn get_horizon_base_weights(horizon: &str) -> HashMap<&'static str, f64> {
    // ⚠ 键必须是 [`EVIDENCE_ANALYST_IDS`] 里的**节点 id**。历史上这里写的是
    //   `a-market`/`a-technical`/`capital`/`macro`/`fundamental`/`sentiment` —— 图中无此节点，
    //   精确查表恒落空 ⇒ 技术面与研报的档间权重从未生效（静默 1.0）。
    // ⚠ 值为 1.0 的条目是**显式声明「本档不偏置该证据」**，不是「没想过」：
    //   `a-catalyst`/`a-lockup`/`a-policy`/`a-research` 历史上压根不在表里，
    //   本次统一 id 时不替它们编造档间差异（要出数值，等 P6/P7 由命中率反推）。
    let mut w = HashMap::new();
    match horizon {
        "ultra_short" => {
            w.insert("a-hot-money", 2.0);
            w.insert("a-sentiment", 1.5);
            w.insert("a-news", 1.5);
            w.insert("a-market-analyst", 1.3);
            w.insert("research-mgr", 0.5);
            w.insert("a-sector", 0.5);
            w.insert("a-fundamentals", 0.3);
            w.insert("value-investor", 0.3);
            w.insert("a-catalyst", 1.0);
            w.insert("a-lockup", 1.0);
            w.insert("a-policy", 1.0);
            w.insert("a-research", 1.0);
        },
        "short" => {
            w.insert("a-market-analyst", 1.5);
            w.insert("a-hot-money", 1.5);
            w.insert("a-sentiment", 1.3);
            w.insert("a-news", 1.2);
            w.insert("a-fundamentals", 0.6);
            w.insert("value-investor", 0.5);
            w.insert("a-sector", 0.8);
            w.insert("research-mgr", 1.0);
            w.insert("a-catalyst", 1.0);
            w.insert("a-lockup", 1.0);
            w.insert("a-policy", 1.0);
            w.insert("a-research", 1.0);
        },
        // 中线：四档中唯一接近「不加权」的基准档，仍逐条显式列出不靠 `_` 兜底
        "mid" => {
            w.insert("a-fundamentals", 1.2);
            w.insert("value-investor", 1.2);
            w.insert("a-sector", 1.1);
            w.insert("research-mgr", 1.1);
            w.insert("a-market-analyst", 1.0);
            w.insert("a-sentiment", 1.0);
            w.insert("a-news", 1.0);
            w.insert("a-hot-money", 0.9);
            w.insert("a-catalyst", 1.0);
            w.insert("a-lockup", 1.0);
            w.insert("a-policy", 1.0);
            w.insert("a-research", 1.0);
        },
        "long" => {
            w.insert("value-investor", 2.0);
            w.insert("a-fundamentals", 1.5);
            w.insert("research-mgr", 1.5);
            w.insert("a-sector", 1.2);
            w.insert("a-market-analyst", 0.6);
            w.insert("a-news", 0.7);
            w.insert("a-sentiment", 0.7);
            w.insert("a-hot-money", 0.5);
            w.insert("a-catalyst", 1.0);
            w.insert("a-lockup", 1.0);
            w.insert("a-policy", 1.0);
            w.insert("a-research", 1.0);
        },
        // 未知周期：按中线基准，但**必须留可检痕迹**（日志/面板可区分声明与兜底）
        _ => {
            tracing::warn!("[evidence_weight] 未知持有周期 '{horizon}'，按中线基准加权");
            return get_horizon_base_weights("mid");
        },
    }
    w
}

/// 决策腿（`portfolio-mgr.rhai` 的 f1~f13）→ 分析师 id 的**唯一桥表**。
///
/// **它现在的用途（2026-10-04 R-11 之后）**：登记表，不是投影表。原先它被
/// `horizon_leg_multipliers()` 用来把「按分析师索引的周期权重」乘到「按因子腿融合的
/// 决策脚本」上 —— 那张乘数表连同投影一起删除了（「同一批腿 × 四个标量」正是 R-11 判为
/// 构造性错误的那个形态）。留下来的这份属主映射仍然承重，因为它锁的是两条与算法无关的
/// 结构不变量：① 每条参与融合的腿必须有**唯一**的证据域属主（否则同一域被计两次方向，
/// 见下面 f3/f10 两处实证错位）；② 桥到的分析师 ∪ 显式登记「无腿」的分析师 = 权威清单
/// （`scripts/check-horizon-weight-parity.mjs` 判据 ⑦⑧ + 本文件的同名闭合测试）。
///
/// `None` 的腿**不是分析师证据**（风险分类、数据质量、trader 观点都是元约束），
/// 不得为了「看起来四档有差异」硬塞一个分析师进去。
///
/// **归因口径（2026-10-03 逐腿核对 `seed_stock_analysis.rs` 的 portfolio-mgr
/// `input_mapping` 后订正）**：腿的信号多数由**算法/工具节点**产出（`t-scoring`、
/// `t-valuation`、`t-lockup-data`、`pace-calc`、`serenity_context`），LLM 分析师并不
/// 直接产出该变量。因此桥按**证据域唯一属主**归因 —— 该域在 12 个分析师里有且仅有一个
/// 职责相符的属主时桥到它（否则逐档权重无从作用），无属主 ⇒ `None`。
/// id 统一后这些归因**会真的改变乘数**：此前两侧 id 对不上 ⇒ 全表恒 1.0、写错也无人察觉，
/// 所以订正不是洁癖而是行为变更的前提。两处实证错位：
/// - `f3` 的变量是 `catalyst_level`，路径 `a-catalyst.content.verdict.catalyst_level`
///   ⇒ 属主 `a-catalyst`（催化剂与叙事完整度），原写 `a-news`（消息面评估，另一节点）。
/// - `f10` 的变量是 `lockup_bundle`，来自 `t-lockup-data`（解禁 + 增减持 + 大宗 + 股东户数）
///   ⇒ 属主 `a-lockup`（解禁减持与质押风险排查），原写 `a-hot-money`（那是 `f9` 的
///   `money_flow` 域）⇒ 旧形态把资金流权重同时套在两条腿上，等于给同一域计了两次方向。
pub const DECISION_LEG_ANALYST: &[(&str, Option<&str>)] = &[
    ("f1", Some("a-market-analyst")), // 技术面 / 趋势（历史上写 `a-technical`，图里无此节点）
    ("f2", Some("research-mgr")),     // 共识裁决域属主（变量本身出自 debate-convergence）
    ("f3", Some("a-catalyst")),       // 催化剂等级（原误写 a-news）
    ("f4", None),                     // 风险分类（元约束）
    ("f5", Some("value-investor")),   // 估值（DCF / 格雷厄姆 / PE 分位腿）
    ("f6", None),                     // 数据质量（元约束）
    ("f7", None),                     // trader 观点（不属于四档分析师证据）
    ("f9", Some("a-hot-money")),      // 资金流 `get_stock_money_flow`（历史上写 `capital`）
    ("f10", Some("a-lockup")),        // 筹码面（原误归 a-hot-money）
    ("f11", Some("a-sentiment")),     // PACE 情绪
    ("f12", Some("a-market-analyst")), // 动量（技术侧）
    ("f13", Some("a-sector")),        // 产业链瓶颈（serenity 合成分，行业/链域唯一属主）
];

/// **当前没有决策腿**的分析师 —— 显式登记，不许由「桥表里查不到」静默表达。
///
/// 与 [`DECISION_LEG_ANALYST`] 的并集必须等于 [`EVIDENCE_ANALYST_IDS`]（测试
/// `bridged_plus_unbridged_covers_every_analyst` 锁闭合）。存在理由：id 统一后逐档权重
/// 真的会作用到桥上 ⇒「不在桥上」= 该分析师的证据对贝叶斯融合**零影响**，这是结论而不是缺省。
/// 每一项都写明它的路在何处，或为什么刻意不做：
/// - `a-fundamentals`（PE/PB/ROE 财务质量）：P4′ 九因子中的 `earningsQuality` 承载
///   （见 `harness::holding_period::Period::verdict_spec`），本档之后才有腿。
/// - `a-research`（券商研报观点汇总）：P4′ 的 `expectationRevision`。
/// - `a-policy`（宏观政策影响）：P4′ 的 `macroRegime`（其数据侧已由 P9-1 五条真序列供上）。
/// - `a-news`（新闻公告影响评估）：**刻意无腿** —— 公告证据的方向通道已由 `a-catalyst`
///   的 `f3` 承载（两节点读同一份 `t-catalyst-data`，见 `portfolio-mgr.rhai` 的
///   f3/f11 协方差衰减注释：同域双桥就是重复计数）。
pub const UNBRIDGED_ANALYST_IDS: &[&str] = &["a-fundamentals", "a-research", "a-policy", "a-news"];

/// 因子 → 数值来源通道（P4′ 子图的 `input_mapping` 必须满足这张表的声明）。
///
/// 三种语义，**不是**「模型想不想给」而是「数字从哪儿拿」：
/// - `tierScore`：该档**自己的**评分节点（`t-scoring-hour|week|month|quarter`）——
///   这是 R-11「逐档取该档数据」的正解，跨档共用一份评分正是被退役的形态；
/// - `sharedTool`：与持有尺度无关的确定性工具输出（涨停池 / 估值带 / 一致预期 / 宏观快照 /
///   解禁包 / 行业排名），四档共用同一次取数，但**只有该档参与该因子时**才进融合；
/// - `verdict`：只有 LLM 分析师能给的判断（资金持续性、催化剂等级），由 P3′ 注入的逐档契约
///   要求该分析师必须输出该字段。
///
/// `microstructure`（封单额 / 炸板次数）与 `breadthState`（家数 / 封板率）都指到
/// `get_limit_up_pool` —— 同一份响应的**两组不同字段**，不是一个因子的两次计数（域属主也不同：
/// 封单是资金行为、家数是情绪状态）。
pub const VERDICT_FACTOR_SOURCES: &[(&str, &str, &str)] = &[
    ("momentumSignal", "tierScore", "macd"),
    ("trendStrength", "tierScore", "totalScore"),
    ("microstructure", "sharedTool", "get_limit_up_pool"),
    ("breadthState", "sharedTool", "get_limit_up_pool"),
    ("flowPersistence", "verdict", ""),
    ("eventCatalyst", "verdict", ""),
    ("valuationBand", "sharedTool", "t-valuation-band"),
    ("earningsQuality", "sharedTool", "t-valuation"),
    ("expectationRevision", "sharedTool", "get_consensus_eps"),
    ("macroRegime", "sharedTool", "macro_data_snapshot"),
    ("supplyShock", "sharedTool", "t-lockup-data"),
    ("sectorRotation", "sharedTool", "get_industry_ranking"),
];

/// 波动带统计窗口（交易日）。**唯一权威在这里**：
///
/// 主链 `src/commands/portfolio-mgr.rhai` 里有一份同名的脚本内常量（`let VOL_LOOKBACK_DAYS = 20;`），
/// 两份数字必须永远相等 —— 由 `seed_consistency_tests::main_chain_vol_lookback_matches_injected_const`
/// 逐字比对钉住（本仓的教训：两处一致地不同，比一处缺失更难发现）。
pub const VOL_LOOKBACK_DAYS: i64 = 20;

/// 该档分支的置信推导口径名（P4′-b 的四份决策脚本各自实现其一）。
///
/// 四档不同**不是**为了看起来不同：每一档的「怎么算赢」由它的出场口径决定，
/// 置信与出场必须同族，否则胜率与止损来自两套假设。
/// - `edge_no_time_scaling`：2 日窗内不做跨期缩放（√h 类变换在持有期短于噪声半衰期时无意义）；
/// - `dual_confirmation`：趋势与资金持续性**同号**才允许加仓（联合门，不是加权平均）；
/// - `sigma_band_position`：价格在 k·σ 带内的位置决定置信与出场（波动带法）；
/// - `margin_of_safety`：估值带 / 安全边际定置信，出场为「到达目标 或 论点被证伪」。
fn confidence_method(p: Period) -> &'static str {
    match p {
        Period::UltraShort => "edge_no_time_scaling",
        Period::Short => "dual_confirmation",
        Period::Mid => "sigma_band_position",
        Period::Long => "margin_of_safety",
    }
}

/// 该档的**入场必要条件门**（不是权重，是「不满足就不许出买入」的结构门）。
fn entry_gate(p: Period) -> &'static str {
    match p {
        // 超短：情绪广度（涨停家数 / 封板率）不达标时，个股形态再好也不给出买入。
        Period::UltraShort => "breadth_required",
        // 短：趋势与资金同向；中：σ 带内才允许建仓；长：必须有正的安全边际。
        Period::Short => "trend_and_flow_same_sign",
        Period::Mid => "inside_sigma_band",
        Period::Long => "positive_margin_of_safety",
    }
}

/// 注入 Rhai 的**逐档分支表**：`{ultra_short: {legs:[…], analysts:[…], …}, …}`。
///
/// 与已删除的乘数表（`leg_mult` / 「同一批腿 × 四个标量」）的区别就是 R-11 的区别：
/// 本表是「每档**自己的腿集合**、每腿自己的角色与来源」。数值仍然全部派生自
/// [`get_horizon_base_weights`]（腿间配比 = 该档分析师偏置在**本档方向腿**上的归一化），
/// 本函数不新增任何一个数 —— 手抄第二份权重就是 §二十四 那批缺陷的成因。
///
/// 腿的 `role`：`direction`（进方向加权）/ `filter`（只作入场时机过滤）/
/// `riskNote`（只作风险提示）/ `partial`（有信息但来源不全覆盖，不进方向）。
/// **非 `direction` 的腿权重恒为 0.0**，并带 `constraint` 说明凭什么打折 —— 否则
/// 「它参与了」与「它只是被看了看」在面板上长得一样。
pub fn horizon_branch_specs() -> serde_json::Value {
    let mut tiers = serde_json::Map::new();
    for p in Period::ALL {
        let spec = p.verdict_spec();
        let bias = get_horizon_base_weights(p.as_str());
        let constraint_of = |f: &str| -> Option<&'static str> {
            spec.qualified.iter().find(|(qf, _)| *qf == f).map(|(_, k)| *k)
        };

        // 本档的腿 = 进方向加权的因子 ∪ 只作过滤/提示/部分来源的条件因子（后者 role 非 direction）
        let direction = spec.participating.clone();
        let filtered_only: Vec<&'static str> =
            spec.qualified.iter().map(|(qf, _)| *qf).filter(|qf| !direction.contains(qf)).collect();

        let mut legs: Vec<serde_json::Value> = Vec::new();
        for (f, is_direction) in direction
            .iter()
            .copied()
            .map(|f| (f, true))
            .chain(filtered_only.into_iter().map(|f| (f, false)))
        {
            let owner = Period::factor_owner(f);
            let source = VERDICT_FACTOR_SOURCES.iter().find(|(cf, _, _)| *cf == f);
            let (Some(owner), Some((_, kind, channel))) = (owner, source) else {
                tracing::warn!(
                    "[evidence_weight] 分支表：因子 {f} 缺属主或缺来源登记 ⇒ 该腿不进融合（档 {}）",
                    p.as_str()
                );
                continue;
            };
            let b = match bias.get(owner) {
                Some(v) => *v,
                None => {
                    tracing::warn!(
                        "[evidence_weight] 档 {} 权重表缺分析师 '{owner}'（因子 {f}）⇒ 配比按 1.0",
                        p.as_str()
                    );
                    1.0
                },
            };
            let constraint = constraint_of(f);
            legs.push(serde_json::json!({
                "factor": f,
                "owner": owner,
                "role": if is_direction {
                    "direction"
                } else {
                    role_of_marker(constraint.unwrap_or(""))
                },
                "bias": b,
                "weight": b,
                "source": { "kind": kind, "channel": channel },
                "constraint": constraint,
            }));
        }

        // 腿间配比只归一化**方向腿**；其余腿权重置 0（角色已在 role 里声明）
        let sum: f64 = legs
            .iter()
            .filter(|l| l["role"] == "direction")
            .filter_map(|l| l["weight"].as_f64())
            .sum();
        if sum <= 0.0 {
            tracing::warn!(
                "[evidence_weight] 档 {} 方向腿配比合计为 0 ⇒ 该档分支不出结论（不得静默退等权）",
                p.as_str()
            );
        }
        let direction_sum_ok = sum > 0.0;

        for l in legs.iter_mut() {
            if l["role"] == "direction" && direction_sum_ok {
                let w = l["weight"].as_f64().unwrap_or(0.0) / sum;
                l["weight"] = serde_json::Value::from((w * 1e6).round() / 1e6);
            } else if l["role"] != "direction" {
                l["weight"] = serde_json::Value::from(0.0);
            }
        }

        tiers.insert(
            p.as_str().to_string(),
            serde_json::json!({
                "tier": p.as_str(),
                "days": p.default_holding_days(),
                "volLookbackDays": crate::evidence_weight::VOL_LOOKBACK_DAYS,
                "positionMultiplier": p.position_multiplier(),
                "exitRule": spec.exit_rule,
                "confidenceMethod": confidence_method(p),
                "entryGate": entry_gate(p),
                // 方向腿配比合计为 0 时置真：脚本必须据此**不出结论**（点名的退化），
                // 而不是退成等权 —— 等权就是拿简化填架构缺口。
                "degraded": !direction_sum_ok,
                "analysts": p.analyst_subset(),
                "legs": legs,
            }),
        );
    }
    serde_json::Value::Object(tiers)
}

/// 标记键 → 腿角色（`qualified` 里「只作过滤 / 只作风险提示」与「来源不全」的三分）。
fn role_of_marker(key: &str) -> &'static str {
    match key {
        "trendFilterOnly" => "filter",
        "supplyAsRiskNoteOnly" => "riskNote",
        _ => "partial",
    }
}

/// 计算市场周期调节系数
///
/// 核心逻辑:
/// - **牛市**: 技术面+资金面权重显著提升 (趋势跟踪有效)，基本面+宏观轻微提升
/// - **熊市**: 基本面+宏观权重显著提升 (防御价值凸显)，技术面+情绪面被削弱
/// - **高波动**: 所有 domain 降低权重，风控优先
/// - **震荡市**: 基本面+情绪面权重提升 (精选个股+预期差)，技术面中性
fn compute_regime_modifiers(regime: &MarketRegimeInfo) -> HashMap<AnalystRole, f64> {
    let mut modifiers = HashMap::new();

    let vol_penalty = match regime.volatility.as_str() {
        "high" => 0.85, // 高波动 → 所有 domain ×0.85
        "low" => 1.05,  // 低波动 → 轻微提升
        _ => 1.0,
    };

    match regime.regime.as_str() {
        "bull" => {
            modifiers.insert(AnalystRole::Technical, 1.30 * vol_penalty);
            modifiers.insert(AnalystRole::Sentiment, 1.20 * vol_penalty);
            modifiers.insert(AnalystRole::Fundamental, 1.10 * vol_penalty);
            modifiers.insert(AnalystRole::Macro, 1.05 * vol_penalty);
            modifiers.insert(AnalystRole::Research, 1.05 * vol_penalty);
        },
        "bear" => {
            modifiers.insert(AnalystRole::Fundamental, 1.35 * vol_penalty);
            modifiers.insert(AnalystRole::Macro, 1.30 * vol_penalty);
            modifiers.insert(AnalystRole::Research, 1.20 * vol_penalty);
            modifiers.insert(AnalystRole::Technical, 0.80 * vol_penalty);
            modifiers.insert(AnalystRole::Sentiment, 0.75 * vol_penalty);
        },
        "volatile" => {
            // 高波动: 全 domain 降权
            modifiers.insert(AnalystRole::Fundamental, 0.80);
            modifiers.insert(AnalystRole::Macro, 0.85);
            modifiers.insert(AnalystRole::Technical, 0.70);
            modifiers.insert(AnalystRole::Sentiment, 0.65);
            modifiers.insert(AnalystRole::Research, 0.90);
        },
        // sideways / 震荡: 精选个股模式
        _ => {
            modifiers.insert(AnalystRole::Fundamental, 1.15 * vol_penalty);
            modifiers.insert(AnalystRole::Sentiment, 1.10 * vol_penalty);
            modifiers.insert(AnalystRole::Research, 1.10 * vol_penalty);
            modifiers.insert(AnalystRole::Macro, 1.00 * vol_penalty);
            modifiers.insert(AnalystRole::Technical, 0.95 * vol_penalty);
        },
    }

    modifiers
}

/// 从分析师的报告文本提取立场方向
///
/// 返回三元组: (direction, confidence, is_untrusted)
/// - is_untrusted=true 表示无可信结构化数据，立场为回退推断，不应参与仓位贡献
fn extract_stance(analyst: &AnalystInput) -> (String, f64, bool) {
    // 优先使用结构化字段
    if let Some(ref stance) = analyst.stance {
        let lower = stance.to_lowercase();
        if lower.contains("买")
            || lower.contains("多")
            || lower.contains("涨")
            || lower.contains("bull")
            || lower.contains("buy")
            || lower.contains("乐观")
            || lower.contains("上行")
            || lower.contains("流入")
        {
            return ("bullish".into(), 0.8, false);
        }
        if lower.contains("卖")
            || lower.contains("空")
            || lower.contains("跌")
            || lower.contains("bear")
            || lower.contains("sell")
            || lower.contains("悲观")
            || lower.contains("下行")
            || lower.contains("流出")
        {
            return ("bearish".into(), 0.8, false);
        }
        if lower.contains("中性")
            || lower.contains("观望")
            || lower.contains("持有")
            || lower.contains("hold")
            || lower.contains("neutral")
            || lower.contains("震荡")
        {
            return ("neutral".into(), 0.7, false);
        }
    }

    // 使用 bull_score / bear_score
    if let (Some(bs), Some(bs2)) = (analyst.bull_score, analyst.bear_score) {
        if bs > bs2 {
            return ("bullish".into(), ((bs - bs2) / 10.0).min(0.9), false);
        }
        if bs2 > bs {
            return ("bearish".into(), ((bs2 - bs) / 10.0).min(0.9), false);
        }
        return ("neutral".into(), 0.5, false);
    }

    // 使用仓位建议
    if let Some(pct) = analyst.position_pct {
        if pct >= 6.0 {
            return ("bullish".into(), (pct / 100.0).min(0.9), false);
        }
        if pct < 0.0 {
            return ("bearish".into(), 0.7, false);
        }
        return ("neutral".into(), 0.5, false);
    }

    // 没有结构化数据 → 从报告文本做简单情感分类
    if let Some(ref text) = analyst.report_text {
        let lower = text.to_lowercase();
        let mut bull_count = 0;
        let mut bear_count = 0;

        let bull_kw =
            ["买入", "增持", "看多", "看涨", "利好", "上涨", "bull", "buy", "增长", "改善"];
        let bear_kw =
            ["卖出", "减持", "看空", "看跌", "利空", "下跌", "bear", "sell", "下滑", "恶化"];

        for kw in &bull_kw {
            if lower.contains(kw) {
                bull_count += 1;
            }
        }
        for kw in &bear_kw {
            if lower.contains(kw) {
                bear_count += 1;
            }
        }

        if bull_count > bear_count {
            let conf = 0.5 + (bull_count as f64 - bear_count as f64) * 0.05;
            return ("bullish".into(), conf.min(0.85), false);
        }
        if bear_count > bull_count {
            let conf = 0.5 + (bear_count as f64 - bull_count as f64) * 0.05;
            return ("bearish".into(), conf.min(0.85), false);
        }
    }

    // 无任何可信数据 → 标记为不可信，position_pct 应为 0
    ("neutral".into(), 0.4, true)
}

/// 检查 HOLD 门控条件
///
/// 借鉴 TradingAgents 的"对称化门控"逻辑:
/// HOLD 仅当**同时满足**以下三个条件时才允许:
/// 1. 技术面无明确趋势 (技术面分析师为 neutral)
/// 2. 资金面无明确方向 (资金面/情绪面分析师为 neutral)
/// 3. 基本面/新闻面无催化剂 (基本面/新闻面分析师为 neutral)
///
/// 任一条件不满足 → 必须选 BUY 或 SELL
fn check_hold_gate(analysts: &[AnalystWeight]) -> HoldGateResult {
    let mut tech_has_trend = false;
    let mut money_has_dir = false;
    let mut fund_has_catalyst = false;

    for a in analysts {
        let domain = classify_role(&a.analyst_id);
        match domain {
            AnalystRole::Technical
                if a.stance_direction != "neutral" && a.stance_confidence > 0.5 =>
            {
                tech_has_trend = true;
            },
            AnalystRole::Sentiment
                if a.stance_direction != "neutral" && a.stance_confidence > 0.5 =>
            {
                money_has_dir = true;
            },
            AnalystRole::Fundamental | AnalystRole::Macro
                if (a.stance_direction == "bullish" || a.stance_direction == "bearish")
                    && a.stance_confidence > 0.5 =>
            {
                fund_has_catalyst = true;
            },
            _ => {},
        }
    }

    let hold_allowed = !tech_has_trend && !money_has_dir && !fund_has_catalyst;

    let (reason, suggested_action) = if hold_allowed {
        ("技术面无趋势 + 资金面无方向 + 基本面无催化剂 → HOLD 允许".into(), "HOLD".into())
    } else if tech_has_trend && money_has_dir {
        (
            format!(
                "技术面有趋势({}) + 资金面有方向({}) → 必须选方向",
                if tech_has_trend { "是" } else { "否" },
                if money_has_dir { "是" } else { "否" }
            ),
            "FORCE_DIRECTION".into(),
        )
    } else if tech_has_trend {
        ("技术面有明确趋势 → 必须选 BUY 或 SELL".into(), "FORCE_DIRECTION".into())
    } else if fund_has_catalyst {
        ("基本面/新闻面有催化剂 → 必须选 BUY 或 SELL".into(), "FORCE_DIRECTION".into())
    } else {
        ("资金面/情绪面有明确方向 → 必须选方向".into(), "FORCE_DIRECTION".into())
    };

    HoldGateResult {
        hold_allowed,
        reason,
        technical_has_trend: tech_has_trend,
        moneyflow_has_direction: money_has_dir,
        fundamental_has_catalyst: fund_has_catalyst,
        suggested_action,
    }
}

/// 综合共识判定（证据质量驱动）
///
/// 不再简单"数人头"，而是按分析师权重 * 立场方向 * 置信度 加权计算
fn compute_evidence_consensus(analysts: &[AnalystWeight]) -> EvidenceConsensus {
    let mut bullish_score = 0.0;
    let mut bearish_score = 0.0;
    let mut neutral_score = 0.0;

    for a in analysts {
        let weighted = a.final_weight * a.stance_confidence;
        match a.stance_direction.as_str() {
            "bullish" => bullish_score += weighted,
            "bearish" => bearish_score += weighted,
            _ => neutral_score += weighted,
        }
    }

    let total_weight = bullish_score + bearish_score + neutral_score;

    let (consensus, confidence) = if total_weight == 0.0 {
        debug!("共识计算边界：所有分析师权重之和为 0，回退为 neutral（无可用证据）");
        ("neutral".into(), 0.0)
    } else {
        let net = bullish_score - bearish_score;
        let max_possible = total_weight;
        // 置信度: 净得分占总权重的比例
        let raw_confidence = (net.abs() / max_possible).clamp(0.0, 1.0);
        let consensus = if net > total_weight * CONSENSUS_DIRECTION_THRESHOLD {
            "bullish"
        } else if net < -total_weight * CONSENSUS_DIRECTION_THRESHOLD {
            "bearish"
        } else if bullish_score > 0.0 && bearish_score > 0.0 {
            "divided"
        } else {
            "neutral"
        };

        let confidence = match consensus {
            "bullish" | "bearish" => {
                // 方向明确时，用净占比作为信心
                (raw_confidence * CONSENSUS_CONFIDENCE_DIR_SCALE + CONSENSUS_CONFIDENCE_BASE)
                    .min(CONSENSUS_CONFIDENCE_DIR_MAX)
            },
            "divided" => {
                // 分歧时，看哪方更强
                let max_side = bullish_score.max(bearish_score);
                (max_side / total_weight * CONSENSUS_CONFIDENCE_DIVIDED_SCALE)
                    .min(CONSENSUS_CONFIDENCE_DIVIDED_MAX)
            },
            _ => CONSENSUS_CONFIDENCE_BASE,
        };

        (consensus.to_string(), confidence)
    };

    EvidenceConsensus {
        bullish_score: (bullish_score * 100.0).round() / 100.0,
        bearish_score: (bearish_score * 100.0).round() / 100.0,
        neutral_score: (neutral_score * 100.0).round() / 100.0,
        total_weight: (total_weight * 100.0).round() / 100.0,
        net_score: ((bullish_score - bearish_score) * 100.0).round() / 100.0,
        consensus,
        confidence,
    }
}

/// 计算推荐仓位
///
/// `pub(crate)`：单周期乘数的判据需要**绕过** HorizonLayer 的分析师权重
/// （那会同时改变共识置信度，端到端比值就不是纯乘数），直测这一段算术。
pub(crate) fn compute_recommended_position(
    consensus: &EvidenceConsensus,
    hold_gate: &HoldGateResult,
    horizon: &str,
) -> f64 {
    if hold_gate.suggested_action == "HOLD" {
        return 0.0; // HOLD = 不持仓
    }

    // 根据共识方向和置信度计算仓位
    let base_pct = match consensus.consensus.as_str() {
        "bullish" => consensus.confidence * 0.8, // 0-80%
        "bearish" => 0.0,                        // 看空 → 不持仓
        "divided" => consensus.confidence * 0.3, // 分歧 → 0-30%
        _ => 0.0,
    };

    // 周期修正（〇-B v2：决策链仓位乘数的唯一权威源是 `Period::position_multiplier`，
    //   见 `axagent_harness::holding_period`；`portfolio-mgr.rhai` 主决策消费**同一个**数字）。
    //   旧实现在此处再抄一份 0.6/0.8/1.0/1.2 且用 `_ => 1.0` 把 mid 混进「未知兜底」——
    //   两份数字一旦分叉，面板与决策就会各说各话。
    let horizon_mult = horizon.parse::<Period>().unwrap_or(Period::Mid).position_multiplier();

    ((base_pct * horizon_mult * 100.0).round() / 100.0).clamp(0.0, 80.0)
}

// ── 主入口 ──

/// 执行证据质量驱动的权重计算
///
/// # 参数
/// - `request`: 包含市场环境、分析师输入、历史权重的完整请求
///
/// # 返回
/// 包含每个分析师最终权重、共识结果、HOLD 门控的完整报告
pub fn compute_evidence_weights(request: EvidenceWeightRequest) -> EvidenceWeightReport {
    // 1. 获取时间维度基础权重
    let horizon_weights = get_horizon_base_weights(&request.time_horizon);

    // 2. 计算市场周期调节系数
    let regime_modifiers = compute_regime_modifiers(&request.market_regime);

    // 3. 对每个分析师计算最终权重
    let mut analyst_weights: Vec<AnalystWeight> = request
        .analysts
        .iter()
        .map(|analyst| {
            let domain = classify_role(analyst_key(&analyst.analyst_id));

            // 时间维度基础权重 —— 先剥黑版前缀再查。
            // `reports` 的键是 `report.{节点 id}`（blackboard.rs 的键规则），而权重表按**裸节点 id**
            // 索引 ⇒ 不剥就是每个分析师都查不到、全员静默退 1.0（本次实测的主缺陷）。
            let bare_id = analyst_key(&analyst.analyst_id);
            let found = horizon_weights.contains_key(bare_id);
            let horizon_w = horizon_weights.get(bare_id).copied().unwrap_or(1.0);
            if !found {
                tracing::warn!(
                    "[evidence_weight] 分析师 '{bare_id}' 在 {} 权重表里没有条目 ⇒ 按 1.0 计（该档未声明对它的偏置）",
                    request.time_horizon
                );
            }

            // 市场周期调节
            let regime_m = regime_modifiers.get(&domain).copied().unwrap_or(1.0);

            // 历史表现权重（如果有）
            let history_m = request
                .historical_weights
                .as_ref()
                .and_then(|hw| hw.get(&analyst.analyst_id))
                .copied()
                .unwrap_or(1.0);

            // 最终权重 = 时间维度 * 市场周期 * 历史表现
            let final_w = (horizon_w * regime_m * history_m).clamp(0.1, 3.0);

            // 提取立场（含不可信标记）
            let (direction, conf, untrusted) = extract_stance(analyst);
            let is_untrusted = untrusted || analyst.untrusted.unwrap_or(false);

            AnalystWeight {
                analyst_id: analyst.analyst_id.clone(),
                domain: format!("{:?}", domain),
                horizon_weight: (horizon_w * 100.0).round() / 100.0,
                regime_modifier: (regime_m * 100.0).round() / 100.0,
                history_modifier: (history_m * 100.0).round() / 100.0,
                final_weight: (final_w * 100.0).round() / 100.0,
                stance_direction: direction,
                stance_confidence: conf,
                is_untrusted,
            }
        })
        .collect();

    // 4. 对 analyst_weights 按 analyst_id 去重（如果前端传了同一个 ID 的不同表示）
    //    用 last-write-wins 策略
    let mut deduped: HashMap<String, AnalystWeight> = HashMap::new();
    for aw in analyst_weights.drain(..) {
        deduped.insert(aw.analyst_id.clone(), aw);
    }
    let mut analyst_weights: Vec<AnalystWeight> = deduped.into_values().collect();
    analyst_weights.sort_by(|a, b| a.analyst_id.cmp(&b.analyst_id));

    // 5. 检查 HOLD 门控
    let hold_gate = check_hold_gate(&analyst_weights);

    // 6. 计算证据驱动共识
    let consensus = compute_evidence_consensus(&analyst_weights);

    // 7. 计算推荐动作
    let suggested_action = hold_gate.suggested_action.clone();
    let recommended_action = if suggested_action == "FORCE_DIRECTION" {
        match consensus.consensus.as_str() {
            "bullish" => "BUY",
            "bearish" => "SELL",
            "divided" | "neutral" => "HOLD",
            _ => "HOLD",
        }
    } else {
        &suggested_action
    };

    // 8. 计算推荐仓位
    let recommended_position_pct =
        compute_recommended_position(&consensus, &hold_gate, &request.time_horizon);

    // 9. 整体置信度
    let overall_confidence = match recommended_action {
        "BUY" | "SELL" => {
            // 方向明确 → 用共识置信度
            consensus.confidence
        },
        "HOLD" => {
            // HOLD → 通常信心较高(因为经过了门槛筛选)
            60.0
        },
        _ => consensus.confidence,
    };

    // 10. 权重坍缩三层策略（不可信信号防护，决策安全最后一道防线）
    //     - 层1：trusted_weight<0.3 或 untrusted_count≥2 → 0%仓位 + confidence×0.5
    //     - 层2：untrusted_count==1 且 posterior≥0.70 → 30%上限 + confidence×0.85（保留买入/增持）
    //     - 层3：untrusted_count==1 且 posterior<0.70 → 禁止加仓 + confidence×0.70
    let untrusted_count = analyst_weights.iter().filter(|a| a.is_untrusted).count();
    let trusted_weight: f64 =
        analyst_weights.iter().filter(|a| !a.is_untrusted).map(|a| a.final_weight).sum();
    let posterior = overall_confidence / 100.0;

    let mut final_position_pct = recommended_position_pct;
    let mut final_confidence = overall_confidence;
    let mut final_action = recommended_action.to_string();

    if trusted_weight < 0.3 || untrusted_count >= 2 {
        // 层1：完全坍缩 — 不可信信号过多，清仓
        final_position_pct = 0.0;
        final_confidence = overall_confidence * 0.5;
        if final_action == "BUY" {
            final_action = "HOLD".to_string();
        }
        tracing::warn!(
            "[权重坍缩-层1] untrusted_count={}, trusted_weight={:.2} → 0%仓位, confidence×0.5",
            untrusted_count,
            trusted_weight
        );
    } else if untrusted_count == 1 {
        if posterior >= 0.70 {
            // 层2：单一不可信但后验充足 → 限制仓位 30% 上限，保留 BUY 信号
            final_position_pct = recommended_position_pct.min(30.0);
            final_confidence = overall_confidence * 0.85;
            tracing::warn!(
                "[权重坍缩-层2] untrusted_count=1, posterior={:.2} → 30%仓位上限, confidence×0.85",
                posterior
            );
        } else {
            // 层3：单一不可信且后验不足 → 禁止加仓
            final_position_pct = 0.0;
            final_confidence = overall_confidence * 0.70;
            if final_action == "BUY" {
                final_action = "HOLD".to_string();
            }
            tracing::warn!(
                "[权重坍缩-层3] untrusted_count=1, posterior={:.2} → 禁止加仓, confidence×0.70",
                posterior
            );
        }
    }

    EvidenceWeightReport {
        market_regime: request.market_regime,
        time_horizon: request.time_horizon,
        analyst_weights,
        consensus,
        hold_gate,
        recommended_action: final_action,
        recommended_position_pct: final_position_pct,
        overall_confidence: final_confidence,
    }
}

#[cfg(test)]
mod tests {

    /// 统一 id 的三条不变量（2026-10-03，A 落地）：
    /// ① 表键 ⊆ 权威清单（不许再有幽灵 id —— 历史上 `a-technical`/`capital`/`macro`
    ///    /`fundamental`/`sentiment`/`a-market` 都不在图里，精确查表恒落空却静默 1.0）；
    /// ② 权威清单每个 id 在**每一档**都有条目（缺条目=该档没声明过偏置，必须显式 1.0）；
    /// ③ 腿桥表引用的分析师 id 也在权威清单里。
    #[test]
    fn horizon_weight_table_is_closed_over_real_analyst_nodes() {
        for p in ["ultra_short", "short", "mid", "long"] {
            let w = get_horizon_base_weights(p);
            for id in w.keys() {
                assert!(
                    EVIDENCE_ANALYST_IDS.contains(id),
                    "{p} 档权重表里有非分析师节点的键 {id}（幽灵 id 会静默退 1.0）"
                );
            }
            for id in EVIDENCE_ANALYST_IDS {
                assert!(
                    w.contains_key(*id),
                    "{p} 档权重表缺 {id} 的显式条目（要嘛给数值，要嘛显式 1.0）"
                );
            }
            assert_eq!(w.len(), EVIDENCE_ANALYST_IDS.len(), "{p} 档键数与权威清单不符");
        }
        for (_leg, analyst) in DECISION_LEG_ANALYST {
            if let Some(id) = analyst {
                assert!(
                    EVIDENCE_ANALYST_IDS.contains(id),
                    "腿 {0:?} 桥到的分析师 '{id}' 不在权威清单里",
                    _leg
                );
            }
        }
    }

    /// 闭合不变量：**有腿 ∪ 无腿 = 权威清单**，且两边不重叠、无腿项不重复。
    ///
    /// 存在理由：id 统一后「不在桥表里」= 该分析师的证据不进贝叶斯融合，这必须是**声明**
    /// 而不是查表落空的结果。少了这道闭合，新增分析师时最容易复现的缺陷就是
    /// 「权重表补了、腿没接」—— 表里看着有档位偏置，实际乘数永远碰不到任何腿。
    #[test]
    fn bridged_plus_unbridged_covers_every_analyst() {
        let mut bridged: Vec<&str> = DECISION_LEG_ANALYST.iter().filter_map(|(_, a)| *a).collect();
        bridged.sort_unstable();
        bridged.dedup();
        for id in UNBRIDGED_ANALYST_IDS {
            assert!(
                !bridged.contains(id),
                "'{id}' 同时出现在桥表与无腿清单 ⇒ 归因有歧义，请二选一并写理由"
            );
        }
        for id in EVIDENCE_ANALYST_IDS {
            assert!(
                bridged.contains(id) || UNBRIDGED_ANALYST_IDS.contains(id),
                "分析师 '{id}' 既无决策腿也未登记为无腿 ⇒ 其逐档权重会静默不影响融合（请补 UNBRIDGED_ANALYST_IDS 并写明理由）"
            );
        }
        // 无腿清单自身不许重复登记（重复会让上面的闭合「看起来更满」）
        let mut u = UNBRIDGED_ANALYST_IDS.to_vec();
        u.sort_unstable();
        let n = u.len();
        u.dedup();
        assert_eq!(u.len(), n, "UNBRIDGED_ANALYST_IDS 有重复项");
    }

    /// 逐档分支表的三条结构不变量：方向腿配比归一为 1、非方向腿权重恒 0 且必须带
    /// `constraint`、四档的腿集合（因子 + 角色）两两不同。
    /// 最后一条是 R-11 的机械证明 —— 若有人把分支表又写成「同一批腿乘不同标量」，
    /// 腿集合会逐字相同，这里当场红。
    #[test]
    fn branch_specs_are_derived_and_role_partitioned() {
        let all = horizon_branch_specs();
        for p in ["ultra_short", "short", "mid", "long"] {
            let tier = &all[p];
            let legs = tier["legs"].as_array().unwrap();
            assert!(!legs.is_empty(), "{p} 没有任何腿 ⇒ 分支表解析失效");
            let dir: Vec<&serde_json::Value> =
                legs.iter().filter(|l| l["role"] == "direction").collect();
            assert!(!dir.is_empty(), "{p} 没有进方向加权的腿 ⇒ 该档不出结论");
            let sum: f64 = dir.iter().map(|l| l["weight"].as_f64().unwrap()).sum();
            assert!((sum - 1.0).abs() < 1e-5, "{p} 方向腿配比合计应为 1，实得 {sum}");
            for l in legs.iter().filter(|l| l["role"] != "direction") {
                assert_eq!(
                    l["weight"].as_f64().unwrap(),
                    0.0,
                    "{} 非方向腿 {} 带权重 ⇒ 「被看了看」冒充「参与了」",
                    p,
                    l["factor"]
                );
                assert!(
                    l["constraint"].is_string(),
                    "{} 非方向腿 {} 必须带 constraint 说明凭什么打折",
                    p,
                    l["factor"]
                );
            }
            for l in legs {
                assert!(l["owner"].is_string(), "{p} 有腿无属主: {l}");
                assert!(l["source"]["kind"].is_string(), "{p} 有腿无来源: {l}");
            }
            let expected =
                Period::ALL.iter().find(|x| x.as_str() == p).expect("档位名非法").analyst_subset();
            let got: Vec<&str> = tier["analysts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap_or_default())
                .collect();
            assert_eq!(got, expected, "{p} 分支表的挂载集合与 harness 推导不一致");
        }

        let shape = |tier: &str| -> Vec<String> {
            all[tier]["legs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| format!("{}:{}", l["factor"], l["role"]))
                .collect()
        };
        let shapes: Vec<Vec<String>> =
            ["ultra_short", "short", "mid", "long"].iter().map(|t| shape(t)).collect();
        for pair in shapes.windows(2) {
            assert_ne!(pair[0], pair[1], "相邻两档的腿集合与角色完全相同 ⇒ 分支是假的");
        }

        // 长档技术腿：必须输出、只作入场过滤、权重恒 0（R-11 明文）
        let trend = all["long"]["legs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["factor"] == "trendStrength")
            .expect("长档应有 trendStrength 腿");
        assert_eq!(trend["role"], "filter");
        assert_eq!(trend["weight"].as_f64().unwrap(), 0.0);
        // 超短：广度门 + 时间止损 + 不做跨期缩放，三者必须同族
        assert_eq!(all["ultra_short"]["entryGate"], "breadth_required");
        assert_eq!(all["ultra_short"]["exitRule"], "time_stop");
        assert_eq!(all["ultra_short"]["confidenceMethod"], "edge_no_time_scaling");
    }

    /// 三张表（因子集 / 属主 / 来源）必须互相覆盖 —— 分处两 crate 的三处清单，
    /// 漏任一处都是「腿存在但拿不到数」或「数拿到了但没人认领」。
    #[test]
    fn factor_sources_cover_every_factor_and_owner() {
        for f in Period::verdict_factors() {
            assert!(
                VERDICT_FACTOR_SOURCES.iter().any(|(cf, _, _)| *cf == *f),
                "逐档因子 {f} 未登记来源通道 ⇒ 分支表会跳过它"
            );
        }
        // 跨档共用因子（eventCatalyst）同样要有来源
        assert!(VERDICT_FACTOR_SOURCES.iter().any(|(cf, _, _)| *cf == "eventCatalyst"));
        for (f, kind, channel) in VERDICT_FACTOR_SOURCES {
            assert!(
                matches!(*kind, "verdict" | "tierScore" | "sharedTool"),
                "因子 {f} 的来源种类非法: {kind}"
            );
            if *kind != "verdict" {
                assert!(!channel.is_empty(), "因子 {f} 是 {kind} 却没写通道名");
            }
            assert!(
                Period::factor_owner(f).is_some(),
                "来源表里的 {f} 没有属主分析师（因子改名要同步三处）"
            );
        }
    }

    /// 跨 crate 闭合：harness 的**因子属主表**推出来的逐档挂载分析师，必须都在本文件的
    /// 权威清单里。存在理由：两张表分处两个 crate，编译器不给任何提示 —— 属主表写错一个
    /// 节点 id，四路子图会挂上一个查不到的分析师（其报告进不了权重表，逐档偏置静默 1.0）。
    #[test]
    fn factor_owners_are_all_registered_analysts() {
        for p in Period::ALL {
            for a in p.analyst_subset() {
                assert!(
                    EVIDENCE_ANALYST_IDS.contains(&a),
                    "档 {} 挂载的分析师 '{a}'（由因子属主表推导）不在权威清单里 ⇒ 它的逐档权重会静默落空",
                    p.as_str()
                );
            }
        }
    }

    /// 黑版键带 `report.` 前缀（blackboard.rs 的键规则），权重表按裸节点 id 索引。
    /// 曾经不剥前缀 ⇒ **每一个**分析师的逐档权重都查不到、全员静默 1.0。
    /// 本测试同时喂两种形态，权重必须一致。
    #[test]
    fn blackboard_key_prefix_no_longer_voids_horizon_weights() {
        let regime = make_regime("bull", "normal", 0.7);
        let one = |id: &str| {
            let req = EvidenceWeightRequest {
                market_regime: regime.clone(),
                time_horizon: "ultra_short".into(),
                analysts: vec![make_analyst(id, "看多", 0.8)],
                historical_weights: None,
            };
            compute_evidence_weights(req).analyst_weights.remove(0).horizon_weight
        };
        let bare = one("a-fundamentals");
        let prefixed = one("report.a-fundamentals");
        assert!((bare - prefixed).abs() < 1e-9, "两种键形态权重不同：{bare} vs {prefixed}");
        // 负控：超短档基本面权重必须是表里的 0.3，不是兜底 1.0
        //（1.0 正是「前缀未剥 / id 是幽灵」时的落点，这条断言会把它抓出来）
        assert!(
            (bare - 0.3).abs() < 1e-9,
            "基本面在超短档的逐档权重应为 0.3，实得 {bare} —— 等于 1.0 就说明查表又落空了"
        );
    }
    use super::*;

    fn make_analyst(id: &str, stance: &str, _conf: f64) -> AnalystInput {
        AnalystInput {
            analyst_id: id.to_string(),
            report_text: None,
            stance: Some(stance.to_string()),
            bull_score: None,
            bear_score: None,
            position_pct: None,
            untrusted: None,
        }
    }

    fn make_regime(regime: &str, vol: &str, conf: f64) -> MarketRegimeInfo {
        MarketRegimeInfo {
            regime: regime.to_string(),
            confidence: conf,
            volatility: vol.to_string(),
            description: "test".into(),
            volatility_pct: None,
            consecutive_up: Some(0),
            consecutive_down: Some(0),
        }
    }

    #[test]
    fn bull_regime_boosts_technical_analysts() {
        let regime = make_regime("bull", "normal", 0.8);
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "short".into(),
            analysts: vec![
                make_analyst("a-market-analyst", "看多", 0.8),
                make_analyst("a-fundamentals", "中性", 0.5),
            ],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        let tech =
            report.analyst_weights.iter().find(|a| a.analyst_id == "a-market-analyst").unwrap();
        let fund =
            report.analyst_weights.iter().find(|a| a.analyst_id == "a-fundamentals").unwrap();
        // 牛市: tech regime_modifier > fund regime_modifier
        assert!(
            tech.regime_modifier > fund.regime_modifier,
            "牛市下 tech({}) 的 regime 调节应 > fund({})",
            tech.regime_modifier,
            fund.regime_modifier
        );
    }

    #[test]
    fn bear_regime_boosts_fundamental_analysts() {
        let regime = make_regime("bear", "normal", 0.8);
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "mid".into(),
            analysts: vec![
                make_analyst("a-market-analyst", "看空", 0.7),
                make_analyst("fundamental", "看多", 0.6),
            ],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        let fund = report.analyst_weights.iter().find(|a| a.analyst_id == "fundamental").unwrap();
        let tech =
            report.analyst_weights.iter().find(|a| a.analyst_id == "a-market-analyst").unwrap();
        // 熊市: fund regime_modifier > tech regime_modifier
        assert!(
            fund.regime_modifier > tech.regime_modifier,
            "熊市下 fund({}) 的 regime 调节应 > tech({})",
            fund.regime_modifier,
            tech.regime_modifier
        );
    }

    #[test]
    fn hold_gate_allows_hold_when_no_signals() {
        let regime = make_regime("sideways", "low", 0.5);
        // 所有分析师 neutral
        let analysts = vec![
            AnalystInput {
                analyst_id: "a-market-analyst".into(),
                report_text: None,
                stance: Some("中性".into()),
                bull_score: None,
                bear_score: None,
                position_pct: None,
                untrusted: None,
            },
            AnalystInput {
                analyst_id: "a-hot-money".into(),
                report_text: None,
                stance: Some("观望".into()),
                bull_score: None,
                bear_score: None,
                position_pct: None,
                untrusted: None,
            },
            AnalystInput {
                analyst_id: "fundamental".into(),
                report_text: None,
                stance: Some("中性".into()),
                bull_score: None,
                bear_score: None,
                position_pct: None,
                untrusted: None,
            },
        ];
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "mid".into(),
            analysts,
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        assert!(report.hold_gate.hold_allowed, "三无(无趋势+无方向+无催化剂)应允许 HOLD");
        assert_eq!(report.recommended_action, "HOLD");
    }

    #[test]
    fn hold_gate_forces_direction_when_technical_has_trend() {
        let regime = make_regime("bull", "normal", 0.7);
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "short".into(),
            analysts: vec![
                make_analyst("a-market-analyst", "看多", 0.9),
                make_analyst("fundamental", "中性", 0.5),
                make_analyst("a-sentiment", "中性", 0.4),
            ],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        assert!(!report.hold_gate.hold_allowed, "技术面有趋势 → 必须选方向");
        assert!(
            report.recommended_action == "BUY" || report.recommended_action == "SELL",
            "推荐动作应为 BUY/SELL, 实际={}",
            report.recommended_action
        );
    }

    #[test]
    fn consensus_reflects_evidence_weighting() {
        let regime = make_regime("sideways", "normal", 0.5);
        // 2 bullish + 1 bearish, but regime assigns different weights
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "mid".into(),
            analysts: vec![
                make_analyst("a-market-analyst", "看多", 0.8),
                make_analyst("fundamental", "看空", 0.7),
                make_analyst("a-sentiment", "看多", 0.6),
            ],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        // consensus 应该是一个可计算的值（不全为0）
        assert!(report.consensus.total_weight > 0.0);
        // 应该能看到权重差异
        assert!(report.consensus.bullish_score >= 0.0);
        assert!(report.consensus.bearish_score >= 0.0);
    }

    #[test]
    fn high_volatility_reduces_all_weights() {
        let regime = make_regime("bull", "high", 0.7);
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "short".into(),
            analysts: vec![make_analyst("a-market-analyst", "看多", 0.8)],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        let tech =
            report.analyst_weights.iter().find(|a| a.analyst_id == "a-market-analyst").unwrap();
        // 高波动下，regime_modifier 应该低于普通牛市
        assert!(
            tech.regime_modifier < 1.3,
            "高波动牛市 regime_modifier({}) 应低于普通牛市(1.3)",
            tech.regime_modifier
        );
    }

    #[test]
    fn historical_weights_integrate_correctly() {
        let regime = make_regime("bull", "normal", 0.7);
        let mut hist_weights = HashMap::new();
        hist_weights.insert("a-market-analyst".to_string(), 0.5);
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "short".into(),
            analysts: vec![make_analyst("a-market-analyst", "看多", 0.8)],
            historical_weights: Some(hist_weights),
        };
        let report = compute_evidence_weights(request);
        let tech =
            report.analyst_weights.iter().find(|a| a.analyst_id == "a-market-analyst").unwrap();
        // history_modifier 应反映传入的 0.5
        assert!(
            (tech.history_modifier - 0.5).abs() < 0.01,
            "history modifier 应为 0.5, 实际={}",
            tech.history_modifier
        );
        // final_weight 应 = horizon(1.5) * regime(1.3) * history(0.5)
        let expected = 1.5 * 1.3 * 0.5;
        assert!(
            (tech.final_weight - expected).abs() < 0.1,
            "final_weight 应约={}, 实际={}",
            expected,
            tech.final_weight
        );
    }

    #[test]
    fn ultra_short_horizon_assigns_low_fundamental_weight() {
        let regime = make_regime("bull", "normal", 0.7);
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "ultra_short".into(),
            analysts: vec![
                make_analyst("a-hot-money", "看多", 0.9),
                make_analyst("value-investor", "看多", 0.8),
            ],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        let money = report.analyst_weights.iter().find(|a| a.analyst_id == "a-hot-money").unwrap();
        let value =
            report.analyst_weights.iter().find(|a| a.analyst_id == "value-investor").unwrap();
        assert!(
            money.final_weight > value.final_weight,
            "超短线: 资金流权重({}) 应 > 价值权重({})",
            money.final_weight,
            value.final_weight
        );
    }

    /// 〇-B v2 第 3 条（Phase 3）：决策链的**周期仓位乘数**逐档 = `Period::position_multiplier`。
    ///
    /// 为什么直测 `compute_recommended_position` 而不是端到端跑 `compute_evidence_weights`：
    /// 换 horizon 时 HorizonLayer 的**分析师权重**也在变 ⇒ 共识置信度随之变 ⇒
    /// 端到端仓位不是 mid 的整数倍（实测：ultra_short 得 0.689×mid 而非 0.6×）。
    /// 乘数判据必须与权重判据分开，否则这道门测的是两者的乘积。
    #[test]
    fn recommended_position_scales_by_period_multiplier_only() {
        let consensus = EvidenceConsensus {
            bullish_score: 60.0,
            bearish_score: 10.0,
            neutral_score: 10.0,
            total_weight: 80.0,
            net_score: 50.0,
            consensus: "bullish".into(),
            confidence: 30.0, // 30 × 0.8 = 24% 基准 ⇒ ×1.2 = 28.8，远在 0-80 值域内
        };
        let gate = HoldGateResult {
            hold_allowed: false,
            reason: "test".into(),
            technical_has_trend: true,
            moneyflow_has_direction: true,
            fundamental_has_catalyst: true,
            suggested_action: "BUY".into(),
        };
        for (h, m) in [
            ("ultra_short", Period::UltraShort.position_multiplier()),
            ("short", Period::Short.position_multiplier()),
            ("mid", Period::Mid.position_multiplier()),
            ("long", Period::Long.position_multiplier()),
        ] {
            let got = compute_recommended_position(&consensus, &gate, h);
            let expect = (24.0 * m * 100.0).round() / 100.0;
            assert!((got - expect).abs() <= 0.001, "{h} 档应 {expect}，实得 {got}");
        }
        // 未知周期兜底 mid（与 get_horizon_base_weights 同一档语义）
        let got = compute_recommended_position(&consensus, &gate, "medium_term");
        assert!((got - 24.0).abs() <= 0.001, "未知周期应兜底 mid 基准 24，实得 {got}");
    }

    /// 乘数本身逐档显式且单调（防有人把 mid 重新塞回 `_` 兜底或改动档位序）。
    #[test]
    fn period_position_multiplier_tiers_are_ordered_and_explicit() {
        let u = Period::UltraShort.position_multiplier();
        let s = Period::Short.position_multiplier();
        let m = Period::Mid.position_multiplier();
        let l = Period::Long.position_multiplier();
        assert!(u < s && s < m && m < l, "乘数应随周期单调递增: {u}/{s}/{m}/{l}");
        assert_eq!((u, s, m, l), (0.6, 0.8, 1.0, 1.2));
    }

    /// 尺度语义锁（**从乘数表测试搬家而来**，2026-10-04）：技术属主随周期变弱、估值属主
    /// 随周期变强。
    ///
    /// 原来这条挂在 `horizon_leg_multipliers()` 上（「f1 随周期衰减 / f5 随周期增长」），
    /// 乘数表随 R-11 退役后判据没有消失 —— 它锁的是**权威权重表本身**的方向，而分支表的
    /// 腿间配比正是从这张表派生的（`branch_specs_are_derived_and_role_partitioned` 只查
    /// 「派生得对不对」，查不出「偏置方向反了」）。删表不搬判据 = 悄悄丢掉一条不变量。
    #[test]
    fn technical_bias_decays_and_valuation_bias_grows_with_horizon() {
        let w = |tier: &str, analyst: &str| {
            get_horizon_base_weights(tier)
                .get(analyst)
                .copied()
                .unwrap_or_else(|| panic!("档 {tier} 缺分析师 {analyst} ⇒ 本锁的前提已变，须同步"))
        };
        let (u, s, m, l) = ("ultra_short", "short", "mid", "long");
        assert!(
            w(l, "a-market-analyst") < w(m, "a-market-analyst")
                && w(m, "a-market-analyst") < w(s, "a-market-analyst"),
            "技术属主未随周期变弱: {}/{}/{}",
            w(u, "a-market-analyst"),
            w(s, "a-market-analyst"),
            w(l, "a-market-analyst")
        );
        assert!(
            w(u, "a-market-analyst") > w(l, "a-market-analyst"),
            "超短档技术权重必须高于长档，实得 {} vs {}",
            w(u, "a-market-analyst"),
            w(l, "a-market-analyst")
        );
        for analyst in ["value-investor", "a-fundamentals"] {
            assert!(
                w(u, analyst) < w(s, analyst)
                    && w(s, analyst) < w(m, analyst)
                    && w(m, analyst) < w(l, analyst),
                "估值属主 {analyst} 未随周期单调变强: {} < {} < {} < {}",
                w(u, analyst),
                w(s, analyst),
                w(m, analyst),
                w(l, analyst)
            );
        }
        // 搬家说明：原 `meta_legs_are_scale_invariant`（元约束腿 f4/f6/f7 恒 1.0）的等价内容
        // 现在由两处共同保证，不在本测试里重复：① 那三条腿在 `DECISION_LEG_ANALYST` 里桥到
        // `None` ⇒ 无任何逐档权重可作用；② 「桥到的 ∪ 显式无腿的 = 权威清单」由同文件的
        // `horizon_weight_table_is_closed_over_real_analyst_nodes` 与
        // `scripts/check-horizon-weight-parity.mjs` 判据 ⑧ 锁住。
    }
}
