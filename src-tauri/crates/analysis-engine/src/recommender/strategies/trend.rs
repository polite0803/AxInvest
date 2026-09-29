//! 趋势跟踪子策略：MA 多头 + 突破 + 量能
//!
//! ## 参数简化（P1-7）
//! 原 ~52 个 per-period read_f64 变量→硬编码周期差异 + 共享乘数。
//! 可配置参数保留 ~10 个，见下方 TREND_VARS 文档。

use super::super::strategy::{read_f64, RecoContext, RecommendStrategy};
use crate::recommender::indicators;
use crate::recommender::scoring::{calc_confidence, calc_position_with_consistency};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_harness::market_data::MarketDataProvider;
use serde_json::Value;
use std::collections::HashMap;

// ── 可保留的用户可配参数（~10 个） ──
// 以下变量名仍通过 read_f64(vars, ...) 读取，默认值在此定义。
// 删除的 per-period 变量（如 trend_ultra_short_entry_low）默认 fallback 到硬编码。

const DEFAULT_AMOUNT_RATIO_MIN: f64 = 0.8;
const DEFAULT_ENTRY_TIGHTNESS: f64 = 1.0; // 入场范围乘数（1.0=标准，1.5=更宽松）
const DEFAULT_STOP_MULT: f64 = 1.0; // 止损乘数（1.0=标准，0.8=更紧）
const DEFAULT_TARGET_MULT: f64 = 1.0; // 目标乘数（1.0=标准，1.2=更激进）
const DEFAULT_POS_ADJ: f64 = 1.0; // 仓位调整（1.0=标准，0.5=半仓）

/// 尺度窗口组（单位=**该档尺度的 bar**，不是日历日；由 `recommender::scale` 决定尺度）。
///
/// 为什么必须自带一组而不是「日线窗口换个条数」：`fast/slow/anchor` 要覆盖该档的决策视野 ——
/// 超短看几小时到两天，长线看数季到两年。换算依据写在各档注释里（1 小时线=0.25 交易日、
/// 1 周线=5 日、1 季线=60 日）。
#[derive(Clone, Copy)]
struct Windows {
    fast: usize,
    slow: usize,
    anchor: usize,
    high_window: usize,
    amount_window: usize,
    /// 是否要求突破近期高点（长线看的是「回踩未破」，不要求创新高）
    require_high_breakout: bool,
    /// 是否加 MACD 方向条件（仅中线原形态有此条件，不擅自扩到别的档）
    require_macd: bool,
}

fn windows_for(p: Period) -> Windows {
    match p {
        // 小时线：8 根=2 日、16 根=4 日、40 根=10 日；高点看 2 日
        Period::UltraShort => Windows {
            fast: 8,
            slow: 16,
            anchor: 40,
            high_window: 8,
            amount_window: 20,
            require_high_breakout: true,
            require_macd: false,
        },
        // 日线：原形态 5/10/20
        Period::Short => Windows {
            fast: 5,
            slow: 10,
            anchor: 20,
            high_window: 20,
            amount_window: 20,
            require_high_breakout: true,
            require_macd: false,
        },
        // 周线：4 周=20 日、8 周=40 日、13 周=一季度；高点看 8 周
        Period::Mid => Windows {
            fast: 4,
            slow: 8,
            anchor: 13,
            high_window: 8,
            amount_window: 13,
            require_high_breakout: true,
            require_macd: true,
        },
        // 季线：3 季=9 月、6 季=18 月、8 季=2 年；长档不要求创新高（原形态是「回踩未破 MA60」）
        Period::Long => Windows {
            fast: 3,
            slow: 6,
            anchor: 8,
            high_window: 4,
            amount_window: 8,
            require_high_breakout: false,
            require_macd: false,
        },
    }
}

/// 按档位返回硬编码的 (entry_low, entry_high, stop_loss, target, base_pos)。
///
/// ⚠ 入场/止损/目标仍是**百分比带宽**：尺度归一由 Phase R-D（波动率风控）处理，本阶段不混做。
#[inline]
fn price_bands(p: Period) -> (f64, f64, f64, f64, f64) {
    match p {
        Period::UltraShort => (0.995, 1.005, 0.98, 1.05, 3.0),
        Period::Short => (0.99, 1.015, 0.95, 1.10, 5.0),
        Period::Mid => (0.97, 1.05, 0.92, 1.20, 8.0),
        Period::Long => (0.95, 1.03, 0.85, 1.30, 10.0),
    }
}

/// 日线口径的均线容差基准（「收盘价不低于 MA × 该比例」）。用户可覆盖，覆盖值同样按尺度归一。
const DAILY_MA_TOLERANCE: f64 = 0.99;
/// 日线口径的高点容差基准（「突破 N 日高 × 该比例」）。
const DAILY_HIGH_TOLERANCE: f64 = 0.97;

pub struct TrendStrategy {
    pub period: Period,
}

impl TrendStrategy {
    pub const fn ultra_short() -> Self {
        Self { period: Period::UltraShort }
    }
    pub const fn short() -> Self {
        Self { period: Period::Short }
    }
    pub const fn mid() -> Self {
        Self { period: Period::Mid }
    }
    pub const fn long() -> Self {
        Self { period: Period::Long }
    }

    async fn scan_one(
        &self,
        client: &dyn MarketDataProvider,
        code: &str,
        name: &str,
        sector: Option<String>,
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        let (el, eh, sl, tg, bp) = price_bands(self.period);
        let w = windows_for(self.period);

        // 尺度由档位决定（超短=60 分钟、短=日线、中=周线、长=季度）。取不到该尺度、
        // 或聚合后 bar 数不足 ⇒ **该风格在该档不出票**，绝不退回日线冒充（门 f 锁这一点）。
        let (klines, profile) =
            match crate::recommender::scale::fetch(client, code, self.period).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("[trend] {code} 档位 {:?} 取数失败: {e}", self.period);
                    return None;
                },
            };
        if !crate::recommender::scale::enough_bars(&klines, &profile) {
            return None;
        }

        let cs = indicators::closes(&klines);
        let last = *cs.last()?;

        // 量比
        let avg_amt = indicators::avg_amount_n(&klines, w.amount_window).unwrap_or(0.0);
        let today_amount = klines.last().map(|k| k.amount).unwrap_or(0.0);
        let turnover_anomaly = if avg_amt > 0.0 {
            today_amount / avg_amt
        } else {
            1.0
        };
        let amount_ratio = turnover_anomaly;

        // ── 共享乘数 ──
        let entry_tightness = read_f64(vars, "trend_entry_tightness", DEFAULT_ENTRY_TIGHTNESS);
        let stop_mult = read_f64(vars, "trend_stop_mult", DEFAULT_STOP_MULT);
        let target_mult = read_f64(vars, "trend_target_mult", DEFAULT_TARGET_MULT);
        let pos_adj = read_f64(vars, "trend_position_adj", DEFAULT_POS_ADJ);

        // 容差按尺度归一：`trend_ma_tolerance_daily` / `trend_high_tolerance_daily` 是
        // **日线口径**基准（用户可覆盖，覆盖值同样按尺度换算），放到每根覆盖 d 个交易日的
        // bar 上按 √d 放宽/收紧 —— 消灭「每张策略各自手抄一档阈值」的形态（诊断 E3②）。
        let ma_tol = crate::recommender::scale::dev_tolerance(
            read_f64(vars, "trend_ma_tolerance_daily", DAILY_MA_TOLERANCE),
            &profile,
        );
        let high_tol = crate::recommender::scale::dev_tolerance(
            read_f64(vars, "trend_high_tolerance_daily", DAILY_HIGH_TOLERANCE),
            &profile,
        );
        let amt_min = read_f64(vars, "trend_amount_ratio_min", DEFAULT_AMOUNT_RATIO_MIN);

        let fast = indicators::sma(&cs, w.fast)?;
        let slow = indicators::sma(&cs, w.slow)?;
        let anchor = indicators::sma(&cs, w.anchor)?;
        if fast <= slow {
            return None;
        }
        if last < anchor * ma_tol {
            return None;
        }
        if amount_ratio < amt_min {
            return None;
        }
        let scale_name = crate::recommender::scale::scale_name(self.period);
        let mut reasons: Vec<String> = vec![
            format!("MA{} {:.2} > MA{} {:.2}（尺度 {}）", w.fast, fast, w.slow, slow, scale_name),
            format!("站上 MA{} {:.2}", w.anchor, anchor),
        ];
        if w.require_high_breakout {
            let high_n = indicators::highest(&klines, w.high_window)?;
            if last < high_n * high_tol {
                return None;
            }
            reasons.push(format!("接近 {} 根 bar 高点 {:.2}", w.high_window, high_n));
        }
        if w.require_macd {
            let (dif, dea, macd_bar) = indicators::macd(&klines, 12, 26, 9)?;
            if dif <= dea {
                return None;
            }
            reasons.push(format!("MACD 红柱 {:.2}", macd_bar));
        }
        reasons.push(format!("量比 {:.2}", amount_ratio));
        let price_ref = fast;

        // 应用共享乘数到硬编码默认值
        let entry_low = price_ref * (1.0 - (1.0 - el) * entry_tightness);
        let entry_high = price_ref * (1.0 + (eh - 1.0) * entry_tightness);
        let stop_loss = price_ref * (1.0 - (1.0 - sl) * stop_mult);
        let target_price = price_ref * (1.0 + (tg - 1.0) * target_mult);
        let base_position = bp * pos_adj;

        // 置信度
        let conf_consistency = read_f64(vars, "trend_conf_consistency", 0.85);
        let conf_signal = read_f64(vars, "trend_conf_signal", 0.7);
        let conf_market = read_f64(vars, "trend_conf_market", 0.0);
        let conf = calc_confidence(
            conf_consistency,
            conf_signal,
            if amount_ratio > 1.5 { 0.8 } else { 0.5 },
            conf_market,
            turnover_anomaly,
        );
        let position = calc_position_with_consistency(base_position, conf, conf_consistency);

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
            risk_notes: vec!["个股回调 / 跌破短期均线风险".to_string()],
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
impl RecommendStrategy for TrendStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::UltraShort => "trend_ultra_short",
            Period::Short => "trend_short",
            Period::Mid => "trend_mid",
            Period::Long => "trend_long",
        }
    }
    fn style(&self) -> Style {
        Style::Trend
    }
    fn period(&self) -> Period {
        self.period
    }
    fn required_vendors(&self) -> &'static [&'static str] {
        &["eastmoney", "tencent", "ths", "akshare"]
    }

    async fn scan(&self, ctx: &RecoContext<'_>) -> Result<Vec<RecoPick>, String> {
        // 获取行业排名数据，用于行业动量过滤
        let sector_momentum: HashMap<String, f64> = ctx
            .client
            .get_industry_ranking()
            .await
            .map(|industries| {
                industries
                    .iter()
                    .take(20)
                    .enumerate()
                    .map(|(i, ind)| {
                        let score = (20.0 - i as f64) / 20.0 * 100.0; // 第1名100分，第20名5分
                        (ind.industry_name.clone(), score)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut picks = Vec::new();
        for (code, name, sector) in ctx.seed {
            let _g = ctx.per_code_locks.lock_for(code).await;
            // 根据股票所属行业查找该行业的动量分
            let sector_mom = sector
                .as_ref()
                .and_then(|s| {
                    // 尝试完全匹配，再尝试前缀匹配
                    sector_momentum.get(s).copied().or_else(|| {
                        sector_momentum
                            .iter()
                            .find(|(k, _)| s.contains(k.as_str()) || k.contains(s))
                            .map(|(_, v)| *v)
                    })
                })
                .unwrap_or(50.0);
            // 注入行业动量到 vars，scan_one 可通过 "sector_momentum" 读取
            let mut enriched_vars = ctx.vars.clone();
            enriched_vars.insert("sector_momentum".to_string(), serde_json::json!(sector_mom));

            if let Some(mut p) =
                self.scan_one(ctx.client, code, name, sector.clone(), &enriched_vars).await
            {
                // 行业动量修正：低于40分的行业扣10%置信度，高于80分的加10%
                if sector_mom < 40.0 {
                    p.confidence = ((p.confidence as f64 * 0.9) as u8).max(1);
                    p.reasons.push(format!("行业动量偏低({:.0}分)，置信度下调10%", sector_mom));
                } else if sector_mom > 80.0 {
                    p.confidence = ((p.confidence as f64 * 1.1).min(100.0)) as u8;
                    p.reasons.push(format!("行业动量强劲({:.0}分)，置信度上调10%", sector_mom));
                }
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
