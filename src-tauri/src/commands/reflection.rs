//! 反思工作流 Tauri 命令
//!
//! 关键改进：
//! - 接受 `started_at_ms`（或同时 `started_at_ms`）使用真实时间戳
//! - 兼容 camelCase / snake_case 两种参数命名（Tauri 自动解析）
//! - 新增 9 个命令：config 读写、insight 反馈、删除、top/recent/high_confidence/prune/decay、export
//! - 所有错误用 `String` 包装返回，匹配 tauri 风格
//! - S1: `reflect_on_task` 仍返回 `Reflection`（前端协议不变），但内部拿到精确的 insight 列表
//! - L9: 新增 `export_insights` 命令，把当前 insight 列表导出为 JSONL

use crate::app_state::AppState;
use axagent_agent::insight_generator::InsightCategory;
use axagent_agent::reflector::{ReflectionConfig, TaskExecutionRecord};
use chrono::{TimeZone, Utc};
use tauri::State;

/// 兼容多命名入参（camelCase + snake_case）
#[derive(Debug, Default)]
struct ReflectArgs {
    task_id: String,
    task_description: String,
    success: bool,
    error: Option<String>,
    tools_used: Vec<String>,
    iterations: usize,
    duration_ms: u64,
    started_at_ms: Option<i64>,
}

fn pick_string(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

fn pick_u64(v: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    for k in keys {
        if let Some(n) = v.get(*k).and_then(|x| x.as_u64()) {
            return Some(n);
        }
    }
    None
}

fn pick_usize(v: &serde_json::Value, keys: &[&str]) -> Option<usize> {
    pick_u64(v, keys).map(|n| n as usize)
}

fn pick_bool(v: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    for k in keys {
        if let Some(b) = v.get(*k).and_then(|x| x.as_bool()) {
            return Some(b);
        }
    }
    None
}

fn pick_i64(v: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for k in keys {
        if let Some(n) = v.get(*k).and_then(|x| x.as_i64()) {
            return Some(n);
        }
    }
    None
}

fn pick_string_array(v: &serde_json::Value, keys: &[&str]) -> Vec<String> {
    for k in keys {
        if let Some(arr) = v.get(*k).and_then(|x| x.as_array()) {
            return arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
        }
    }
    Vec::new()
}

fn parse_args(args: serde_json::Value) -> ReflectArgs {
    ReflectArgs {
        task_id: pick_string(&args, &["taskId", "task_id"]).unwrap_or_default(),
        task_description: pick_string(&args, &["taskDescription", "task_description"])
            .unwrap_or_default(),
        success: pick_bool(&args, &["success"]).unwrap_or(false),
        error: pick_string(&args, &["error"]),
        tools_used: pick_string_array(&args, &["toolsUsed", "tools_used"]),
        iterations: pick_usize(&args, &["iterations"]).unwrap_or(0),
        duration_ms: pick_u64(&args, &["durationMs", "duration_ms"]).unwrap_or(0),
        started_at_ms: pick_i64(&args, &["startedAtMs", "started_at_ms"]),
    }
}

#[tauri::command]
pub async fn reflect_on_task(
    state: State<'_, AppState>,
    args: serde_json::Value,
) -> Result<axagent_agent::reflector::Reflection, String> {
    let a = parse_args(args);
    if a.task_id.is_empty() {
        return Err("task_id is required".to_string());
    }

    let now = Utc::now();
    let end_time = now;
    // 优先用 started_at_ms；没有则用 now - duration
    let start_time = match a.started_at_ms {
        Some(ms) if ms > 0 => Utc
            .timestamp_millis_opt(ms)
            .single()
            .unwrap_or(end_time - chrono::Duration::milliseconds(a.duration_ms as i64)),
        _ => end_time - chrono::Duration::milliseconds(a.duration_ms as i64),
    };

    let mut record = TaskExecutionRecord::new(
        a.task_id.clone(),
        a.task_description.clone(),
        start_time,
        end_time,
    )
    .with_tools(a.tools_used)
    .with_iterations(a.iterations);

    if a.success && a.error.is_none() {
        record = record.with_success(true);
    }
    if let Some(e) = a.error {
        record = record.with_error(e);
    }
    record.compute_duration();
    // 若计算后 duration 仍为 0 但传入 duration_ms 有效，回填
    if record.duration_ms == 0 && a.duration_ms > 0 {
        record.duration_ms = a.duration_ms;
    }

    // S1: reflect 现在返回 (Reflection, Vec<Insight>)
    // 公开 Tauri 命令的 schema 仍是 Reflection，避免破坏前端协议；
    // 自动触发链路（spawn_reflection_task）才会用到精确的 insight 列表。
    let (reflection, _new_insights) = state.reflector.reflect(&record).await;
    Ok(reflection)
}

#[tauri::command]
pub async fn get_reflection_history(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_agent::reflector::Reflection>, String> {
    Ok(state.reflector.get_history().await)
}

#[tauri::command]
pub async fn clear_reflection_history(state: State<'_, AppState>) -> Result<(), String> {
    state.reflector.clear_history().await;
    Ok(())
}

#[tauri::command]
pub async fn get_reflection_insights(
    state: State<'_, AppState>,
    category: Option<String>,
) -> Result<Vec<axagent_agent::insight_generator::Insight>, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.get_insights_by_category_str(category.as_deref()).await)
}

#[tauri::command]
pub async fn search_reflection_insights(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<axagent_agent::insight_generator::Insight>, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.search_insights(&query).await)
}

#[tauri::command]
pub async fn get_reflection_insight_stats(
    state: State<'_, AppState>,
) -> Result<axagent_agent::insight_generator::InsightStats, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.get_stats().await)
}

// ── 新增命令 ──

#[tauri::command]
pub async fn get_reflection_config(state: State<'_, AppState>) -> Result<ReflectionConfig, String> {
    Ok(state.reflector.get_config().await)
}

#[tauri::command]
pub async fn update_reflection_config(
    state: State<'_, AppState>,
    config: ReflectionConfig,
) -> Result<(), String> {
    state.reflector.update_config(config).await;
    Ok(())
}

#[tauri::command]
pub async fn delete_insight(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.delete_insight(&id).await)
}

#[tauri::command]
pub async fn record_insight_feedback(
    state: State<'_, AppState>,
    id: String,
    useful: bool,
) -> Result<Option<axagent_agent::insight_generator::Insight>, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.record_feedback(&id, useful).await)
}

#[tauri::command]
pub async fn get_top_insights(
    state: State<'_, AppState>,
    n: Option<usize>,
) -> Result<Vec<axagent_agent::insight_generator::Insight>, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.get_top_insights(n.unwrap_or(10)).await)
}

#[tauri::command]
pub async fn get_recent_insights(
    state: State<'_, AppState>,
    n: Option<usize>,
) -> Result<Vec<axagent_agent::insight_generator::Insight>, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.get_recent_insights(n.unwrap_or(20)).await)
}

#[tauri::command]
pub async fn get_high_confidence_insights(
    state: State<'_, AppState>,
    threshold: Option<f32>,
) -> Result<Vec<axagent_agent::insight_generator::Insight>, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig
        .get_high_confidence_insights(threshold.unwrap_or(0.6))
        .await)
}

#[tauri::command]
pub async fn prune_stale_insights(
    state: State<'_, AppState>,
    min_confidence: Option<f32>,
) -> Result<usize, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.prune_stale(min_confidence.unwrap_or(0.2)).await)
}

#[tauri::command]
pub async fn decay_stale_insights(state: State<'_, AppState>) -> Result<usize, String> {
    let ig = state.reflector.get_insight_generator();
    Ok(ig.decay_stale().await)
}

#[tauri::command]
pub async fn get_insights_by_category(
    state: State<'_, AppState>,
    category: String,
) -> Result<Vec<axagent_agent::insight_generator::Insight>, String> {
    let ig = state.reflector.get_insight_generator();
    let parsed = match category.as_str() {
        "error_pattern" => InsightCategory::ErrorPattern,
        "success_pattern" => InsightCategory::SuccessPattern,
        "optimization" => InsightCategory::Optimization,
        "knowledge" => InsightCategory::Knowledge,
        "workflow" => InsightCategory::Workflow,
        "tool_usage" => InsightCategory::ToolUsage,
        _ => return Err(format!("unknown category: {category}")),
    };
    Ok(ig.get_insights_by_category(parsed).await)
}

// ── L9: 导出所有 insight 为 JSONL 字符串（前端可下载为 .jsonl 文件） ──

#[tauri::command]
pub async fn export_insights(state: State<'_, AppState>) -> Result<String, String> {
    let ig = state.reflector.get_insight_generator();
    let insights = ig.get_insights().await;
    let mut buf = String::new();
    for ins in &insights {
        match serde_json::to_string(ins) {
            Ok(s) => {
                buf.push_str(&s);
                buf.push('\n');
            },
            Err(e) => {
                tracing::warn!("[export_insights] skip {}: {}", ins.id, e);
            },
        }
    }
    Ok(buf)
}

// ── P3 桥接：把 Reflector 的 insight 推入 AppState 的 LearningInsightSystem ──
// 真正的桥接逻辑位于 `commands::agent::bridge_reflection_to_insight_system_with`，
// 因为自动触发需要反射器与 insight_system 的原始 Arc 句柄（避免再 wrap 一层）。
// 类别映射的唯一来源在 `commands::agent::map_category_to_trajectory`（M8）。
