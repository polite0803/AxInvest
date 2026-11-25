// SPDX-License-Identifier: AGPL-3.0-only

//! P4 部门运行配额 Tauri 命令（docs/bot-integration/P4/IPC-P4 §1/§2）。
//!
//! 三段：get（无行回 P3 默认 + `isDefault=true`）、set（全量 upsert + 逐字段
//! 范围校验）、reset（删行恢复默认，幂等）。数据访问全走
//! `axagent_dao::repo::bot_department_quota`，不直接碰 entities（check:layers）。

use axagent_dao::repo::bot_department as dept_repo;
use axagent_dao::repo::bot_department_quota as quota_repo;
use axagent_harness::types::{BotDepartmentQuota, BotDepartmentQuotaInput};
use tauri::State;

use crate::AppState;
use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

fn invalid(field: &str, detail: impl Into<String>) -> String {
    ErrorResponse::new(bot_err::QUOTA_INVALID)
        .with_category(ErrorCategory::Validation)
        .with_param("field", field)
        .with_detail(detail)
        .to_string()
}

fn map_error(err: axagent_harness::core_error::AxAgentError) -> String {
    match err {
        axagent_harness::core_error::AxAgentError::NotFound(detail) => {
            ErrorResponse::new(bot_err::DEPARTMENT_NOT_FOUND)
                .with_category(ErrorCategory::Validation)
                .with_detail(detail)
                .to_string()
        },
        other => ErrorResponse::from_error(other, ErrorCategory::Unrecoverable).to_string(),
    }
}

/// 逐字段范围校验（上限即 P3 默认，ADR-P4-2：配额只收紧不放大）。
fn validate_input(input: &BotDepartmentQuotaInput) -> Result<(), String> {
    type Check = (&'static str, i32, i32, i32);
    let checks: [Check; 6] = [
        ("maxConcurrency", input.max_concurrency, 1, 4),
        ("totalTimeoutSecs", input.total_timeout_secs, 60, 3600),
        ("maxSteps", input.max_steps, 1, 12),
        ("maxInFlightPerDepth", input.max_in_flight_per_depth, 1, 4),
        ("stepTimeoutSecs", input.step_timeout_secs, 30, 900),
        ("maxConcurrentRuns", input.max_concurrent_runs, 1, 8),
    ];
    for (field, value, min, max) in checks {
        if value < min || value > max {
            return Err(invalid(field, format!("{field} 必须在 {min}..{max} 之间，实得 {value}")));
        }
    }
    Ok(())
}

async fn ensure_department(
    db: &axagent_dao::DatabaseConnection,
    department_id: &str,
) -> Result<(), String> {
    dept_repo::get_department(db, department_id).await.map(|_| ()).map_err(map_error)
}

#[tauri::command]
pub async fn get_bot_department_quota(
    state: State<'_, AppState>,
    department_id: String,
) -> Result<BotDepartmentQuota, String> {
    let db = state.harness.db();
    ensure_department(db, &department_id).await?;
    Ok(quota_repo::get_by_department(db, &department_id)
        .await
        .map_err(map_error)?
        .unwrap_or_else(|| BotDepartmentQuota::defaults(&department_id)))
}

#[tauri::command]
pub async fn set_bot_department_quota(
    state: State<'_, AppState>,
    input: BotDepartmentQuotaInput,
) -> Result<BotDepartmentQuota, String> {
    validate_input(&input)?;
    let db = state.harness.db();
    ensure_department(db, &input.department_id).await?;
    quota_repo::upsert(db, &input).await.map_err(map_error)
}

#[tauri::command]
pub async fn reset_bot_department_quota(
    state: State<'_, AppState>,
    department_id: String,
) -> Result<(), String> {
    let db = state.harness.db();
    ensure_department(db, &department_id).await?;
    quota_repo::delete_by_department(db, &department_id).await.map(|_| ()).map_err(map_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_input() -> BotDepartmentQuotaInput {
        BotDepartmentQuotaInput {
            department_id: "dept-1".into(),
            max_concurrency: 2,
            total_timeout_secs: 900,
            max_steps: 12,
            max_in_flight_per_depth: 2,
            step_timeout_secs: 300,
            max_concurrent_runs: 2,
        }
    }

    #[test]
    fn defaults_and_bounds_are_accepted() {
        validate_input(&ok_input()).expect("边界内应通过");
        let mut low = ok_input();
        low.max_concurrency = 1;
        low.total_timeout_secs = 60;
        low.max_steps = 1;
        low.max_in_flight_per_depth = 1;
        low.step_timeout_secs = 30;
        low.max_concurrent_runs = 1;
        validate_input(&low).expect("下界应通过");
    }

    #[test]
    fn each_out_of_range_field_reports_its_name() {
        type Case = (&'static str, fn(&mut BotDepartmentQuotaInput));
        let cases: [Case; 6] = [
            ("maxConcurrency", |i| i.max_concurrency = 5),
            ("totalTimeoutSecs", |i| i.total_timeout_secs = 59),
            ("maxSteps", |i| i.max_steps = 13),
            ("maxInFlightPerDepth", |i| i.max_in_flight_per_depth = 0),
            ("stepTimeoutSecs", |i| i.step_timeout_secs = 901),
            ("maxConcurrentRuns", |i| i.max_concurrent_runs = 9),
        ];
        for (field, mutate) in cases {
            let mut input = ok_input();
            mutate(&mut input);
            let err = validate_input(&input).expect_err(field);
            assert!(err.contains("BOT_QUOTA_INVALID"), "{field}: {err}");
            assert!(err.contains(field), "detail 应带字段名 {field}: {err}");
        }
    }
}
