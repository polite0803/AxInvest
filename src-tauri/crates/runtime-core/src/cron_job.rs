// SPDX-License-Identifier: AGPL-3.0-only

//! 统一定时任务数据模型 — 合并 CronJob + ScheduledTaskService。
//!
//! CronJob + CronJobStore — 供 runtime/cron 调度器、tools/cron.rs 工具、
//! 和 src/commands/ Tauri 命令共用。

// ⚠ 本文件**不得**依赖任何实现层 crate（`axagent-entities` / `sea-orm`）——
//   分层规则见 `scripts/check-contracts.mjs` 的 [D] 项：consumer crate
//   （agent / gateway / orchestrator / **runtime-core**）只允许依赖 `axagent-harness`。
//   原实现直接 `use axagent_entities::{cron_job, cron_job_history}`、
//   用 `Schema::create_table_from_entity` 建表、自己 upsert，被该门禁判为
//   「越界依赖实现层」（2026-09-18 修）。
//   DB 访问改走 harness 里的持久化**端口** `CronJobPersistence`，实现落在
//   `axagent-dao::repo::cron_job_persistence` —— 与本 crate 的 `cron_delivery.rs`
//   （「DTO + Trait 在 harness，具体实现在 wiring 层」）同一套 port/adapter 设计。
use axagent_harness::cron_persistence::{CronHistoryRecord, CronJobPersistence};
use chrono::Timelike;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

// G17: 引入 Cron delivery 配置（来自 harness）
pub use axagent_harness::cron_delivery::{
    CronDeliveryChannel, CronDeliveryConfig, CronDeliveryPayload, CronDeliverySink,
};

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ── 建表与列类型修复已下沉到 DAO（2026-09-18）──
//
// 此处原有三个函数：
//   `ensure_table`（用 `Schema::create_table_from_entity` 建 `cron_jobs`）、
//   `ensure_history_table`（建 `cron_job_history`）、
//   `heal_history_timestamps`（把三个时间戳列从 `integer` 迁到 `bigint`，仅 PG）。
// 它们的形状是「DAO 该干的事」，且正是本文件被判「越界依赖实现层」的成因。
// 现整体搬到 `axagent-dao::repo::cron_job_persistence::PgCronJobPersistence::ensure_tables`
// —— 与两条既有修复的逐字说明一起搬（建表语句由实体生成、时间戳必须 BIGINT），
// 本文件只通过端口调用，不再需要任何 backend 判断。

// ── CronJob 最大合集 ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Cron 表达式 (如 "0 9 * * *") 或原始调度字符串
    pub schedule: String,
    /// 任务指令 / prompt
    pub prompt: String,
    /// 关联的工作流 ID
    pub workflow_id: Option<String>,
    /// 任务类型标签 (用于模板查找)
    pub task_type: Option<String>,
    /// 消息平台
    pub platform: Option<String>,
    /// 启用的工具集
    pub enabled_toolsets: Option<Vec<String>>,
    /// 三态状态
    pub status: CronJobStatus,
    /// 是否循环 (false = 一次性)
    pub recurring: bool,
    /// 执行次数
    pub run_count: u32,
    /// 上次执行时间 (epoch millis)
    pub last_run_at: Option<i64>,
    /// 上次执行结果
    pub last_result: Option<TaskRunResult>,
    /// 下次执行时间 (epoch millis)
    pub next_run_at: Option<i64>,
    /// 重试/超时配置
    pub config: TaskConfig,
    /// G17: 执行结果投递配置（可选，None 表示不投递）
    #[serde(default)]
    pub delivery: Option<CronDeliveryConfig>,
    /// 优先级：low / medium / high / batch（调度排序用）
    #[serde(default = "default_priority")]
    pub priority: String,
    /// 预估成本（E8 折算，用于择时与成本门控）
    #[serde(default)]
    pub epoch_cost_estimate: Option<f64>,
    /// 创建/更新时间
    pub created_at: i64,
    pub updated_at: i64,
}

/// serde 默认优先级（兼容旧库中无该字段的持久化任务）
fn default_priority() -> String {
    CronJobPriority::default().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CronJobPriority {
    Batch,
    Low,
    #[default]
    Medium,
    High,
}

impl CronJobPriority {
    /// 数值化排序权重：High > Medium > Low > Batch
    pub fn weight(&self) -> i32 {
        match self {
            Self::High => 4,
            Self::Medium => 3,
            Self::Low => 2,
            Self::Batch => 1,
        }
    }
}

impl std::str::FromStr for CronJobPriority {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" => Ok(Self::High),
            "low" => Ok(Self::Low),
            "batch" => Ok(Self::Batch),
            _ => Ok(Self::Medium),
        }
    }
}

impl std::fmt::Display for CronJobPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Batch => "batch",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CronJobStatus {
    Active,
    Paused,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRunResult {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub executed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    pub timeout_seconds: u32,
    pub retry_on_failure: bool,
    pub max_retries: u32,
    pub retry_delay_seconds: u32,
    pub notification_enabled: bool,
    pub run_on_startup: bool,
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 300,
            retry_on_failure: true,
            max_retries: 3,
            retry_delay_seconds: 60,
            notification_enabled: false,
            run_on_startup: false,
        }
    }
}

impl CronJob {
    pub fn new(name: &str, schedule: &str, prompt: &str, description: &str) -> Self {
        let now = now_millis();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            schedule: schedule.to_string(),
            prompt: prompt.to_string(),
            workflow_id: None,
            task_type: None,
            platform: None,
            enabled_toolsets: None,
            status: CronJobStatus::Active,
            recurring: true,
            run_count: 0,
            last_run_at: None,
            last_result: None,
            next_run_at: None,
            config: TaskConfig::default(),
            delivery: None,
            priority: CronJobPriority::default().to_string(),
            epoch_cost_estimate: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_platform(mut self, platform: &str) -> Self {
        self.platform = Some(platform.to_string());
        self
    }

    pub fn with_priority(mut self, priority: impl Into<String>) -> Self {
        self.priority = priority.into();
        self
    }

    pub fn with_cost_estimate(mut self, estimate: Option<f64>) -> Self {
        self.epoch_cost_estimate = estimate;
        self
    }

    pub fn with_toolsets(mut self, toolsets: Vec<String>) -> Self {
        self.enabled_toolsets = Some(toolsets);
        self
    }

    pub fn with_workflow_id(mut self, workflow_id: String) -> Self {
        self.workflow_id = Some(workflow_id);
        self
    }

    pub fn with_task_type(mut self, task_type: &str) -> Self {
        self.task_type = Some(task_type.to_string());
        self
    }

    /// G17: 设置 delivery 配置（链式调用）
    pub fn with_delivery(mut self, delivery: CronDeliveryConfig) -> Self {
        self.delivery = Some(delivery);
        self
    }

    pub fn is_active(&self) -> bool {
        self.status == CronJobStatus::Active
    }
}

/// 根据 cron 表达式和当前时间戳（毫秒）计算下次执行时间。
/// 返回 None 表示无法计算（非循环任务或无效表达式）。
fn calculate_next_run(schedule: &str, now_ms: i64) -> Option<i64> {
    let parts: Vec<&str> = schedule.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }

    let now_sec = now_ms / 1000;

    // 处理 */N 间隔模式（仅分钟字段）
    if let Some(step) = parts[0].strip_prefix("*/")
        && let Ok(interval) = step.parse::<i64>()
    {
        let next = now_sec + interval * 60;
        return Some(next * 1000);
    }

    // 处理 */N 间隔模式（仅小时字段，分钟为 0 时）
    if parts[0] == "0"
        && let Some(step) = parts[1].strip_prefix("*/")
        && let Ok(interval) = step.parse::<i64>()
    {
        let next = now_sec + interval * 3600;
        return Some(next * 1000);
    }

    // 对于具体时间点（如 "0 9 * * *"），计算下一次触发时间
    // 使用简单的 UTC 时间计算
    if let (Ok(minute), Ok(hour)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
        let target_sec = hour * 3600 + minute * 60;
        let day_sec = 86400;
        let current_day_sec = {
            let dt = chrono::DateTime::from_timestamp(now_sec, 0).unwrap_or_else(|| {
                chrono::DateTime::from_timestamp(0, 0).expect("Cron：Unix epoch 0 应始终有效")
            });
            (dt.hour() as i64) * 3600 + (dt.minute() as i64) * 60
        };

        let offset = if target_sec > current_day_sec {
            target_sec - current_day_sec
        } else {
            day_sec - current_day_sec + target_sec
        };
        return Some((now_sec + offset) * 1000);
    }

    // 兜底：30 秒后
    Some(now_ms + 30_000)
}

// ── CronJobStore ──────────────────────────────────────────────

pub struct CronJobStore {
    jobs: Arc<RwLock<Vec<CronJob>>>,
    /// `None`（`new_ephemeral`）时是**跳过一切 DB 读写的纯内存模式**。
    ///
    /// 用 `Option` 而不是「持有一个连接 + 一个 bool 开关」：后者允许
    /// 「有连接但自称纯内存」这种自相矛盾的状态存在 —— 旧实现正是那样（它塞进一个
    /// `DatabaseConnection::default()`，而 sea-orm 在构建 SQL 时会直接 panic，
    /// 且该 panic **不可被 Result 捕获**）。这里让「没有持久化」在类型上不可绕过。
    persistence: Option<Arc<dyn CronJobPersistence>>,
}

impl CronJobStore {
    /// 纯内存模式（测试/降级用），不含 DB 持久化。
    pub fn new_ephemeral() -> Self {
        Self { jobs: Arc::new(RwLock::new(Vec::new())), persistence: None }
    }

    /// 构造 CronJobStore 并自动从 DB 恢复已持久化的任务。
    ///
    /// 恢复的任务会重新计算 `next_run_at`：设为 0 使其在下次调度时立即触发，
    /// 避免因重启导致错过的任务被无限推后。
    ///
    /// 参数是端口对象而非 `DatabaseConnection`：把连接交进来就等于把「怎么读写 DB」
    /// 也交进来了（原实现正是如此，于是本 crate 里长出了建表语句和 upsert）。
    /// 具体实现由 wiring 层（`src/init/state.rs`）注入。
    pub async fn new(persistence: Arc<dyn CronJobPersistence>) -> Self {
        persistence.ensure_tables().await;

        let jobs = Self::load_from_db(persistence.as_ref()).await;

        let count = jobs.len();
        if count > 0 {
            info!("[CronJobStore] 从 DB 恢复了 {count} 个定时任务");
        }

        Self { jobs: Arc::new(RwLock::new(jobs)), persistence: Some(persistence) }
    }

    /// 持久化端口；`None` 表示纯内存模式（一切 DB 调用都必须跳过）。
    fn port(&self) -> Option<&dyn CronJobPersistence> {
        self.persistence.as_deref()
    }

    /// 从 DB 加载全部任务，恢复时重置 next_run_at 为 0（立即触发）。
    ///
    /// `CronJob` 的**反序列化留在本层**（端口只给 JSON 字符串），排序也留在本层：
    /// 排序键 `created_at` 藏在 `data` JSON 内部，而两方言取 JSON 字段的写法不同
    /// （SQLite `json_extract` / PG `(data::json->>'created_at')::bigint`），
    /// 无法共用一条 SQL ⇒ 改为应用层按其排序。`created_at` 正是从该 JSON 字段
    /// 反序列化而来，序与原 SQL 完全一致；任务量级为个位到百位，成本可忽略。
    async fn load_from_db(persistence: &dyn CronJobPersistence) -> Vec<CronJob> {
        let raw = match persistence.load_jobs().await {
            Ok(raw) => raw,
            Err(e) => {
                // 此前静默返回空 ⇒ 「DB 里明明有任务，重启后一个都不剩」无从排查。
                error!("[CronJobStore] 从 DB 加载定时任务失败: {e}");
                return Vec::new();
            },
        };

        let now = now_millis();
        let mut jobs: Vec<CronJob> = raw
            .into_iter()
            .filter_map(|data| {
                let mut job: CronJob = serde_json::from_str(&data).ok()?;
                // 重启后重置 next_run_at：活跃任务立即触发，暂停/禁用保持不变
                if job.is_active() {
                    job.next_run_at = Some(0);
                }
                job.updated_at = now;
                Some(job)
            })
            .collect();

        jobs.sort_by_key(|j| j.created_at);
        jobs
    }

    /// 把一条任务写回 DB（已存在则覆盖 `data`）。
    ///
    /// 冲突处理（原按 backend 分支：SQLite `INSERT OR REPLACE` / PG `ON CONFLICT`）
    /// 现由 DAO 侧用 sea-orm 的 `OnConflict` 生成；本表只有 `id` / `data` 两列
    /// 且 upsert 时都显式赋值，故两种语义等价。
    async fn upsert_job(persistence: &dyn CronJobPersistence, id: &str, json: String) {
        if let Err(e) = persistence.upsert_job(id, &json).await {
            error!("[CronJobStore] 持久化任务 {id} 失败（重启后会丢失）: {e}");
        }
    }

    pub async fn add(&self, job: CronJob) -> String {
        let id = job.id.clone();
        // 写入 DB（端口内部按 backend 生成占位符与冲突处理）；ephemeral 模式跳过
        if let Some(p) = self.port() {
            match serde_json::to_string(&job) {
                Ok(json) => Self::upsert_job(p, &id, json).await,
                Err(e) => {
                    error!("[CronJobStore] 序列化任务 {id} 失败，无法持久化: {e}");
                },
            }
        }
        // 写入内存
        let mut jobs = self.jobs.write().await;
        jobs.push(job);
        id
    }

    pub async fn remove(&self, id: &str) -> bool {
        // 删除 DB 记录；ephemeral 模式跳过
        if let Some(p) = self.port()
            && let Err(e) = p.delete_job(id).await
        {
            error!("[CronJobStore] 删除任务 {id} 的 DB 记录失败（重启后可能复活）: {e}");
        }
        // 删除内存
        let mut jobs = self.jobs.write().await;
        let len = jobs.len();
        jobs.retain(|j| j.id != id);
        jobs.len() < len
    }

    pub async fn get(&self, id: &str) -> Option<CronJob> {
        let jobs = self.jobs.read().await;
        jobs.iter().find(|j| j.id == id).cloned()
    }

    pub async fn update(&self, id: &str, updater: impl FnOnce(&mut CronJob)) -> bool {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            updater(job);
            job.updated_at = now_millis();
            // 同步写入 DB；ephemeral 模式跳过
            if let Some(p) = self.port() {
                match serde_json::to_string(job) {
                    Ok(json) => {
                        let job_id = job.id.clone();
                        Self::upsert_job(p, &job_id, json).await;
                    },
                    Err(e) => {
                        error!(
                            "[CronJobStore] 序列化任务 {} 失败（状态变更重启后丢失）: {e}",
                            job.id
                        );
                    },
                }
            }
            true
        } else {
            false
        }
    }

    pub async fn list(&self) -> Vec<CronJob> {
        self.jobs.read().await.clone()
    }

    pub async fn list_active(&self) -> Vec<CronJob> {
        let jobs = self.jobs.read().await;
        jobs.iter().filter(|j| j.is_active()).cloned().collect()
    }

    pub async fn list_due(&self) -> Vec<CronJob> {
        let now = now_millis();
        let jobs = self.jobs.read().await;
        let mut due: Vec<CronJob> = jobs
            .iter()
            .filter(|j| j.is_active() && j.next_run_at.is_none_or(|next| now >= next))
            .cloned()
            .collect();
        // 优先级排序：priority(高→低), 预估耗时(短→长), 提交时间(早→晚)
        due.sort_by(|a, b| {
            let pa = a.priority.parse::<CronJobPriority>().map_or(3, |p| p.weight());
            let pb = b.priority.parse::<CronJobPriority>().map_or(3, |p| p.weight());
            pb.cmp(&pa)
                .then_with(|| {
                    let ea = a.epoch_cost_estimate.unwrap_or(0.0);
                    let eb = b.epoch_cost_estimate.unwrap_or(0.0);
                    ea.partial_cmp(&eb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        due
    }

    pub async fn set_status(&self, id: &str, status: CronJobStatus) -> bool {
        self.update(id, |job| {
            job.status = status;
        })
        .await
    }

    pub async fn record_run(&self, id: &str, result: TaskRunResult) -> bool {
        let now = now_millis();
        let updated = self
            .update(id, |job| {
                job.last_run_at = Some(now);
                job.run_count += 1;
                job.last_result = Some(result.clone());
                job.next_run_at = calculate_next_run(&job.schedule, now);
            })
            .await;

        // 同时保存到执行历史表（ephemeral 模式跳过）
        if updated && let Some(p) = self.port() {
            // 历史记录的 id 仍由本层生成（业务侧决定用什么做主键），端口只负责落库。
            let record = CronHistoryRecord {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: id.to_string(),
                started_at: result.executed_at,
                completed_at: Some(result.executed_at + result.duration_ms as i64),
                success: result.success,
                output: Some(result.output.clone().unwrap_or_default()),
                error: Some(result.error.clone().unwrap_or_default()),
                duration_ms: result.duration_ms as i64,
            };
            if let Err(e) = p.insert_history(record).await {
                error!("[CronJobStore] 写入任务 {id} 的执行历史失败: {e}");
            }
        }

        updated
    }

    /// G17: 记录执行结果并按 delivery 配置投递（如果配置了 sink）
    ///
    /// 与 `record_run` 区别：此方法会在记录完成后，如果 job 配置了 `delivery`
    /// 且传入了 sink，会调用 `sink.deliver_all` 把结果推送到配置的渠道。
    /// 单渠道失败不影响其他渠道，仅记录日志。
    pub async fn record_run_with_delivery(
        &self,
        id: &str,
        result: TaskRunResult,
        sink: Option<&dyn CronDeliverySink>,
    ) -> bool {
        let updated = self.record_run(id, result.clone()).await;

        if updated && let Some(sink) = sink {
            // 读取 job 信息构造 payload
            let (job_name, run_count, delivery) = {
                let jobs = self.jobs.read().await;
                let job = jobs.iter().find(|j| j.id == id);
                match job {
                    Some(j) => (j.name.clone(), j.run_count, j.delivery.clone()),
                    None => return updated,
                }
            };

            if let Some(delivery_config) = delivery {
                let payload = CronDeliveryPayload {
                    job_id: id.to_string(),
                    job_name: job_name.clone(),
                    success: result.success,
                    output: result.output.clone(),
                    error: result.error.clone(),
                    duration_ms: result.duration_ms,
                    executed_at: result.executed_at,
                    run_count,
                };

                if let Err(errors) = sink.deliver_all(&delivery_config, &payload).await {
                    tracing::warn!(
                        "[CronDelivery] 任务 {id}({job_name}) 部分渠道投递失败: {errors:?}"
                    );
                } else {
                    tracing::info!(
                        "[CronDelivery] 任务 {id}({job_name}) 投递成功（{} 个渠道）",
                        delivery_config.channels.len()
                    );
                }
            }
        }

        updated
    }

    pub async fn count(&self) -> usize {
        self.jobs.read().await.len()
    }

    /// 批量加载任务（用于从 DB 恢复）
    pub async fn load_batch(&self, jobs: Vec<CronJob>) {
        let mut store = self.jobs.write().await;
        *store = jobs;
    }

    /// 从 DB 重新加载所有任务（刷新内存状态）；ephemeral 模式跳过 DB。
    pub async fn reload_from_db(&self) -> usize {
        let Some(p) = self.port() else {
            return 0;
        };
        let jobs = Self::load_from_db(p).await;
        let count = jobs.len();
        let mut store = self.jobs.write().await;
        *store = jobs;
        count
    }

    /// 查询指定任务的执行历史（最近 50 条，倒序）；ephemeral 模式返回空。
    pub async fn get_execution_history(&self, task_id: &str) -> Vec<ExecutionRecord> {
        let Some(p) = self.port() else {
            return Vec::new();
        };
        let rows = match p.load_history(task_id, 50).await {
            Ok(rows) => rows,
            Err(e) => {
                error!("[CronJobStore] 查询任务 {task_id} 的执行历史失败: {e}");
                return Vec::new();
            },
        };
        rows.into_iter()
            .map(|m| ExecutionRecord {
                id: m.id,
                task_id: m.task_id,
                started_at: m.started_at,
                completed_at: m.completed_at,
                success: m.success,
                output: m.output,
                error: m.error,
                duration_ms: m.duration_ms,
            })
            .collect()
    }
}

/// 执行历史记录（供前端查询）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub id: String,
    pub task_id: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: i64,
}

// ── Harness trait 实现 ──

impl From<axagent_harness::tool_service::CronJobData> for CronJob {
    fn from(data: axagent_harness::tool_service::CronJobData) -> Self {
        let now = now_millis();
        Self {
            id: data.name.clone(),
            name: data.name,
            description: data.description,
            schedule: data.schedule,
            prompt: data.prompt,
            workflow_id: None,
            task_type: None,
            platform: None,
            enabled_toolsets: None,
            status: if data.is_active {
                CronJobStatus::Active
            } else {
                CronJobStatus::Paused
            },
            recurring: true,
            run_count: data.run_count,
            last_run_at: None,
            last_result: None,
            next_run_at: None,
            config: TaskConfig::default(),
            delivery: None,
            priority: data.priority.unwrap_or_else(|| CronJobPriority::default().to_string()),
            epoch_cost_estimate: None,
            created_at: now,
            updated_at: now,
        }
    }
}

impl From<&CronJob> for axagent_harness::tool_service::CronJobData {
    fn from(job: &CronJob) -> Self {
        Self {
            name: job.name.clone(),
            schedule: job.schedule.clone(),
            prompt: job.prompt.clone(),
            description: job.description.clone(),
            is_active: job.is_active(),
            run_count: job.run_count,
            priority: Some(job.priority.clone()),
        }
    }
}

#[async_trait::async_trait]
impl axagent_harness::tool_service::CronJobStore for CronJobStore {
    async fn add(&self, job: axagent_harness::tool_service::CronJobData) -> String {
        let cron_job: CronJob = job.into();
        CronJobStore::add(self, cron_job).await
    }

    async fn remove(&self, id: &str) -> bool {
        CronJobStore::remove(self, id).await
    }

    async fn get(&self, id: &str) -> Option<axagent_harness::tool_service::CronJobData> {
        CronJobStore::get(self, id)
            .await
            .map(|job| axagent_harness::tool_service::CronJobData::from(&job))
    }

    async fn list(&self) -> Vec<axagent_harness::tool_service::CronJobData> {
        CronJobStore::list(self)
            .await
            .into_iter()
            .map(|job| axagent_harness::tool_service::CronJobData::from(&job))
            .collect()
    }

    async fn count(&self) -> usize {
        CronJobStore::count(self).await
    }
}
