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
    pub const fn ultra_short() -> Self {
        Self { period: Period::UltraShort }
    }
    pub const fn short() -> Self {
        Self { period: Period::Short }
    }
    pub const fn mid() -> Self {
        Self { period: Period::Mid }
    }
    // as-of K线代理回退

    /// as-of 模式下 money_flow / north_bound 不可用，回退到 K 线量价检测
    async fn scan_from_klines(
        &self,
        client: &AStockClient,
        code: &str,
        name: &str,
        sector: Option<String>,
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        let quote = client.get_quote(code).await.ok()?;
        let price = quote.price;
        // 尺度由档位决定（K线代理回退也须按档取尺度，否则「长档看 20 个交易日」这种
        // 与 90 天持有期脱节的口径会继续留在产出里）。
        let (klines, profile) =
            match crate::recommender::scale::fetch(client, code, self.period).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("[capital] {code} 档位 {:?} 取数失败: {e}", self.period);
                    return None;
                },
            };
        if !crate::recommender::scale::enough_bars(&klines, &profile) {
            return None;
        }

        // 量价信号：最近 5 日平均量 / 20 日平均量 > 阈值
        // 窗口按**日历日**换算到当前尺度（5 日 / 20 日），并保证至少 5 根 bar 才算「平均」
        let w5 = (crate::recommender::scale::bars_for_daily_span(5, &profile)).max(5);
        let w20 = (crate::recommender::scale::bars_for_daily_span(20, &profile)).max(5);
        let vols_5: Vec<f64> = klines.iter().rev().take(w5).map(|k| k.amount).collect();
        let avg_vol_5 = vols_5.iter().sum::<f64>() / (vols_5.len() as f64).max(1.0);
        let vols_20: Vec<f64> = klines.iter().rev().take(w20).map(|k| k.amount).collect();
        let avg_vol_20 = vols_20.iter().sum::<f64>() / (vols_20.len() as f64).max(1.0);
        let vol_ratio = if avg_vol_20 > 0.0 {
            avg_vol_5 / avg_vol_20
        } else {
            1.0
        };

        // 价格动量：近 w5 根 bar 的涨幅（= 日历 5 个交易日换算来的 bar 数）
        let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
        let mom_5 = if closes.len() > w5 {
            closes[closes.len() - 1] / closes[closes.len() - 1 - w5] - 1.0
        } else {
            0.0
        };

        let vol_ratio_min = read_f64(vars, "cap_kline_vol_ratio_min", 1.5);
        let mom_5_min = read_f64(vars, "cap_kline_mom_5_min", -0.02);
        let mom_5_max = read_f64(vars, "cap_kline_mom_5_max", 0.10);

        if vol_ratio < vol_ratio_min || mom_5 < mom_5_min || mom_5 > mom_5_max {
            return None;
        }

        // 计算置信度（量比贡献 0.6，动量贡献 0.4）
        let vol_score = ((vol_ratio - 1.0) / 3.0).clamp(0.0, 1.0);
        let mom_score = ((mom_5 - mom_5_min) / (mom_5_max - mom_5_min)).clamp(0.0, 1.0);
        let conf_raw = 0.6 * vol_score + 0.4 * mom_score;

        // 持有天数一律取 `Period::default_holding_days()`（全仓唯一天数源，`harness::holding_period`）。
        // 此前这里自带一张逐档表且 short=**7**，而权威源 short=**5** ⇒ 荐股按 7 天判成熟、
        // 反思/回测按 5 天分档，两处各自都「没 bug」但互相错档（门 a 的第一条靶）。
        let (entry_low, entry_high, stop_loss, target, base_position) = match self.period {
            Period::UltraShort => (price * 0.998, price * 1.005, price * 0.97, price * 1.05, 3.0),
            Period::Short => (price * 0.97, price * 1.03, price * 0.93, price * 1.10, 5.0),
            Period::Mid => (price * 0.95, price * 1.05, price * 0.90, price * 1.20, 8.0),
            Period::Long => (price * 0.95, price * 1.05, price * 0.88, price * 1.30, 10.0),
        };

        let conf = (conf_raw * 100.0).round() as u8;
        let position = calc_position(base_position, conf);

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
            reasons: vec![
                format!("K线量比 {:.2}x", vol_ratio),
                format!("5日动量 {:.2}%", mom_5 * 100.0),
            ],
            risk_notes: vec!["K线代理模式：无资金流向数据，仅基于量价".to_string()],
            secondary_styles: vec![],
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            stop_source: None,
            position_source: None,

            synthetic: true,
        })
    }

    pub const fn long() -> Self {
        Self { period: Period::Long }
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

        // 检测"空壳" MoneyFlow：Tencent 等vendor 返回了对象但关键字段全为 0，
        // 这种情况等同于数据不可用，应回退到 K 线量价检测。
        let mf_is_effective = |m: &axagent_astock_data::MoneyFlow| -> bool {
            // None = 该源未披露这一档 ⇒ 与 0 一样不计入「有内容」，不改变原有判定语义
            m.main_net_inflow != 0.0
                || m.super_large_net.unwrap_or(0.0) != 0.0
                || m.large_net.unwrap_or(0.0) != 0.0
                || m.medium_net.unwrap_or(0.0) != 0.0
                || m.small_net.unwrap_or(0.0) != 0.0
        };

        // eastmoney 资金流向被反爬拦截，或 vendor 返回零值空壳数据时回退到 K 线量价检测。
        //
        // 顺序要求：本判断必须在拉 north_bound / dragon_tiger **之前**。
        // 旧实现无条件先拉三个源再判断，导致"资金流已注定走 K 线代理"的股票
        // 仍要付出 north_bound + dragon_tiger 的完整请求与重试代价
        // （北向个股持仓已全源失效，单只冷启动 3 源 × 3 轮重试实测 ≈ 8.5s；
        //   筛池 80+ 只累加即达分钟级，期间租户只能干等）。
        if mf.as_ref().is_none_or(|m| !mf_is_effective(m)) {
            return self.scan_from_klines(client, code, name, sector, vars).await;
        }

        // 仅当资金流确实有效、要走资金驱动路径时，才继续拉另外两个源
        let nb = client.get_north_bound_holding(code).await.ok().flatten();
        let dt = client.get_dragon_tiger(code).await.ok();

        // 三个资金数据源在 as-of 下可能全部不可用，回退到 K 线量价检测
        if nb.is_none() && dt.as_ref().is_none_or(|e| e.is_empty()) {
            return self.scan_from_klines(client, code, name, sector, vars).await;
        }

        // 主力净流入
        let main_inflow_wan = mf.as_ref().map(|m| m.main_net_inflow / 10_000.0).unwrap_or(0.0);
        // 北向持仓占比
        let nb_ratio = nb.as_ref().map(|n| n.holding_ratio).unwrap_or(0.0);
        // 龙虎榜净买入
        let dt_net_wan = dt
            .as_ref()
            .map(|entries| entries.iter().map(|e| e.net_amount).sum::<f64>() / 10_000.0)
            .unwrap_or(0.0);

        let (pass, reasons) = match self.period {
            Period::UltraShort => {
                // 超短线只看龙虎榜 — 游资隔夜行为
                let dt_net_min = read_f64(vars, "cap_ultra_short_dt_net_min", 100.0);
                if dt_net_wan.abs() < dt_net_min {
                    return None;
                }
                let turnover_min = read_f64(vars, "cap_ultra_short_turnover_min", 5.0);
                if quote.turnover_rate < turnover_min {
                    return None;
                }
                let mut r = Vec::new();
                if dt_net_wan > 0.0 {
                    r.push(format!("龙虎榜净买入 {:.0} 万", dt_net_wan));
                } else {
                    r.push(format!("龙虎榜净卖出 {:.0} 万（反弹博弈）", dt_net_wan.abs()));
                }
                r.push(format!("换手 {:.2}%", quote.turnover_rate));
                (true, r)
            },
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
            Period::UltraShort => {
                let el = read_f64(vars, "cap_ultra_short_entry_low", 0.998);
                let eh = read_f64(vars, "cap_ultra_short_entry_high", 1.005);
                let sl = read_f64(vars, "cap_ultra_short_stop", 0.97);
                let tg = read_f64(vars, "cap_ultra_short_target", 1.05);
                let bp = read_f64(vars, "cap_ultra_short_base_pos", 3.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
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
            1.0, // turnover_anomaly: 资金驱动策略无量比数据（有 money_flow），默认无惩罚
        );
        let position = calc_position(base_position, conf);

        let risk = match self.period {
            Period::UltraShort => vec!["次日冲高回落 / T+1 无法止损".to_string()],
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
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            stop_source: None,
            position_source: None,

            synthetic: false,
        })
    }
}

#[async_trait]
impl RecommendStrategy for CapitalStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::UltraShort => "capital_ultra_short",
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
            if let Some(p) = self.scan_one(ctx.client, code, name, sector.clone(), ctx.vars).await {
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
        assert_eq!(CapitalStrategy::ultra_short().id(), "capital_ultra_short");
        assert_eq!(CapitalStrategy::short().id(), "capital_short");
        assert_eq!(CapitalStrategy::mid().id(), "capital_mid");
        assert_eq!(CapitalStrategy::long().id(), "capital_long");
    }
}
