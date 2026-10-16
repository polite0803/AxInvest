//! 市场状态研判 — 基于沪深300近60日K线输出牛/熊/震荡 + 波动率
//!
//! 无需额外数据源，纯用已有 `get_klines("000300", "daily", 60)` 数据判断。
//! 不引入新依赖，纯函数模块。

use crate::types::{MarketRegimeType, VolatilityLevel};
use axagent_astock_data::KLine;

/// 市场状态
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketRegime {
    pub regime: MarketRegimeType,
    /// 置信度 0-1
    pub confidence: f64,
    pub volatility: VolatilityLevel,
    /// 可读描述
    pub description: String,
}

/// 用沪深300近60日 K 线判断市场状态
///
/// 规则（纯技术面，仅用近60日数据，无需额外数据）:
/// - 价格站上 MA60 且 MA20 多头排列 + 向上斜率 → 牛市
/// - 价格跌破 MA60 且 MA20 空头排列 + 向下斜率 → 熊市
/// - 其余 → 震荡
/// - 布林带宽度（20日收盘价标准差 / MA20）> 20% → 高波动，< 10% → 低波动
///
/// 限制：仅取近60日K线，不计算 MA120/MA250，牛熊判定基于价格相对均线的位置与斜率。
pub fn classify_regime(klines: &[KLine]) -> MarketRegime {
    if klines.len() < 20 {
        return MarketRegime {
            regime: MarketRegimeType::Sideways,
            confidence: 0.3,
            volatility: VolatilityLevel::Low,
            description: format!("数据不足（仅{}日），默认震荡", klines.len()),
        };
    }

    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();

    // 均线计算
    let ma20 = simple_moving_average(&closes, 20);
    let ma60_is_real = closes.len() >= 60;
    let ma60 = if ma60_is_real {
        simple_moving_average(&closes, 60)
    } else {
        ma20
    };
    // len<60 时 ma60 实为 ma20，描述文案需如实反映，避免误标"60日均线"
    let ma60_label = if ma60_is_real {
        "60日均线"
    } else {
        "短期均线"
    };

    // 当前价格相对均线的位置
    let current_price = closes.last().copied().unwrap_or(0.0);
    let price_above_ma20 = if ma20 > 0.0 {
        (current_price - ma20) / ma20
    } else {
        0.0
    };
    let price_above_ma60 = if ma60 > 0.0 {
        (current_price - ma60) / ma60
    } else {
        0.0
    };

    // 最近 N 日斜率（用线性回归简化：最后5日 vs 之前5日）
    let slope = if closes.len() >= 10 {
        let recent_5: f64 = closes[closes.len() - 5..].iter().sum::<f64>() / 5.0;
        let prev_5: f64 = closes[closes.len() - 10..closes.len() - 5].iter().sum::<f64>() / 5.0;
        (recent_5 - prev_5) / prev_5
    } else {
        0.0
    };

    // 布林带宽度（波动率）：20日收盘价标准差 / ma20
    let (bollinger_pct, vol_level) = if closes.len() >= 20 {
        let variance =
            closes[closes.len() - 20..].iter().map(|c| (c - ma20).powi(2)).sum::<f64>() / 20.0;
        let std_dev = variance.sqrt();
        let bbp = if ma20 > 0.0 { std_dev / ma20 } else { 0.0 };
        let vl = VolatilityLevel::from_bollinger_pct(bbp);
        (bbp, vl)
    } else {
        (0.0, VolatilityLevel::Normal)
    };

    // 决策逻辑
    let (regime, confidence, desc) =
        if price_above_ma60 > 0.05 && price_above_ma20 > 0.02 && slope > 0.01 {
            // 价格在 MA60 上方 5% + MA20 上方 2% + 向上斜率
            let c = (price_above_ma60 * 2.0).clamp(0.5, 0.95);
            let vol_note = if bollinger_pct > 0.20 {
                "（高波动预警）"
            } else {
                ""
            };
            (
                MarketRegimeType::Bull,
                c,
                format!(
                    "沪深300站上{ma60_label}{:.1}%，短期均线多头排列{}",
                    price_above_ma60 * 100.0,
                    vol_note
                ),
            )
        } else if price_above_ma60 < -0.03 && price_above_ma20 < -0.01 && slope < -0.005 {
            // 价格在 MA60 下方 3% + MA20 下方 + 向下斜率
            let c = (price_above_ma60.abs() * 2.0).clamp(0.5, 0.95);
            (
                MarketRegimeType::Bear,
                c,
                format!(
                    "沪深300跌破{ma60_label}{:.1}%，短期均线空头排列",
                    price_above_ma60.abs() * 100.0
                ),
            )
        } else {
            // 不满足牛/熊条件 → 震荡
            let c = 0.5 + (slope.abs() * 3.0).min(0.3); // 斜率越大信心越高
            (
                MarketRegimeType::Sideways,
                c.min(0.8),
                format!("均线交叉/粘合，方向不明确（斜率{:.2}%）", slope * 100.0),
            )
        };

    MarketRegime { regime, confidence, volatility: vol_level, description: desc }
}

/// 简单移动平均
fn simple_moving_average(data: &[f64], window: usize) -> f64 {
    if data.len() < window || window == 0 {
        return 0.0;
    }
    data[data.len() - window..].iter().sum::<f64>() / window as f64
}

/// 摆动极值的分形窗口半宽：拐点须比左右各 `PIVOT_WING` 根都高（低）。
const PIVOT_WING: usize = 2;
/// 波段结构回看根数（与均线体系同窗，避免两套窗口口径）。
const SWING_LOOKBACK: usize = 60;
/// 波段票的置信度：道氏结构是**弱信号**，只用于均线判震荡时补票。
const SWING_VOTE_CONFIDENCE: f64 = 0.6;
/// 只有一只指数有效时的降级乘数。
const SINGLE_VOTE_PENALTY: f64 = 0.7;
/// 计入票所需最少 K 线根数（与 `classify_regime` 的均线窗对齐）。
const VOTE_MIN_BARS: usize = 20;

/// 波段结构方向（道氏：相邻两组摆动高点与摆动低点是否同步移动）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwingDirection {
    Rising,
    Falling,
    Mixed,
}

/// 波段结构读数（最近两组摆动高/低点的比较）。
#[derive(Debug, Clone, Copy)]
pub struct SwingState {
    pub direction: SwingDirection,
    pub prior_high: f64,
    pub last_high: f64,
    pub prior_low: f64,
    pub last_low: f64,
    /// 最近一个摆动极值距序列末端的根数 —— 结构新鲜度，供描述层判读。
    pub pivot_age: usize,
}

/// 在近 `lookback` 根 K 线内按分形法找摆动高点/低点，比较最近两组是否同步移动。
///
/// 高点或低点各不足 2 个时返回 `None`：「结构识别不出」不等于「没有下移」，调用方
/// 必须把 `None` 与 `Mixed` 都算作**不加票**，否则缺席会被当成中性确认。
pub fn swing_structure(klines: &[KLine], lookback: usize) -> Option<SwingState> {
    let need = PIVOT_WING * 2 + 1;
    if klines.len() < need {
        return None;
    }
    let start = klines.len().saturating_sub(lookback.max(need));
    let ks = &klines[start..];
    if ks.len() < need {
        return None;
    }
    let mut highs: Vec<(usize, f64)> = Vec::new();
    let mut lows: Vec<(usize, f64)> = Vec::new();
    for i in PIVOT_WING..ks.len() - PIVOT_WING {
        let h = ks[i].high;
        if (1..=PIVOT_WING).all(|d| ks[i - d].high < h && ks[i + d].high < h) {
            highs.push((i, h));
        }
        let l = ks[i].low;
        if (1..=PIVOT_WING).all(|d| ks[i - d].low > l && ks[i + d].low > l) {
            lows.push((i, l));
        }
    }
    if highs.len() < 2 || lows.len() < 2 {
        return None;
    }
    let prior_high = highs[highs.len() - 2].1;
    let (last_high_i, last_high) = *highs.last()?;
    let prior_low = lows[lows.len() - 2].1;
    let (last_low_i, last_low) = *lows.last()?;
    let direction = if last_high < prior_high && last_low < prior_low {
        SwingDirection::Falling
    } else if last_high > prior_high && last_low > prior_low {
        SwingDirection::Rising
    } else {
        SwingDirection::Mixed
    };
    Some(SwingState {
        direction,
        prior_high,
        last_high,
        prior_low,
        last_low,
        pivot_age: (ks.len() - 1 - last_high_i).max(ks.len() - 1 - last_low_i),
    })
}

/// 大盘融合所用的指数清单（东财**显式 secid**，市场位 1=沪 / 0=深）。
///
/// 必须写显式 secid：裸码 `000300` 过 `to_em_secid` 会按首位数字推市场位、拼成深市
/// `0.000300`，只是恰好被 vendor 兜底回指数序列才没露馅（2026-10-16 live 探针
/// `axagent-astock-data` 的 `index_secid_live_probe` 实测：裸码 4179.51、显式 `1.000300`
/// 4178.89，同量级但不同源）。裸码写法还依赖 `is_benchmark_code` 的复权短路按字面匹配，
/// 换代码时两边必须同步扩。
pub const REGIME_INDEX_SECIDS: [(&str, &str); 3] =
    [("1.000300", "沪深300"), ("1.000985", "中证全指"), ("1.000016", "上证50")];

/// 单个指数对大盘结论的贡献。
#[derive(Debug, Clone)]
pub struct IndexVote<'a> {
    pub label: &'a str,
    pub regime: MarketRegimeType,
    pub confidence: f64,
    pub volatility: VolatilityLevel,
    /// 均线体系给出的方向（用于区分「均线确认」与「波段补票」）。
    pub ma_regime: MarketRegimeType,
    pub swing: Option<SwingState>,
}

/// 一个指数的 K 线 → 一张票：均线判出方向时直接用它；均线判震荡时用波段结构补票。
///
/// 补票是 P2-e 的全部要点 —— 60 日均线体系确认熊市要数周，而高低点同步下移在数日内
/// 就成立（紫金 10-09 误判发生时均线仍是震荡）。
pub fn index_vote<'a>(label: &'a str, klines: &[KLine]) -> IndexVote<'a> {
    let ma = classify_regime(klines);
    let swing = swing_structure(klines, SWING_LOOKBACK);
    let (regime, confidence) = match ma.regime {
        MarketRegimeType::Sideways => match swing.map(|s| s.direction) {
            Some(SwingDirection::Falling) => (MarketRegimeType::Bear, SWING_VOTE_CONFIDENCE),
            Some(SwingDirection::Rising) => (MarketRegimeType::Bull, SWING_VOTE_CONFIDENCE),
            Some(SwingDirection::Mixed) | None => (MarketRegimeType::Sideways, ma.confidence),
        },
        other => (other, ma.confidence),
    };
    IndexVote { label, regime, confidence, volatility: ma.volatility, ma_regime: ma.regime, swing }
}

impl SwingState {
    fn tag(&self) -> &'static str {
        match self.direction {
            SwingDirection::Rising => "波段上移",
            SwingDirection::Falling => "波段下移",
            SwingDirection::Mixed => "波段未确立",
        }
    }
}

impl IndexVote<'_> {
    fn text(&self) -> String {
        let swing = match self.swing {
            Some(ref s) => format!("，{}（结构距今{}根）", s.tag(), s.pivot_age),
            None => "，波段不可识别".to_string(),
        };
        let source =
            if self.ma_regime == MarketRegimeType::Sideways && self.regime != self.ma_regime {
                "波段补票"
            } else {
                "均线"
            };
        format!(
            "{}={} {:.0}%（{}{}）",
            self.label,
            self.regime.label(),
            self.confidence * 100.0,
            source,
            swing
        )
    }
}

/// 多指数 + 波段融合的确认票规则。
///
/// 规则（PLAN P2-e 裁定）：某指数的一张票 = 均线判出该方向，或均线判震荡而波段结构同步
/// 下移/上移；**≥2 bear 票 ⇒ 熊、≥2 bull 票 ⇒ 牛、否则震荡**。取数失败或不足 `VOTE_MIN_BARS`
/// 的指数不进票，但必须在描述里点名 —— 「拿不到」与「拿到了但中性」是两件事。
pub fn classify_regime_multi(series: &[(&str, &[KLine])]) -> MarketRegime {
    let mut votes: Vec<IndexVote> = Vec::new();
    let mut absent: Vec<&str> = Vec::new();
    for &(label, klines) in series {
        if klines.len() < VOTE_MIN_BARS {
            absent.push(label);
        } else {
            votes.push(index_vote(label, klines));
        }
    }

    if votes.is_empty() {
        return MarketRegime {
            regime: MarketRegimeType::Sideways,
            confidence: 0.3,
            volatility: VolatilityLevel::Normal,
            description: format!(
                "大盘确认不可用：{} 只指数均无有效 K 线（缺席：{}）",
                series.len(),
                absent.join("、")
            ),
        };
    }

    let bear: Vec<&IndexVote> =
        votes.iter().filter(|v| v.regime == MarketRegimeType::Bear).collect();
    let bull: Vec<&IndexVote> =
        votes.iter().filter(|v| v.regime == MarketRegimeType::Bull).collect();
    let flat: Vec<&IndexVote> =
        votes.iter().filter(|v| v.regime == MarketRegimeType::Sideways).collect();

    // 单指数有效时不假装三票确认：沿用其方向但按降级乘数收缩置信。
    let (regime, confidence, basis) = if votes.len() == 1 {
        let v = &votes[0];
        (v.regime, v.confidence * SINGLE_VOTE_PENALTY, "仅 1 只指数有效")
    } else if bear.len() >= 2 {
        (MarketRegimeType::Bear, mean_conf(&bear), "≥2 指数给出熊市票")
    } else if bull.len() >= 2 {
        (MarketRegimeType::Bull, mean_conf(&bull), "≥2 指数给出牛市票")
    } else if flat.is_empty() {
        // 方向票互相对冲（如 1 熊 1 牛）：仍是震荡，但置信不给高分
        (MarketRegimeType::Sideways, 0.5, "指数方向分歧，未取得 2 票共识")
    } else {
        (MarketRegimeType::Sideways, mean_conf(&flat), "无一方取得 2 票")
    };

    let absent_txt = if absent.is_empty() {
        "无".to_string()
    } else {
        absent.join("、")
    };
    let vote_txt = votes.iter().map(|v| v.text()).collect::<Vec<_>>().join("；");
    let description = format!(
        "多指数确认（有效 {} 票：熊 {} / 牛 {} / 震荡 {}，依据：{}）⇒ {}。明细：{}。缺席指数：{}",
        votes.len(),
        bear.len(),
        bull.len(),
        flat.len(),
        basis,
        regime.label(),
        vote_txt,
        absent_txt
    );

    MarketRegime { regime, confidence, volatility: median_volatility(&votes), description }
}

/// 多指数波动率取中位等级（不取平均：等级是序数，均值无意义）。
fn mean_conf(votes: &[&IndexVote]) -> f64 {
    votes.iter().map(|v| v.confidence).sum::<f64>() / votes.len() as f64
}

/// 多指数波动率取中位等级（不取平均：等级是序数，均值无意义）。
fn median_volatility(votes: &[IndexVote]) -> VolatilityLevel {
    let rank = |v: VolatilityLevel| match v {
        VolatilityLevel::Low => 0,
        VolatilityLevel::Normal => 1,
        VolatilityLevel::High => 2,
    };
    let mut r: Vec<u8> = votes.iter().map(|v| rank(v.volatility)).collect();
    r.sort_unstable();
    match r[r.len() / 2] {
        0 => VolatilityLevel::Low,
        2 => VolatilityLevel::High,
        _ => VolatilityLevel::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_klines(closes: &[f64]) -> Vec<KLine> {
        let mut klines = Vec::new();
        for (i, &c) in closes.iter().enumerate() {
            klines.push(KLine {
                date: format!("2026-01-{:02}", i + 1),
                open: c * 0.99,
                high: c * 1.02,
                low: c * 0.98,
                close: c,
                volume: 100_000.0,
                amount: c * 100_000.0 * 100.0,
                turnover_rate: None,
                adj_factor: None,
            });
        }
        klines
    }

    #[test]
    fn test_bull_market() {
        // 持续上涨 → 牛市
        let closes: Vec<f64> = (1..=60).map(|i| 3000.0 + i as f64 * 15.0).collect();
        let r = classify_regime(&make_klines(&closes));
        assert_eq!(r.regime, MarketRegimeType::Bull, "持续上涨应识别为牛市: {}", r.description);
        assert!(r.confidence >= 0.5);
    }

    #[test]
    fn test_bear_market() {
        // 持续下跌 → 熊市
        let closes: Vec<f64> = (1..=60).map(|i| 4000.0 - i as f64 * 15.0).collect();
        let r = classify_regime(&make_klines(&closes));
        assert_eq!(r.regime, MarketRegimeType::Bear, "持续下跌应识别为熊市: {}", r.description);
        assert!(r.confidence >= 0.5);
    }

    #[test]
    fn test_sideways_market() {
        // 窄幅震荡 → 震荡
        let mut closes = vec![3500.0; 60];
        for i in 1..60 {
            closes[i] = closes[i - 1] + (i as f64 % 5.0 - 2.0);
        }
        let r = classify_regime(&make_klines(&closes));
        assert_eq!(r.regime, MarketRegimeType::Sideways, "窄幅震荡应识别为震荡: {}", r.description);
    }

    #[test]
    fn test_insufficient_data() {
        let closes = vec![3500.0; 10];
        let r = classify_regime(&make_klines(&closes));
        assert_eq!(r.regime, MarketRegimeType::Sideways);
        assert!(r.confidence <= 0.3);
    }

    #[test]
    fn test_volatility_detection() {
        // 高波动：大幅震荡，振幅 > 20%
        let mut closes = Vec::new();
        for i in 0..60 {
            let cycle = (i as f64 * 0.5).sin(); // -1 ~ +1
            closes.push(3500.0 + cycle * 1200.0); // ±1200, ~34% 振幅
        }
        let r = classify_regime(&make_klines(&closes));
        assert_eq!(
            r.volatility,
            VolatilityLevel::High,
            "大幅震荡应识别为高波动, got={:?}",
            r.volatility
        );
    }

    /// 相邻拐点间线性插值成收盘序列（每段 `leg` 根，拐点值落在段首，末尾补最后一个拐点）。
    fn zig(turns: &[f64], leg: usize) -> Vec<f64> {
        let mut out = Vec::new();
        for w in turns.windows(2) {
            for s in 0..leg {
                out.push(w[0] + (w[1] - w[0]) * (s as f64) / (leg as f64));
            }
        }
        out.push(*turns.last().expect("拐点序列不得为空"));
        out
    }

    /// 均线判不出熊市（末点只低于短期均线 1.6%，未到 -3% 阈值），但高低点同步下移 ——
    /// 这正是 P2-e 要用波段补票去压缩滞后的形状。
    fn falling_klines() -> Vec<KLine> {
        make_klines(&zig(&[3500., 3400., 3480., 3380., 3460., 3360.], 10))
    }

    fn rising_klines() -> Vec<KLine> {
        make_klines(&zig(&[3360., 3460., 3380., 3480., 3400., 3500.], 10))
    }

    /// 高点下移而低点持平 ⇒ 结构未确立，不得进票。
    fn mixed_klines() -> Vec<KLine> {
        make_klines(&zig(&[3500., 3400., 3480., 3400., 3460., 3380.], 10))
    }

    fn strong_bear_klines() -> Vec<KLine> {
        make_klines(&(1..=60).map(|i| 4000.0 - i as f64 * 15.0).collect::<Vec<f64>>())
    }

    fn no_pivot_klines() -> Vec<KLine> {
        make_klines(&(1..=60).map(|i| 3500.0 + i as f64).collect::<Vec<f64>>())
    }

    #[test]
    fn swing_structure_reads_falling_pivots() {
        let s = swing_structure(&falling_klines(), 60).expect("两组下降拐点应识别出结构");
        assert_eq!(s.direction, SwingDirection::Falling);
        assert!(s.last_high < s.prior_high && s.last_low < s.prior_low);
    }

    #[test]
    fn swing_structure_reads_rising_pivots() {
        let s = swing_structure(&rising_klines(), 60).expect("两组上升拐点应识别出结构");
        assert_eq!(s.direction, SwingDirection::Rising);
        assert!(s.last_high > s.prior_high && s.last_low > s.prior_low);
    }

    #[test]
    fn swing_structure_reports_mixed_when_only_highs_move() {
        let s = swing_structure(&mixed_klines(), 60).expect("高低点各 2 组应能比较");
        assert_eq!(
            s.direction,
            SwingDirection::Mixed,
            "高点下移而低点持平不得算作同步下移: {:?}",
            s
        );
    }

    #[test]
    fn swing_structure_returns_none_without_pivots() {
        assert!(
            swing_structure(&no_pivot_klines(), 60).is_none(),
            "单调序列没有内部拐点，必须返回 None（缺席不等于未下移）"
        );
    }

    #[test]
    fn swing_vote_bears_when_ma_still_sideways() {
        let v = index_vote("沪深300", &falling_klines());
        assert_eq!(
            v.ma_regime,
            MarketRegimeType::Sideways,
            "夹具本身必须先被均线判成震荡，否则测不到补票: {}",
            classify_regime(&falling_klines()).description
        );
        assert_eq!(v.regime, MarketRegimeType::Bear, "均线震荡 + 波段下移应补出熊市票");
        assert!((v.confidence - SWING_VOTE_CONFIDENCE).abs() < 1e-9);
    }

    #[test]
    fn multi_vote_bears_on_two_swing_votes() {
        let (f, m) = (falling_klines(), mixed_klines());
        let r = classify_regime_multi(&[("沪深300", &f), ("中证全指", &f), ("上证50", &m)]);
        assert_eq!(r.regime, MarketRegimeType::Bear, "两只波段下移票应确认熊市: {}", r.description);
        assert!(r.description.contains("明细"), "描述须逐指数给出票面: {}", r.description);
        assert!(
            r.description.contains("上证50=震荡"),
            "未确立结构的指数应显式呈为震荡: {}",
            r.description
        );
    }

    #[test]
    fn multi_vote_refuses_single_bear_vote() {
        let (b, r2, m) = (strong_bear_klines(), rising_klines(), mixed_klines());
        let got = classify_regime_multi(&[("沪深300", &b), ("中证全指", &r2), ("上证50", &m)]);
        assert_eq!(
            got.regime,
            MarketRegimeType::Sideways,
            "一票不足以确认全市场方向: {}",
            got.description
        );
    }

    #[test]
    fn multi_vote_bulls_symmetrically() {
        let (u, u2, m) = (rising_klines(), rising_klines(), mixed_klines());
        let r = classify_regime_multi(&[("沪深300", &u), ("中证全指", &u2), ("上证50", &m)]);
        assert_eq!(r.regime, MarketRegimeType::Bull, "判据须对称，不得只补空票: {}", r.description);
    }

    #[test]
    fn multi_vote_names_absent_index_in_description() {
        let f = falling_klines();
        let empty: Vec<KLine> = Vec::new();
        let r = classify_regime_multi(&[("沪深300", &f), ("中证全指", &f), ("上证50", &empty)]);
        assert_eq!(r.regime, MarketRegimeType::Bear);
        assert!(
            r.description.contains("缺席指数：上证50"),
            "取数失败的指数必须点名，不得静默少一票: {}",
            r.description
        );
    }

    #[test]
    fn multi_vote_single_valid_index_is_penalised() {
        let b = strong_bear_klines();
        let empty: Vec<KLine> = Vec::new();
        let v = index_vote("沪深300", &b);
        let r = classify_regime_multi(&[("沪深300", &b), ("中证全指", &empty), ("上证50", &empty)]);
        assert_eq!(r.regime, v.regime);
        assert!(
            (r.confidence - v.confidence * SINGLE_VOTE_PENALTY).abs() < 1e-9,
            "仅一票时应按降级乘数收缩置信，实得 {}",
            r.confidence
        );
        assert!(r.description.contains("仅 1 只指数有效"), "{}", r.description);
    }

    #[test]
    fn multi_vote_without_any_data_is_low_confidence() {
        let empty: Vec<KLine> = Vec::new();
        let r =
            classify_regime_multi(&[("沪深300", &empty), ("中证全指", &empty), ("上证50", &empty)]);
        assert_eq!(r.regime, MarketRegimeType::Sideways);
        assert!(r.confidence <= 0.3, "无数据时不得给出高置信: {}", r.confidence);
        assert!(r.description.contains("大盘确认不可用"), "{}", r.description);
    }
}
