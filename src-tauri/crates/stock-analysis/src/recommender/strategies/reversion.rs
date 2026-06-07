//! 超跌反弹子策略：RSI 超卖 / 底背离 / 缩量回踩
//!
//! v1 不做长线（设计文档 §2.3）

use super::super::strategy::{RecoContext, RecommendStrategy};
use crate::recommender::indicators;
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;

pub struct ReversionStrategy {
    pub period: Period,
}

impl ReversionStrategy {
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

    async fn scan_one(
        &self,
        client: &AStockClient,
        code: &str,
        name: &str,
        sector: Option<String>,
    ) -> Option<RecoPick> {
        let klines = client.get_klines(code, "daily", 250).await.ok()?;
        // 放宽 K 线最低长度：60 → 30
        if klines.len() < 30 {
            return None;
        }
        let price = klines.last()?.close;
        let rsi_value = indicators::rsi(&klines, 6)?;

        let (pass, reasons) = match self.period {
            Period::Short => {
                // RSI(6) 25 → 35（旧的 25 太严，市场常态 RSI 就在 30~70）
                if rsi_value >= 35.0 {
                    return None;
                }
                // 缩量：today < avg_5 * 0.8 → today < avg_5 * 1.2（容许温和放量也入选）
                let avg_5 = indicators::avg_amount_n(&klines, 5).unwrap_or(0.0);
                let today = klines.last().map(|k| k.amount).unwrap_or(0.0);
                if avg_5 <= 0.0 || today > avg_5 * 1.2 {
                    return None;
                }
                (
                    true,
                    vec![
                        format!("RSI(6) {:.1} 超卖", rsi_value),
                        format!("量比 5 日均 {:.0}%", today / avg_5 * 100.0),
                    ],
                )
            },
            Period::Mid => {
                // 距 250 日新高回撤 30% → 20%
                let dd = indicators::drawdown_from_high(&klines, 250).unwrap_or(0.0);
                if dd < 20.0 {
                    return None;
                }
                // 月线 RSI 40 → 50
                let rsi_30 = indicators::rsi(&klines, 30).unwrap_or(50.0);
                if rsi_30 > 50.0 {
                    return None;
                }
                (
                    true,
                    vec![
                        format!("距 250 日高回撤 {:.0}%", dd),
                        format!("月线 RSI {:.1}", rsi_30),
                    ],
                )
            },
            Period::Long => return None, // v1 不做长线超跌
        };

        if !pass {
            return None;
        }

        let (entry_low, entry_high, stop_loss, target, base_position) = match self.period {
            Period::Short => (price * 0.97, price * 1.03, price * 0.93, price * 1.08, 3.0),
            Period::Mid => (price * 0.95, price * 1.05, price * 0.88, price * 1.20, 5.0),
            Period::Long => return None,
        };

        let conf = calc_confidence(0.7, 0.8, 0.6, 0.0, 1.0);
        let position = calc_position(base_position, conf, self.period);

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Reversion,
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
            risk_notes: vec!["下跌趋势未尽 / 抄底过早".to_string()],
            secondary_styles: vec![],
            synthetic: false,
        })
    }
}

#[async_trait]
impl RecommendStrategy for ReversionStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::Short => "rev_short",
            Period::Mid => "rev_mid",
            Period::Long => "rev_long",
        }
    }
    fn style(&self) -> Style {
        Style::Reversion
    }
    fn period(&self) -> Period {
        self.period
    }
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
    fn reversion_strategy_ids() {
        assert_eq!(ReversionStrategy::short().id(), "rev_short");
        assert_eq!(ReversionStrategy::mid().id(), "rev_mid");
    }
}
