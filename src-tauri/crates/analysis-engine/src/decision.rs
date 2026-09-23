use serde::{Deserialize, Serialize};

/// 投资决策
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockDecision {
    /// 买入/增持/持有/减持/卖出
    pub action: String,
    /// 建议仓位百分比 (0-100)
    pub position_pct: f64,
    /// 目标价
    pub target_price: Option<f64>,
    /// 止损价
    pub stop_loss: Option<f64>,
    /// 决策理由
    pub reasoning: String,
    /// 风险等级: 低/中/高
    pub risk_level: String,
    /// 置信度 (0-100) — 看多概率，用于仓位计算。低值表示看空。
    pub confidence: f64,
    /// 决策方向置信度 (0-100) — 无论买卖方向都体现"多确信"。
    /// = max(effective_posterior, 1-effective_posterior) × 100
    /// 解决痛点：看空决策 confidence 偏低被误读为"不确信"。
    #[serde(default)]
    pub decision_confidence: Option<f64>,
    /// 信号强度 (0-100) — 偏离中性的程度。
    /// = |effective_posterior - 0.5| × 200
    /// 0=完全中性，100=极端强信号。
    #[serde(default)]
    pub signal_strength: Option<f64>,
    /// 时间维度: "ultra_short" | "short" | "mid" | "long"
    #[serde(default)]
    pub time_horizon: Option<String>,
    /// 期望持有天数（交易日）
    #[serde(default)]
    pub expected_holding_days: Option<u32>,
    /// 目标价预期实现时间框架: "1d" | "1w" | "1m" | "3m" | "6m"
    #[serde(default)]
    pub target_timeframe: Option<String>,
}

/// 修复 H6: 统一决策映射层
///
/// 规格给定的 6 档 action 体系（中文）：
///   强烈买入 / 买入 / 增持 / 持有 / 减持 / 卖出
///
/// 后验概率阈值（规格）：
///   - p >= 0.63 → 强烈买入
///   - 0.53 <= p < 0.63 → 买入
///   - 0.48 <= p < 0.53 → 增持
///   - 0.38 <= p < 0.48 → 持有
///   - 0.30 <= p < 0.38 → 减持
///   - p < 0.30 → 卖出
///
/// 同时提供与 astock-data/scoring.rs map_signal（100 分制 6 档）的桥接：
///   strong_buy / buy / hold / watch / sell / strong_sell
///   ↘ 强烈买入 / 买入 / 增持 / 持有 / 减持 / 卖出
pub fn map_posterior_to_action(posterior: f64) -> &'static str {
    if posterior >= 0.63 {
        "强烈买入"
    } else if posterior >= 0.53 {
        "买入"
    } else if posterior >= 0.48 {
        "增持"
    } else if posterior >= 0.38 {
        "持有"
    } else if posterior >= 0.30 {
        "减持"
    } else {
        "卖出"
    }
}

/// 将 astock-data/scoring.rs 的 signal_code 统一映射到 6 档中文 action
pub fn map_signal_code_to_action(signal_code: &str) -> &'static str {
    match signal_code {
        "strong_buy" => "强烈买入",
        "buy" => "买入",
        // hold/watch 在 6 档中分别对应 增持/持有，
        // 但 scoring.rs 的 hold 是"分数中性偏多"，watch 是"分数中性偏空"
        "hold" => "增持",
        "watch" => "持有",
        "sell" => "减持",
        "strong_sell" => "卖出",
        _ => "持有",
    }
}

/// 将 evidence_weight.rs 的 BUY/SELL/HOLD 映射到 6 档中文 action
pub fn map_evidence_action(evidence_action: &str) -> &'static str {
    match evidence_action {
        "BUY" => "买入",
        "SELL" => "卖出",
        "HOLD" => "持有",
        _ => "持有",
    }
}

/// 分析阶段性事件（通过 broadcast channel 推送前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum AnalysisEvent {
    Started { stock_code: String, stock_name: String, date: String },
    DataLoaded { kline_count: usize, news_count: usize },
    AnalystProgress { expert_id: String, status: String, progress_pct: u8 },
    AnalystReport { expert_id: String, report_text: String },
    DebateRound { round: u32, bull_argument: String, bear_argument: String },
    RiskAssessment { risk_type: String, report: String },
    InvestmentPlan { plan: String },
    Decision(StockDecision),
    Error { stage: String, message: String },
}

/// 可配置的评分权重
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoringWeights {
    pub trend: f64,
    pub deviation: f64,
    pub macd: f64,
    pub volume: f64,
    pub rsi: f64,
    pub support: f64,
    #[serde(default = "default_boll")]
    pub boll: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            trend: 30.0,
            deviation: 20.0,
            macd: 15.0,
            volume: 15.0,
            rsi: 10.0,
            support: 5.0,
            boll: 5.0,
        }
    }
}

fn default_boll() -> f64 {
    5.0
}

// ── 规则引擎可调阈值 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleConfig {
    #[serde(default = "default_rsi_overbought")]
    pub rsi_overbought: f64,
    #[serde(default = "default_bias_limit")]
    pub bias_limit: f64,
    #[serde(default = "default_volume_block")]
    pub volume_signal_block: bool,
    #[serde(default = "default_bear_low_score")]
    pub bear_low_score: u32,
    #[serde(default = "default_rsi_oversold")]
    pub rsi_oversold: f64,
    #[serde(default = "default_auto_stop_loss_pct")]
    pub auto_stop_loss_pct: f64,
}

fn default_rsi_overbought() -> f64 {
    80.0
}
fn default_bias_limit() -> f64 {
    5.0
}
fn default_volume_block() -> bool {
    true
}
fn default_bear_low_score() -> u32 {
    30
}
fn default_rsi_oversold() -> f64 {
    20.0
}
fn default_auto_stop_loss_pct() -> f64 {
    5.0
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            rsi_overbought: 80.0,
            bias_limit: 5.0,
            volume_signal_block: true,
            bear_low_score: 30,
            rsi_oversold: 20.0,
            auto_stop_loss_pct: 5.0,
        }
    }
}

// ── 估值参数 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueConfig {
    #[serde(default = "default_dcf_growth")]
    pub dcf_growth_rate: f64,
    #[serde(default = "default_dcf_perpetual")]
    pub dcf_perpetual_rate: f64,
    #[serde(default = "default_dcf_discount")]
    pub dcf_discount_rate: f64,
    #[serde(default = "default_moat_threshold")]
    pub moat_threshold: u32,
    #[serde(default = "default_fscore_buy")]
    pub f_score_buy_threshold: u32,
    #[serde(default = "default_safety_margin")]
    pub safety_margin_min: f64,
}

// ⚠️ 2026-09-23：本组默认值改为**派生**自 `astock-data::mcp_tools` 的唯一真相源常量。
//
// 此前它们是**第五份手抄**（12.0 / 4.0 / 8.5），而常量区的「唯一真相源」表只列了四处、
// 未包含本文件 ⇒ 真相源一改，这里就静默漂移。本组单位是**百分数**，故 ×100 换算。
//
// 本结构当前**全仓无消费方**（仅定义 + 文档引用）。它的历史角色正是「假修复」的载体：
// `seed_stock_analysis.rs` 的 v32 变更日志记录 —— 2026-09-12 那次 A 股校准
// 「只改到了**未被消费的** `decision::ValueConfig`，没改到实际执行的常量」，
// 于是面板显示 8.5/4.0/12.0 而 `astock-data` 仍按 0.10/0.03/0.08 执行。
// 漂移的地雷在**无消费方**时最危险（改它、不生效、也无人报警），故此处必须派生：
// 即便将来有人接线，也不会再引入第五处口径。
fn default_dcf_growth() -> f64 {
    axagent_astock_data::mcp_tools::DEFAULT_GROWTH * 100.0
}
fn default_dcf_perpetual() -> f64 {
    axagent_astock_data::mcp_tools::PERPETUAL_GROWTH * 100.0
}
fn default_dcf_discount() -> f64 {
    axagent_astock_data::mcp_tools::DISCOUNT_RATE * 100.0
}
fn default_moat_threshold() -> u32 {
    60
}
fn default_fscore_buy() -> u32 {
    7
}
fn default_safety_margin() -> f64 {
    20.0
}

impl Default for ValueConfig {
    fn default() -> Self {
        // ⚠️ 2026-09-23：与上方 `default_dcf_*` **同源派生**，不再各写一份字面量
        //   （本节曾是手抄第四份，且与函数版可独立漂移）。
        use axagent_astock_data::mcp_tools as m;
        Self {
            dcf_growth_rate: m::DEFAULT_GROWTH * 100.0,
            dcf_perpetual_rate: m::PERPETUAL_GROWTH * 100.0,
            dcf_discount_rate: m::DISCOUNT_RATE * 100.0,
            moat_threshold: 60,
            f_score_buy_threshold: 7,
            safety_margin_min: 20.0,
        }
    }
}
