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
use axagent_astock_data::AStockClient;
use axagent_astock_data::as_of::{self, AsOfContext, AsOfSource};
use axagent_dao::repo::mover_recall_analysis_batches::{self, BatchRunModel, BatchRunRecord};
use axagent_dao::repo::mover_recall_events::{self, CHAIN_ROWS_PER_EVENT, EventModel};
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
/// 为什么是 10 而不是手动面板的 20：cron 每天跑，配额×频次才是真实日消耗；
/// 且它打的是 as-of 历史行情链（push2 族在本机是速率触发 RST），一轮 10 只已经把
/// 请求量摊开在不同天，而不是把通道打死。面板手动发起仍是 20（`default_batch_limit`）。
pub const CRON_BATCH_LIMIT: usize = 60;

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

/// 一只待补事件与其 as-of 分组内的候选构造输入。
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
        // guard 内：首趟逐只分析 + 失败子集立即重试一趟（共两趟，重试仅一次）。
        let pair = as_of::with_optional_asof(
            Some(ctx),
            run_group_with_retry(
                &db_ref,
                &client_ref,
                &engine_ref,
                &batch_id_ref,
                &window_ref,
                group,
            ),
        )
        .await;

        // guard 外回写事件状态（无 as_of 取数依赖，只是状态落库）
        for (event_id, result) in pair {
            match result {
                Ok(analysis_id) => {
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
    let date_froms = mover_recall_events::load_run_date_froms(db)
        .await
        .map_err(|e| format!("读核查轮次 date_from 失败: {e}"))?;
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
        let m = group.into_iter().next().expect("group 非空");
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
