//! 智能荐股 — 置信度、仓位、去重、缓存

use crate::recommender::types::{Period, RecoPick, Style};
use std::collections::{BTreeMap, HashMap};

/// 评分权重（一致性 / 信号强度 / 流动性 / 动量）。
///
/// ⚠ 这里**没有**「自适应」：旧实现挂着一个 `Mutex<ScoringWeights>` + `nudge_scoring_weights`
/// （EWMA 0.7/0.3）注释写「可通过回测/反思反馈调优」，但全仓**没有任何调用方**写它
/// ⇒ 四档永远读到的是出厂常量，而注释让人以为它已被数据校准过（纸面可调）。
/// 已删该死机制；真要接回写通路，得与 `weighted_signal_calibration` / 逐档 IC（Phase R-E）
/// 一起设计，而不是留一份没人写的表在这里充当证据。
#[derive(Debug, Clone, Copy)]
pub struct ScoringWeights {
    pub consistency: f64,
    pub signal_strength: f64,
    pub liquidity: f64,
    pub price_momentum: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self { consistency: 0.45, signal_strength: 0.35, liquidity: 0.15, price_momentum: 0.05 }
    }
}

pub const WEIGHTS: ScoringWeights = ScoringWeights {
    consistency: 0.45,
    signal_strength: 0.35,
    liquidity: 0.15,
    price_momentum: 0.05,
};

/// 把「该档先验胜率」与「该风格内的评分」在 **logit 空间**合成为一个绝对置信度（Phase R-C）。
///
/// `logit(conf) = logit(prior) + 2·s·(score − 0.5)`
///
/// - 先验来自 `horizon_prior::horizon_prior_map`（该档历史方向命中率经经验贝叶斯收缩，
///   κ 可调）⇒ **四档各自有自己的锚**，不再是共用一个数（诊断 E1 在荐股链的形态 R1）。
/// - 斜率 `s` 是唯一新增可调量（`reco_conf_sensitivity`，出厂 1.0）：s=0 ⇒ 只承认先验、
///   评分不起作用；s 越大越信评分。写成 logit 加法而不是线性相加，是为了让「先验 0.5 附近
///   的小改进」与「先验极端处的小改进」有可比的信息量（对数几率的可加性）。
/// - 先验不可得 ⇒ 退回 `score`（调用方必须标 `priorSource="absent"`），不假装合成过。
pub fn blend_confidence(prior_win_rate: Option<f64>, score: f64, sensitivity: f64) -> Option<u8> {
    let p = prior_win_rate?;
    if !(p > 0.0 && p < 1.0) || !score.is_finite() {
        return None;
    }
    let logit = (p / (1.0 - p)).ln() + 2.0 * sensitivity * (score - 0.5);
    let blended = 1.0 / (1.0 + (-logit).exp());
    if !blended.is_finite() {
        return None;
    }
    Some((blended * 100.0).clamp(0.0, 100.0).round() as u8)
}

pub fn calc_confidence(
    score_consistency: f64,
    signal_strength: f64,
    liquidity_score: f64,
    price_momentum: f64,
    turnover_anomaly: f64,
) -> u8 {
    // sanitize inputs
    let clean = |v: f64| {
        if v.is_nan() || v.is_infinite() {
            0.0
        } else {
            v
        }
    };
    let score_consistency = clean(score_consistency);
    let signal_strength = clean(signal_strength);
    let liquidity_score = clean(liquidity_score);
    let price_momentum = clean(price_momentum);
    let turnover_anomaly = clean(turnover_anomaly);

    let w = WEIGHTS;
    let mut c = w.consistency * score_consistency
        + w.signal_strength * signal_strength
        + w.liquidity * liquidity_score
        + w.price_momentum * price_momentum;

    // 成交额异常阶梯惩罚（替代原断崖式 40%）：
    //   1.0-2.0x → 无惩罚（正常交易）
    //   2.0-3.0x → 10% 置信度衰减（温和放量）
    //   3.0-5.0x → 25% 置信度衰减（异常放量，可能是出货）
    //   >5.0x    → 40% 置信度衰减（极端放量，高概率操纵）
    if turnover_anomaly > 5.0 {
        c *= 0.60; // −40%
    } else if turnover_anomaly > 3.0 {
        c *= 0.75; // −25%
    } else if turnover_anomaly > 2.0 {
        c *= 0.90; // −10%
    }

    if c.is_nan() || c.is_infinite() {
        return 0;
    }
    (c * 100.0).clamp(0.0, 100.0).round() as u8
}

/// 仓位动态化：Kelly 公式近似
///
/// 标准 Kelly: f* = (p·b − q) / b
///   其中 p=胜率, q=1−p, b=盈亏比(target/stop)
///
/// 本实现：base × confidence/100 × period_factor × consistency_penalty
///   - confidence/100 近似 p（置信度 ∝ 预期胜率）
///   - base 由各策略按 target/stop 比预先设定（近似 Kelly f*）
///   - period_factor 按持有期缩放（超短线 0.4 → 长线 1.0）
///   - consistency 多因子一致性（0-1），低一致性 → 降仓惩罚
///
/// 简化原因：p 和 b 无法在子策略内精确估计（依赖 K 线外数据），
/// 故用信度代理概率、用参数化的 base 代理赔率调整。
///
/// 惩罚曲线：
///   consistency ≥ 0.6 → 无惩罚 (×1.0)
///   consistency 0.3 → ×0.85
///   consistency 0.0 → ×0.60
pub fn calc_position(base: f64, confidence: u8) -> f64 {
    calc_position_with_consistency(base, confidence, 1.0)
}

/// 带一致性惩罚的仓位计算
///
/// `consistency` (0-1)：同一策略内多因子方向一致率，
///   例如 4 个因子中 3 个方向一致 → 0.75。
///   低一致性时应用线性惩罚降仓。
pub fn calc_position_with_consistency(base: f64, confidence: u8, consistency: f64) -> f64 {
    if base.is_nan() {
        return 0.0;
    }
    let c = confidence as f64 / 100.0;
    // 主路径**不乘**经验周期乘数：仓位由 `risk::risk_budget_position`（风险预算）承载，
    // 乘数只在 σ 不可得时作为降级分支出现（`calc_position_fallback_mult`，门 b 锁这一点）。
    let raw = base * c;
    // 一致性惩罚：consistency 0.0→0.60, 0.3→0.85, 0.6→1.0
    let penalty = if consistency >= 0.6 {
        1.0
    } else if consistency >= 0.3 {
        // 0.3~0.6 线性插值: 0.85 → 1.0
        0.85 + (consistency - 0.3) / 0.3 * 0.15
    } else {
        // 0.0~0.3 线性插值: 0.60 → 0.85
        0.60 + consistency / 0.3 * 0.25
    };
    (raw * penalty * 100.0).round() / 100.0
}

/// 降级分支：σ 不可得时，仓位退回「base × 置信 × 经验周期乘数」。
///
/// 之所以单独成函数而不是混在主路径里：`Period::factor()`（0.4/0.6/0.8/1.0）是**没有推导的
/// 经验数**（诊断 E6），它只能出现在「拿不到波动率」这条分支上，且调用方必须把
/// `positionSource` 标成 `fallback_kelly_x_mult` —— 否则用户读到的是「按风险预算定的仓位」。
pub fn calc_position_fallback_mult(base: f64, confidence: u8, period: Period) -> f64 {
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

/// 按风格分组 + 每组 top N + 同组置信度归一化
pub fn group_by_style_and_trim(
    picks: &mut Vec<RecoPick>,
    per_style_limit: usize,
) -> BTreeMap<Style, Vec<RecoPick>> {
    let mut by_style: BTreeMap<Style, Vec<RecoPick>> = BTreeMap::new();
    for p in picks.drain(..) {
        by_style.entry(p.style).or_default().push(p);
    }
    for v in by_style.values_mut() {
        // 同一风格内的置信度 min-max 归一化，解决不同策略置信度不可比问题
        let min_conf = v.iter().map(|p| p.confidence).min().unwrap_or(0);
        let max_conf = v.iter().map(|p| p.confidence).max().unwrap_or(100);
        if max_conf <= min_conf {
            // 组内置信度完全一致 → min-max 归一化在数学上无定义（分母为 0）。
            //
            // 这里必须保持原值：同策略同周期的 conf 由同一组固定参数算出，
            // 只要该策略不消费 per-stock 的变率因子，组内必然全部同值
            // （capital.rs / value.rs 的 turnover_anomaly 传常数 1.0，100% 命中此分支）。
            //
            // 旧实现此处把 range 回退成 100.0，于是 normalized 恒等于 0，
            // 再被 clamp 到下限 1 —— 真实 pick 的置信度被整体压成 1，
            // 反而低于 synthetic 兜底（conf=40），真实信号在展示上被假数据反超。
            v.sort_by_key(|b| std::cmp::Reverse(b.confidence));
            v.truncate(per_style_limit);
            continue;
        }
        let range = (max_conf - min_conf) as f64;
        for p in v.iter_mut() {
            // 覆写 `confidence` 会把「该档胜率 62%」变成「今天组内第二好」——
            // 分位数与概率是两个量纲，混写会让按 conf 做的命中率/IC 标定全部失真（诊断 R9）。
            // 现在分位单独进 `confidence_percentile`，绝对置信度原样保留。
            let original = p.confidence;
            let normalized = ((original as f64 - min_conf as f64) / range * 100.0).round() as u8;
            p.confidence_percentile = Some(normalized.clamp(1, 100));
            p.reasons.push(format!(
                "组内当日分位 {}（绝对置信 {} 未改写）",
                normalized.clamp(1, 100),
                original
            ));
        }
        v.sort_by_key(|b| std::cmp::Reverse(b.confidence));
        v.truncate(per_style_limit);
    }
    by_style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_basic_80() {
        // 0.45*0.8 + 0.35*0.8 + 0.15*0.8 + 0.05*0.0 = 0.36+0.28+0.12 = 0.76 → 76
        let c = calc_confidence(0.8, 0.8, 0.8, 0.0, 1.0);
        assert_eq!(c, 76, "expected 76 got {}", c);
    }

    #[test]
    fn confidence_full_bull() {
        // 0.45+0.35+0.15+0.05*0.5 = 0.45+0.35+0.15+0.025 = 0.975 → 98
        let c = calc_confidence(1.0, 1.0, 1.0, 0.5, 1.0);
        assert_eq!(c, 98, "expected 98 got {}", c);
    }

    #[test]
    fn confidence_turnover_anomaly_graded() {
        // 正常 score 0.76 → 3-5x 扣 25% → 0.76*0.75 = 0.57 → 57
        let c = calc_confidence(0.8, 0.8, 0.8, 0.0, 4.0);
        assert_eq!(c, 57, "expected 57 got {}", c);
    }

    #[test]
    fn confidence_clamped_to_100() {
        let c = calc_confidence(1.0, 1.0, 1.0, 1.0, 1.0);
        assert_eq!(c, 100);
    }

    /// 主路径仓位 = base × 置信，**不乘**经验周期乘数（Phase R-D）。
    /// 旧断言写的是 `5×0.6×0.6=1.8`（含 short factor 0.6），那是「周期仓位差异靠一个拍脑袋
    /// 乘数」的形态；现在周期差异由波动率风险预算承载（`risk::risk_budget_position`）。
    #[test]
    fn position_main_path_does_not_multiply_period_factor() {
        let p = calc_position(5.0, 60);
        assert!((p - 3.0).abs() < 0.01, "base×conf = 5×0.6 = 3.0，实际 {p}");
        let q = calc_position(10.0, 80);
        assert!((q - 8.0).abs() < 0.01, "base×conf = 10×0.8 = 8.0，实际 {q}");
        // 主路径仓位上限与档位**无关**：周期差异全部由该档止损宽度经风险预算折算而来
        // （`risk::risk_budget_position` + `recommender::mod.rs` 后处理），故签名不再收 period。
    }

    /// 降级分支仍然乘经验乘数，且这是它**唯一**存在的理由（门 b 锁消费面只在 fallback）。
    #[test]
    fn fallback_mult_branch_keeps_period_multiplier() {
        let p = calc_position_fallback_mult(5.0, 60, Period::Short);
        assert!((p - 1.8).abs() < 0.01, "5×0.6×0.6 = 1.8，实际 {p}");
        let q = calc_position_fallback_mult(5.0, 60, Period::UltraShort);
        assert!((q - 1.2).abs() < 0.01, "超短 factor 0.4 ⇒ 1.2，实际 {q}");
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
                confidence_percentile: None,
                prior_source: None,
                prior_samples: None,
                stop_source: None,
                position_source: None,

                synthetic: false,
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
                confidence_percentile: None,
                prior_source: None,
                prior_samples: None,
                stop_source: None,
                position_source: None,

                synthetic: false,
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
                confidence_percentile: None,
                prior_source: None,
                prior_samples: None,
                stop_source: None,
                position_source: None,

                synthetic: false,
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
                confidence_percentile: None,
                prior_source: None,
                prior_samples: None,
                stop_source: None,
                position_source: None,

                synthetic: false,
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
                confidence_percentile: None,
                prior_source: None,
                prior_samples: None,
                stop_source: None,
                position_source: None,

                synthetic: false,
            })
            .collect();
        let grouped = group_by_style_and_trim(&mut picks, 10);
        assert_eq!(grouped.get(&Style::Trend).unwrap().len(), 10);
    }
}
