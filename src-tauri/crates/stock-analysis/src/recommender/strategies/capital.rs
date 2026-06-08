//! 资金驱动子策略：主力净流入 / 北向加仓 / 龙虎榜
//!
//! 注：v1 资金流向数据仅当日，逻辑上做了"日级窗口"近似；
//! 真正多日窗口需后续扩 AStockClient 接口。

use super::super::strategy::{read_f64, RecoContext, RecommendStrategy};
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;
use serde_json::Value;
use std::collections::HashMap;

pub struct CapitalStrategy {
    pub period: Period,
}

impl CapitalStrategy {
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
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        let quote = client.get_quote(code).await.ok()?;
        let price = quote.price;

        let mf = client.get_money_flow(code).await.ok().flatten();
        let nb = client.get_north_bound_holding(code).await.ok().flatten();
        let dt = client.get_dragon_tiger(code).await.ok();

        // 三个资金数据源至少有一个能拿到真实数据，否则视为数据缺失直接放弃
        if mf.is_none() && nb.is_none() && dt.as_ref().map_or(true, |e| e.is_empty()) {
            return None;
        }

        // 主力净流入
        let main_inflow_wan = mf
            .as_ref()
            .map(|m| m.main_net_inflow / 10_000.0)
            .unwrap_or(0.0);
        // 北向持仓占比
        let nb_ratio = nb.as_ref().map(|n| n.holding_ratio).unwrap_or(0.0);
        // 龙虎榜净买入
        let dt_net_wan = dt
            .as_ref()
            .map(|entries| entries.iter().map(|e| e.net_amount).sum::<f64>() / 10_000.0)
            .unwrap_or(0.0);

        let (pass, reasons) = match self.period {
            Period::Short => {
                let main_inflow_min = read_f64(vars, "cap_short_main_inflow_min", 200.0);
                if main_inflow_wan < main_inflow_min {
                    return None;
                }
                let turnover_min = read_f64(vars, "cap_short_turnover_min", 2.0);
                if quote.turnover_rate < turnover_min {
                    return None;
                }
                (
                    true,
                    vec![
                        format!("主力净流入 {:.0} 万", main_inflow_wan),
                        format!("换手 {:.2}%", quote.turnover_rate),
                    ],
                )
            },
            Period::Mid => {
                let nb_ratio_min = read_f64(vars, "cap_mid_nb_ratio_min", 0.3);
                let main_inflow_min = read_f64(vars, "cap_mid_main_inflow_min", 500.0);
                if nb_ratio < nb_ratio_min && main_inflow_wan < main_inflow_min {
                    return None;
                }
                let mut r = Vec::new();
                if main_inflow_wan > 0.0 {
                    r.push(format!("主力净流入 {:.0} 万", main_inflow_wan));
                }
                if nb_ratio > 0.0 {
                    r.push(format!("北向持仓 {:.2}%", nb_ratio));
                }
                if dt_net_wan.abs() > 0.0 {
                    r.push(format!("龙虎榜净买入 {:.0} 万", dt_net_wan));
                }
                (true, r)
            },
            Period::Long => {
                let nb_ratio_min = read_f64(vars, "cap_long_nb_ratio_min", 0.1);
                let main_inflow_min = read_f64(vars, "cap_long_main_inflow_min", 100.0);
                if nb_ratio < nb_ratio_min && main_inflow_wan < main_inflow_min {
                    return None;
                }
                let r = if nb_ratio > 0.0 {
                    vec![format!("北向长期持仓 {:.2}%", nb_ratio)]
                } else {
                    vec![format!("主力长期净流入 {:.0} 万", main_inflow_wan)]
                };
                (true, r)
            },
        };

        if !pass {
            return None;
        }

        let (entry_low, entry_high, stop_loss, target, base_position) = match self.period {
            Period::Short => {
                let el = read_f64(vars, "cap_short_entry_low", 0.97);
                let eh = read_f64(vars, "cap_short_entry_high", 1.03);
                let sl = read_f64(vars, "cap_short_stop", 0.93);
                let tg = read_f64(vars, "cap_short_target", 1.10);
                let bp = read_f64(vars, "cap_short_base_pos", 5.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Mid => {
                let el = read_f64(vars, "cap_mid_entry_low", 0.95);
                let eh = read_f64(vars, "cap_mid_entry_high", 1.05);
                let sl = read_f64(vars, "cap_mid_stop", 0.90);
                let tg = read_f64(vars, "cap_mid_target", 1.20);
                let bp = read_f64(vars, "cap_mid_base_pos", 8.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Long => {
                let el = read_f64(vars, "cap_long_entry_low", 0.95);
                let eh = read_f64(vars, "cap_long_entry_high", 1.05);
                let sl = read_f64(vars, "cap_long_stop", 0.88);
                let tg = read_f64(vars, "cap_long_target", 1.30);
                let bp = read_f64(vars, "cap_long_base_pos", 10.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
        };

        let conf = calc_confidence(
            read_f64(vars, "cap_conf_consistency", 0.80),
            read_f64(vars, "cap_conf_signal", 0.7),
            read_f64(vars, "cap_conf_direction", 0.7),
            read_f64(vars, "cap_conf_market", 0.0),
            read_f64(vars, "cap_conf_base", 1.0),
        );
        let position = calc_position(base_position, conf, self.period);

        let risk = match self.period {
            Period::Short => vec!["次日冲高回落".to_string()],
            Period::Mid => vec!["资金切换 / 主力出货".to_string()],
            Period::Long => vec!["机构持仓变动".to_string()],
        };

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Capital,
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
            risk_notes: risk,
            secondary_styles: vec![],
            synthetic: false,
        })
    }
}

#[async_trait]
impl RecommendStrategy for CapitalStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::Short => "capital_short",
            Period::Mid => "capital_mid",
            Period::Long => "capital_long",
        }
    }
    fn style(&self) -> Style {
        Style::Capital
    }
    fn period(&self) -> Period {
        self.period
    }
    fn required_vendors(&self) -> &'static [&'static str] {
        &["ths", "baidu_stock"]
    }

    async fn scan(&self, ctx: &RecoContext<'_>) -> Result<Vec<RecoPick>, String> {
        let mut picks = Vec::new();
        for (code, name, sector) in ctx.seed {
            let _g = ctx.per_code_locks.lock_for(code).await;
            if let Some(p) = self
                .scan_one(ctx.client, code, name, sector.clone(), ctx.vars)
                .await
            {
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
    fn capital_strategy_ids() {
        assert_eq!(CapitalStrategy::short().id(), "capital_short");
        assert_eq!(CapitalStrategy::mid().id(), "capital_mid");
        assert_eq!(CapitalStrategy::long().id(), "capital_long");
    }
}
