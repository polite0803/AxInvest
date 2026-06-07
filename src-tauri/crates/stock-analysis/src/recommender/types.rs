//! 智能荐股 — 公共类型

use serde::{Deserialize, Serialize};

/// 风格（4 种）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// 趋势跟踪
    Trend,
    /// 价值低估
    Value,
    /// 资金驱动
    Capital,
    /// 超跌反弹
    Reversion,
}

impl Style {
    pub fn as_str(&self) -> &'static str {
        match self {
            Style::Trend => "trend",
            Style::Value => "value",
            Style::Capital => "capital",
            Style::Reversion => "reversion",
        }
    }
}

/// 持有周期（3 种）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    /// 短线 1-2 周
    Short,
    /// 中线 3-8 周
    Mid,
    /// 长线 3 个月+
    Long,
}

impl Period {
    pub fn as_str(&self) -> &'static str {
        match self {
            Period::Short => "short",
            Period::Mid => "mid",
            Period::Long => "long",
        }
    }

    /// 周期因子（用于动态仓位）
    pub fn factor(&self) -> f64 {
        match self {
            Period::Short => 0.6,
            Period::Mid => 0.8,
            Period::Long => 1.0,
        }
    }

    /// 建议持有天数
    pub fn default_holding_days(&self) -> u32 {
        match self {
            Period::Short => 5,
            Period::Mid => 28,
            Period::Long => 90,
        }
    }
}

/// 单条推荐
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoPick {
    pub stock_code: String,
    pub stock_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sector: Option<String>,
    pub style: Style,
    pub period: Period,
    /// 当前价
    pub price: f64,
    /// 入场下沿
    pub entry_low: f64,
    /// 入场上沿
    pub entry_high: f64,
    /// 止损
    pub stop_loss: f64,
    /// 目标位
    pub target_price: f64,
    /// 建议仓位（%）
    pub position_pct: f64,
    /// 持有天数
    pub holding_days: u32,
    /// 置信度 0-100
    pub confidence: u8,
    /// 命中理由
    pub reasons: Vec<String>,
    /// 风险提示
    pub risk_notes: Vec<String>,
    /// 次选风格（同票被多策略命中时记录）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondary_styles: Vec<Style>,
}

/// 荐股响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoResponse {
    pub period: Period,
    /// 按风格分组的 picks，每组 ≤ 10
    pub picks: std::collections::HashMap<Style, Vec<RecoPick>>,
    /// 被 vendor 缺失禁用的风格
    pub disabled_styles: Vec<Style>,
    /// 生成时间戳（毫秒）
    pub generated_at: i64,
    /// **过滤前**的 seed pool 大小（hot + industry 龙头去重后）
    /// 实际参与扫描的池大小更小（流动性过滤会进一步剔除）
    pub raw_seed_pool_size: usize,
}
