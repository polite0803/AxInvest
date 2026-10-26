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
use axagent_harness::market_data::{KLine, MarketDataProvider};
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

/// trend 个股证据强度（0~1，2026-10-18 四档常数塌缩修复）。
///
/// 均线多头间距 40% + 站上 anchor 幅度 30% + 量比强度 30%。
/// 归一化刻度（**非过滤阈值**，不拦票只调强度）：间距 >10% 满格、
/// 站上 >8% 满格、量比 3x 满格。负值/非有限输入按 0 处理（clamp 语义）。
// 纯评分函数：8 个标量就是它全部输入（无状态、无副作用），拆成结构体只会把
// 调用点写成 `Struct { .. }`，不增加任何可读性。阶段 1 把连续证据从 3 个加到 5 个后触到默认上限。
#[allow(clippy::too_many_arguments)]
fn trend_evidence_strength(
    fast: f64,
    slow: f64,
    last: f64,
    anchor: f64,
    amount_ratio: f64,
    atr_pct: f64,
    vol_percentile: f64,
    rs: f64,
) -> f64 {
    if slow <= 0.0 || anchor <= 0.0 || !fast.is_finite() || !last.is_finite() {
        return 0.0;
    }
    let ma_spread = ((fast - slow) / slow).clamp(0.0, 0.10) / 0.10;
    let anchor_lift = ((last / anchor) - 1.0).clamp(0.0, 0.08) / 0.08;
    // 量能证据（2026-10-19，阶段 1）：优先用「今日量在该档窗口内的分位」——
    // 它不依赖成交额字段（回放里源常不给额），且是连续量而不是「>1.5 就 0.8」的二值。
    // 分位取不到时退回原量比归一，保持旧行为可复现。
    let vol_evidence = if vol_percentile.is_finite() {
        vol_percentile.clamp(0.0, 1.0)
    } else if amount_ratio.is_finite() {
        (amount_ratio / 3.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // 波动率归一的斜率（2026-10-19，阶段 1）：同样的均线间距，波动越大越不可靠，
    // 以「1 个 ATR/bar 的斜率」为满格。这是让分数随个股波动拉开差距的第一处连续量。
    let slope_per_atr = if atr_pct.is_finite() && atr_pct > 0.0 {
        (((fast - slow) / slow) / atr_pct).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // 相对强度（2026-10-19，阶段 1）：调用方已把「个股收益 − 指数同尺度收益」映射到 0~1
    // （−20% ⇒ 0、0 ⇒ 0.5、+20% ⇒ 1）。NaN ⇒ 中性 0.5——不把「取不到指数」当成「跑输」。
    let rs_norm = if rs.is_finite() {
        rs.clamp(0.0, 1.0)
    } else {
        0.5
    };
    (0.25 * ma_spread
        + 0.15 * anchor_lift
        + 0.25 * vol_evidence
        + 0.20 * slope_per_atr
        + 0.15 * rs_norm)
        .clamp(0.0, 1.0)
}

/// 阶段 1 返工：证据（0~1）占 **50%**、原五项公式（u8）占 50%，合成最终置信度。
/// 同一 base 下 evidence 每 +0.1 ⇒ 置信度约 +5 分；NaN 证据按中性 0.5（不退化成 0 分）。
fn blend_evidence_confidence(evidence: f64, base: u8) -> u8 {
    let e = if evidence.is_finite() {
        evidence.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let blended = 0.5 * e * 100.0 + 0.5 * (base as f64).clamp(0.0, 100.0);
    blended.round().clamp(1.0, 100.0) as u8
}

/// 近 `n` 根 bar 的收益率（不足时用可用的全部；<2 根返回 NaN，由调用方给中性值）。
fn bars_return_pct(klines: &[KLine], n: usize) -> f64 {
    if klines.len() < 2 {
        return f64::NAN;
    }
    let last = klines[klines.len() - 1].close;
    let idx = klines.len() - 1 - n.min(klines.len() - 1);
    let base = klines[idx].close;
    if base > 0.0 && last.is_finite() {
        (last - base) / base
    } else {
        f64::NAN
    }
}

/// 尺度窗口组（单位=**该档尺度的 bar**，不是日历日）。
///
/// ## 2026-10-18：改为由窗口计划现推，不再手抄四档数字
///
/// 此前这里有一张手抄表（超短 8/16/40、短 5/10/20、中 4/8/13、长 3/6/8），它隐含的前提是
/// 「短=日线、中=周线、长=季线」——正是 `PLAN-horizon-scale-alignment.md` D3 那条：
/// 档位阶梯一改（长档 90 交易日 = 月线），手抄表立刻与权威脱节，而且**没有任何门能发现**
/// （每个数字在注释里都自洽）。
///
/// 现在的推导（锚在日线默认形状 5/10/20 的**比值**上，与 `ScaleWindowPlan` 的
/// 「长带 = 3 × 中带」同源）：
/// - `anchor = 长带`（3 × 中带，覆盖 3 倍持有期）
/// - `slow = 2/3 · anchor`、`fast = 1/3 · anchor`（下限 2 根，否则指标退化成常数）
/// - `high_window = amount_window = 中带`（最近一个持有期的高点与量能基线）
///
/// 四档读数（`cargo test -p axagent-astock-data horizon_window_plan_dump` 核对）：
/// 超短 8/16/24、短 5/10/15、中 6/12/18、长 5/10/15。
#[derive(Clone, Copy)]
struct Windows {
    fast: usize,
    slow: usize,
    anchor: usize,
    high_window: usize,
    amount_window: usize,
    /// 是否要求突破近期高点（长档看的是「回踩未破」，不要求创新高）
    require_high_breakout: bool,
    /// 是否加 MACD 方向条件（仅中线原形态有此条件，不擅自扩到别的档）
    require_macd: bool,
}

fn windows_for(p: Period) -> Windows {
    let plan = crate::recommender::scale::window_plan(p);
    let anchor = plan.long_bars.max(3);
    Windows {
        fast: (anchor / 3).max(2),
        slow: (anchor * 2 / 3).max(2),
        anchor,
        high_window: plan.short_bars,
        amount_window: plan.short_bars,
        require_high_breakout: !matches!(p, Period::Long),
        require_macd: matches!(p, Period::Mid),
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

        // 尺度由档位权威决定（`Period::scale_key`：超短=60 分钟、短=日线、中=周线、长=月线，
        // 定阶规则 `d ≤ h/3` 见 harness 的该函数文档）。取不到该尺度、或聚合后 bar 数不足
        // ⇒ **该风格在该档不出票**，绝不退回日线冒充（门 f 锁这一点）。
        let (klines, profile) =
            match crate::recommender::scale::fetch(client, code, self.period, 0).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("[trend] {code} 档位 {:?} 取数失败: {e}", self.period);
                    return None;
                },
            };
        if !crate::recommender::scale::enough_bars(&klines, self.period) {
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
        // ── 个股证据强度（2026-10-18，四档常数塌缩修复）──
        // 此前 signal 槽是**模板常数**、liquidity 槽是量比二值 ⇒ 同档所有票的原始
        // 置信几乎相同，经 logit 合成 + SNR 折算后塌成「档位常数」（60/56/53/51），
        // 置信度不携带任何个股信息。现把三个连续证据量归一成 evidence ∈ [0,1]：
        //   均线多头间距 40% + 站上 anchor 幅度 30% + 量比强度 30%
        // （间距 >10% 满格 / 站上 >8% 满格 / 量比 3x 满格——归一化刻度，非过滤阈值）。
        // signal 槽调制：`signal × (0.6 + 0.4×evidence)` —— 证据满格不变、零证据六折，
        // 满跨度 ≈ w.signal(0.35)×0.7×0.4 ≈ 10 个置信点，足以穿透档位常数。
        // 阶段 1（2026-10-19）：把两处**连续证据**从 K 线直接算出来——
        // ① ATR(14)/收盘：波动率归一的斜率基准；② 今日成交量在本档量能窗口内的分位。
        // 二者都只用 K 线自带的字段（不依赖成交额），回放里同样可得。
        let atr_pct = {
            let n = klines.len().min(14);
            let last_close = klines.last().map(|k| k.close).unwrap_or(0.0);
            if n == 0 || last_close <= 0.0 {
                0.0
            } else {
                let sum: f64 =
                    klines[klines.len() - n..].iter().map(|k| (k.high - k.low).abs()).sum();
                (sum / n as f64) / last_close
            }
        };
        let vol_percentile = {
            let n = klines.len().min(w.amount_window.max(1));
            if n == 0 {
                0.5
            } else {
                let today_vol = klines.last().map(|k| k.volume).unwrap_or(0.0);
                let le =
                    klines[klines.len() - n..].iter().filter(|k| k.volume <= today_vol).count();
                le as f64 / n as f64
            }
        };
        // 相对强度：个股 20 bar 收益 − 指数同尺度 20 bar 收益，映射到 0~1（±20% 截断）。
        let stock_ret = bars_return_pct(&klines, 20);
        let index_ret = read_f64(vars, "reco_index_ret_20", f64::NAN);
        let rs = if stock_ret.is_finite() && index_ret.is_finite() {
            ((stock_ret - index_ret).clamp(-0.2, 0.2) + 0.2) / 0.4
        } else {
            0.5
        };
        let evidence = trend_evidence_strength(
            fast,
            slow,
            last,
            anchor,
            amount_ratio,
            atr_pct,
            vol_percentile,
            rs,
        );
        // ── 证据做主项（2026-10-19 用户裁定，阶段 1 返工）──
        // 原实现让证据只调制 signal 一个槽（`conf_signal×(0.6+0.4×evidence)`），满跨度 ≈10 分，
        // 被其余三项常数淹没 ⇒ 实测置信度只有 8~9 个取值、与后涨相关性 −0.15。
        // 现在：证据直接占 **50%**，原五项公式占 50%——后者保留「一致性/流动性/动量」的约束，
        // 前者决定排序。这样同档票的置信度随证据连续拉开，面板上的 65/66/67 才有意义。
        let base_conf = calc_confidence(
            conf_consistency,
            conf_signal,
            if amount_ratio > 1.5 { 0.8 } else { 0.5 },
            conf_market,
            turnover_anomaly,
        );
        let conf = blend_evidence_confidence(evidence, base_conf);
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
            evidence_score: None,
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

        // 阶段 1（2026-10-19）：相对强度 = 个股 N-bar 收益 − 指数同尺度收益。
        // 指数与个股同走 `scale::fetch`，保证两边是同一尺度；取不到时记 NaN，
        // 证据项按中性 0.5 处理（不把「取不到指数」当成「跑输指数」）。
        let index_ret =
            match crate::recommender::scale::fetch(ctx.client, "000300", self.period, 0).await {
                Ok((ks, _)) => bars_return_pct(&ks, 20),
                Err(_) => f64::NAN,
            };

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
            enriched_vars.insert("reco_index_ret_20".to_string(), serde_json::json!(index_ret));

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

    #[test]
    fn trend_evidence_is_monotonic_in_each_input() {
        let base = trend_evidence_strength(10.2, 10.0, 10.1, 10.0, 1.0, 0.02, 0.5, 0.5);
        assert!(base > 0.0 && base < 1.0, "中性输入应落在 (0,1)，实得 {base}");
        // 均线间距 ↑ ⇒ 证据 ↑
        assert!(trend_evidence_strength(11.0, 10.0, 10.1, 10.0, 1.0, 0.02, 0.5, 0.5) > base);
        // 站上 anchor 幅度 ↑ ⇒ 证据 ↑
        assert!(trend_evidence_strength(10.2, 10.0, 10.5, 10.0, 1.0, 0.02, 0.5, 0.5) > base);
        // 量能分位 ↑ ⇒ 证据 ↑（阶段 1 起的连续量）
        assert!(trend_evidence_strength(10.2, 10.0, 10.1, 10.0, 1.0, 0.02, 0.9, 0.5) > base);
        // 波动率 ↑（同样的均线间距）⇒ 证据 ↓（波动率归一）
        assert!(trend_evidence_strength(10.2, 10.0, 10.1, 10.0, 1.0, 0.08, 0.5, 0.5) < base);
        // 相对强度 ↑（同形态、同波动、同量能）⇒ 证据 ↑（阶段 1 新增项）
        assert!(
            trend_evidence_strength(10.2, 10.0, 10.1, 10.0, 1.0, 0.02, 0.5, 0.9)
                > trend_evidence_strength(10.2, 10.0, 10.1, 10.0, 1.0, 0.02, 0.5, 0.3)
        );
    }

    /// 阶段 1 的核心验收：**相同均线形态、不同波动与量能，分数必须拉得开**。
    /// 旧实现里这两只票的 evidence 完全相同（只看间距/站上/量比），正是「置信度塌成档位常数」的来源。
    #[test]
    fn trend_evidence_separates_low_vol_high_volume_from_high_vol_low_volume() {
        let calm_strong = trend_evidence_strength(10.2, 10.0, 10.1, 10.0, 1.0, 0.015, 0.9, 0.5);
        let wild_weak = trend_evidence_strength(10.2, 10.0, 10.1, 10.0, 1.0, 0.06, 0.2, 0.5);
        assert!(
            calm_strong - wild_weak >= 0.15,
            "两只票的证据强度必须拉开 ≥0.15，实得 {calm_strong} vs {wild_weak}"
        );
    }

    #[test]
    fn trend_evidence_clamps_and_fails_open_on_degenerate_inputs() {
        // 满格：间距 ≥10% + 站上 ≥8% + 量能分位 1 + 低波动 ⇒ 1.0
        assert!(
            (trend_evidence_strength(11.0, 10.0, 10.9, 10.0, 3.0, 0.01, 1.0, 1.0) - 1.0).abs()
                < 1e-9
        );
        // 零证据：均线死叉 + 贴 anchor + 量能分位 0 + 完全跑输指数（rs=0）⇒ 0
        assert!(
            (trend_evidence_strength(10.0, 10.0, 10.0, 10.0, 0.0, 0.02, 0.0, 0.0) - 0.0).abs()
                < 1e-9
        );
        // 中性相对强度（取不到指数 ⇒ 0.5）自带 0.075 的地板：这是刻意的，
        // 「取不到指数」不能当成「跑输指数」，但也不该被当成满格证据。
        assert!(
            (trend_evidence_strength(10.0, 10.0, 10.0, 10.0, 0.0, 0.02, 0.0, 0.5) - 0.075).abs()
                < 1e-9
        );
        // 退化输入：slow/anchor ≤0 ⇒ 0（fail-open）
        assert_eq!(trend_evidence_strength(10.2, 0.0, 10.1, 10.0, 1.0, 0.02, 0.5, 0.5), 0.0);
        assert_eq!(trend_evidence_strength(10.2, 10.0, 10.1, 0.0, 1.0, 0.02, 0.5, 0.5), 0.0);
        // NaN 不得把分数污染成 NaN：分位不可用时退回量比归一，仍是有限值
        let nan_safe =
            trend_evidence_strength(10.2, 10.0, 10.1, 10.0, 1.0, f64::NAN, f64::NAN, 0.5);
        assert!(nan_safe.is_finite(), "NaN 输入必须给出有限分数，实得 {nan_safe}");
    }
}

#[cfg(test)]
mod evidence_blend_tests {
    use super::*;

    /// 用户裁定「证据做主项」的判据：同一 base 下，证据从 0.2 到 0.8 必须拉开 ≥20 分；
    /// 证据 NaN 时按中性 0.5（不得退化成 0 分）；结果恒在 1~100。
    #[test]
    fn evidence_is_the_dominant_term_of_confidence() {
        let lo = blend_evidence_confidence(0.2, 50);
        let hi = blend_evidence_confidence(0.8, 50);
        assert!(hi - lo >= 20, "证据主项的跨度不足：{lo} vs {hi}");
        assert!(lo >= 1 && hi <= 100);
        let nan = blend_evidence_confidence(f64::NAN, 50);
        assert_eq!(nan, 50, "NaN 证据应按中性 0.5 合成（0.5×50+0.5×50=50），实得 {nan}");
        assert!(
            blend_evidence_confidence(1.0, 100) <= 100 && blend_evidence_confidence(0.0, 0) >= 1
        );
    }
}
