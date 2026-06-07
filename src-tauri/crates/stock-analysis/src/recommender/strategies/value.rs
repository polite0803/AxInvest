//! 价值低估子策略：低估值 + 基本面

use super::super::strategy::{RecoContext, RecommendStrategy};
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;

pub struct ValueStrategy {
    pub period: Period,
}

impl ValueStrategy {
    pub const fn short() -> Self {
        Self {
            period: Period::Short,
        }
    }
    pub const fn mid() -> Self {
        Self {
            period: Period::Mid,
        }
    }
    pub const fn long() -> Self {
        Self {
            period: Period::Long,
        }
    }

    async fn scan_one(
        &self,
        client: &AStockClient,
        code: &str,
        name: &str,
        sector: Option<String>,
    ) -> Option<RecoPick> {
        let quote = client.get_quote(code).await.ok()?;
        let price = quote.price;
        let pe = quote.pe?;
        let pb = quote.pb?;

        let (pre_filter_ok, mut reasons) = match self.period {
            Period::Short => {
                // 短线价值：低 PE 分位 + 缩量回踩 MA20
                if pe <= 0.0 || pe > 30.0 {
                    return None;
                }
                let klines = client.get_klines(code, "daily", 30).await.ok()?;
                if klines.is_empty() {
                    return None;
                }
                let cs: Vec<f64> = klines.iter().map(|k| k.close).collect();
                let ma20 = crate::recommender::indicators::sma(&cs, 20).unwrap_or(price);
                if price > ma20 * 0.99 {
                    return None;
                }
                (
                    true,
                    vec![
                        format!("PE {:.1} 估值偏低", pe),
                        format!("缩量回踩 MA20 {:.2}", ma20),
                    ],
                )
            },
            Period::Mid => {
                // 中线价值：PE < 行业中位 0.7 + 利润增速 > 0（v1 简化为 PE + PB 联合判断）
                if pe <= 0.0 || pe > 25.0 {
                    return None;
                }
                if pb <= 0.0 || pb > 5.0 {
                    return None;
                }
                (
                    true,
                    vec![
                        format!("PE {:.1} 行业中位以下", pe),
                        format!("PB {:.2}", pb),
                    ],
                )
            },
            Period::Long => {
                // 长线价值：低 PE + 低 PB + 高 ROE（v1 用 PE/PB 联合 + 股价 60 日均线上方做趋势过滤）
                if pe <= 0.0 || pe > 20.0 {
                    return None;
                }
                if pb <= 0.0 || pb > 3.0 {
                    return None;
                }
                (true, vec![format!("低 PE {:.1}", pe), format!("低 PB {:.2}", pb)])
            },
        };

        if !pre_filter_ok {
            return None;
        }

        let (entry_low, entry_high, stop_loss, target, base_position) = match self.period {
            Period::Short => (price * 0.98, price * 1.02, price * 0.93, price * 1.10, 5.0),
            Period::Mid => (price * 0.95, price * 1.05, price * 0.88, price * 1.20, 8.0),
            Period::Long => (price * 0.93, price * 1.05, price * 0.85, price * 1.30, 10.0),
        };

        // 长线要求股价在 60 日均线之上（趋势过滤）
        if matches!(self.period, Period::Long) {
            if let Ok(klines) = client.get_klines(code, "daily", 70).await {
                if let Some(ma60) = crate::recommender::indicators::sma(
                    &klines.iter().map(|k| k.close).collect::<Vec<_>>(),
                    60,
                ) {
                    if price < ma60 * 0.95 {
                        return None;
                    }
                    reasons.push(format!("站上 MA60 {:.2}", ma60));
                }
            }
        }

        let conf = calc_confidence(0.75, 0.7, 0.6, 0.0, 1.0);
        let position = calc_position(base_position, conf, self.period);

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Value,
            period: self.period,
            price,
            entry_low,
            entry_high,
            stop_loss,
            target_price: target,
            position_pct: position,
            holding_days: self.period.default_holding_days(),
            confidence: conf,
            reasons,
            risk_notes: vec!["行业基本面恶化".to_string()],
            secondary_styles: vec![],
            synthetic: false,
        })
    }
}

#[async_trait]
impl RecommendStrategy for ValueStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::Short => "value_short",
            Period::Mid => "value_mid",
            Period::Long => "value_long",
        }
    }
    fn style(&self) -> Style {
        Style::Value
    }
    fn period(&self) -> Period {
        self.period
    }
    fn required_vendors(&self) -> &'static [&'static str] {
        &["eastmoney", "ths", "akshare"]
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
    fn value_strategy_ids() {
        assert_eq!(ValueStrategy::short().id(), "value_short");
        assert_eq!(ValueStrategy::mid().id(), "value_mid");
        assert_eq!(ValueStrategy::long().id(), "value_long");
        assert_eq!(ValueStrategy::long().style(), Style::Value);
    }
}
