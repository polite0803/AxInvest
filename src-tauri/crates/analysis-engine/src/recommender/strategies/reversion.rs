//! 超跌反弹子策略：RSI 超卖 / RSI 底背离 / 看涨 K 线形态 / 缩量回踩
//!
//! v1 不做长线（设计文档 §2.3）

use super::super::strategy::{read_f64, RecoContext, RecommendStrategy};
use crate::candlestick_pattern;
use crate::divergence;
use crate::recommender::indicators;
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_harness::market_data::MarketDataProvider;
use serde_json::Value;
use std::collections::HashMap;

/// 短档量能门：当日量能不得超过近 `n` 日均量的 `mult` 倍。返回 `(当日, 均量, 是否用成交量)`。
///
/// ⚠ **必须带成交量回退**（2026-10-20）：日 K 源（腾讯 `day`）只有 6 个字段、没有成交额
/// ⇒ `amount = 0`；旧实现读到 `avg_5 <= 0` 就 `return None`，于是**短档超跌反弹在回放里
/// 结构性死亡** —— 现网读数：回放 `short × reversion` 历史 **0 行**，实盘 63 行。
/// 判据本身只比较「当日 / 均量」的**比值**，两侧取同一个字段即可，不涉及单位换算；
/// `amount` 与 `volume` 都不可用（均量 ≤0）或当日超过倍数 ⇒ `None`。
fn short_energy_gate(
    klines: &[axagent_harness::market_data::KLine],
    n: usize,
    mult: f64,
) -> Option<(f64, f64, bool)> {
    let today_amount = klines.last().map(|k| k.amount).unwrap_or(0.0);
    let avg_amount = indicators::avg_amount_n(klines, n).unwrap_or(0.0);
    let (today, avg, used_volume) = if today_amount > 0.0 && avg_amount > 0.0 {
        (today_amount, avg_amount, false)
    } else {
        let today_volume = klines.last().map(|k| k.volume).unwrap_or(0.0);
        let avg_volume = indicators::avg_volume_n(klines, n).unwrap_or(0.0);
        (today_volume, avg_volume, true)
    };
    if !avg.is_finite() || avg <= 0.0 || !today.is_finite() || today > avg * mult {
        return None;
    }
    Some((today, avg, used_volume))
}

pub struct ReversionStrategy {
    pub period: Period,
}

impl ReversionStrategy {
    pub const fn ultra_short() -> Self {
        Self { period: Period::UltraShort }
    }
    pub const fn short() -> Self {
        Self { period: Period::Short }
    }
    pub const fn mid() -> Self {
        Self { period: Period::Mid }
    }

    async fn scan_one(
        &self,
        client: &dyn MarketDataProvider,
        code: &str,
        name: &str,
        sector: Option<String>,
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        // 尺度由档位决定；取数根数与出分所需最少 bar 数都来自 `ScaleProfile`
        //（旧形态自带 `rev_kline_limit=250` / `rev_min_kline_len=30` 一份日线口径的数，
        //  周线/季线上毫无对应含义 ⇒ 两个变量随之从种子表与面板摘除）。
        let (klines, profile) =
            match crate::recommender::scale::fetch(client, code, self.period).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("[reversion] {code} 档位 {:?} 取数失败: {e}", self.period);
                    return None;
                },
            };
        if !crate::recommender::scale::enough_bars(&klines, self.period) {
            return None;
        }
        let price = klines.last()?.close;

        // ── 新增：底背离检测 + 看涨 K 线形态检测 ──
        // 背离回看窗：日历 14 日 → 当前尺度的 bar 数（下限 5 根，否则粗尺度上变成「拿 1 根比高低」）
        // ⚠ 这里的两个键名原先带 `_days` 后缀（`rev_divergence_lookback_days` / `rev_rsi_period_days`），
        //   而 `seed_variables.rs` 声明的是 `rev_divergence_lookback`(14) / `rev_rsi_period`(6) ⇒
        //   读取侧恒不命中、永远走兜底，面板上那两格改了没有任何东西读（兜底值恰好等于默认值，所以从不现形）。
        //   判据落在 `scripts/check-variable-consumer-registry.mjs`：声明名与读取名必须逐字一致。
        let lookback = (crate::recommender::scale::bars_for_daily_span(
            read_f64(vars, "rev_divergence_lookback", 14.0) as usize,
            &profile,
        ))
        .max(5);
        let min_divergence_strength = read_f64(vars, "rev_min_divergence_strength", 0.3);
        let divergence_result = divergence::detect_all_divergences(&klines, 14, lookback);
        let has_bullish_divergence = divergence_result.iter().any(|d| {
            d.divergence_type == "regular_bullish" && d.strength >= min_divergence_strength
        });
        let best_divergence =
            divergence_result.into_iter().find(|d| d.divergence_type == "regular_bullish");

        let patterns = candlestick_pattern::detect_all_patterns(&klines);
        let has_bullish_pattern =
            patterns.iter().any(|p| p.direction == "看涨" && p.confidence >= 0.5);
        let best_bullish_pattern =
            patterns.into_iter().find(|p| p.direction == "看涨" && p.confidence >= 0.5);

        let divergence_bonus = if has_bullish_divergence {
            read_f64(vars, "rev_divergence_bonus", 0.10)
        } else {
            0.0
        };
        let pattern_bonus = if has_bullish_pattern {
            read_f64(vars, "rev_pattern_bonus", 0.05)
        } else {
            0.0
        };
        // ── 新增结束 ──

        let rsi_period = (crate::recommender::scale::bars_for_daily_span(
            read_f64(vars, "rev_rsi_period", 6.0) as usize,
            &profile,
        ))
        .max(5);
        let rsi_value = indicators::rsi(&klines, rsi_period)?;

        let (pass, reasons) = match self.period {
            Period::UltraShort => return None, // 超短线不适用超跌反弹
            Period::Short => {
                // 底背离 → 放宽 RSI 阈值从 35 至 40，提前捕捉反转
                let rsi_short_max = if has_bullish_divergence {
                    read_f64(vars, "rev_rsi_short_max_divergence", 40.0)
                } else {
                    read_f64(vars, "rev_rsi_short_max", 35.0)
                };
                if rsi_value >= rsi_short_max {
                    return None;
                }
                let avg_period = read_f64(vars, "rev_avg_amount_period", 5.0) as usize;
                let avg_mult = read_f64(vars, "rev_avg_amount_mult", 1.2);
                // 量能门走 `short_energy_gate`：成交额缺失时退回成交量（日 K 无 amount 字段）。
                let (today, avg, used_volume) = short_energy_gate(&klines, avg_period, avg_mult)?;
                let mut r = vec![
                    format!("RSI({}) {:.1} 超卖", rsi_period, rsi_value),
                    format!(
                        "{} {} 日均 {:.0}%",
                        if used_volume {
                            "量比"
                        } else {
                            "成交额比"
                        },
                        avg_period,
                        today / avg * 100.0
                    ),
                ];
                // 追加背离/形态理由
                if let Some(d) = &best_divergence {
                    r.push(format!(
                        "{}（强度 {:.0}%）",
                        d.description.split('，').next().unwrap_or(&d.description),
                        d.strength * 100.0
                    ));
                }
                if let Some(p) = &best_bullish_pattern {
                    r.push(format!("出现{}形态", p.pattern));
                }
                (true, r)
            },
            Period::Mid => {
                let dd_period = read_f64(vars, "rev_dd_period", 250.0) as usize;
                let dd_min = read_f64(vars, "rev_dd_min", 20.0);
                let dd = indicators::drawdown_from_high(&klines, dd_period).unwrap_or(0.0);
                if dd < dd_min {
                    return None;
                }
                let rsi_mid_period = read_f64(vars, "rev_rsi_mid_period", 30.0) as usize;
                let rsi_mid_max = if has_bullish_divergence {
                    // 底背离放宽月线 RSI 阈值从 50 至 55
                    read_f64(vars, "rev_rsi_mid_max_divergence", 55.0)
                } else {
                    read_f64(vars, "rev_rsi_mid_max", 50.0)
                };
                let rsi_30 = indicators::rsi(&klines, rsi_mid_period).unwrap_or(rsi_mid_max);
                if rsi_30 > rsi_mid_max {
                    return None;
                }
                let mut r = vec![
                    format!("距 {} 日高回撤 {:.0}%", dd_period, dd),
                    format!("月线 RSI {:.1}", rsi_30),
                ];
                if let Some(d) = &best_divergence {
                    r.push(format!(
                        "{}（强度 {:.0}%）",
                        d.description.split('，').next().unwrap_or(&d.description),
                        d.strength * 100.0
                    ));
                }
                if let Some(p) = &best_bullish_pattern {
                    r.push(format!("出现{}形态", p.pattern));
                }
                (true, r)
            },
            Period::Long => return None, // v1 不做长线超跌
        };

        if !pass {
            return None;
        }

        let (entry_low, entry_high, stop_loss, target, base_position) = match self.period {
            Period::UltraShort => return None,
            Period::Short => {
                let el = read_f64(vars, "rev_short_entry_low", 0.97);
                let eh = read_f64(vars, "rev_short_entry_high", 1.03);
                let sl = read_f64(vars, "rev_short_stop", 0.93);
                let tg = read_f64(vars, "rev_short_target", 1.08);
                let bp = read_f64(vars, "rev_short_base_pos", 3.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Mid => {
                let el = read_f64(vars, "rev_mid_entry_low", 0.95);
                let eh = read_f64(vars, "rev_mid_entry_high", 1.05);
                let sl = read_f64(vars, "rev_mid_stop", 0.88);
                let tg = read_f64(vars, "rev_mid_target", 1.20);
                let bp = read_f64(vars, "rev_mid_base_pos", 5.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Long => return None,
        };

        // 底背离 + 看涨形态 → 提升 signal_strength 系数
        let signal_strength =
            (read_f64(vars, "rev_conf_signal", 0.8) + divergence_bonus + pattern_bonus).min(0.99);
        let conf = calc_confidence(
            read_f64(vars, "rev_conf_consistency", 0.7),
            signal_strength,
            read_f64(vars, "rev_conf_direction", 0.6),
            read_f64(vars, "rev_conf_market", 0.0),
            read_f64(vars, "rev_conf_base", 1.0),
        );
        let position = calc_position(base_position, conf);

        // 底背离 → 在 risk_notes 中减弱"抄底过早"警告
        let mut risk_notes = vec!["下跌趋势未尽 / 抄底过早".to_string()];
        if has_bullish_divergence {
            risk_notes.push("RSI 底背离出现，下跌动能衰减".to_string());
        }

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
            risk_notes,
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
impl RecommendStrategy for ReversionStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::UltraShort => "rev_ultra_short",
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
    fn reversion_strategy_ids() {
        assert_eq!(ReversionStrategy::ultra_short().id(), "rev_ultra_short");
        assert_eq!(ReversionStrategy::short().id(), "rev_short");
        assert_eq!(ReversionStrategy::mid().id(), "rev_mid");
    }
}

#[cfg(test)]
mod energy_gate_tests {
    use super::*;
    use axagent_harness::market_data::KLine;

    fn bar(amount: f64, volume: f64) -> KLine {
        KLine {
            date: "2026-09-28".into(),
            open: 10.0,
            high: 10.0,
            low: 10.0,
            close: 10.0,
            volume,
            amount,
            turnover_rate: None,
            adj_factor: None,
        }
    }

    /// 成交额可用时优先用成交额（与旧行为一致）。
    /// 均量按**含当日**的最近 5 根计算：4×1000 + 500 ⇒ 均 900。
    #[test]
    fn amount_wins_when_present() {
        let ks = vec![
            bar(1000.0, 5.0),
            bar(1000.0, 5.0),
            bar(1000.0, 5.0),
            bar(1000.0, 5.0),
            bar(500.0, 999.0),
        ];
        assert_eq!(short_energy_gate(&ks, 5, 1.2), Some((500.0, 900.0, false)));
    }

    /// **回归判据**：日 K 无成交额（腾讯日线 6 字段 ⇒ amount 全 0）时必须退回成交量。
    /// 旧实现 `avg_5 <= 0 ⇒ return None` 让短档超跌反弹在回放里恒不出票（历史 0 行）。
    /// 均量同样含当日：4×100 + 60 ⇒ 均 92。
    #[test]
    fn falls_back_to_volume_when_amount_is_missing() {
        let ks = vec![
            bar(0.0, 100.0),
            bar(0.0, 100.0),
            bar(0.0, 100.0),
            bar(0.0, 100.0),
            bar(0.0, 60.0),
        ];
        assert_eq!(
            short_energy_gate(&ks, 5, 1.2),
            Some((60.0, 92.0, true)),
            "成交额缺失时要用成交量算出同一个比值，而不是直接判死"
        );
    }

    /// 负控 A：当日量能超过均量的 mult 倍 ⇒ 放量下跌不接（None）。
    #[test]
    fn rejects_volume_spike() {
        let ks = vec![
            bar(0.0, 100.0),
            bar(0.0, 100.0),
            bar(0.0, 100.0),
            bar(0.0, 100.0),
            bar(0.0, 200.0),
        ];
        assert_eq!(short_energy_gate(&ks, 5, 1.2), None);
    }

    /// 负控 B：额与量都取不到（全 0）⇒ None，不得拿 0/0 当成「缩量」。
    #[test]
    fn rejects_when_no_energy_field_is_usable() {
        let ks = vec![bar(0.0, 0.0), bar(0.0, 0.0), bar(0.0, 0.0)];
        assert_eq!(short_energy_gate(&ks, 3, 1.2), None);
    }

    /// 负控 C：量能是 NaN ⇒ 必须拒绝 —— `avg <= 0.0` 对 NaN 为假，单靠它兜不住
    /// （`today > avg*mult` 对 NaN 也恒假 ⇒ 会把 NaN 当「缩量」放过去）。
    #[test]
    fn rejects_non_finite_energy() {
        let ks = vec![bar(0.0, f64::NAN), bar(0.0, f64::NAN), bar(0.0, f64::NAN)];
        assert_eq!(short_energy_gate(&ks, 3, 1.2), None);
        let ks_last_nan = vec![bar(0.0, 100.0), bar(0.0, 100.0), bar(0.0, f64::NAN)];
        assert_eq!(short_energy_gate(&ks_last_nan, 3, 1.2), None);
    }
}
