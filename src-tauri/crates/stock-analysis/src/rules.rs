use axagent_astock_data::indicators::TechnicalIndicators;

use crate::scoring::ObjectiveScore;

/// 规则检查结果
#[derive(Debug, Clone)]
pub struct RuleCheckResult {
    pub passed: bool,
    pub violations: Vec<String>,
    pub corrections: Vec<String>,
    pub force_signal: Option<String>, // 强制覆盖的信号
}

/// 严进规则引擎 -- 巴菲特+段永平投资纪律
pub struct RuleEngine;

impl RuleEngine {
    /// 检查交易方案是否违反硬性规则
    pub fn check(
        indicators: &TechnicalIndicators,
        score: &ObjectiveScore,
        proposed_action: &str,
        proposed_stop_loss: Option<f64>,
        proposed_entry_price: Option<f64>,
    ) -> RuleCheckResult {
        let mut violations = Vec::new();
        let mut corrections = Vec::new();
        let mut force_signal = None;

        let is_buy = matches!(proposed_action, "买入" | "增持");

        // 规则1: RSI > 80 -> 绝不给买入信号
        if is_buy && indicators.rsi6 > 80.0 {
            violations.push(format!(
                "RSI6={:.1}>80处于严重超买，禁止买入。建议等待回调至RSI<60。",
                indicators.rsi6
            ));
            force_signal = Some("🔴强制调整为观望 — RSI超买".to_string());
        }

        // 规则2: 乖离MA5 > 5% -> 绝不给买入信号（不追高）
        if is_buy && indicators.bias_ma5 > 5.0 {
            violations.push(format!(
                "乖离MA5={:.1}%>5%，禁止追高。建议等待回调至MA5附近（约{:.2}）。",
                indicators.bias_ma5, indicators.ma5
            ));
            force_signal = Some("🔴强制调整为观望 — 乖离率过高".to_string());
        }

        // 规则3: 必须给出精确止损价
        if proposed_stop_loss.is_none() || proposed_stop_loss == Some(0.0) {
            violations.push("缺少止损价位，不符合严进策略要求。".to_string());
            // 自动计算建议止损：MA20 或 入场价-5%
            let auto_stop = if let Some(entry) = proposed_entry_price {
                let ma20_stop = indicators.ma20;
                let pct_stop = entry * 0.95;
                if ma20_stop > pct_stop {
                    ma20_stop
                } else {
                    pct_stop
                }
            } else {
                indicators.ma20
            };
            let entry = proposed_entry_price.unwrap_or(0.0);
            corrections.push(format!(
                "自动设定止损价: {:.2}（MA20={:.2}和-5%止损={:.2}取较大值）",
                auto_stop,
                indicators.ma20,
                entry * 0.95
            ));
        }

        // 规则4: 放量下跌 -> 禁止买入
        if is_buy && indicators.volume_signal == "放量下跌" {
            violations.push("放量下跌中禁止买入，等待缩量企稳信号。".to_string());
            force_signal = Some("🔴强制调整为观望 — 放量下跌".to_string());
        }

        // 规则5: 空头排列且评分<30 -> 强制卖出/观望
        if is_buy && indicators.ma_alignment == "空头排列" && score.total < 30 {
            violations.push("空头排列+评分<30，绝不适合买入。".to_string());
            force_signal = Some("🔴强制调整为观望/卖出 — 空头低分".to_string());
        }

        // 规则6: RSI < 20 -> 关注超跌反弹（提醒）
        if indicators.rsi6 < 20.0 {
            corrections.push(format!(
                "提示: RSI6={:.1}<20处于超卖区域，关注超跌反弹机会。",
                indicators.rsi6
            ));
        }

        RuleCheckResult {
            passed: violations.is_empty(),
            violations,
            corrections,
            force_signal,
        }
    }
}
