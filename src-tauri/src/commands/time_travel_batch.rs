//! 时间旅行批次复盘（MVP：进程内批次状态，不落批次表）。
//!
//! 流水线（见 `.trae/documents/time-travel-batch_plan.md`）：
//! 1. 选定历史交易日 `as_of`，进入 [`as_of::with_optional_asof`] 封闭世界；
//! 2. **guard 内**：内存荐股（**不写 `reco_picks` 主表**，铁律 B2）→ picks 映射候选
//!    → [`run_analysis_for_candidates`] 逐只 replay 分析（`analysis_kind='replay'`，
//!    分析日 / pending 锚点均为 `as_of`）；
//! 3. **退出 guard 后**再做四档反思（铁律 B3）：guard 内「今天」= as_of 会导致
//!    2/5/28/90 交易日档全部判未到期；
//! 4. 到期档（hindsight ≤ 北京今日）立即反思；未到期档留 pending，由既有
//!    18:00 batch-reflection cron 自然补做。
//!
//! 互斥：AppState 上的全局批次锁 + live 流水线时间窗拒绝
//! （23:50-02:10 / 17:50-18:10，北京时间）——as_of 栈是进程级的，
//! 批次既不能与 live 流水线并发，也不能与另一个批次并发。

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};

use axagent_agent_macro::agent_command;
use axagent_astock_data::AStockClient;
use axagent_astock_data::as_of::{self, AsOfContext, AsOfSource};
use axagent_astock_data::calendar;
use axagent_entities::stock_reflections;
use axagent_harness::holding_period::Period;
use axagent_search::vector_store::VectorStore;
use axagent_trajectory::TrajectoryStorage;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::error::ErrorResponse;
use super::error_code::time_travel as ec;
use super::pool_scan::{PoolCandidate, PoolCandidateResult, run_analysis_for_candidates};
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
const PHASE_ANALYSIS: &str = "analysis";
const PHASE_REFLECTION: &str = "reflection";

/// 发起时间旅行批次的请求（camelCase 与前端契约对齐）。
#[derive(Debug, Clone, Deserialize)]
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
    /// MVP 恒为 true：批次只支持「回放荐股 → 逐只分析」完整链
    #[serde(default = "default_true")]
    pub run_reco: bool,
    /// 趋势智选（Serenity）；MVP 不支持，传 true 会被拒绝
    #[serde(default)]
    pub run_serenity: bool,
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

/// 批次运行状态（序列化给前端轮询）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRunState {
    pub batch_id: String,
    pub as_of: String,
    pub trading_days_ago: Option<i32>,
    /// running / completed / partial / failed
    pub status: String,
    /// analysis / reflection
    pub phase: String,
    /// 人话补充（如「当日荐股无候选」）
    pub message: Option<String>,
    pub total: usize,
    pub ok: usize,
    pub fail: usize,
    pub analyses: Vec<PoolCandidateResult>,
    pub horizons: Vec<HorizonSummary>,
    /// batch_reflection_inner 返回的原始 JSON 摘要（调试/明细用）
    pub reflection_summary: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}

/// 单个复盘档的批次级汇总。
#[derive(Debug, Clone, Serialize)]
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
    fn new(batch_id: String, as_of: String, trading_days_ago: Option<i32>) -> Self {
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
            analyses: Vec::new(),
            horizons: Vec::new(),
            reflection_summary: None,
            error_code: None,
            error_detail: None,
            created_at_ms: now,
            updated_at_ms: now,
            finished_at_ms: None,
        }
    }
}

async fn save_state(mut s: BatchRunState) {
    s.updated_at_ms = now_ms();
    BATCH_RUNS.lock().await.insert(s.batch_id.clone(), s);
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
    // 1. MVP 能力边界
    if request.run_serenity {
        return Err(ErrorResponse::err_with_detail(
            ec::INVALID_PARAMS,
            "时间旅行批次 MVP 暂不支持趋势智选（runSerenity=true）",
        ));
    }
    if !request.run_reco {
        return Err(ErrorResponse::err_with_detail(
            ec::INVALID_PARAMS,
            "时间旅行批次 MVP 仅支持完整链（runReco 不能为 false）",
        ));
    }

    // 2. 周期档解析（空 = 全四档）
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

    // 3. as_of 锚点解析（date 优先；trading_days_ago 从北京今日按交易日回退）
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

    // 4. live 时间窗拒绝（先于抢锁，给出可操作的拒绝原因）
    if in_live_window() {
        return Err(ErrorResponse::err_with_detail(
            ec::LIVE_WINDOW,
            "当前处于 live 流水线时间窗（23:50-02:10 / 17:50-18:10），请避开后再发起时间旅行批次",
        ));
    }

    // 5. 全局批次互斥（try_lock：已在跑直接拒绝，不排队）
    let permit = state
        .time_travel_batch_lock
        .clone()
        .try_lock_owned()
        .map_err(|_| ErrorResponse::err(ec::ALREADY_RUNNING))?;

    // 6. 建批次状态并后台执行
    let batch_id = uuid::Uuid::new_v4().to_string();
    save_state(BatchRunState::new(
        batch_id.clone(),
        as_of.format("%Y-%m-%d").to_string(),
        trading_days_ago,
    ))
    .await;
    // spawn 会 move batch_id，返回给调用方的 id 单独留一份
    let batch_id_to_return = batch_id.clone();

    let db = state.harness.db().clone();
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

/// 查询批次运行状态；批次不存在（重启后 / id 错误）返回 null。
#[agent_command(domain = time_travel, safety = Safe, call_mode = Manual, description = "查询时间旅行批次状态")]
#[tauri::command]
pub async fn get_time_travel_batch(batch_id: String) -> Result<Option<BatchRunState>, String> {
    Ok(BATCH_RUNS.lock().await.get(&batch_id).cloned())
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
    let mut s = BATCH_RUNS
        .lock()
        .await
        .get(&batch_id)
        .cloned()
        .unwrap_or_else(|| BatchRunState::new(batch_id.clone(), as_of_str.clone(), None));

    // ── 阶段 1-3：guard 内内存荐股 → 逐只 replay 分析 ────────────────────────
    // 整个 pipeline future 在 with_optional_asof 内执行；future 返回即 guard drop，
    // 之后阶段 4 的反思天然落在 guard 外（铁律 B3）。
    let pipeline = async {
        // 与荐股 cron 共用模板变量加载（缺陷 D2：两条入口必须同一套权重/阈值）。
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
        let candidates: Vec<PoolCandidate> = scan
            .picks
            .iter()
            .map(|p| PoolCandidate {
                stock_code: p.stock_code.clone(),
                stock_name: p.stock_name.clone(),
                period: p.period,
                confidence: p.confidence as i32,
            })
            .collect();
        let pick_count = candidates.len();
        tracing::info!(
            "[time-travel-batch] {as_of_str} 内存荐股 {pick_count} 只候选（不落 reco_picks），开始逐只 replay 分析"
        );

        let outcome = run_analysis_for_candidates(&db, &client, &engine, candidates).await;
        (pick_count, outcome)
    };

    let (pick_count, outcome) = as_of::with_optional_asof(Some(ctx), pipeline).await;

    // 成功分析的 analysis_id 集合 —— 阶段 4 反思范围严格收口于此
    let analysis_ids: Vec<String> =
        outcome.results.iter().filter_map(|r| r.analysis_id.clone()).collect();

    s.total = outcome.candidates;
    s.ok = outcome.ok;
    s.fail = outcome.fail;
    s.analyses = outcome.results;
    if pick_count == 0 {
        s.message = Some(format!("{as_of_str} 荐股无候选（可能当日数据缺失或阈值过高）"));
    } else if s.total == 0 {
        s.message = Some("候选队列执行异常：0 只进入分析".to_string());
    }
    save_state(s.clone()).await;

    // ── 阶段 4：guard 外四档反思 ─────────────────────────────────────────────
    // 此时 as_of 栈已退出；run_reflection_workflow 内部会按各行 hindsight_date
    // 自行包 AsOfContext（hindsight 锚点），与本批次互不冲突。
    let mut reflection_error: Option<String> = None;
    if reflect {
        if analysis_ids.is_empty() {
            tracing::info!("[time-travel-batch] {batch_id} 无成功分析，跳过反思阶段");
        } else {
            s.phase = PHASE_REFLECTION.to_string();
            save_state(s.clone()).await;

            // 反思前快照（主要为拿到各档 due/pending 分布）
            s.horizons =
                summarize_horizons(&db, &analysis_ids, beijing_today()).await.unwrap_or_default();
            save_state(s.clone()).await;

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
    } else if s.fail > 0 || any_horizon_failed || reflection_error.is_some() {
        if let Some(e) = reflection_error {
            s.error_detail = Some(format!("批次反思失败: {e}"));
        }
        STATUS_PARTIAL
    } else {
        STATUS_COMPLETED
    }
    .to_string();
    s.finished_at_ms = Some(now_ms());
    save_state(s).await;
}

/// 按四档汇总本批 pending/completed/failed 行（同档各行 hindsight 相同，取首行）。
async fn summarize_horizons(
    db: &DatabaseConnection,
    analysis_ids: &[String],
    today: chrono::NaiveDate,
) -> Result<Vec<HorizonSummary>, String> {
    let rows = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::OriginalAnalysisId.is_in(analysis_ids.to_vec()))
        .all(db)
        .await
        .map_err(|e| format!("查询批次反思行失败: {e}"))?;

    // horizon 字符串 → 行集合（本批 pending 行建点即写 horizon，None 老行不计入四档汇总）
    let mut groups: BTreeMap<String, Vec<&stock_reflections::Model>> = BTreeMap::new();
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
