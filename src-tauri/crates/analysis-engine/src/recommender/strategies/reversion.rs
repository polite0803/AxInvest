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
        //
        // ⚠ R-3e（2026-10-26）：`required_daily_bars` 是**本档指标窗口的日口径需求**，
        // 换算到当前尺度后与窗口计划的 `fetch_limit` 取大者。
        //   - Mid 档：`rev_dd_period=250`（天）⇒ weekly 下需 50 根，而 `fetch_limit` 只有 36 根
        //     ⇒ 旧实现永远在「不够的窗口」上算回撤；
        //   - Short 档：只需 `rev_divergence_lookback=14`（天）⇒ daily 下换算得 14 根
        //     < `fetch_limit=30` ⇒ **取数根数逐字不变**（零回归）。
        // 所以**必须按档位给不同需求** —— 若 Short 也报 250，daily 档会从 30 根变 250 根，
        // `detect_all_patterns` 会在更多 K 线上跑 ⇒ short 路径的结果会变。
        let required_daily_bars = match self.period {
            // 中线：回撤回看 250 天（`rev_dd_period`）+ RSI 30 天，取大者
            Period::Mid => 250,
            // 超短线：`reversion` 在该档直接 `return None`（超跌反弹不适用超短档）
            // ⇒ 不需要任何指标窗口 ⇒ 声明 0（否则 hourly 尺度 14 天 = 56 根 > fetch_limit 48 根，
            // 会无谓地改取数根数 —— 判据 `strategy_declared_requirements_...` 会拦下）
            Period::UltraShort => 0,
            // 其余档：背离 14 天 + RSI 6 天 + 均量 5 天，取大者
            _ => 14,
        };
        let (klines, profile) =
            match crate::recommender::scale::fetch(client, code, self.period, required_daily_bars)
                .await
            {
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
        // ── R-3f（2026-10-26）：背离检测的 RSI 平滑窗口也必须走尺度换算 ──
        // `detect_all_divergences(klines, rsi_period, lookback)` 的两个参数**都在同一
        // bar 序列上取下标**：`rsi_period` 是 RSI 平滑窗（`compute_rsi_series`），
        // `lookback` 是价格/RSI 的比较窗长度 ⇒ **两者必须同量纲**。
        // 原实现把 `rsi_period` 硬编码成 **14 根 bar**（未换算），而 `lookback` 已换算：
        //   daily 尺度  ⇒ rsi_period=14 根日线、lookback=14 根日线 ⇒ 恰好同量纲（无碍）
        //   weekly 尺度 ⇒ rsi_period=**14 根周线**（=98 日）、lookback=3 根周线（=14 日）
        //                  ⇒ **不同量纲**（平滑窗比比较窗长 4.7 倍）
        //   monthly尺度 ⇒ rsi_period=**14 根月线**（=14 月）、lookback=1→5 根月线
        //                  ⇒ 更离谱（比较窗被 `.max(5)` 兜到 5，平滑窗 14）
        // 修法：与 `lookback` 同源换算（输入同为「日历 14 日」）。
        // **daily 尺度恒等**（`per_bar=1` ⇒ ceil(14/1)=14）⇒ **零回归**；
        // 粗尺度上从「14 根周线」修正为「3 根周线」，与比较窗同量纲。
        // ⚠ 下限同样 `.max(5)`：否则粗尺度上平滑窗(3) < 比较窗(5) ⇒
        //   `detect_rsi_divergence` 的 `rsi_vals.len() < lookback * 2` 判据会误判「数据不足」。
        let divergence_rsi_period =
            (crate::recommender::scale::bars_for_daily_span(14, &profile)).max(5);
        let divergence_result =
            divergence::detect_all_divergences(&klines, divergence_rsi_period, lookback);
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
                // ⚠ R-3e（2026-10-26）：本参数的口径是**天数**（seed 描述「均量计算天数」），
                // 而此处直接当 **bar 数**用 ⇒ 在 weekly/monthly 尺度下量纲错
                // （5 天 → 5 周 = 25 天）。daily 尺度恰好等价，所以缺陷被掩盖。
                // 走 `bars_for_daily_span` 换算：**daily 尺度结果完全不变**（d=1 ⇒ 恒等），
                // 只修好 weekly/monthly 两档 ⇒ 零回归。
                let avg_period = crate::recommender::scale::bars_for_daily_span(
                    read_f64(vars, "rev_avg_amount_period", 5.0) as usize,
                    &profile,
                )
                .max(2);
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
                // ⚠ R-3e（2026-10-26）：口径是**天数**（seed 描述「回撤回看天数」/「RSI 计算周期」），
                // 直接当 bar 数用 ⇒ weekly 尺度下 250 天被读成 250 周（1250 天）、
                // 30 天被读成 30 周（150 天）。
                //
                // ⚠⚠ **实测纠正**：原注释写「该档恒 return None」——**不成立**。
                // 实盘 mid 档 reversion 有 79 条真票、avg_conf 59.3（高于 short 档的 55.7）
                // ⇒ 信号不弱，只是**窗口偏短**（`scale::fetch` 原先只给 36 根周线
                // = 180 天，而 `rev_dd_period` 想要 250 天）。
                //
                // 本次修复是**语义正确化**（180 天 → 250 天）+ 取数根数对齐
                // （见 `scale::fetch` 的 `required_daily_bars`），
                // **不等于**「mid 档在回放下会出票」——那仍待实跑确认。
                let dd_period = crate::recommender::scale::bars_for_daily_span(
                    read_f64(vars, "rev_dd_period", 250.0) as usize,
                    &profile,
                )
                .max(2);
                let dd_min = read_f64(vars, "rev_dd_min", 20.0);
                let dd = indicators::drawdown_from_high(&klines, dd_period).unwrap_or(0.0);
                if dd < dd_min {
                    return None;
                }
                let rsi_mid_period = crate::recommender::scale::bars_for_daily_span(
                    read_f64(vars, "rev_rsi_mid_period", 30.0) as usize,
                    &profile,
                )
                .max(2);
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
