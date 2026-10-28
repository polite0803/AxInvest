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
use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

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
    /// 最近一次执行的触发来源（`manual` / `schedule` / `system`）。
    ///
    /// 随 `cron_jobs.data` 一起持久化；旧记录无此字段 ⇒ `None`（未知），
    /// 前端显示「未知」而不是猜成「定时」。写点只有 `record_run`（从在跑登记里取）。
    #[serde(default)]
    pub last_trigger: Option<String>,
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
    /// 结构化业务参数（时间旅行批次等参数化任务使用；与 prompt 中的遗留 JSON 配置并存）。
    /// 旧持久化记录无此字段，serde default 保证可反序列化。
    #[serde(default)]
    pub params: Option<serde_json::Value>,
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
    /// 本轮被前置闸门跳过（如非交易日），未真正执行业务。
    /// success 仍为 true（跳过是预期行为，不是失败）；前端据此显示「跳过」而非「0ms」。
    /// `#[serde(default)]`：兼容 cron_jobs JSON 中的历史记录。
    #[serde(default)]
    pub skipped: bool,
    /// P1-10（`PLAN-stock-decision-reliability.md`）：批量任务「部分完成」——
    /// 达到单轮墙钟预算提前收尾 / 队列被单轮上限截断（属部分完成而非失败，剩余标的
    /// 下轮续跑）。超墙钟收尾时 `success` 记 false（本轮没跑完规划量），前端据
    /// `partial` 与 `success` 的组合显示「部分完成」而非「失败」。
    /// `#[serde(default)]`：兼容 cron_jobs JSON 中的历史记录。
    #[serde(default)]
    pub partial: bool,
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
            last_trigger: None,
            next_run_at: None,
            config: TaskConfig::default(),
            delivery: None,
            priority: CronJobPriority::default().to_string(),
            epoch_cost_estimate: None,
            params: None,
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

    /// 设置结构化业务参数（链式调用）
    pub fn with_params(mut self, params: serde_json::Value) -> Self {
        self.params = Some(params);
        self
    }

    pub fn is_active(&self) -> bool {
        self.status == CronJobStatus::Active
    }
}

/// 根据 cron 表达式计算 `now_ms` 之后下一次触发时刻（UTC 毫秒）。
///
/// 这是调度器的**唯一**档期口径：调度循环（`runtime/src/cron/scheduler.rs`）
/// 不再自行匹配 cron 字段，而是「`next_run_at` 到期即派发」；`next_run_at` 由
/// `record_run`（触发后推进到下一档期）、`add`（新建即编排）、`resumed_next_run`
/// （重启恢复）三处调用本函数算出。因此本函数的 Local 时区 / 五段 AND / 词法
/// 怪癖就是调度器的全部语义，没有第二份需要同步的匹配实现。
///
/// 1. 按**本地时区**匹配（表达式按北京时间书写，按 UTC 重算 ⇒ 显示/触发的下次
///    运行时间偏 8 小时）；
/// 2. 五段全部参与匹配，包括日 / 月 / 星期（忽略这三段 ⇒ 「每周一 9:00」
///    被算成「今天/明天 9:00」）；
/// 3. day-of-month 与 day-of-week 取 AND（非标准 Vixie OR 语义，历史行为沿用）；
/// 4. 词法门槛与怪癖（只认 `*`、`*/N`、逗号、a-b、纯数字；表达式须含
///    `* , - /` 之一）。
///
/// 从「当前分钟的下一分钟」起逐分钟扫描（上限 366 天）：record_run 在触发当刻
/// 调用，含当前分钟会再次命中自己。
/// 无法算出（非法表达式 / 一年内无匹配）返回 None —— 调用方据此保持
/// `next_run_at = None`，调度器对 None 不派发。
fn calculate_next_run(schedule: &str, now_ms: i64) -> Option<i64> {
    let parts: Vec<&str> = schedule.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    // 词法门槛：不含 * / , - 的表达式无档期语义，直接判为算不出
    if !schedule.contains(['*', '/', ',', '-']) {
        return None;
    }

    let now_local = chrono::TimeZone::timestamp_millis_opt(&chrono::Local, now_ms).single()?;
    // 秒/纳秒归零后推进到下一分钟，保证结果严格晚于本次触发
    let mut cursor = now_local
        .with_second(0)?
        .with_nanosecond(0)?
        .checked_add_signed(chrono::Duration::minutes(1))?;

    let max_minutes = 366 * 24 * 60;
    for _ in 0..max_minutes {
        let matched = cron_field_matches(parts[0], cursor.minute() as i64)
            && cron_field_matches(parts[1], cursor.hour() as i64)
            && cron_field_matches(parts[2], cursor.day() as i64)
            && cron_field_matches(parts[3], cursor.month() as i64)
            && cron_field_matches(parts[4], cursor.weekday().num_days_from_sunday() as i64);
        if matched {
            return Some(cursor.timestamp_millis());
        }
        cursor = cursor.checked_add_signed(chrono::Duration::minutes(1))?;
    }
    None
}

/// 单段 cron 字段匹配 —— 仅服务同文件的 `calculate_next_run`（调度器不再自带
/// 匹配实现，档期口径以本文件为唯一权威）。
/// 行为怪癖：只支持 `*/N` 前缀步进，不支持 `a-b/N`。
fn cron_field_matches(field: &str, current: i64) -> bool {
    if field == "*" {
        return true;
    }
    if let Some(step) = field.strip_prefix("*/")
        && let Ok(interval) = step.parse::<i64>()
        && interval > 0
    {
        return current % interval == 0;
    }
    if field.contains(',') {
        return field.split(',').any(|p| cron_field_matches(p, current));
    }
    if field.contains('-') {
        let bounds: Vec<&str> = field.split('-').collect();
        if bounds.len() == 2
            && let (Ok(lo), Ok(hi)) = (bounds[0].parse::<i64>(), bounds[1].parse::<i64>())
        {
            return current >= lo && current <= hi;
        }
        return false;
    }
    if let Ok(exact) = field.parse::<i64>() {
        return current == exact;
    }
    false
}

/// 重启恢复时 `next_run_at` 的取值策略（纯函数，便于单测锁死三态语义）。
///
/// 调度器是「到期即触发，错过的档期补发一次」模型（见
/// `runtime/src/cron/scheduler.rs`），因此恢复值直接决定重启后会不会跑：
///
/// - **非活跃**（暂停/禁用）：原值原样保留，不动调度状态；
/// - **活跃 + 未来档期**（`next > now`）：保留。今天已经跑过、next 已推进到
///   明天的任务，重启后不得再跑一遍；
/// - **活跃 + 过期/0**：置 `Some(0)` ⇒ 首轮 tick 即到期，补发**一次**
///   （睡眠/关机/重启错过的档期只补一次，不逐档回溯，避免停机一周后任务雪崩）；
/// - **活跃 + None（从未编排）**：当场按表达式算出下一档期；表达式非法 / 一年
///   内无匹配则保持 `None` —— 与「非法表达式永不触发」的旧行为一致，调度器
///   对 None 不派发。
fn resumed_next_run(
    current: Option<i64>,
    schedule: &str,
    active: bool,
    now_ms: i64,
) -> Option<i64> {
    if !active {
        return current;
    }
    match current {
        Some(next) if next > now_ms => Some(next),
        Some(_) => Some(0),
        None => calculate_next_run(schedule, now_ms),
    }
}

// ── CronJobStore ──────────────────────────────────────────────

/// 进程内一条「正在执行」的任务登记。
#[derive(Debug, Clone)]
pub struct RunningJob {
    pub started_at_ms: i64,
    /// `manual` / `schedule`
    pub trigger: &'static str,
}

/// 在跑登记的兜底寿命：超过它视为登记泄漏（handler 内 panic，没走到 `record_run`）。
/// 进程退出时本表整体清空，所以它只防「同进程内泄漏」，不承担跨进程收敛。
const RUNNING_STALE_MS: i64 = 6 * 3600 * 1000;

/// CronJobStore 的调度语义集中在本文件（`calculate_next_run` / `resumed_next_run`），
/// 调度循环在 `runtime/src/cron/scheduler.rs`。
pub struct CronJobStore {
    jobs: Arc<RwLock<Vec<CronJob>>>,
    /// 本进程内正在执行的任务：id → 开始时刻 + 触发来源。
    ///
    /// 纯内存：进程退出即清空 ⇒ 重启后不会留下「假 running」（与 DB 行不同，
    /// 不需要启动收敛）。标记点在调度器（定时 tick 与手动 `trigger_now`），
    /// 摘除点在 `record_run`（所有 handler 分支最终都会走到那里）。
    running: Arc<RwLock<HashMap<String, RunningJob>>>,
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
        Self {
            jobs: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(RwLock::new(HashMap::new())),
            persistence: None,
        }
    }

    /// 构造 CronJobStore 并自动从 DB 恢复已持久化的任务。
    ///
    /// 恢复的 `next_run_at` 三态策略见 [`resumed_next_run`]：未来档期保留（当天已
    /// 跑过的任务重启不重跑）、过期/0 置 0（错过的档期在首轮 tick 补发一次）、
    /// None 当场按表达式补算（非法表达式保持 None，调度器不触发）。
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

        Self {
            jobs: Arc::new(RwLock::new(jobs)),
            running: Arc::new(RwLock::new(HashMap::new())),
            persistence: Some(persistence),
        }
    }

    /// 持久化端口；`None` 表示纯内存模式（一切 DB 调用都必须跳过）。
    fn port(&self) -> Option<&dyn CronJobPersistence> {
        self.persistence.as_deref()
    }

    /// 从 DB 加载全部任务，按 [`resumed_next_run`] 恢复 `next_run_at`。
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
        let mut jobs: Vec<CronJob> = Vec::with_capacity(raw.len());
        for data in raw {
            let Ok(mut job) = serde_json::from_str::<CronJob>(&data) else {
                // 单条损坏不应拖垮全部任务（旧实现 filter_map 整体跳过本行）
                continue;
            };
            let original_next = job.next_run_at;
            // 0 = **无效档期**（epoch），不是合法时间戳。
            //
            // 2026-10-28 实证：`reflection-due-daily` 的 `next_run_at` 被写成 0，UI 把它
            // 渲染成「1970/01/01 08:00」并打上「档期停滞」；而调度器按「Some 且 now >= next
            // 即派发」判定（见 `due` 过滤），0 这个值既不像"没有档期"（那该是 None），也
            // 不等于一次真实到期 —— 结果这一行既没被正常重排，也没按点触发。
            // 归零后走 None 分支：`resumed_next_run` 会按 cron 重新算，`materialize_next_runs`
            // 也会把它补算回 DB，等于自愈。
            let sanitized_next = job.next_run_at.filter(|t| *t > 0);
            job.next_run_at = resumed_next_run(sanitized_next, &job.schedule, job.is_active(), now);
            job.updated_at = now;
            // 恢复策略改变了档期（过期→0 的补发标记、None→补算的下一档期）时回写 DB：
            // 否则 None→Some 只存在于内存，`materialize_next_runs` 又只扫内存中的
            // None（此时已是 Some）⇒ DB 永远停在 None，外部工具看到的是陈旧状态。
            if job.next_run_at != original_next
                && let Ok(json) = serde_json::to_string(&job)
            {
                Self::upsert_job(persistence, &job.id, json).await;
            }
            jobs.push(job);
        }

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

    pub async fn add(&self, mut job: CronJob) -> String {
        let id = job.id.clone();
        // 新建活跃任务立即编排下次档期：调度器只认 `next_run_at` 到期，不再做
        // 「当前分钟匹配 cron」判定，None 任务永远不会被触发。非法表达式算出 None
        // 时保持 None（调度器不派发），与「非法表达式永不触发」的旧行为一致。
        if job.is_active() && job.next_run_at.is_none() {
            job.next_run_at = calculate_next_run(&job.schedule, now_millis());
        }
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

    /// 为 `next_run_at = None` 的活跃任务按 cron 表达式补算下次触发时刻并持久化。
    ///
    /// 正常路径（`add` / DB 恢复）已经编排好，这里只是运行期兜底（外部直接写内存、
    /// 老版本遗留的 None 行）。返回补算条数；非法表达式（一年内无档期）保持 None，
    /// 调度器不派发，也不会在每轮 tick 重复尝试写库。
    pub async fn materialize_next_runs(&self) -> usize {
        let pending: Vec<(String, String)> = {
            let jobs = self.jobs.read().await;
            jobs.iter()
                .filter(|j| j.is_active() && j.next_run_at.is_none())
                .map(|j| (j.id.clone(), j.schedule.clone()))
                .collect()
        };

        let mut count = 0;
        for (id, schedule) in pending {
            let Some(next) = calculate_next_run(&schedule, now_millis()) else {
                continue;
            };
            if self.update(&id, |job| job.next_run_at = Some(next)).await {
                count += 1;
            }
        }
        count
    }

    pub async fn list_due(&self) -> Vec<CronJob> {
        let now = now_millis();
        let jobs = self.jobs.read().await;
        let mut due: Vec<CronJob> = jobs
            .iter()
            // 触发模型是「next_run_at 到期即触发」：只有显式编排了档期（Some）且已到期
            // 的任务才进 due。None = 从未编排 / 非法表达式，**不**派发 —— 否则非法
            // 表达式会在 record_run 反复算出 None 后被每轮 tick 重复触发。
            .filter(|j| j.is_active() && j.next_run_at.is_some_and(|next| now >= next))
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

    /// 登记「本进程正在执行」：调度器在定时派发与手动 `trigger_now` 两处调用。
    pub async fn mark_running(&self, id: &str, started_at_ms: i64, trigger: &'static str) {
        self.running.write().await.insert(id.to_string(), RunningJob { started_at_ms, trigger });
    }

    /// 摘除在跑登记（`record_run` 开头调用；handler 各分支最终都会走到那里）。
    pub async fn clear_running(&self, id: &str) {
        self.running.write().await.remove(id);
    }

    /// 当前在跑任务快照（带兜底寿命，防同进程内的登记泄漏）。
    pub async fn list_running(&self) -> HashMap<String, RunningJob> {
        let now = now_millis();
        let mut running = self.running.write().await;
        running.retain(|_, r| now - r.started_at_ms < RUNNING_STALE_MS);
        running.clone()
    }

    pub async fn record_run(&self, id: &str, result: TaskRunResult) -> bool {
        // 在跑登记是触发来源的权威：取出即摘除（与 last_result / history 同一份）。
        // 没有登记的调用（外部直接调 record_run 的路径）落 None ⇒ 未知。
        let trigger = self.running.write().await.remove(id).map(|r| r.trigger.to_string());
        let now = now_millis();
        let updated = self
            .update(id, |job| {
                job.last_run_at = Some(now);
                job.run_count += 1;
                job.last_result = Some(result.clone());
                job.last_trigger = trigger.clone();
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
                partial: result.partial,
                trigger: trigger.clone(),
            };
            if let Err(e) = p.insert_history(record).await {
                error!("[CronJobStore] 写入任务 {id} 的执行历史失败: {e}");
            }
        }

        updated
    }

    /// 记录一次「系统兜底轮」执行到 `cron_job_history`，但**不推进任何调度状态**
    /// （不写 `last_run_at` / `run_count`，尤其**不重排 `next_run_at`**）。
    ///
    /// 用途：与某个 cron 任务同源、但由独立 `tokio::interval` 驱动的兜底执行
    /// （如每 6h 的批量反思兜底轮、每 12h 的需求发现扫描）。它们不是档期触发：
    /// 走 `record_run` 会把档期提前重排、与调度器的「到期派发」互相干扰；但成败
    /// 若只进 tracing 日志，前端任务历史面板零可见性 —— 故单独落一条历史，
    /// 建议在 `output` 中以「系统兜底轮」前缀与档期触发记录区分。
    ///
    /// 返回 false：ephemeral 模式（无持久化端口）、`task_id` 在内存中无对应任务
    /// （历史记录成了孤儿行）或落库失败。
    pub async fn record_auxiliary_run(&self, task_id: &str, result: TaskRunResult) -> bool {
        let Some(p) = self.port() else {
            return false;
        };
        let exists = self.jobs.read().await.iter().any(|j| j.id == task_id);
        if !exists {
            warn!(
                "[CronJobStore] 兜底轮历史引用了不存在的任务 {task_id}，跳过落库（避免孤儿历史行）"
            );
            return false;
        }
        let record = CronHistoryRecord {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            started_at: result.executed_at,
            completed_at: Some(result.executed_at + result.duration_ms as i64),
            success: result.success,
            output: Some(result.output.clone().unwrap_or_default()),
            error: Some(result.error.clone().unwrap_or_default()),
            duration_ms: result.duration_ms as i64,
            partial: result.partial,
            // 兜底轮由进程内独立 interval 驱动，既非档期也非用户点击。
            trigger: Some("system".to_string()),
        };
        match p.insert_history(record).await {
            Ok(()) => true,
            Err(e) => {
                error!("[CronJobStore] 写入任务 {task_id} 的兜底轮执行历史失败: {e}");
                false
            },
        }
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
                partial: m.partial,
                trigger: m.trigger,
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
    /// 本轮「部分完成」（超墙钟预算收尾 / 单轮上限截断）。
    ///
    /// `#[serde(default)]`：历史表加列前的旧行读出 0，序列化成 false 即可，
    /// 前端把 false 读作「不是部分完成」而不是「未知」。
    #[serde(default)]
    pub partial: bool,
    /// 触发来源：`manual` / `schedule` / `system`；`None` = 本列引入前的旧行（未知）。
    #[serde(default)]
    pub trigger: Option<String>,
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
            last_trigger: None,
            next_run_at: None,
            config: TaskConfig::default(),
            delivery: None,
            priority: data.priority.unwrap_or_else(|| CronJobPriority::default().to_string()),
            epoch_cost_estimate: None,
            params: None,
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

#[cfg(test)]
mod calculate_next_run_tests {
    use super::{calculate_next_run, resumed_next_run};
    use chrono::{Datelike, Local, TimeZone, Timelike};

    /// 构造本地时区的毫秒时间戳
    fn local_ms(y: i32, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, m, d, hh, mm, ss)
            .single()
            .expect("测试时刻在本地时区必须唯一")
            .timestamp_millis()
    }

    /// 把毫秒转回本地并断言完整日历字段
    fn assert_local(got: Option<i64>, y: i32, m: u32, d: u32, hh: u32, mm: u32) {
        let dt = Local
            .timestamp_millis_opt(got.expect("应算出下次时间"))
            .single()
            .expect("结果时间戳必须有效");
        assert_eq!(
            (dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute()),
            (y, m, d, hh, mm),
            "下次触发的本地日历字段不符"
        );
    }

    #[test]
    fn test_daily_fixed_time_later_today() {
        // 当天 08:00 → 今天 09:00
        assert_local(
            calculate_next_run("0 9 * * *", local_ms(2026, 10, 12, 8, 0, 0)),
            2026,
            10,
            12,
            9,
            0,
        );
    }

    #[test]
    fn test_daily_fixed_time_after_today_rolls_to_tomorrow() {
        // 当天 10:30 → 次日 09:00（旧 UTC 实现会算成当天/次日的错误钟点）
        assert_local(
            calculate_next_run("0 9 * * *", local_ms(2026, 10, 12, 10, 30, 0)),
            2026,
            10,
            13,
            9,
            0,
        );
    }

    #[test]
    fn test_never_rematches_current_minute() {
        // record_run 在 09:00 触发后调用：含秒的当前分钟不得再次命中自己
        assert_local(
            calculate_next_run("0 9 * * *", local_ms(2026, 10, 12, 9, 0, 42)),
            2026,
            10,
            13,
            9,
            0,
        );
        // 凌晨 01:00 档同理 → 次日 01:00（本地钟点，不偏 8 小时）
        assert_local(
            calculate_next_run("0 1 * * *", local_ms(2026, 10, 12, 1, 0, 5)),
            2026,
            10,
            13,
            1,
            0,
        );
    }

    #[test]
    fn test_minute_step_aligns_to_grid() {
        // */15：10:07 之后的网格点是 10:15（旧实现给 now+15min=10:22，错误）
        assert_local(
            calculate_next_run("*/15 * * * *", local_ms(2026, 10, 12, 10, 7, 0)),
            2026,
            10,
            12,
            10,
            15,
        );
    }

    #[test]
    fn test_comma_list() {
        // 0,30 9 * * *：08 点档 → 09:00；09:10 → 09:30；09:40 → 次日 09:00
        assert_local(
            calculate_next_run("0,30 9 * * *", local_ms(2026, 10, 12, 8, 0, 0)),
            2026,
            10,
            12,
            9,
            0,
        );
        assert_local(
            calculate_next_run("0,30 9 * * *", local_ms(2026, 10, 12, 9, 10, 0)),
            2026,
            10,
            12,
            9,
            30,
        );
        assert_local(
            calculate_next_run("0,30 9 * * *", local_ms(2026, 10, 12, 9, 40, 0)),
            2026,
            10,
            13,
            9,
            0,
        );
    }

    #[test]
    fn test_weekly_weekday_field_is_honored() {
        // 每周一 09:00；2026-10-11 是周日 → 2026-10-12（周一）09:00
        // 旧实现忽略星期字段，会错算成次日以外的任意日期
        assert_local(
            calculate_next_run("0 9 * * 1", local_ms(2026, 10, 11, 12, 0, 0)),
            2026,
            10,
            12,
            9,
            0,
        );
        // 周一 10:00 再算 → 下周一（10-19）
        assert_local(
            calculate_next_run("0 9 * * 1", local_ms(2026, 10, 12, 10, 0, 0)),
            2026,
            10,
            19,
            9,
            0,
        );
    }

    #[test]
    fn test_monthly_day_field_is_honored() {
        // 每月 1 日 18:00；10-11 → 11-01 18:00
        assert_local(
            calculate_next_run("0 18 1 * *", local_ms(2026, 10, 11, 9, 0, 0)),
            2026,
            11,
            1,
            18,
            0,
        );
    }

    #[test]
    fn test_invalid_and_lexical_gate_return_none() {
        // 非法 / 段数不对
        assert!(calculate_next_run("not a cron", 0).is_none());
        assert!(calculate_next_run("0 9 * *", local_ms(2026, 10, 12, 8, 0, 0)).is_none());
        // 词法门槛：不含 * , - / 的表达式无档期语义，calculate_next_run 返回 None
        // ⇒ next_run_at 保持 None，调度器不派发
        assert!(calculate_next_run("0 9 1 1 1", local_ms(2026, 10, 12, 8, 0, 0)).is_none());
    }

    // ── resumed_next_run 三态恢复策略 ──

    #[test]
    fn test_reserved_future_next_run_is_preserved() {
        // 当天 06:00 重启：今天 09:00 的档期在未来，原样保留（不重跑已完成任务的前提
        // 是 next 已推进；这里直接锁住「未来不动」这一半语义）。
        let now = local_ms(2026, 10, 14, 6, 0, 0);
        let future = local_ms(2026, 10, 14, 9, 0, 0);
        assert_eq!(resumed_next_run(Some(future), "0 9 * * *", true, now), Some(future));
    }

    #[test]
    fn test_expired_or_zero_next_run_becomes_zero_for_catch_up() {
        // 昨天的档期 / 0（旧版本恢复值）⇒ 置 0，首轮 tick 即补发一次。
        let now = local_ms(2026, 10, 14, 3, 22, 0);
        let yesterday = local_ms(2026, 10, 13, 0, 0, 0);
        assert_eq!(resumed_next_run(Some(yesterday), "0 0 * * *", true, now), Some(0));
        assert_eq!(resumed_next_run(Some(0), "0 0 * * *", true, now), Some(0));
    }

    #[test]
    fn test_none_next_run_active_is_scheduled_from_expression() {
        // 周六 09:00 周任务，周三重启 ⇒ 算出本周六，不立即补发（周中没错过它的档期）。
        let now = local_ms(2026, 10, 14, 3, 22, 0); // 2026-10-14 周三
        let got = resumed_next_run(None, "0 9 * * 6", true, now).expect("应算出周六档期");
        assert_local(Some(got), 2026, 10, 17, 9, 0);
    }

    #[test]
    fn test_none_next_run_invalid_expression_stays_none() {
        // 非法表达式保持 None：调度器对 None 不派发，避免每轮 tick 重复触发。
        let now = local_ms(2026, 10, 14, 3, 22, 0);
        assert!(resumed_next_run(None, "not a cron", true, now).is_none());
        assert!(resumed_next_run(None, "0 9 1 1 1", true, now).is_none());
    }

    #[test]
    fn test_inactive_job_next_run_is_untouched() {
        // 暂停 / 禁用任务：任何现值（含过期 / None）原样保留。
        let now = local_ms(2026, 10, 14, 3, 22, 0);
        let expired = local_ms(2026, 10, 10, 2, 0, 0);
        assert_eq!(resumed_next_run(Some(expired), "0 2 * * *", false, now), Some(expired));
        assert_eq!(resumed_next_run(None, "0 9 * * *", false, now), None);
    }
}

#[cfg(test)]
mod running_registry_tests {
    use super::{CronJob, CronJobStore, TaskRunResult, now_millis};

    /// 在跑登记 → `record_run` 摘除，且触发来源随结果落进 `last_trigger`。
    /// 这条锁住三件事：① 登记可读回；② 任务结束不会留下假 running；
    /// ③ 触发来源不是前端猜的，而是从登记一路带到持久化。
    #[tokio::test]
    async fn running_registry_persists_trigger_and_clears_on_record_run() {
        let store = CronJobStore::new_ephemeral();
        let job = CronJob::new("t-run", "0 9 * * *", "p", "d");
        let id = job.id.clone();
        store.add(job).await;

        let started = now_millis();
        store.mark_running(&id, started, "manual").await;
        let running = store.list_running().await;
        let entry = running.get(&id).expect("登记应可读回");
        assert_eq!(entry.started_at_ms, started);
        assert_eq!(entry.trigger, "manual");

        let result = TaskRunResult {
            success: true,
            output: Some("ok".into()),
            error: None,
            duration_ms: 5,
            executed_at: started,
            skipped: false,
            partial: false,
        };
        assert!(store.record_run(&id, result).await);
        assert!(store.list_running().await.is_empty(), "record_run 必须摘除在跑登记");
        let job = store.get(&id).await.expect("任务应仍在");
        assert_eq!(job.last_trigger.as_deref(), Some("manual"), "触发来源随结果落盘");
    }
}
