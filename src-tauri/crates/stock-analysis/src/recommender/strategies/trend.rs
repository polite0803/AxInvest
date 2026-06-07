//! 趋势跟踪子策略：MA 多头 + 突破 + 量能

use super::super::strategy::{RecoContext, RecommendStrategy};
use crate::recommender::indicators;
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;

pub struct TrendStrategy {
    pub period: Period,
}

impl TrendStrategy {
    pub const fn short() -> Self { Self { period: Period::Short } }
    pub const fn mid() -> Self { Self { period: Period::Mid } }
    pub const fn long() -> Self { Self { period: Period::Long } }

    async fn scan_one(
        &self,
        client: &AStockClient,
        code: &str,
        name: &str,
        sector: Option<String>,
    ) -> Option<RecoPick> {
        let klines = client.get_klines(code, "daily", 250).await.ok()?;
        if klines.len() < 60 {
            return None;
        }
        let cs = indicators::closes(&klines);
        let last = *cs.last()?;

        // 拉取 20 日均成交额，量比估算
        let avg_20 = indicators::avg_amount_20d(&klines).unwrap_or(0.0);
        let today_amount = klines.last().map(|k| k.amount).unwrap_or(0.0);
        let turnover_anomaly = if avg_20 > 0.0 { today_amount / avg_20 } else { 1.0 };
        let amount_ratio = turnover_anomaly; // 量比近似

        let (entry_low, entry_high, stop_loss, target_price, base_position, reasons) = match self.period {
            Period::Short => {
                // MA5/10/20 多头
                let ma5 = indicators::sma(&cs, 5)?;
                let ma10 = indicators::sma(&cs, 10)?;
                let ma20 = indicators::sma(&cs, 20)?;
                if !(ma5 > ma10 && ma10 > ma20) {
                    return None;
                }
                // 突破 20 日新高
                let high_20 = indicators::highest(&klines, 20)?;
                if last < high_20 * 0.998 {
                    return None;
                }
                // 量比 > 1.2
                if amount_ratio < 1.2 {
                    return None;
                }
                let reasons = vec![
                    format!("MA5 {:.2} > MA10 {:.2} > MA20 {:.2}", ma5, ma10, ma20),
                    format!("突破 20 日高 {:.2}", high_20),
                    format!("量比 {:.2}", amount_ratio),
                ];
                let _atr = (high_20 - indicators::lowest(&klines, 20).unwrap_or(ma20 * 0.95)) * 0.5;
                (ma5 * 0.99, ma5 * 1.015, ma5 * 0.95, ma5 * 1.10, 5.0, reasons)
            },
            Period::Mid => {
                let ma20 = indicators::sma(&cs, 20)?;
                let ma60 = indicators::sma(&cs, 60)?;
                if ma60.is_nan() || last <= ma60 {
                    return None;
                }
                // 突破 60 日高
                let high_60 = indicators::highest(&klines, 60)?;
                if last < high_60 * 0.995 {
                    return None;
                }
                // MACD 红柱
                if let Some((dif, dea, macd_bar)) = indicators::macd(&klines, 12, 26, 9) {
                    if macd_bar <= 0.0 || dif <= dea {
                        return None;
                    }
                    let reasons = vec![
                        format!("站上 MA60 {:.2}", ma60),
                        format!("突破 60 日高 {:.2}", high_60),
                        format!("MACD 红柱 {:.2}", macd_bar),
                    ];
                    (ma20 * 0.97, ma20 * 1.05, ma20 * 0.92, high_60 * 1.05, 8.0, reasons)
                } else {
                    return None;
                }
            },
            Period::Long => {
                let ma60 = indicators::sma(&cs, 60)?;
                let ma250 = indicators::sma(&cs, 250)?;
                if ma250.is_nan() || ma60 <= ma250 {
                    return None;
                }
                // 不跌破 MA60（最新一根 close > MA60）
                if last < ma60 * 0.97 {
                    return None;
                }
                let reasons = vec![
                    format!("MA60 {:.2} > MA250 {:.2} 长期多头", ma60, ma250),
                    format!("回踩未破 MA60"),
                ];
                (ma60 * 0.95, ma60 * 1.03, ma60 * 0.85, last * 1.30, 10.0, reasons)
            },
        };

        // 置信度
        let conf = calc_confidence(
            0.85,                    // 子策略内多因子方向一致
            0.7,                     // 信号强度
            if amount_ratio > 1.5 { 0.8 } else { 0.5 },
            0.0,                     // market_regime 占位，v1 简化为 0
            turnover_anomaly,
        );
        let position = calc_position(base_position, conf, self.period);

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Trend,
            period: self.period,
            price: last,
            entry_low,
            entry_high,
            stop_loss,
            target_price,
            position_pct: position,
            holding_days: self.period.default_holding_days(),
            confidence: conf,
            reasons,
            risk_notes: vec!["大盘破 20 日均线 -10%".to_string()],
            secondary_styles: vec![],
        })
    }
}

#[async_trait]
impl RecommendStrategy for TrendStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::Short => "trend_short",
            Period::Mid => "trend_mid",
            Period::Long => "trend_long",
        }
    }
    fn style(&self) -> Style { Style::Trend }
    fn period(&self) -> Period { self.period }
    fn required_vendors(&self) -> &'static [&'static str] {
        &["eastmoney", "tencent", "ths", "akshare"]
    }

    async fn scan(&self, ctx: &RecoContext<'_>) -> Result<Vec<RecoPick>, String> {
        let mut picks = Vec::new();
        for (code, name, sector) in ctx.seed {
            let _g = ctx.per_code_locks.lock_for(code).await;
            if let Some(p) = self.scan_one(ctx.client, code, name, sector.clone()).await {
                picks.push(p);
            }
        }
        Ok(picks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trend_strategy_ids() {
        assert_eq!(TrendStrategy::short().id(), "trend_short");
        assert_eq!(TrendStrategy::mid().id(), "trend_mid");
        assert_eq!(TrendStrategy::long().id(), "trend_long");
        assert_eq!(TrendStrategy::short().style(), Style::Trend);
        assert_eq!(TrendStrategy::mid().period(), Period::Mid);
    }
}
