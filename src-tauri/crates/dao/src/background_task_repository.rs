// SPDX-License-Identifier: AGPL-3.0-only

//! DAO implementation of BackgroundTaskRepository using SeaORM.
//!
//! ## 与命令层的关系（P1-C：唯一写路径）
//!
//! 本仓库是**工具层**访问 `background_tasks` 的入口（`TaskCreate` /
//! `TaskGet` / `TaskList` / `TaskStop` / `TaskUpdate` / `TaskOutput` 六个 agent 工具
//! 经此路径）。修复前它与 Tauri 命令层 `commands/background_tasks.rs` 是两条
//! 互不知晓的写路径，差异包括：
//!
//! | 维度 | 命令层（修复前） | 本层（修复前） |
//! |---|---|---|
//! | 时间戳单位 | 毫秒 | **秒** |
//! | 状态校验 | 无 | 无 |
//! | 终态集合 | completed/failed/stopped | completed/failed/stopped |
//!
//! 现在两层的状态写入都经 `crate::task_ledger::transition_task`：
//! 语义来自 `harness::task_state`，时间统一毫秒，每次迁移同事务追加事件。

use async_trait::async_trait;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, QueryOrder, Set};
use uuid::Uuid;

use axagent_entities::background_tasks;
use axagent_harness::repo_dtos::{BackgroundTask, CreateBackgroundTaskInput};
use axagent_harness::repositories::BackgroundTaskRepository;
use axagent_harness::task_state::{TaskSource, TaskStatus};

use crate::task_ledger::{self, TransitionRequest};

fn model_to_dto(m: background_tasks::Model) -> BackgroundTask {
    BackgroundTask {
        id: m.id,
        title: m.title,
        description: m.description,
        task_type: m.task_type,
        command: m.command,
        prompt: m.prompt,
        status: m.status,
        output: m.output,
        exit_code: m.exit_code,
        conversation_id: m.conversation_id,
        created_by: m.created_by,
        idempotency_key: m.idempotency_key,
        attempt: m.attempt,
        resume_from: m.resume_from,
        created_at: m.created_at,
        updated_at: m.updated_at,
        finished_at: m.finished_at,
    }
}

pub struct DaoBackgroundTaskRepository {
    db: DatabaseConnection,
}

impl DaoBackgroundTaskRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl BackgroundTaskRepository for DaoBackgroundTaskRepository {
    async fn spawn_task(&self, input: CreateBackgroundTaskInput) -> Result<BackgroundTask, String> {
        // 统一毫秒：命令层 `spawn_background_task` 用的是 `timestamp_millis()`，
        // 本层原来是 `timestamp()`（秒）。同列混存两种单位会让按时间排序静默错序。
        let now = chrono::Utc::now().timestamp_millis();
        let id = Uuid::new_v4().to_string();

        let am = background_tasks::ActiveModel {
            id: Set(id.clone()),
            title: Set(input.title),
            description: Set(input.description),
            task_type: Set(input.task_type),
            command: Set(input.command),
            prompt: Set(input.prompt),
            status: Set(TaskStatus::Pending.as_db_str().to_string()),
            output: Set(String::new()),
            exit_code: Set(None),
            conversation_id: Set(None),
            created_by: Set(input.created_by),
            idempotency_key: Set(input.idempotency_key),
            attempt: Set(0),
            resume_from: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            finished_at: Set(None),
        };

        let model = am.insert(&self.db).await.map_err(|e| format!("spawn_task: {}", e))?;

        // 创建事件（`source = tool`：这是 agent 工具路径，与前端命令路径区分开）。
        // 记录失败不阻断创建，但必须显式记日志 —— 不要 `let _ =` 吞掉。
        if let Err(e) =
            task_ledger::record_task_created(&self.db, &id, TaskSource::Tool, "agent").await
        {
            tracing::error!(
                task_id = %id,
                "[background_task_repository] 写入创建事件失败（审计脊将缺少起源）: {}", e
            );
        }

        Ok(model_to_dto(model))
    }

    async fn get_task(&self, id: &str) -> Result<Option<BackgroundTask>, String> {
        let model = background_tasks::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| format!("get_task: {}", e))?;

        Ok(model.map(model_to_dto))
    }

    async fn list_tasks(&self) -> Result<Vec<BackgroundTask>, String> {
        let models = background_tasks::Entity::find()
            .order_by_desc(background_tasks::Column::CreatedAt)
            .all(&self.db)
            .await
            .map_err(|e| format!("list_tasks: {}", e))?;

        Ok(models.into_iter().map(model_to_dto).collect())
    }

    async fn stop_task(&self, id: &str) -> Result<(), String> {
        // 收敛到账本：状态校验 + 事件脊 + 毫秒时间戳。
        // 原实现无条件写 `stopped`（即使任务已 completed/failed），
        // 会把「已完成」的任务改写成「已停止」—— 账本的迁移表会拒掉。
        task_ledger::transition_task(
            &self.db,
            TransitionRequest::new(id, TaskStatus::Stopped, TaskSource::Tool, "agent")
                .reason("工具调用：停止任务"),
        )
        .await
        .map(|_| ())
        .map_err(|e| format!("stop_task: {}", e))
    }

    async fn update_status(&self, id: &str, status: &str) -> Result<(), String> {
        // `TaskUpdate` 工具的 `status` 来自模型，是**不可信输入**。
        // 原实现直接落库（模型可以写进 `"banana"`，之后所有 `match` 落空、
        // 前端 `STATUS_CONFIG` 兜底成 pending 显示 —— 状态字段说谎且无人发现）。
        // 现在：解析失败即报错，不猜、不兜底（铁律 #12）。
        let to = TaskStatus::from_db_str(status);
        if to == TaskStatus::Unknown {
            return Err(format!(
                "update_status: 未知状态 {status:?}（合法值：pending/running/completed/failed/stopped）"
            ));
        }
        // 工具路径不写 exit_code：模型的 `status` 参数里没有退出码语义，
        // 用 0 兜底等于伪造数据。
        task_ledger::transition_task(
            &self.db,
            TransitionRequest::new(id, to, TaskSource::Tool, "agent").reason("工具调用：更新状态"),
        )
        .await
        .map(|_| ())
        .map_err(|e| format!("update_status: {}", e))
    }

    async fn get_output(&self, id: &str) -> Result<Option<String>, String> {
        let model = background_tasks::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| format!("get_output: {}", e))?;

        Ok(model.map(|m| m.output))
    }
}
