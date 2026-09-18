// SPDX-License-Identifier: AGPL-3.0-only

//! 任务「不丢失、不重复」能力 — 恢复引导。
//!
//! 启动时扫描 `background_tasks` 中未完成（pending / running）的任务，重新入队。
//! - `bash` 类型：`attempt += 1` 后重跑（临时失败重试）。
//! - `agent` 类型：从 `resume_from`（断点位置）续跑；无断点则复位为 pending。
//!
//! Tauri 命令入口 `restore_pending_tasks`（手动触发，调试用），启动钩子
//! 对应实现由 `init/services.rs` 调用，通过传入 `DatabaseConnection` 与
//! 可选的 `AppHandle` 发送事件，保持本模块与 `AppState` 解耦。

use axagent_entities::background_tasks;
use axagent_harness::task_state::{TaskSource, TaskStatus};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tauri::{Emitter, State};
use tracing::{error, info};

use crate::AppState;

/// 未完成任务将重新入队的可见状态集合
const INCOMPLETE: &[&str] = &["pending", "running"];

/// Tauri 命令：手动触发恢复引导（调试/运维用）。
#[tauri::command]
pub async fn restore_pending_tasks(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
    let db = state.harness.db().clone();
    restore_incomplete_tasks(&db, Some(&app_handle)).await.map_err(|e| {
        crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Retryable,
        )
        .to_string()
    })
}

/// 恢复引导核心逻辑：扫描未完成任务 → 置回 pending（attempt 自增）。
///
/// 幂等：每个任务原子地复位为 pending；重复调用不会重复计数。
/// 返回被恢复的任务 id 列表。
pub async fn restore_incomplete_tasks(
    db: &sea_orm::DatabaseConnection,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<Vec<String>, sea_orm::DbErr> {
    let tasks = background_tasks::Entity::find()
        .filter(background_tasks::Column::Status.is_in(INCOMPLETE.to_vec()))
        .all(db)
        .await?;

    let mut recovered = Vec::new();
    for task in tasks {
        let task_id = task.id.clone();
        let current = task.status.clone();
        let next_attempt = task.attempt.saturating_add(1);
        // agent 任务无断点则从零开始；有断点保留 resume_from 供续跑。
        let resume_from = if task.task_type == "agent" {
            task.resume_from.clone()
        } else {
            None
        };

        // ── 状态列走唯一写路径（P1-C）──
        // 修复前这里直接 `am.status = Set("pending")`，是 `background_tasks.status`
        // 的第二个写路径（另一个在 `commands/background_tasks.rs`），且 `updated_at`
        // 用的是秒（命令层用毫秒）。现在状态迁移经账本，附带迁移校验 + 事件脊 + 毫秒。
        let from = TaskStatus::from_db_str(&current);
        let already_pending = from == TaskStatus::Pending;
        if !already_pending {
            if let Err(e) = axagent_dao::task_ledger::transition_task(
                db,
                axagent_dao::task_ledger::TransitionRequest::new(
                    &task_id,
                    TaskStatus::Pending,
                    TaskSource::Restore,
                    "scheduler",
                )
                .reason("启动恢复：未完成任务重新入队"),
            )
            .await
            {
                // 迁移被拒（例如状态未知）：**跳过**该任务而不是继续写 attempt。
                // 若继续写 attempt 而不改状态，会出现「attempt 增加了但状态没变」
                // 的账实不符 —— 比起漏恢复一个任务，静默的账实不符更难查。
                error!(
                    task_id = %task_id, raw_status = %current,
                    "[scheduler.restore] 跳过：状态迁移被拒: {}", e
                );
                continue;
            }
        }

        // ── 非状态列（attempt / resume_from）：状态列已由账本处理 ──
        // 这两列不参与状态迁移，故在此单独更新；`status` **不在这里写**，
        // 以保证「status 只有一个写路径」这条不变量成立。
        let mut am: background_tasks::ActiveModel = task.into();
        am.resume_from = Set(resume_from);
        am.attempt = Set(next_attempt);
        am.updated_at = Set(chrono::Utc::now().timestamp_millis());
        am.update(db).await?;
        recovered.push(task_id);
    }

    if !recovered.is_empty() {
        info!("[scheduler.restore] 恢复 {} 个未完成任务: {:?}", recovered.len(), recovered);
        if let Some(app) = app_handle {
            let _ = app.emit("background-task:restored", &recovered);
        }
    } else {
        info!("[scheduler.restore] 无不完整后台任务，无需恢复");
    }
    Ok(recovered)
}
