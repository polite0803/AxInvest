// SPDX-License-Identifier: AGPL-3.0-only

//! 定时任务 Tauri 命令 — 基于 CronJobStore 统一调度系统。
//! 命令名保持与旧 ScheduledTaskService 兼容，供前端 SchedulerSettings 调用。

use axagent_dao::db::DatabaseConnection;
use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_department as dept_repo;
use axagent_dao::repo::bot_workflow as workflow_repo;
use axagent_dao::repo::conversation as conv_repo;

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::bot as bot_err;
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
    match validate_cron_detail(cron) {
        Ok(()) => Ok(()),
        Err(detail) => Err(ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            detail,
        )),
    }
}

/// 纯校验核心：非法时返回失败 detail（与 IPC-P2 §2.1 的「包文案」口径对应），
/// 合法返回 `()`。错误码由调用方决定：旧路径（`validate_cron_expression`）仍是
/// `COMMON_INVALID_INPUT`（IPC 文中 ERR_VALIDATION 为占位写法）；Bot 例程路径
/// 用 `BOT_SCHEDULE_CRON_INVALID`。
pub(crate) fn validate_cron_detail(cron: &str) -> Result<(), String> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!("Cron must have exactly 5 fields, got {}: '{}'", parts.len(), cron));
    }
    // 检查 minute 字段（第一字段），拒绝过于频繁的执行
    let minute = parts[0];
    if minute == "*" || minute == "*/1" {
        return Err("Cron 表达式过于频繁：不允许每分钟执行，最小间隔为 2 分钟".to_string());
    }
    // 检查 */N 模式中 N < 1 的情况（实际上 N=1 已被上面覆盖）
    if let Some(rest) = minute.strip_prefix("*/") {
        if let Ok(n) = rest.parse::<u32>() {
            if n < 1 {
                return Err("Cron interval less than 1 minute is not allowed".to_string());
            }
        }
    }
    Ok(())
}

/// P2-a（IPC-P2 §2.1）：IANA 时区名校验 —— `chrono_tz::Tz::from_str` 可解析才算合法。
///
/// `pub(crate)`：Bot 便捷命令（`commands/bot.rs`）复用同一校验，避免两边口径分叉。
/// `None`（未给时区）= 宿主 Local，是合法值。
pub(crate) fn validate_timezone(timezone: Option<&str>) -> Result<(), String> {
    let Some(tz) = timezone.map(str::trim).filter(|t| !t.is_empty()) else {
        // None 或 trim 后空白 = 宿主 Local，与 `normalized_timezone` 同一口径。
        return Ok(());
    };
    if tz.parse::<chrono_tz::Tz>().is_ok() {
        return Ok(());
    }
    Err(ErrorResponse::err_with_detail(
        bot_err::SCHEDULE_TIMEZONE_INVALID,
        format!("Unknown IANA time zone: '{tz}'"),
    ))
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
    /// 本进程内该任务**正在执行**的开始时刻（ms）；null = 没在跑。
    ///
    /// 来自 `CronJobStore` 的纯内存登记，不是 DB 状态 ⇒ 进程重启自然清空，
    /// 不会留下假的「运行中」。
    #[serde(default)]
    pub running_since: Option<i64>,
    /// 在跑的触发来源（`manual` / `schedule`）；null = 没在跑。
    #[serde(default)]
    pub running_trigger: Option<String>,
    /// 最近一次执行的触发来源；null = 本列引入前的旧记录（未知）。
    #[serde(default)]
    pub last_trigger: Option<String>,
    /// P2-a：例程归属的 Bot id；null = 普通（非 Bot）定时任务。
    #[serde(default)]
    pub bot_id: Option<String>,
    /// P2-a：档期时区（IANA 名）；null = 宿主 Local（与现状一致）。
    #[serde(default)]
    pub timezone: Option<String>,
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
    /// P1-10：批量任务「部分完成」（超墙钟预算收尾 / 单轮上限截断）——
    /// 前端据此显示「部分完成」徽章而非「失败」（旧记录缺省 false）。
    #[serde(default)]
    pub partial: bool,
    /// 本次结果是否**等到了终态**。true ⇒ `output` 是这一轮的结论，前端应直接展示；
    /// false ⇒ 只是「已触发」（长任务异步落库），前端只能提示稍后刷新。
    /// 没有这个字段时，前端无法区分两种 output，只能一律弹 toast（2026-10-21 实机反馈）。
    #[serde(default)]
    pub awaited: bool,
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
    // 漏检事件补分析（2026-10-20 A-1）：与种子、services.rs 分支、前端 taskGrouping.ts 同步。
    crate::commands::constants::MOVER_RECALL_ANALYSIS_TASK_TYPE,
    crate::commands::constants::UNIVERSE_SCAN_TASK_TYPE,
    crate::commands::constants::EVOLUTION_PROPOSAL_TASK_TYPE,
    crate::commands::constants::LESSON_VALIDATION_TASK_TYPE,
    crate::commands::constants::PORTFOLIO_METRICS_TASK_TYPE,
    crate::commands::constants::INTENT_EXPIRY_TASK_TYPE,
    // 中断分析自动重跑（2026-10-17）。⚠ 与 `init/services.rs` 的 cron executor 分支、
    // 以及前端 `taskGrouping.ts` 的两个白名单必须**同步**，漏一处 =
    // 「立即执行」报 MANUAL_RUN_NOT_SUPPORTED（而到点自动执行照常 ⇒ 只在手动时露头）。
    crate::commands::constants::STOCK_ANALYSIS_RERUN_TASK_TYPE,
    // 每日时间旅行回放（R-2，2026-10-26）：executor 分支在 `init/services.rs`（且进交易日闸门），
    // 回放日可判定性逐日推进 ⇒ 手动触发就是「今天补跑一次攒样本」，与到点自动执行同一条路径。
    crate::commands::constants::TIME_TRAVEL_BATCH_TASK_TYPE,
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
            // 已落库的 last_result 是**跑完的结果** ⇒ output 就是结论（awaited=true）。
            // 这个字段只服务「手动执行刚返回时」的区分，列表回读不受影响。
            success: r.success,
            output: r.output.clone(),
            error: r.error.clone(),
            duration_ms: r.duration_ms,
            skipped: r.skipped,
            partial: r.partial,
            awaited: true,
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
        // 在跑字段由 list/get 命令叠加（cron_to_dto 只有任务定义，看不到进程内登记）。
        running_since: None,
        running_trigger: None,
        last_trigger: job.last_trigger.clone(),
        bot_id: job.bot_id.clone(),
        timezone: job.timezone.clone(),
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
    /// P2-a：例程归属的 Bot id；给出时校验 Bot 存在。不传 = 普通定时任务（行为不变）。
    #[serde(rename = "botId", default)]
    pub bot_id: Option<String>,
    /// P2-a：档期时区（IANA 名）；null/缺省 = 宿主 Local。
    #[serde(default)]
    pub timezone: Option<String>,
}

/// 归一化时区入参：trim 后空白视为 None（宿主 Local）。
fn normalized_timezone(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// 把进程内「在跑」登记叠加到 DTO（`cron_to_dto` 只看得见任务定义）。
fn overlay_running(
    mut dto: ScheduledTaskDto,
    running: &std::collections::HashMap<String, axagent_runtime_core::cron_job::RunningJob>,
) -> ScheduledTaskDto {
    if let Some(r) = running.get(&dto.id) {
        dto.running_since = Some(r.started_at_ms);
        dto.running_trigger = Some(r.trigger.to_string());
    }
    dto
}

#[agent_command(domain = scheduled_task, safety = Safe, call_mode = StateOnly, description = "列出定时任务")]
#[tauri::command]
pub async fn list_scheduled_tasks(
    state: State<'_, AppState>,
    bot_id: Option<String>,
) -> Result<Vec<ScheduledTaskDto>, String> {
    require_operator(&state)?;
    // P2-a（IPC-P2 §2.4）：bot_id 给出时按 data.botId 过滤（不存在 -> BOT_NOT_FOUND）；
    // 不传返回全量（现状不变）。JSON 字段不可 SQL 索引（GAP G-10），命令层过滤。
    if let Some(ref bid) = bot_id {
        ensure_bot_exists(&state.harness.db(), bid).await?;
    }
    let jobs = state.cron_job_store.list().await;
    let running = state.cron_job_store.list_running().await;
    Ok(jobs
        .iter()
        .filter(|job| match &bot_id {
            Some(bid) => job.bot_id.as_deref() == Some(bid.as_str()),
            None => true,
        })
        .map(|job| overlay_running(cron_to_dto(job), &running))
        .collect())
}

#[agent_command(domain = scheduled_task, safety = Safe, call_mode = StateInput, description = "获取定时任务详情")]
#[tauri::command]
pub async fn get_scheduled_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<ScheduledTaskDto>, String> {
    require_operator(&state)?;
    let job = state.cron_job_store.get(&task_id).await;
    let running = state.cron_job_store.list_running().await;
    Ok(job.as_ref().map(|j| overlay_running(cron_to_dto(j), &running)))
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
    validate_timezone(input.timezone.as_deref())?;
    if let Some(ref bot_id) = input.bot_id {
        // P2-a：botId 给出时校验 Bot 存在；旧调用不传，行为逐字节不变。
        ensure_bot_exists(&state.harness.db(), bot_id).await?;
    }
    let schedule = input.cron_expression.unwrap_or_else(|| "0 9 * * *".to_string());
    let desc = input.description.unwrap_or_else(|| input.name.clone());
    let mut job = CronJob::new(&input.name, &schedule, &desc, &desc);
    if let Some(ref task_type) = input.task_type {
        job.task_type = Some(task_type.clone());
    }
    job.bot_id = input.bot_id.clone();
    job.timezone = normalized_timezone(input.timezone.clone());
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
    timezone: Option<Option<String>>,
    bot_id: Option<Option<String>>,
) -> Result<(), String> {
    require_operator(&state)?;
    // P2-a（IPC-P2 §2.5）双层 Option 清除语义：外层 None = 不改；Some(None) = 清除；
    // Some(Some(v)) = 设置并校验。旧调用不传新参 => 双 None => 行为不变。
    if let Some(Some(ref tz)) = timezone {
        validate_timezone(Some(tz))?;
    }
    if let Some(Some(ref bid)) = bot_id {
        ensure_bot_exists(&state.harness.db(), bid).await?;
    }
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
            match timezone {
                None => {},
                Some(None) => job.timezone = None,
                Some(Some(ref tz)) => job.timezone = Some(tz.clone()),
            }
            match bot_id {
                None => {},
                Some(None) => job.bot_id = None,
                Some(Some(ref bid)) => job.bot_id = Some(bid.clone()),
            }
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
        // 手动执行同样登记「在跑」：长工作流期间任务中心能显示运行中，record_run 时摘除。
        state.cron_job_store.mark_running(&task_id, started, "manual").await;
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
                    partial: false,
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
                    partial: false,
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
            partial: result.partial,
            awaited: true,
        });
    }

    // 分支 1.5：漏检核查是**快**任务（一次库内扫描），手动执行必须等结果。
    // 走分支 2 的 `scheduler.trigger_now` 只会返回「已触发」，用户点完什么都看不到
    // （2026-10-21 实机反馈：计划任务页只弹一个 toast）。
    // 这里直接复用 cron 的同一实现体，并以 `manual` 落 trigger_source（F-15 的区分）。
    if job.task_type.as_deref() == Some(crate::commands::mover_recall::MOVER_RECALL_TASK_TYPE) {
        // 手动快路径同样登记「在跑」，让任务中心看到「手动」而不是干等。
        state.cron_job_store.mark_running(&task_id, started, "manual").await;
        let result = match crate::commands::mover_recall::run_mover_recall_check(
            state.harness.db(),
            &state.stock().astock_client,
            "manual",
        )
        .await
        {
            Ok(o) => axagent_runtime_core::TaskRunResult {
                success: true,
                output: Some(o.summary),
                error: None,
                duration_ms: (axagent_runtime_core::cron_job::now_millis() - started) as u64,
                executed_at: started,
                skipped: false,
                partial: false,
            },
            Err(e) => axagent_runtime_core::TaskRunResult {
                success: false,
                output: None,
                error: Some(e),
                duration_ms: (axagent_runtime_core::cron_job::now_millis() - started) as u64,
                executed_at: started,
                skipped: false,
                partial: false,
            },
        };
        state.cron_job_store.record_run(&task_id, result.clone()).await;
        return Ok(TaskRunResultDto {
            success: result.success,
            output: result.output,
            error: result.error,
            duration_ms: result.duration_ms,
            skipped: false,
            partial: result.partial,
            awaited: true,
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
                partial: false,
                awaited: false,
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

// ───────────────────────────── Bot 例程便捷命令（P2-a，IPC-P2 §2.1/§2.2）─────────────────────────────
//
// Q3 裁定（PLAN-P2 开放问题）：Bot 语义入口与本命令族共用同一套校验/落库/DTO，
// 避免两条口径；物理上也落在本模块（check:layers 规则 2：命令兄弟模块不得横向互调）。

/// Bot 例程 DTO 中的 task_type 预设值。executor 分支（init/services.rs）据此识别 Bot 例程。
pub(crate) const BOT_ROUTINE_TASK_TYPE: &str = "bot_routine";
/// P3（IPC-P3 §2.4）：Bot 编排定时触发的 task_type 预设值。
pub(crate) const BOT_WORKFLOW_TASK_TYPE: &str = "bot_workflow";

/// P3 T4：cron 分支判定 —— 只有 `task_type="bot_workflow"` 的作业走编排执行路径。
pub(crate) fn is_bot_workflow_job(job: &CronJob) -> bool {
    job.task_type.as_deref() == Some(BOT_WORKFLOW_TASK_TYPE)
}

/// P3 T4：从编排定时作业 params 读取 workflowId（兼容历史 snake_case 写法）。
pub(crate) fn bot_workflow_id_from_job(job: &CronJob) -> Option<String> {
    if !is_bot_workflow_job(job) {
        return None;
    }
    job.params
        .as_ref()
        .and_then(|p| p.get("workflowId").or_else(|| p.get("workflow_id")))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// P2-a（IPC-P2 §2.1/§2.2）：Bot id 存在性校验；不存在 -> `BOT_NOT_FOUND`。
pub(crate) async fn ensure_bot_exists(db: &DatabaseConnection, bot_id: &str) -> Result<(), String> {
    if bot_repo::get_bot(db, bot_id).await.is_ok() {
        return Ok(());
    }
    Err(ErrorResponse::new(bot_err::NOT_FOUND)
        .with_detail(format!("Bot not found: {bot_id}"))
        .to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBotScheduledTaskInput {
    pub bot_id: String,
    pub name: String,
    /// Bot 例程跑 agent_query 的 prompt（非空）
    pub prompt: String,
    #[serde(rename = "cronExpression")]
    pub cron_expression: String,
    /// IANA 时区名；null/缺省 = 宿主 Local
    pub timezone: Option<String>,
    /// 缺省 true
    pub enabled: Option<bool>,
}

fn validate_bot_routine_prompt(prompt: &str) -> Result<(), String> {
    if prompt.trim().is_empty() {
        return Err(ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            "prompt must not be blank",
        ));
    }
    Ok(())
}

/// IPC-P2 §2.1：Bot 例程创建 —— 预设 task_type = "bot_routine"，其余语义同 §2.3。
/// cron 失败用专用码 `BOT_SCHEDULE_CRON_INVALID` 包同一文案，前端免解析自由字符串。
#[tauri::command]
pub async fn create_bot_scheduled_task(
    state: State<'_, AppState>,
    input: CreateBotScheduledTaskInput,
) -> Result<ScheduledTaskDto, String> {
    validate_bot_routine_prompt(&input.prompt)?;
    validate_cron_detail(&input.cron_expression)
        .map_err(|detail| ErrorResponse::err_with_detail(bot_err::SCHEDULE_CRON_INVALID, detail))?;
    create_scheduled_task(
        state,
        CreateTaskInput {
            name: input.name,
            description: None,
            cron_expression: Some(input.cron_expression),
            task_type: Some(BOT_ROUTINE_TASK_TYPE.to_string()),
            workflow_id: None,
            enabled: input.enabled,
            config: None,
            params: None,
            bot_id: Some(input.bot_id),
            timezone: input.timezone,
        },
    )
    .await
}

/// IPC-P2 §2.2：按 Bot id 过滤的例程清单（含 BOT_NOT_FOUND 校验）。
#[tauri::command]
pub async fn list_bot_scheduled_tasks(
    state: State<'_, AppState>,
    bot_id: String,
) -> Result<Vec<ScheduledTaskDto>, String> {
    list_scheduled_tasks(state, Some(bot_id)).await
}

#[cfg(test)]
mod p2_bot_schedule_tests {
    use super::*;

    // P2-a（IPC-P2 §2.1）：坏时区 -> BOT_SCHEDULE_TIMEZONE_INVALID；
    // None / 合法时区通过。
    #[test]
    fn validate_timezone_rejects_unknown_iana_name() {
        let err = validate_timezone(Some("Not/AZone")).unwrap_err();
        assert!(
            err.contains("BOT_SCHEDULE_TIMEZONE_INVALID"),
            "错误码必须是 BOT_SCHEDULE_TIMEZONE_INVALID: {err}"
        );
        assert!(validate_timezone(None).is_ok(), "None = 宿主 Local，合法");
        assert!(validate_timezone(Some("Asia/Shanghai")).is_ok());
        assert!(validate_timezone(Some("UTC")).is_ok());
        assert!(
            validate_timezone(Some("  ")).is_ok(),
            "空白由 normalized_timezone 归一化，此处只验 parse"
        );
    }

    // P2-a：DTO 新字段透传（botId/timezone 序列化为 camelCase）。
    #[test]
    fn dto_serializes_bot_id_and_timezone_camel_case() {
        let mut job = CronJob::new("t", "0 9 * * *", "p", "d");
        job.bot_id = Some("bot-1".to_string());
        job.timezone = Some("Asia/Shanghai".to_string());
        let dto = cron_to_dto(&job);
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["botId"], "bot-1");
        assert_eq!(json["timezone"], "Asia/Shanghai");
    }

    // P2-a：归一化——空白时区回落 None（宿主 Local）。
    #[test]
    fn normalized_timezone_blanks_fall_back_to_none() {
        assert_eq!(normalized_timezone(None), None);
        assert_eq!(normalized_timezone(Some("  ".to_string())), None);
        assert_eq!(
            normalized_timezone(Some(" Asia/Shanghai ".to_string())),
            Some("Asia/Shanghai".to_string())
        );
    }

    // P2-a（IPC-P2 §2.3）：旧 CreateTaskInput JSON（无 botId/timezone 键）
    // 反序列化得 None —— 旧调用行为逐字节不变。
    #[test]
    fn legacy_create_input_json_deserializes_without_new_keys() {
        let json = r#"{"name":"legacy","cronExpression":"0 9 * * *"}"#;
        let input: CreateTaskInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.bot_id, None);
        assert_eq!(input.timezone, None);
    }

    // P2-a（IPC-P2 §2.1）：旧路径 cron 失败仍是既有通用码（行为逐字节不变），
    // detail 文案与纯核心一致。
    #[test]
    fn legacy_cron_path_still_returns_err_validation() {
        let err = validate_cron_expression("* * * * *").unwrap_err();
        assert!(
            err.contains("COMMON_INVALID_INPUT"),
            "旧路径错误码不变（通用 INVALID_INPUT）: {err}"
        );
        assert!(err.contains("每分钟"), "detail 文案保持原样: {err}");
        assert!(validate_cron_expression("0 9 * * *").is_ok());
    }

    // P2-a（IPC-P2 §2.1）：Bot 例程路径 detail 与旧路径一致，仅错误码换成
    // BOT_SCHEDULE_CRON_INVALID（由 commands/bot.rs 包裹，此处验核心）。
    #[test]
    fn cron_detail_core_matches_legacy_text() {
        let detail = validate_cron_detail("0 9").unwrap_err();
        assert!(detail.contains("5 fields"), "detail: {detail}");
        let detail2 = validate_cron_detail("*/1 * * * *").unwrap_err();
        assert!(detail2.contains("每分钟"), "detail: {detail2}");
        assert!(validate_cron_detail("0 9 * * *").is_ok());
    }
}

// ───────────────────────────── Bot 编排定时命令（P3，IPC-P3 §2.4）─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBotWorkflowScheduleInput {
    pub workflow_id: String,
    #[serde(rename = "cronExpression")]
    pub cron_expression: String,
    /// IANA 时区名；null/缺省 = 宿主 Local
    pub timezone: Option<String>,
    /// 缺省 true
    pub enabled: Option<bool>,
}

/// IPC-P3 §2.4：为编排创建定时触发 —— task_type="bot_workflow"、params.workflowId、bot_id=None。
#[tauri::command]
pub async fn create_bot_workflow_schedule(
    state: State<'_, AppState>,
    input: CreateBotWorkflowScheduleInput,
) -> Result<ScheduledTaskDto, String> {
    let db = state.harness.db();
    let workflow = workflow_repo::get_workflow(db, &input.workflow_id).await.map_err(|e| {
        ErrorResponse::err_with_detail(bot_err::WORKFLOW_NOT_FOUND, format!("编排不存在: {e}"))
    })?;

    validate_cron_detail(&input.cron_expression)
        .map_err(|detail| ErrorResponse::err_with_detail(bot_err::SCHEDULE_CRON_INVALID, detail))?;
    validate_timezone(input.timezone.as_deref())?;

    let schedule = input.cron_expression;
    let name = format!("Bot 编排: {}", workflow.name);
    let mut job = CronJob::new(&name, &schedule, &name, &name);
    job.task_type = Some(BOT_WORKFLOW_TASK_TYPE.to_string());
    job.bot_id = None;
    job.timezone = normalized_timezone(input.timezone);
    job.params = Some(serde_json::json!({ "workflowId": workflow.id }));
    if input.enabled == Some(false) {
        job.status = CronJobStatus::Paused;
    }
    state.cron_job_store.add(job.clone()).await;
    Ok(cron_to_dto(&job))
}

/// Cron 触发 Bot 编排时创建持久线程（PLAN-P3 §4）：department 编排归 Chief，否则 bot_id NULL。
pub(crate) async fn create_bot_workflow_persistent_conversation(
    db: &DatabaseConnection,
    workflow: &axagent_harness::types::BotWorkflow,
) -> Result<axagent_harness::types::Conversation, String> {
    let title = format!("Bot 编排定时: {}", workflow.name);
    let bot_id = if workflow.owner_scope == "department" {
        if let Some(dept_id) = workflow.department_id.as_deref() {
            let dept = dept_repo::get_department(db, dept_id).await.map_err(|e| e.to_string())?;
            dept.chief_bot_id
        } else {
            None
        }
    } else {
        None
    };
    conv_repo::create_conversation_with_defaults(
        db,
        &title,
        "",
        "",
        None,
        conv_repo::ConversationParamDefaults::default(),
        bot_id.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod p3_bot_workflow_schedule_tests {
    use super::*;

    fn job(task_type: Option<&str>, params: Option<serde_json::Value>) -> CronJob {
        let mut job = CronJob::new("编排定时", "0 9 * * *", "编排定时", "编排定时");
        job.task_type = task_type.map(str::to_string);
        job.params = params;
        job
    }

    #[test]
    fn bot_workflow_branch_only_matches_declared_task_type() {
        assert!(is_bot_workflow_job(&job(Some("bot_workflow"), None)));
        assert!(!is_bot_workflow_job(&job(Some("bot_routine"), None)));
        assert!(!is_bot_workflow_job(&job(None, None)));
    }

    #[test]
    fn bot_workflow_branch_reads_workflow_id_from_params() {
        let camel = job(Some("bot_workflow"), Some(serde_json::json!({ "workflowId": "wf-1" })));
        assert_eq!(bot_workflow_id_from_job(&camel).as_deref(), Some("wf-1"));

        let legacy = job(Some("bot_workflow"), Some(serde_json::json!({ "workflow_id": "wf-2" })));
        assert_eq!(bot_workflow_id_from_job(&legacy).as_deref(), Some("wf-2"));

        let missing = job(Some("bot_workflow"), Some(serde_json::json!({})));
        assert_eq!(bot_workflow_id_from_job(&missing), None);

        let wrong_type =
            job(Some("bot_routine"), Some(serde_json::json!({ "workflowId": "wf-3" })));
        assert_eq!(bot_workflow_id_from_job(&wrong_type), None);
    }
}
