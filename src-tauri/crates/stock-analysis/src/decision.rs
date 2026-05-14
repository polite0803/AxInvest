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
    pub max_debate_rounds: u32,
    /// K线周期
    pub kline_period: String,
    /// K线数量
    pub kline_limit: u32,
    /// 新闻数量
    pub news_limit: u32,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            max_debate_rounds: 3,
            kline_period: "daily".to_string(),
            kline_limit: 120,
            news_limit: 30,
        }
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
