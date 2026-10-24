// SPDX-License-Identifier: AGPL-3.0-only

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::task as task_err;
use crate::commands::spawn_guard::panic_message;
use axagent_agent_macro::agent_command;
use axagent_astock_data::task_cancel::CancelToken;
use axagent_dao::task_ledger::{TransitionRequest, record_task_created, transition_task};
use axagent_entities::background_tasks;
use axagent_harness::IpcEventName;
use axagent_harness::task_state::{TaskSource, TaskStatus};
use chrono::Utc;
use futures::FutureExt;
use sea_orm::*;
use serde::{Deserialize, Serialize};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, State};
use tracing::warn;

/// 对传给 shell 的命令参数做基础转义，防止命令注入。
/// 仅允许字母数字、空格和常见安全字符，拒绝包含 shell 元字符的命令。
fn validate_command(cmd: &str) -> Result<(), String> {
    // 危险字符黑名单：管道、重定向、命令分隔符、命令替换、变量展开、单引号（纵深防御）
    const DANGEROUS_CHARS: &[char] =
        &[';', '&', '|', '`', '$', '(', ')', '<', '>', '\n', '\r', '\''];
    for ch in DANGEROUS_CHARS {
        if cmd.contains(*ch) {
            warn!("background_tasks: 命令包含危险字符 '{}', 已拒绝: {}", ch, cmd);
            return Err(serde_json::to_string(
                &ErrorResponse::new(task_err::DANGEROUS_COMMAND)
                    .with_detail(format!("命令包含危险字符 '{}', 已拒绝: {}", ch, cmd)),
            )
            .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {}\"}}", e)));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTaskInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub task_type: String,
    pub command: Option<String>,
    pub prompt: Option<String>,
    pub status: String,
    pub output: String,
    /// 结构化报告（JSON 字符串）；慢命令的消费方读这里，`output` 只是摘要
    pub result_json: Option<String>,
    pub exit_code: Option<i32>,
    pub conversation_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub attempt: i32,
    pub resume_from: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
}

impl From<background_tasks::Model> for BackgroundTaskInfo {
    fn from(m: background_tasks::Model) -> Self {
        Self {
            id: m.id,
            title: m.title,
            description: m.description,
            task_type: m.task_type,
            command: m.command,
            prompt: m.prompt,
            status: m.status,
            output: m.output,
            result_json: m.result_json,
            exit_code: m.exit_code,
            conversation_id: m.conversation_id,
            idempotency_key: m.idempotency_key,
            attempt: m.attempt,
            resume_from: m.resume_from,
            created_at: m.created_at,
            updated_at: m.updated_at,
            finished_at: m.finished_at,
        }
    }
}

/// 任务事件时间线的一行（`task_events` 的前端投影）。
///
/// 字段名走 camelCase（AGENTS.md 禁区 13）：Rust 侧保持 snake_case，
/// 靠 `#[serde(rename_all = "camelCase")]` 输给前端。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEventInfo {
    pub id: String,
    pub task_id: String,
    /// 写入入口：`command` | `tool` | `restore` | `other`
    pub source: String,
    /// 迁移前状态；创建事件为 `None`
    pub from_status: Option<String>,
    pub to_status: String,
    /// 触发者：`user` | `system` | `scheduler` | `agent` | `unknown`
    pub actor: String,
    pub reason: Option<String>,
    /// 附加 JSON 字符串（原样透传，前端自行解析；不在此处 parse 是因为
    /// 解析失败只会变成 `null`，反而掩盖了「写入了非 JSON」这个事实）
    pub payload: Option<String>,
    /// 毫秒时间戳
    pub created_at: i64,
}

impl From<axagent_entities::task_events::Model> for TaskEventInfo {
    fn from(m: axagent_entities::task_events::Model) -> Self {
        Self {
            id: m.id,
            task_id: m.task_id,
            source: m.source,
            from_status: m.from_status,
            to_status: m.to_status,
            actor: m.actor,
            reason: m.reason,
            payload: m.payload,
            created_at: m.created_at,
        }
    }
}

async fn append_output(db: &DatabaseConnection, task_id: &str, text: &str) -> Result<(), String> {
    let now = Utc::now().timestamp_millis();
    let task = background_tasks::Entity::find_by_id(task_id)
        .one(db)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?
        .ok_or_else(|| {
            serde_json::to_string(&ErrorResponse::new(task_err::NOT_FOUND))
                .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {}\"}}", e))
        })?;
    let mut new_output = task.output.clone();
    new_output.push_str(text);
    if !text.ends_with('\n') {
        new_output.push('\n');
    }
    let mut am: background_tasks::ActiveModel = task.into();
    am.output = Set(new_output);
    am.updated_at = Set(now);
    am.update(db).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    Ok(())
}

/// **任务状态写入的唯一入口**（命令层）。
///
/// 修复前这里直接 `am.status = Set(status.to_string())` —— 裸字符串、无迁移校验、
/// 与 DAO 仓库层 (`background_task_repository.rs`) 各写各的。现在收敛到
/// `axagent_dao::task_ledger::transition_task`：状态语义来自
/// `harness::task_state` 单一真源，每次迁移在同一事务内追加 `task_events`。
///
/// 参数类型从 `&str` 改成 `TaskStatus` 是**故意的**：裸字符串是这条链上
/// 「拼写漂移 + 非法回环」的载体，用枚举让编译器兜住。
async fn update_status(
    db: &DatabaseConnection,
    task_id: &str,
    to: TaskStatus,
    exit_code: Option<i32>,
) -> Result<(), String> {
    let mut req = TransitionRequest::new(task_id, to, TaskSource::Command, "system");
    if let Some(code) = exit_code {
        req = req.exit_code(code).payload(serde_json::json!({ "exitCode": code }));
    }
    axagent_dao::task_ledger::transition_task(db, req).await.map(|_| ()).map_err(|e| {
        // 非法迁移必须显式暴露：静默吞掉会让「任务卡在 running」变得无从归因。
        warn!(task_id = %task_id, to = %to, "[background_tasks] 状态迁移失败: {}", e);
        e.to_string()
    })
}

/// 慢命令的受理回执：只回任务 id，报告经 `result_json` 读侧取回。
///
/// 命令不再 await 到执行结束 —— 130s 的回补期间面板可以切走 / 重载，
/// 报告在任务终态后从库里取（重载也不丢）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAccepted {
    pub task_id: String,
}

/// task_id → 取消旗标。`stop_background_task` 据它置位；spawn 体在终态时摘除。
///
/// 用 tokio 异步锁（禁区 8）且只在极短临界区内持有，不跨 await 等待业务。
type CancelRegistry = tokio::sync::RwLock<std::collections::HashMap<String, CancelToken>>;

fn cancel_tokens() -> &'static CancelRegistry {
    static TOKENS: std::sync::OnceLock<CancelRegistry> = std::sync::OnceLock::new();
    TOKENS.get_or_init(|| tokio::sync::RwLock::new(std::collections::HashMap::new()))
}

async fn register_cancel_token(task_id: &str) -> CancelToken {
    let token = CancelToken::new();
    cancel_tokens().write().await.insert(task_id.to_string(), token.clone());
    token
}

async fn unregister_cancel_token(task_id: &str) {
    cancel_tokens().write().await.remove(task_id);
}

/// 请求取消（stop 命令的接线点）。返回是否命中一个在跑的受管任务。
async fn request_cancel(task_id: &str) -> bool {
    let token = cancel_tokens().read().await.get(task_id).cloned();
    match token {
        Some(t) => {
            t.cancel();
            true
        },
        None => false,
    }
}

fn emit_task_event(app: Option<&tauri::AppHandle>, event: IpcEventName, task_id: &str) {
    if let Some(app) = app {
        let _ = app.emit(event.as_str(), task_id);
    }
}

/// 写终态行：output 存摘要（给人看），result_json 存结构化报告（给消费方）。
///
/// `ActiveValue::Set` 包 Option：Option 字段的 `Set(None)` 是「置空」，
/// 而裸 `None` 是「不写」。
async fn write_terminal_row(
    db: &DatabaseConnection,
    task_id: &str,
    output: String,
    result_json: Option<String>,
) -> Result<(), String> {
    let am = background_tasks::ActiveModel {
        id: Set(task_id.to_string()),
        output: Set(output),
        result_json: Set(result_json),
        updated_at: Set(Utc::now().timestamp_millis()),
        ..Default::default()
    };
    am.update(db).await.map(|_| ()).map_err(|e| {
        String::from(ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 慢命令的异步台账入口：登记任务 → `tokio::spawn` 执行 → 立即返回任务 id。
///
/// 执行体在内嵌台账里同步跑会冻住 IPC（回补 130s）；这里把它挪进 spawn，
/// 命令本身只做「建行 + 起任务」。终态（completed / failed / stopped）与
/// `result_json` 都由 spawn 体写，panic 经 catch_unwind 落 failed，不留 running 僵尸。
///
/// 取消是**协作式**：`stop_background_task` 置旗标，执行体里的 engine 循环
/// 在批间经 `axagent_astock_data::task_cancel::is_cancelled()` 读到后带部分结果
/// 提前返回；随后 spawn 体按「旗标已置位」写 `stopped` 终态（取消≠失败）。
///
/// 台账写入失败只记日志**不阻断业务**：台账是辅助证据面，让「跑成了但没记录」
/// 比「因记录失败而业务失败」便宜得多。
pub async fn spawn_tracked<T, S>(
    app: &tauri::AppHandle,
    db: &DatabaseConnection,
    title: &str,
    task_type: &str,
    description: &str,
    work: impl std::future::Future<Output = Result<T, String>> + Send + 'static,
    summarize: S,
) -> Result<TaskAccepted, String>
where
    T: Serialize + Send + 'static,
    S: FnOnce(&T) -> String + Send + 'static,
{
    spawn_tracked_impl(Some(app.clone()), db, title, task_type, description, work, summarize).await
}

/// 测试可调的核心实现：`app = None` 时不发事件（lib 测试没有 Tauri AppHandle）。
async fn spawn_tracked_impl<T, S>(
    app: Option<tauri::AppHandle>,
    db: &DatabaseConnection,
    title: &str,
    task_type: &str,
    description: &str,
    work: impl std::future::Future<Output = Result<T, String>> + Send + 'static,
    summarize: S,
) -> Result<TaskAccepted, String>
where
    T: Serialize + Send + 'static,
    S: FnOnce(&T) -> String + Send + 'static,
{
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().timestamp_millis();
    let model = background_tasks::ActiveModel {
        id: Set(id.clone()),
        title: Set(title.to_string()),
        description: Set(description.to_string()),
        task_type: Set(task_type.to_string()),
        command: Set(None),
        prompt: Set(None),
        status: Set(TaskStatus::Pending.as_db_str().to_string()),
        output: Set(String::new()),
        result_json: Set(None),
        exit_code: Set(None),
        conversation_id: Set(None),
        created_by: Set(None),
        idempotency_key: Set(None),
        attempt: Set(0),
        resume_from: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        finished_at: Set(None),
    };
    background_tasks::Entity::insert(model).exec(db).await.map_err(|e| {
        String::from(ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    if let Err(e) = record_task_created(db, &id, TaskSource::Command, "user").await {
        tracing::error!(task_id = %id, "[spawn_tracked] 写入创建事件失败（审计脊将缺少起源）: {e}");
    }
    // 先注册再 spawn：stop 必须能命中「已受理、还没进 running」的窗口
    let token = register_cancel_token(&id).await;
    emit_task_event(app.as_ref(), IpcEventName::BackgroundTaskCreated, &id);

    let task_id = id.clone();
    let db = db.clone();
    // 与 spawn_background_task 同形态：命令本身跑在 tokio 上
    tokio::spawn(async move {
        if let Err(e) = transition_task(
            &db,
            TransitionRequest::new(&task_id, TaskStatus::Running, TaskSource::Command, "user"),
        )
        .await
        {
            warn!(task_id = %task_id, "[spawn_tracked] 置 running 失败（继续执行）: {e}");
        }
        emit_task_event(app.as_ref(), IpcEventName::BackgroundTaskUpdated, &task_id);

        let outcome = axagent_astock_data::task_cancel::with_cancel(
            token.clone(),
            AssertUnwindSafe(work).catch_unwind(),
        )
        .await;

        let (to, exit_code, reason): (TaskStatus, i32, &str) = match &outcome {
            Ok(Ok(_)) if token.is_cancelled() => {
                (TaskStatus::Stopped, 1, "用户取消：已完成部分保留")
            },
            Ok(Ok(_)) => (TaskStatus::Completed, 0, "命令执行完成"),
            Ok(Err(_)) => (TaskStatus::Failed, 1, "命令执行返回错误"),
            Err(_) => (TaskStatus::Failed, 1, "执行体 panic"),
        };

        let (output, result_json) = match outcome {
            Ok(Ok(value)) => {
                let summary = summarize(&value);
                let json = match serde_json::to_string(&value) {
                    Ok(j) => Some(j),
                    Err(e) => {
                        tracing::error!(task_id = %task_id, "[spawn_tracked] 报告序列化失败: {e}");
                        None
                    },
                };
                let output = if token.is_cancelled() {
                    format!("用户取消（已完成部分）：{summary}")
                } else {
                    summary
                };
                (output, json)
            },
            Ok(Err(err)) => (err, None),
            Err(panic) => (panic_message(&panic), None),
        };

        if let Err(e) = write_terminal_row(&db, &task_id, output, result_json).await {
            tracing::error!(task_id = %task_id, "[spawn_tracked] 写入终态行失败: {e}");
        }
        if let Err(e) = transition_task(
            &db,
            TransitionRequest::new(&task_id, to, TaskSource::Command, "user")
                .exit_code(exit_code)
                .reason(reason),
        )
        .await
        {
            tracing::error!(task_id = %task_id, "[spawn_tracked] 置终态 {:?} 失败: {e}", to);
        }
        unregister_cancel_token(&task_id).await;
        emit_task_event(app.as_ref(), IpcEventName::BackgroundTaskUpdated, &task_id);
    });

    Ok(TaskAccepted { task_id: id })
}

#[agent_command(domain = system, safety = Caution, call_mode = StateInput, description = "创建后台任务")]
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn spawn_background_task(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    title: String,
    task_type: String,
    command: Option<String>,
    prompt: Option<String>,
    description: Option<String>,
    idempotency_key: Option<String>,
) -> Result<String, String> {
    let db = state.harness.db().clone();

    // 幂等：若提供 idempotency_key 且已存在任务，直接返回已有任务 id，不重复提交
    if let Some(key) = idempotency_key.as_deref() {
        let existing = background_tasks::Entity::find()
            .filter(background_tasks::Column::IdempotencyKey.eq(key))
            .one(&db)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
        if let Some(task) = existing {
            tracing::info!(
                task_id = %task.id, key = %key,
                "[background_tasks] 命中幂等键，返回已有任务"
            );
            return Ok(task.id);
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().timestamp_millis();

    let model = background_tasks::ActiveModel {
        id: Set(id.clone()),
        title: Set(title.clone()),
        description: Set(description.unwrap_or_default()),
        task_type: Set(task_type.clone()),
        command: Set(command.clone()),
        prompt: Set(prompt.clone()),
        status: Set(TaskStatus::Pending.as_db_str().to_string()),
        output: Set(String::new()),
        result_json: Set(None),
        exit_code: Set(None),
        conversation_id: Set(None),
        created_by: Set(None),
        idempotency_key: Set(idempotency_key),
        attempt: Set(0),
        resume_from: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        finished_at: Set(None),
    };
    background_tasks::Entity::insert(model).exec(&db).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // 创建事件：让「任务从哪来」在事件脊上有起源。
    // 失败不阻断创建（业务行已落库，回滚没有意义），但**必须显式记 error** ——
    // 「事件写失败」如果只是 `let _ =`，事后排障会把事件脊的空洞当成噪声忽略，
    // 而空洞恰恰是最需要解释的状态。
    if let Err(e) =
        axagent_dao::task_ledger::record_task_created(&db, &id, TaskSource::Command, "user").await
    {
        tracing::error!(
            task_id = %id,
            "[spawn_background_task] 写入创建事件失败（任务已创建，审计脊将缺少起源）: {}", e
        );
    }

    if task_type == "bash" {
        if let Some(cmd) = command {
            // 安全校验：拒绝包含 shell 元字符的命令，防止命令注入
            validate_command(&cmd)?;
            let db1 = db.clone();
            let db2 = db.clone();
            let db3 = db.clone();
            let tid1 = id.clone();
            let tid2 = id.clone();
            let tid3 = id.clone();
            let tid4 = id.clone();
            let tid5 = id.clone();
            let app = app_handle.clone();
            tokio::spawn(async move {
                // === 1. Drop guard：任何路径退出都把 status 收敛到终态 ===
                // 业务完成前 finished=false；任何 panic / 早退 / 早 return
                // 都会触发 drop() 兜底：把 status 强制落为 failed 并 emit
                // `background-task:failed` 事件，防止任务卡在 running。
                struct TaskGuard {
                    db: sea_orm::DatabaseConnection,
                    task_id: String,
                    app: tauri::AppHandle,
                    finished: AtomicBool,
                }
                impl Drop for TaskGuard {
                    fn drop(&mut self) {
                        if self.finished.load(Ordering::Acquire) {
                            return;
                        }
                        // drop 路径上不能 .await，用 fire-and-forget spawn 兜底写库
                        let db = self.db.clone();
                        let tid = self.task_id.clone();
                        let app = self.app.clone();
                        tokio::spawn(async move {
                            if let Err(e) =
                                update_status(&db, &tid, TaskStatus::Failed, Some(-1)).await
                            {
                                warn!("TaskGuard drop 兜底更新 status 失败: {}", e);
                            }
                            let _ = app.emit("background-task:failed", &tid);
                        });
                    }
                }
                let guard = TaskGuard {
                    db: db1.clone(),
                    task_id: tid1.clone(),
                    app: app.clone(),
                    finished: AtomicBool::new(false),
                };

                // === 2. catch_unwind 包裹主体 ===
                let result = AssertUnwindSafe(async {
                    if let Err(e) = update_status(&db1, &tid1, TaskStatus::Running, None).await {
                        warn!("更新任务状态失败: {}", e);
                    }
                    let mut cmd_builder =
                        tokio::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
                    cmd_builder
                        .arg(if cfg!(windows) { "/C" } else { "-c" })
                        .arg(&cmd)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped());
                    // Windows: 隐藏控制台窗口
                    #[cfg(windows)]
                    axagent_kit::utils::hide_window(cmd_builder.as_std_mut());
                    let mut child = match cmd_builder.spawn() {
                        Ok(c) => c,
                        Err(e) => {
                            if let Err(e2) =
                                append_output(&db2, &tid2, &format!("启动失败: {}", e)).await
                            {
                                warn!("追加输出失败: {}", e2);
                            }
                            // Drop guard 会兜底更新 status=failed + emit failed 事件
                            return;
                        },
                    };
                    let stdout = child.stdout.take();
                    let stderr = child.stderr.take();
                    let db_o = db3.clone();
                    let tid_o = tid3.clone();
                    // stdout_task: 子 spawn panic 只会丢部分输出
                    // 用 catch_unwind 兜住, 不污染外层状态机
                    let stdout_task = tokio::spawn(async move {
                        let inner = AssertUnwindSafe(async {
                            if let Some(mut reader) = stdout {
                                use tokio::io::AsyncBufReadExt;
                                let mut lines = tokio::io::BufReader::new(&mut reader).lines();
                                while let Ok(Some(line)) = lines.next_line().await {
                                    if let Err(e) = append_output(&db_o, &tid_o, &line).await {
                                        warn!("追加 stdout 输出失败: {}", e);
                                    }
                                }
                            }
                        })
                        .catch_unwind()
                        .await;
                        if let Err(p) = inner {
                            tracing::error!(
                                "[spawn_background_task] stdout_task PANIC task={}: {}",
                                tid_o,
                                panic_message(&p)
                            );
                        }
                    });
                    let db_e = db3.clone();
                    let tid_e = tid5.clone();
                    let stderr_task = tokio::spawn(async move {
                        let inner = AssertUnwindSafe(async {
                            if let Some(mut reader) = stderr {
                                use tokio::io::AsyncBufReadExt;
                                let mut lines = tokio::io::BufReader::new(&mut reader).lines();
                                while let Ok(Some(line)) = lines.next_line().await {
                                    if let Err(e) =
                                        append_output(&db_e, &tid_e, &format!("[stderr] {}", line))
                                            .await
                                    {
                                        warn!("追加 stderr 输出失败: {}", e);
                                    }
                                }
                            }
                        })
                        .catch_unwind()
                        .await;
                        if let Err(p) = inner {
                            tracing::error!(
                                "[spawn_background_task] stderr_task PANIC task={}: {}",
                                tid_e,
                                panic_message(&p)
                            );
                        }
                    });
                    let status = child.wait().await;
                    let _ = stdout_task.await;
                    let _ = stderr_task.await;
                    match status {
                        Ok(exit) => {
                            let code = exit.code().unwrap_or(-1);
                            if exit.success() {
                                if let Err(e) = append_output(
                                    &db3,
                                    &tid4,
                                    &format!("\n--- 完成 (exit: {}) ---", code),
                                )
                                .await
                                {
                                    tracing::warn!("后台任务追加输出失败 task_id={}: {}", tid4, e);
                                }
                                if let Err(e) =
                                    update_status(&db3, &tid4, TaskStatus::Completed, Some(code))
                                        .await
                                {
                                    tracing::warn!(
                                        "后台任务状态更新为 completed 失败 task_id={}: {}",
                                        tid4,
                                        e
                                    );
                                }
                            } else {
                                if let Err(e) = append_output(
                                    &db3,
                                    &tid4,
                                    &format!("\n--- 失败 (exit: {}) ---", code),
                                )
                                .await
                                {
                                    tracing::warn!("后台任务追加输出失败 task_id={}: {}", tid4, e);
                                }
                                if let Err(e) =
                                    update_status(&db3, &tid4, TaskStatus::Failed, Some(code)).await
                                {
                                    tracing::warn!(
                                        "后台任务状态更新为 failed 失败 task_id={}: {}",
                                        tid4,
                                        e
                                    );
                                }
                            }
                        },
                        Err(e) => {
                            if let Err(ee) =
                                append_output(&db3, &tid4, &format!("\n--- 执行错误: {} ---", e))
                                    .await
                            {
                                tracing::warn!("后台任务追加输出失败 task_id={}: {}", tid4, ee);
                            }
                            // Drop guard 兜底覆盖 status=failed
                            return;
                        },
                    }
                    let _ = app.emit(IpcEventName::BackgroundTaskUpdated.as_str(), &tid4);
                    // === 3. 业务正常完成, 通知 Drop guard 跳过兜底 ===
                    guard.finished.store(true, Ordering::Release);
                })
                .catch_unwind()
                .await;

                if let Err(panic) = result {
                    // Drop guard 在 outer scope 末尾 drop, 会自动把 status 落为 failed
                    tracing::error!(
                        "[spawn_background_task] PANIC task={}: {}",
                        tid1,
                        panic_message(&panic)
                    );
                }
                // guard 在此 drop —— finished=false 时触发兜底
            });
        }
    } else if task_type == "agent" {
        if let Err(e) = update_status(&db, &id, TaskStatus::Running, None).await {
            tracing::warn!("后台任务启动状态更新为 running 失败 task_id={}: {}", id, e);
        }
    }
    let _ = app_handle.emit(IpcEventName::BackgroundTaskCreated.as_str(), &id);
    Ok(id)
}

#[agent_command(domain = system, safety = Safe, call_mode = StateOnly, description = "列出后台任务")]
#[tauri::command]
pub async fn list_background_tasks(
    state: State<'_, AppState>,
) -> Result<Vec<BackgroundTaskInfo>, String> {
    let tasks = background_tasks::Entity::find()
        .order_by_desc(background_tasks::Column::CreatedAt)
        .all(state.harness.db())
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    Ok(tasks.into_iter().map(Into::into).collect())
}

#[agent_command(domain = system, safety = Safe, call_mode = StateInput, description = "获取后台任务输出")]
#[tauri::command]
pub async fn get_background_task_output(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<BackgroundTaskInfo, String> {
    let task = background_tasks::Entity::find_by_id(&task_id)
        .one(state.harness.db())
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?
        .ok_or_else(|| {
            serde_json::to_string(&ErrorResponse::new(task_err::NOT_FOUND))
                .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {}\"}}", e))
        })?;
    Ok(task.into())
}

#[agent_command(domain = system, safety = Caution, call_mode = StateInput, description = "停止后台任务")]
#[tauri::command]
pub async fn stop_background_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    let task = background_tasks::Entity::find_by_id(&task_id)
        .one(state.harness.db())
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?
        .ok_or_else(|| {
            serde_json::to_string(&ErrorResponse::new(task_err::NOT_FOUND))
                .unwrap_or_else(|e| format!("{{\"error\":\"serialization failed: {}\"}}", e))
        })?;
    // 受管慢任务（spawn_tracked）：置旗标即可，终态与部分报告由 spawn 体写 ——
    // 这里直接改库会和它的终态写相撞（状态迁移被拒 + 两处写终态）。
    if request_cancel(&task_id).await {
        return Ok(());
    }
    // 可停止态用枚举判定（原来是裸字符串比较，`"runing"` 之类拼写错误会静默
    // 变成「不可停止」而不是报错）。未知状态不假装可停止。
    let current = TaskStatus::from_db_str(&task.status);
    if matches!(current, TaskStatus::Running | TaskStatus::Pending) {
        update_status(state.harness.db(), &task_id, TaskStatus::Stopped, None).await?;
    } else if current == TaskStatus::Unknown {
        warn!(
            task_id = %task_id, raw = %task.status,
            "[stop_background_task] 任务状态未知，按「不可停止」处理（不猜）"
        );
    }
    Ok(())
}

/// 单次查询最多返回的事件条数。
///
/// `task_events` 是追加型表，长跑任务（每秒一条进度）能让单任务事件数上万。
/// 前端时间线不需要全量，因此硬性截断 —— 但**截断策略写在读命令里而不是
/// `limit` 参数上**：调用方传 `u64::MAX` 时不至于把整个表拖进内存。
const MAX_TASK_EVENTS: u64 = 500;

/// 拉取某任务的状态迁移时间线（正序：最早在前，最新在后）。
///
/// 这是 `task_events` 的**读路径**。写入型审计表如果没有读消费者，它就是
/// 死表 —— 表在建、数据在写、但没有任何一处能把它显示出来，
/// `cargo check` 与单测照样全绿。因此该命令与 `TaskPanel` 的时间线是一对，
/// 缺任何一个都算未完成。
#[agent_command(domain = system, safety = Safe, call_mode = StateInput, description = "获取后台任务状态迁移时间线")]
#[tauri::command]
pub async fn list_task_events(
    state: State<'_, AppState>,
    task_id: String,
    limit: Option<u64>,
) -> Result<Vec<TaskEventInfo>, String> {
    let effective = limit.unwrap_or(100).clamp(1, MAX_TASK_EVENTS);
    let rows = axagent_dao::task_ledger::list_task_events(state.harness.db(), &task_id, effective)
        .await
        .map_err(|e| {
            // 读失败就是读失败，不返回空 `Vec`：空列表会被前端渲染成
            // 「该任务没有任何状态迁移」，那是**归因字段说谎**（铁律 #12）。
            warn!(task_id = %task_id, "[list_task_events] 读取任务事件失败: {}", e);
            e.to_string()
        })?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[cfg(test)]
mod spawn_tracked_tests {
    use super::*;

    const TERMINAL: [TaskStatus; 3] =
        [TaskStatus::Completed, TaskStatus::Failed, TaskStatus::Stopped];

    /// 轮询到终态（spawn 体与测试不共享 JoinHandle）。
    async fn wait_terminal(db: &DatabaseConnection, id: &str) -> background_tasks::Model {
        for _ in 0..500 {
            let row = background_tasks::Entity::find_by_id(id)
                .one(db)
                .await
                .expect("查询应成功")
                .expect("受理后行应立即存在");
            if TERMINAL.contains(&TaskStatus::from_db_str(&row.status)) {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("任务在 5s 内未进入终态（spawn 体可能没跑起来）");
    }

    #[tokio::test]
    async fn success_records_completed_with_summary_and_result_json() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应创建成功");
        let db = &h.conn;
        let accepted = spawn_tracked_impl(
            None,
            db,
            "测试任务",
            "test-type",
            "成功路径",
            async { Ok::<_, String>(7_i32) },
            |v| format!("结果 {v}"),
        )
        .await
        .expect("应受理");

        let r = wait_terminal(db, &accepted.task_id).await;
        assert_eq!(r.status, TaskStatus::Completed.as_db_str());
        assert_eq!(r.exit_code, Some(0));
        assert_eq!(r.output, "结果 7", "output 只放摘要");
        assert_eq!(r.result_json.as_deref(), Some("7"), "结构化报告落在 result_json");
        assert!(r.finished_at.is_some(), "终态必须带结束时间");
    }

    #[tokio::test]
    async fn business_error_records_failed_without_result_json() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应创建成功");
        let db = &h.conn;
        let accepted = spawn_tracked_impl(
            None,
            db,
            "测试任务",
            "test-type",
            "业务失败路径",
            async { Err::<i32, _>("引擎报错了".to_string()) },
            |v| format!("结果 {v}"),
        )
        .await
        .expect("受理本身不应失败");

        let r = wait_terminal(db, &accepted.task_id).await;
        assert_eq!(r.status, TaskStatus::Failed.as_db_str());
        assert_eq!(r.exit_code, Some(1));
        assert_eq!(r.output, "引擎报错了", "失败原因要落在 output，前端才有得显示");
        assert!(r.result_json.is_none(), "失败时不得留下半份报告");
        assert!(r.finished_at.is_some());
    }

    #[tokio::test]
    async fn panic_records_failed_not_running_zombie() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应创建成功");
        let db = &h.conn;
        let accepted = spawn_tracked_impl(
            None,
            db,
            "测试任务",
            "test-type",
            "panic 路径",
            async {
                panic!("执行体炸了");
                #[allow(unreachable_code)]
                Ok::<i32, String>(0)
            },
            |v| format!("结果 {v}"),
        )
        .await
        .expect("受理本身不应失败");

        let r = wait_terminal(db, &accepted.task_id).await;
        assert_eq!(
            r.status,
            TaskStatus::Failed.as_db_str(),
            "panic 不得留 running 僵尸（管道僵尸行教训）"
        );
        assert_eq!(r.exit_code, Some(1));
        assert!(r.output.contains("执行体炸了"), "panic 消息应透出，实际: {}", r.output);
        assert!(r.finished_at.is_some());
    }

    #[tokio::test]
    async fn cancel_records_stopped_with_partial_report() {
        use axagent_astock_data::task_cancel::is_cancelled;

        let h = axagent_dao::db::create_test_pool().await.expect("测试库应创建成功");
        let db = &h.conn;
        let accepted = spawn_tracked_impl(
            None,
            db,
            "测试任务",
            "test-type",
            "取消路径",
            async {
                // 模拟 engine 批间循环：每步检查一次旗标，命中就带部分结果返回
                let mut steps = 0;
                while steps < 1000 {
                    if is_cancelled() {
                        break;
                    }
                    steps += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                Ok::<_, String>(serde_json::json!({
                    "steps": steps,
                    "cancelled": is_cancelled(),
                }))
            },
            |v| format!("完成 {} 步", v["steps"]),
        )
        .await
        .expect("应受理");

        assert!(request_cancel(&accepted.task_id).await, "受理后旗标必须已注册");

        let r = wait_terminal(db, &accepted.task_id).await;
        assert_eq!(
            r.status,
            TaskStatus::Stopped.as_db_str(),
            "取消是 stopped 不是 failed（取消≠失败）"
        );
        assert!(r.output.starts_with("用户取消"), "摘要要显式说明被取消，实际: {}", r.output);
        let json: serde_json::Value =
            serde_json::from_str(r.result_json.as_deref().expect("部分报告应落库"))
                .expect("result_json 应是合法 JSON");
        assert_eq!(json["cancelled"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn cancel_without_registration_is_noop() {
        // 老任务 / 非受管任务的 stop 走原状态迁移路径，不能因注册表未命中而报错
        assert!(!request_cancel("no-such-task").await);
    }
}
