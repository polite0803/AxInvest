// SPDX-License-Identifier: AGPL-3.0-only

//! P5 费用 / Token 配额与用量 Tauri 命令（docs/bot-integration/P5/IPC-P5 §1.1/§2）。

use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_cost_quota as quota_repo;
use axagent_dao::repo::bot_department as dept_repo;
use axagent_dao::repo::bot_usage_ledger as ledger_repo;
use axagent_harness::types::{
    BotCostQuota, BotCostQuotaInput, BotCostQuotaScope, BotUsageEntry, BotUsageFilter,
    BotUsageSummary,
};
use tauri::State;

use crate::AppState;
use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

fn map_err(err: axagent_harness::core_error::AxAgentError) -> String {
    match err {
        axagent_harness::core_error::AxAgentError::NotFound(detail) => {
            ErrorResponse::new(bot_err::NOT_FOUND)
                .with_category(ErrorCategory::Validation)
                .with_detail(detail)
                .to_string()
        },
        other => ErrorResponse::from_error(other, ErrorCategory::Unrecoverable).to_string(),
    }
}

fn scope_of(scope: &str) -> Result<BotCostQuotaScope, String> {
    match scope {
        "bot" => Ok(BotCostQuotaScope::Bot),
        "department" => Ok(BotCostQuotaScope::Department),
        other => Err(ErrorResponse::new(bot_err::COST_QUOTA_INVALID)
            .with_category(ErrorCategory::Validation)
            .with_param("field", "scope")
            .with_detail(format!("scope 必须是 bot/department，实得 {other}"))
            .to_string()),
    }
}

async fn ensure_scope_target(
    db: &axagent_dao::DatabaseConnection,
    scope: &str,
    scope_id: &str,
) -> Result<(), String> {
    match scope {
        "bot" => bot_repo::get_bot(db, scope_id).await.map(|_| ()).map_err(|e| match e {
            axagent_harness::core_error::AxAgentError::NotFound(detail) => {
                ErrorResponse::new(bot_err::NOT_FOUND)
                    .with_category(ErrorCategory::Validation)
                    .with_detail(detail)
                    .to_string()
            },
            other => map_err(other),
        }),
        "department" => {
            dept_repo::get_department(db, scope_id).await.map(|_| ()).map_err(|e| match e {
                axagent_harness::core_error::AxAgentError::NotFound(detail) => {
                    ErrorResponse::new(bot_err::DEPARTMENT_NOT_FOUND)
                        .with_category(ErrorCategory::Validation)
                        .with_detail(detail)
                        .to_string()
                },
                other => map_err(other),
            })
        },
        _ => Err(ErrorResponse::new(bot_err::COST_QUOTA_INVALID)
            .with_category(ErrorCategory::Validation)
            .with_param("field", "scope")
            .with_detail("scope 必须是 bot/department")
            .to_string()),
    }
}

#[tauri::command]
pub async fn get_bot_cost_quota(
    state: State<'_, AppState>,
    scope: String,
    scope_id: String,
) -> Result<BotCostQuota, String> {
    let scope_enum = scope_of(&scope)?;
    let db = state.harness.db();
    ensure_scope_target(db, &scope, &scope_id).await?;
    let row = quota_repo::get_by_scope(db, &scope, &scope_id).await.map_err(map_err)?;
    Ok(row.unwrap_or_else(|| BotCostQuota::defaults(scope_enum, &scope_id)))
}

#[tauri::command]
pub async fn set_bot_cost_quota(
    state: State<'_, AppState>,
    input: BotCostQuotaInput,
) -> Result<BotCostQuota, String> {
    crate::bot_cost_quota::validate_quota_input(&input)?;
    let db = state.harness.db();
    ensure_scope_target(db, input.scope.as_str(), &input.scope_id).await?;
    // 全 0 = 不限，归一为删行 + 返回默认 DTO（IPC-P5 §1.3）。
    if input.max_total_tokens <= 0 && input.max_cost_usd <= 0.0 {
        quota_repo::delete_by_scope(db, input.scope.as_str(), &input.scope_id)
            .await
            .map_err(map_err)?;
        return Ok(BotCostQuota::defaults(input.scope, &input.scope_id));
    }
    quota_repo::upsert(db, &input).await.map_err(map_err)
}

#[tauri::command]
pub async fn reset_bot_cost_quota(
    state: State<'_, AppState>,
    scope: String,
    scope_id: String,
) -> Result<(), String> {
    scope_of(&scope)?;
    let db = state.harness.db();
    ensure_scope_target(db, &scope, &scope_id).await?;
    quota_repo::delete_by_scope(db, &scope, &scope_id).await.map_err(map_err)?;
    Ok(())
}

#[tauri::command]
pub async fn list_bot_usage(
    state: State<'_, AppState>,
    filter: BotUsageFilter,
) -> Result<Vec<BotUsageEntry>, String> {
    let db = state.harness.db();
    ledger_repo::list(db, &filter).await.map_err(map_err)
}

#[tauri::command]
pub async fn get_bot_usage_summary(
    state: State<'_, AppState>,
    scope: String,
    scope_id: String,
    window_hours: Option<i64>,
) -> Result<BotUsageSummary, String> {
    scope_of(&scope)?;
    let db = state.harness.db();
    ensure_scope_target(db, &scope, &scope_id).await?;
    crate::bot_cost_quota::usage_summary(db, &scope, &scope_id, window_hours).await
}
