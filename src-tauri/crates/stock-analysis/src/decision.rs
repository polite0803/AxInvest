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
    pub max_debate_rounds: u32,
    /// K线周期
    pub kline_period: String,
    /// K线数量
    pub kline_limit: u32,
    /// 新闻数量
    pub news_limit: u32,
    /// LLM temperature (0-1)
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    /// LLM max tokens
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_temperature() -> f64 {
    0.3
}
fn default_max_tokens() -> u32 {
    4096
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            max_debate_rounds: 3,
            kline_period: "daily".to_string(),
            kline_limit: 120,
            news_limit: 30,
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
