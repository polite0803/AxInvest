//! 候选池兜底策略：仅依赖 `get_quote`，永远能出 picks
//!
//! 当 4 个主策略（trend/value/capital/reversion）因网络/数据不可用全部返回空时，
//! 这个策略保证面板至少展示 seed pool 的"系统初筛"列表，附带基于现价的合成
//! 入场/止损/目标位（按周期调整振幅）。
//!
//! 不做任何技术信号判定，所以"confidence"和"position"都给得保守（低 0.5 / 0.4 base），
//! 真实仓位由用户在账户页面自己分配。

use super::super::strategy::{RecoContext, RecommendStrategy};
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;

pub struct WatchlistStrategy {
    pub period: Period,
}

impl WatchlistStrategy {
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
        // 只 fetch 实时 quote，不依赖 K 线 / 财务 / 资金流，确保尽量总有数据
        let quote = client.get_quote(code).await.ok()?;
        if quote.is_st {
            return None;
        }
        if quote.price <= 0.0 {
            return None;
        }
        let price = quote.price;

        // 按周期给不同的振幅
        let (entry_low, entry_high, stop_loss, target_price, base_position, holding_days, reason) =
            match self.period {
                Period::Short => (
                    price * 0.99,
                    price * 1.01,
                    price * 0.96,
                    price * 1.06,
                    4.0,
                    5,
                    "候选池初筛（短线：日内-1% 进场，+6% 目标）",
                ),
                Period::Mid => (
                    price * 0.97,
                    price * 1.03,
                    price * 0.92,
                    price * 1.15,
                    6.0,
                    28,
                    "候选池初筛（中线：-3% 进场，+15% 目标）",
                ),
                Period::Long => (
                    price * 0.95,
                    price * 1.05,
                    price * 0.88,
                    price * 1.25,
                    8.0,
                    90,
                    "候选池初筛（长线：-5% 进场，+25% 目标）",
                ),
            };

        // 信心度低（0.55），因为没有技术信号支撑
        let conf = calc_confidence(0.55, 0.5, 0.5, 0.0, 1.0);
        let position = calc_position(base_position, conf, self.period);

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Watchlist,
            period: self.period,
            price,
            entry_low,
            entry_high,
            stop_loss,
            target_price,
            position_pct: position,
            holding_days,
            confidence: conf,
            reasons: vec![reason.to_string()],
            risk_notes: vec![
                "初筛列表，无技术信号，请结合其他风格确认".to_string(),
                "若数据源恢复可重新拉取获取更准确入场区间".to_string(),
            ],
            secondary_styles: vec![],
        })
    }
}

#[async_trait]
impl RecommendStrategy for WatchlistStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::Short => "watchlist_short",
            Period::Mid => "watchlist_mid",
            Period::Long => "watchlist_long",
        }
    }
    fn style(&self) -> Style {
        Style::Watchlist
    }
    fn period(&self) -> Period {
        self.period
    }
    fn required_vendors(&self) -> &'static [&'static str] {
        // 任意实时 quote vendor 即可
        &["tencent", "eastmoney", "mootdx"]
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
    fn watchlist_strategy_ids() {
        assert_eq!(WatchlistStrategy::short().id(), "watchlist_short");
        assert_eq!(WatchlistStrategy::mid().id(), "watchlist_mid");
        assert_eq!(WatchlistStrategy::long().id(), "watchlist_long");
        assert_eq!(WatchlistStrategy::short().style(), Style::Watchlist);
    }
}
