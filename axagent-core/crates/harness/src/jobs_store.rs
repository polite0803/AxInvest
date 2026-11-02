// SPDX-License-Identifier: AGPL-3.0-only

//! 定时任务（cron job）查询接缝 — DTO + Trait。
//!
//! 宿主把 `runtime-core` 的 `CronJobStore` 经本接缝以**只读视图**暴露给插件侧；
//! 插件经 `call_seam("jobs.store", op, args)` 按 op 查询任务列表 / 单条 / 计数。
//!
//! **只读**：本接缝不提供创建、修改、删除、触发等写面 —— op 级权限校验落地前
//! 不发布任何写 op。视图也刻意**不暴露**以下内容：
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

/// 定时任务查询提供者（只读）。
///
/// 实现方：宿主 `runtime-core` 的 `CronJobStore`（wiring 在 `src/init/state.rs`）。
/// 默认实现统一返回「不支持此数据面」，便于外部实现只覆盖关心的 op。
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
}
