//! 逐档先验的分层收缩（四周期科学化 Phase C）
//!
//! ## 为什么需要它
//!
//! `portfolio-mgr.rhai` 的 `prior` 只有一个数（由市场状态派生：牛 0.55 / 震荡 0.50 / 熊 0.45），
//! 四档共用 ⇒ 这是诊断 E1 的根：**四个持有期共享同一个「上涨先验」**。
//! 但反思链路早就按档统计了方向命中率（`reflection_stats::HitrateStats::by_horizon`，
//! 样本 < `MIN_SAMPLE` 时为 `None`），只是没有任何地方回头用它。
//!
//! ## 做法（经验贝叶斯 / James–Stein 两点收缩）
//!
//! ```text
//! prior_h = (n_h · p_h + κ · p_pool) / (n_h + κ)
//! ```
//!
//! - `p_h` = 该档自己的方向命中率，`n_h` = 该档成熟样本数；
//! - `p_pool` = 全档合并命中率（缺 ⇒ 0.5 中性，并标 `neutral_default`）；
//! - `κ` = 收缩强度（**进设置面板、可被反思建议覆盖**；κ→0 等于完全采信该档自身，
//!   κ→∞ 等于退回共用先验 —— 也就是旧行为，故 κ 的取值必须可见、可调，不能拍死）。
//!
//! 两端都写死都是错的：全用共用先验 = E1；全用该档自身 = 短线档几十条样本就把长线口径带偏。
//! 收缩结果与它的两个输入一起透出（`samples` / `source`），让「这个先验有几分来自本档」可被审计。

use crate::reflection_stats::{HitrateStats, MIN_SAMPLE};
use axagent_harness::Period;

/// 收缩强度 κ 的**默认值**（设置面板缺省、`seed_variables` 缺省、hooks 兜底都用它 ——
/// 三处若各写一份，改一处忘一处就会让「面板显示 20 / 实际用别的数」这种漂移无从发现）。
pub const DEFAULT_KAPPA: f64 = 20.0;

/// 收缩后可见性要求：先验不得取 0 或 1（那会把后验钉死，让证据失去作用）。
/// 取 [0.05, 0.95] 与拉普拉斯平滑同族，是**防饱和**而不是对收益的假设。
const PRIOR_FLOOR: f64 = 0.05;
const PRIOR_CEIL: f64 = 0.95;

/// 一个档的先验估计（连同它的来源，供生效快照与反思归因）。
#[derive(Debug, Clone, PartialEq)]
pub struct PriorEstimate {
    pub prior: f64,
    pub samples: usize,
    /// `own` 完全采信本档 | `shrunk` 与本档收缩 | `pooled` 本档样本不足 | `neutral_default` 连合并样本都没有
    pub source: &'static str,
}

/// 两点收缩。`own` 为 `None` 表示该档样本不足（`reflection_stats` 已按 `MIN_SAMPLE` 判过）。
pub fn shrink_prior(
    own: Option<f64>,
    own_samples: usize,
    pooled: Option<f64>,
    kappa: f64,
) -> PriorEstimate {
    let (Some(p_h), Some(p_pool)) = (own, pooled) else {
        // 该档无统计 **或** 连合并基准都没有 ⇒ 只能退回中性/合并值，并如实标注。
        let (base, src) = match (own, pooled) {
            (Some(p), None) => (p, "own"),
            (None, Some(p)) => (p, "pooled"),
            (None, None) => (0.5, "neutral_default"),
            // 上面的 let-else 已排除「两者都有」，此臂只为穷尽性存在。
            (Some(p), Some(_)) => (p, "own"),
        };
        return PriorEstimate {
            prior: base.clamp(PRIOR_FLOOR, PRIOR_CEIL),
            samples: own_samples,
            source: src,
        };
    };
    if kappa <= 0.0 {
        return PriorEstimate {
            prior: p_h.clamp(PRIOR_FLOOR, PRIOR_CEIL),
            samples: own_samples,
            source: "own",
        };
    }
    let n = own_samples as f64;
    let shrunk = (n * p_h + kappa * p_pool) / (n + kappa);
    PriorEstimate {
        prior: shrunk.clamp(PRIOR_FLOOR, PRIOR_CEIL),
        samples: own_samples,
        source: "shrunk",
    }
}

/// 由命中率统计产出「四档先验表」，形状与脚本消费口径一致：
/// `{ultra_short: {prior, samples, source}, …}`。
///
/// 合并基准取 `HitrateStats` 的整体方向命中率（`overall` 字段名以结构为准 —— 这里用
/// 全档样本加权，避免「样本多的档主导」以外的第二种偏置）。
pub fn horizon_prior_map(stats: &HitrateStats, kappa: f64) -> serde_json::Value {
    let total: usize = stats.by_horizon.iter().map(|g| g.samples).sum();
    let pooled = stats
        .by_horizon
        .iter()
        .filter(|g| g.samples >= MIN_SAMPLE)
        .filter_map(|g| g.direction_hit_rate.map(|r| (r, g.samples)))
        .map(|(r, n)| r * n as f64)
        .sum::<f64>();
    let pooled_rate = if total > 0 {
        let weighted_n: usize =
            stats.by_horizon.iter().filter(|g| g.samples >= MIN_SAMPLE).map(|g| g.samples).sum();
        if weighted_n > 0 {
            Some(pooled / weighted_n as f64)
        } else {
            None
        }
    } else {
        None
    };

    let mut obj = serde_json::Map::new();
    for p in Period::ALL {
        let key = p.as_str();
        let group = stats.by_horizon.iter().find(|g| g.key == key);
        let est = shrink_prior(
            group.and_then(|g| g.direction_hit_rate),
            group.map(|g| g.samples).unwrap_or(0),
            pooled_rate,
            kappa,
        );
        obj.insert(
            key.to_string(),
            serde_json::json!({
                "prior": (est.prior * 10_000.0).round() / 10_000.0,
                "samples": est.samples,
                "source": est.source,
            }),
        );
    }
    serde_json::Value::Object(obj)
}

#[cfg(test)]
mod horizon_prior_tests {
    use super::*;
    use crate::reflection_stats::HitrateGroup;

    fn group(key: &str, samples: usize, rate: Option<f64>) -> HitrateGroup {
        HitrateGroup {
            key: key.to_string(),
            samples,
            direction_hit_rate: rate,
            ..Default::default()
        }
    }

    /// 收缩公式逐项正确，且「样本越多越靠近本档」这一性质必须成立。
    #[test]
    fn shrinkage_follows_the_formula_and_moves_with_sample_size() {
        let small = shrink_prior(Some(0.70), 5, Some(0.50), 20.0);
        let big = shrink_prior(Some(0.70), 200, Some(0.50), 20.0);
        assert!(
            (small.prior - (5.0 * 0.70 + 20.0 * 0.50) / 25.0).abs() < 1e-12,
            "小样本收缩值算错: {}",
            small.prior
        );
        assert!(
            (big.prior - (200.0 * 0.70 + 20.0 * 0.50) / 220.0).abs() < 1e-12,
            "大样本收缩值算错: {}",
            big.prior
        );
        assert!(small.prior < big.prior, "样本越多应越靠近本档");
        assert_eq!((small.source, big.source), ("shrunk", "shrunk"));
    }

    /// κ→0 完全采信本档；κ 极大 ≈ 回到共用先验（即旧行为，必须可表达）。
    #[test]
    fn kappa_endpoints_reproduce_both_extremes() {
        let own = shrink_prior(Some(0.66), 40, Some(0.50), 0.0);
        assert_eq!((own.source, own.prior), ("own", 0.66));
        let near_pooled = shrink_prior(Some(0.66), 40, Some(0.50), 1e9);
        assert!(
            (near_pooled.prior - 0.50).abs() < 1e-6,
            "κ 极大应收敛到合并基准，实得 {}",
            near_pooled.prior
        );
    }

    /// 该档样本不足 ⇒ 用合并基准并**如实标注**（不得伪装成本档统计）。
    #[test]
    fn insufficient_sample_falls_back_to_pooled_and_says_so() {
        let est = shrink_prior(None, 2, Some(0.52), 20.0);
        assert_eq!((est.source, est.samples), ("pooled", 2));
        assert!((est.prior - 0.52).abs() < 1e-12);
    }

    /// 连合并基准都没有 ⇒ 中性 0.5 + `neutral_default`（新装库、零反思样本的真实形态）。
    #[test]
    fn no_statistics_at_all_yields_neutral_default() {
        let est = shrink_prior(None, 0, None, 20.0);
        assert_eq!((est.source, est.prior), ("neutral_default", 0.5));
    }

    /// 防饱和：命中率取到 0 / 1 也不能把后验钉死。
    #[test]
    fn prior_is_clamped_away_from_zero_and_one() {
        assert_eq!(shrink_prior(Some(1.0), 500, Some(0.5), 0.0).prior, PRIOR_CEIL);
        assert_eq!(shrink_prior(Some(0.0), 500, Some(0.5), 0.0).prior, PRIOR_FLOOR);
    }

    /// 四档齐表；缺档（该档从无样本）也要出现并标 pooled/neutral，不能整档消失。
    #[test]
    fn map_covers_all_four_periods() {
        let stats = HitrateStats {
            by_horizon: vec![group("short", 60, Some(0.60)), group("mid", 3, None)],
            ..Default::default()
        };
        let map = horizon_prior_map(&stats, 20.0);
        for p in Period::ALL {
            let row = map.get(p.as_str()).unwrap_or_else(|| panic!("先验表缺档 {}", p.as_str()));
            assert!(row.get("prior").is_some(), "档 {} 缺 prior", p.as_str());
            assert!(row.get("source").is_some(), "档 {} 缺 source", p.as_str());
        }
        assert_eq!(map["short"]["source"], "shrunk");
        assert_eq!(map["mid"]["source"], "pooled");
    }
}
