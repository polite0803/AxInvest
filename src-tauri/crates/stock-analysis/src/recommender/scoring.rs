//! 智能荐股 — 置信度、仓位、去重、缓存

use crate::recommender::types::{Period, RecoPick, Style};
use std::collections::HashMap;

/// 置信度计算
///
/// - `score_consistency`: 子策略内多因子方向一致率 (0-1)
/// - `signal_strength`: 关键因子偏离度分位 (0-1)
/// - `liquidity_score`: 成交额 / 换手率分位 (0-1)
/// - `market_regime`: 大盘环境 (-0.5 ~ +0.5)
/// - `turnover_anomaly`: 今日成交额 / 20日均（> 3 视为异常）
pub fn calc_confidence(
    score_consistency: f64,
    signal_strength: f64,
    liquidity_score: f64,
    market_regime: f64,
    turnover_anomaly: f64,
) -> u8 {
    // P3-1: sanitize inputs — NaN propagates through arithmetic and breaks .round() downstream.
    let clean = |v: f64| if v.is_nan() { 0.0 } else { v };
    let score_consistency = clean(score_consistency);
    let signal_strength = clean(signal_strength);
    let liquidity_score = clean(liquidity_score);
    let market_regime = clean(market_regime);
    let turnover_anomaly = clean(turnover_anomaly);

    let mut c = 0.45 * score_consistency
        + 0.25 * signal_strength
        + 0.15 * liquidity_score
        + 0.15 * market_regime;

    // 成交额异常否决：> 3x 平均 → 减 40% 封顶（不低于原值 60%）
    if turnover_anomaly > 3.0 {
        c *= 0.6;
    }

    if c.is_nan() {
        return 0;
    }
    (c * 100.0).clamp(0.0, 100.0).round() as u8
}

/// 仓位动态化：base × confidence/100 × period_factor
pub fn calc_position(base: f64, confidence: u8, period: Period) -> f64 {
    if base.is_nan() {
        return 0.0;
    }
    let c = confidence as f64 / 100.0;
    (base * c * period.factor() * 100.0).round() / 100.0
}

/// 同票去重：保留 confidence 最高，标注次选风格
pub fn dedup_and_merge(picks: &mut Vec<RecoPick>) {
    let mut by_code: HashMap<String, RecoPick> = HashMap::new();
    for p in picks.drain(..) {
        if let Some(existing) = by_code.get_mut(&p.stock_code) {
            // 同票被多风格命中
            if p.confidence > existing.confidence {
                // p 取代 existing，existing 的风格 + 两者各自的 secondary 合入
                let mut merged = p;
                let mut secondaries: Vec<Style> = Vec::new();
                secondaries.push(existing.style);
                secondaries.extend(existing.secondary_styles.iter().copied());
                secondaries.extend(merged.secondary_styles.iter().copied());
                secondaries.sort_by_key(|st| st.as_str());
                secondaries.dedup();
                merged.secondary_styles = secondaries;
                *existing = merged;
            } else {
                // p 的 confidence 不更高：把 p 的风格追加到 existing 的 secondary
                let mut secondaries: Vec<Style> = Vec::new();
                secondaries.push(p.style);
                secondaries.extend(existing.secondary_styles.iter().copied());
                secondaries.sort_by_key(|st| st.as_str());
                secondaries.dedup();
                existing.secondary_styles = secondaries;
            }
        } else {
            by_code.insert(p.stock_code.clone(), p);
        }
    }
    picks.extend(by_code.into_values());
}

/// 按风格分组 + 每组 top N
pub fn group_by_style_and_trim(picks: &mut Vec<RecoPick>, per_style_limit: usize) -> HashMap<Style, Vec<RecoPick>> {
    let mut by_style: HashMap<Style, Vec<RecoPick>> = HashMap::new();
    for p in picks.drain(..) {
        by_style.entry(p.style).or_default().push(p);
    }
    for v in by_style.values_mut() {
        v.sort_by(|a, b| b.confidence.cmp(&a.confidence));
        v.truncate(per_style_limit);
    }
    by_style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_basic_80() {
        // 全 0.8 因子 → 0.45*0.8 + 0.25*0.8 + 0.15*0.8 + 0.15*0.0 = 0.68 → 68
        let c = calc_confidence(0.8, 0.8, 0.8, 0.0, 1.0);
        assert_eq!(c, 68, "expected 68 got {}", c);
    }

    #[test]
    fn confidence_full_bull() {
        // 全 1.0 因子 + 牛市 +0.5 → 0.45+0.25+0.15+0.075 = 0.925 → 93
        let c = calc_confidence(1.0, 1.0, 1.0, 0.5, 1.0);
        assert_eq!(c, 93, "expected 93 got {}", c);
    }

    #[test]
    fn confidence_turnover_anomaly_caps_at_minus_40pct() {
        // 正常 80 → 异常减 40% 封顶 → 80 * 0.6 = 48
        let c = calc_confidence(0.8, 0.8, 0.8, 0.0, 5.0);
        assert_eq!(c, 41, "expected 41 got {}", c); // 0.68 * 0.6 = 0.408 → 41
    }

    #[test]
    fn confidence_clamped_to_100() {
        let c = calc_confidence(1.0, 1.0, 1.0, 1.0, 1.0);
        assert_eq!(c, 100);
    }

    #[test]
    fn position_short_low_conf() {
        // base=5, conf=60, short factor 0.6 → 5*0.6*0.6 = 1.8
        let p = calc_position(5.0, 60, Period::Short);
        assert!((p - 1.8).abs() < 0.01, "got {}", p);
    }

    #[test]
    fn position_long_high_conf() {
        // base=10, conf=80, long factor 1.0 → 10*0.8*1.0 = 8.0
        let p = calc_position(10.0, 80, Period::Long);
        assert!((p - 8.0).abs() < 0.01, "got {}", p);
    }

    #[test]
    fn dedup_keeps_higher_confidence_and_records_secondary() {
        let mut picks = vec![
            RecoPick {
                stock_code: "600519".into(),
                stock_name: "贵州茅台".into(),
                sector: None,
                style: Style::Trend,
                period: Period::Mid,
                price: 100.0,
                entry_low: 99.0,
                entry_high: 101.0,
                stop_loss: 95.0,
                target_price: 110.0,
                position_pct: 5.0,
                holding_days: 28,
                confidence: 70,
                reasons: vec![],
                risk_notes: vec![],
                secondary_styles: vec![],
            },
            RecoPick {
                stock_code: "600519".into(),
                stock_name: "贵州茅台".into(),
                sector: None,
                style: Style::Value,
                period: Period::Mid,
                price: 100.0,
                entry_low: 99.0,
                entry_high: 101.0,
                stop_loss: 95.0,
                target_price: 110.0,
                position_pct: 5.0,
                holding_days: 28,
                confidence: 80,
                reasons: vec![],
                risk_notes: vec![],
                secondary_styles: vec![],
            },
        ];
        dedup_and_merge(&mut picks);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].style, Style::Value);
        assert_eq!(picks[0].confidence, 80);
        assert!(picks[0].secondary_styles.contains(&Style::Trend));
    }

    #[test]
    fn dedup_preserves_existing_secondary_styles() {
        // A 已被 Trend 命中，secondary=[Value]
        // C 用更高 confidence 命中，合并后 secondary 应是 [Trend, Value]
        let mut picks = vec![
            RecoPick {
                stock_code: "600519".into(),
                stock_name: "贵州茅台".into(),
                sector: None,
                style: Style::Trend,
                period: Period::Mid,
                price: 100.0,
                entry_low: 99.0,
                entry_high: 101.0,
                stop_loss: 95.0,
                target_price: 110.0,
                position_pct: 5.0,
                holding_days: 28,
                confidence: 60,
                reasons: vec![],
                risk_notes: vec![],
                secondary_styles: vec![Style::Value],
            },
            RecoPick {
                stock_code: "600519".into(),
                stock_name: "贵州茅台".into(),
                sector: None,
                style: Style::Capital,
                period: Period::Mid,
                price: 100.0,
                entry_low: 99.0,
                entry_high: 101.0,
                stop_loss: 95.0,
                target_price: 110.0,
                position_pct: 5.0,
                holding_days: 28,
                confidence: 80,
                reasons: vec![],
                risk_notes: vec![],
                secondary_styles: vec![],
            },
        ];
        dedup_and_merge(&mut picks);
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].style, Style::Capital);
        let secs = &picks[0].secondary_styles;
        assert!(secs.contains(&Style::Trend), "应保留 Trend: {:?}", secs);
        assert!(secs.contains(&Style::Value), "应保留 Value: {:?}", secs);
        assert_eq!(secs.len(), 2, "去重后应只有 2 个: {:?}", secs);
    }

    #[test]
    fn group_by_style_trims_to_limit() {
        let mut picks: Vec<RecoPick> = (0..15)
            .map(|i| RecoPick {
                stock_code: format!("{}", i),
                stock_name: "X".into(),
                sector: None,
                style: Style::Trend,
                period: Period::Short,
                price: 10.0,
                entry_low: 9.5,
                entry_high: 10.5,
                stop_loss: 9.0,
                target_price: 11.0,
                position_pct: 3.0,
                holding_days: 5,
                confidence: i as u8,
                reasons: vec![],
                risk_notes: vec![],
                secondary_styles: vec![],
            })
            .collect();
        let grouped = group_by_style_and_trim(&mut picks, 10);
        assert_eq!(grouped.get(&Style::Trend).unwrap().len(), 10);
    }
}
