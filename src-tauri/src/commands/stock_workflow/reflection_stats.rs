// SPDX-License-Identifier: AGPL-3.0-only

//! M3（PLAN-stock-decision-hitrate-validation）：命中率统计只读命令 `reflection_stats`。
//!
//! 跨三表聚合：
//! - `strategy_performance`：主表（was_correct 确定性判定 + 复盘 win/loss 写回、return_pct）
//! - `stock_reflections`：反思行（alpha_return、blackboard_snapshot 里递归找 target_reached）
//! - `stock_analyses`：维度（decision_action / decision_time_horizon）
//!
//! join 口径：
//! 1. **反思行**（strategy_id="reflection_verdict"，reflection.rs Gap1 写入）：
//!    (stock_code, created_at) 精确匹配 stock_reflections **同批记录**（Gap1 与反思落库同刻）
//!    → 经 original_analysis_id 精确取 stock_analyses 维度；alpha / target_reached 取反思行。
//! 2. **分析回测行**（复盘 cron 写回，strategy_id=trend/value/...）：stock_code + decision_at
//!    与 stock_analyses.created_at 时间窗（±3 天）匹配最近分析行取维度。
//!
//! 诚实性：target_reached 未独立落库（反思时仅在内存 MarketSnapshot），故从
//! blackboard_snapshot 递归查找；找不到 → None（前端按「样本不足/暂无数据」展示，不伪报）。

use std::collections::HashMap;

use tauri::State;

use axagent_analysis_engine::reflection_stats::{
    DecisionPerformanceSample, HitrateStats, SampleDataSource, compute_reflection_stats,
};
use axagent_entities::{stock_analyses, stock_reflections, strategy_performance};
use sea_orm::EntityTrait;

use crate::AppState;

/// 时间窗匹配阈值：decision_at 与分析 created_at 允许的最大偏差（3 天）
const JOIN_WINDOW_MS: i64 = 3 * 86_400_000;

/// 在 JSON 值中递归查找指定 key 的布尔值（blackboard_snapshot 结构不确定，
/// 兼容「一层嵌套 / 多层嵌套 / 数组内」三种形态；找不到 → None）。
fn find_bool_recursive(value: &serde_json::Value, key: &str) -> Option<bool> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(b) = map.get(key).and_then(|v| v.as_bool()) {
                return Some(b);
            }
            for v in map.values() {
                if let Some(b) = find_bool_recursive(v, key) {
                    return Some(b);
                }
            }
            None
        },
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| find_bool_recursive(v, key)),
        _ => None,
    }
}

/// 从四周期反思 JSON（`stock_reflections.horizon_results_json`）展开统计样本。
///
/// 展开口径（PLAN-stock-reflection-four-horizon 批次 4 第 11 条）：
/// - 仅 `mature` / `legacy` 周期可成样本；`immature` / `unavailable` **不进分母**。
/// - 还要求 `evaluation.wasCorrect ∈ {0,1}`（中性档 = 不可判定）且 `market.returnPct` 为有限数，
///   否则该周期整体跳过 —— 不伪造 0% 收益，也不把「未到期」写成「判错」。
/// - 已成熟周期的 action / 收益 / alpha / 目标价全部取自该周期自身条目，不与其他周期串线。
///
/// 返回 `None` 表示该 JSON 不是四周期对象（无法解析 / 非对象），调用方回退 legacy 单周期字段。
fn expand_horizon_samples(json: &str) -> Option<Vec<DecisionPerformanceSample>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let map = value.as_object()?;
    let mut samples = Vec::new();
    for (horizon, entry) in map {
        let Some(entry) = entry.as_object() else {
            continue; // schemaVersion 等标量键
        };
        let status = entry.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status != "mature" && status != "legacy" {
            continue;
        }
        let was_correct = entry
            .get("evaluation")
            .and_then(|e| e.get("wasCorrect"))
            .and_then(|v| v.as_i64())
            .and_then(|v| i32::try_from(v).ok())
            .filter(|v| *v == 0 || *v == 1);
        let Some(was_correct) = was_correct else { continue };
        let market = entry.get("market");
        let return_pct = market
            .and_then(|m| m.get("returnPct"))
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite());
        let Some(return_pct) = return_pct else { continue };

        samples.push(DecisionPerformanceSample {
            action: entry
                .get("decision")
                .and_then(|d| d.get("action"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            horizon: Some(horizon.clone()),
            was_correct,
            return_pct,
            alpha_pct: market
                .and_then(|m| m.get("alphaPct"))
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite()),
            target_reached: market.and_then(|m| m.get("targetReached")).and_then(|v| v.as_bool()),
            data_source: if status == "legacy" {
                SampleDataSource::Legacy
            } else {
                SampleDataSource::Reflection
            },
        });
    }
    Some(samples)
}

/// 按 (stock_code, decision_at) 在分析行中找时间窗内最近的一条，返回其维度。
fn nearest_analysis_dimensions<'a>(
    analyses: &'a [stock_analyses::Model],
    stock_code: &str,
    decision_at: i64,
) -> Option<&'a stock_analyses::Model> {
    analyses
        .iter()
        .filter(|a| a.stock_code == stock_code)
        .filter(|a| (a.created_at - decision_at).abs() <= JOIN_WINDOW_MS)
        .min_by_key(|a| (a.created_at - decision_at).abs())
}

/// 命中率统计（只读）：聚合 strategy_performance 全部已结算样本。
#[tauri::command]
pub async fn reflection_stats(state: State<'_, AppState>) -> Result<HitrateStats, String> {
    let db = state.harness.db();

    let sp_rows = strategy_performance::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 strategy_performance 失败: {e}"))?;
    let ref_rows = stock_reflections::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 stock_reflections 失败: {e}"))?;
    let ana_rows = stock_analyses::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("读取 stock_analyses 失败: {e}"))?;

    // 索引 1：stock_analyses.id → Model
    let ana_by_id: HashMap<&str, &stock_analyses::Model> =
        ana_rows.iter().map(|a| (a.id.as_str(), a)).collect();
    // 索引 2：反思行按 (stock_code, created_at) 批匹配（同批写入的时间戳相同）
    let mut ref_by_batch: HashMap<(String, i64), &stock_reflections::Model> = HashMap::new();
    for r in &ref_rows {
        ref_by_batch.entry((r.stock_code.clone(), r.created_at)).or_insert(r);
    }

    let mut samples = Vec::with_capacity(sp_rows.len());
    for sp in &sp_rows {
        let matched = ref_by_batch.get(&(sp.stock_code.clone(), sp.created_at));

        // 路径 1：反思行有四周期结果 JSON → 按周期独立展开（每周期一条样本）
        if let Some(expanded) =
            matched.and_then(|r| r.horizon_results_json.as_deref()).and_then(expand_horizon_samples)
        {
            samples.extend(expanded);
            continue;
        }

        // 路径 2：旧记录无四周期 JSON → 回退 legacy 主周期字段，并带 legacy 数据源标识
        let mut action = None;
        let mut horizon = None;
        let mut alpha_pct = None;
        let mut target_reached = None;

        if let Some(r) = matched {
            alpha_pct = r.alpha_return;
            target_reached = r
                .blackboard_snapshot
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .and_then(|v| find_bool_recursive(&v, "target_reached"));
            // `original_analysis_id` 是**非空 String**（entity `stock_reflections.rs:14`），
            // 不是 Option ⇒ 直接取 `&str` 查索引，没有 Option 链可 `.and_then`。
            if let Some(a) = ana_by_id.get(r.original_analysis_id.as_str()) {
                action = a.decision_action.clone();
                horizon = a.decision_time_horizon.clone();
            }
        }

        // 时间窗匹配（分析回测行 / 反思批匹配失败兜底）
        if action.is_none() {
            if let Some(a) = nearest_analysis_dimensions(&ana_rows, &sp.stock_code, sp.decision_at)
            {
                action = a.decision_action.clone();
                horizon = a.decision_time_horizon.clone();
            }
        }

        samples.push(DecisionPerformanceSample {
            action,
            horizon,
            was_correct: sp.was_correct,
            return_pct: sp.return_pct,
            alpha_pct,
            target_reached,
            data_source: SampleDataSource::Legacy,
        });
    }

    Ok(compute_reflection_stats(&samples))
}

// ── 单元测试 ── 追加在文件末尾（防 clippy::items_after_test_module）
#[cfg(test)]
mod reflection_stats_command_tests {
    use super::*;

    #[test]
    fn find_bool_recursive_walks_nested_and_arrays() {
        let v: serde_json::Value = serde_json::json!({
            "report": {
                "items": [
                    { "name": "a" },
                    { "snapshot": { "target_reached": true } }
                ]
            }
        });
        assert_eq!(find_bool_recursive(&v, "target_reached"), Some(true));
        assert_eq!(find_bool_recursive(&v, "target_price"), None);
        let flat: serde_json::Value = serde_json::json!({ "target_reached": false });
        assert_eq!(find_bool_recursive(&flat, "target_reached"), Some(false));
        let non_bool: serde_json::Value = serde_json::json!({ "target_reached": "yes" });
        assert_eq!(find_bool_recursive(&non_bool, "target_reached"), None);
    }

    /// 四周期 JSON：ultra_short 成熟、short 未到期、mid 无行情、long 缺失（null）、
    /// 以及一个标量 `schemaVersion` 键（不是周期条目）。
    fn four_horizon_json() -> String {
        serde_json::json!({
            "ultra_short": {
                "status": "mature",
                "decision": { "action": "买入" },
                "market": { "returnPct": 3.1, "alphaPct": 1.4, "targetReached": true },
                "evaluation": { "wasCorrect": 1 }
            },
            "short": {
                "status": "immature",
                "decision": { "action": "买入" },
                "market": { "returnPct": 0.4, "alphaPct": 0.1, "targetReached": false },
                "evaluation": { "wasCorrect": null }
            },
            "mid": {
                "status": "unavailable",
                "decision": { "action": "卖出" },
                "market": null,
                "evaluation": null
            },
            "long": null,
            "schemaVersion": 1
        })
        .to_string()
    }

    #[test]
    fn expand_only_mature_and_legacy_horizons() {
        let samples = expand_horizon_samples(&four_horizon_json()).unwrap();
        // 未到期 / 无行情 / 缺失周期一律不进样本，也不进命中率分母
        assert_eq!(samples.len(), 1);
        let s = &samples[0];
        assert_eq!(s.horizon.as_deref(), Some("ultra_short"));
        assert_eq!(s.action.as_deref(), Some("买入"));
        assert_eq!(s.was_correct, 1);
        assert_eq!(s.return_pct, 3.1);
        assert_eq!(s.alpha_pct, Some(1.4));
        assert_eq!(s.target_reached, Some(true));
        assert_eq!(s.data_source, SampleDataSource::Reflection);
    }

    #[test]
    fn expand_skips_neutral_action_and_missing_return() {
        let json = serde_json::json!({
            "ultra_short": {
                "status": "mature",
                "decision": { "action": "持有" },
                "market": { "returnPct": 1.0 },
                "evaluation": { "wasCorrect": null }
            },
            "short": {
                "status": "mature",
                "decision": { "action": "买入" },
                "market": { "returnPct": null },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        // 中性档不可判定、收益缺失不可伪报 0% → 两者都不成样本
        assert!(expand_horizon_samples(&json).unwrap().is_empty());
    }

    #[test]
    fn expand_marks_legacy_entries_and_rejects_non_object_json() {
        let json = serde_json::json!({
            "short": {
                "status": "legacy",
                "decision": { "action": "卖出" },
                "market": { "returnPct": -2.5, "targetReached": false },
                "evaluation": { "wasCorrect": 1 }
            }
        })
        .to_string();
        let samples = expand_horizon_samples(&json).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].data_source, SampleDataSource::Legacy);
        assert_eq!(samples[0].alpha_pct, None);
        assert_eq!(samples[0].target_reached, Some(false));

        assert!(expand_horizon_samples("[]").is_none());
        assert!(expand_horizon_samples("not-json").is_none());
    }
}
