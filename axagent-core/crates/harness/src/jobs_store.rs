// SPDX-License-Identifier: AGPL-3.0-only

//! 定时任务（cron job）查询接缝 — DTO + Trait。
//!
//! 宿主把 `runtime-core` 的 `CronJobStore` 经本接缝暴露给插件侧；
//! 插件经 `call_seam("jobs.store", op, args)` 按 op 查询 / 新增 / 删除 / 改状态。
//!
//! **写面权限**：`add` / `remove` / `set_status` 要求插件已声明 `write` 权限
//! （由 worker 分派层强制；本层只提供契约，不自行校验）。`update`（patch）不在本批。
//! 内置种子任务受保护：`remove` 对保护 id 返回 [`RemoveJobOutcome::Protected`]，
//! **不删除**；改状态不受保护限制（允许用户暂停内置任务）。
//!
//! 查询视图刻意**不暴露**以下内容：
//! - `config`（重试/超时配置）与 `delivery`（投递配置）：属宿主内部执行参数；
//! - 进程内在跑登记（`running`）：纯内存且随进程退出清空，跨边界无意义；
//! - `last_result.output`：单次执行输出可能很大，不过帧（见 [`CronJobRunView`]）。
//!
//! `task_type` 过滤为 `None` 时返回全部任务（不过滤），`Some(t)` 时只返回
//! `task_type == Some(t)` 的任务。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};

/// 单次执行结果视图（`last_result` 的跨边界形态）。
///
/// 与宿主内部 `TaskRunResult` 的区别：**不携带 `output`**（避免大文本过帧）；
/// `partial` 保留，供面板区分「部分完成」与「失败」。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobRunView {
    pub success: bool,
    pub skipped: bool,
    pub partial: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub executed_at: i64,
}

/// 定时任务只读视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schedule: String,
    pub prompt: String,
    pub workflow_id: Option<String>,
    pub task_type: Option<String>,
    pub platform: Option<String>,
    pub enabled_toolsets: Option<Vec<String>>,
    /// 三态状态，字面量与宿主持久化一致：`"active"` | `"paused"` | `"disabled"`。
    pub status: String,
    pub recurring: bool,
    pub run_count: u32,
    pub last_run_at: Option<i64>,
    pub last_result: Option<CronJobRunView>,
    pub last_trigger: Option<String>,
    pub next_run_at: Option<i64>,
    pub priority: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新建定时任务的入参（`jobs.store` 的 `add` op）。
///
/// `id` 由调用方指定：须非空且在宿主内唯一（重复由实现方拒绝，避免静默覆盖）。
/// 宿主侧固定初始状态为 `active`、`run_count = 0`，时间戳由宿主生成。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCronJob {
    /// 调用方指定的任务 id：非空、唯一。
    pub id: String,
    pub name: String,
    pub description: String,
    /// cron 表达式（如 `"0 9 * * *"`），非空。
    pub schedule: String,
    pub prompt: String,
    pub workflow_id: Option<String>,
    pub task_type: Option<String>,
    pub platform: Option<String>,
    pub enabled_toolsets: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub recurring: bool,
    /// 优先级字面量：`low` / `medium` / `high` / `batch`，缺省 `medium`。
    #[serde(default = "default_priority")]
    pub priority: String,
    pub params: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}

fn default_priority() -> String {
    "medium".to_string()
}

/// `remove` 的结果：区分「已删除」「不存在」「受保护未删除」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoveJobOutcome {
    Removed,
    NotFound,
    /// 内置种子任务：受保护，**未删除**。
    Protected,
}

/// 定时任务查询提供者（读面 + 写面）。
///
/// 实现方：宿主 `runtime-core` 的 `CronJobStore`（wiring 在 `src/init/state.rs`）。
/// 默认实现统一返回「不支持此数据面」，便于外部实现只覆盖关心的 op。
///
/// 写方法（`add_job` / `remove_job` / `set_job_status`）要求插件已声明 `write`
/// 权限；该检查在 worker 分派层完成（返回 `PLUGIN_PERMISSION_DENIED`），本 trait 不重复校验。
#[async_trait]
pub trait JobStoreProvider: Send + Sync {
    /// 列出任务。`task_type = None` 返回全部；`active_only = true` 只返回
    /// `status == "active"` 的任务。
    async fn list_jobs(
        &self,
        _task_type: Option<&str>,
        _active_only: bool,
    ) -> Result<Vec<CronJobView>> {
        Err(AxAgentError::Provider(
            "JobStoreProvider::list_jobs 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 按 id 取单条任务；不存在返回 `Ok(None)`。
    async fn get_job(&self, _id: &str) -> Result<Option<CronJobView>> {
        Err(AxAgentError::Provider(
            "JobStoreProvider::get_job 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 统计任务数。`task_type = None` 统计全部。
    async fn count_jobs(&self, _task_type: Option<&str>) -> Result<u64> {
        Err(AxAgentError::Provider(
            "JobStoreProvider::count_jobs 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 新增任务，返回任务 id。要求插件已声明 `write` 权限（分派层强制）。
    ///
    /// 实现方须校验 `id` / `schedule` 非空、`id` 未重复；宿主固定初始状态为
    /// `active`，并立即编排下次触发时刻。
    async fn add_job(&self, _job: NewCronJob) -> Result<String> {
        Err(AxAgentError::Provider(
            "JobStoreProvider::add_job 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 删除任务。要求插件已声明 `write` 权限（分派层强制）。
    ///
    /// 内置种子任务 id 返回 [`RemoveJobOutcome::Protected`] 且**不删除**；
    /// 不存在返回 [`RemoveJobOutcome::NotFound`]。
    async fn remove_job(&self, _id: &str) -> Result<RemoveJobOutcome> {
        Err(AxAgentError::Provider(
            "JobStoreProvider::remove_job 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 修改任务状态：仅接受 `"active"` / `"paused"` / `"disabled"`。
    ///
    /// 要求插件已声明 `write` 权限（分派层强制）。非法状态返回
    /// `Provider` 错误（跨层后无法区分参数错误与提供者错误，故不走 `invalid_args`）。
    /// 返回 `false` 表示任务不存在。受保护 id **允许**改状态。
    async fn set_job_status(&self, _id: &str, _status: &str) -> Result<bool> {
        Err(AxAgentError::Provider(
            "JobStoreProvider::set_job_status 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}
