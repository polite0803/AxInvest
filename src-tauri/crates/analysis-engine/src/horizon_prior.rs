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

/// 合并基准 `p_pool` **自身**的样本量收缩伪计数（把中性 0.5 当作这么多次观测）。
///
/// 为什么需要（2026-10-01 实证）：`p_pool` 是全档合并方向命中率，会被 [`shrink_prior`]
/// 的 `pooled` 分支**直接当作**「该档无自身样本时的上涨先验」。而本机实测该值 = **0.375**，
/// 其样本只有**个位数**（`strategy_performance` 8 行、含周期展开的反思行 1 行）。
/// 个位数样本的命中率同时混着「这套体系有没有 edge」与「市场基准涨跌」两件事，
/// 不是上涨概率的可靠估计 ⇒ 原样采用会把**四档先验全部钉死在同一个 0.375**
/// （比市况先验 0.45 低 7.5pt，四档后验因此系统性低于主链）。
/// 这正是本文件开头警告的 E1（「四个持有期共享同一个上涨先验」）以新数值复发。
///
/// 取 20 与 [`DEFAULT_KAPPA`] 同量级：样本 < 20 时中性值占主导，样本充足后实测命中率
/// 才逐步接管。收缩强度本身仍由 κ 统一控制，本常量只管**基准自身的可靠性**。
pub const POOL_PSEUDO_COUNT: f64 = 20.0;

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
    // 全档合并命中率：只统计**样本达 MIN_SAMPLE** 的档，按样本量加权
    //（「样本多的档主导」以外的第二种偏置不再引入）。
    let mut pooled_num = 0.0;
    let mut pooled_n: usize = 0;
    for g in &stats.by_horizon {
        if g.samples < MIN_SAMPLE {
            continue;
        }
        if let Some(r) = g.direction_hit_rate {
            pooled_num += r * g.samples as f64;
            pooled_n += g.samples;
        }
    }
    // 再把该基准**按自身样本量向中性 0.5 收缩**（见 POOL_PSEUDO_COUNT 的实证理由）：
    // 个位数样本的命中率不得被当作确定的上涨先验去钉死四档。
    let pooled_rate = if pooled_n > 0 {
        let raw = pooled_num / pooled_n as f64;
        let n = pooled_n as f64;
        let k = POOL_PSEUDO_COUNT;
        Some((n * raw + k * 0.5) / (n + k))
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

/// 从 DB 取统计并产出逐档先验表 —— 取数与收缩的**唯一实现**。
///
/// 为什么需要它（2026-10-02）：本函数原先只存在于命令层
///（`commands/stock_analysis.rs` 的 `reco_horizon_prior`），而趋势智选链
///（`commands/stock_workflow/serenity.rs`）也要吃同一份先验 ⇒ 只能跨 commands 模块调用，
/// 撞上分层护栏 `commands-no-sibling-call`。正确解法是把**取数**下沉到本层，命令层两侧
/// 都只做「读出 κ 后调本函数」的薄包装 —— 既消除跨模块调用，也保证收缩口径只有一份（禁区 12）。
///
/// `kappa` 由调用方从变量表读出（`horizon_prior_kappa`，可被反思建议覆盖）：
/// 本层不读变量表 —— 它不该知道模板变量的存在。
pub async fn horizon_prior_from_db(
    db: &sea_orm::DatabaseConnection,
    kappa: f64,
) -> Option<serde_json::Value> {
    let stats = crate::reflection_stats::build_hitrate_stats(db).await.ok()?;
    Some(horizon_prior_map(&stats, kappa))
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

    /// **根因 2 的回归测试**（2026-10-01 实证）。
    ///
    /// 实测形态：`mid` 档**自身样本为 0**（`source = "pooled"`）而合并基准 `p_pool` = 0.375
    /// 只来自个位数样本 ⇒ 原实现把 0.375 原样当作该档上涨先验，四档被同一个值钉死
    ///（比市况先验 0.45 低 7.5pt，四档后验系统性低于主链）。
    ///
    /// ⚠ 构造必须让被测档**自身样本不足**（`direction_hit_rate = None`）才会走 `pooled`
    /// 分支 —— 若给该档 ≥ `MIN_SAMPLE` 条样本，走的是 `shrunk`，测不到本修复的作用点。
    #[test]
    fn small_pool_is_shrunk_toward_neutral() {
        let stats = HitrateStats {
            by_horizon: vec![
                group("short", 8, Some(0.375)), // 唯一达 MIN_SAMPLE 的档 ⇒ 合并基准的来源
                group("mid", 2, None),          // 自身样本不足 ⇒ own = None ⇒ 吃合并基准
            ],
            ..Default::default()
        };
        let map = horizon_prior_map(&stats, 20.0);
        let expected = (8.0 * 0.375 + POOL_PSEUDO_COUNT * 0.5) / (8.0 + POOL_PSEUDO_COUNT);
        let got = map["mid"]["prior"].as_f64().expect("mid 档必须有 prior");
        assert_eq!(map["mid"]["source"], "pooled", "该档样本不足 ⇒ 来源须如实标为合并基准");
        assert!(
            (got - expected).abs() < 1e-4,
            "合并基准必须按样本量向中性收缩：期望 {expected}，实得 {got}"
        );
        assert!(got > 0.375, "不得原样采用小样本命中率（那会把四档先验钉死在 0.375）");
        assert!(got < 0.5, "收缩不得越过中性值");
    }

    /// 反向：基准来源的样本充足时几乎不被收缩 —— 收缩不得把有效统计一起抹平。
    #[test]
    fn large_pool_is_barely_shrunk() {
        let stats = HitrateStats {
            by_horizon: vec![group("short", 1000, Some(0.60)), group("mid", 2, None)],
            ..Default::default()
        };
        let map = horizon_prior_map(&stats, 20.0);
        let expected = (1000.0 * 0.60 + POOL_PSEUDO_COUNT * 0.5) / (1000.0 + POOL_PSEUDO_COUNT);
        let got = map["mid"]["prior"].as_f64().expect("mid 档必须有 prior");
        assert!((got - expected).abs() < 1e-4, "大样本应几乎不被收缩：期望 {expected}，实得 {got}");
        assert!(got > 0.59, "1000 样本时收缩幅度应小于 1pt，实得 {got}");
    }
}
