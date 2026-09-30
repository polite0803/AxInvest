//! 荐股链的逐档预测力度量（Phase R-E）—— **只报数，不回写权重**
//!
//! ## 为什么单独一层
//!
//! 「哪个风格在哪一档真有信息量」此前只能靠人写死：策略的自适应权重没有任何写入方
//! （见 `scoring.rs` 的说明），质量校准也只有胜率没有力度量。本层补上**可证伪**的那一维：
//! 把落库的预测（`decision_validations.confidence`）与其实现收益
//! （`entry_price` → `t_plus_n_price`）按 `(风格, 档位)` 配对，算 **rank IC（Spearman）**。
//!
//! 口径全部复用分析链已实现的基元，**不再写第四份**（禁区 12）：
//! - 秩相关：[`axagent_harness::indicators::spearman_rank_ic`]
//! - 样本门槛：[`crate::reflection_stats::IC_MIN_SAMPLE`]（= 8，**高于**命中率的 5；
//!   相关系数是四阶矩量，n=5 的抽样噪声就能把 ρ 推到 ±0.8）
//! - 半衰期拟合：[`crate::reflection_stats::signal_half_life_days`]（三条拒绝条件在它内部）
//!
//! 缺席必须分句：`icStatus ∈ {ok, insufficient_ic_samples, degenerate_variance}`；
//! 没配对到样本的格**也要出现**（`cells` 是 风格×档位 的全集），否则前端无法区分
//! 「这格没样本」与「这格没实现」。

use serde::Serialize;

use crate::recommender::types::Period;

use crate::reflection_stats::{signal_half_life_days, IC_MIN_SAMPLE};

/// 一条已实现的荐股预测样本。
#[derive(Debug, Clone)]
pub struct RecoIcRow {
    pub style: String,
    pub period: String,
    /// 预测时的置信度 0-100
    pub confidence: f64,
    /// 实现收益（%），按该档验证窗口从 entry → t_plus_n 算出
    pub realized_return_pct: f64,
    /// 该样本的验证窗口（天），半衰期拟合的横轴
    pub holding_days: u32,
}

/// 一个 (风格, 档位) 格的度量结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoIcCell {
    pub style: String,
    pub period: String,
    /// rank IC（Spearman ρ），不可得为 `None`
    pub rank_ic: Option<f64>,
    /// 参与计算的成对样本数
    pub samples: usize,
    /// `ok` | `insufficient_ic_samples` | `degenerate_variance`
    pub ic_status: &'static str,
    /// 该格用到的持有期（天，取样本众数）——半衰期拟合的横轴
    pub holding_days: Option<u32>,
}

/// 单风格的汇总（含该风格自己的预测半衰期）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleIc {
    pub style: String,
    pub cells: Vec<RecoIcCell>,
    /// 该风格可用档数 ≥ 3 且 IC 随持有期衰减时才可得
    pub half_life_days: Option<f64>,
    /// 半衰期不可得的原因：`ok` | `insufficient_tiers` | `ic_rises_with_horizon` | `poor_fit`
    pub half_life_status: &'static str,
}

/// 全量汇总。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoIcStats {
    pub styles: Vec<StyleIc>,
    pub total_cells: usize,
    pub usable_cells: usize,
    pub total_samples: usize,
    /// 24 格契约视图（`contract_cells()`）——行的存在性与不成立理由的**唯一来源**。
    /// 与 `styles`（观测）分列：某格没有样本 ≠ 该格不存在。
    pub matrix: Vec<MatrixCellView>,
    /// 闭环视图（Phase D）——命令层用 `reco_loop::compute_loop_cell_weights` 现算填充；
    /// `aggregate_reco_ic` 自身只给默认（shadow / 空格），使本函数的纯契约不变。
    #[serde(rename = "loop")]
    pub loop_view: crate::recommender::reco_loop::RecoLoopView,
}

/// 秩相关不可得时区分「样本不够」与「某一侧全是同一个值（方差退化）」。
fn classify(pairs: &[(f64, f64)]) -> (&'static str, Option<f64>) {
    if pairs.len() < IC_MIN_SAMPLE {
        return ("insufficient_ic_samples", None);
    }
    let ic = axagent_harness::indicators::spearman_rank_ic(pairs);
    match ic {
        Some(v) => ("ok", Some(v)),
        None => ("degenerate_variance", None),
    }
}

/// 契约视图的一格 —— 来自 `style_matrix`，**与有没有数据无关**：没出票的格也照样在表里，
/// 并带上「为什么不成立」的理由码。
///
/// 为什么单列而不是塞进 `RecoIcCell`：`RecoIcCell` 是**观测**（有样本才存在），
/// 契约是**声明**（24 格恒在）。把两者混成一格，就会出现「按设计不做的格」与
/// 「还没跑出样本的格」在 UI 上同为空白 —— 这正是 `style_matrix` 立项时要消灭的形态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixCellView {
    /// 矩阵名目（`serenity`，不是落库的 `bottleneck`）
    pub style: String,
    pub period: &'static str,
    /// 该格是否出票
    pub active: bool,
    /// `cell_is_active` | 不成立理由码 | `cell_not_in_matrix`
    pub reason_code: &'static str,
    /// 出票但已知档-因子错配的理由码（`None` = 无错配声明）
    pub misfit_code: Option<&'static str>,
    /// 该风格在落库/回测里的实际写法（`serenity` ⇒ `["serenity","bottleneck"]`）
    pub db_styles: Vec<&'static str>,
}

/// 24 格契约视图（行=风格×列=档位）。前端矩阵的行/格集合必须由本表驱动，
/// 不得再自带一份风格清单（旧 `RecoStrategyMatrix.tsx` 的 4 项 `STYLE_KEYS` 让
/// watchlist 与 serenity 两行连声明位置都没有）。
pub fn contract_cells() -> Vec<MatrixCellView> {
    let mut out = Vec::new();
    for style in crate::recommender::style_matrix::style_keys() {
        for period in Period::ALL.iter() {
            let reason = crate::recommender::style_matrix::reason_code_by_key(style, *period);
            out.push(MatrixCellView {
                style: style.to_string(),
                period: period.as_str(),
                active: reason == "cell_is_active",
                misfit_code: crate::recommender::style_matrix::misfit_reason(style, *period),
                reason_code: reason,
                db_styles: crate::recommender::style_matrix::db_style_aliases(style),
            });
        }
    }
    out
}

/// 聚合：按 (风格, 档位) 分组算 rank IC，再按风格拟合预测半衰期。
pub fn aggregate_reco_ic(rows: &[RecoIcRow]) -> RecoIcStats {
    let mut styles: Vec<String> = rows.iter().map(|r| r.style.clone()).collect();
    styles.sort();
    styles.dedup();
    // 档位遍历序 = `Period::ALL`（短→长）。若按 period 字符串字典序排，JSON 里会出现
    // long,mid,short,ultra_short 这种与档位无关的顺序，下游极易误当成「由长到短」。
    let periods: Vec<String> = Period::ALL
        .iter()
        .map(|p| p.as_str().to_string())
        .filter(|name| rows.iter().any(|r| &r.period == name))
        .collect();

    let mut out_styles = Vec::new();
    let mut total_cells = 0usize;
    let mut usable_cells = 0usize;

    for style in styles {
        let mut cells = Vec::new();
        let mut fit_points: Vec<(f64, f64)> = Vec::new();
        for period in &periods {
            let group: Vec<&RecoIcRow> =
                rows.iter().filter(|r| r.style == *style && &r.period == period).collect();
            if group.is_empty() {
                continue;
            }
            total_cells += 1;
            let pairs: Vec<(f64, f64)> =
                group.iter().map(|r| (r.confidence, r.realized_return_pct)).collect();
            let (status, ic) = classify(&pairs);
            let holding = group.iter().map(|r| r.holding_days).max().unwrap_or(0);
            if status == "ok" {
                usable_cells += 1;
                if let (Some(v), h) = (ic, holding) {
                    if v > 0.0 {
                        fit_points.push((h as f64, v));
                    }
                }
            }
            cells.push(RecoIcCell {
                style: style.clone(),
                period: period.clone(),
                rank_ic: ic.map(|v| (v * 10_000.0).round() / 10_000.0),
                samples: group.len(),
                ic_status: status,
                holding_days: if holding > 0 { Some(holding) } else { None },
            });
        }
        // 半衰期的三条拒绝条件在 `signal_half_life_days` 内部（有效档 < 3 / 斜率 ≥ 0 / R² < 0.5）
        let (hl, hl_status) = match signal_half_life_days(&fit_points) {
            Some(v) => (Some(v), "ok"),
            None => (
                None,
                if fit_points.len() < 3 {
                    "insufficient_tiers"
                } else {
                    "rejected_fit"
                },
            ),
        };
        out_styles.push(StyleIc { style, cells, half_life_days: hl, half_life_status: hl_status });
    }

    RecoIcStats {
        styles: out_styles,
        total_cells,
        usable_cells,
        total_samples: rows.len(),
        matrix: contract_cells(),
        loop_view: crate::recommender::reco_loop::RecoLoopView::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(style: &str, period: &str, conf: f64, ret: f64, days: u32) -> RecoIcRow {
        RecoIcRow {
            style: style.to_string(),
            period: period.to_string(),
            confidence: conf,
            realized_return_pct: ret,
            holding_days: days,
        }
    }

    /// 完美单调 ⇒ ρ = 1；样本刚好够门槛 ⇒ 报数而不是"样本不足"。
    #[test]
    fn perfect_monotone_pairing_gives_unit_ic() {
        let rows: Vec<RecoIcRow> = (0..IC_MIN_SAMPLE)
            .map(|i| row("trend", "short", 50.0 + i as f64, i as f64, 5))
            .collect();
        let stats = aggregate_reco_ic(&rows);
        let cell = &stats.styles[0].cells[0];
        assert_eq!(cell.ic_status, "ok");
        assert!((cell.rank_ic.unwrap() - 1.0).abs() < 1e-9, "{:?}", cell.rank_ic);
        assert_eq!(cell.samples, IC_MIN_SAMPLE);
    }

    /// 只差一个样本 ⇒ 必须报 `insufficient_ic_samples`，不许给一个看起来能用的 ρ。
    #[test]
    fn one_sample_short_reports_insufficient_not_a_number() {
        let rows: Vec<RecoIcRow> = (0..IC_MIN_SAMPLE - 1)
            .map(|i| row("trend", "short", 50.0 + i as f64, i as f64, 5))
            .collect();
        let stats = aggregate_reco_ic(&rows);
        let cell = &stats.styles[0].cells[0];
        assert_eq!(cell.ic_status, "insufficient_ic_samples");
        assert!(cell.rank_ic.is_none(), "样本不足时不得产出 IC");
    }

    /// 一侧全是同一个值（方差退化）⇒ 与「样本不足」分句，两者要做的事不同。
    #[test]
    fn degenerate_variance_is_its_own_status() {
        let rows: Vec<RecoIcRow> =
            (0..IC_MIN_SAMPLE).map(|i| row("trend", "mid", 60.0, i as f64, 28)).collect();
        let cell = &aggregate_reco_ic(&rows).styles[0].cells[0];
        assert_eq!(cell.ic_status, "degenerate_variance");
        assert!(cell.rank_ic.is_none());
    }

    /// 半衰期：三个可用档、IC 随持有期**衰减** ⇒ 出数；只剩两档 ⇒ `insufficient_tiers`。
    #[test]
    fn half_life_needs_three_usable_tiers_and_decay() {
        // 每档做 k 次相邻逆序 ⇒ Spearman ρ 依次为 1、0.976、0.952（随持有期衰减）
        let mut rows: Vec<RecoIcRow> = Vec::new();
        for (k, (period, days)) in [("short", 5u32), ("mid", 28), ("long", 90)].iter().enumerate() {
            let n = IC_MIN_SAMPLE;
            let mut ys: Vec<f64> = (0..n).map(|i| i as f64).collect();
            for j in 0..k {
                ys.swap(n - 1 - 2 * j, n - 2 - 2 * j);
            }
            for (i, y) in ys.iter().enumerate() {
                rows.push(row("trend", period, i as f64, *y, *days));
            }
        }
        let stats = aggregate_reco_ic(&rows);
        assert_eq!(stats.usable_cells, 3, "三档都应可用: {stats:?}");
        let ic_of = |want: &str| -> f64 {
            stats.styles[0]
                .cells
                .iter()
                .find(|c| c.period == want)
                .unwrap_or_else(|| {
                    panic!(
                        "缺档位 {want}: {:?}",
                        stats.styles[0].cells.iter().map(|c| &c.period).collect::<Vec<_>>()
                    )
                })
                .rank_ic
                .expect("该档应有 IC")
        };
        assert!(
            ic_of("short") > ic_of("mid") && ic_of("mid") > ic_of("long"),
            "IC 应随持有期衰减: short={}",
            ic_of("short")
        );
        assert_eq!(
            stats.styles[0].cells.iter().map(|c| c.period.as_str()).collect::<Vec<_>>(),
            vec!["short", "mid", "long"],
            "cells 必须按档位序（短→长），不是 period 字符串字典序"
        );
        assert!(
            stats.styles[0].half_life_days.is_some(),
            "应拟合出半衰期，实际状态 {}",
            stats.styles[0].half_life_status
        );

        let two_tier: Vec<RecoIcRow> = rows.into_iter().filter(|r| r.period != "long").collect();
        let s2 = aggregate_reco_ic(&two_tier);
        assert_eq!(s2.styles[0].half_life_status, "insufficient_tiers");
        assert!(s2.styles[0].half_life_days.is_none());
    }

    /// IPC 契约（禁区 13）：新 DTO 跨边界必须 camelCase，缺注解不会有任何编译或
    /// `check:serde` 报红，前端按 camelCase 读只会恒为 undefined（命中率卡的前车之鉴）。
    #[test]
    fn reco_ic_dto_serializes_camel_case_for_ipc() {
        let rows = vec![row("trend", "short", 55.0, 1.0, 5)];
        let value = serde_json::to_value(aggregate_reco_ic(&rows)).unwrap();
        let top = value.as_object().unwrap();
        for key in ["styles", "totalCells", "usableCells", "totalSamples", "matrix", "loop"] {
            assert!(top.contains_key(key), "RecoIcStats 缺 camelCase 键 {key}: {top:?}");
        }
        assert!(!top.contains_key("total_cells"), "snake_case 不得回到 IPC 边界");
        let cell = value["styles"][0]["cells"].as_array().unwrap()[0].as_object().unwrap().clone();
        for key in ["rankIc", "icStatus", "holdingDays", "style", "period", "samples"] {
            assert!(cell.contains_key(key), "RecoIcCell 缺 camelCase 键 {key}: {cell:?}");
        }
        assert!(!cell.contains_key("rank_ic"));
        assert!(!cell.contains_key("ic_status"));
        let view = value["matrix"].as_array().unwrap()[0].as_object().unwrap().clone();
        for key in ["reasonCode", "misfitCode", "dbStyles", "active", "style", "period"] {
            assert!(view.contains_key(key), "MatrixCellView 缺 camelCase 键 {key}: {view:?}");
        }
        assert!(!view.contains_key("reason_code"));
    }

    /// 契约视图必须**恒出 24 格**（6 风格 × 4 档），与有没有回测样本无关：
    /// 「按设计不做」的格也得在表里，否则前端只能渲染成空白。
    #[test]
    fn contract_cells_cover_the_matrix_regardless_of_samples() {
        let cells = contract_cells();
        assert_eq!(cells.len(), 24, "6 风格 × 4 档位 = 24 格");
        // 空表（无任何回测样本）时契约视图照样齐 —— 行的存在性不依赖观测
        let stats = aggregate_reco_ic(&[]);
        assert_eq!(stats.matrix.len(), 24);
        let serenity: Vec<&MatrixCellView> =
            cells.iter().filter(|c| c.style == "serenity").collect();
        assert_eq!(serenity.len(), 4, "趋势智选必须有完整四列，短/超短以理由码声明不适用");
        for c in &serenity {
            match c.period {
                "mid" | "long" => {
                    assert!(c.active, "serenity×{} 应按设计出票", c.period);
                    assert_eq!(c.reason_code, "cell_is_active");
                },
                _ => {
                    assert!(!c.active, "serenity×{} 不应出票", c.period);
                    assert_eq!(c.reason_code, "serenity_needs_week_or_longer_realization");
                },
            }
        }
    }

    /// 一个名目两写（工作流链 `serenity` / 策略链 `bottleneck`）必须在契约里带上别名，
    /// 否则「趋势智选」行永远匹配不到自己的统计，格子又是空白 —— 只是换了个原因的空白。
    #[test]
    fn serenity_row_carries_its_db_style_aliases() {
        let cells = contract_cells();
        let view = cells.iter().find(|c| c.style == "serenity" && c.period == "mid").unwrap();
        assert_eq!(view.db_styles, vec!["serenity", "bottleneck"]);
        let trend = cells.iter().find(|c| c.style == "trend" && c.period == "mid").unwrap();
        assert_eq!(trend.db_styles, vec!["trend"]);
    }

    /// 错配声明（出票但该方法论在该尺度兑现不了）单独成字段，不与「不成立」混同。
    #[test]
    fn misfit_is_a_third_state_not_absence() {
        let cells = contract_cells();
        let value_short =
            cells.iter().find(|c| c.style == "value" && c.period == "ultra_short").unwrap();
        assert!(value_short.active, "value×超短 按用户裁定保留出票");
        assert_eq!(
            value_short.misfit_code,
            Some("valuation_needs_weeks_to_realize_kept_by_user_decision")
        );
        let plain = cells.iter().find(|c| c.style == "trend" && c.period == "short").unwrap();
        assert_eq!(plain.misfit_code, None);
    }
}
