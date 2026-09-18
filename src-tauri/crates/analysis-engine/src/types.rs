//! 股票分析领域类型定义
//!
//! 集中管理 analysis-engine crate 中使用的枚举类型，
//! 消除魔法字符串，提升类型安全。

use serde::{Deserialize, Serialize};

/// 市场状态类型
///
/// 基于沪深300均线位置和斜率判断：
/// - `Bull`: 价格站上 MA60 + 多头排列 + 向上斜率
/// - `Bear`: 价格跌破 MA60 + 空头排列 + 向下斜率
/// - `Sideways`: 震荡
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarketRegimeType {
    Bull,
    Bear,
    Sideways,
}

impl MarketRegimeType {
    /// 中文标签
    pub fn label(self) -> &'static str {
        match self {
            Self::Bull => "牛市",
            Self::Bear => "熊市",
            Self::Sideways => "震荡",
        }
    }
}

/// 波动率水平
///
/// 基于布林带宽度（20日收盘价标准差 / MA20）：
/// - `High`: > 20%
/// - `Low`: < 10%
/// - `Normal`: 10-20%
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolatilityLevel {
    High,
    Low,
    Normal,
}

impl VolatilityLevel {
    /// 从波动率百分比推导等级
    pub fn from_bollinger_pct(pct: f64) -> Self {
        if pct > 0.20 {
            Self::High
        } else if pct < 0.10 {
            Self::Low
        } else {
            Self::Normal
        }
    }

    /// 中文标签
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "高波动",
            Self::Low => "低波动",
            Self::Normal => "正常",
        }
    }
}

/// 策略趋势方向
///
/// 用于 evolution_drift 策略漂移追踪：
/// - `Up`: 策略净收益上升 > 5%
/// - `Down`: 策略净收益下降 > 5%
/// - `Stable`: 策略净收益波动在 ±5% 内
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrategyTrend {
    Up,
    Down,
    Stable,
}

impl StrategyTrend {
    /// 从净收益率（百分比）推导趋势
    pub fn from_net_delta(delta_pct: f64) -> Self {
        if delta_pct > 5.0 {
            Self::Up
        } else if delta_pct < -5.0 {
            Self::Down
        } else {
            Self::Stable
        }
    }

    /// 中文标签
    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "上升",
            Self::Down => "下降",
            Self::Stable => "稳定",
        }
    }
}

/// 反馈趋势（分析师胜率变化）
///
/// 用于 backtest_feedback 反馈分析：
/// - `Improving`: 分析师胜率改善
/// - `Declining`: 分析师胜率下降
/// - `Stable`: 分析师胜率稳定
/// - `InsufficientData`: 数据不足
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackTrend {
    Improving,
    Declining,
    Stable,
    InsufficientData,
}

impl std::fmt::Display for StrategyTrend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl FeedbackTrend {
    /// 中文标签
    pub fn label(self) -> &'static str {
        match self {
            Self::Improving => "改善",
            Self::Declining => "下降",
            Self::Stable => "稳定",
            Self::InsufficientData => "数据不足",
        }
    }
}

impl std::fmt::Display for FeedbackTrend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 建议调整类型
///
/// 用于 backtest_feedback 对分析师的改进建议：
/// - `None`: 无需调整
/// - `AdjustWeight`: 仅调整权重
/// - `TweakPrompt`: 微调提示词
/// - `ReviewLogic`: 重构分析逻辑
/// - `Disable`: 暂时禁用
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionType {
    None,
    AdjustWeight,
    TweakPrompt,
    ReviewLogic,
    Disable,
}

impl SuggestionType {
    /// 中文标签
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "无需调整",
            Self::AdjustWeight => "调整权重",
            Self::TweakPrompt => "微调提示词",
            Self::ReviewLogic => "重构逻辑",
            Self::Disable => "暂时禁用",
        }
    }
}

impl std::fmt::Display for SuggestionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 分析师立场
///
/// 用于 backtest_feedback 分析师立场分析：
/// - `Bullish`: 看涨
/// - `Bearish`: 看跌
/// - `Neutral`: 中性
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalystStance {
    Bullish,
    Bearish,
    Neutral,
}

impl AnalystStance {
    /// 中文标签
    pub fn label(self) -> &'static str {
        match self {
            Self::Bullish => "看涨",
            Self::Bearish => "看跌",
            Self::Neutral => "中性",
        }
    }
}

impl std::fmt::Display for AnalystStance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volatility_from_pct() {
        assert_eq!(VolatilityLevel::from_bollinger_pct(0.25), VolatilityLevel::High);
        assert_eq!(VolatilityLevel::from_bollinger_pct(0.15), VolatilityLevel::Normal);
        assert_eq!(VolatilityLevel::from_bollinger_pct(0.05), VolatilityLevel::Low);
    }

    #[test]
    fn test_serde_roundtrip() {
        let regime = MarketRegimeType::Bull;
        let json = serde_json::to_string(&regime).unwrap();
        assert_eq!(json, "\"bull\"");
        let back: MarketRegimeType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, MarketRegimeType::Bull);
    }
}
