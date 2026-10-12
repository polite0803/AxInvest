//! 时间旅行批次复盘（运行态内存 + `time_travel_batch_runs` 历史表双写）。
//!
//! 状态存储：
//! - **运行态**：进程内 `BATCH_RUNS`（轮询低延迟，互斥语义见下）；
//! - **历史态**：每次状态变更同步 upsert 到 `time_travel_batch_runs` 表
//!   （实体 `axagent_entities::time_travel_batch_runs`，声明式建表），重启后
//!   `get_time_travel_batch` 从库中还原历史；`list_time_travel_batches` 给任务中心
//!   提供倒序历史。进程异常退出遗留的 `running` 行在新进程首次访问时收敛为
//!   `interrupted`——as_of 封闭世界栈是进程级的，不支持跨进程续跑。
//!
//! 流水线（见 `.trae/documents/time-travel-batch_plan.md`）：
//! 1. 选定历史交易日 `as_of`，进入 [`as_of::with_optional_asof`] 封闭世界；
//! 2. **guard 内**一轮统一回放两条选股链（顶层平级，批次不区分）：
//!    a. 内存荐股（**不写 `reco_picks` 主表**，铁律 B2）；
//!    b. 趋势智选 serenity 原工作流（静默复用，as-of 守卫使其同样不落
//!    `reco_picks`）；两链候选合并去重（同 code 留置信最高一条）后，
//!    经 [`run_analysis_for_candidates`] 逐只 replay 分析
//!    （`analysis_kind='replay'`，分析日 / pending 锚点均为 `as_of`）；
//!    serenity 链 best-effort：失败只降级批次终态，不拖垮荐股链；
//! 3. **退出 guard 后**再做四档反思（铁律 B3）：guard 内「今天」= as_of 会导致
//!    2/5/28/90 交易日档全部判未到期；
//! 4. 到期档（hindsight ≤ 北京今日）立即反思；未到期档留 pending，由既有
//!    18:00 batch-reflection cron 自然补做。
//!
//! 互斥：AppState 上的全局批次锁 + live 流水线时间窗拒绝
//! （23:50-02:10 / 17:50-18:10，北京时间）——as_of 栈是进程级的，
//! 批次既不能与 live 流水线并发，也不能与另一个批次并发。

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use axagent_agent_macro::agent_command;
use axagent_astock_data::AStockClient;
use axagent_astock_data::as_of::{self, AsOfContext, AsOfSource};
use axagent_astock_data::calendar;
use axagent_entities::time_travel_batch_runs;
use axagent_harness::{DatabaseConnection, holding_period::Period};
use axagent_search::vector_store::VectorStore;
use axagent_trajectory::TrajectoryStorage;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::error::ErrorResponse;
use super::error_code::time_travel as ec;
use super::pool_scan::{
    PoolCandidate, PoolCandidateResult, dedup_explicit_candidates, run_analysis_for_candidates,
};
use super::stock_workflow::{ReflectionFilter, run_batch_reflection_inner};
use crate::app_state::AppState;

// ── 批次状态（进程内单例；MVP 不建批次表，重启即清空）─────────────────────────

/// batch_id → 批次运行状态。锁用 tokio Mutex（临界区内无跨 await 持有的 std guard，
/// 且全仓禁用 std 同步锁）。
static BATCH_RUNS: LazyLock<Arc<Mutex<HashMap<String, BatchRunState>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

// 批次终态 / 阶段常量
const STATUS_RUNNING: &str = "running";
const STATUS_COMPLETED: &str = "completed";
const STATUS_PARTIAL: &str = "partial";
const STATUS_FAILED: &str = "failed";
/// 进程重启后，上次运行中未终态的批次被收敛为此状态（不可续跑，仅留痕）
const STATUS_INTERRUPTED: &str = "interrupted";
const PHASE_ANALYSIS: &str = "analysis";
const PHASE_REFLECTION: &str = "reflection";

/// 发起时间旅行批次的请求（camelCase 与前端契约对齐）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeTravelBatchRequest {
    /// 固定锚点日 `YYYY-MM-DD`；与 `trading_days_ago` 二选一（date 优先）
    #[serde(default)]
    pub date: Option<String>,
    /// 相对北京今日回看的交易日数（90 = 90 个交易日前）
    #[serde(default)]
    pub trading_days_ago: Option<i32>,
    /// 参与的周期档名（`ultra_short`/`short`/`mid`/`long`，兼容 `ultrashort`）；
    /// 空 = 全四档
    #[serde(default)]
    pub periods: Vec<String>,
    /// 荐股最低置信度（0-100）
    #[serde(default = "default_min_confidence")]
    pub min_confidence: u8,
    /// 每档 top-N（0 = 荐股侧不限）
    #[serde(default = "default_top_n")]
    pub top_n: usize,
    /// 是否在 guard 外立即反思已到期档（false = 只分析，反思全部留给 cron）
    #[serde(default = "default_true")]
    pub reflect: bool,
}

fn default_min_confidence() -> u8 {
    50
}
fn default_top_n() -> usize {
    20
}
fn default_true() -> bool {
    true
}

/// 批次运行状态（序列化给前端轮询；同时可从历史表 JSON 反序列化还原）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRunState {
    pub batch_id: String,
    pub as_of: String,
    pub trading_days_ago: Option<i32>,
    /// running / completed / partial / failed / interrupted
    pub status: String,
    /// analysis / reflection
    pub phase: String,
    /// 人话补充（如「当日荐股无候选」）
    pub message: Option<String>,
    pub total: usize,
    pub ok: usize,
    pub fail: usize,
    /// 智能荐股链产出的候选行数（合并去重前；四档各一排）
    pub reco_picks: usize,
    /// 趋势智选链产出的逐档候选行数（合并去重前；mid/long 两档展开）
    pub serenity_picks: usize,
    /// 趋势智选工作流失败原因（best-effort：荐股链成功时批次仍继续，终态降级 partial）
    pub serenity_error: Option<String>,
    pub analyses: Vec<PoolCandidateResult>,
    pub horizons: Vec<HorizonSummary>,
    /// batch_reflection_inner 返回的原始 JSON 摘要（调试/明细用）
    pub reflection_summary: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    /// 发起请求快照：仅用于落库 request_json，不向前端输出、从库还原时为 None
    #[serde(skip)]
    pub request: Option<TimeTravelBatchRequest>,
}

/// 单个复盘档的批次级汇总。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonSummary {
    /// ultra_short / short / mid / long
    pub period: String,
    pub holding_days: u32,
    /// 该档名义 hindsight（交易日外推；同档各行同日）
    pub hindsight_date: String,
    /// hindsight 是否已到期（≤ 北京今日）
    pub due: bool,
    pub total: usize,
    pub completed: usize,
    pub pending: usize,
    pub failed: usize,
}

impl BatchRunState {
    fn new(
        batch_id: String,
        as_of: String,
        trading_days_ago: Option<i32>,
        request: TimeTravelBatchRequest,
    ) -> Self {
        let now = now_ms();
        Self {
            batch_id,
            as_of,
            trading_days_ago,
            status: STATUS_RUNNING.to_string(),
            phase: PHASE_ANALYSIS.to_string(),
            message: None,
            total: 0,
            ok: 0,
            fail: 0,
            reco_picks: 0,
            serenity_picks: 0,
            serenity_error: None,
            analyses: Vec::new(),
            horizons: Vec::new(),
            reflection_summary: None,
            error_code: None,
            error_detail: None,
            created_at_ms: now,
            updated_at_ms: now,
            finished_at_ms: None,
            request: Some(request),
        }
    }
}

/// 更新内存态并尽力同步落库（落库失败只告警，不阻断批次流水线；
/// 内存态是运行轮询的权威，库是重启后的历史权威）。
async fn save_state(db: &DatabaseConnection, mut s: BatchRunState) {
    s.updated_at_ms = now_ms();
    if let Err(e) = upsert_batch_run(db, &s).await {
        tracing::warn!("[time-travel-batch] 批次 {} 状态落库失败（不影响运行）: {e}", s.batch_id);
    }
    BATCH_RUNS.lock().await.insert(s.batch_id.clone(), s);
}

/// 全量 upsert 一行批次历史（与 save_state 调用同阶：创建 / 分析完成 /
/// 反思阶段切换 / 反思汇总 / 终态，不做逐只分析的高频写）。
async fn upsert_batch_run(
    db: &DatabaseConnection,
    s: &BatchRunState,
) -> Result<(), sea_orm::DbErr> {
    let request_json = s.request.as_ref().and_then(|r| serde_json::to_string(r).ok());
    let analyses_json = serde_json::to_string(&s.analyses).ok();
    let horizons_json = serde_json::to_string(&s.horizons).ok();

    let am = time_travel_batch_runs::ActiveModel {
        id: Set(s.batch_id.clone()),
        as_date: Set(s.as_of.clone()),
        trading_days_ago: Set(s.trading_days_ago),
        status: Set(s.status.clone()),
        phase: Set(s.phase.clone()),
        message: Set(s.message.clone()),
        total: Set(s.total as i32),
        ok: Set(s.ok as i32),
        fail: Set(s.fail as i32),
        reco_picks: Set(s.reco_picks as i32),
        serenity_picks: Set(s.serenity_picks as i32),
        serenity_error: Set(s.serenity_error.clone()),
        request_json: Set(request_json),
        analyses_json: Set(analyses_json),
        horizons_json: Set(horizons_json),
        reflection_summary: Set(s.reflection_summary.clone()),
        error_code: Set(s.error_code.clone()),
        error_detail: Set(s.error_detail.clone()),
        created_at_ms: Set(s.created_at_ms),
        updated_at_ms: Set(s.updated_at_ms),
        finished_at_ms: Set(s.finished_at_ms),
    };

    time_travel_batch_runs::Entity::insert(am)
        .on_conflict(
            // 主键冲突（同批次后续状态更新）时覆盖除 id / 创建时间 / 原始请求外的列。
            // request_json 首插后永不更新：execute_batch 中途若从库还原（request 为
            // #[serde(skip)] 的 None）再 upsert，不能让 None 覆盖原始请求。
            OnConflict::column(time_travel_batch_runs::Column::Id)
                .update_columns([
                    time_travel_batch_runs::Column::AsDate,
                    time_travel_batch_runs::Column::TradingDaysAgo,
                    time_travel_batch_runs::Column::Status,
                    time_travel_batch_runs::Column::Phase,
                    time_travel_batch_runs::Column::Message,
                    time_travel_batch_runs::Column::Total,
                    time_travel_batch_runs::Column::Ok,
                    time_travel_batch_runs::Column::Fail,
                    time_travel_batch_runs::Column::RecoPicks,
                    time_travel_batch_runs::Column::SerenityPicks,
                    time_travel_batch_runs::Column::SerenityError,
                    time_travel_batch_runs::Column::AnalysesJson,
                    time_travel_batch_runs::Column::HorizonsJson,
                    time_travel_batch_runs::Column::ReflectionSummary,
                    time_travel_batch_runs::Column::ErrorCode,
                    time_travel_batch_runs::Column::ErrorDetail,
                    time_travel_batch_runs::Column::UpdatedAtMs,
                    time_travel_batch_runs::Column::FinishedAtMs,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// 进程首次访问时收敛上次进程遗留的 running 批次。
///
/// as_of 封闭世界栈是进程级的（见模块头），重启后旧批次不可能续跑，
/// 库内 `running` 只会是异常退出的僵尸。本函数每进程只执行一次：
/// 首次成功发起批次之前必先经过这里（run 命令先 reap 再 insert），
/// 因此不会误伤当前进程的存活批次。
static STALE_REAP_DONE: AtomicBool = AtomicBool::new(false);

async fn ensure_stale_reaped(db: &DatabaseConnection) {
    if STALE_REAP_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    match mark_stale_interrupted(db).await {
        Ok(0) => {},
        Ok(n) => tracing::info!(
            "[time-travel-batch] 启动收敛：{n} 个遗留 running 批次标记为 interrupted"
        ),
        Err(e) => tracing::warn!("[time-travel-batch] 遗留批次收敛失败: {e}"),
    }
}

/// 把库内所有 running 批次标记为 interrupted，返回受影响行数。
async fn mark_stale_interrupted(db: &DatabaseConnection) -> Result<u64, sea_orm::DbErr> {
    let now = now_ms();
    let r = time_travel_batch_runs::Entity::update_many()
        .col_expr(time_travel_batch_runs::Column::Status, Expr::value(STATUS_INTERRUPTED))
        .col_expr(
            time_travel_batch_runs::Column::Message,
            Expr::value("应用重启：批次运行中进程退出，as-of 封闭世界无法跨进程续跑"),
        )
        .col_expr(time_travel_batch_runs::Column::UpdatedAtMs, Expr::value(now))
        .col_expr(time_travel_batch_runs::Column::FinishedAtMs, Expr::value(now))
        .filter(time_travel_batch_runs::Column::Status.eq(STATUS_RUNNING))
        .exec(db)
        .await?;
    Ok(r.rows_affected)
}

/// 历史表行还原为运行状态。`summary_only=true` 时不反序列化大 JSON 明细
/// （列表场景；analyses/horizons/reflection_summary 留空，计数列仍完整）。
fn state_from_model(m: time_travel_batch_runs::Model, summary_only: bool) -> BatchRunState {
    let (analyses, horizons, reflection_summary) = if summary_only {
        (Vec::new(), Vec::new(), None)
    } else {
        let analyses = m
            .analyses_json
            .and_then(|j| serde_json::from_str::<Vec<PoolCandidateResult>>(&j).ok())
            .unwrap_or_default();
        let horizons = m
            .horizons_json
            .and_then(|j| serde_json::from_str::<Vec<HorizonSummary>>(&j).ok())
            .unwrap_or_default();
        (analyses, horizons, m.reflection_summary)
    };
    BatchRunState {
        batch_id: m.id,
        as_of: m.as_date,
        trading_days_ago: m.trading_days_ago,
        status: m.status,
        phase: m.phase,
        message: m.message,
        total: m.total.max(0) as usize,
        ok: m.ok.max(0) as usize,
        fail: m.fail.max(0) as usize,
        reco_picks: m.reco_picks.max(0) as usize,
        serenity_picks: m.serenity_picks.max(0) as usize,
        serenity_error: m.serenity_error,
        analyses,
        horizons,
        reflection_summary,
        error_code: m.error_code,
        error_detail: m.error_detail,
        created_at_ms: m.created_at_ms,
        updated_at_ms: m.updated_at_ms,
        finished_at_ms: m.finished_at_ms,
        request: None,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 北京当前日期（UTC+8 固定偏移，与 reflection/calendar 既有口径一致）。
fn beijing_today() -> chrono::NaiveDate {
    (chrono::Utc::now() + chrono::Duration::hours(8)).date_naive()
}

/// 是否落在 live 流水线时间窗（23:50-02:10 / 17:50-18:10，北京时间）。
///
/// 边界与四个 live cron 的启动窗对齐：窗内启动批次，全局 as_of 栈可能串扰正在跑的
/// live 任务（荐股 / pool-scan / batch-reflection），故一律拒绝。
fn in_live_window() -> bool {
    let bj = chrono::Utc::now() + chrono::Duration::hours(8);
    let hm = bj.format("%H:%M").to_string();
    let (h, m): (u32, u32) = {
        let mut it = hm.split(':').map(|x| x.parse::<u32>().unwrap_or(0));
        (it.next().unwrap_or(0), it.next().unwrap_or(0))
    };
    let mins = h * 60 + m;
    // 23:50-24:00 或 00:00-02:10
    mins >= 23 * 60 + 50 || mins <= 2 * 60 + 10 || (17 * 60 + 50..=18 * 60 + 10).contains(&mins)
}

// ── Tauri 命令 ────────────────────────────────────────────────────────────────

/// 发起一次时间旅行批次复盘；立即返回 batch_id，执行在后台进行，
/// 用 [`get_time_travel_batch`] 轮询进度与结果。
#[agent_command(domain = time_travel, safety = Caution, call_mode = StateInput, description = "发起时间旅行批次复盘")]
#[tauri::command]
pub async fn run_time_travel_batch(
    state: State<'_, AppState>,
    request: TimeTravelBatchRequest,
) -> Result<String, String> {
    // 批次恒同时回放「智能荐股 + 趋势智选」两条平级选股链（请求体无开关）。

    // 1. 周期档解析（空 = 全四档）
    let periods: Vec<Period> = if request.periods.is_empty() {
        Period::ALL.to_vec()
    } else {
        let mut parsed = Vec::with_capacity(request.periods.len());
        for raw in &request.periods {
            match raw.parse::<Period>() {
                Ok(p) => parsed.push(p),
                Err(e) => {
                    return Err(ErrorResponse::err_with_detail(
                        ec::INVALID_PARAMS,
                        format!("周期档无法识别 {raw:?}: {e}"),
                    ));
                },
            }
        }
        parsed
    };

    // 2. as_of 锚点解析（date 优先；trading_days_ago 从北京今日按交易日回退）
    let today = beijing_today();
    let (as_of, trading_days_ago) = match (request.date.as_deref(), request.trading_days_ago) {
        (Some(d), _) => {
            let nd = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|e| {
                ErrorResponse::err_with_detail(ec::INVALID_PARAMS, format!("date 解析失败: {e}"))
            })?;
            // 回显时估算交易日距离（失败不阻断）
            let ago = calendar::trading_days_between(nd, today);
            (nd, Some(ago.max(0)))
        },
        (None, Some(n)) if n > 0 => (calendar::nth_trading_day(today, -n), Some(n)),
        (None, Some(_)) => {
            return Err(ErrorResponse::err_with_detail(
                ec::INVALID_PARAMS,
                "tradingDaysAgo 必须为正整数",
            ));
        },
        (None, None) => {
            return Err(ErrorResponse::err_with_detail(
                ec::INVALID_PARAMS,
                "必须提供 date 或 tradingDaysAgo 之一",
            ));
        },
    };

    // 锚点必须是交易日（周末 / 假日不能作为回放交易日）
    if !calendar::is_trading_day(&as_of) {
        return Err(ErrorResponse::err_with_detail(
            ec::ASOF_INVALID,
            format!("{as_of} 不是 A 股交易日，请选择最近的交易日"),
        ));
    }
    // AsOfContext::new 内含「不得晚于今天」的契约校验
    let ctx = AsOfContext::new(as_of, AsOfSource::UserReplay)
        .map_err(|e| ErrorResponse::err_with_detail(ec::ASOF_INVALID, e.to_string()))?;

    // 3. live 时间窗拒绝（先于抢锁，给出可操作的拒绝原因）
    if in_live_window() {
        return Err(ErrorResponse::err_with_detail(
            ec::LIVE_WINDOW,
            "当前处于 live 流水线时间窗（23:50-02:10 / 17:50-18:10），请避开后再发起时间旅行批次",
        ));
    }

    // 4. 全局批次互斥（try_lock：已在跑直接拒绝，不排队）
    let permit = state
        .time_travel_batch_lock
        .clone()
        .try_lock_owned()
        .map_err(|_| ErrorResponse::err(ec::ALREADY_RUNNING))?;

    let db = state.harness.db().clone();
    // 新进程首次发起批次前，收敛上次进程异常退出遗留的 running 行
    // （必须在本行 insert 之前，避免把本批次误判为僵尸）。
    ensure_stale_reaped(&db).await;

    // 5. 建批次状态并后台执行（内存 + 历史表双写）
    let batch_id = uuid::Uuid::new_v4().to_string();
    save_state(
        &db,
        BatchRunState::new(
            batch_id.clone(),
            as_of.format("%Y-%m-%d").to_string(),
            trading_days_ago,
            request.clone(),
        ),
    )
    .await;
    // spawn 会 move batch_id，返回给调用方的 id 单独留一份
    let batch_id_to_return = batch_id.clone();

    let client = state.astock_client.clone();
    let engine = state.work_engine.clone();
    let vector_store = state.vector_store.clone();
    let master_key = state.harness.master_key_owned();
    let trajectory_storage = state.trajectory_storage.clone();
    let reflect = request.reflect;
    let min_confidence = request.min_confidence;
    let top_n = request.top_n;

    tauri::async_runtime::spawn(async move {
        execute_batch(
            batch_id,
            ctx,
            periods,
            min_confidence,
            top_n,
            reflect,
            db,
            client,
            engine,
            vector_store,
            master_key,
            trajectory_storage,
            permit,
        )
        .await;
        tracing::info!("[time-travel-batch] 批次结束");
    });

    Ok(batch_id_to_return)
}

/// 查询批次运行状态：优先内存运行态；重启后内存已失则从历史表还原完整明细。
/// 批次不存在返回 null。
#[agent_command(domain = time_travel, safety = Safe, call_mode = StateInput, description = "查询时间旅行批次状态")]
#[tauri::command]
pub async fn get_time_travel_batch(
    state: State<'_, AppState>,
    batch_id: String,
) -> Result<Option<BatchRunState>, String> {
    let db = state.harness.db();
    ensure_stale_reaped(db).await;

    if let Some(s) = BATCH_RUNS.lock().await.get(&batch_id).cloned() {
        return Ok(Some(s));
    }

    let model =
        time_travel_batch_runs::Entity::find_by_id(&batch_id).one(db).await.map_err(|e| {
            ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("查询批次历史失败: {e}"))
        })?;
    Ok(model.map(|m| state_from_model(m, false)))
}

/// 查询批次复盘历史（任务中心用）：按创建时间倒序，默认 50 条；
/// 摘要不含 analyses / horizons 明细，明细走 [`get_time_travel_batch`]。
#[agent_command(domain = time_travel, safety = Safe, call_mode = StateInput, description = "查询时间旅行批次历史")]
#[tauri::command]
pub async fn list_time_travel_batches(
    state: State<'_, AppState>,
    limit: Option<u64>,
    status_filter: Option<String>,
    as_of: Option<String>,
) -> Result<Vec<BatchRunState>, String> {
    let db = state.harness.db();
    ensure_stale_reaped(db).await;

    let mut query = time_travel_batch_runs::Entity::find()
        .order_by_desc(time_travel_batch_runs::Column::CreatedAtMs);
    if let Some(status) = status_filter.filter(|s| !s.is_empty()) {
        query = query.filter(time_travel_batch_runs::Column::Status.eq(status));
    }
    if let Some(as_of) = as_of.filter(|s| !s.is_empty()) {
        query = query.filter(time_travel_batch_runs::Column::AsDate.eq(as_of));
    }
    let rows = query.limit(limit.unwrap_or(50).min(200)).all(db).await.map_err(|e| {
        ErrorResponse::err_with_detail(ec::QUERY_FAILED, format!("查询批次历史失败: {e}"))
    })?;

    Ok(rows.into_iter().map(|m| state_from_model(m, true)).collect())
}

// ── 后台执行体 ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn execute_batch(
    batch_id: String,
    ctx: AsOfContext,
    periods: Vec<Period>,
    min_confidence: u8,
    top_n: usize,
    reflect: bool,
    db: DatabaseConnection,
    client: Arc<AStockClient>,
    engine: Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    vector_store: Arc<VectorStore>,
    master_key: [u8; 32],
    trajectory_storage: Arc<TrajectoryStorage>,
    // 仅用于把全局批次锁持有到批次结束：drop 即释放。
    _permit: OwnedMutexGuard<()>,
) {
    let as_of_str = ctx.as_of_date.format("%Y-%m-%d").to_string();
    // run 命令在 spawn 前已把初始态写入内存；缺失说明状态异常，不再兜底构造
    // （new 需要完整请求参数），直接放弃本轮后台执行。
    let mut s = match BATCH_RUNS.lock().await.get(&batch_id).cloned() {
        Some(s) => s,
        None => {
            tracing::error!(
                "[time-travel-batch] {batch_id} 内存运行态缺失，放弃后台执行（历史表保留 interrupted 由下次启动收敛）"
            );
            return;
        },
    };

    // ── 阶段 1-3：guard 内两链统一回放 → 合并去重 → 逐只 replay 分析 ───────────
    // 整个 pipeline future 在 with_optional_asof 内执行；future 返回即 guard drop，
    // 之后阶段 4 的反思天然落在 guard 外（铁律 B3）。
    let pipeline = async {
        // 链 A：内存荐股（与荐股 cron 共用模板变量加载，缺陷 D2：两条入口必须同一套权重/阈值）。
        let vars = match axagent_analysis_engine::recommender::reco_loop::load_reco_served_vars_db(
            &db,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    "[time-travel-batch] {as_of_str} 读取荐股模板变量失败 ({e})，按硬编码默认参数执行"
                );
                Vec::new()
            },
        };
        let prior = crate::commands::stock_analysis::reco_horizon_prior(&db, &vars).await;
        let scan = axagent_analysis_engine::recommender::run_recommendation_scan(
            client.clone(),
            &periods,
            &vars,
            min_confidence,
            top_n,
            prior,
        )
        .await;

        // picks 仅在内存映射为候选队列 —— 绝不调 persist_reco_picks（铁律 B2：
        // 回放荐股不写 reco_picks 主表，避免污染 live 候选池）。
        let reco_candidates: Vec<PoolCandidate> = scan
            .picks
            .iter()
            .map(|p| PoolCandidate {
                stock_code: p.stock_code.clone(),
                stock_name: p.stock_name.clone(),
                period: p.period,
                confidence: p.confidence as i32,
            })
            .collect();
        let reco_count = reco_candidates.len();
        tracing::info!(
            "[time-travel-batch] {as_of_str} 链A 内存荐股 {reco_count} 只候选（不落 reco_picks）"
        );

        // 链 B：趋势智选 serenity 原工作流（静默 app=None；内部工作流节点自带同锚点
        // as-of scope，与本 guard 嵌套安全）。as_of_ctx=Some 时其 reco_picks 落库分支
        // 自带跳过守卫，回放不落库。best-effort：失败不阻断批次，候选只用荐股链。
        let (serenity_candidates, serenity_error) =
            match serenity_replay(&db, &client, &engine, ctx).await {
                Ok(v) => (v, None),
                Err(e) => {
                    tracing::warn!(
                        "[time-travel-batch] {as_of_str} 链B 趋势智选回放失败，降级为仅荐股链: {e}"
                    );
                    (Vec::new(), Some(e))
                },
            };
        let serenity_count = serenity_candidates.len();
        tracing::info!(
            "[time-travel-batch] {as_of_str} 链B 趋势智选 {serenity_count} 只逐档候选（不落 reco_picks）"
        );

        // 两链合并去重：同 code 多条（四档 + mid/long、两链撞车）只留置信最高一条，
        // 口径与 live pool-scan 同一个纯函数（dedup_explicit_candidates）。
        let mut merged = reco_candidates;
        merged.extend(serenity_candidates);
        let candidates = dedup_explicit_candidates(merged);
        let merged_count = candidates.len();
        tracing::info!(
            "[time-travel-batch] {as_of_str} 两链去重后 {merged_count} 只进入逐只 replay 分析"
        );

        let outcome = run_analysis_for_candidates(&db, &client, &engine, candidates).await;
        (reco_count, serenity_count, serenity_error, outcome)
    };

    let (reco_count, serenity_count, serenity_error, outcome) =
        as_of::with_optional_asof(Some(ctx), pipeline).await;

    // 成功分析的 analysis_id 集合 —— 阶段 4 反思范围严格收口于此
    let analysis_ids: Vec<String> =
        outcome.results.iter().filter_map(|r| r.analysis_id.clone()).collect();

    s.total = outcome.candidates;
    s.ok = outcome.ok;
    s.fail = outcome.fail;
    s.reco_picks = reco_count;
    s.serenity_picks = serenity_count;
    s.serenity_error = serenity_error.clone();
    s.analyses = outcome.results;
    if reco_count == 0 && serenity_count == 0 {
        s.message =
            Some(format!("{as_of_str} 荐股与趋势智选均无候选（可能当日数据缺失或阈值过高）"));
    } else if s.total == 0 {
        s.message = Some("候选队列执行异常：0 只进入分析".to_string());
    } else {
        // 两链候选数 + 去重后入队数，让用户看到平级两链各自的产出与撞车规模。
        let mut msg = format!(
            "{as_of_str} 荐股 {reco_count} + 趋势智选 {serenity_count} 只候选，去重后 {total} 只进入分析",
            total = s.total
        );
        if serenity_error.is_some() {
            msg.push_str("；趋势智选回放失败已降级（仅计荐股链候选）");
        }
        s.message = Some(msg);
    }
    save_state(&db, s.clone()).await;

    // ── 阶段 4：guard 外四档反思 ─────────────────────────────────────────────
    // 此时 as_of 栈已退出；run_reflection_workflow 内部会按各行 hindsight_date
    // 自行包 AsOfContext（hindsight 锚点），与本批次互不冲突。
    let mut reflection_error: Option<String> = None;
    if reflect {
        if analysis_ids.is_empty() {
            tracing::info!("[time-travel-batch] {batch_id} 无成功分析，跳过反思阶段");
        } else {
            s.phase = PHASE_REFLECTION.to_string();
            save_state(&db, s.clone()).await;

            // 反思前快照（主要为拿到各档 due/pending 分布）
            s.horizons =
                summarize_horizons(&db, &analysis_ids, beijing_today()).await.unwrap_or_default();
            save_state(&db, s.clone()).await;

            // 只反思「本批 + 已到期」的 pending 行；未到期的留给 18:00 cron。
            // 预算给到 分析数×4+8（每分析最多四档行），保证本批到期行一轮跑完。
            let due_filter = ReflectionFilter {
                period: None,
                due_only: true,
                min_confidence: None,
                depth_override: None,
            };
            let budget = (analysis_ids.len() * 4 + 8) as u32;
            match run_batch_reflection_inner(
                &db,
                &client,
                &engine,
                &vector_store,
                &master_key,
                Some(budget),
                Some(&trajectory_storage),
                Some(&due_filter),
                Some(&analysis_ids),
            )
            .await
            {
                Ok(summary) => {
                    s.reflection_summary = Some(summary.to_string());
                },
                Err(e) => {
                    tracing::warn!("[time-travel-batch] {batch_id} guard 外批次反思失败: {e}");
                    reflection_error = Some(e);
                },
            }

            // 反思后刷新各档终态计数
            s.horizons =
                summarize_horizons(&db, &analysis_ids, beijing_today()).await.unwrap_or_default();
        }
    }

    // ── 终态汇总 ─────────────────────────────────────────────────────────────
    let any_horizon_failed = s.horizons.iter().any(|h| h.failed > 0);
    s.status = if s.total > 0 && s.ok == 0 {
        // 全部个股分析失败 = 批次硬失败
        s.error_code = Some(ec::EXEC_FAILED.to_string());
        s.error_detail = Some("全部候选分析失败".to_string());
        STATUS_FAILED
    } else if s.fail > 0
        || any_horizon_failed
        || reflection_error.is_some()
        || serenity_error.is_some()
    {
        // 趋势智选链失败 / 部分个股分析失败 / 反思失败均为降级成功（partial）
        if let Some(e) = reflection_error {
            s.error_detail = Some(format!("批次反思失败: {e}"));
        } else if let Some(e) = serenity_error {
            // 仅 serenity 链失败时给出详情；反思失败优先展示反思详情
            s.error_detail = Some(format!("趋势智选回放失败（荐股链已正常完成）: {e}"));
        }
        STATUS_PARTIAL
    } else {
        STATUS_COMPLETED
    }
    .to_string();
    s.finished_at_ms = Some(now_ms());
    save_state(&db, s).await;
}

/// 批次专用：以静默方式（无 AppHandle、不发 IPC 事件）按 as-of 锚点回放一轮
/// 趋势智选工作流，并把逐档候选映射为批次候选队列。
///
/// 复用 `run_serenity_screening_inner`（与手动 `run_serenity_screening` 命令同一实现）：
/// inner 内部工作流节点自带同锚点 as-of scope（与批次外层 guard 嵌套安全），
/// 且 `as_of_ctx=Some` 时自动跳过 reco_picks 落库与全局 serenity 种子同步 ——
/// 回放不污染 live 候选池（铁律 B2）。
async fn serenity_replay(
    db: &DatabaseConnection,
    client: &AStockClient,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    ctx: AsOfContext,
) -> Result<Vec<PoolCandidate>, String> {
    use super::stock_workflow::serenity::{
        SERENITY_TEMPLATE_IDS, SerenityScreeningArgs, run_serenity_screening_inner,
    };
    let value = run_serenity_screening_inner(SerenityScreeningArgs {
        db,
        client,
        engine: engine.clone(),
        app: None,
        as_of_ctx: Some(ctx),
        themes: None,
        run_id: None,
        template_id: SERENITY_TEMPLATE_IDS[0].to_string(),
    })
    .await?;
    Ok(serenity_candidates_from_json(&value))
}

/// 把 `run_serenity_screening_inner` 返回 JSON 的 `candidates`（mid/long 逐档展开）
/// 映射为 [`PoolCandidate`]。
///
/// 注意键名是**混合形态**：候选本体来自 LLM 工作流，`stock_code`/`stock_name`/
/// `confidence` 为 snake_case；逐档覆盖字段（`period`/`winRate` 等）为 camelCase。
/// 置信度优先取 `winRate`（逐档折算胜率，与 reco_picks.confidence 同量纲，
/// 供两链合并去重比较）；period 缺失/无法识别的行跳过 —— 空展开回退分支
/// （退回无档位原始数组）不该进入逐只分析。
fn serenity_candidates_from_json(value: &serde_json::Value) -> Vec<PoolCandidate> {
    let Some(arr) = value.get("candidates").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for c in arr {
        let Some(code) =
            c.get("stock_code").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(period) =
            c.get("period").and_then(|v| v.as_str()).and_then(|p| p.parse::<Period>().ok())
        else {
            tracing::warn!("[time-travel-batch] 趋势智选候选 {code} 缺周期档/档名无法识别，跳过");
            continue;
        };
        let name = c.get("stock_name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        // winRate = 该档折算胜率（与落库 confidence 同量纲）；缺失时退回原始候选评分。
        let confidence =
            c.get("winRate").or_else(|| c.get("confidence")).and_then(|v| v.as_i64()).unwrap_or(50)
                as i32;
        out.push(PoolCandidate {
            stock_code: code.to_string(),
            stock_name: name,
            period,
            confidence,
        });
    }
    out
}

/// 按四档汇总本批 pending/completed/failed 行（同档各行 hindsight 相同，取首行）。
async fn summarize_horizons(
    db: &DatabaseConnection,
    analysis_ids: &[String],
    today: chrono::NaiveDate,
) -> Result<Vec<HorizonSummary>, String> {
    // 反思行查询下沉 dao（分层护栏：命令层不得直连 sea_orm / entities）
    let rows =
        axagent_dao::repo::stock_lesson_queries::fetch_by_original_analysis_ids(db, analysis_ids)
            .await
            .map_err(|e| format!("查询批次反思行失败: {e}"))?;

    // horizon 字符串 → 行集合（本批 pending 行建点即写 horizon，None 老行不计入四档汇总）
    let mut groups: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for r in &rows {
        if let Some(h) = r.horizon.as_deref() {
            groups.entry(h.to_string()).or_default().push(r);
        }
    }

    let mut out = Vec::new();
    for period in Period::ALL {
        let key = period.as_str();
        // horizon 列历史写法 ultrashort 也应认到 UltraShort（parse 已兼容）
        let grp = groups
            .iter()
            .find(|(k, _)| k.parse::<Period>().is_ok_and(|p| p.as_str() == period.as_str()));
        let Some((_, grp)) = grp else {
            continue;
        };

        let hindsight_date = grp.first().map(|r| r.hindsight_date.clone()).unwrap_or_default();
        let due = chrono::NaiveDate::parse_from_str(&hindsight_date, "%Y-%m-%d")
            .is_ok_and(|h| h <= today);

        let mut completed = 0usize;
        let mut pending = 0usize;
        let mut failed = 0usize;
        for r in grp {
            match r.status.as_str() {
                "completed" => completed += 1,
                "pending" | "running" => pending += 1,
                _ => failed += 1,
            }
        }

        out.push(HorizonSummary {
            period: key.to_string(),
            holding_days: period.default_holding_days(),
            hindsight_date,
            due,
            total: grp.len(),
            completed,
            pending,
            failed,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// serenity inner 返回 JSON 的键名是混合形态（候选本体 snake_case +
    /// 逐档覆盖 camelCase）；置信度必须优先取 winRate（折算胜率，与 reco_picks
    /// 落库口径同量纲），无 winRate 才回退原始 confidence；无档位 / 空 code 丢弃。
    #[test]
    fn serenity_json_maps_snake_case_body_prefers_winrate() {
        let value = serde_json::json!({
            "status": "completed",
            "candidates": [
                { "stock_code": "600001", "stock_name": "甲", "confidence": 91, "period": "mid", "winRate": 72 },
                { "stock_code": "600002", "stock_name": "乙", "confidence": 80, "period": "long" },
                // 空展开回退分支：无逐档 period → 不进逐只分析
                { "stock_code": "600003", "stock_name": "丙", "confidence": 80 },
                // 残缺 code 丢弃
                { "stock_code": "  ", "stock_name": "丁", "period": "mid" },
            ]
        });
        let rows = serenity_candidates_from_json(&value);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].stock_code, "600001");
        assert_eq!(rows[0].stock_name, "甲");
        assert_eq!(rows[0].period, Period::Mid);
        assert_eq!(rows[0].confidence, 72, "winRate 优先于原始候选评分");
        assert_eq!(rows[1].stock_code, "600002");
        assert_eq!(rows[1].period, Period::Long);
        assert_eq!(rows[1].confidence, 80, "无 winRate 回退原始 confidence");
    }

    /// 返回体没有 candidates 段（异常形态）→ 空队列而非 panic
    #[test]
    fn serenity_json_missing_candidates_yields_empty() {
        let value = serde_json::json!({ "status": "completed" });
        assert!(serenity_candidates_from_json(&value).is_empty());
    }

    // ── 持久化（time_travel_batch_runs）─────────────────────────────────────────

    /// 构造带两条分析结果 / 一个档汇总的测试状态（created_at 显式可控，便于验倒序）。
    fn make_persist_state(
        batch_id: &str,
        as_of: &str,
        status: &str,
        created_at_ms: i64,
    ) -> BatchRunState {
        let mut s = BatchRunState::new(
            batch_id.to_string(),
            as_of.to_string(),
            Some(9),
            TimeTravelBatchRequest {
                date: Some(as_of.to_string()),
                trading_days_ago: None,
                periods: vec![],
                min_confidence: 55,
                top_n: 10,
                reflect: false,
            },
        );
        s.status = status.to_string();
        s.created_at_ms = created_at_ms;
        s.updated_at_ms = created_at_ms;
        if status != STATUS_RUNNING {
            s.finished_at_ms = Some(created_at_ms + 5_000);
        }
        s.total = 2;
        s.ok = 1;
        s.fail = 1;
        s.reco_picks = 3;
        s.serenity_picks = 2;
        s.analyses = vec![
            PoolCandidateResult {
                stock_code: "600001".to_string(),
                stock_name: "甲".to_string(),
                period: "mid".to_string(),
                confidence: 80,
                analysis_id: Some("a-1".to_string()),
                error: None,
            },
            PoolCandidateResult {
                stock_code: "600002".to_string(),
                stock_name: "乙".to_string(),
                period: "long".to_string(),
                confidence: 70,
                analysis_id: None,
                error: Some("数据缺失".to_string()),
            },
        ];
        s.horizons = vec![HorizonSummary {
            period: "mid".to_string(),
            holding_days: 14,
            hindsight_date: "2026-05-15".to_string(),
            due: true,
            total: 1,
            completed: 1,
            pending: 0,
            failed: 0,
        }];
        s
    }

    async fn test_db() -> DatabaseConnection {
        axagent_dao::db::create_test_pool().await.expect("测试：创建连接池应成功").conn
    }

    /// upsert 后从库还原（模拟重启：不经内存）全字段一致；同一批次二次 upsert
    /// 为更新而非重复插入。
    #[tokio::test]
    async fn persist_then_restore_roundtrip_and_upsert_updates() {
        let db = test_db().await;
        let mut s = make_persist_state("b1", "2026-05-01", STATUS_COMPLETED, 1_000);
        upsert_batch_run(&db, &s).await.expect("首次 insert 应成功");

        // 同批次状态推进（completed 计数变化）→ upsert 更新
        s.ok = 2;
        s.fail = 0;
        s.updated_at_ms = 2_000;
        upsert_batch_run(&db, &s).await.expect("冲突 upsert 应成功");

        let m = time_travel_batch_runs::Entity::find_by_id("b1")
            .one(&db)
            .await
            .expect("查询应成功")
            .expect("批次行应存在");
        assert_eq!(m.ok, 2, "upsert 应覆盖计数列");
        assert_eq!(m.created_at_ms, 1_000, "创建时间不被更新覆盖");

        let restored = state_from_model(m, false);
        assert_eq!(restored.batch_id, "b1");
        assert_eq!(restored.as_of, "2026-05-01");
        assert_eq!(restored.total, 2);
        assert_eq!(restored.reco_picks, 3);
        assert_eq!(restored.analyses.len(), 2, "明细 JSON 应完整还原");
        assert_eq!(restored.analyses[0].stock_code, "600001");
        assert_eq!(restored.analyses[1].error.as_deref(), Some("数据缺失"));
        assert_eq!(restored.horizons[0].period, "mid");
        assert_eq!(restored.horizons[0].completed, 1);

        // 全表仍只有一行（upsert 未产生重复）
        let count = time_travel_batch_runs::Entity::find().all(&db).await.unwrap().len();
        assert_eq!(count, 1);
    }

    /// 列表按 created_at 倒序、默认上限与 status/as_of 过滤生效；
    /// 摘要模式不携带明细 JSON。
    #[tokio::test]
    async fn list_returns_desc_summary_with_filters() {
        let db = test_db().await;
        // 三条：两条 completed（不同日期）、一条 running
        upsert_batch_run(&db, &make_persist_state("old", "2026-04-01", STATUS_COMPLETED, 1_000))
            .await
            .unwrap();
        upsert_batch_run(&db, &make_persist_state("new", "2026-05-01", STATUS_COMPLETED, 3_000))
            .await
            .unwrap();
        upsert_batch_run(&db, &make_persist_state("run", "2026-05-01", STATUS_RUNNING, 2_000))
            .await
            .unwrap();

        // 直接复用命令的查询逻辑（与 list_time_travel_batches 同构）
        let all: Vec<_> = time_travel_batch_runs::Entity::find()
            .order_by_desc(time_travel_batch_runs::Column::CreatedAtMs)
            .limit(50)
            .all(&db)
            .await
            .unwrap();
        let all: Vec<_> = all.into_iter().map(|m| state_from_model(m, true)).collect();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].batch_id, "new", "倒序：最新创建在前");
        assert_eq!(all[1].batch_id, "run");
        assert_eq!(all[2].batch_id, "old");
        assert!(all[0].analyses.is_empty(), "摘要模式不携带 analyses 明细");
        assert!(all[0].horizons.is_empty());
        assert_eq!(all[0].total, 2, "但标量计数仍完整");

        // status 过滤
        let completed: Vec<_> = time_travel_batch_runs::Entity::find()
            .filter(time_travel_batch_runs::Column::Status.eq(STATUS_COMPLETED))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(completed.len(), 2);

        // as_of 过滤
        let on_day: Vec<_> = time_travel_batch_runs::Entity::find()
            .filter(time_travel_batch_runs::Column::AsDate.eq("2026-05-01"))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(on_day.len(), 2);
    }

    /// 模拟重启：库内遗留 running 行被一次性收敛为 interrupted，
    /// 非 running 终态行不受影响。
    #[tokio::test]
    async fn stale_running_rows_are_interrupted() {
        let db = test_db().await;
        upsert_batch_run(&db, &make_persist_state("zombie", "2026-05-01", STATUS_RUNNING, 1_000))
            .await
            .unwrap();
        upsert_batch_run(&db, &make_persist_state("done", "2026-05-02", STATUS_COMPLETED, 2_000))
            .await
            .unwrap();

        let affected = mark_stale_interrupted(&db).await.unwrap();
        assert_eq!(affected, 1, "仅 running 行被收敛");

        let zombie =
            time_travel_batch_runs::Entity::find_by_id("zombie").one(&db).await.unwrap().unwrap();
        assert_eq!(zombie.status, STATUS_INTERRUPTED);
        assert!(zombie.finished_at_ms.is_some(), "interrupted 也写完成时刻");
        assert!(zombie.message.unwrap().contains("重启"));

        let done =
            time_travel_batch_runs::Entity::find_by_id("done").one(&db).await.unwrap().unwrap();
        assert_eq!(done.status, STATUS_COMPLETED, "终态行不受收敛影响");

        // 再执行一次：无遗留 running，幂等返回 0
        assert_eq!(mark_stale_interrupted(&db).await.unwrap(), 0);
    }
}
