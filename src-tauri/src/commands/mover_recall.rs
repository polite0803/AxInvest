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
use axagent_analysis_engine::mover_recall::{self, Evidence, MissLayer, MoverEvent};
use axagent_dao::DatabaseConnection;
use axagent_dao::repo::mover_recall_runs::{self, RunEventRecord, RunSnapshotRecord};
use axagent_dao::repo::{stock_analysis_snapshot, stock_lesson_queries};
use serde::Serialize;

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;

/// 妖股漏检核查定时任务的 task_type（前端任务中心白名单与本常量逐字对齐）。
pub use crate::commands::constants::MOVER_RECALL_TASK_TYPE;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverRecallView {
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
    /// 漏检事件明细（按 |累计涨幅| 降序，上限 200 条，完整集在 layers 计数里）
    pub misses: Vec<MissRow>,
}

#[derive(Debug, Clone, Serialize)]
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
) -> Result<MoverRecallView, String> {
    let mut view = analyze_mover_recall_inner(state.harness.db(), to).await?;
    // 面板只展示 top 200（inner 保留全集供调度侧落 mover_recall_events）
    view.misses.truncate(200);
    Ok(view)
}

/// 核查实现：命令与 `mover-recall` cron 共用同一套逻辑
/// （同 `run_decision_backtest_inner` 的形态——cron 拿不到 `tauri::State`）。
pub async fn analyze_mover_recall_inner(
    db: &DatabaseConnection,
    to: Option<String>,
) -> Result<MoverRecallView, String> {
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

    let mut layers: BTreeMap<&'static str, (MissLayer, usize, BTreeMap<String, usize>)> =
        BTreeMap::new();
    let mut misses: Vec<MissRow> = Vec::new();
    let mut events_total = 0usize;

    for rule in &rules {
        let period_ok = mover_recall::period_has_active_style(rule.period);
        let evs = mover_recall::events_for_tier(&by_stock, rule, &from, &upper);
        for mut ev in evs {
            events_total += 1;
            // window_start 由 events_for_tier 在识别事件时一并算定（同一窗口切片的首日），
            // 此处不得再按 close_dates 重算一份 —— 两份日期序列同源但各算一遍会制造
            // 「pick 过滤窗口」与「事件窗口」漂移的可能（单一来源，见禁区 12）。
            let window_start = ev.window_start.clone();
            let in_window: Vec<_> = picks
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
            let picked_any_period =
                in_window.iter().any(|p| p.stock_code == ev.stock_code && p.period == ev.period)
                    || in_window.iter().any(|p| p.stock_code == ev.stock_code);
            let in_pool = in_window.iter().any(|p| {
                mover_recall::code_in_pool_snapshot(p.seed_pool_json.as_deref(), &ev.stock_code)
            });
            // 本轮池快照为空 ⇒ 降级（判据绑定本轮，不用别轮的值兜）
            let pool_degraded = !in_window.is_empty()
                && in_window.iter().any(|p| match p.seed_pool_json.as_deref() {
                    None => true,
                    Some(s) => s.trim() == "[]" || s.trim().is_empty(),
                });

            let scored_out = trim_evidence
                .get(&(ev.period.clone(), ev.stock_code.clone()))
                .cloned()
                .unwrap_or_default();

            let evidence = Evidence {
                picks_for_period: picks_same_period,
                picked_in_other_period: picked_any_period
                    && in_window
                        .iter()
                        .any(|p| p.stock_code == ev.stock_code && p.period != ev.period),
                in_pool,
                pool_degraded,
                matrix_active: period_ok,
                l3_scored_out_styles: scored_out.clone(),
            };
            let layer = mover_recall::attribute(true, &evidence);
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

    Ok(MoverRecallView {
        from,
        to: upper,
        data_since: close_dates.first().cloned().unwrap_or_default(),
        collected_days,
        universe_size,
        universe_confirmed,
        rules: rules.clone(),
        rates,
        layers: layer_rows,
        misses,
    })
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
    let rules_json = serde_json::to_string(&view.rules).map_err(|e| e.to_string())?;
    let rates_json = serde_json::to_string(&view.rates).map_err(|e| e.to_string())?;
    let layers_json = serde_json::to_string(&view.layers).map_err(|e| e.to_string())?;
    let misses_json = if view.misses.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&view.misses).map_err(|e| e.to_string())?)
    };

    let run_row = RunSnapshotRecord {
        id: id.clone(),
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

    mover_recall_runs::insert_run_snapshot(db, &run_row, &event_rows)
        .await
        .map_err(|e| format!("mover_recall 核查落库失败: {e}"))?;
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
) -> Result<Option<MoverRecallEventReview>, String> {
    let db = state.harness.db();
    let anchor = date_of(&anchor_date);

    // 同三键可能被多轮核查重复落（每轮全量替换各自 run 的行）⇒ dao 取最新一轮。
    let event = match axagent_dao::repo::mover_recall_events::find_latest_by_key(
        db,
        &stock_code,
        anchor,
        &period,
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
