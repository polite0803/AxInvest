//! 价值低估子策略：低估值 + 基本面

use super::super::strategy::{read_f64, RecoContext, RecommendStrategy};
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_harness::market_data::MarketDataProvider;
use serde_json::Value;
use std::collections::HashMap;

/// value 个股证据强度（0~1，2026-10-18 四档常数塌缩修复）。
///
/// 便宜程度 = PE 相对该档上限的余量 60% + PB 余量 40%（上限与过滤门同源同变量）。
/// **pe ≤ 0（亏损/无披露）⇒ 整体 0** —— 亏损是价值策略的根本性否定信号，不是
/// 「缺一个维度」；pe_max ≤ 0（配置错误）同此处置。pb ≤ 0 / pb_max ≤ 0（无 PB 数据）
/// ⇒ 仅 PB 维度 0（保守：缺数据不奖励也不重归一权重）。
fn value_evidence_strength(pe: f64, pb: f64, pe_max: f64, pb_max: f64) -> f64 {
    if pe <= 0.0 || pe_max <= 0.0 {
        return 0.0;
    }
    let pe_headroom = ((pe_max - pe) / pe_max).clamp(0.0, 1.0);
    let pb_headroom = if pb > 0.0 && pb_max > 0.0 {
        ((pb_max - pb) / pb_max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (0.6 * pe_headroom + 0.4 * pb_headroom).clamp(0.0, 1.0)
}

pub struct ValueStrategy {
    pub period: Period,
}

impl ValueStrategy {
    pub const fn short() -> Self {
        Self { period: Period::Short }
    }
    pub const fn mid() -> Self {
        Self { period: Period::Mid }
    }
    pub const fn long() -> Self {
        Self { period: Period::Long }
    }
    pub const fn ultra_short() -> Self {
        Self { period: Period::UltraShort }
    }

    async fn scan_one(
        &self,
        client: &dyn MarketDataProvider,
        code: &str,
        name: &str,
        sector: Option<String>,
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        let quote = client.get_quote(code).await.ok()?;
        let price = quote.price;
        let pe = quote.pe.unwrap_or(0.0);
        let pb = quote.pb.unwrap_or(0.0);
        if pe <= 0.0 && pb <= 0.0 {
            return None;
        }

        // 尺度由档位决定，且四个分支**共用同一份取数**（旧形态是每个分支各取一次日线）。
        // 取不到该尺度 / bar 数不足 ⇒ 本风格本档不出票，不退回日线冒充。
        let (klines, profile) =
            match crate::recommender::scale::fetch(client, code, self.period).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("[value] {code} 档位 {:?} 取数失败: {e}", self.period);
                    return None;
                },
            };
        if !crate::recommender::scale::enough_bars(&klines, self.period) {
            return None;
        }
        let cs: Vec<f64> = klines.iter().map(|k| k.close).collect();

        let (pre_filter_ok, mut reasons) = match self.period {
            Period::UltraShort => {
                let pe_max = read_f64(vars, "val_ultra_short_pe_max", 60.0);
                if pe > 0.0 && pe > pe_max {
                    return None;
                }
                // 均线窗口 = 日历 10 日换算到当前尺度的 bar 数（下限 5 根）
                let ma_period = (crate::recommender::scale::bars_for_daily_span(
                    read_f64(vars, "val_ultra_short_ma_period", 10.0) as usize,
                    &profile,
                ))
                .max(5);
                let ma10 = crate::recommender::indicators::sma(&cs, ma_period)?;
                let ma_mult = read_f64(vars, "val_ultra_short_ma_mult", 1.005);
                if price > ma10 * ma_mult {
                    return None;
                }
                let mut r = Vec::new();
                if pe > 0.0 {
                    r.push(format!("PE {:.1} 严重压缩", pe));
                }
                if pb > 0.0 {
                    r.push(format!("PB {:.2}", pb));
                }
                r.push(format!("10日均线下方 {:.2}", ma10));
                (true, r)
            },
            Period::Short => {
                let pe_max = read_f64(vars, "val_short_pe_max", 50.0);
                if pe > 0.0 && pe > pe_max {
                    return None;
                }
                let ma_period = (crate::recommender::scale::bars_for_daily_span(
                    read_f64(vars, "val_short_ma_period", 20.0) as usize,
                    &profile,
                ))
                .max(5);
                let ma20 = crate::recommender::indicators::sma(&cs, ma_period)?;
                let ma_mult = read_f64(vars, "val_short_ma_mult", 1.005);
                if price > ma20 * ma_mult {
                    return None;
                }
                let mut r = Vec::new();
                if pe > 0.0 {
                    r.push(format!("PE {:.1} 估值偏低", pe));
                }
                if pb > 0.0 {
                    r.push(format!("PB {:.2}", pb));
                }
                r.push(format!("回踩 MA{} {:.2}", ma_period, ma20));
                (true, r)
            },
            Period::Mid => {
                let pe_max = read_f64(vars, "val_mid_pe_max", 40.0);
                if pe > 0.0 && pe > pe_max {
                    return None;
                }
                let pb_max = read_f64(vars, "val_mid_pb_max", 8.0);
                if pb > 0.0 && pb > pb_max {
                    return None;
                }
                let mut r = Vec::new();
                if pe > 0.0 {
                    r.push(format!("PE {:.1} 行业中位以下", pe));
                }
                if pb > 0.0 {
                    r.push(format!("PB {:.2}", pb));
                }
                (true, r)
            },
            Period::Long => {
                let pe_max = read_f64(vars, "val_long_pe_max", 35.0);
                if pe > 0.0 && pe > pe_max {
                    return None;
                }
                let pb_max = read_f64(vars, "val_long_pb_max", 6.0);
                if pb > 0.0 && pb > pb_max {
                    return None;
                }
                let mut r = Vec::new();
                if pe > 0.0 {
                    r.push(format!("低 PE {:.1}", pe));
                }
                if pb > 0.0 {
                    r.push(format!("低 PB {:.2}", pb));
                }
                (true, r)
            },
        };

        if !pre_filter_ok {
            return None;
        }

        let (entry_low, entry_high, stop_loss, target, base_position) = match self.period {
            Period::UltraShort => {
                let el = read_f64(vars, "val_ultra_short_entry_low", 0.998);
                let eh = read_f64(vars, "val_ultra_short_entry_high", 1.005);
                let sl = read_f64(vars, "val_ultra_short_stop", 0.97);
                let tg = read_f64(vars, "val_ultra_short_target", 1.03);
                let bp = read_f64(vars, "val_ultra_short_base_pos", 3.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Short => {
                let el = read_f64(vars, "val_short_entry_low", 0.98);
                let eh = read_f64(vars, "val_short_entry_high", 1.02);
                let sl = read_f64(vars, "val_short_stop", 0.93);
                let tg = read_f64(vars, "val_short_target", 1.10);
                let bp = read_f64(vars, "val_short_base_pos", 5.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Mid => {
                let el = read_f64(vars, "val_mid_entry_low", 0.95);
                let eh = read_f64(vars, "val_mid_entry_high", 1.05);
                let sl = read_f64(vars, "val_mid_stop", 0.88);
                let tg = read_f64(vars, "val_mid_target", 1.20);
                let bp = read_f64(vars, "val_mid_base_pos", 8.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Long => {
                let el = read_f64(vars, "val_long_entry_low", 0.93);
                let eh = read_f64(vars, "val_long_entry_high", 1.05);
                let sl = read_f64(vars, "val_long_stop", 0.85);
                let tg = read_f64(vars, "val_long_target", 1.30);
                let bp = read_f64(vars, "val_long_base_pos", 10.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
        };

        // 长线要求股价在「日历 60 日」均线之上（趋势过滤），用的是同一份按档取来的 bar 序列
        if matches!(self.period, Period::Long) {
            let ma_period = (crate::recommender::scale::bars_for_daily_span(
                read_f64(vars, "val_long_ma_period", 60.0) as usize,
                &profile,
            ))
            .max(5);
            if let Some(ma) = crate::recommender::indicators::sma(&cs, ma_period) {
                let ma_mult = read_f64(vars, "val_long_ma60_mult", 0.90);
                if price < ma * ma_mult {
                    return None;
                }
                reasons.push(format!("站上 MA{} {:.2}", ma_period, ma));
            }
        }

        // ── 个股证据强度（2026-10-18，四档常数塌缩修复）──
        // 此前 conf 五个输入**全部是模板常数** ⇒ value 出的每一只票原始置信完全相同，
        // 经合成后塌成档位常数。现以「便宜程度」调制 signal 槽：
        //   PE 相对该档上限的余量 60% + PB 余量 40%（上限走各档既有模板变量，与过滤门同源）
        // pe<=0（亏损/无披露）⇒ headroom 0 —— 价值策略不奖励亏损标的。
        let pe_max = read_f64(vars, &format!("val_{}_pe_max", self.period.as_str()), 40.0);
        let pb_max = read_f64(vars, &format!("val_{}_pb_max", self.period.as_str()), 6.0);
        let evidence = value_evidence_strength(pe, pb, pe_max, pb_max);
        let conf = calc_confidence(
            read_f64(vars, "val_conf_consistency", 0.75),
            read_f64(vars, "val_conf_signal", 0.7) * (0.6 + 0.4 * evidence),
            read_f64(vars, "val_conf_direction", 0.6),
            read_f64(vars, "val_conf_market", 0.0),
            1.0, // turnover_anomaly: 价值策略无量比数据，默认无惩罚
        );
        let position = calc_position(base_position, conf);

        let _risk = if matches!(self.period, Period::UltraShort) {
            "超短线估值博弈，次日即需监控"
        } else {
            "行业基本面恶化"
        };

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
            risk_notes: vec![_risk.to_string()],
            secondary_styles: vec![],
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            evidence_score: None,
            stop_source: None,
            position_source: None,

            synthetic: false,
        })
    }
}

#[async_trait]
impl RecommendStrategy for ValueStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::UltraShort => "value_ultra_short",
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
    fn value_evidence_headroom_and_fail_open() {
        // PE 20/上限 40 ⇒ 余量 0.5；PB 2/上限 6 ⇒ 余量 2/3×0.4=0.267；合计 0.5×0.6+0.267=0.567
        let p = value_evidence_strength(20.0, 2.0, 40.0, 6.0);
        assert!((p - (0.6 * 0.5 + 0.4 * (2.0 / 3.0))).abs() < 1e-9, "实得 {p}");
        // 越便宜证据越高（单调）
        assert!(value_evidence_strength(10.0, 2.0, 40.0, 6.0) > p);
        // pe<=0（亏损/无披露）⇒ 0 —— 价值策略不奖励亏损标的
        assert_eq!(value_evidence_strength(-1.0, 2.0, 40.0, 6.0), 0.0);
        assert_eq!(value_evidence_strength(0.0, 2.0, 40.0, 6.0), 0.0);
        // 上限 ≤0（配置错误）⇒ 0（fail-open）
        assert_eq!(value_evidence_strength(20.0, 2.0, 0.0, 6.0), 0.0);
        // 高于上限（不可能通过过滤门，防御性）⇒ PE 维度 clamp 到 0（不为负拖累），
        // PB 维度照常贡献：0.4 × (4/6)
        let p = value_evidence_strength(50.0, 2.0, 40.0, 6.0);
        assert!((p - 0.4 * (4.0 / 6.0)).abs() < 1e-9, "实得 {p}");
    }

    #[test]
    fn value_strategy_ids() {
        assert_eq!(ValueStrategy::ultra_short().id(), "value_ultra_short");
        assert_eq!(ValueStrategy::short().id(), "value_short");
        assert_eq!(ValueStrategy::mid().id(), "value_mid");
        assert_eq!(ValueStrategy::long().id(), "value_long");
        assert_eq!(ValueStrategy::long().style(), Style::Value);
    }
}
