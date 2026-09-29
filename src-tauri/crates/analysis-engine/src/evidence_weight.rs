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

/// 分析师 ID 列表（用于权重映射）
pub const ANALYST_IDS: &[&str] = &[
    "a-fundamentals",
    "fundamental",
    "value-investor",
    "a-macro",
    "macro",
    "a-sector",
    "research-mgr",
    "a-market",
    "a-technical",
    "a-sentiment",
    "sentiment",
    "a-news",
    "a-hot-money",
    "capital",
];

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
    match analyst_id {
        "fundamental" | "a-fundamentals" | "value-investor" => AnalystRole::Fundamental,
        "macro" | "a-macro" | "a-sector" => AnalystRole::Macro,
        "a-market" | "a-technical" => AnalystRole::Technical,
        "sentiment" | "a-sentiment" | "a-news" | "a-hot-money" | "capital" => {
            AnalystRole::Sentiment
        },
        "research-mgr" => AnalystRole::Research,
        _ => {
            // 按关键词后缀推断
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
    let mut w = HashMap::new();
    match horizon {
        "ultra_short" => {
            w.insert("a-hot-money", 2.0);
            w.insert("capital", 2.0);
            w.insert("a-sentiment", 1.5);
            w.insert("sentiment", 1.5);
            w.insert("a-news", 1.5);
            w.insert("a-market", 1.3);
            w.insert("a-technical", 1.3);
            w.insert("research-mgr", 0.5);
            w.insert("a-fundamentals", 0.3);
            w.insert("fundamental", 0.3);
            w.insert("value-investor", 0.3);
            w.insert("a-macro", 0.3);
            w.insert("macro", 0.3);
            w.insert("a-sector", 0.5);
        },
        "short" => {
            w.insert("a-market", 1.5);
            w.insert("a-technical", 1.5);
            w.insert("a-hot-money", 1.5);
            w.insert("capital", 1.5);
            w.insert("a-sentiment", 1.3);
            w.insert("sentiment", 1.3);
            w.insert("a-news", 1.2);
            w.insert("value-investor", 0.5);
            w.insert("a-fundamentals", 0.6);
            w.insert("fundamental", 0.6);
            w.insert("a-macro", 0.7);
            w.insert("macro", 0.7);
            w.insert("a-sector", 0.8);
            w.insert("research-mgr", 1.0);
        },
        "long" => {
            w.insert("fundamental", 1.5);
            w.insert("a-fundamentals", 1.5);
            w.insert("value-investor", 2.0);
            w.insert("a-macro", 1.3);
            w.insert("macro", 1.3);
            w.insert("a-sector", 1.2);
            w.insert("research-mgr", 1.5);
            w.insert("a-news", 0.7);
            w.insert("sentiment", 0.7);
            w.insert("a-sentiment", 0.7);
            w.insert("a-hot-money", 0.5);
            w.insert("capital", 0.5);
            w.insert("a-technical", 0.6);
            w.insert("a-market", 0.6);
        },
        // 中线：四档中唯一接近「不加权」的基准档，显式列出不靠 `_` 兜底
        "mid" => {
            w.insert("a-fundamentals", 1.2);
            w.insert("fundamental", 1.2);
            w.insert("value-investor", 1.2);
            w.insert("a-macro", 1.1);
            w.insert("macro", 1.1);
            w.insert("a-sector", 1.1);
            w.insert("research-mgr", 1.1);
            w.insert("a-market", 1.0);
            w.insert("a-technical", 1.0);
            w.insert("a-sentiment", 1.0);
            w.insert("sentiment", 1.0);
            w.insert("a-news", 1.0);
            w.insert("a-hot-money", 0.9);
            w.insert("capital", 0.9);
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
/// 为什么必须有它（四周期科学化 Phase A）：「同一批证据、逐档重新加权」要求把
/// 按**分析师 id** 索引的周期权重表，作用到按**因子腿**融合的决策脚本上；两侧没有
/// 登记的桥就会各写一套倍数 ⇒ 又回到手抄多副本。
///
/// `None` 的腿**不是分析师证据**（风险分类、数据质量、trader 观点都是元约束），
/// 逐档乘数恒 1.0 —— 不得为了「看起来四档有差异」硬塞一个分析师进去。
pub const DECISION_LEG_ANALYST: &[(&str, Option<&str>)] = &[
    ("f1", Some("a-technical")),    // 技术面 / 趋势
    ("f2", Some("research-mgr")),   // 分析师共识
    ("f3", Some("a-news")),         // 催化剂 / 公告
    ("f4", None),                   // 风险分类（元约束）
    ("f5", Some("value-investor")), // 估值（DCF / 格雷厄姆腿）
    ("f6", None),                   // 数据质量（元约束）
    ("f7", None),                   // trader 观点（不属于四档分析师证据）
    ("f9", Some("capital")),        // 资金流
    ("f10", Some("a-hot-money")),   // 筹码面（增减持 / 解禁 / 大宗）
    ("f11", Some("a-sentiment")),   // PACE 情绪
    ("f12", Some("a-technical")),   // 动量（技术侧）
    ("f13", Some("a-sector")),      // 产业链瓶颈
];

/// 注入 Rhai 的「逐档 × 逐腿」乘数表：`{ultra_short: {f1: 1.3, …}, …}`。
///
/// 数值**全部派生**自 [`get_horizon_base_weights`]，本函数不新增任何一个数字；
/// 表里查不到该分析师 ⇒ 乘数 1.0（该腿不变）并 `warn`，不静默。
/// 由 `stock_workflow/hooks.rs` 注入为变量 `horizon_leg_weights_json`，
/// 脚本侧**禁止**再手抄任何一档的权重倍数（同 `horizon_consts_json` 的纪律）。
pub fn horizon_leg_multipliers() -> serde_json::Value {
    let mut tiers = serde_json::Map::new();
    for p in Period::ALL {
        let weights = get_horizon_base_weights(p.as_str());
        let mut legs = serde_json::Map::new();
        for (leg, analyst) in DECISION_LEG_ANALYST {
            let mult = match analyst {
                Some(id) => match weights.get(*id) {
                    Some(v) => *v,
                    None => {
                        tracing::warn!(
                            "[evidence_weight] 周期权重表缺分析师 '{id}'（腿 {leg}，档 {}）⇒ 该腿乘数按 1.0",
                            p.as_str()
                        );
                        1.0
                    },
                },
                None => 1.0,
            };
            legs.insert((*leg).to_string(), serde_json::Value::from(mult));
        }
        tiers.insert(p.as_str().to_string(), serde_json::Value::Object(legs));
    }
    serde_json::Value::Object(tiers)
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
            let domain = classify_role(&analyst.analyst_id);

            // 时间维度基础权重
            let horizon_w =
                horizon_weights.get(analyst.analyst_id.as_str()).copied().unwrap_or(1.0);

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
                make_analyst("a-technical", "看多", 0.8),
                make_analyst("a-fundamentals", "中性", 0.5),
            ],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        let tech = report.analyst_weights.iter().find(|a| a.analyst_id == "a-technical").unwrap();
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
                make_analyst("a-technical", "看空", 0.7),
                make_analyst("fundamental", "看多", 0.6),
            ],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        let fund = report.analyst_weights.iter().find(|a| a.analyst_id == "fundamental").unwrap();
        let tech = report.analyst_weights.iter().find(|a| a.analyst_id == "a-technical").unwrap();
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
                analyst_id: "a-technical".into(),
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
                make_analyst("a-technical", "看多", 0.9),
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
                make_analyst("a-technical", "看多", 0.8),
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
            analysts: vec![make_analyst("a-technical", "看多", 0.8)],
            historical_weights: None,
        };
        let report = compute_evidence_weights(request);
        let tech = report.analyst_weights.iter().find(|a| a.analyst_id == "a-technical").unwrap();
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
        hist_weights.insert("a-technical".to_string(), 0.5);
        let request = EvidenceWeightRequest {
            market_regime: regime,
            time_horizon: "short".into(),
            analysts: vec![make_analyst("a-technical", "看多", 0.8)],
            historical_weights: Some(hist_weights),
        };
        let report = compute_evidence_weights(request);
        let tech = report.analyst_weights.iter().find(|a| a.analyst_id == "a-technical").unwrap();
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

    /// 逐档 × 逐腿乘数表必须四档齐、腿数与桥表一致（缺档 ⇒ 脚本侧该档无从取值）。
    #[test]
    fn horizon_leg_multipliers_covers_every_tier_and_leg() {
        let map = horizon_leg_multipliers();
        for p in Period::ALL {
            let row = map.get(p.as_str()).unwrap_or_else(|| panic!("乘数表缺档 {}", p.as_str()));
            let obj = row.as_object().unwrap_or_else(|| panic!("档 {} 不是对象", p.as_str()));
            assert_eq!(
                obj.len(),
                DECISION_LEG_ANALYST.len(),
                "档 {} 腿数 {} ≠ 桥表 {}",
                p.as_str(),
                obj.len(),
                DECISION_LEG_ANALYST.len()
            );
        }
    }

    /// 元约束腿（风险分类 / 数据质量 / trader 观点）**不得随周期变** —— 它们不是分析师证据。
    #[test]
    fn meta_legs_are_scale_invariant() {
        let map = horizon_leg_multipliers();
        for leg in ["f4", "f6", "f7"] {
            for p in Period::ALL {
                let got = map[p.as_str()][leg].as_f64().unwrap_or_else(|| panic!("腿 {leg} 缺值"));
                assert_eq!(got, 1.0, "元约束腿 {leg} 在档 {} 被改了乘数", p.as_str());
            }
        }
    }

    /// 派生性：表里每个数都必须等于该档分析师权重，**不得有任何新数字**。
    #[test]
    fn multipliers_are_derived_from_the_horizon_table() {
        let map = horizon_leg_multipliers();
        for p in Period::ALL {
            let weights = get_horizon_base_weights(p.as_str());
            for (leg, analyst) in DECISION_LEG_ANALYST {
                let got = map[p.as_str()][*leg].as_f64().expect("腿应有值");
                let want = match analyst {
                    Some(id) => *weights.get(*id).unwrap_or(&1.0),
                    None => 1.0,
                };
                assert!(
                    (got - want).abs() <= f64::EPSILON,
                    "档 {} 腿 {leg}：表给 {got}，权重表给 {want} ⇒ 乘数表被另写了数字",
                    p.as_str()
                );
            }
        }
    }

    /// 尺度语义锁：技术腿随周期衰减、估值腿随周期增长（防有人把表改平）。
    #[test]
    fn technical_leg_decays_and_valuation_leg_grows_with_horizon() {
        let map = horizon_leg_multipliers();
        let at = |leg: &str, tier: Period| map[tier.as_str()][leg].as_f64().expect("腿应有值");
        let (u, s, m, l) = (Period::UltraShort, Period::Short, Period::Mid, Period::Long);
        assert!(
            at("f1", l) < at("f1", m) && at("f1", m) < at("f1", s) && at("f1", u) > at("f1", l),
            "技术腿应随周期变弱: {}",
            ["ultra_short", "short", "mid", "long"].map(|t| map[t]["f1"].to_string()).join("/")
        );
        assert!(
            at("f5", u) < at("f5", m) && at("f5", m) < at("f5", l),
            "估值腿应随周期变强: {}",
            ["ultra_short", "short", "mid", "long"].map(|t| map[t]["f5"].to_string()).join("/")
        );
    }
}
