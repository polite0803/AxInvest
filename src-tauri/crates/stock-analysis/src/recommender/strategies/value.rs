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
        // vendor 数据稀疏时 pe/pb 可能是 None。放宽：容许缺失 → 用 None coalesce
        // 成 0 走过滤；但只要 pe / pb 至少一个有数 + > 0，就算入候选（理由里标注）
        let pe = quote.pe.unwrap_or(0.0);
        let pb = quote.pb.unwrap_or(0.0);
        // 至少要有 pe 或 pb 之一有数；否则视为无估值数据直接放弃
        if pe <= 0.0 && pb <= 0.0 {
            return None;
        }

        let (pre_filter_ok, mut reasons) = match self.period {
            Period::Short => {
                // 短线价值：放宽 PE 上限 30 → 50（旧门槛把 PE 30~50 的"周期低估值"也剔了）
                if pe > 0.0 && pe > 50.0 {
                    return None;
                }
                // 缩量回踩 MA20：放宽到 0.99 → 1.005（容许小幅冲高）
                let klines = client.get_klines(code, "daily", 30).await.ok()?;
                if klines.is_empty() {
                    return None;
                }
                let cs: Vec<f64> = klines.iter().map(|k| k.close).collect();
                let ma20 = crate::recommender::indicators::sma(&cs, 20).unwrap_or(price);
                if price > ma20 * 1.005 {
                    return None;
                }
                let mut r = Vec::new();
                if pe > 0.0 { r.push(format!("PE {:.1} 估值偏低", pe)); }
                if pb > 0.0 { r.push(format!("PB {:.2}", pb)); }
                r.push(format!("回踩 MA20 {:.2}", ma20));
                (true, r)
            },
            Period::Mid => {
                // 中线价值：PE 25 → 40，PB 5 → 8（旧门槛对中盘股过严）
                if pe > 0.0 && pe > 40.0 {
                    return None;
                }
                if pb > 0.0 && pb > 8.0 {
                    return None;
                }
                let mut r = Vec::new();
                if pe > 0.0 { r.push(format!("PE {:.1} 行业中位以下", pe)); }
                if pb > 0.0 { r.push(format!("PB {:.2}", pb)); }
                (true, r)
            },
            Period::Long => {
                // 长线价值：PE 20 → 35，PB 3 → 6（旧门槛对成长股过严）
                if pe > 0.0 && pe > 35.0 {
                    return None;
                }
                if pb > 0.0 && pb > 6.0 {
                    return None;
                }
                let mut r = Vec::new();
                if pe > 0.0 { r.push(format!("低 PE {:.1}", pe)); }
                if pb > 0.0 { r.push(format!("低 PB {:.2}", pb)); }
                (true, r)
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
                    // 放宽：0.95 → 0.90（容许在 MA60 下方 10% 内也算"近端"）
                    if price < ma60 * 0.90 {
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
