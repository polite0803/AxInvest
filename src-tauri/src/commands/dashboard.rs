// SPDX-License-Identifier: AGPL-3.0-only

use crate::AppState;
use axagent_agent_macro::agent_command;
use axagent_plugins::types::DashboardPluginInfo;
use sea_orm::entity::prelude::*;
use serde::Serialize;
use tauri::State;

/// 仪表盘面板清单 —— dashboard 合流后为 PluginManager 的只读投影
/// （见 `PLAN-plugin-gap-closure.md` §3）。
///
/// 真相源是 `PluginManifest.dashboard_panels`；安装 / 启停 / 卸载等操作**直接复用
/// `plugin_*` 命令**（它们自带护照索引同步与 UI 贡献撤销，此处不再造第二套生命周期）。
#[agent_command(domain = dashboard, safety = Safe, call_mode = StateOnly, description = "列出仪表盘插件")]
#[tauri::command]
pub async fn dashboard_list_plugins(
    state: State<'_, AppState>,
) -> Result<Vec<DashboardPluginInfo>, String> {
    let plugin_manager = state.plugin_manager.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let manager = plugin_manager.blocking_read();
        manager.list_dashboard_plugins().map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
    })
    .await
    .map_err(|e| format!("dashboard list task panicked: {e}"))?
}

#[agent_command(domain = dashboard, safety = Safe, call_mode = StateInput, description = "打开插件安装目录")]
#[tauri::command]
pub async fn dashboard_open_plugins_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // 合流后仪表盘插件与其余插件同根：统一指向 PluginManager 的安装目录。
    let dir = state.plugin_manager.read().await.install_root();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create plugins dir: {}", e))?;
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(&dir).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub total_conversations: i64,
    pub total_messages: i64,
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_tokens: i64,
    pub total_agent_sessions: i64,
    pub completed_agent_sessions: i64,
    pub failed_agent_sessions: i64,
    pub total_agent_tokens: i64,
    pub total_cost_usd: f64,
    pub total_tool_calls: i64,
    /// 今日（本地时区）消息数
    pub today_messages: i64,
    /// 今日（本地时区）输入 token 数
    pub today_prompt_tokens: i64,
    /// 今日（本地时区）输出 token 数
    pub today_completion_tokens: i64,
    /// 今日（本地时区）总 token 数 = today_prompt_tokens + today_completion_tokens
    pub today_tokens: i64,
}

#[agent_command(domain = dashboard, safety = Safe, call_mode = StateOnly, description = "获取仪表盘统计数据")]
#[tauri::command]
pub async fn get_dashboard_stats(state: State<'_, AppState>) -> Result<DashboardStats, String> {
    let db = state.harness.db();
    // 2026-07-31 修复：DB 已切 PostgreSQL（db_config.json db_type=postgres）。
    // 原原生 SQL 全部用 DatabaseBackend::Sqlite + `?` 占位符，在 PG 上必然报错
    // （占位符风格不匹配），导致仪表盘统计静默为 0/报错。按 backend 分支统一处理。
    let is_pg = db.get_database_backend() == sea_orm::DbBackend::Postgres;
    let backend = if is_pg {
        sea_orm::DbBackend::Postgres
    } else {
        sea_orm::DbBackend::Sqlite
    };

    // 会话总数
    let total_conversations =
        axagent_entities::conversations::Entity::find().count(db).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    // 消息聚合查询：COUNT + SUM tokens，避免全表加载到内存
    let msg_stats = db
        .query_all_raw(sea_orm::Statement::from_sql_and_values(
            backend,
            "SELECT \
             COUNT(*) as total_messages, \
             COALESCE(SUM(prompt_tokens), 0) as total_prompt_tokens, \
             COALESCE(SUM(completion_tokens), 0) as total_completion_tokens, \
             COALESCE(SUM(token_count), 0) as total_token_count \
             FROM messages WHERE is_active = 1",
            vec![],
        ))
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let first_row = msg_stats.first();
    let total_messages: i64 =
        first_row.and_then(|r| r.try_get("", "total_messages").ok()).unwrap_or(0);
    let total_prompt_tokens: i64 =
        first_row.and_then(|r| r.try_get("", "total_prompt_tokens").ok()).unwrap_or(0);
    let total_completion_tokens: i64 =
        first_row.and_then(|r| r.try_get("", "total_completion_tokens").ok()).unwrap_or(0);
    let total_token_count: i64 =
        first_row.and_then(|r| r.try_get("", "total_token_count").ok()).unwrap_or(0);

    let total_tokens = if total_prompt_tokens > 0 || total_completion_tokens > 0 {
        total_prompt_tokens + total_completion_tokens
    } else {
        total_token_count
    };

    // 今日（本地时区）消息统计：messages.created_at 是毫秒时间戳
    let today_start_millis = axagent_harness::util_fns::today_start_local_ts() * 1000;
    // 占位符按方言：PG 用 $1，SQLite 用 ?
    let today_msg_sql = if is_pg {
        "SELECT \
         COUNT(*) as today_messages, \
         COALESCE(SUM(prompt_tokens), 0) as today_prompt_tokens, \
         COALESCE(SUM(completion_tokens), 0) as today_completion_tokens \
         FROM messages WHERE is_active = 1 AND created_at >= $1"
    } else {
        "SELECT \
         COUNT(*) as today_messages, \
         COALESCE(SUM(prompt_tokens), 0) as today_prompt_tokens, \
         COALESCE(SUM(completion_tokens), 0) as today_completion_tokens \
         FROM messages WHERE is_active = 1 AND created_at >= ?"
    };
    let today_msg_stats = db
        .query_all_raw(sea_orm::Statement::from_sql_and_values(
            backend,
            today_msg_sql,
            vec![today_start_millis.into()],
        ))
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let today_row = today_msg_stats.first();
    let today_messages: i64 =
        today_row.and_then(|r| r.try_get("", "today_messages").ok()).unwrap_or(0);
    let today_prompt_tokens: i64 =
        today_row.and_then(|r| r.try_get("", "today_prompt_tokens").ok()).unwrap_or(0);
    let today_completion_tokens: i64 =
        today_row.and_then(|r| r.try_get("", "today_completion_tokens").ok()).unwrap_or(0);
    let today_tokens = today_prompt_tokens + today_completion_tokens;

    // 智能体会话聚合查询
    let session_stats = db
        .query_all_raw(sea_orm::Statement::from_sql_and_values(
            backend,
            "SELECT \
             COUNT(*) as total_sessions, \
             COALESCE(SUM(total_tokens), 0) as total_agent_tokens, \
             COALESCE(SUM(total_cost_usd), 0.0) as total_cost_usd, \
             COALESCE(SUM(CASE WHEN runtime_status = 'completed' THEN 1 ELSE 0 END), 0) as completed, \
             COALESCE(SUM(CASE WHEN runtime_status = 'failed' THEN 1 ELSE 0 END), 0) as failed \
             FROM agent_sessions",
            vec![],
        ))
        .await
        .map_err(|e| String::from(crate::commands::error::ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)))?;

    let session_row = session_stats.first();
    let total_agent_sessions: i64 =
        session_row.and_then(|r| r.try_get("", "total_sessions").ok()).unwrap_or(0);
    let total_agent_tokens: i64 =
        session_row.and_then(|r| r.try_get("", "total_agent_tokens").ok()).unwrap_or(0);
    let total_cost_usd: f64 =
        session_row.and_then(|r| r.try_get("", "total_cost_usd").ok()).unwrap_or(0.0);
    let completed_agent_sessions: i64 =
        session_row.and_then(|r| r.try_get("", "completed").ok()).unwrap_or(0);
    let failed_agent_sessions: i64 =
        session_row.and_then(|r| r.try_get("", "failed").ok()).unwrap_or(0);

    // 工具调用统计
    let total_tool_calls =
        axagent_entities::tool_executions::Entity::find().count(db).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    Ok(DashboardStats {
        total_conversations: total_conversations as i64,
        total_messages,
        total_prompt_tokens,
        total_completion_tokens,
        total_tokens,
        total_agent_sessions,
        completed_agent_sessions,
        failed_agent_sessions,
        total_agent_tokens,
        total_cost_usd,
        total_tool_calls: total_tool_calls as i64,
        today_messages,
        today_prompt_tokens,
        today_completion_tokens,
        today_tokens,
    })
}

/// 按提供商统计网关使用量。成本数据由 agent_sessions 跟踪，此处仅返回用量。
#[agent_command(domain = dashboard, safety = Safe, call_mode = StateOnly, description = "按提供商统计用量")]
#[tauri::command]
pub async fn get_cost_by_provider(
    state: State<'_, AppState>,
) -> Result<Vec<axagent_harness::types::CostByProvider>, String> {
    let db = state.harness.db();
    // 2026-07-31 修复：backend 标记按方言（无占位符，SQL 本身 PG 兼容）
    let backend = if db.get_database_backend() == sea_orm::DbBackend::Postgres {
        sea_orm::DbBackend::Postgres
    } else {
        sea_orm::DbBackend::Sqlite
    };

    let rows = db
        .query_all_raw(sea_orm::Statement::from_sql_and_values(
            backend,
            "SELECT gu.provider_id, \
             COUNT(*) as request_count, \
             COALESCE(SUM(gu.request_tokens + gu.response_tokens), 0) as token_count \
             FROM gateway_usage gu \
             GROUP BY gu.provider_id \
             ORDER BY token_count DESC",
            vec![],
        ))
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let results: Vec<axagent_harness::types::CostByProvider> = rows
        .iter()
        .map(|r| axagent_harness::types::CostByProvider {
            provider_id: r.try_get("", "provider_id").unwrap_or_default(),
            request_count: r.try_get("", "request_count").unwrap_or(0),
            token_count: r.try_get("", "token_count").unwrap_or(0),
            cost_usd: 0.0,
        })
        .collect();

    Ok(results)
}

#[agent_command(domain = dashboard, safety = Safe, call_mode = StateInput, description = "获取使用量趋势")]
#[tauri::command]
pub async fn get_usage_trend(
    state: State<'_, AppState>,
    days: Option<u32>,
) -> Result<Vec<axagent_harness::types::DailyUsage>, String> {
    let db = state.harness.db();
    let days = days.unwrap_or(30);
    axagent_dao::repo::message::get_daily_message_usage(db, days).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}
