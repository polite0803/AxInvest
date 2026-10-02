//! 智能荐股 — 置信度、仓位、去重、缓存

use crate::recommender::types::{Period, RecoPick, Style, TrimmedPick};
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

/// 把「该档先验胜率」与「该风格内的评分」合成为该档的**上涨胜率**（0-1）。
///
/// 口径与股票分析决策链**同标尺**（Phase R-C + 2026-10-01 口径统一）。两步缺一不可：
///
/// ① **证据合成（logit 空间）**：`logit(后验) = logit(prior) + 2·s·(score − 0.5)`
///    - 先验来自 `horizon_prior::horizon_prior_map`（与分析链 `horizon_prior_json`
///      **同一份实现**）；
///    - 斜率 `s` 是唯一可调量（`reco_conf_sensitivity`，出厂 1.0）：s=0 ⇒ 只承认先验、
///      评分不起作用。写成 logit 加法而不是线性相加，是让「先验 0.5 附近的小改进」与
///      「先验极端处的小改进」有可比的信息量（对数几率的可加性）。
///
/// ② **时间折算**：`snr_confidence(后验, 该档持有天数, 锚定天数)`
///    —— 与分析链 `portfolio-mgr.rhai` 的 `pm_snr_confidence(heff, daysh, SNR_ANCHOR_DAYS)`
///    **同一函数、同一锚**：锚定档不改，短档向 0.5 收缩，长档放大（含 1.5× 上限）。
///
/// ⚠ 为什么必须补第 ② 步（2026-10-01 用户裁定「统一口径」）：
///   原实现到第 ① 步为止 ⇒ 荐股的「该档胜率」**没有时间维度**，而分析链有四档的
///   confidence 在两侧是两把尺子 —— 同一个「长期」概念，分析链里会被放大
///   （负边缘同样放大），荐股链里纹丝不动。
///   实证佐证：逐档先验在样本不足时四档**同值**（`source="pooled"`，实测 0.375）
///   ⇒ 档位差异**全部**来自本步折算；荐股链缺它，等于四档 confidence 只差一个先验，
///   而那个先验当时还四档同值 —— 即四档数值实质上无区别。
///
/// 先验不可得 ⇒ 返回 `None`（调用方退回纯评分并标 `priorSource="absent"`，
/// 不假装合成过）。
pub fn blend_win_rate(
    prior_win_rate: Option<f64>,
    score: f64,
    sensitivity: f64,
    holding_days: u32,
    anchor_days: u32,
) -> Option<f64> {
    let p = prior_win_rate?;
    if !(p > 0.0 && p < 1.0) || !score.is_finite() {
        return None;
    }
    let logit = (p / (1.0 - p)).ln() + 2.0 * sensitivity * (score - 0.5);
    let posterior = 1.0 / (1.0 + (-logit).exp());
    if !posterior.is_finite() {
        return None;
    }
    let win_rate = axagent_harness::indicators::snr_confidence(
        posterior,
        holding_days as usize,
        anchor_days as usize,
    );
    if win_rate.is_finite() {
        Some(win_rate)
    } else {
        None
    }
}

/// 把「工作流候选评分（0-100）」折算成该档**上涨胜率（0-100）**，返回 `(胜率, priorSource)`。
///
/// 口径与本仓另外两条链**完全一致**（`blend_win_rate`）：先验 → logit 合成 → 时间折算。
///
/// ⚠ 为什么需要它（2026-10-02 实测）：趋势智选链（`stock_workflow/serenity.rs`）原先把
/// 工作流 LLM 产出的候选评分**直接落库**，并把 `priorSource` 硬编码为 `absent`
/// ⇒ 该 `confidence` **不是概率**（它是"这候选有多符合瓶颈策略"的评分），而分析链的
/// `horizon_decisions[档].confidence` 是"该档上涨胜率" ⇒ 两者在 UI 上并排展示时，
/// 「荐股 78 vs 分析 46」是**两个不同量纲的数在比**，不是观点分歧。
///
/// 先验不可得 ⇒ 如实退回纯评分并标 `absent`（不假装合成过）—— 与策略链同一条退化纪律。
pub fn candidate_score_to_win_rate(
    conf_pct: i32,
    tier_prior: Option<&serde_json::Value>,
    sensitivity: f64,
    holding_days: u32,
    anchor_days: u32,
) -> (u8, String) {
    let prior = tier_prior.and_then(|r| r.get("prior")).and_then(|v| v.as_f64());
    let source = tier_prior
        .and_then(|r| r.get("source"))
        .and_then(|v| v.as_str())
        .unwrap_or("absent")
        .to_string();
    match blend_win_rate(prior, conf_pct as f64 / 100.0, sensitivity, holding_days, anchor_days) {
        Some(wr) => ((wr * 100.0).round().clamp(0.0, 100.0) as u8, source),
        None => (conf_pct.clamp(0, 100) as u8, "absent".to_string()),
    }
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
///
/// 只关心出票集合的调用方走这里；需要「被截断的候选」时用
/// [`group_by_style_and_trim_with_audit`]（同一实现，不另写一份排序）。
pub fn group_by_style_and_trim(
    picks: &mut Vec<RecoPick>,
    per_style_limit: usize,
) -> BTreeMap<Style, Vec<RecoPick>> {
    group_by_style_and_trim_with_audit(picks, per_style_limit).0
}

/// 同 [`group_by_style_and_trim`]，额外交出**被 top-N 截断**的候选（L3 留痕来源）。
///
/// 为什么在这里留：截断点只有这一处，而「策略确实返回了它、只是组内排序没进前 N」
/// 正是唯一无歧义的「评分压出」证据（见 `entities::reco_scan_audit` 的语义边界）。
/// `rank` 是组内名次（1 = 第一个被截掉的），与保留侧的排序同一次比较。
pub fn group_by_style_and_trim_with_audit(
    picks: &mut Vec<RecoPick>,
    per_style_limit: usize,
) -> (BTreeMap<Style, Vec<RecoPick>>, Vec<TrimmedPick>) {
    let mut trimmed: Vec<TrimmedPick> = Vec::new();
    let mut by_style: BTreeMap<Style, Vec<RecoPick>> = BTreeMap::new();
    for p in picks.drain(..) {
        by_style.entry(p.style).or_default().push(p);
    }
    for (style, v) in by_style.iter_mut() {
        let style = *style;
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
            trimmed.extend(drain_trimmed(v, style, per_style_limit));
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
        trimmed.extend(drain_trimmed(v, style, per_style_limit));
    }
    (by_style, trimmed)
}

/// 截断并交出尾部（名次从 1 起，1 = 第一个被截掉的）。
fn drain_trimmed(v: &mut Vec<RecoPick>, style: Style, limit: usize) -> Vec<TrimmedPick> {
    if v.len() <= limit {
        return Vec::new();
    }
    v.split_off(limit)
        .into_iter()
        .enumerate()
        .map(|(i, p)| TrimmedPick {
            stock_code: p.stock_code,
            stock_name: p.stock_name,
            style,
            confidence: p.confidence as i32,
            rank: (i + 1) as i32,
        })
        .collect()
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

    /// **口径统一回归**（2026-10-01）：荐股胜率 = logit 合成 **∘** 时间折算 ——
    /// 第二步必须与分析链**同一函数**（`harness::indicators::snr_confidence`）逐位一致。
    #[test]
    fn blend_win_rate_composes_logit_then_the_shared_time_scaling() {
        let anchor = 28u32;
        let prior: f64 = 0.62;
        let score: f64 = 0.7;
        let sensitivity: f64 = 1.0;
        // 独立复算第一步（logit 合成），再断言第二步 == snr_confidence(该后验)
        let posterior = {
            let logit = (prior / (1.0 - prior)).ln() + 2.0 * sensitivity * (score - 0.5);
            1.0 / (1.0 + (-logit).exp())
        };
        for h in [2u32, 5, 28, 90] {
            let got = blend_win_rate(Some(prior), score, sensitivity, h, anchor).expect("有先验");
            let want =
                axagent_harness::indicators::snr_confidence(posterior, h as usize, anchor as usize);
            assert!(
                (got - want).abs() < 1e-12,
                "h={h} 必须与分析链同函数折算：got={got} want={want}"
            );
        }
        // 锚定档不改：h == anchor 时第二步是恒等 ⇒ 结果就是纯 logit 合成值
        let mid = blend_win_rate(Some(prior), score, sensitivity, anchor, anchor).expect("有先验");
        assert!((mid - posterior).abs() < 1e-12, "锚定档不得被折算改变，实得 {mid}");
    }

    /// **本次修复的直接回归**：同一先验 + 同一评分，档间必须拉开差距。
    /// 原实现（只做 logit 合成、无时间折算）下四档**逐位相同**。
    #[test]
    fn time_scaling_makes_tiers_differ_in_both_directions() {
        let anchor = 28;
        // 看多边缘（posterior 0.60 ⇒ score 中性、先验即后验）：长档放大、短档收缩
        let bull_short = blend_win_rate(Some(0.60), 0.5, 1.0, 5, anchor).expect("有先验");
        let bull_long = blend_win_rate(Some(0.60), 0.5, 1.0, 90, anchor).expect("有先验");
        assert!(bull_short < 0.60, "短档应向 0.5 收缩，实得 {bull_short}");
        assert!(bull_long > 0.60, "长档应放大，实得 {bull_long}");
        // 看空边缘（posterior 0.40）：**对称**放大 ⇒ 长档更悲观
        let bear_short = blend_win_rate(Some(0.40), 0.5, 1.0, 5, anchor).expect("有先验");
        let bear_long = blend_win_rate(Some(0.40), 0.5, 1.0, 90, anchor).expect("有先验");
        assert!(bear_short > 0.40, "短档应向 0.5 收缩，实得 {bear_short}");
        assert!(
            bear_long < bear_short,
            "看空边缘在长档必须更悲观：long={bear_long} short={bear_short}"
        );
        assert!(
            (bear_long - bear_short).abs() > 0.05,
            "档间必须拉开可观测差距（原实现为 0）：{}",
            (bear_long - bear_short).abs()
        );
    }

    /// 退化输入：先验缺失 / 无效 ⇒ `None`（调用方退回纯评分并标 `absent`，不假装合成过）；
    /// `sensitivity=0` ⇒ 评分不起作用，只剩先验。
    #[test]
    fn blend_win_rate_declines_invalid_priors_and_honours_zero_sensitivity() {
        assert!(blend_win_rate(None, 0.9, 1.0, 28, 28).is_none(), "先验缺失");
        assert!(blend_win_rate(Some(0.0), 0.9, 1.0, 28, 28).is_none(), "先验 0 无效");
        assert!(blend_win_rate(Some(1.0), 0.9, 1.0, 28, 28).is_none(), "先验 1 无效");
        assert!(blend_win_rate(Some(0.5), f64::NAN, 1.0, 28, 28).is_none(), "评分 NaN 无效");
        let s0 = blend_win_rate(Some(0.60), 0.99, 0.0, 28, 28).expect("s=0 仍应有值");
        assert!((s0 - 0.60).abs() < 1e-12, "s=0 必须只承认先验，实得 {s0}");
    }

    /// **口径统一回归（2026-10-02）**：趋势智选链的候选评分必须经同一套合成，
    /// 不得再原样落库（实测形态：候选评分 78 直接入库，`priorSource` 硬编码 `absent`）。
    #[test]
    fn candidate_score_is_folded_into_the_shared_win_rate_scale() {
        let anchor = 28u32;
        // 实测形态：prior=0.4643 / source=pooled（修复后的真实注入值）
        let prior_row = serde_json::json!({"prior": 0.4643, "samples": 0, "source": "pooled"});
        let (mid, src) = candidate_score_to_win_rate(78, Some(&prior_row), 1.0, 28, anchor);
        assert_eq!(src, "pooled", "priorSource 必须如实反映来源，不得再硬编码 absent");
        assert!(mid < 78, "候选评分必须经先验合成后再落库，实得 {mid}");
        assert!(mid > 46, "但不应被压到分析链的 46 附近（证据源本就不同），实得 {mid}");
        // 时间折算必须存在：同一评分在 long 档与 mid 档必须不同（原实现两档逐位相同）
        let (long, _) = candidate_score_to_win_rate(78, Some(&prior_row), 1.0, 90, anchor);
        assert_ne!(long, mid, "long 档必须与 mid 档不同（时间折算），实得 {long} vs {mid}");
        assert!(long > mid, "看多评分在长档应被放大，实得 {long} vs {mid}");
    }

    /// 退化：无先验 ⇒ 原样评分 + `absent`（不假装合成过）；档位在表里缺失同理。
    #[test]
    fn candidate_score_without_prior_degrades_to_absent() {
        let (c, src) = candidate_score_to_win_rate(78, None, 1.0, 28, 28);
        assert_eq!((c, src.as_str()), (78, "absent"));
        let table = serde_json::json!({"mid": {"prior": 0.4643, "source": "pooled"}});
        let (c2, src2) = candidate_score_to_win_rate(78, table.get("long"), 1.0, 90, 28);
        assert_eq!((c2, src2.as_str()), (78, "absent"), "缺该档也不得借用别档先验");
    }
}
