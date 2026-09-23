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
    DecisionPerformanceSample, HitrateStats, compute_reflection_stats,
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
        let mut action = None;
        let mut horizon = None;
        let mut alpha_pct = None;
        let mut target_reached = None;

        // 路径 1：反思行批匹配 → original_analysis_id 精确取维度
        if let Some(r) = ref_by_batch.get(&(sp.stock_code.clone(), sp.created_at)) {
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

        // 路径 2：时间窗匹配（分析回测行 / 反思批匹配失败兜底）
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
}
