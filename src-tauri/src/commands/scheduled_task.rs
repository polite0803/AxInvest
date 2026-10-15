// SPDX-License-Identifier: AGPL-3.0-only

//! 定时任务 Tauri 命令 — 基于 CronJobStore 统一调度系统。
//! 命令名保持与旧 ScheduledTaskService 兼容，供前端 SchedulerSettings 调用。

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::task as task_err;
use axagent_agent_macro::agent_command;
use axagent_runtime_core::{CronJob, CronJobStatus, ExecutionRecord, TaskConfig};
use serde::{Deserialize, Serialize};
use tauri::State;

/// SECURITY (C5): 调度任务操作者鉴权。
///
/// 信任模型：AxAgent 是桌面应用，Tauri IPC 仅允许本地 webview 调用命令，
/// 恶意网页无法直接访问 IPC 通道（由 Tauri capability + CSP 隔离）。
/// 因此 `state_has_operator` 始终返回 `true`，表示桌面用户即为操作者。
///
/// 未来若引入多用户/远程模式，应在 AppState 中加入 `active_operator` 字段，
/// 并将此函数改为 `state.active_operator.is_some()`。
fn require_operator(state: &State<'_, AppState>) -> Result<(), String> {
    if !state_has_operator(state) {
        return Err(ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            "operator not authenticated",
        ));
    }
    Ok(())
}

fn state_has_operator(_state: &State<'_, AppState>) -> bool {
    // 桌面应用信任边界：Tauri IPC 调用者即为操作者。
    // 远程/多用户模式下需改为真实认证校验。
    true
}

/// SECURITY (M6): 5 字段 cron 表达式 + 频率下限（最小 1 分钟一次）。
///
/// `pub(crate)`：业务模块装配定时任务时复用同一套校验（如
/// `opc_demand_subscription::opc_ensure_demand_scan_job`），避免校验规则分叉。
pub(crate) fn validate_cron_expression(cron: &str) -> Result<(), String> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            format!("Cron must have exactly 5 fields, got {}: '{}'", parts.len(), cron),
        ));
    }
    // 检查 minute 字段（第一字段），拒绝过于频繁的执行
    let minute = parts[0];
    if minute == "*" || minute == "*/1" {
        return Err(ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            "Cron 表达式过于频繁：不允许每分钟执行，最小间隔为 2 分钟",
        ));
    }
    // 检查 */N 模式中 N < 1 的情况（实际上 N=1 已被上面覆盖）
    if let Some(rest) = minute.strip_prefix("*/") {
        if let Ok(n) = rest.parse::<u32>() {
            if n < 1 {
                return Err(ErrorResponse::err_with_detail(
                    crate::commands::error_code::common::INVALID_INPUT,
                    "Cron interval less than 1 minute is not allowed",
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTaskDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub task_type: String,
    pub cron_expression: Option<String>,
    pub interval_seconds: Option<i64>,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_result: Option<TaskRunResultDto>,
    pub status: String,
    pub config: TaskConfigDto,
    pub created_at: String,
    pub updated_at: String,
    #[serde(rename = "workflowId")]
    pub workflow_id: Option<String>,
    /// 结构化业务参数（参数化任务使用，如时间旅行批次的 asOf/档位勾选）
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunResultDto {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
    /// 本轮被闸门跳过（非交易日等），未真正执行业务
    #[serde(default)]
    pub skipped: bool,
}

/// 拥有内置执行 handler、但**没有**关联工作流的 task_type 集合。
///
/// 与 `init/services.rs` 中 cron executor handler 的分支一一对应 ——
/// 新增内置任务类型时两处必须同步，否则「立即执行」会误报「不支持手动执行」。
/// （带 `workflow_id` 的任务，如 trend-screening / opc-demand-discovery，
/// 走工作流执行分支，不在此集合。）
const BUILTIN_TASK_TYPES: &[&str] = &[
    "knowledge_source_fetch_all",
    "stock-recommendation",
    "watchlist-scan",
    crate::commands::pool_scan::POOL_SCAN_TASK_TYPE,
    "batch-reflection",
    "validate-decisions",
    crate::commands::opc_demand_subscription::SCAN_JOB_TASK_TYPE,
    crate::commands::backtest_validation::DECISION_BACKTEST_TASK_TYPE,
    crate::commands::constants::MOVER_RECALL_TASK_TYPE,
    crate::commands::constants::EVOLUTION_PROPOSAL_TASK_TYPE,
    crate::commands::stock_workflow::reflection::LESSON_VALIDATION_TASK_TYPE,
    crate::commands::constants::PORTFOLIO_METRICS_TASK_TYPE,
    crate::commands::constants::INTENT_EXPIRY_TASK_TYPE,
];

/// 「立即执行」白名单的只读入口。
///
/// 供种子侧的齐备性判据核对（`init/stock_automation_cron_seed.rs`
/// 的 `every_builtin_job_is_manually_executable`）：内置任务若漏登记这里，
/// 任务中心的「立即执行」会报 `MANUAL_RUN_NOT_SUPPORTED` —— 而到点自动执行照常，
/// 所以这个缺陷只有拿真实 task_type 对表才看得见。
///
/// 生产侧的手动执行门直接引用 `BUILTIN_TASK_TYPES` 常量（见同文件 `run_scheduled_task`），
/// 故此访问器仅测试 target 需要，`#[cfg(test)]` 避免在 lib 产物里留死代码。
#[cfg(test)]
pub fn builtin_task_types() -> &'static [&'static str] {
    BUILTIN_TASK_TYPES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskConfigDto {
    pub timeout_seconds: u32,
    pub retry_on_failure: bool,
    pub max_retries: u32,
    pub retry_delay_seconds: u32,
    pub notification_enabled: bool,
    pub run_on_startup: bool,
}

/// CronJob → 前端 DTO。
///
/// `pub(crate)`：其他 domain 装配的定时任务（如需求订阅扫描）复用同一份
/// 序列化契约，前端无需为每种任务类型写两套解析。
pub(crate) fn cron_to_dto(job: &CronJob) -> ScheduledTaskDto {
    let status = match job.status {
        CronJobStatus::Active => "active",
        CronJobStatus::Paused => "paused",
        CronJobStatus::Disabled => "disabled",
    };
    ScheduledTaskDto {
        id: job.id.clone(),
        name: job.name.clone(),
        description: job.description.clone(),
        task_type: job.task_type.clone().unwrap_or_else(|| "custom".to_string()),
        cron_expression: if job.schedule.is_empty() {
            None
        } else {
            Some(job.schedule.clone())
        },
        interval_seconds: None,
        next_run_at: job.next_run_at.map(|ts| {
            chrono::DateTime::from_timestamp_millis(ts)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        }),
        last_run_at: job.last_run_at.map(|ts| {
            chrono::DateTime::from_timestamp_millis(ts)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        }),
        last_result: job.last_result.as_ref().map(|r| TaskRunResultDto {
            success: r.success,
            output: r.output.clone(),
            error: r.error.clone(),
            duration_ms: r.duration_ms,
            skipped: r.skipped,
        }),
        status: status.to_string(),
        config: TaskConfigDto {
            timeout_seconds: job.config.timeout_seconds,
            retry_on_failure: job.config.retry_on_failure,
            max_retries: job.config.max_retries,
            retry_delay_seconds: job.config.retry_delay_seconds,
            notification_enabled: job.config.notification_enabled,
            run_on_startup: job.config.run_on_startup,
        },
        created_at: chrono::DateTime::from_timestamp_millis(job.created_at)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
        updated_at: chrono::DateTime::from_timestamp_millis(job.updated_at)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default(),
        workflow_id: job.workflow_id.clone(),
        params: job.params.clone(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskInput {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "cronExpression")]
    pub cron_expression: Option<String>,
    #[serde(rename = "taskType")]
    pub task_type: Option<String>,
    #[serde(rename = "workflowId")]
    pub workflow_id: Option<String>,
    pub enabled: Option<bool>,
    pub config: Option<TaskConfigDto>,
    /// 结构化业务参数
    pub params: Option<serde_json::Value>,
}

#[agent_command(domain = scheduled_task, safety = Safe, call_mode = StateOnly, description = "列出定时任务")]
#[tauri::command]
pub async fn list_scheduled_tasks(
    state: State<'_, AppState>,
) -> Result<Vec<ScheduledTaskDto>, String> {
    require_operator(&state)?;
    let jobs = state.cron_job_store.list().await;
    Ok(jobs.iter().map(cron_to_dto).collect())
}

#[agent_command(domain = scheduled_task, safety = Safe, call_mode = StateInput, description = "获取定时任务详情")]
#[tauri::command]
pub async fn get_scheduled_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<ScheduledTaskDto>, String> {
    require_operator(&state)?;
    let job = state.cron_job_store.get(&task_id).await;
    Ok(job.as_ref().map(cron_to_dto))
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateInput, description = "创建定时任务")]
#[tauri::command]
pub async fn create_scheduled_task(
    state: State<'_, AppState>,
    input: CreateTaskInput,
) -> Result<ScheduledTaskDto, String> {
    require_operator(&state)?;
    // SECURITY (M6): cron 表达式必须在 5 字段范围内，并显式限制频率（最小 60s 间隔）。
    if let Some(ref cron) = input.cron_expression {
        validate_cron_expression(cron)?;
    }
    let schedule = input.cron_expression.unwrap_or_else(|| "0 9 * * *".to_string());
    let desc = input.description.unwrap_or_else(|| input.name.clone());
    let mut job = CronJob::new(&input.name, &schedule, &desc, &desc);
    if let Some(ref task_type) = input.task_type {
        job.task_type = Some(task_type.clone());
    }
    if let Some(ref wf_id) = input.workflow_id {
        job.workflow_id = Some(wf_id.clone());
    }
    if input.params.is_some() {
        job.params = input.params.clone();
    }
    if let Some(ref cfg) = input.config {
        job.config = TaskConfig {
            timeout_seconds: cfg.timeout_seconds,
            retry_on_failure: cfg.retry_on_failure,
            max_retries: cfg.max_retries,
            retry_delay_seconds: cfg.retry_delay_seconds,
            notification_enabled: cfg.notification_enabled,
            run_on_startup: cfg.run_on_startup,
        };
    }
    if input.enabled == Some(false) {
        job.status = CronJobStatus::Paused;
    }
    state.cron_job_store.add(job.clone()).await;
    Ok(cron_to_dto(&job))
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateInput, description = "更新定时任务")]
#[tauri::command]
pub async fn update_scheduled_task(
    state: State<'_, AppState>,
    task_id: String,
    task: ScheduledTaskDto,
) -> Result<(), String> {
    require_operator(&state)?;
    if let Some(ref cron) = task.cron_expression {
        validate_cron_expression(cron)?;
    }
    state
        .cron_job_store
        .update(&task_id, |job| {
            job.name = task.name.clone();
            job.description = task.description.clone();
            if let Some(ref cron) = task.cron_expression {
                job.schedule = cron.clone();
            }
            job.task_type = Some(task.task_type.clone());
            job.workflow_id = task.workflow_id.clone();
            job.params = task.params.clone();
            job.config = TaskConfig {
                timeout_seconds: task.config.timeout_seconds,
                retry_on_failure: task.config.retry_on_failure,
                max_retries: task.config.max_retries,
                retry_delay_seconds: task.config.retry_delay_seconds,
                notification_enabled: task.config.notification_enabled,
                run_on_startup: task.config.run_on_startup,
            };
            job.status = match task.status.as_str() {
                "active" => CronJobStatus::Active,
                "paused" => CronJobStatus::Paused,
                _ => CronJobStatus::Disabled,
            };
        })
        .await;
    Ok(())
}

#[agent_command(domain = scheduled_task, safety = Dangerous, call_mode = StateInput, description = "删除定时任务")]
#[tauri::command]
pub async fn delete_scheduled_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    require_operator(&state)?;
    state.cron_job_store.remove(&task_id).await;
    Ok(())
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateInput, description = "暂停定时任务")]
#[tauri::command]
pub async fn pause_scheduled_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    require_operator(&state)?;
    state.cron_job_store.set_status(&task_id, CronJobStatus::Paused).await;
    Ok(())
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateInput, description = "恢复定时任务")]
#[tauri::command]
pub async fn resume_scheduled_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    require_operator(&state)?;
    state.cron_job_store.set_status(&task_id, CronJobStatus::Active).await;
    Ok(())
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateInput, description = "手动执行定时任务")]
#[tauri::command]
pub async fn execute_scheduled_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<TaskRunResultDto, String> {
    require_operator(&state)?;
    let job = state.cron_job_store.get(&task_id).await.ok_or_else(|| {
        ErrorResponse::err_with_detail(task_err::NOT_FOUND, format!("Task not found: {}", task_id))
    })?;

    let started = axagent_runtime_core::cron_job::now_millis();

    // 分支 1：关联了工作流 → 真正执行工作流并等待完成
    if let Some(ref wf_id) = job.workflow_id {
        let opts = axagent_runtime::work_engine::RunOptions::default();
        let result = match state.work_engine.run_workflow(wf_id, opts).await {
            Ok(workflow) => {
                tracing::info!(
                    "[execute_scheduled_task] 工作流任务 '{}' 手动执行完成: {:?}",
                    job.name,
                    workflow.status
                );
                axagent_runtime_core::TaskRunResult {
                    success: true,
                    output: Some(format!("{:?}", workflow.status)),
                    error: None,
                    duration_ms: (axagent_runtime_core::cron_job::now_millis() - started) as u64,
                    executed_at: started,
                    skipped: false,
                }
            },
            Err(e) => {
                let err_msg = format!("{:?}", e);
                tracing::error!(
                    "[execute_scheduled_task] 工作流任务 '{}' 手动执行失败: {err_msg}",
                    job.name
                );
                axagent_runtime_core::TaskRunResult {
                    success: false,
                    output: None,
                    error: Some(err_msg),
                    duration_ms: (axagent_runtime_core::cron_job::now_millis() - started) as u64,
                    executed_at: started,
                    skipped: false,
                }
            },
        };

        state.cron_job_store.record_run(&task_id, result.clone()).await;
        return Ok(TaskRunResultDto {
            success: result.success,
            output: result.output,
            error: result.error,
            duration_ms: result.duration_ms,
            skipped: false,
        });
    }

    // 分支 2：无工作流、但 task_type 有内置 handler（荐股 / 扫描 / 反思 / 回测等）
    // → 复用定时调度的 executor 链（含交易日闸门），由 handler 内部自行 record_run。
    // 此前这里直接记一条「已手动触发（无关联工作流）」的假成功，真实逻辑从未执行。
    if let Some(task_type) = job.task_type.as_deref()
        && BUILTIN_TASK_TYPES.contains(&task_type)
    {
        if let Some(scheduler) = state.cron_scheduler.read().await.as_ref() {
            scheduler.trigger_now(job.clone()).await;
            tracing::info!(
                "[execute_scheduled_task] 内置任务 '{}' ({}) 已手动触发",
                job.name,
                task_type
            );
            // 结果由 handler 异步落库（长任务可达数分钟），这里只确认已触发
            return Ok(TaskRunResultDto {
                success: true,
                output: Some(format!("任务 '{}' 已触发，请稍后刷新查看执行结果", job.name)),
                error: None,
                duration_ms: 0,
                skipped: false,
            });
        }
        // 调度器未启动（启动流程异常）：无法安全执行内置 handler
        return Err(ErrorResponse::err_with_detail(
            task_err::MANUAL_RUN_NOT_SUPPORTED,
            format!("内置任务 '{}' 的调度器尚未启动，无法手动执行；请重启应用后重试", job.name),
        ));
    }

    // 分支 3：既无关联工作流、也无内置 handler（custom/general 空壳任务）
    // → 明确报错，不再记假成功、不污染执行历史
    Err(ErrorResponse::err_with_detail(
        task_err::MANUAL_RUN_NOT_SUPPORTED,
        format!(
            "任务 '{}' 没有关联工作流，也没有内置执行器，无法手动执行；\
             请先为其关联工作流或选择一种内置任务类型",
            job.name
        ),
    ))
}

#[agent_command(domain = scheduled_task, safety = Safe, call_mode = StateOnly, description = "获取定时任务模板")]
#[tauri::command]
pub async fn get_scheduled_task_templates(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    require_operator(&state)?;
    use axagent_entities::workflow_template;
    use sea_orm::EntityTrait;

    let mut templates: Vec<serde_json::Value> = vec![
        serde_json::json!({"templateType":"custom","name":"自定义任务","description":"自定义 cron 定时任务","template_type":"custom","default_schedule":"0 9 * * *"}),
    ];

    // 查询所有已持久化的工作流模板
    if let Ok(wf_templates) = workflow_template::Entity::find().all(state.harness.db()).await {
        for wt in wf_templates {
            templates.push(serde_json::json!({
                "templateType": wt.id,
                "name": wt.name,
                "description": wt.description.unwrap_or_default(),
                "template_type": "workflow",
                "workflow_id": wt.id,
                "default_schedule": "0 9 * * *",
            }));
        }
    }

    Ok(templates)
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateInput, description = "创建每日摘要任务")]
#[tauri::command]
pub async fn create_daily_summary_task(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
    cron_expression: Option<String>,
) -> Result<ScheduledTaskDto, String> {
    require_operator(&state)?;
    if let Some(ref c) = cron_expression {
        validate_cron_expression(c)?;
    }
    let schedule = cron_expression.unwrap_or_else(|| "0 9 * * *".to_string());
    let desc = description.unwrap_or_else(|| name.clone());
    let mut job = CronJob::new(&name, &schedule, &desc, &desc);
    job.task_type = Some("daily_summary".to_string());
    state.cron_job_store.add(job.clone()).await;
    Ok(cron_to_dto(&job))
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateInput, description = "创建备份任务")]
#[tauri::command]
pub async fn create_backup_task(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
    cron_expression: Option<String>,
) -> Result<ScheduledTaskDto, String> {
    require_operator(&state)?;
    if let Some(ref c) = cron_expression {
        validate_cron_expression(c)?;
    }
    let schedule = cron_expression.unwrap_or_else(|| "0 2 * * *".to_string());
    let desc = description.unwrap_or_else(|| name.clone());
    let mut job = CronJob::new(&name, &schedule, &desc, &desc);
    job.task_type = Some("backup".to_string());
    state.cron_job_store.add(job.clone()).await;
    Ok(cron_to_dto(&job))
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateInput, description = "创建清理任务")]
#[tauri::command]
pub async fn create_cleanup_task(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
    cron_expression: Option<String>,
) -> Result<ScheduledTaskDto, String> {
    require_operator(&state)?;
    if let Some(ref c) = cron_expression {
        validate_cron_expression(c)?;
    }
    let schedule = cron_expression.unwrap_or_else(|| "0 3 * * 0".to_string());
    let desc = description.unwrap_or_else(|| name.clone());
    let mut job = CronJob::new(&name, &schedule, &desc, &desc);
    job.task_type = Some("cleanup".to_string());
    state.cron_job_store.add(job.clone()).await;
    Ok(cron_to_dto(&job))
}

#[agent_command(domain = scheduled_task, safety = Caution, call_mode = StateOnly, description = "从数据库加载定时任务")]
#[tauri::command]
pub async fn load_scheduled_tasks_from_db(state: State<'_, AppState>) -> Result<usize, String> {
    let count = state.cron_job_store.reload_from_db().await;
    tracing::info!("[scheduled_task] 从 DB 重新加载了 {count} 个定时任务");
    Ok(count)
}

#[agent_command(domain = scheduled_task, safety = Safe, call_mode = StateInput, description = "获取任务执行历史")]
#[tauri::command]
pub async fn get_task_execution_history(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<ExecutionRecord>, String> {
    require_operator(&state)?;
    Ok(state.cron_job_store.get_execution_history(&task_id).await)
}
