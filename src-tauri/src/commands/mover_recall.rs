//! 窗口涨幅达标漏检核查 —— 命令层（`PLAN-mover-recall-attribution.md` Phase 2/3）
//!
//! 判据与归因全在 `axagent_analysis_engine::mover_recall`（纯函数 + 单测）；这里只做
//! 「把库里的事实喂进去、把结果按板块分组报出来」。
//!
//! ⚠ 三条硬边界：
//! 1. 数据起点 = `market_daily_close` 首个有数据的交易日。早于此的日期**显式声明未采集**，
//!    不做任何回填推断，也不得报成「这段时间没有大涨股」。
//! 2. 候选池降级（快照为空/缺）单独成一层 `pool_degraded`，且判据绑定本轮样本，
//!    不允许把「池没拿到」算成「荐股漏了」。
//! 3. 按设计不出票的档位（如趋势智选超短/短）走 `by_design`，**不进漏检分母**，
//!    在面板上属于说明段而不是待优化清单。

use std::collections::{BTreeMap, HashMap};

use axagent_agent_macro::agent_command;
use axagent_analysis_engine::mover_recall::{
    self, CHAIN_UNLABELED, Evidence, MissLayer, MoverEvent, RecoChain, reco_picks,
};
use axagent_dao::DatabaseConnection;
use axagent_dao::repo::mover_recall_runs::{self, RunEventRecord, RunModel, RunSnapshotRecord};
use axagent_dao::repo::{stock_analysis_snapshot, stock_lesson_queries};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::common as common_err;
use crate::commands::error_code::stock_workflow as wf_err;

/// 妖股漏检核查定时任务的 task_type（前端任务中心白名单与本常量逐字对齐）。
pub use crate::commands::constants::MOVER_RECALL_TASK_TYPE;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverRecallView {
    /// 本视图属于哪条链：`reco` | `serenity` | `unlabeled`（A-2 之前落库的轮次）。
    /// 值域唯一权威在 `mover_recall::RecoChain` / `CHAIN_UNLABELED`，面板与门都不另抄一份。
    pub chain: String,
    /// 核查覆盖的日期区间（实际可用的，非用户请求的）
    pub from: String,
    pub to: String,
    /// 数据起点声明：早于此没有快照 ⇒ 面板必须原文显示这句，不得留白
    pub data_since: String,
    /// 区间内实际采集的交易日数（只数到 `to`）：某档可判定 ⇔ 本值 ≥ 该档 `window_days`。
    /// 未达阈时该档「无事件」不是结论而是「尚不可判定」——面板必须逐档标注，不得混说。
    pub collected_days: usize,
    pub universe_size: i64,
    /// 清单里被东财全量确认过的比例（<1 ⇒ 枚举域由本地票拼成，池外大涨票可能不在样本内）
    pub universe_confirmed: i64,
    pub rules: Vec<mover_recall::TierRule>,
    pub rates: mover_recall::RecallRates,
    pub layers: Vec<mover_recall::LayerRow>,
    /// 逐档达标事件数（`period -> events`）：C-2 的三态依据 —— 本档已满 + 0 事件 =「真无达标票」，
    /// 必须能与「本档有达标」分开说。`None` = 这条载荷没记录过（旧轮次）⇒ 面板第三态，不显示 0。
    pub events_by_tier: Option<BTreeMap<String, usize>>,
    /// 漏检事件明细（按 |累计涨幅| 降序，上限 200 条，完整集在 layers 计数里）
    pub misses: Vec<MissRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissRow {
    pub event: MoverEvent,
    pub layer: MissLayer,
    /// 该票在窗口内是否有过任一档推荐（true 且 layer 仍是漏检 ⇒ 档位错配）
    pub picked_any_period: bool,
    /// 有截断留痕的风格键（矩阵名目）—— 仅 `l3ScoredOut` 非空；
    /// 空表示**无留痕**，不代表没有风格淘汰过它（见 `reco_scan_audit` 语义边界）
    pub scored_out_styles: Vec<String>,
}

fn date_of(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

/// 主入口：跑一次核查（手动面板用，**不落库**）。
///
/// A-2 起返回**每链一份**视图（荐股在前、智选在后）：同一条达标事件在两条链上各有
/// 「当时为什么没选它」。混成一份就会让趋势智选的命中顶掉智能荐股的漏检（反之亦然），
/// 两条链都没法各自被优化 —— 那正是本轮需求的前提。
#[agent_command(
    domain = "finance",
    safety = Safe,
    call_mode = StateInput,
    description = "窗口涨幅达标漏检核查"
)]
#[tauri::command]
pub async fn analyze_mover_recall(
    state: tauri::State<'_, AppState>,
    to: Option<String>,
) -> Result<Vec<MoverRecallView>, String> {
    let mut views = analyze_mover_recall_inner(state.harness.db(), to).await?;
    // 面板每链只展示 top 200（inner 保留全集供调度侧落 mover_recall_events）
    for v in views.iter_mut() {
        v.misses.truncate(200);
    }
    Ok(views)
}

/// 一轮核查的共享事实：两条链从同一份取数出发，只有命中集合按链切。
///
/// 取数只做一次 ⇒ 分链**不会**把收盘快照的读取翻倍（`market_daily_close` 是这条链最贵的一面）。
struct SharedFacts {
    from: String,
    to: String,
    data_since: String,
    collected_days: usize,
    universe_size: i64,
    universe_confirmed: i64,
    rules: Vec<mover_recall::TierRule>,
    by_stock: mover_recall::CloseSeriesByCode,
    picks: Vec<reco_picks::Model>,
    trim_evidence: HashMap<(String, String), Vec<String>>,
}

/// 核查实现：命令与 `mover-recall` cron 共用同一套逻辑
/// （同 `run_decision_backtest_inner` 的形态——cron 拿不到 `tauri::State`）。
pub async fn analyze_mover_recall_inner(
    db: &DatabaseConnection,
    to: Option<String>,
) -> Result<Vec<MoverRecallView>, String> {
    let served =
        axagent_analysis_engine::recommender::reco_loop::load_reco_served_vars_db(db).await?;
    let rules = mover_recall::tier_rules_from_vars(&served);
    if rules.is_empty() {
        return Err(ErrorResponse::new(wf_err::INTERNAL)
            .with_detail("四档涨幅判据全部无效（阈值变量非正数），本轮不产出核查结果")
            .to_string());
    }

    let close_dates = mover_recall::load_close_dates(db).await?;
    if close_dates.is_empty() {
        return Err(ErrorResponse::new(wf_err::INTERNAL)
            .with_detail(
                "market_daily_close 为空：尚未采集过全市场收盘快照，核查不可运行（非「无大涨股」）",
            )
            .to_string());
    }
    let from = close_dates.first().cloned().unwrap_or_default();
    let to = to.unwrap_or_else(|| close_dates.last().cloned().unwrap_or_default());
    let upper = if to.as_str() > close_dates.last().map(String::as_str).unwrap_or("") {
        close_dates.last().cloned().unwrap_or_default()
    } else {
        to.clone()
    };
    // 采集日数只数到 `upper`（事件锚点不可能晚于它）——某档可判定 ⇔ 本值 ≥ window_days
    let collected_days = close_dates.iter().take_while(|d| d.as_str() <= upper.as_str()).count();

    let by_stock = mover_recall::load_closes_by_stock(db, &from, &upper).await?;
    let picks = mover_recall::load_picks_in_range(db, &from, &upper, None).await?;
    let trim_evidence = mover_recall::load_trim_evidence(db, &from, &upper, None).await?;
    let (universe_size, universe_confirmed) = mover_recall::load_universe_counts(db).await?;

    let facts = SharedFacts {
        from,
        to: upper,
        data_since: close_dates.first().cloned().unwrap_or_default(),
        collected_days,
        universe_size,
        universe_confirmed,
        rules,
        by_stock,
        picks,
        trim_evidence,
    };
    Ok(RecoChain::ALL.iter().map(|c| build_chain_view(*c, &facts)).collect())
}

/// 纯函数：从共享事实算**一条链**的视图。
///
/// 拆成纯函数的目的就是让「智选命中不得顶替荐股、荐股留痕不得替智选凑账」这两条判据
/// 能在不连库的情况下被钉住（见 `mod chain_attribution_tests`，含两条姊妹负控）。
fn build_chain_view(chain: RecoChain, f: &SharedFacts) -> MoverRecallView {
    let chain_picks: Vec<&reco_picks::Model> =
        f.picks.iter().filter(|p| RecoChain::of_style(&p.style) == chain).collect();

    let mut layers: BTreeMap<&'static str, (MissLayer, usize, BTreeMap<String, usize>)> =
        BTreeMap::new();
    let mut misses: Vec<MissRow> = Vec::new();
    let mut events_total = 0usize;
    let mut events_by_tier: BTreeMap<String, usize> = BTreeMap::new();

    for rule in &f.rules {
        let tier_active = mover_recall::chain_period_has_active_style(chain, rule.period);
        let evs = mover_recall::events_for_tier(&f.by_stock, rule, &f.from, &f.to);
        for mut ev in evs {
            events_total += 1;
            *events_by_tier.entry(rule.period.as_str().to_string()).or_default() += 1;
            // window_start 由 events_for_tier 在识别事件时一并算定（同一窗口切片的首日），
            // 此处不得再按 close_dates 重算一份 —— 两份日期序列同源但各算一遍会制造
            // 「pick 过滤窗口」与「事件窗口」漂移的可能（单一来源，见禁区 12）。
            let window_start = ev.window_start.clone();
            let in_window: Vec<_> = chain_picks
                .iter()
                .filter(|p| {
                    let d = date_of(&p.generated_at);
                    d >= window_start.as_str() && d <= ev.anchor_date.as_str()
                })
                .collect();

            let picks_same_period: Vec<String> = in_window
                .iter()
                .filter(|p| p.stock_code == ev.stock_code && p.period == ev.period)
                .map(|p| p.generated_at.clone())
                .collect();
            ev.recommended = !picks_same_period.is_empty();
            // 同链内「该链在这只票的任一档出过票」——不再重复 `period` 相等那一支（被本条件包含）
            let picked_any_period = in_window.iter().any(|p| p.stock_code == ev.stock_code);
            // 阶段 5 / G1：本窗没有**该链**的批次 ⇒ 是「不知道」，不是「不在池里」。
            // 这一格直接决定事件落 `not_in_pool` 还是 `unexplained`，也就是把优化方向
            // 带去「扩候选宇宙」还是「补留痕」。
            let in_pool = if in_window.is_empty() {
                None
            } else {
                Some(in_window.iter().any(|p| {
                    mover_recall::code_in_pool_snapshot(p.seed_pool_json.as_deref(), &ev.stock_code)
                }))
            };
            // A-3 `LateTiming` 的证据：本链在同档、**窗口起点之前**出过票 ⇒ 链看见过这只票，
            // 只是时点不落在本窗。不算漏检（`counts_as_miss` 已排除），但必须单独成层，
            // 否则它会被并进 `not_in_pool`，把「推荐时机」的问题报成「候选宇宙」的问题。
            let picked_before_window = chain_picks.iter().any(|p| {
                p.stock_code == ev.stock_code
                    && p.period == ev.period
                    && date_of(&p.generated_at) < window_start.as_str()
            });

            // 本轮池快照为空 ⇒ 降级（判据绑定本轮，不用别轮的值兜）
            let pool_degraded = !in_window.is_empty()
                && in_window.iter().any(|p| match p.seed_pool_json.as_deref() {
                    None => true,
                    Some(s) => s.trim() == "[]" || s.trim().is_empty(),
                });

            // 截断留痕只由**荐股链**的 top-N 截断写（`dao::repo::reco_pool`）；趋势智选工作流
            // 不写 `reco_scan_audit` ⇒ 智选链这一格必须留空，不能借荐股的留痕凑一个
            // 「算出却淘汰」。智选侧补写留痕是 PLAN 的 B-2，尚未落地。
            let scored_out = if chain == RecoChain::Reco {
                f.trim_evidence
                    .get(&(ev.period.clone(), ev.stock_code.clone()))
                    .cloned()
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            let evidence = Evidence {
                picks_for_period: picks_same_period,
                picked_in_other_period: picked_any_period
                    && in_window
                        .iter()
                        .any(|p| p.stock_code == ev.stock_code && p.period != ev.period),
                in_pool,
                pool_degraded,
                chain_tier_active: tier_active,
                picked_before_window,
                l3_scored_out_styles: scored_out.clone(),
            };
            let layer = mover_recall::attribute(&evidence);
            let entry = layers.entry(layer.as_str()).or_insert((layer, 0, BTreeMap::new()));
            entry.1 += 1;
            *entry.2.entry(ev.market_type.clone()).or_default() += 1;
            if layer.counts_as_miss() {
                misses.push(MissRow {
                    event: ev,
                    layer,
                    picked_any_period,
                    scored_out_styles: scored_out,
                });
            }
        }
    }

    let mut counts: HashMap<MissLayer, usize> = HashMap::new();
    for (layer, n, _) in layers.values() {
        counts.insert(*layer, *n);
    }
    let rates = mover_recall::compute_rates(&counts, events_total);
    let layer_rows: Vec<mover_recall::LayerRow> = layers
        .values()
        .map(|(layer, n, by_m)| mover_recall::LayerRow {
            layer: *layer,
            count: *n,
            by_market_type: by_m.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        })
        .collect();

    misses.sort_by(|a, b| b.event.cum_gain_pct.total_cmp(&a.event.cum_gain_pct));
    // 注意：inner 返回**全集**（P2 落 mover_recall_events 需要），200 条截断是面板表现层
    // 关注点，由薄命令 analyze_mover_recall 在返回前做；persist 走全集。

    MoverRecallView {
        chain: chain.as_str().to_string(),
        from: f.from.clone(),
        to: f.to.clone(),
        data_since: f.data_since.clone(),
        collected_days: f.collected_days,
        universe_size: f.universe_size,
        universe_confirmed: f.universe_confirmed,
        rules: f.rules.clone(),
        rates,
        layers: layer_rows,
        events_by_tier: Some(events_by_tier),
        misses,
    }
}

/// 把一轮核查结果快照落 `mover_recall_runs`，并把漏检事件**全集**落
/// `mover_recall_events`（P2 补分析样本池），同一事务，返回 run id。
///
/// 仅调度器分支调用（定时到点与任务中心「立即执行」共用同一 dispatch）；
/// 前端面板的只读核查不落库。
///
/// 事件初态：`pool_degraded`（数据可达性问题，方案明确不进样本）落库即 `excluded`；
/// 其余四层（not_in_pool / l3_scored_out / other_period / unexplained）为 `pending`。
/// 注意 `unexplained` 是**事件级 pending + 批次级过滤**：补分析选 run 时再按
/// `rates.unexplained_share` 整轮排除高占比轮次（见 mover_recall_analysis 模块）。
pub async fn persist_mover_recall_run(
    db: &DatabaseConnection,
    view: &MoverRecallView,
    trigger_source: &str,
) -> Result<String, String> {
    let id = format!("mrr-{}", uuid::Uuid::new_v4());
    let now_ms = chrono::Utc::now().timestamp_millis();
    let rules_json = serde_json::to_string(&view.rules).map_err(|e| {
        ErrorResponse::err_with_detail(
            wf_err::SERIALIZE_FAILED,
            format!("mover_recall_runs.rules_json 序列化失败: {e}"),
        )
    })?;
    let rates_json = serde_json::to_string(&view.rates).map_err(|e| {
        ErrorResponse::err_with_detail(
            wf_err::SERIALIZE_FAILED,
            format!("mover_recall_runs.rates_json 序列化失败: {e}"),
        )
    })?;
    let layers_json = serde_json::to_string(&view.layers).map_err(|e| {
        ErrorResponse::err_with_detail(
            wf_err::SERIALIZE_FAILED,
            format!("mover_recall_runs.layers_json 序列化失败: {e}"),
        )
    })?;
    let events_by_tier_json = view
        .events_by_tier
        .as_ref()
        .map(|m| {
            serde_json::to_string(m).map_err(|e| {
                ErrorResponse::err_with_detail(
                    wf_err::SERIALIZE_FAILED,
                    format!("mover_recall_runs.events_by_tier_json 序列化失败: {e}"),
                )
            })
        })
        .transpose()?;
    let misses_json = if view.misses.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&view.misses).map_err(|e| {
            ErrorResponse::err_with_detail(
                wf_err::SERIALIZE_FAILED,
                format!("mover_recall_runs.misses_json 序列化失败: {e}"),
            )
        })?)
    };

    let run_row = RunSnapshotRecord {
        id: id.clone(),
        chain: Some(view.chain.clone()),
        date_from: view.from.clone(),
        date_to: view.to.clone(),
        data_since: view.data_since.clone(),
        collected_days: view.collected_days as i32,
        universe_size: view.universe_size,
        universe_confirmed: view.universe_confirmed,
        rules_json,
        rates_json,
        layers_json,
        events_total: view.rates.events as i32,
        miss_count: view.rates.misses as i32,
        misses_json,
        events_by_tier_json,
        status: "success".to_string(),
        trigger_source: trigger_source.to_string(),
        created_at_ms: now_ms,
    };

    // 事件全集（view.misses 此时未截断 —— 截断只在薄命令返回面板前做）
    let event_rows: Vec<RunEventRecord> = view
        .misses
        .iter()
        .map(|m| RunEventRecord {
            id: format!("mre-{}", uuid::Uuid::new_v4()),
            run_id: id.clone(),
            chain: Some(view.chain.clone()),
            stock_code: m.event.stock_code.clone(),
            stock_name: m.event.stock_name.clone(),
            period: m.event.period.clone(),
            anchor_date: m.event.anchor_date.clone(),
            window_start: m.event.window_start.clone(),
            window_end: m.event.anchor_date.clone(),
            window_days: m.event.window_days as i32,
            cum_gain_pct: m.event.cum_gain_pct,
            max_daily_pct: m.event.max_daily_pct,
            threshold_pct: m.event.threshold_pct,
            market_type: m.event.market_type.clone(),
            miss_layer: m.layer.as_str().to_string(),
            picked_any_period: if m.picked_any_period { 1 } else { 0 },
            scored_out_styles_json: if m.scored_out_styles.is_empty() {
                None
            } else {
                serde_json::to_string(&m.scored_out_styles).ok()
            },
            // pool_degraded（数据可达性问题，方案明确不进样本）落库即 excluded；
            // 其余四层为 pending。unexplained 的整轮过滤在补分析选 run 时做。
            status: if m.layer == MissLayer::PoolDegraded {
                "excluded".to_string()
            } else {
                "pending".to_string()
            },
            created_at_ms: now_ms,
        })
        .collect();

    mover_recall_runs::insert_run_snapshot(db, &run_row, &event_rows).await.map_err(|e| {
        ErrorResponse::err_with_detail(wf_err::INTERNAL, format!("mover_recall 核查落库失败: {e}"))
    })?;
    Ok(id)
}

// ── P3：漏检事件 → 补分析报告 + 逐档反思结论（面板展开行懒加载）─────────────────

/// 单个漏检事件的复盘视图：事件事实 + 补分析行（若已成功）+ 逐档反思行（若已复盘）。
///
/// 三键（stock_code + anchor_date + period）定位，因为实时核查视图 `MissRow`
/// 不带 eventId/analysisId；同一三键跨多轮核查时取最新一轮（created_at_ms DESC）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverRecallEventReview {
    pub event_id: String,
    pub run_id: String,
    /// pending / excluded / running / analyzed / failed / reflected
    pub status: String,
    /// 权威归因层码（系统机械判定）
    pub miss_layer: String,
    pub period: String,
    pub stock_code: String,
    pub stock_name: String,
    pub window_start: String,
    pub anchor_date: String,
    pub window_days: i32,
    pub cum_gain_pct: f64,
    pub max_daily_pct: f64,
    pub batch_id: Option<String>,
    pub analysis_id: Option<String>,
    /// 首个 completed 反思档的行 id（事件 reflected 时非空）
    pub reflection_id: Option<String>,
    pub error_detail: Option<String>,
    /// as-of 补分析整行（stock_analyses 序列化为 camelCase）；未跑到成功为 None
    pub analysis: Option<serde_json::Value>,
    /// 逐档反思行投影（与 list_reflections 同一投影函数），按 horizon 升序
    pub reflections: Vec<serde_json::Value>,
}

/// 查单个漏检事件的补分析报告与反思结论（查无此事件返回 `Ok(None)`）。
#[agent_command(
    domain = "finance",
    safety = Safe,
    call_mode = StateInput,
    description = "查询妖股漏检事件的补分析报告与逐档反思结论"
)]
#[tauri::command]
pub async fn get_mover_recall_event_review(
    state: tauri::State<'_, AppState>,
    stock_code: String,
    anchor_date: String,
    period: String,
    // 面板当前视图所属链（`reco` | `serenity`）；旧轮次的 `unlabeled` 传 None。
    // 形参上不能挂 ///（Rust 只允许内置属性），故留在函数体内说明见 find_latest_by_key 的文档。
    chain: Option<String>,
) -> Result<Option<MoverRecallEventReview>, String> {
    let db = state.harness.db();
    let anchor = date_of(&anchor_date);

    // 同三键可能被多轮核查重复落（每轮全量替换各自 run 的行）⇒ dao 取最新一轮。
    let event = match axagent_dao::repo::mover_recall_events::find_latest_by_key(
        db,
        &stock_code,
        anchor,
        &period,
        chain.filter(|c| c != CHAIN_UNLABELED).as_deref(),
    )
    .await
    {
        Ok(ev) => ev,
        Err(e) => {
            return Err(ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("查询漏检事件失败: {e}"))
                .to_string());
        },
    };
    let Some(ev) = event else {
        return Ok(None);
    };

    // 补分析行：failed/pending/running 时 analysis_id 可能为空或指向非 completed 行，
    // 都照实投影，前端按 event.status + analysis.status 分态展示。
    let analysis = if let Some(aid) = ev.analysis_id.as_deref() {
        stock_analysis_snapshot::fetch_analysis_by_id(db, aid)
            .await
            .map_err(|e| {
                ErrorResponse::new(wf_err::INTERNAL)
                    .with_detail(format!("查询补分析行失败: {e}"))
                    .to_string()
            })?
            .and_then(|m| serde_json::to_value(m).ok())
    } else {
        None
    };

    // 逐档反思：按补分析行 id 反查（一个事件多档 = 多行）。
    let reflections = if let Some(aid) = ev.analysis_id.as_deref() {
        stock_lesson_queries::fetch_by_analysis_id(db, aid)
            .await
            .map_err(|e| {
                ErrorResponse::new(wf_err::INTERNAL)
                    .with_detail(format!("查询逐档反思失败: {e}"))
                    .to_string()
            })?
            .iter()
            .map(stock_lesson_queries::reflection_review_json)
            .collect()
    } else {
        Vec::new()
    };

    Ok(Some(MoverRecallEventReview {
        event_id: ev.id,
        run_id: ev.run_id,
        status: ev.status,
        miss_layer: ev.miss_layer,
        period: ev.period,
        stock_code: ev.stock_code,
        stock_name: ev.stock_name,
        window_start: ev.window_start,
        anchor_date: ev.anchor_date,
        window_days: ev.window_days,
        cum_gain_pct: ev.cum_gain_pct,
        max_daily_pct: ev.max_daily_pct,
        batch_id: ev.batch_id,
        analysis_id: ev.analysis_id,
        reflection_id: ev.reflection_id,
        error_detail: ev.error_detail,
        analysis,
        reflections,
    }))
}

// ── D-5：按轮次读回已落库的核查结果（不重算、不落库）──────────────────────────
//
// 为什么要有这一组：核查轮次由调度器（含任务中心「立即执行」）落 `mover_recall_runs` /
// `mover_recall_events`，但面板只会调 `analyze_mover_recall` **现场重算** ⇒ 用户点完执行
// 在 UI 里拿不到「那一次」的结果（2026-10-20 实机反馈）。落库而无读回入口同时是
// `check-reflection-output-consumers.mjs` 登记的消费面之一。

/// 一轮核查的元信息（面板轮次选择器 + 「本轮来源」声明用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverRecallRunMeta {
    pub id: String,
    /// 本轮属于哪条链；NULL = A-2 之前的混池轮次（面板单独成句，不得当成某一链）
    pub chain: Option<String>,
    pub date_from: String,
    pub date_to: String,
    pub data_since: String,
    pub collected_days: i32,
    pub universe_size: i64,
    pub universe_confirmed: i64,
    pub events_total: i32,
    pub miss_count: i32,
    /// 当前只有 `success` 会落轮次行（失败不落库）⇒ 面板不得把「库里没有这一轮」读成
    /// 「那一轮没有达标事件」，两者是不同的缺席。
    pub status: String,
    pub trigger_source: String,
    pub created_at_ms: i64,
}

/// `get_mover_recall_run_view` 响应：轮次元信息 + 与实时核查**同形**的载荷。
///
/// `view` 摊平进同一层（`flatten`）⇒ 面板复用 `MoverRecallView` 的整套渲染与形状守卫，
/// 不出现「两种来源两种形状、渲染分支各写一遍」的第二套呈现判据。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverRecallRunView {
    pub run: MoverRecallRunMeta,
    #[serde(flatten)]
    pub view: MoverRecallView,
}

/// 轮次列表的缺省条数（周频小表，20 轮 ≈ 5 个月）。
const RUN_LIST_DEFAULT_LIMIT: u32 = 20;

/// 一轮载荷的拆分（字段与 `MoverRecallView` 一一对应）。
struct StoredRunPayload {
    rules: Vec<mover_recall::TierRule>,
    rates: mover_recall::RecallRates,
    layers: Vec<mover_recall::LayerRow>,
    misses: Vec<MissRow>,
}

fn run_read_err(detail: String) -> String {
    ErrorResponse::new(wf_err::INTERNAL).with_detail(detail).to_string()
}

fn run_meta_of(run: &RunModel) -> MoverRecallRunMeta {
    MoverRecallRunMeta {
        id: run.id.clone(),
        chain: run.chain.clone(),
        date_from: run.date_from.clone(),
        date_to: run.date_to.clone(),
        data_since: run.data_since.clone(),
        collected_days: run.collected_days,
        universe_size: run.universe_size,
        universe_confirmed: run.universe_confirmed,
        events_total: run.events_total,
        miss_count: run.miss_count,
        status: run.status.clone(),
        trigger_source: run.trigger_source.clone(),
        created_at_ms: run.created_at_ms,
    }
}

/// 把一轮的四份 JSON 还原成载荷。纯字符串入参 ⇒ 坏样本不连库就能单测。
///
/// 任一 JSON 解不出来一律 `Err`：载荷缺一半还硬出图，就是把「读不到」演成「本轮没有事件」。
/// 唯一合法的「空」是 `misses_json` 为 NULL —— 落库侧只在 `misses.is_empty()` 时写 NULL
/// （见 `persist_mover_recall_run`），所以它确实表示「那一次没有漏检」，不是解析失败。
fn parse_run_payload(
    rules_json: &str,
    rates_json: &str,
    layers_json: &str,
    misses_json: Option<&str>,
) -> Result<StoredRunPayload, String> {
    let stored_rules: Vec<mover_recall::StoredTierRule> = serde_json::from_str(rules_json)
        .map_err(|e| {
            ErrorResponse::err_with_detail(
                common_err::INVALID_INPUT,
                format!("rules_json 解析失败: {e}"),
            )
        })?;
    let rules = mover_recall::rules_from_stored(&stored_rules)?;
    let stored_rates: mover_recall::StoredRates =
        serde_json::from_str(rates_json).map_err(|e| {
            ErrorResponse::err_with_detail(
                common_err::INVALID_INPUT,
                format!("rates_json 解析失败: {e}"),
            )
        })?;
    let layers: Vec<mover_recall::LayerRow> = serde_json::from_str(layers_json).map_err(|e| {
        ErrorResponse::err_with_detail(
            common_err::INVALID_INPUT,
            format!("layers_json 解析失败: {e}"),
        )
    })?;
    let misses = match misses_json {
        None => Vec::new(),
        Some(raw) => serde_json::from_str(raw).map_err(|e| {
            ErrorResponse::err_with_detail(
                common_err::INVALID_INPUT,
                format!("misses_json 解析失败: {e}"),
            )
        })?,
    };
    Ok(StoredRunPayload { rules, rates: stored_rates.into_rates(), layers, misses })
}

fn run_payload_of(run: &RunModel) -> Result<MoverRecallView, String> {
    let p = parse_run_payload(
        &run.rules_json,
        &run.rates_json,
        &run.layers_json,
        run.misses_json.as_deref(),
    )?;
    Ok(MoverRecallView {
        chain: run.chain.clone().unwrap_or_else(|| CHAIN_UNLABELED.to_string()),
        from: run.date_from.clone(),
        to: run.date_to.clone(),
        data_since: run.data_since.clone(),
        collected_days: run.collected_days as usize,
        universe_size: run.universe_size,
        universe_confirmed: run.universe_confirmed,
        rules: p.rules,
        rates: p.rates,
        layers: p.layers,
        // 旧轮次没有这一列 ⇒ `None`（面板成句「本轮未记录逐档计数」），绝不兜成空表当 0
        events_by_tier: match run.events_by_tier_json.as_deref() {
            None => None,
            Some(raw) => Some(
                serde_json::from_str(raw)
                    .map_err(|e| run_read_err(format!("events_by_tier_json 解析失败: {e}")))?,
            ),
        },
        misses: p.misses,
    })
}

/// 列出已落库的核查轮次（最新在前）。空数组 = 库里一轮都没有 ⇒ 面板必须显式说
/// 「还没有已落库轮次」，不得让它看起来像「核查过但没有漏检」。
#[agent_command(
    domain = "finance",
    safety = Safe,
    call_mode = StateInput,
    description = "列出已落库的窗口涨幅达标核查轮次"
)]
#[tauri::command]
pub async fn list_mover_recall_runs(
    state: tauri::State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<MoverRecallRunMeta>, String> {
    let n = limit.unwrap_or(RUN_LIST_DEFAULT_LIMIT).max(1) as u64;
    let rows = mover_recall_runs::list_recent_runs(state.harness.db(), n)
        .await
        .map_err(|e| run_read_err(format!("读 mover_recall_runs 失败: {e}")))?;
    Ok(rows.iter().map(run_meta_of).collect())
}

/// 按轮次读回那一次核查的载荷（**不重算、不落库**）。
///
/// `runId` 必填：「不传就取最新」是一个会随时间换答案的隐藏缺省，面板的轮次选择器
/// 本来就先列后选，不需要这个自由度。查无该轮 ⇒ `Ok(None)`（缺席由面板成句声明）。
#[agent_command(
    domain = "finance",
    safety = Safe,
    call_mode = StateInput,
    description = "按轮次读回窗口涨幅达标核查结果"
)]
#[tauri::command]
pub async fn get_mover_recall_run_view(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Result<Option<MoverRecallRunView>, String> {
    let db = state.harness.db();
    let found = mover_recall_runs::fetch_run(db, &run_id)
        .await
        .map_err(|e| run_read_err(format!("读 mover_recall_runs 失败: {e}")))?;
    let Some(run) = found else {
        return Ok(None);
    };
    let view = run_payload_of(&run).map_err(run_read_err)?;
    Ok(Some(MoverRecallRunView { run: run_meta_of(&run), view }))
}

#[cfg(test)]
mod stored_run_tests {
    use super::*;

    // 夹具取自现网 `mrr-6eaaeed5-…` 的实际落库形态（键必须真，值可为构造样本）。
    const RULES: &str = r#"[{"period":"ultra_short","varName":"mover_gain_ultra_short","gainPct":10.0,"windowDays":2},{"period":"mid","varName":"mover_gain_mid","gainPct":35.0,"windowDays":28}]"#;
    const RATES: &str = r#"{"reachability":50.64935064935065,"coverage":30.76923076923077,"unexplainedShare":3.076923076923077,"events":77,"misses":65}"#;
    const LAYERS: &str = r#"[{"layer":"not_in_pool","count":38,"byMarketType":[["chinext",3],["main_sh",2],["main_sz",33]]}]"#;
    const MISSES: &str = r#"[{"event":{"stockCode":"001246","stockName":"力勤资源","period":"ultra_short","anchorDate":"2026-10-09","windowStart":"2026-09-30","windowDays":2,"cumGainPct":157.93,"maxDailyPct":206.59,"thresholdPct":10.0,"marketType":"main_sz","recommended":false},"layer":"other_period","pickedAnyPeriod":true,"scoredOutStyles":["capital"]}]"#;

    #[test]
    fn parse_run_payload_reads_back_a_stored_run() {
        let p = parse_run_payload(RULES, RATES, LAYERS, Some(MISSES)).unwrap();
        assert_eq!(p.rules.len(), 2);
        assert_eq!(p.rules[1].var_name, "mover_gain_mid");
        assert_eq!(p.rules[1].gain_pct, 35.0, "读回必须是那一代阈值，不是当前的服务值");
        assert_eq!(p.rules[1].window_days, 28);
        assert!((p.rates.reachability - 50.64935064935065).abs() < 1e-9);
        assert_eq!(p.layers[0].count, 38);
        assert_eq!(p.misses.len(), 1);
        assert_eq!(p.misses[0].layer, MissLayer::OtherPeriod);
        assert_eq!(p.misses[0].event.stock_code, "001246");
        assert_eq!(p.misses[0].scored_out_styles, vec!["capital".to_string()]);
    }

    /// `misses_json` 为 NULL 是落库侧「该轮零漏检」的唯一合法表示 ⇒ 空集，不是错误。
    #[test]
    fn parse_run_payload_treats_null_misses_as_empty_not_failure() {
        let p = parse_run_payload(RULES, RATES, LAYERS, None).unwrap();
        assert!(p.misses.is_empty());
        assert_eq!(p.rules.len(), 2, "零漏检不影响判据表");
    }

    /// 坏样本：变量名不在登记表 ⇒ Err（静默丢一档会让面板少显示一档而无人报警）。
    #[test]
    fn parse_run_payload_rejects_unregistered_var_name() {
        let bad = RULES.replace("mover_gain_mid", "mover_gain_quarter");
        assert!(parse_run_payload(&bad, RATES, LAYERS, Some(MISSES)).is_err());
    }

    /// 坏样本：载荷 JSON 半截 ⇒ Err，不得给空视图（空视图会被读成「本轮没有事件」）。
    #[test]
    fn parse_run_payload_rejects_truncated_layers_json() {
        assert!(parse_run_payload(RULES, RATES, "[{", Some(MISSES)).is_err());
    }

    /// 边界：算不出来的率落库是 `null` ⇒ 读回仍是 NaN（「样本不足」），不是 0。
    #[test]
    fn parse_run_payload_keeps_uncomputable_rates_as_nan() {
        let null_rates = r#"{"reachability":null,"coverage":null,"unexplainedShare":null,"events":0,"misses":0}"#;
        let p = parse_run_payload(RULES, null_rates, LAYERS, None).unwrap();
        assert!(p.rates.reachability.is_nan());
        assert!(p.rates.coverage.is_nan());
        assert!(p.rates.unexplained_share.is_nan());
        assert_eq!(p.rates.events, 0);
    }
}

#[cfg(test)]
mod chain_attribution_tests {
    use super::*;
    use axagent_analysis_engine::mover_recall::CloseSeriesByCode;

    /// 默认出票时点：落在 10-08/10-09 那个超短窗内。
    const PICK_AT: &str = "2026-10-08T09:00:00";

    fn pick_on(
        id: &str,
        style: &str,
        code: &str,
        period: &str,
        pool: Option<&str>,
        generated_at: &str,
    ) -> reco_picks::Model {
        reco_picks::Model {
            id: id.to_string(),
            generated_at: generated_at.to_string(),
            period: period.to_string(),
            stock_code: code.to_string(),
            stock_name: format!("股{code}"),
            style: style.to_string(),
            confidence: 60,
            synthetic: 0,
            seed_pool_json: pool.map(str::to_string),
            strategy_weights_json: None,
            pick_data: None,
            reco_version: None,
            as_of_date: None,
            created_at: generated_at.to_string(),
        }
    }

    fn pick(
        id: &str,
        style: &str,
        code: &str,
        period: &str,
        pool: Option<&str>,
    ) -> reco_picks::Model {
        pick_on(id, style, code, period, pool, PICK_AT)
    }

    /// 一条票的收盘序列：(代码, 名称, [(日期, 当日涨幅%)])。命名是为了过 `type_complexity`，
    /// 也是为了下面两个夹具能共用同一套构造（日期带生命期是因为 mid 夹具自己 `format!` 出日期）。
    type Series<'a> = (&'a str, &'a str, Vec<(&'a str, f64)>);

    /// 通用事实夹具：收盘序列 + 出票 + 截断留痕，区间由调用方给（超短窗与 28 日中窗共用一套）。
    fn facts_with(
        closes: Vec<Series<'_>>,
        picks: Vec<reco_picks::Model>,
        trim: Vec<(&str, &str, &[&str])>,
        from: &str,
        to: &str,
        collected_days: usize,
    ) -> SharedFacts {
        let mut by_stock: CloseSeriesByCode = HashMap::new();
        for (code, name, series) in closes {
            by_stock.insert(
                code.to_string(),
                (name.to_string(), series.into_iter().map(|(d, v)| (d.to_string(), v)).collect()),
            );
        }
        let mut trim_evidence: HashMap<(String, String), Vec<String>> = HashMap::new();
        for (period, code, styles) in trim {
            trim_evidence.insert(
                (period.to_string(), code.to_string()),
                styles.iter().map(|s| s.to_string()).collect(),
            );
        }
        SharedFacts {
            from: from.to_string(),
            to: to.to_string(),
            data_since: from.to_string(),
            collected_days,
            universe_size: 2,
            universe_confirmed: 2,
            rules: mover_recall::tier_rules_from_vars(&[]),
            by_stock,
            picks,
            trim_evidence,
        }
    }

    /// 002696 在超短档两日各 +10% ⇒ 复利累计 ≈21% ≥ 出厂阈值 10% ⇒ 恰产出一条事件。
    fn facts(picks: Vec<reco_picks::Model>, trim: Vec<(&str, &str, &[&str])>) -> SharedFacts {
        facts_with(
            vec![
                ("002696", "百洋股份", vec![("2026-10-08", 10.0), ("2026-10-09", 10.0)]),
                ("600519", "贵州茅台", vec![("2026-10-08", 1.0), ("2026-10-09", 1.0)]),
            ],
            picks,
            trim,
            "2026-10-08",
            "2026-10-09",
            2,
        )
    }

    /// 一条 mid 档达标事件：09-01~09-28 共 28 个交易日、每日 +2% ⇒ 复利 ≈74% ≥ 阈 30%，
    /// 且**只有** mid 满窗（超短 2 日 4% < 10、短 5 日 10.4% < 20、长 90 日不满窗）。
    /// 用它是因为 A-3 之后智选链在超短/短档一律判 `by_design`，走不到池层与留痕层。
    fn facts_mid(picks: Vec<reco_picks::Model>, trim: Vec<(&str, &str, &[&str])>) -> SharedFacts {
        let days: Vec<String> = (1..=28).map(|d| format!("2026-09-{d:02}")).collect();
        let series: Vec<(&str, f64)> = days.iter().map(|d| (d.as_str(), 2.0)).collect();
        facts_with(
            vec![("002696", "百洋股份", series), ("600519", "贵州茅台", vec![("2026-09-28", 0.0)])],
            picks,
            trim,
            "2026-09-01",
            "2026-09-28",
            28,
        )
    }

    /// 缺陷本体（A-2 修的就是它）：**趋势智选推荐过**不能算成智能荐股也命中。
    /// 混池时这条事件在唯一一份视图里被判 `recommended`，荐股的漏检被智选的账掩盖掉。
    #[test]
    fn serenity_hit_does_not_hide_reco_miss() {
        let f = facts(
            vec![
                // 荐股链：本轮扫过别的票，池快照里没有 002696
                pick("p1", "trend", "600519", "ultra_short", Some(r#"[["600519","n"]]"#)),
                // 智选链：推荐了 002696，且它的 seed_pool_json 是**单个对象**
                pick("p2", "serenity", "002696", "ultra_short", Some(r#"{"stockCode":"002696"}"#)),
            ],
            vec![],
        );
        let reco = build_chain_view(RecoChain::Reco, &f);
        assert_eq!(reco.chain, "reco");
        assert_eq!(reco.misses.len(), 1, "荐股链必须把这条算成漏检");
        assert_eq!(reco.misses[0].layer, MissLayer::NotInPool);

        let serenity = build_chain_view(RecoChain::Serenity, &f);
        assert_eq!(serenity.chain, "serenity");
        assert!(serenity.misses.is_empty(), "智选链命中 ⇒ 不该出现在它的漏检清单里");
        assert_eq!(serenity.rates.misses, 0);
        assert!(serenity.rates.coverage.is_finite(), "命中/入池都有数 ⇒ 覆盖率该算得出来");
    }

    /// 姊妹判据：**荐股链的截断留痕不能替智选凑一个「算出却被淘汰」**。
    /// 智选工作流不写 `reco_scan_audit`（PLAN 的 B-2 才补），所以智选链这一格必须留空。
    #[test]
    fn reco_trim_evidence_is_not_borrowed_by_serenity_chain() {
        let f = facts_mid(
            vec![
                // 荐股链本轮扫的是 600519，但池快照里**有** 002696 ⇒ 入池且未出票（才走到留痕那一层）
                pick_on(
                    "p1",
                    "trend",
                    "600519",
                    "mid",
                    Some(r#"[["002696","n"],["600519","n"]]"#),
                    "2026-09-20T09:00:00",
                ),
                pick_on(
                    "p2",
                    "serenity",
                    "600519",
                    "mid",
                    Some(r#"{"stockCode":"600519"}"#),
                    "2026-09-26T09:00:00",
                ),
            ],
            vec![("mid", "002696", &["trend"][..])],
        );
        let reco = build_chain_view(RecoChain::Reco, &f);
        assert_eq!(reco.misses.len(), 1, "夹具应当只产一条 mid 事件，实得 {:?}", reco.layers);
        assert_eq!(reco.misses[0].layer, MissLayer::L3ScoredOut, "入池 + 有截断留痕 ⇒ 可归因到格");
        assert_eq!(reco.misses[0].scored_out_styles, vec!["trend".to_string()]);

        let serenity = build_chain_view(RecoChain::Serenity, &f);
        assert_eq!(serenity.misses.len(), 1, "智选链没推荐它 ⇒ 仍是智选链的漏检");
        assert!(
            serenity.misses[0].scored_out_styles.is_empty(),
            "智选链不得借荐股的留痕把自己判成 l3_scored_out（那是第二套真相）"
        );
        assert_eq!(serenity.misses[0].layer, MissLayer::NotInPool);
    }

    /// 负控：链值域写错（既不是 reco 也不是 serenity 的 style）必须归荐股链而不是静默丢弃。
    /// `of_style` 是二分判据 ⇒ 新出现的 style 名目默认落在荐股侧，漏配不会让事件凭空消失。
    #[test]
    fn unknown_style_falls_to_reco_chain() {
        let f =
            facts(vec![pick("p1", "brand_new_style", "002696", "ultra_short", Some("[]"))], vec![]);
        let reco = build_chain_view(RecoChain::Reco, &f);
        let serenity = build_chain_view(RecoChain::Serenity, &f);
        assert_eq!(reco.rates.events, 1);
        assert_eq!(serenity.rates.events, 1, "同一批达标事实在两条链各算一次，不因链而少一条");
        assert!(reco.misses.is_empty(), "未知 style 的出票算在荐股链上 ⇒ 该链命中");
        assert!(
            serenity.misses.is_empty(),
            "智选链在超短档是按设计不出票 ⇒ by_design，不再冒充一条漏检"
        );
        let by_design = serenity.layers.iter().find(|l| l.layer == MissLayer::ByDesign);
        assert!(
            by_design.is_some_and(|l| l.count == 1),
            "by_design 要留在 layers 里：{:#?}",
            serenity.layers
        );
    }

    /// A-3 ①：同档在**窗口起点之前**出过票 ⇒ 判 `late_timing`、不进漏检清单也不进补分析队列。
    /// 它此前恒不可达：`in_window` 只认窗口内的票，早先那次推荐被当成「从没推荐过」，
    /// 于是「推荐时机」的问题被报成了「候选宇宙」的问题（优化方向会去扩池）。
    #[test]
    fn pick_before_window_is_late_timing_not_a_miss() {
        let f = facts(
            vec![pick_on(
                "p0",
                "trend",
                "002696",
                "ultra_short",
                Some(r#"[["002696","n"]]"#),
                "2026-10-07T09:00:00",
            )],
            vec![],
        );
        let reco = build_chain_view(RecoChain::Reco, &f);
        assert_eq!(reco.rates.events, 1, "达标事件照旧计数（分母不因层而变）");
        assert!(reco.misses.is_empty(), "晚推荐不是漏检");
        assert_eq!(reco.rates.misses, 0);
        let late = reco.layers.iter().find(|l| l.layer == MissLayer::LateTiming);
        assert!(
            late.is_some_and(|l| l.count == 1),
            "layers 必须留下 late_timing：{:#?}",
            reco.layers
        );

        let serenity = build_chain_view(RecoChain::Serenity, &f);
        assert!(serenity.misses.is_empty(), "智选链同一条也不计漏检（它按设计不做超短档）");
    }

    /// A-3 ②：智选链的 ByDesign 用的是**链级**档位判据（矩阵 serenity 行），不是档位级。
    /// 荐股链在同一档照常出票 ⇒ 两条链对同一条事件给出不同层，正是分链要的效果。
    #[test]
    fn serenity_short_tiers_are_by_design_while_reco_still_judges_pool() {
        let f = facts(
            vec![
                pick("p1", "trend", "600519", "ultra_short", Some(r#"[["600519","n"]]"#)),
                pick("p2", "serenity", "600519", "ultra_short", Some(r#"{"stockCode":"600519"}"#)),
            ],
            vec![],
        );
        let serenity = build_chain_view(RecoChain::Serenity, &f);
        assert!(
            serenity.misses.is_empty(),
            "超短档对智选链是按设计 ⇒ 不该有漏检，实得 {:#?}",
            serenity.misses
        );
        assert!(
            serenity.layers.iter().any(|l| l.layer == MissLayer::ByDesign && l.count == 1),
            "by_design 必须成格：{:#?}",
            serenity.layers
        );
        assert!(
            !serenity.rates.coverage.is_finite(),
            "整档按设计 ⇒ 池内事件为 0，覆盖率算不出来（不得显示 0%）"
        );

        let reco = build_chain_view(RecoChain::Reco, &f);
        assert_eq!(reco.misses.len(), 1, "同一事件在荐股链仍是漏检（该链超短档照常出票）");
        assert_eq!(reco.misses[0].layer, MissLayer::NotInPool);
    }
}
