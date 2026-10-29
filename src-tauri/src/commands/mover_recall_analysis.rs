//! 妖股漏检 → as-of 历史补分析批次（P2，`PLAN-batch-automation-mover-recall.md` §P2）。
//!
//! ## 这个批次做什么
//!
//! 消费核查落库的 `mover_recall_events`（`pending` / 上批 `failed`），
//! 以每个**事件的窗口起点** `window_start` 为 as-of 锚点重放完整 stock-analysis 工作流：
//! - 只补可算法归因的三层：`not_in_pool` / `l3_scored_out` / `other_period`
//!   （`pool_degraded` 落库即 `excluded`；`unexplained` 事件**批次级过滤**：
//!   所属核查轮次 unexplained 占比 ≥ 阈值时整轮不取，留痕不足的批次不进样本）；
//! - 分析行走 `run_single_stock_analysis` 通道，落 `source='mover_recall'`、
//!   `synthetic=1`、`anchor_event_id=<事件id>` —— 不写 reco_picks，
//!   决策回测真实命中率分母天然不含之；
//! - 每只失败在**同一 as-of guard 内立即重试一次**，再败标 `failed`。
//!
//! ## 状态存储（与 `time_travel_batch.rs` 同族）
//!
//! 运行态在进程内 `BATCH_RUNS`（轮询权威），每次状态变更同步 upsert
//! `mover_recall_analysis_batches`（重启后历史权威）；逐票明细不存批次 JSON，
//! 由 `mover_recall_events.batch_id` 反查。异常退出遗留的 running 批次 / 事件
//! 在新进程首次访问时收敛（批次→interrupted，事件→pending 可再认领）。
//!
//! ## 互斥
//!
//! as_of 封闭世界栈是进程级单例 ⇒ 与时间旅行批次**共用**
//! `AppState::time_travel_batch_lock`（try_lock，不排队），并共用同一 live 时间窗
//! 拒绝判据（`time_travel_batch::in_live_window`）。

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use axagent_agent_macro::agent_command;
use axagent_analysis_engine::horizon_prior;
use axagent_analysis_engine::mover_recall as mover_core;
use axagent_analysis_engine::recommender::{self, RecoPick, SeedItem, SeedOverrides};
use axagent_astock_data::AStockClient;
use axagent_astock_data::as_of::{self, AsOfContext, AsOfSource};
use axagent_dao::repo::mover_recall_analysis_batches::{self, BatchRunModel, BatchRunRecord};
use axagent_dao::repo::mover_recall_events::{self, CHAIN_ROWS_PER_EVENT, EventModel};
use axagent_dao::repo::reco_seed;
use axagent_harness::holding_period::Period;
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::error::ErrorResponse;
use super::error_code::common as common_err;
use super::error_code::mover_recall_analysis as ec;
use super::pool_scan::{PoolCandidate, run_analysis_for_candidates};
use super::time_travel_batch::in_live_window;
use crate::app_state::AppState;

/// 任务类型常量。
///
/// 2026-10-20 起有**两个**触发方：内置定时任务 `mover-recall-analysis-daily`（播种见
/// `init/stock_automation_cron_seed.rs`）与任务中心「立即执行」。值同时登记在
/// `scheduled_task::BUILTIN_TASK_TYPES` 与前端 `taskGrouping.ts` 的两处白名单，
/// 三者由 `scripts/check-task-type-whitelist.mjs` 逐值对表 —— 漏一处不是报错，
/// 而是「到点自动跑、手动点它说不支持」这种只在手动时露头的缺陷。
pub use crate::commands::constants::MOVER_RECALL_ANALYSIS_TASK_TYPE;

/// 内置定时任务单轮补分析的事件数上限。
///
/// **2026-10-29 由 60 降到 12**：实测 10-26 / 10-27 / 10-28 三次 cron 批次全部
/// `interrupted` —— 60 只 × 每只数分钟 = 要跑数小时，期间应用一重启进程就退出，
/// 批次被打断、ok 只有 0~2。单轮 12 只约 30–60 分钟，能在两次重启之间跑完；
/// 漏检事件按天累积，多轮同样能消化。手动面板仍是 20（`default_batch_limit`）。
pub const CRON_BATCH_LIMIT: usize = 12;

/// 旧版内置任务的配额（10）。只服务**一次性升级**：库里仍是它的任务行会被抬到
/// [`CRON_BATCH_LIMIT`]；用户改过配额的行一律不动（种子的「存在即跳过」不能被绕过）。
pub const LEGACY_CRON_BATCH_LIMIT: usize = 10;

static BATCH_RUNS: LazyLock<Arc<Mutex<HashMap<String, MoverBatchState>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

const STATUS_RUNNING: &str = "running";
const STATUS_COMPLETED: &str = "completed";
const STATUS_PARTIAL: &str = "partial";
const STATUS_FAILED: &str = "failed";
const STATUS_INTERRUPTED: &str = "interrupted";

const EVENT_ANALYZED: &str = "analyzed";
const EVENT_FAILED: &str = "failed";

/// 选样查询的最大扫描行数（unexplained 过滤前）。周频核查单轮事件量级数百，500 足够。
const SELECT_SCAN_LIMIT: u64 = 500;

/// 发起漏检补分析批次的请求（camelCase 前端契约）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMoverRecallAnalysisRequest {
    /// 单批最多补分析的事件数（1-100，默认 20）
    #[serde(default = "default_batch_limit")]
    pub batch_limit: usize,
    /// 所属核查轮次 unexplained 占比 ≥ 该阈值的事件整批不取。
    /// **单位是 0-1 归一化占比**（0.5 = 50%）；`rates_json` 里的百分数由 dao 读取时归一。
    #[serde(default = "default_max_unexplained_share")]
    pub max_unexplained_share: f64,
    /// 仅处理指定核查轮次（mover_recall_runs.id）；None = 跨全部轮次选样
    #[serde(default)]
    pub run_id: Option<String>,
}

fn default_batch_limit() -> usize {
    20
}
/// 默认阈值 0.5 = 50%（归一化占比，不是百分数）。
fn default_max_unexplained_share() -> f64 {
    0.5
}

impl RunMoverRecallAnalysisRequest {
    /// 内置定时任务的批次参数：配额取 [`CRON_BATCH_LIMIT`]，轮次不限（跨全部已落库轮次选样）。
    pub fn cron_default() -> Self {
        Self {
            batch_limit: CRON_BATCH_LIMIT,
            max_unexplained_share: default_max_unexplained_share(),
            run_id: None,
        }
    }

    /// 写进 `cron_jobs.prompt`（与 `PoolScanConfig::to_json` 同族形态）。
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| {
            ErrorResponse::err_with_detail(
                common_err::INTERNAL,
                format!("序列化 {MOVER_RECALL_ANALYSIS_TASK_TYPE} 配置失败: {e}"),
            )
        })
    }

    /// 读回 `cron_jobs.prompt`。
    ///
    /// 空串是**正常形态**（种子任务也会写空 = 全用缺省）；非空却解不出来一律 `Err` ——
    /// 静默回退缺省会把用户设在任务面板里的配额悄悄换成 10，而列表里显示的仍是原文。
    pub fn from_json(raw: &str) -> Result<Self, String> {
        if raw.trim().is_empty() {
            return Ok(Self::cron_default());
        }
        serde_json::from_str(raw).map_err(|e| {
            ErrorResponse::err_with_detail(
                common_err::INVALID_INPUT,
                format!("漏检补分析配置解析失败: {e}"),
            )
        })
    }
}

/// 批次内逐票状态（序列化给前端明细抽屉；从 events 表反查重建）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventItem {
    pub event_id: String,
    pub run_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub period: String,
    pub window_start: String,
    pub anchor_date: String,
    pub cum_gain_pct: f64,
    pub miss_layer: String,
    /// running / analyzed / failed
    pub status: String,
    pub analysis_id: Option<String>,
    pub error_detail: Option<String>,
}

impl From<&EventModel> for EventItem {
    fn from(m: &EventModel) -> Self {
        Self {
            event_id: m.id.clone(),
            run_id: m.run_id.clone(),
            stock_code: m.stock_code.clone(),
            stock_name: m.stock_name.clone(),
            period: m.period.clone(),
            window_start: m.window_start.clone(),
            anchor_date: m.anchor_date.clone(),
            cum_gain_pct: m.cum_gain_pct,
            miss_layer: m.miss_layer.clone(),
            status: m.status.clone(),
            analysis_id: m.analysis_id.clone(),
            error_detail: m.error_detail.clone(),
        }
    }
}

/// 批次运行状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverBatchState {
    pub batch_id: String,
    /// running / completed / partial / failed / interrupted
    pub status: String,
    pub message: Option<String>,
    pub total: usize,
    pub ok: usize,
    pub fail: usize,
    pub items: Vec<EventItem>,
    pub error_detail: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    /// 触发来源：`manual` / `schedule`；`None` = 旧批次（未知）。
    pub trigger: Option<String>,
    /// 阶段 E-4：选样留痕四数。`None` = 选样还没跑 ⇒ 与「扫了 0 个」是两句不同的话。
    pub scanned: Option<i32>,
    pub skipped_by_share: Option<i32>,
    pub skipped_by_period: Option<i32>,
    pub skipped_by_window: Option<i32>,
    /// 请求快照仅落库 request_json，不向前端输出、从库还原为 None
    #[serde(skip)]
    pub request: Option<RunMoverRecallAnalysisRequest>,
}

impl MoverBatchState {
    fn new(
        batch_id: String,
        request: RunMoverRecallAnalysisRequest,
        trigger: &'static str,
    ) -> Self {
        let now = now_ms();
        Self {
            batch_id,
            status: STATUS_RUNNING.to_string(),
            message: None,
            total: 0,
            ok: 0,
            fail: 0,
            items: Vec::new(),
            error_detail: None,
            created_at_ms: now,
            updated_at_ms: now,
            finished_at_ms: None,
            trigger: Some(trigger.to_string()),
            scanned: None,
            skipped_by_share: None,
            skipped_by_period: None,
            skipped_by_window: None,
            request: Some(request),
        }
    }
}

fn record_from_state(s: &MoverBatchState) -> BatchRunRecord {
    BatchRunRecord {
        id: s.batch_id.clone(),
        status: s.status.clone(),
        message: s.message.clone(),
        total: s.total as i32,
        ok: s.ok as i32,
        fail: s.fail as i32,
        request_json: s.request.as_ref().and_then(|r| serde_json::to_string(r).ok()),
        error_detail: s.error_detail.clone(),
        created_at_ms: s.created_at_ms,
        updated_at_ms: s.updated_at_ms,
        finished_at_ms: s.finished_at_ms,
        trigger: s.trigger.clone(),
        scanned: s.scanned,
        skipped_by_share: s.skipped_by_share,
        skipped_by_period: s.skipped_by_period,
        skipped_by_window: s.skipped_by_window,
    }
}

async fn save_state(db: &axagent_harness::DatabaseConnection, mut s: MoverBatchState) {
    s.updated_at_ms = now_ms();
    if let Err(e) =
        mover_recall_analysis_batches::upsert_batch_run(db, &record_from_state(&s)).await
    {
        tracing::warn!(
            "[mover-recall-analysis] 批次 {} 状态落库失败（不影响运行）: {e}",
            s.batch_id
        );
    }
    BATCH_RUNS.lock().await.insert(s.batch_id.clone(), s);
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

static STALE_REAP_DONE: AtomicBool = AtomicBool::new(false);

/// 首次访问收敛：遗留 running 批次 → interrupted；其名下仍 running 的事件 → pending
/// （as_of 栈进程级，不支持跨进程续跑，事件必须释放给后续批次）。
async fn ensure_stale_reaped(db: &axagent_harness::DatabaseConnection) {
    if STALE_REAP_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    let now = now_ms();
    let interrupted_msg = "应用重启：补分析批次运行中进程退出，as-of 封闭世界无法跨进程续跑";
    let stale = mover_recall_analysis_batches::mark_stale_batches(
        db,
        now,
        STATUS_RUNNING,
        STATUS_INTERRUPTED,
        interrupted_msg,
    )
    .await;
    match stale {
        Ok(0) => {},
        Ok(n) => tracing::info!(
            "[mover-recall-analysis] 启动收敛：{n} 个遗留 running 批次标记为 interrupted"
        ),
        Err(e) => tracing::warn!("[mover-recall-analysis] 遗留批次收敛失败: {e}"),
    }
    // 释放僵尸批次名下的 running 事件（失败只告警，下次人工/重试仍可再处理）
    if let Ok(models) = mover_recall_analysis_batches::list_recent_batch_runs(
        db,
        SELECT_SCAN_LIMIT,
        Some(STATUS_INTERRUPTED),
    )
    .await
    {
        for m in models {
            // 只收敛本次重启刚打断的批次（finished_at 与当前收敛时刻相同无法区分，
            // 故按「批次 interrupted 且仍有 running 事件」幂等释放；历史 interrupted
            // 批次的事件早已释放，二次调用零命中）。
            if let Err(e) =
                mover_recall_events::release_running_events(db, &m.id, interrupted_msg, now).await
            {
                tracing::warn!("[mover-recall-analysis] 批次 {} 事件释放失败: {e}", m.id);
            }
        }
    }
}

/// 历史表行 → 批次状态；items 由调用方从 events 反查后填充（list 摘要场景传空）。
fn state_from_model(m: BatchRunModel, items: Vec<EventItem>) -> MoverBatchState {
    MoverBatchState {
        batch_id: m.id,
        status: m.status,
        message: m.message,
        total: m.total.max(0) as usize,
        ok: m.ok.max(0) as usize,
        fail: m.fail.max(0) as usize,
        items,
        error_detail: m.error_detail,
        created_at_ms: m.created_at_ms,
        updated_at_ms: m.updated_at_ms,
        finished_at_ms: m.finished_at_ms,
        trigger: m.trigger,
        scanned: m.scanned,
        skipped_by_share: m.skipped_by_share,
        skipped_by_period: m.skipped_by_period,
        skipped_by_window: m.skipped_by_window,
        request: None,
    }
}

// ── Tauri 命令 ────────────────────────────────────────────────────────────────

/// 发起一次漏检补分析批次；立即返回 batch_id，后台执行，用
/// [`get_mover_recall_analysis_batch`] 轮询。
#[agent_command(
    domain = "finance",
    safety = Caution,
    call_mode = StateInput,
    description = "发起妖股漏检 as-of 历史补分析批次"
)]
#[tauri::command]
pub async fn run_mover_recall_analysis(
    state: State<'_, AppState>,
    request: RunMoverRecallAnalysisRequest,
) -> Result<String, String> {
    launch(
        &state.time_travel_batch_lock,
        state.harness.db().clone(),
        state.astock_client.clone(),
        state.work_engine.clone(),
        request,
        "manual",
    )
    .await
}

/// 供调度器分支发起一轮补分析：cron 拿不到 `tauri::State`，只能拿调度器已持有的零件
/// （同 `analyze_mover_recall_inner` 的既有理由）。校验与抢锁判据与手动命令**共用** [`launch`]。
///
/// 返回的 `Ok(batch_id)` 只表示**已发起**：批次跑在独立多线程 runtime（as-of 链要 32MB 栈），
/// 调用方不等它。因此 cron 的任务历史写「已发起 + batchId」，逐票成败看任务中心
/// 「漏检补分析」来源 —— 两侧都不假装知道终态。
pub(crate) async fn start_for_scheduler(
    asof_lock: &Arc<Mutex<()>>,
    db: axagent_harness::DatabaseConnection,
    client: Arc<AStockClient>,
    engine: Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    request: RunMoverRecallAnalysisRequest,
) -> Result<String, String> {
    launch(asof_lock, db, client, engine, request, "schedule").await
}

/// 校验 → live 时间窗拒绝 → 抢全局 as-of 批次锁 → 落批次起点 → 后台发起。
async fn launch(
    asof_lock: &Arc<Mutex<()>>,
    db: axagent_harness::DatabaseConnection,
    client: Arc<AStockClient>,
    engine: Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    request: RunMoverRecallAnalysisRequest,
    trigger: &'static str,
) -> Result<String, String> {
    if request.batch_limit == 0 || request.batch_limit > 100 {
        return Err(ErrorResponse::err_with_detail(
            ec::INVALID_PARAMS,
            "batchLimit 必须在 1-100 之间",
        ));
    }
    if !(0.0..=1.0).contains(&request.max_unexplained_share) {
        return Err(ErrorResponse::err_with_detail(
            ec::INVALID_PARAMS,
            "maxUnexplainedShare 必须在 0-1 之间",
        ));
    }

    // live 时间窗拒绝先于抢锁（与时间旅行批次同一判据）
    if in_live_window() {
        return Err(ErrorResponse::err_with_detail(
            ec::LIVE_WINDOW,
            "当前处于 live 流水线时间窗（23:50-02:10 / 17:50-18:10），请避开后再发起漏检补分析",
        ));
    }

    // 全局 as-of 批次互斥（与时间旅行批次共用同一把锁）
    let permit =
        asof_lock.clone().try_lock_owned().map_err(|_| ErrorResponse::err(ec::ALREADY_RUNNING))?;

    ensure_stale_reaped(&db).await;

    let batch_id = uuid::Uuid::new_v4().to_string();
    save_state(&db, MoverBatchState::new(batch_id.clone(), request.clone(), trigger)).await;
    let batch_id_to_return = batch_id.clone();

    // 同时间旅行批次：独立多线程 runtime + 32MB 栈（Tauri 默认 2MB worker 栈会溢出，
    // 且引擎链路含 block_in_place，current-thread runtime 会 panic）。
    const BATCH_STACK: usize = 32 * 1024 * 1024;
    crate::spawn_fire_and_forget_multithread(
        "mover_recall_analysis",
        BATCH_STACK,
        4,
        BATCH_STACK,
        async move {
            execute_batch(batch_id, request, db, client, engine, permit).await;
            tracing::info!("[mover-recall-analysis] 批次结束");
        },
    );

    Ok(batch_id_to_return)
}

/// 查询批次状态（含逐票明细）；不存在返回 null。
#[agent_command(
    domain = "finance",
    safety = Safe,
    call_mode = StateInput,
    description = "查询漏检补分析批次状态"
)]
#[tauri::command]
pub async fn get_mover_recall_analysis_batch(
    state: State<'_, AppState>,
    batch_id: String,
) -> Result<Option<MoverBatchState>, String> {
    let db = state.harness.db();
    ensure_stale_reaped(db).await;

    if let Some(mut s) = BATCH_RUNS.lock().await.get(&batch_id).cloned() {
        // 运行态的 items 已在内存；保险起见不查库（内存是轮询权威）
        if !s.items.is_empty() || s.status == STATUS_RUNNING {
            return Ok(Some(s));
        }
        // 极端情况下内存态 items 为空（如未来代码路径），落到库反查补齐
        s.items = load_items(db, &batch_id).await?;
        return Ok(Some(s));
    }

    let model =
        mover_recall_analysis_batches::fetch_batch_by_id(db, &batch_id).await.map_err(|e| {
            ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("查询批次历史失败: {e}"))
        })?;
    match model {
        None => Ok(None),
        Some(m) => {
            let items = load_items(db, &batch_id).await?;
            Ok(Some(state_from_model(m, items)))
        },
    }
}

/// 批次历史（任务中心列表，摘要不含 items）。
#[agent_command(
    domain = "finance",
    safety = Safe,
    call_mode = StateInput,
    description = "查询漏检补分析批次历史"
)]
#[tauri::command]
pub async fn list_mover_recall_analysis_batches(
    state: State<'_, AppState>,
    limit: Option<u64>,
    status_filter: Option<String>,
) -> Result<Vec<MoverBatchState>, String> {
    let db = state.harness.db();
    ensure_stale_reaped(db).await;

    let rows = mover_recall_analysis_batches::list_recent_batch_runs(
        db,
        limit.unwrap_or(50).min(200),
        status_filter.as_deref(),
    )
    .await
    .map_err(|e| {
        ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("查询批次历史失败: {e}"))
    })?;
    Ok(rows.into_iter().map(|m| state_from_model(m, Vec::new())).collect())
}

async fn load_items(
    db: &axagent_harness::DatabaseConnection,
    batch_id: &str,
) -> Result<Vec<EventItem>, String> {
    mover_recall_events::list_by_batch(db, batch_id)
        .await
        .map(|rows| rows.iter().map(EventItem::from).collect())
        .map_err(|e| format!("查询批次逐票明细失败: {e}"))
}

// ── 后台执行体 ────────────────────────────────────────────────────────────────

/// 组内三步的**顺序契约**：回放 → 差异 → 逐票分析。
///
/// 为什么单成函数：顺序本身是 PLAN A-7 的一条可测要求（先分析会让漏检票自己的
/// as-of 分析进入定档池）。用「注入三个 future」的形式把「谁先被 await」变成可断言；
/// 生产调用点必须走这里，否则改回手写顺序就测不出来。
///
/// `diff` 只借用回放结果、不消费它：差异落库失败不影响后续分析。
async fn replay_then_analyze<R, A, DFut>(
    replay: impl std::future::Future<Output = Result<R, String>>,
    diff: impl FnOnce(Result<R, String>) -> DFut,
    analyze: impl std::future::Future<Output = A>,
) -> A
where
    DFut: std::future::Future<Output = ()>,
{
    let replay_result = replay.await;
    diff(replay_result).await;
    analyze.await
}

/// 一只待补事件与其 as-of 分组内的候选构造输入。
#[derive(Clone)]
struct PendingEvent {
    model: EventModel,
    period: Period,
}

#[allow(clippy::too_many_lines)]
async fn execute_batch(
    batch_id: String,
    request: RunMoverRecallAnalysisRequest,
    db: axagent_harness::DatabaseConnection,
    client: Arc<AStockClient>,
    engine: Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    // 持有到批次结束：drop 即释放全局 as-of 批次锁
    _permit: OwnedMutexGuard<()>,
) {
    let mut s = match BATCH_RUNS.lock().await.get(&batch_id).cloned() {
        Some(s) => s,
        None => {
            tracing::error!("[mover-recall-analysis] {batch_id} 内存运行态缺失，放弃后台执行");
            return;
        },
    };

    // ── 阶段 1：选样（pending/failed，剔除高 unexplained 轮次，配额截断）────────
    let (selected, selection) = match select_events(&db, &request).await {
        Ok(v) => v,
        Err(e) => {
            s.status = STATUS_FAILED.to_string();
            s.error_detail = Some(format!("选样失败: {e}"));
            s.finished_at_ms = Some(now_ms());
            save_state(&db, s).await;
            return;
        },
    };

    // E-4：选样留痕随批次一起落库（空批和有票批都要有）——否则「这批被什么跳过」
    // 只活在 tracing 里，面板与任务中心只能显示 status。
    s.scanned = Some(selection.scanned as i32);
    s.skipped_by_share = Some(selection.skipped_by_share as i32);
    s.skipped_by_period = Some(selection.skipped_by_period as i32);
    s.skipped_by_window = Some(selection.skipped_by_window as i32);

    let total = selected.len();

    if total == 0 {
        // 空态必须按实际跳过原因分桶：旧实现把「档位解析失败」等也写成
        // 「全部因 unexplained 占比超阈值」，还因单位错配把 0.5% 显示成 50%。
        let msg = empty_selection_message(selection, request.max_unexplained_share);
        if selection.scanned > 0 {
            tracing::warn!(
                "[mover-recall-analysis] {batch_id} 选样为空：scanned={} skipped_by_share={} \
                 skipped_by_period={} threshold={}",
                selection.scanned,
                selection.skipped_by_share,
                selection.skipped_by_period,
                request.max_unexplained_share
            );
        }
        s.status = STATUS_COMPLETED.to_string();
        s.message = Some(msg);
        s.finished_at_ms = Some(now_ms());
        save_state(&db, s).await;
        return;
    }

    // ── 阶段 2：认领（仅仍处于候选态的行会被认领）────────────────────────────
    let event_ids: Vec<String> = selected.iter().map(|p| p.model.id.clone()).collect();
    let claimed =
        match mover_recall_events::claim_events(&db, &event_ids, &batch_id, now_ms()).await {
            Ok(n) => n as usize,
            Err(e) => {
                s.status = STATUS_FAILED.to_string();
                s.error_detail = Some(format!("事件认领失败: {e}"));
                s.finished_at_ms = Some(now_ms());
                save_state(&db, s).await;
                return;
            },
        };
    if claimed != total {
        tracing::warn!(
            "[mover-recall-analysis] {batch_id} 计划认领 {total} 个事件，实际认领 {claimed} 个 \
             （部分事件已被并发认领或状态变化）"
        );
    }

    // 内存明细：以实际认领成功的事件为准
    let claimed_ids: std::collections::HashSet<String> =
        mover_recall_events::list_by_batch(&db, &batch_id)
            .await
            .map(|rows| rows.iter().map(|r| r.id.clone()).collect())
            .unwrap_or_default();
    let selected: Vec<PendingEvent> =
        selected.into_iter().filter(|p| claimed_ids.contains(&p.model.id)).collect();
    let total = selected.len();
    // 本批会改动哪些轮次的 as-of 摘要（批次结束时按轮重算一次）。
    let touched_runs: std::collections::HashSet<String> =
        selected.iter().map(|p| p.model.run_id.clone()).collect();
    s.total = total;
    s.items = selected.iter().map(|p| EventItem::from(&p.model)).collect();
    save_state(&db, s.clone()).await;

    // ── 阶段 3：按 window_start 分组，逐日 as-of 回放 ──────────────────────────
    // BTreeMap 保证日期升序（先补早窗口）；组内保持选样顺序（累计涨幅降序）。
    let mut groups: BTreeMap<String, Vec<PendingEvent>> = BTreeMap::new();
    for p in selected {
        groups.entry(p.model.window_start.clone()).or_default().push(p);
    }

    let mut ok_count = 0usize;
    let mut fail_count = 0usize;

    for (window_start, group) in groups {
        let date = match chrono::NaiveDate::parse_from_str(&window_start, "%Y-%m-%d") {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(
                    "[mover-recall-analysis] {batch_id} window_start={window_start} 日期非法: {e}"
                );
                for p in &group {
                    finish_item_failed(
                        &db,
                        &mut s,
                        &p.model.id,
                        &format!("window_start 日期非法: {e}"),
                    )
                    .await;
                    fail_count += 1;
                }
                save_state(&db, s.clone()).await;
                continue;
            },
        };
        let ctx = match AsOfContext::new(date, AsOfSource::ScheduledReplay) {
            Ok(ctx) => ctx,
            Err(e) => {
                // 晚于今日等契约失败：整日事件失败（不释放，error 写明原因）
                for p in &group {
                    finish_item_failed(&db, &mut s, &p.model.id, &format!("as-of 锚点无效: {e}"))
                        .await;
                    fail_count += 1;
                }
                save_state(&db, s.clone()).await;
                continue;
            },
        };

        let db_ref = db.clone();
        let client_ref = client.clone();
        let engine_ref = engine.clone();
        let batch_id_ref = batch_id.clone();
        let window_ref = window_start.clone();
        let period_by_event: std::collections::HashMap<String, String> =
            group.iter().map(|p| (p.model.id.clone(), p.model.period.clone())).collect();
        // 一次回放的批次键（写 picks / 池批次 / 留痕三表 + 事件差异），必须在 guard 外
        // 先定值 —— 回放整体失败时事件仍要记「哪一次尝试失败」。
        let asof_generated_at = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
        // guard 内：**先** as-of 荐股回放，**再**逐只分析（顺序不可换：先分析会让漏检票
        // 自己的 as-of 分析进入定档池，等于把已知清单喂回候选池）。回放失败不阻塞分析。
        // 顺序走 `replay_then_analyze`（A-7 的可测契约），不在这里手写三段 await。
        // 差异闭包与逐票分析都要用 group：先克隆一份给差异，避免「借用 + 移动」撞车。
        let group_for_diff = group.clone();
        let pair = as_of::with_optional_asof(Some(ctx), async {
            replay_then_analyze(
                run_asof_reco_replay(&db_ref, &client_ref, &window_ref, &asof_generated_at),
                |res| {
                    let diff_group = group_for_diff.clone();
                    let db = db_ref.clone();
                    let ws = window_ref.clone();
                    let at = asof_generated_at.clone();
                    async move {
                        match res {
                            Ok(r) => {
                                apply_asof_diff(&db, &ws, &at, &diff_group, Some(&r), None).await;
                            },
                            Err(e) => {
                                tracing::warn!(
                                    "[mover-recall-analysis] as_of={ws} 荐股回放失败（逐票分析照跑）: {e}"
                                );
                                apply_asof_diff(
                                    &db,
                                    &ws,
                                    &at,
                                    &diff_group,
                                    None,
                                    Some(e.as_str()),
                                )
                                .await;
                            },
                        }
                    }
                },
                run_group_with_retry(
                    &db_ref,
                    &client_ref,
                    &engine_ref,
                    &batch_id_ref,
                    &window_ref,
                    group,
                ),
            )
            .await
        })
        .await;

        // guard 外回写事件状态（无 as_of 取数依赖，只是状态落库）
        for (event_id, result) in pair {
            match result {
                Ok(analysis_id) => {
                    if let Some(period) = period_by_event.get(&event_id) {
                        write_analysis_signal(&db, &event_id, &analysis_id, period).await;
                    }
                    if let Err(e) = mover_recall_events::finish_event(
                        &db,
                        &event_id,
                        EVENT_ANALYZED,
                        Some(&analysis_id),
                        None,
                        now_ms(),
                    )
                    .await
                    {
                        tracing::error!(
                            "[mover-recall-analysis] 事件 {event_id} analyzed 回写失败: {e}"
                        );
                    }
                    update_item(&mut s, &event_id, |it| {
                        it.status = EVENT_ANALYZED.to_string();
                        it.analysis_id = Some(analysis_id);
                        it.error_detail = None;
                    });
                    ok_count += 1;
                },
                Err(err) => {
                    if let Err(e) = mover_recall_events::finish_event(
                        &db,
                        &event_id,
                        EVENT_FAILED,
                        None,
                        Some(&err),
                        now_ms(),
                    )
                    .await
                    {
                        tracing::error!(
                            "[mover-recall-analysis] 事件 {event_id} failed 回写失败: {e}"
                        );
                    }
                    update_item(&mut s, &event_id, |it| {
                        it.status = EVENT_FAILED.to_string();
                        it.error_detail = Some(err);
                    });
                    fail_count += 1;
                },
            }
        }
        s.ok = ok_count;
        s.fail = fail_count;
        save_state(&db, s.clone()).await;
    }

    // 轮级 as-of 摘要：本批覆盖过的每一轮都按「该轮全部已回填事件」重算一次。
    persist_run_asof_summaries(&db, &touched_runs).await;

    // ── 终态 ─────────────────────────────────────────────────────────────────
    s.ok = ok_count;
    s.fail = fail_count;
    s.status = if fail_count == 0 {
        STATUS_COMPLETED
    } else {
        STATUS_PARTIAL
    }
    .to_string();
    if fail_count > 0 {
        s.message = Some(format!(
            "{ok_count} 只补分析成功，{fail_count} 只在重试一次后仍失败（可在非 live 时间窗再次发起批次重试）"
        ));
    }
    s.finished_at_ms = Some(now_ms());
    save_state(&db, s).await;
}

/// 选样统计：空态消息只按这里的分桶成句，不再猜唯一原因。
#[derive(Debug, Default, Clone, Copy)]
struct SelectionStats {
    scanned: usize,
    skipped_by_share: usize,
    skipped_by_period: usize,
    /// 窗口起点早于所属核查轮次 `date_from` ⇒ 伪窗口（历史回补稀疏行），不进补分析。
    skipped_by_window: usize,
}

/// 阈值与 share 必须同为 0-1 归一化占比。缺占比信息（null/无字段）不拦截。
fn share_blocks(share: Option<f64>, threshold: f64) -> bool {
    share.is_some_and(|s| s >= threshold)
}

fn empty_selection_message(stats: SelectionStats, threshold: f64) -> String {
    if stats.scanned == 0 {
        return "当前没有待补分析的漏检事件（pending / failed 均为空）".to_string();
    }
    let mut reasons = Vec::new();
    if stats.skipped_by_share > 0 {
        reasons.push(format!(
            "{} 个所属轮次 unexplained 占比 ≥ {:.0}%（阈值按 0-1 归一化占比计）",
            stats.skipped_by_share,
            threshold * 100.0
        ));
    }
    if stats.skipped_by_period > 0 {
        reasons.push(format!("{} 个事件的档位无法解析", stats.skipped_by_period));
    }
    if stats.skipped_by_window > 0 {
        reasons.push(format!(
            "{} 个事件的窗口起点早于所属轮次 date_from（历史回补稀疏行，伪窗口）",
            stats.skipped_by_window
        ));
    }
    let classified = stats.skipped_by_share + stats.skipped_by_period + stats.skipped_by_window;
    if classified < stats.scanned {
        reasons.push(format!("{} 个原因未记录", stats.scanned - classified));
    }
    format!("候选 {} 个事件全部被跳过：{}", stats.scanned, reasons.join("；"))
}

/// 选样：候选事件 → 剔除高 unexplained 轮次 → 配额截断 → 可解析档位。
async fn select_events(
    db: &axagent_harness::DatabaseConnection,
    request: &RunMoverRecallAnalysisRequest,
) -> Result<(Vec<PendingEvent>, SelectionStats), String> {
    let shares = mover_recall_events::load_run_unexplained_shares(db)
        .await
        .map_err(|e| format!("读核查轮次 unexplained 占比失败: {e}"))?;
    let date_froms = mover_recall_events::load_run_date_froms(db).await.map_err(|e| {
        ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("读核查轮次 date_from 失败: {e}"))
            .to_string()
    })?;
    let rows = mover_recall_events::list_candidate_event_rows(
        db,
        request.run_id.as_deref(),
        SELECT_SCAN_LIMIT.saturating_mul(CHAIN_ROWS_PER_EVENT),
    )
    .await
    .map_err(|e| format!("读待补事件失败: {e}"))?;

    // 按三键分组：同一事件两链各一行。只要有一条链的轮次通过阈值就选它 ——
    // 不能先 dedupe 再看保留行，保留哪条链由同毫秒写入顺序决定，会让
    // 「同一只票能否被补分析」带随机性。
    let mut groups: Vec<Vec<EventModel>> = Vec::new();
    let mut index: HashMap<(String, String, String), usize> = HashMap::new();
    for m in rows {
        let key = (m.stock_code.clone(), m.anchor_date.clone(), m.period.clone());
        if let Some(&i) = index.get(&key) {
            groups[i].push(m);
        } else {
            index.insert(key, groups.len());
            groups.push(vec![m]);
        }
    }

    let mut stats = SelectionStats { scanned: groups.len(), ..Default::default() };
    let mut out = Vec::new();
    for group in groups {
        // 整组都超阈值才跳过；任一条链通过（或占比缺失）都保留该事件。
        let all_blocked = group
            .iter()
            .all(|m| share_blocks(shares.get(&m.run_id).copied(), request.max_unexplained_share));
        if all_blocked {
            stats.skipped_by_share += 1;
            continue;
        }
        // 三键去重后**优先保留荐股链那一行**：as-of 回放证据只写在 reco 行上，
        // 补分析的 anchor_event_id 又直接取这一行的 id ⇒ 若留下的是 serenity 行，
        // 反思侧 `mover_event.asof_layer` 永远为空，「吃差异」会静默退化成「吃 live」。
        let m = group
            .into_iter()
            .min_by_key(|m| {
                if m.chain.as_deref() == Some("reco") {
                    0
                } else {
                    1
                }
            })
            .expect("group 非空");
        // 伪窗口：窗口起点早于该轮核查的数据起点 ⇒ 历史回补稀疏行拼出来的事件，不补分析。
        if let Some(from) = date_froms.get(&m.run_id) {
            if m.window_start.as_str() < from.as_str() {
                stats.skipped_by_window += 1;
                continue;
            }
        }
        let period = match m.period.parse::<Period>() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(
                    "[mover-recall-analysis] 事件 {} period={:?} 无法解析，跳过: {e}",
                    m.id,
                    m.period
                );
                stats.skipped_by_period += 1;
                continue;
            },
        };
        out.push(PendingEvent { model: m, period });
        if out.len() >= request.batch_limit {
            break;
        }
    }
    Ok((out, stats))
}

/// guard 内执行一个 as-of 日分组的逐只分析（首趟 + 失败重试一趟）。
/// 返回 (event_id, Ok(analysis_id)/Err(最后一次错误))，顺序不做承诺。
async fn run_group_with_retry(
    db: &axagent_harness::DatabaseConnection,
    client: &Arc<AStockClient>,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    batch_id: &str,
    window_start: &str,
    group: Vec<PendingEvent>,
) -> Vec<(String, Result<String, String>)> {
    let candidates: Vec<PoolCandidate> = group
        .iter()
        .map(|p| PoolCandidate {
            stock_code: p.model.stock_code.clone(),
            stock_name: p.model.stock_name.clone(),
            period: p.period,
            confidence: 0,
            anchor_event_id: Some(p.model.id.clone()),
        })
        .collect();

    let outcome =
        run_analysis_for_candidates(db, client, engine, candidates.clone(), "mover_recall").await;

    // results 与 candidates 严格同序同长（执行体逐条 push，成功失败都 push；本入口不去重）
    let mut pairs: Vec<(String, Result<String, String>)> = Vec::with_capacity(group.len());
    let mut retry: Vec<(usize, PoolCandidate)> = Vec::new();
    for (idx, (p, r)) in group.iter().zip(outcome.results).enumerate() {
        match r.analysis_id {
            Some(aid) => pairs.push((p.model.id.clone(), Ok(aid))),
            None => {
                let err = r.error.unwrap_or_else(|| "分析失败（无错误详情）".to_string());
                pairs.push((p.model.id.clone(), Err(err)));
                retry.push((idx, candidates[idx].clone()));
            },
        }
    }

    if retry.is_empty() {
        return pairs;
    }
    tracing::info!(
        "[mover-recall-analysis] {batch_id} as_of={window_start} 首趟 {n} 只失败，重试一趟",
        n = retry.len()
    );
    let retry_cands: Vec<PoolCandidate> = retry.iter().map(|(_, c)| c.clone()).collect();
    let retry_outcome =
        run_analysis_for_candidates(db, client, engine, retry_cands, "mover_recall").await;
    for ((idx, _), r) in retry.iter().zip(retry_outcome.results) {
        let event_id = group[*idx].model.id.clone();
        // 用重试结果覆盖 pairs 中同事件的首趟失败
        let slot = pairs
            .iter_mut()
            .find(|(id, _)| *id == event_id)
            .expect("pairs 与 group 同源，事件必在其中");
        match r.analysis_id {
            Some(aid) => *slot = (event_id, Ok(aid)),
            None => {
                let err = r.error.unwrap_or_else(|| "重试仍失败（无错误详情）".to_string());
                *slot = (event_id, Err(err));
            },
        }
    }
    pairs
}

/// guard 外把整日失败（日期/as-of 契约非法）的事件回写 failed 并同步内存明细。
async fn finish_item_failed(
    db: &axagent_harness::DatabaseConnection,
    s: &mut MoverBatchState,
    event_id: &str,
    err: &str,
) {
    if let Err(e) =
        mover_recall_events::finish_event(db, event_id, EVENT_FAILED, None, Some(err), now_ms())
            .await
    {
        tracing::error!("[mover-recall-analysis] 事件 {event_id} failed 回写失败: {e}");
    }
    update_item(s, event_id, |it| {
        it.status = EVENT_FAILED.to_string();
        it.error_detail = Some(err.to_string());
    });
}

fn update_item(s: &mut MoverBatchState, event_id: &str, f: impl FnOnce(&mut EventItem)) {
    if let Some(it) = s.items.iter_mut().find(|it| it.event_id == event_id) {
        f(it);
    }
}

// ── as-of 荐股回放与差异对账（阶段 A）────────────────────────────────────────
//
// ## 为什么两步、且回放在前
//
// `served_vars_with_analysis_tiers`（live 四档入口）会把 `stock_analyses.as_of_date =
// window_start` 的定档变量并进 `reco_analysis_tiers`。若先跑逐票分析，漏检票自己的
// as-of 分析会进入定档池 —— 等于把「我们已知的漏检清单」喂回候选池，回放不再独立。
// 因此回放只吃 `load_reco_served_vars_db`（当前模板 + 当前闭环权重），并**刻意不喂**
// 分析定档来源；差异 JSON 的 `analysisTierSourceIncluded=false` 是对这条的显式声明。
//
// ## 语义边界（不得混读）
//
// - 回放回答的是「**当前系统**在历史数据上会怎么选」，不是「当时的系统怎么选」
//   （后者要历史权重，与逐票 as-of 分析口径不一致 —— PLAN 待拍板 #1 的推荐项）。
// - `asof_layer` 与 `miss_layer` 分列：前者是反思的权威归因，后者是 live 事实；
//   两者分歧本身是诊断信息，任何一侧都不得顶替另一侧。
// - 池 `None`（取数缺痕）⇒ `unexplained`；池 `"[]"`（真空池）⇒ `pool_degraded`。
//   把前者写成后者等于把「拿不到」演成「池子漏了它」。
// - `picked_before_window` 在 as-of 侧**恒 false**：回放只有窗口起点一天，
//   「此前覆盖」无从判定，不伪造。

/// 涨幅榜取数上限（与 `refresh_mover_seed_db` 的 `MOVER_TOP_N` 同口径）。
const ASOF_MOVER_TOP_N: u64 = 50;

/// as-of 回放里的一条出票（只保留归因需要的四列）。
#[derive(Debug, Clone)]
struct AsOfPick {
    code: String,
    period: String,
    style: String,
    synthetic: bool,
}

/// 一次 `window_start` 的 as-of 回放证据包。
#[derive(Debug, Default)]
struct AsOfReplay {
    /// 档位 → 实际池快照（`None` = 引擎没给；空串/`"[]"` = 真空池）
    pools: std::collections::HashMap<String, Option<String>>,
    picks: Vec<AsOfPick>,
    /// (档位, 代码) → 截断风格（矩阵名目归一后）
    trimmed: std::collections::HashMap<(String, String), Vec<String>>,
    /// 档位 → 降级风格
    degraded: std::collections::HashMap<String, Vec<String>>,
    /// 档位 → 回放失败原因
    failures: std::collections::HashMap<String, String>,
    /// 档位 → 池来源构成（`SeedPoolOrigin` 序列化）
    pool_origin: std::collections::HashMap<String, serde_json::Value>,
    mover_seed_count: usize,
}

/// 跑一次 `window_start` 的 as-of 荐股回放，并在**同一 `generated_at`** 下落三张表。
///
/// 调用方必须先进入 `AsOfContext::new(window_start, ScheduledReplay)` 作用域 ——
/// 本函数自身不建 scope（批次里同一次 guard 还要跑逐票分析）。
async fn run_asof_reco_replay(
    db: &axagent_harness::DatabaseConnection,
    client: &Arc<AStockClient>,
    window_start: &str,
    generated_at: &str,
) -> Result<AsOfReplay, String> {
    let started = std::time::Instant::now();
    let vars = recommender::reco_loop::load_reco_served_vars_db(db).await?;
    // 先验按窗口起点截断（PLAN 待拍板 #7 的推荐项）：沿用全量样本会把
    // window_start 之后才结算的样本算进历史决策。
    let prior = horizon_prior::horizon_prior_from_vars_db(db, &vars, Some(window_start)).await;

    // 涨幅榜：全市场、截至窗口起点最近交易日的收盘快照（ST 只能按名称剔）。
    let mover_raw =
        reco_seed::top_close_rows_by_change_on_or_before(db, window_start, ASOF_MOVER_TOP_N * 4)
            .await
            .map_err(|e| {
                ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("读收盘快照失败: {e}"))
            })?;
    let mover: Vec<SeedItem> = mover_raw
        .into_iter()
        .filter(|(_, name)| !name.contains("ST"))
        .take(ASOF_MOVER_TOP_N as usize)
        .map(|(code, name)| (code, name, None))
        .collect();

    let weights_json = vars
        .iter()
        .find(|(k, _)| k == "reco_strategy_weights")
        .and_then(|(_, v)| v.is_object().then(|| v.to_string()));

    let results = recommender::recommend_stocks_multi(
        client.clone(),
        &Period::ALL,
        &vars,
        None,
        prior,
        SeedOverrides::asof_replay(mover.clone()),
    )
    .await;

    let mut replay = AsOfReplay { mover_seed_count: mover.len(), ..Default::default() };
    for (period, res) in results {
        let p = period.as_str().to_string();
        match res {
            Err(e) => {
                replay.failures.insert(p, e);
            },
            Ok(resp) => {
                let snapshot = resp.seed_pool_snapshot.clone();
                replay.pool_origin.insert(
                    p.clone(),
                    serde_json::to_value(resp.seed_pool_origin).unwrap_or(serde_json::Value::Null),
                );
                replay.degraded.insert(
                    p.clone(),
                    resp.degraded_styles.iter().map(|s| s.as_str().to_string()).collect(),
                );
                if let Some(detail) = resp.error_detail.as_deref() {
                    // 两源全挂时引擎返回 `Ok` + `error_detail`（不静默出空表）——
                    // 这同样是「本次回放失败」，池 `None` 只能落 `unexplained`。
                    replay.failures.insert(p.clone(), detail.to_string());
                }
                let flat: Vec<RecoPick> =
                    resp.picks.values().flat_map(|v| v.iter().cloned()).collect();
                for pick in &flat {
                    replay.picks.push(AsOfPick {
                        code: pick.stock_code.clone(),
                        period: p.clone(),
                        // 与 trimmedStyles 同一写法（矩阵名目）：否则面板两列一名两形。
                        style: mover_core::normalize_style_key(pick.style.as_str()).to_string(),
                        synthetic: pick.synthetic,
                    });
                }
                for t in &resp.scan_audit {
                    let entry =
                        replay.trimmed.entry((p.clone(), t.stock_code.clone())).or_default();
                    let key = mover_core::normalize_style_key(t.style.as_str()).to_string();
                    if !entry.contains(&key) {
                        entry.push(key);
                    }
                }

                // 逐档落库（与 live 同形态：picks / 池批次 / L3 留痕共用 generated_at）。
                // 代际与用途显式具名：`as_of_date` = 窗口起点，`generated_at` = 本次运行时刻
                // （差异查询按后者取，避免同一天多次回放串批 —— 代际门 R10）。
                let as_of_date = Some(window_start);
                let written = crate::commands::_shared_reco_pool::persist_reco_picks(
                    db,
                    &flat,
                    snapshot.clone(),
                    weights_json.clone(),
                    &axagent_dao::repo::reco_pool::BatchKey {
                        generated_at,
                        as_of_date,
                        purpose: crate::commands::_shared_reco_pool::purpose::MOVER_REPLAY,
                    },
                )
                .await;
                crate::commands::_shared_reco_pool::persist_reco_pool_batch(
                    db,
                    "reco",
                    &p,
                    snapshot.as_deref(),
                    generated_at,
                    as_of_date,
                    crate::commands::_shared_reco_pool::purpose::MOVER_REPLAY,
                )
                .await;
                if !resp.scan_audit.is_empty() {
                    crate::commands::_shared_reco_pool::persist_reco_scan_audit(
                        db,
                        &resp.scan_audit,
                        period,
                        generated_at,
                        as_of_date,
                        crate::commands::_shared_reco_pool::purpose::MOVER_REPLAY,
                    )
                    .await;
                }
                tracing::info!(
                    "[mover-recall-analysis] as_of={window_start} 档 {p}：出票 {} / 池 {} / 截断 {} / 落库 {written}",
                    flat.len(),
                    snapshot.as_deref().map(|s| s.len()).unwrap_or(0),
                    resp.scan_audit.len()
                );
                replay.pools.insert(p, snapshot);
            },
        }
    }
    tracing::info!(
        "[mover-recall-analysis] as_of={window_start} as-of 荐股回放完成：{} 档成功 / {} 档失败，\
         涨幅榜种子 {} 只，耗时 {}ms",
        replay.pools.len(),
        replay.failures.len(),
        replay.mover_seed_count,
        started.elapsed().as_millis()
    );
    Ok(replay)
}

/// 差异对账的纯输入（只含判据需要的字段，便于不连库逐层断言）。
struct AsOfDiffInput<'a> {
    code: &'a str,
    period: &'a str,
    window_start: &'a str,
    generated_at: &'a str,
    live_layer: &'a str,
    /// 该档实际池快照：`None` = 引擎没给；`Some("[]")` = 真空池。
    pool_snapshot: Option<&'a str>,
    same_period_styles: &'a [String],
    other_periods: &'a [String],
    trimmed_styles: &'a [String],
    degraded_styles: &'a [String],
    pool_origin: serde_json::Value,
    mover_seed_count: usize,
    failure: Option<&'a str>,
}

/// 纯函数：按 as-of 证据算归因层，并组装落库用的差异 JSON。
///
/// 复用 [`mover_core::attribute`]（八层语义不变），只替换证据来源。
fn build_asof_diff(input: &AsOfDiffInput<'_>) -> (mover_core::MissLayer, serde_json::Value) {
    let (in_pool, pool_state) = match input.pool_snapshot {
        None => (None, "absent"),
        Some(raw) if raw.trim().is_empty() || raw.trim() == "[]" => (Some(false), "empty"),
        // 非空但形状不可解析 ⇒ `None`（无从判定），不得折成「不在池」。
        Some(raw) => (mover_core::code_in_pool_snapshot_opt(Some(raw), input.code), "present"),
    };
    let period = input.period.parse::<Period>().ok();
    let layer = if input.failure.is_some() {
        // 回放整体失败/该档失败 ⇒ 没有证据可判，落兜底桶并写明原因。
        mover_core::MissLayer::Unexplained
    } else {
        mover_core::attribute(&mover_core::Evidence {
            picks_for_period: input.same_period_styles.to_vec(),
            picked_in_other_period: !input.other_periods.is_empty(),
            in_pool,
            pool_degraded: pool_state == "empty",
            chain_tier_active: period.map(mover_core::period_has_active_style).unwrap_or(false),
            // as-of 回放只有窗口起点一天：不伪造「此前覆盖」（该层只在 live 归因保留）。
            picked_before_window: false,
            l3_scored_out_styles: input.trimmed_styles.to_vec(),
        })
    };
    let json = serde_json::json!({
        "asOfDate": input.window_start,
        "generatedAt": input.generated_at,
        "period": input.period,
        "code": input.code,
        "replayFailure": input.failure,
        "poolState": pool_state,
        "inPool": in_pool,
        "poolOrigin": input.pool_origin,
        "moverSeedCount": input.mover_seed_count,
        "pickedStyles": input.same_period_styles,
        "pickedOtherPeriods": input.other_periods,
        "trimmedStyles": input.trimmed_styles,
        "degradedStyles": input.degraded_styles,
        // 显式声明：本轮 as-of 回放刻意未含「分析定档」来源（防把已知漏检清单喂回池）。
        "analysisTierSourceIncluded": false,
        "liveLayer": input.live_layer,
        "divergedFromLive": layer.as_str() != input.live_layer,
    });
    (layer, json)
}

/// 把一次回放的差异逐事件落库（只写**荐股链**行；智选链不做同款回放）。
async fn apply_asof_diff(
    db: &axagent_harness::DatabaseConnection,
    window_start: &str,
    generated_at: &str,
    group: &[PendingEvent],
    replay: Option<&AsOfReplay>,
    whole_failure: Option<&str>,
) {
    for p in group {
        let code = p.model.stock_code.as_str();
        let period = p.period.as_str();
        let mut same_styles: Vec<String> = Vec::new();
        let mut other_periods: Vec<String> = Vec::new();
        if let Some(r) = replay {
            for pick in &r.picks {
                if pick.synthetic || pick.code != code {
                    continue;
                }
                if pick.period == period {
                    if !same_styles.contains(&pick.style) {
                        same_styles.push(pick.style.clone());
                    }
                } else if !other_periods.contains(&pick.period) {
                    other_periods.push(pick.period.clone());
                }
            }
        }
        let empty: Vec<String> = Vec::new();
        let trimmed = replay
            .and_then(|r| r.trimmed.get(&(period.to_string(), code.to_string())))
            .unwrap_or(&empty);
        let degraded = replay.and_then(|r| r.degraded.get(period)).unwrap_or(&empty);
        let origin = replay
            .and_then(|r| r.pool_origin.get(period))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let snapshot = replay.and_then(|r| r.pools.get(period)).and_then(|o| o.as_deref());
        let failure = whole_failure
            .map(str::to_string)
            .or_else(|| replay.and_then(|r| r.failures.get(period)).cloned());

        let input = AsOfDiffInput {
            code,
            period,
            window_start,
            generated_at,
            live_layer: p.model.miss_layer.as_str(),
            pool_snapshot: snapshot,
            same_period_styles: &same_styles,
            other_periods: &other_periods,
            trimmed_styles: trimmed,
            degraded_styles: degraded,
            pool_origin: origin,
            mover_seed_count: replay.map(|r| r.mover_seed_count).unwrap_or(0),
            failure: failure.as_deref(),
        };
        let (layer, diff) = build_asof_diff(&input);
        let diff_json = diff.to_string();
        if let Err(e) = mover_recall_events::update_asof_for_key_in_run(
            db,
            &p.model.run_id,
            code,
            &p.model.anchor_date,
            &p.model.period,
            &mover_recall_events::AsOfEvidence {
                generated_at,
                layer: layer.as_str(),
                diff_json: &diff_json,
            },
            now_ms(),
        )
        .await
        {
            tracing::warn!(
                "[mover-recall-analysis] as_of={window_start} 事件 {} 差异回填失败: {e}",
                p.model.id
            );
        }
    }
}

/// 事件档决策摘要（纯函数，F-4）：从 `stock_analyses.horizon_decisions` 取**事件档**的
/// 动作与置信度。双键取（snake / camel）与 `reviewable_horizons` 同一口径。
///
/// 无逐档决策 / 该档缺席 ⇒ `None`（面板显示「未记录」，不得折成 hold）。
fn event_decision_signal_json(
    horizon_decisions: Option<&str>,
    period: &str,
) -> Option<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(horizon_decisions?).ok()?;
    let obj = v.as_object()?;
    let camel = period.parse::<Period>().ok().map(|p| p.camel_key()).unwrap_or(period);
    let d = obj.get(period).or_else(|| obj.get(camel))?;
    Some(serde_json::json!({
        "horizon": period,
        "action": d.get("action").and_then(|x| x.as_str()),
        "confidence": d.get("confidence").and_then(|x| x.as_f64()),
    }))
}

/// 补分析成功后回写事件档决策摘要（F-4）。失败只 warn：补分析本身已成功，
/// 面板少一行摘要不该把事件降级成 failed。
async fn write_analysis_signal(
    db: &axagent_harness::DatabaseConnection,
    event_id: &str,
    analysis_id: &str,
    period: &str,
) {
    let model = match axagent_dao::repo::stock_analysis_snapshot::fetch_analysis_by_id(
        db,
        analysis_id,
    )
    .await
    {
        Ok(Some(m)) => m,
        Ok(None) => {
            tracing::warn!(
                "[mover-recall-analysis] 事件 {event_id} 的补分析行 {analysis_id} 查不到 ⇒ 决策摘要不写"
            );
            return;
        },
        Err(e) => {
            tracing::warn!("[mover-recall-analysis] 事件 {event_id} 读补分析行失败: {e}");
            return;
        },
    };
    let Some(signal) = event_decision_signal_json(model.horizon_decisions.as_deref(), period)
    else {
        return;
    };
    if let Err(e) =
        mover_recall_events::update_analysis_signal(db, event_id, &signal.to_string(), now_ms())
            .await
    {
        tracing::warn!("[mover-recall-analysis] 事件 {event_id} 决策摘要回写失败: {e}");
    }
}

/// 轮级 as-of 摘要（纯函数）：输入该轮已回填 as-of 的事件行。
///
/// 返回 `(asof_layers_json, asof_diff_summary_json)`。两层口径：
/// - `byLayer`：机械归因层 × 档（八层语义不变）；
/// - `byPeriod`：面板要的六桶（进池未出票 / 进池被截断 / 别档出票 / 未进池 /
///   池降级 / 口径不可用）。`unavailable` 桶专门收「池为 None 或回放失败」——
///   把它并进 `notInPool` 就是把取数缺痕写成「池子漏了它」。
fn aggregate_asof_summary(rows: &[EventModel]) -> (String, String) {
    let mut layers: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut by_period: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    let mut by_style: BTreeMap<String, BTreeMap<String, (usize, usize)>> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    let mut diverged = 0usize;

    for row in rows {
        let layer = row.asof_layer.clone().unwrap_or_else(|| "unavailable".to_string());
        *layers.entry(layer.clone()).or_default().entry(row.period.clone()).or_default() += 1;

        let diff: serde_json::Value = row
            .asof_diff_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null);
        let in_pool = diff.get("inPool").and_then(|v| v.as_bool());
        let bucket: &'static str = match layer.as_str() {
            "recommended" => "picked",
            "prior_covered" => "priorCovered",
            "by_design" => "byDesign",
            "l3_scored_out" => "inPoolTrimmed",
            "other_period" => "otherPeriod",
            "not_in_pool" => "notInPool",
            "pool_degraded" => "poolDegraded",
            _ => {
                if in_pool == Some(true) {
                    "inPoolNotPicked"
                } else {
                    "unavailable"
                }
            },
        };
        *by_period.entry(row.period.clone()).or_default().entry(bucket).or_default() += 1;

        if let Some(arr) = diff.get("pickedStyles").and_then(|v| v.as_array()) {
            for st in arr.iter().filter_map(|v| v.as_str()) {
                by_style
                    .entry(row.period.clone())
                    .or_default()
                    .entry(st.to_string())
                    .or_default()
                    .0 += 1;
            }
        }
        if let Some(arr) = diff.get("trimmedStyles").and_then(|v| v.as_array()) {
            for st in arr.iter().filter_map(|v| v.as_str()) {
                by_style
                    .entry(row.period.clone())
                    .or_default()
                    .entry(st.to_string())
                    .or_default()
                    .1 += 1;
            }
        }
        if let Some(f) = diff.get("replayFailure").and_then(|v| v.as_str()) {
            failures.push(format!("{} {}: {f}", row.period, row.stock_code));
        }
        if diff.get("divergedFromLive").and_then(|v| v.as_bool()) == Some(true) {
            diverged += 1;
        }
    }

    let style_json: BTreeMap<String, BTreeMap<String, serde_json::Value>> = by_style
        .into_iter()
        .map(|(p, styles)| {
            (
                p,
                styles
                    .into_iter()
                    .map(|(s, (picked, trimmed))| {
                        (s, serde_json::json!({"picked": picked, "trimmed": trimmed}))
                    })
                    .collect(),
            )
        })
        .collect();

    let layers_json = serde_json::to_string(&layers).unwrap_or_else(|_| "{}".to_string());
    let summary_json = serde_json::json!({
        "events": rows.len(),
        "byLayer": layers,
        "byPeriod": by_period,
        "byStyle": style_json,
        "divergedFromLive": diverged,
        "failures": failures,
        "analysisTierSourceIncluded": false,
    })
    .to_string();
    (layers_json, summary_json)
}

/// 批次结束时按轮重算 as-of 摘要并回填 `mover_recall_runs`。
async fn persist_run_asof_summaries(
    db: &axagent_harness::DatabaseConnection,
    run_ids: &std::collections::HashSet<String>,
) {
    for run_id in run_ids {
        match mover_recall_events::list_asof_rows_for_run(db, run_id).await {
            Ok(rows) if !rows.is_empty() => {
                let (layers, summary) = aggregate_asof_summary(&rows);
                if let Err(e) = axagent_dao::repo::mover_recall_runs::update_asof_summary(
                    db, run_id, &layers, &summary,
                )
                .await
                {
                    tracing::warn!("[mover-recall-analysis] 轮次 {run_id} as-of 摘要回填失败: {e}");
                }
            },
            Ok(_) => {},
            Err(e) => {
                tracing::warn!("[mover-recall-analysis] 轮次 {run_id} as-of 事件读取失败: {e}")
            },
        }
    }
}

#[cfg(test)]
mod cron_config_tests {
    use super::*;

    /// 内置任务的配额来自 [`CRON_BATCH_LIMIT`]，并且**写进 prompt 看得见**：
    /// 任务行是用户能改的对象，把配额只留在代码里会让面板显示与实跑两套。
    #[test]
    fn cron_request_roundtrips_through_job_prompt() {
        let raw = RunMoverRecallAnalysisRequest::cron_default().to_json().unwrap();
        assert!(raw.contains(&CRON_BATCH_LIMIT.to_string()), "prompt 应带上配额，实得 {raw}");
        let back = RunMoverRecallAnalysisRequest::from_json(&raw).unwrap();
        assert_eq!(back.batch_limit, CRON_BATCH_LIMIT);
        assert_eq!(back.max_unexplained_share, 0.5);
        assert_eq!(back.run_id, None);
    }

    /// 空 prompt = 正常形态（用缺省）；非空却解不出来 = 显式 `Err`，不静默兜底。
    #[test]
    fn cron_prompt_empty_is_default_but_garbage_is_error() {
        assert_eq!(
            RunMoverRecallAnalysisRequest::from_json("").unwrap().batch_limit,
            CRON_BATCH_LIMIT
        );
        assert!(RunMoverRecallAnalysisRequest::from_json("   ").unwrap().run_id.is_none());
        assert!(RunMoverRecallAnalysisRequest::from_json("not json").is_err());
    }

    /// 字段名只认 camelCase（与手动命令同一套 serde 契约）。
    /// 这条是防「种子写 batchLimit、读侧认 batch_limit」——snake 拼写会被当成缺字段，
    /// 配额静默落回缺省且不报错，正是本轮要消灭的形态。
    #[test]
    fn cron_prompt_requires_camel_case_keys() {
        let ok = RunMoverRecallAnalysisRequest::from_json(
            r#"{"batchLimit":3,"maxUnexplainedShare":0.25}"#,
        )
        .unwrap();
        assert_eq!(ok.batch_limit, 3);
        assert_eq!(ok.max_unexplained_share, 0.25);

        let snake = RunMoverRecallAnalysisRequest::from_json(r#"{"batch_limit":3}"#).unwrap();
        assert_eq!(
            snake.batch_limit,
            default_batch_limit(),
            "snake 拼写必须落回缺省（证明读侧只认 camelCase，也证明拼错的后果是静默而非报错）"
        );
    }

    /// 阈值是 0-1 归一化占比：45.45% 不得被 0.5 拦，50.5% 必须被拦。
    /// 旧实现直接拿百分数 45.45 与 0.5 比 ⇒ 45% 被当 4500%。
    #[test]
    fn share_threshold_is_unit_interval() {
        assert!(!share_blocks(Some(0.4545), 0.5), "45.45% 不应被 50% 阈值拦");
        assert!(share_blocks(Some(0.505), 0.5), "50.5% 必须被 50% 阈值拦");
        assert!(!share_blocks(None, 0.5), "缺占比信息不拦截");
        assert!(!share_blocks(Some(0.0), 0.5), "0% 不拦截");
    }

    /// 空态不得把所有跳过原因都写成「unexplained 占比超阈值」：
    /// 两种原因分开计数，未分类的显式写「原因未记录」。
    #[test]
    fn empty_selection_message_names_each_skip_reason() {
        let msg = empty_selection_message(
            SelectionStats {
                scanned: 4,
                skipped_by_share: 2,
                skipped_by_period: 1,
                skipped_by_window: 0,
            },
            0.5,
        );
        assert!(msg.contains("2 个所属轮次 unexplained 占比 ≥ 50%"), "实得 {msg}");
        assert!(msg.contains("1 个事件的档位无法解析"), "实得 {msg}");
        assert!(msg.contains("1 个原因未记录"), "实得 {msg}");
        assert!(!msg.contains("全部因"), "不得再把唯一原因写成全部");

        let none = empty_selection_message(SelectionStats::default(), 0.5);
        assert!(none.contains("没有待补分析"), "实得 {none}");
    }
}

#[cfg(test)]
mod asof_diff_tests {
    use super::*;

    /// 纯函数夹具：只有「池快照 / 同档出票 / 别档出票 / 截断 / 失败」五组证据会变，
    /// 其余字段固定，让每条断言只对着一层。
    struct Case<'a> {
        pool: Option<&'a str>,
        same: &'a [String],
        other: &'a [String],
        trimmed: &'a [String],
        failure: Option<&'a str>,
    }

    fn diff(case: Case<'_>) -> (mover_core::MissLayer, serde_json::Value) {
        let degraded: Vec<String> = Vec::new();
        let input = AsOfDiffInput {
            code: "002696",
            period: "mid",
            window_start: "2026-09-04",
            generated_at: "2026-10-24T10:00:00.000",
            live_layer: "not_in_pool",
            pool_snapshot: case.pool,
            same_period_styles: case.same,
            other_periods: case.other,
            trimmed_styles: case.trimmed,
            degraded_styles: &degraded,
            pool_origin: serde_json::json!({"hot": 1, "industry": 2, "fallback": 3, "mover": 4}),
            mover_seed_count: 50,
            failure: case.failure,
        };
        build_asof_diff(&input)
    }

    fn none() -> Vec<String> {
        Vec::new()
    }

    /// 1) 进池、无出票、无截断留痕 ⇒ 兜底桶（策略内部否决不落痕，不可归因）。
    #[test]
    fn in_pool_without_evidence_is_unexplained_and_keeps_pool_facts() {
        let (layer, json) = diff(Case {
            pool: Some(r#"[["002696","百洋股份"]]"#),
            same: &none(),
            other: &none(),
            trimmed: &none(),
            failure: None,
        });
        assert_eq!(layer, mover_core::MissLayer::Unexplained);
        assert_eq!(json["inPool"], serde_json::Value::Bool(true));
        assert_eq!(json["poolState"], "present");
        // 显式声明：本轮回放未含分析定档来源（防止把 not_in_pool 读成 live 也没有）
        assert_eq!(json["analysisTierSourceIncluded"], serde_json::Value::Bool(false));
        assert_eq!(json["moverSeedCount"], 50);
        // live 层是 not_in_pool、as-of 层是 unexplained（入池但无留痕）⇒ **必须标成分歧**：
        // 「池里没有它」与「池里有它但没法归因」是两个不同的诊断。
        assert_eq!(json["divergedFromLive"], serde_json::Value::Bool(true));
    }

    /// 2) 进池 + 有截断留痕 ⇒ l3_scored_out，且 styles 原样落进差异 JSON。
    #[test]
    fn trimmed_style_is_l3_scored_out_with_styles_recorded() {
        let trimmed = vec!["trend".to_string()];
        let (layer, json) = diff(Case {
            pool: Some(r#"[["002696","百洋股份"]]"#),
            same: &none(),
            other: &none(),
            trimmed: &trimmed,
            failure: None,
        });
        assert_eq!(layer, mover_core::MissLayer::L3ScoredOut);
        assert_eq!(json["trimmedStyles"][0], "trend");
        assert_eq!(json["inPool"], serde_json::Value::Bool(true));
    }

    /// 3) 不在池 ⇒ not_in_pool（池快照真实非空）。
    #[test]
    fn absent_from_real_pool_is_not_in_pool() {
        let (layer, json) = diff(Case {
            pool: Some(r#"[["600000","浦发银行"]]"#),
            same: &none(),
            other: &none(),
            trimmed: &none(),
            failure: None,
        });
        assert_eq!(layer, mover_core::MissLayer::NotInPool);
        assert_eq!(json["inPool"], serde_json::Value::Bool(false));
        assert_eq!(json["poolState"], "present");
    }

    /// 4) 池为 `None`（取数缺痕）⇒ unexplained，**不得**落 not_in_pool。
    #[test]
    fn missing_pool_snapshot_is_unexplained_not_not_in_pool() {
        let (layer, json) = diff(Case {
            pool: None,
            same: &none(),
            other: &none(),
            trimmed: &none(),
            failure: None,
        });
        assert_eq!(layer, mover_core::MissLayer::Unexplained);
        assert_eq!(json["inPool"], serde_json::Value::Null);
        assert_eq!(json["poolState"], "absent");
    }

    /// 5) 池为 `"[]"`（真空池）⇒ pool_degraded，与 `None` 严格分列。
    #[test]
    fn empty_pool_is_pool_degraded() {
        for raw in ["[]", "  [] ", ""] {
            let (layer, json) = diff(Case {
                pool: Some(raw),
                same: &none(),
                other: &none(),
                trimmed: &none(),
                failure: None,
            });
            assert_eq!(layer, mover_core::MissLayer::PoolDegraded, "池 {raw:?} 应判降级");
            assert_eq!(json["poolState"], "empty", "池 {raw:?} 状态");
        }
    }

    /// 6) 同档有 as-of 出票 ⇒ recommended（即使池里查不到，也以出票为准）。
    #[test]
    fn same_period_pick_is_recommended() {
        let same = vec!["capital".to_string()];
        let (layer, json) =
            diff(Case { pool: None, same: &same, other: &none(), trimmed: &none(), failure: None });
        assert_eq!(layer, mover_core::MissLayer::Recommended);
        assert_eq!(json["pickedStyles"][0], "capital");
        // live 是 not_in_pool、as-of 是 recommended ⇒ 必须标成分歧（诊断信息）
        assert_eq!(json["divergedFromLive"], serde_json::Value::Bool(true));
    }

    /// 7) 只在别的档出票 ⇒ other_period，并记下是哪些档。
    #[test]
    fn pick_in_other_period_is_other_period() {
        let other = vec!["short".to_string()];
        let (layer, json) = diff(Case {
            pool: Some(r#"[["002696","百洋股份"]]"#),
            same: &none(),
            other: &other,
            trimmed: &none(),
            failure: None,
        });
        assert_eq!(layer, mover_core::MissLayer::OtherPeriod);
        assert_eq!(json["pickedOtherPeriods"][0], "short");
    }

    /// 8) 回放整体失败 ⇒ unexplained + 失败原因落 JSON；池事实仍如实记录。
    #[test]
    fn replay_failure_is_unexplained_with_reason() {
        let (layer, json) = diff(Case {
            pool: None,
            same: &none(),
            other: &none(),
            trimmed: &none(),
            failure: Some("行情与K线数据源均不可用"),
        });
        assert_eq!(layer, mover_core::MissLayer::Unexplained);
        assert_eq!(json["replayFailure"], "行情与K线数据源均不可用");
        assert_eq!(json["poolState"], "absent");
    }

    /// A-7 顺序契约：回放先于分析，差异夹在中间；三步都执行一次。
    #[tokio::test]
    async fn replay_runs_before_diff_and_analysis() {
        use std::sync::Arc;
        let order = Arc::new(tokio::sync::Mutex::new(Vec::<&'static str>::new()));

        let o1 = order.clone();
        let o2 = order.clone();
        let o3 = order.clone();
        let analysis = replay_then_analyze(
            async move {
                o1.lock().await.push("replay");
                Ok::<u8, String>(7)
            },
            move |res| async move {
                assert_eq!(res, Ok(7), "差异必须看到回放结果");
                o2.lock().await.push("diff");
            },
            async move {
                o3.lock().await.push("analyze");
                9u8
            },
        )
        .await;

        assert_eq!(analysis, 9);
        assert_eq!(
            *order.lock().await,
            vec!["replay", "diff", "analyze"],
            "顺序不可换：先分析会把漏检票自己的定档变量喂回候选池"
        );
    }

    /// 汇总纯函数：六桶互斥、`unavailable` 不收「进池未出票」，失败清单与分歧计数可读。
    #[test]
    fn summary_buckets_are_exclusive_and_keep_unavailable_separate() {
        fn row(period: &str, layer: &str, in_pool: Option<bool>) -> EventModel {
            EventModel {
                id: format!("e-{period}-{layer}"),
                run_id: "mrr-t".to_string(),
                stock_code: "002696".to_string(),
                stock_name: "百洋股份".to_string(),
                period: period.to_string(),
                anchor_date: "2026-09-30".to_string(),
                window_start: "2026-09-04".to_string(),
                window_end: "2026-09-30".to_string(),
                window_days: 28,
                cum_gain_pct: 30.0,
                max_daily_pct: 10.0,
                threshold_pct: 30.0,
                market_type: "main_sz".to_string(),
                chain: Some("reco".to_string()),
                miss_layer: "not_in_pool".to_string(),
                picked_any_period: 0,
                scored_out_styles_json: None,
                batch_id: None,
                analysis_id: None,
                reflection_id: None,
                status: "analyzed".to_string(),
                error_detail: None,
                reflow_note: None,
                reflow_pending: None,
                pool_evidence_at: None,
                analysis_signal_at_start: None,
                asof_reco_generated_at: Some("2026-10-24T10:00:00.000".to_string()),
                asof_layer: Some(layer.to_string()),
                asof_diff_json: Some(
                    serde_json::json!({
                        "inPool": in_pool,
                        // 只有「进池未出票」那行带出票风格；其余行不得凭空给 capital 记一次命中。
                        "pickedStyles": if in_pool == Some(true) && layer == "unexplained" {
                            serde_json::json!(["capital"])
                        } else {
                            serde_json::json!([])
                        },
                        // 截断风格只属于 l3_scored_out 行。
                        "trimmedStyles": if layer == "l3_scored_out" {
                            serde_json::json!(["trend"])
                        } else {
                            serde_json::json!([])
                        },
                        "divergedFromLive": true,
                        "replayFailure": null,
                    })
                    .to_string(),
                ),
                created_at_ms: 1,
                updated_at_ms: 1,
            }
        }
        let rows = vec![
            row("mid", "unexplained", Some(true)),
            row("mid", "not_in_pool", Some(false)),
            row("short", "l3_scored_out", Some(true)),
            row("long", "unexplained", None),
        ];
        let (layers_json, summary_json) = aggregate_asof_summary(&rows);
        let layers: serde_json::Value = serde_json::from_str(&layers_json).unwrap();
        assert_eq!(layers["unexplained"]["mid"], 1);
        assert_eq!(layers["unexplained"]["long"], 1);

        let summary: serde_json::Value = serde_json::from_str(&summary_json).unwrap();
        assert_eq!(summary["byPeriod"]["mid"]["inPoolNotPicked"], 1);
        assert_eq!(summary["byPeriod"]["mid"]["notInPool"], 1);
        assert_eq!(summary["byPeriod"]["short"]["inPoolTrimmed"], 1);
        // 池 None 的行落 unavailable，不得被并进 notInPool
        assert_eq!(summary["byPeriod"]["long"]["unavailable"], 1);
        assert_eq!(summary["byPeriod"]["long"]["notInPool"], serde_json::Value::Null);
        assert_eq!(summary["divergedFromLive"], 4);
        assert_eq!(summary["byStyle"]["mid"]["capital"]["picked"], 1);
        assert_eq!(summary["byStyle"]["short"]["trend"]["trimmed"], 1);
    }
}
