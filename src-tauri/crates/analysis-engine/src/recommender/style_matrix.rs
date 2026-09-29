//! 风格 × 档位可用性矩阵（Phase R-B）——「哪个风格在哪一档成立」的唯一契约表
//!
//! ## 为什么需要它
//!
//! 此前这张表**只存在于 `mod.rs` 的 `match period` 里**（哪些 `XxxStrategy::tier()` 被塞进 vec），
//! 理由只写在注释（「超跌反弹至少需要中线」「v1 不做长线超跌」）。后果有三条，都是本轮查出的：
//!
//! ① **前端各自硬编码空格**：`RecoStrategyMatrix.tsx` 里写死 `reversion × long` 特例，
//!   而 `reversion × ultra_short` 同样不成立却没有对应处理 ⇒ 一格有解释、一格没解释；
//! ② **档-因子错配无人校验**：`value`（PE/PB 估值）在超短档（2 天）出票 —— 估值在 2 天尺度上
//!   没有可兑现路径，这件事没有任何地方登记过（用户裁定**保留该档**，但必须**显式声明局限**）；
//! ③ **缺格分不清是「按设计不做」还是「实现漏了」**。
//!
//! 本表把 24 格逐格登记：`None` = 该风格在该档出票；`Some(reason_code)` = 按设计不做，
//! 且**必须给机器可读的理由码**（前端据此出文案，不得再自己硬编码哪格为空）。
//! 门：`check-reco-horizon-parity.mjs` 的 e 段要求 24 格齐全；本文件的
//! `matrix_covers_all_24_cells` 与 `matrix_matches_strategy_selection` 锁「表 ⇔ 实际策略选择」一致。

use crate::recommender::types::{Period, Style};

/// 一格：`(风格, 档位, 不成立理由码)`。`None` = 该格出票。
pub type Cell = (&'static str, &'static str, Option<&'static str>);

/// 只有理由文案、没有「不成立」状态的一格（用于档-因子错配声明）。
pub type Cell2 = (&'static str, &'static str, &'static str);

/// 全部 24 格（6 风格 × 4 档位）。顺序固定为「风格分组 × `Period::ALL`」，便于 diff 与门禁解析。
pub const MATRIX: [Cell; 24] = [
    // ── trend 趋势跟踪：四档全开（短期动量 → 长期趋势，期限结构上本就跨尺度）
    ("trend", "ultra_short", None),
    ("trend", "short", None),
    ("trend", "mid", None),
    ("trend", "long", None),
    // ── value 价值低估：四档全开，但超短档是**已知错配**（见 MISFIT_DECLARATIONS）
    ("value", "ultra_short", None),
    ("value", "short", None),
    ("value", "mid", None),
    ("value", "long", None),
    // ── capital 资金驱动：四档全开
    ("capital", "ultra_short", None),
    ("capital", "short", None),
    ("capital", "mid", None),
    ("capital", "long", None),
    // ── reversion 超跌反弹：仅短/中档
    ("reversion", "ultra_short", Some("oversold_rebound_needs_at_least_mid_horizon")),
    ("reversion", "short", None),
    ("reversion", "mid", None),
    ("reversion", "long", Some("oversold_rebound_not_applied_to_long_horizon")),
    // ── watchlist 候选池兜底：四档全开（它不出技术信号，只声明「候选」）
    ("watchlist", "ultra_short", None),
    ("watchlist", "short", None),
    ("watchlist", "mid", None),
    ("watchlist", "long", None),
    // ── serenity 趋势智选（瓶颈/政策/业绩…）：仅中/长档 —— 其证据是产业链与政策叙事，
    //     兑现尺度以周/月计，2 天与 1 周内无可验证路径
    ("serenity", "ultra_short", Some("serenity_needs_week_or_longer_realization")),
    ("serenity", "short", Some("serenity_needs_week_or_longer_realization")),
    ("serenity", "mid", None),
    ("serenity", "long", None),
];

/// 出票但**已知档-因子错配**的格：不删（用户裁定），必须在产出与 UI 上显式声明局限。
///
/// 为什么单列：矩阵只能表达「做 / 不做」，而「做但这个方法论在这个尺度上兑现不了」
/// 是第三种状态；把它混进 `None` 就等于继续假装它没问题。
pub const MISFIT_DECLARATIONS: [(&str, &str, &str); 1] =
    [("value", "ultra_short", "valuation_needs_weeks_to_realize_kept_by_user_decision")];

/// 该档实际应选的风格（`mod.rs` 的策略选择必须由本表驱动，两处不得各列一份清单）。
pub fn styles_for(period: Period) -> Vec<Style> {
    let tier = period.as_str();
    MATRIX
        .iter()
        .filter(|(_, t, absent)| *t == tier && absent.is_none())
        .filter_map(|(s, _, _)| style_from_str(s))
        .collect()
}

/// 该格按设计不成立的理由码（`None` = 出票）。查不到该格 ⇒ `Some("cell_not_in_matrix")`，
/// 让「实现漏格」也走显式声明而不是静默当作不做。
pub fn absence_reason(style: Style, period: Period) -> Option<&'static str> {
    let s = style.as_str();
    let t = period.as_str();
    MATRIX
        .iter()
        .find(|(ms, mt, _)| *ms == s && *mt == t)
        .map(|(_, _, reason)| reason.unwrap_or("cell_is_active"))
        .or(Some("cell_not_in_matrix"))
}

/// 该格是否出票。
pub fn is_active(style: Style, period: Period) -> bool {
    let s = style.as_str();
    let t = period.as_str();
    MATRIX.iter().any(|(ms, mt, absent)| *ms == s && *mt == t && absent.is_none())
}

fn style_from_str(s: &str) -> Option<Style> {
    match s {
        "trend" => Some(Style::Trend),
        "value" => Some(Style::Value),
        "capital" => Some(Style::Capital),
        "reversion" => Some(Style::Reversion),
        "watchlist" => Some(Style::Watchlist),
        // serenity 在工作流里叫 `serenity`、落库按子类型（bottleneck…）序列化：
        // 矩阵只用 `serenity` 一个名目，具体子风格由策略内部决定。
        "serenity" => Some(Style::Bottleneck),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 24 格齐全，且 (风格,档位) 组合不重复 —— 漏一格或写重一格都算红。
    #[test]
    fn matrix_covers_all_24_cells_without_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for (s, t, _) in MATRIX.iter() {
            assert!(seen.insert((*s, *t)), "矩阵重复格 {s}×{t}");
        }
        assert_eq!(MATRIX.len(), 24, "6 风格 × 4 档位 = 24 格");
        for t in ["ultra_short", "short", "mid", "long"] {
            for s in ["trend", "value", "capital", "reversion", "watchlist", "serenity"] {
                assert!(seen.contains(&(s, t)), "矩阵缺格 {s}×{t}");
            }
        }
    }

    /// 不成立的格必须带理由码；成立的格必须是 `None`（不许写 `Some("…")` 却仍出票）。
    #[test]
    fn every_inactive_cell_carries_a_reason() {
        for (s, t, reason) in MATRIX.iter() {
            if reason.is_some() {
                assert!(reason.unwrap().chars().next().is_some(), "{s}×{t} 声明不成立但理由码为空");
            }
        }
    }

    /// 错配声明只能指向**出票**的格（否则该从矩阵里删掉，而不是声明错配）。
    #[test]
    fn misfit_declarations_point_at_active_cells() {
        for (s, t, _) in MISFIT_DECLARATIONS.iter() {
            let cell = MATRIX.iter().find(|(ms, mt, _)| ms == s && mt == t);
            let reason = cell.map(|(_, _, r)| *r).expect("错配声明的格不在矩阵里");
            assert!(reason.is_none(), "{s}×{t} 已声明不成立，不应再登记为错配");
        }
    }

    /// 每档出票风格数量对得上（防「表改了、策略选择没改」的漂移；实际选择由
    /// `matrix_matches_strategy_selection` 在 `mod.rs` 侧再锁一次）。
    #[test]
    fn per_tier_active_counts() {
        assert_eq!(styles_for(Period::UltraShort).len(), 4, "超短: trend/value/capital/watchlist");
        assert_eq!(styles_for(Period::Short).len(), 5, "短线再加 reversion");
        assert_eq!(styles_for(Period::Mid).len(), 6, "中线全开");
        assert_eq!(styles_for(Period::Long).len(), 5, "长线无 reversion");
    }
}
