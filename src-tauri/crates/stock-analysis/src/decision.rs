use async_trait::async_trait;
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
    /// 置信度 (0-1)
    pub confidence: f64,
}

/// 分析配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisConfig {
    /// 最大辩论轮数
    #[serde(default = "default_debate_rounds")]
    pub max_debate_rounds: u32,
    /// K线周期
    #[serde(default = "default_kline_period")]
    pub kline_period: String,
    /// K线数量
    #[serde(default = "default_kline_limit")]
    pub kline_limit: u32,
    /// 新闻数量
    #[serde(default = "default_news_limit")]
    pub news_limit: u32,
    /// 并行分析数
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
    /// LLM temperature (0-1)
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    /// LLM max tokens
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_debate_rounds() -> u32 { 3 }
fn default_kline_period() -> String { "daily".into() }
fn default_kline_limit() -> u32 { 120 }
fn default_news_limit() -> u32 { 30 }
fn default_max_concurrent() -> u32 { 9 }
fn default_temperature() -> f64 { 0.3 }
fn default_max_tokens() -> u32 { 4096 }

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            max_debate_rounds: 3,
            kline_period: "daily".to_string(),
            kline_limit: 120,
            news_limit: 30,
            max_concurrent: 9,
            temperature: 0.3,
            max_tokens: 4096,
        }
    }
}

impl AnalysisConfig {
    /// 验证配置参数合法性
    pub fn validate(&self) -> Result<(), String> {
        if self.max_debate_rounds == 0 {
            return Err("max_debate_rounds must be > 0".into());
        }
        if self.max_debate_rounds > 10 {
            return Err("max_debate_rounds must be <= 10".into());
        }
        if self.kline_limit == 0 || self.kline_limit > 500 {
            return Err("kline_limit must be 1-500".into());
        }
        if self.news_limit == 0 || self.news_limit > 100 {
            return Err("news_limit must be 1-100".into());
        }
        if !["daily", "weekly", "monthly"].contains(&self.kline_period.as_str()) {
            return Err("kline_period must be daily/weekly/monthly".into());
        }
        Ok(())
    }
}

/// 分析阶段性事件（通过 broadcast channel 推送前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum AnalysisEvent {
    Started {
        stock_code: String,
        stock_name: String,
        date: String,
    },
    DataLoaded {
        kline_count: usize,
        news_count: usize,
    },
    AnalystProgress {
        expert_id: String,
        status: String,
        progress_pct: u8,
    },
    AnalystReport {
        expert_id: String,
        report_text: String,
    },
    DebateRound {
        round: u32,
        bull_argument: String,
        bear_argument: String,
    },
    RiskAssessment {
        risk_type: String,
        report: String,
    },
    InvestmentPlan {
        plan: String,
    },
    Decision(StockDecision),
    Error {
        stage: String,
        message: String,
    },
}

/// Agent 执行器抽象 — 由命令层注入，编排器通过此 trait 调用 LLM Agent
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// 运行单个专家 Agent
    ///
    /// * `expert_id` - 专家标识，如 "market-analyst"
    /// * `system_prompt` - 系统提示
    /// * `user_prompt` - 用户提示（含数据上下文）
    ///
    /// 返回专家报告文本
    async fn run_agent(
        &self,
        expert_id: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<String, String>;
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
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            trend: 30.0,
            deviation: 20.0,
            macd: 15.0,
            volume: 15.0,
            rsi: 10.0,
            support: 10.0,
        }
    }
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

fn default_rsi_overbought() -> f64 { 80.0 }
fn default_bias_limit() -> f64 { 5.0 }
fn default_volume_block() -> bool { true }
fn default_bear_low_score() -> u32 { 30 }
fn default_rsi_oversold() -> f64 { 20.0 }
fn default_auto_stop_loss_pct() -> f64 { 5.0 }

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

// ── 仓位限制 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionLimitsConfig {
    #[serde(default = "default_max_single_stock")]
    pub max_single_stock_pct: f64,
    #[serde(default = "default_max_total_pos")]
    pub max_total_positions: u32,
    #[serde(default = "default_max_sector")]
    pub max_sector_exposure_pct: f64,
}

fn default_max_single_stock() -> f64 { 20.0 }
fn default_max_total_pos() -> u32 { 10 }
fn default_max_sector() -> f64 { 40.0 }

impl Default for PositionLimitsConfig {
    fn default() -> Self {
        Self {
            max_single_stock_pct: 20.0,
            max_total_positions: 10,
            max_sector_exposure_pct: 40.0,
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

fn default_dcf_growth() -> f64 { 8.0 }
fn default_dcf_perpetual() -> f64 { 3.0 }
fn default_dcf_discount() -> f64 { 10.0 }
fn default_moat_threshold() -> u32 { 60 }
fn default_fscore_buy() -> u32 { 7 }
fn default_safety_margin() -> f64 { 20.0 }

impl Default for ValueConfig {
    fn default() -> Self {
        Self {
            dcf_growth_rate: 8.0,
            dcf_perpetual_rate: 3.0,
            dcf_discount_rate: 10.0,
            moat_threshold: 60,
            f_score_buy_threshold: 7,
            safety_margin_min: 20.0,
        }
    }
}

// ── 监控参数 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorConfig {
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u32,
    #[serde(default = "default_change_pct")]
    pub change_pct_threshold: f64,
    #[serde(default = "default_turnover")]
    pub turnover_threshold: f64,
}

fn default_poll_interval() -> u32 { 30 }
fn default_change_pct() -> f64 { 5.0 }
fn default_turnover() -> f64 { 10.0 }

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 30,
            change_pct_threshold: 5.0,
            turnover_threshold: 10.0,
        }
    }
}

// ── 完整配置（版本化持久化）──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockAnalysisFullConfig {
    #[serde(default)]
    pub analysis: AnalysisConfig,
    #[serde(default)]
    pub scoring: ScoringWeights,
    #[serde(default)]
    pub rules: RuleConfig,
    #[serde(default)]
    pub position: PositionLimitsConfig,
    #[serde(default)]
    pub value: ValueConfig,
    #[serde(default)]
    pub monitor: MonitorConfig,
}

impl Default for StockAnalysisFullConfig {
    fn default() -> Self {
        Self {
            analysis: AnalysisConfig::default(),
            scoring: ScoringWeights::default(),
            rules: RuleConfig::default(),
            position: PositionLimitsConfig::default(),
            value: ValueConfig::default(),
            monitor: MonitorConfig::default(),
        }
    }
}
