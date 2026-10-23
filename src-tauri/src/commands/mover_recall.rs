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
    /// 行情源申报的全市场总数（0 = 未取到）—— 全 A 股样本的分母。
    /// 与 `universe_size`（已枚举）和 `codes_with_close`（有收盘）是三个计数域，面板分别标注。
    pub reported_total: i64,
    pub rules: Vec<mover_recall::TierRule>,
    pub rates: mover_recall::RecallRates,
    pub layers: Vec<mover_recall::LayerRow>,
    /// 逐档达标事件数（`period -> events`）：C-2 的三态依据 —— 本档已满 + 0 事件 =「真无达标票」，
    /// 必须能与「本档有达标」分开说。`None` = 这条载荷没记录过（旧轮次）⇒ 面板第三态，不显示 0。
    pub events_by_tier: Option<BTreeMap<String, usize>>,
    /// 有收盘行的票数 —— 事件实际可来自的票数。`None` = 旧轮次未记录。
    /// 与 `universe_size`（清单规模）是两个域，面板必须分开显示。
    pub codes_with_close: Option<usize>,
    /// 逐档可判票数（该票历史长度 ≥ 该档窗口天数）。`None` = 旧轮次未记录。
    pub judgeable_codes_by_tier: Option<BTreeMap<String, usize>>,
    /// 归因层口径版本。`None` = 旧轮次未记录（跨版本漏检数不可直接比较）。
    pub gate_version: Option<String>,
    /// 漏检事件明细（按 |累计涨幅| 降序，上限 200 条，完整集在 layers 计数里）
    pub misses: Vec<MissRow>,
}

/// 归因层口径版本：层枚举/阈值语义变化时递增，落进轮次快照供面板标注可比性。
pub const MOVER_GATE_VERSION: &str = "mover-layers-v2";

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

/// 窗口起点前的「已覆盖」回看范围（日历日）：交易日的 7/5 折算 + 3 天缓冲。
fn prior_cover_calendar_days(window_days: u32) -> i64 {
    (window_days as i64 * 7 + 4) / 5 + 3
}

/// 该推荐是否落在「窗口起点前一个窗口长度内」。
///
/// 旧实现只看 `< window_start`、没有下界：一年前推荐过也永久豁免本窗漏检。
/// 日期解析失败返回 false（不豁免）——「读不出日期」不能静默变成「已覆盖」。
fn prior_cover_within(pick_date: &str, window_start: &str, window_days: u32) -> bool {
    let Ok(start) = chrono::NaiveDate::parse_from_str(window_start, "%Y-%m-%d") else {
        return false;
    };
    let Ok(pick) = chrono::NaiveDate::parse_from_str(pick_date, "%Y-%m-%d") else {
        return false;
    };
    pick < start && pick >= start - chrono::Duration::days(prior_cover_calendar_days(window_days))
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
    /// 行情源申报的全市场总数（0 = 未取到）
    reported_total: i64,
    rules: Vec<mover_recall::TierRule>,
    by_stock: mover_recall::CloseSeriesByCode,
    /// 全局收盘日期序列（升序）—— 逐档窗口起点按它反推，避免每档各算一套日期。
    close_dates: Vec<String>,
    picks: Vec<reco_picks::Model>,
    trim_evidence: HashMap<(String, String), Vec<String>>,
    /// 批次级候选池快照（含 0 票批次）；旧数据为空时回退到 pick 行上的 `seed_pool_json`。
    pool_batches: Vec<PoolBatchView>,
}

#[derive(Debug, Clone)]
struct PoolBatchView {
    chain: String,
    generated_at: String,
    seed_pool_json: Option<String>,
    degraded: bool,
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
    let to = to.unwrap_or_else(|| close_dates.last().cloned().unwrap_or_default());
    let upper = if to.as_str() > close_dates.last().map(String::as_str).unwrap_or("") {
        close_dates.last().cloned().unwrap_or_default()
    } else {
        to.clone()
    };
    // 采集日数只数到 `upper`（事件锚点不可能晚于它）——某档可判定 ⇔ 本值 ≥ window_days
    let collected_days = close_dates.iter().take_while(|d| d.as_str() <= upper.as_str()).count();

    // 取数与「数据起点」都只回看到最长档窗口起点：库里有十年历史也不该把核查拖成十年。
    let max_window = rules.iter().map(|r| r.window_days).max().unwrap_or(1) as usize;
    let from = close_dates
        .get(collected_days.saturating_sub(max_window))
        .cloned()
        .unwrap_or_else(|| close_dates.first().cloned().unwrap_or_default());
    let by_stock = mover_recall::load_closes_by_stock(db, &from, &upper).await?;
    let picks = mover_recall::load_picks_in_range(db, &from, &upper, None).await?;
    let trim_evidence = mover_recall::load_trim_evidence(db, &from, &upper, None).await?;
    let mut pool_batches = Vec::new();
    for chain in RecoChain::ALL {
        let rows = axagent_dao::repo::reco_pool::load_pool_batches(
            db,
            chain.as_str(),
            &from,
            &upper,
            None,
        )
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("读候选池批次失败: {e}"))
                .to_string()
        })?;
        pool_batches.extend(rows.into_iter().map(|b| PoolBatchView {
            chain: b.chain,
            generated_at: b.generated_at,
            seed_pool_json: b.seed_pool_json,
            degraded: b.degraded,
        }));
    }
    let (universe_size, universe_confirmed) = mover_recall::load_universe_counts(db).await?;
    // 全 A 股分母：行情源申报总数（`market_universe_stats`）。取不到 ⇒ 0（面板显示未取到）。
    let reported_total =
        axagent_analysis_engine::market_close_store::latest_reported_total(db).await.unwrap_or(0);

    let facts = SharedFacts {
        data_since: from.clone(),
        from,
        to: upper,
        collected_days,
        universe_size,
        universe_confirmed,
        reported_total,
        rules,
        by_stock,
        close_dates,
        picks,
        trim_evidence,
        pool_batches,
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
    // synthetic=1 是降级占位行（watchlist 初筛、capital K 线代理等），不是策略主张：
    // 只用于「池覆盖」证据，不能算「已推荐」——否则漏检被系统性低估。
    // 与 `load_mover_cell_signal` 的 hit 口径（只认 synthetic==0）保持一致。
    let hit_picks: Vec<&reco_picks::Model> =
        chain_picks.iter().copied().filter(|p| p.synthetic == 0).collect();

    let mut layers: BTreeMap<&'static str, (MissLayer, usize, BTreeMap<String, usize>)> =
        BTreeMap::new();
    let mut misses: Vec<MissRow> = Vec::new();
    let mut events_total = 0usize;
    let mut events_by_tier: BTreeMap<String, usize> = BTreeMap::new();

    let mut judgeable_starts: Vec<String> = Vec::new();
    for rule in &f.rules {
        let tier_active = mover_recall::chain_period_has_active_style(chain, rule.period);
        // 逐档只看**最近一个窗口**：窗口长度 = 本档 holding days，右端 = 最新收盘日。
        // 旧实现从最早采集日滚到最新 ⇒ 同一只票在每个滚动窗口各算一次（62 条「超短漏检」
        // 里有 38 只票反复出现），而且把「历史所有窗口」当成「本次漏检」。
        let tier_start = f
            .collected_days
            .checked_sub(rule.window_days as usize)
            .and_then(|i| f.close_dates.get(i))
            .cloned();
        if let Some(start) = &tier_start {
            judgeable_starts.push(start.clone());
        }
        let evs: Vec<_> =
            mover_recall::events_for_tier(&f.by_stock, rule, &f.from, &f.to, &f.close_dates)
                .into_iter()
                .filter(|ev| ev.anchor_date == f.to)
                .collect();
        for mut ev in evs {
            events_total += 1;
            *events_by_tier.entry(rule.period.as_str().to_string()).or_default() += 1;
            // window_start 由 events_for_tier 在识别事件时一并算定（同一窗口切片的首日），
            // 此处不得再按 close_dates 重算一份 —— 两份日期序列同源但各算一遍会制造
            // 「pick 过滤窗口」与「事件窗口」漂移的可能（单一来源，见禁区 12）。
            let window_start = ev.window_start.clone();
            let in_window_hits: Vec<_> = hit_picks
                .iter()
                .filter(|p| {
                    let d = date_of(&p.generated_at);
                    d >= window_start.as_str() && d <= ev.anchor_date.as_str()
                })
                .collect();
            let in_window_pool: Vec<_> = chain_picks
                .iter()
                .filter(|p| {
                    let d = date_of(&p.generated_at);
                    d >= window_start.as_str() && d <= ev.anchor_date.as_str()
                })
                .collect();

            let picks_same_period: Vec<String> = in_window_hits
                .iter()
                .filter(|p| p.stock_code == ev.stock_code && p.period == ev.period)
                .map(|p| p.generated_at.clone())
                .collect();
            ev.recommended = !picks_same_period.is_empty();
            // 同链内「该链在这只票的任一档出过票」——不再重复 `period` 相等那一支（被本条件包含）
            let picked_any_period = in_window_hits.iter().any(|p| p.stock_code == ev.stock_code);
            // 阶段 5 / G1：本窗没有**该链**的批次 ⇒ 是「不知道」，不是「不在池里」。
            // 这一格直接决定事件落 `not_in_pool` 还是 `unexplained`，也就是把优化方向
            // 带去「扩候选宇宙」还是「补留痕」。
            // 批次级池快照优先；旧轮次没有批次行时回退到 pick 行上的快照。
            let window_batches: Vec<&PoolBatchView> = f
                .pool_batches
                .iter()
                .filter(|b| {
                    b.chain == chain.as_str() && {
                        let d = date_of(&b.generated_at);
                        d >= window_start.as_str() && d <= ev.anchor_date.as_str()
                    }
                })
                .collect();
            let in_pool = if !window_batches.is_empty() {
                Some(window_batches.iter().any(|b| {
                    mover_recall::code_in_pool_snapshot(b.seed_pool_json.as_deref(), &ev.stock_code)
                }))
            } else if in_window_pool.is_empty() {
                None
            } else {
                Some(in_window_pool.iter().any(|p| {
                    mover_recall::code_in_pool_snapshot(p.seed_pool_json.as_deref(), &ev.stock_code)
                }))
            };
            // A-3 `PriorCovered` 的证据：本链在同档、**窗口起点之前一个窗口长度内**出过票
            // ⇒ 链看见过这只票，只是时点不落在本窗。不算漏检（`counts_as_miss` 已排除），
            // 但必须单独成层，否则会被并进 `not_in_pool`，把「推荐时机」报成「候选宇宙」。
            let picked_before_window = hit_picks.iter().any(|p| {
                p.stock_code == ev.stock_code
                    && p.period == ev.period
                    && prior_cover_within(date_of(&p.generated_at), &window_start, ev.window_days)
            });

            // 本窗该链**所有**池快照都为空才判降级；只有一个空批次时，
            // 正常批次里的池证据仍然有效（旧的 any 会把整个事件误判成 excluded）。
            let pool_degraded = if !window_batches.is_empty() {
                window_batches.iter().all(|b| b.degraded)
            } else {
                !in_window_pool.is_empty()
                    && in_window_pool.iter().all(|p| match p.seed_pool_json.as_deref() {
                        None => true,
                        Some(s) => s.trim() == "[]" || s.trim().is_empty(),
                    })
            };

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
                    && in_window_hits
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
    // 事件域的票数 = 有收盘行的票；逐档可判票数按**全局日历上的覆盖率**算，
    // 不能只数「该票自己的行数」（稀疏历史会把 28 行横跨十几年的票算成可判）。
    let codes_with_close = f.by_stock.len();
    let judgeable_codes_by_tier: BTreeMap<String, usize> = f
        .rules
        .iter()
        .map(|rule| {
            let n = f
                .by_stock
                .values()
                .filter(|(_, series)| {
                    mover_recall::has_judgeable_window(series, &f.close_dates, rule.window_days)
                })
                .count();
            (rule.period.as_str().to_string(), n)
        })
        .collect();
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

    // 区间 = 本轮**可判档**里最早的窗口起点（不是最早采集日）：长档未满时不该把
    // 「一个月前」写成本轮口径。没有可判档 ⇒ 退化成 to 本身。
    let view_from = judgeable_starts.into_iter().min().unwrap_or_else(|| f.to.clone());
    MoverRecallView {
        chain: chain.as_str().to_string(),
        from: view_from,
        to: f.to.clone(),
        data_since: f.data_since.clone(),
        collected_days: f.collected_days,
        universe_size: f.universe_size,
        universe_confirmed: f.universe_confirmed,
        reported_total: f.reported_total,
        rules: f.rules.clone(),
        rates,
        layers: layer_rows,
        events_by_tier: Some(events_by_tier),
        codes_with_close: Some(codes_with_close),
        judgeable_codes_by_tier: Some(judgeable_codes_by_tier),
        gate_version: Some(MOVER_GATE_VERSION.to_string()),
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
    let judgeable_codes_by_tier_json = view
        .judgeable_codes_by_tier
        .as_ref()
        .map(|m| {
            serde_json::to_string(m).map_err(|e| {
                ErrorResponse::err_with_detail(
                    wf_err::SERIALIZE_FAILED,
                    format!("mover_recall_runs.judgeable_codes_by_tier_json 序列化失败: {e}"),
                )
            })
        })
        .transpose()?;

    let run_row = RunSnapshotRecord {
        id: id.clone(),
        chain: Some(view.chain.clone()),
        date_from: view.from.clone(),
        date_to: view.to.clone(),
        data_since: view.data_since.clone(),
        collected_days: view.collected_days as i32,
        universe_size: view.universe_size,
        universe_confirmed: view.universe_confirmed,
        reported_total: Some(view.reported_total),
        rules_json,
        rates_json,
        layers_json,
        events_total: view.rates.events as i32,
        miss_count: view.rates.misses as i32,
        misses_json,
        events_by_tier_json,
        codes_with_close: view.codes_with_close.map(|n| n as i32),
        judgeable_codes_by_tier_json,
        gate_version: view.gate_version.clone(),
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
    /// 行情源申报的全市场总数；0 = 旧轮次未记录（面板显示未记录，不拿枚举数冒充）
    pub reported_total: i64,
    pub events_total: i32,
    pub miss_count: i32,
    /// 有收盘行的票数；`None` = 旧轮次未记录
    pub codes_with_close: Option<i32>,
    /// 归因层口径版本；`None` = 旧轮次未记录
    pub gate_version: Option<String>,
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
        reported_total: run.reported_total.unwrap_or(0),
        events_total: run.events_total,
        miss_count: run.miss_count,
        codes_with_close: run.codes_with_close,
        gate_version: run.gate_version.clone(),
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
        reported_total: run.reported_total.unwrap_or(0),
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
        codes_with_close: run.codes_with_close.map(|n| n.max(0) as usize),
        judgeable_codes_by_tier: match run.judgeable_codes_by_tier_json.as_deref() {
            None => None,
            Some(raw) => Some(serde_json::from_str(raw).map_err(|e| {
                run_read_err(format!("judgeable_codes_by_tier_json 解析失败: {e}"))
            })?),
        },
        gate_version: run.gate_version.clone(),
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

/// 回灌清单条目（D-4）：一条「反思判定本应入池」的漏检票。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverReflowCandidate {
    pub event_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub period: String,
    pub anchor_date: String,
    /// 反思给出的理由（`reflow_note`；旧行可能为空）
    pub note: Option<String>,
}

/// 回灌清单 + 转正就绪度。
///
/// `ready = settled >= required`（fail-closed 的下限）；**三态由面板成句**：
/// 无候选 / 有候选但样本不足（shadow）/ 可转正（仍需人工确认才并入种子）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverReflowView {
    pub items: Vec<MoverReflowCandidate>,
    pub settled: usize,
    pub required: usize,
    pub ready: bool,
}

/// 读回灌清单（只读；不改状态）。
#[agent_command(
    domain = "finance",
    safety = Safe,
    call_mode = StateInput,
    description = "读取漏检回灌清单与转正就绪度"
)]
#[tauri::command]
pub async fn list_mover_reflow_candidates(
    state: tauri::State<'_, AppState>,
) -> Result<MoverReflowView, String> {
    let db = state.harness.db();
    let rows = axagent_dao::repo::mover_recall_events::list_reflow_candidates(db, 100)
        .await
        .map_err(|e| run_read_err(format!("读回灌清单失败: {e}")))?;
    let codes: Vec<String> = rows.iter().map(|r| r.stock_code.clone()).collect();
    let settled = axagent_dao::repo::mover_recall_events::count_settled_reflow(db, &codes)
        .await
        .map_err(|e| run_read_err(format!("统计已结算样本失败: {e}")))?;
    Ok(MoverReflowView {
        items: rows
            .into_iter()
            .map(|r| MoverReflowCandidate {
                event_id: r.id,
                stock_code: r.stock_code,
                stock_name: r.stock_name,
                period: r.period,
                anchor_date: r.anchor_date,
                note: r.reflow_note,
            })
            .collect(),
        settled,
        required: axagent_analysis_engine::mover_recall::REFLOW_MIN_SETTLED,
        ready: axagent_analysis_engine::mover_recall::reflow_ready(settled),
    })
}
/// 自动窗口的数据前提：本地收盘历史不足最长档（90 个交易日）时，用日线补齐一次。
///
/// 只在**不足**时跑（首轮约 680 次日线请求，之后每天有快照就不再触发）；失败不阻断核查 ——
/// 达不到的长档会如实显示「窗口未满」，不伪装成「无漏检」。
pub(crate) async fn ensure_close_history(
    db: &DatabaseConnection,
    client: &axagent_astock_data::AStockClient,
) {
    use std::sync::atomic::{AtomicI64, Ordering};
    const LONGEST_WINDOW: u32 = 90;
    const DAILY_BACKFILL_LIMIT: usize = 200;
    // 每天只补一批（进程内闩；重启会再补一批，幂等）。不放核查前同步跑：
    // 2500+ 只逐票请求会把行情通道打爆，也会把面板刷新拖成分钟级。
    static LAST_DAY: AtomicI64 = AtomicI64::new(0);
    let day: i64 = chrono::Local::now().format("%Y%m%d").to_string().parse().unwrap_or(0);
    if day > 0 && LAST_DAY.swap(day, Ordering::Relaxed) == day {
        return;
    }
    let codes = match axagent_analysis_engine::market_close_store::codes_needing_history(
        db,
        LONGEST_WINDOW,
        DAILY_BACKFILL_LIMIT,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[mover-recall] 逐票历史缺口查询失败: {e}");
            return;
        },
    };
    if codes.is_empty() {
        return;
    }
    tracing::info!(
        "[mover-recall] 逐票历史回补：本批 {} 只（目标 {LONGEST_WINDOW} 根）",
        codes.len()
    );
    if let Err(e) = axagent_analysis_engine::market_close_store::backfill_close_history_for_codes(
        db,
        client,
        &codes,
        LONGEST_WINDOW,
        120,
    )
    .await
    {
        tracing::warn!("[mover-recall] 逐票历史回补失败（下一批继续）: {e}");
    }
}

/// 一轮核查的结果（供调用方决定「有没有漏检要不要通知」+ 任务摘要）。
pub(crate) struct MoverCheckOutcome {
    pub summary: String,
    pub misses: usize,
}

/// 手动与 cron 共用的核查入口：算两链视图 → 逐链落库 → 拼任务摘要。
///
/// 抽出来的理由（2026-10-21 实机反馈）：此前「立即执行」只调用调度器的 `trigger_now`
/// 并立刻返回「已触发」，用户点完看不到任何结果。核查本身是**快**任务（一次库内扫描），
/// 手动路径应当直接等它跑完并把摘要返回。
/// `trigger_source` 由调用方给：手动 `manual` / 定时 `scheduler`（F-15 的区分）。
/// 四档的展示名目（顺序固定；判据本身来自 `view.rules`，这里只管称呼）。
const TIER_LABELS: [(&str, &str); 4] =
    [("ultra_short", "超短期"), ("short", "短期"), ("mid", "中期"), ("long", "长期")];

/// 每档最多列多少只（任务摘要不是导出表；剩下的用「等 N 只」交代）。
const SUMMARY_PER_TIER: usize = 8;

/// 单链摘要（纯函数）：**先列票，再说处置**。
///
/// 用户口径（2026-10-21）：「超短期：某某股上涨** / … / 以上股票被漏检，开始对它们进行反思」。
/// 因此这里不做率的陈述，而是逐档给出**被漏检的票 + 累计涨幅**；三类别混淆必须分开：
///   ① 窗口未满 ⇒ 「尚不可判定」；
///   ② 本链该档按设计不做 ⇒ 「不适用，不是漏检」；
///   ③ 有可判事件且确实没有漏检 ⇒ 「无漏检」。
/// 率算不出来不进摘要（分母 0 时印 `NaN` 只会制造第二个歧义）。
fn chain_summary_lines(view: &MoverRecallView) -> Vec<String> {
    let chain = if view.chain == "serenity" {
        RecoChain::Serenity
    } else {
        RecoChain::Reco
    };
    let mut body: Vec<String> = Vec::new();
    let mut judgeable_tiers = 0usize;
    for (key, label) in TIER_LABELS {
        let Some(rule) = view.rules.iter().find(|r| r.period.as_str() == key) else {
            body.push(format!("{label}：本档判据缺失（阈值变量无效）"));
            continue;
        };
        if view.collected_days < rule.window_days as usize {
            body.push(format!(
                "{label}：窗口未满（已采 {}/需 {} 个交易日）⇒ 尚不可判定",
                view.collected_days, rule.window_days
            ));
            continue;
        }
        if !mover_recall::chain_period_has_active_style(chain, rule.period) {
            body.push(format!("{label}：按设计不做此档 ⇒ 不适用，不是漏检"));
            continue;
        }
        judgeable_tiers += 1;
        let mut rows: Vec<&MissRow> =
            view.misses.iter().filter(|m| m.event.period == key).collect();
        rows.sort_by(|a, b| b.event.cum_gain_pct.total_cmp(&a.event.cum_gain_pct));
        // 同一只票在同一档可能有多条窗口事件（不同 anchor）⇒ 清单按票去重，保留涨幅最大的那条；
        // 否则同一行里会出现「某某 +21.0%；某某 +20.3%」这种看起来像两只票的重复。
        let mut seen_codes = std::collections::HashSet::new();
        rows.retain(|m| seen_codes.insert(m.event.stock_code.clone()));
        if rows.is_empty() {
            body.push(format!("{label}：无漏检"));
            continue;
        }
        let listed: Vec<String> = rows
            .iter()
            .take(SUMMARY_PER_TIER)
            .map(|m| {
                format!(
                    "{} {} {:+.1}%",
                    m.event.stock_code, m.event.stock_name, m.event.cum_gain_pct
                )
            })
            .collect();
        let more = if rows.len() > SUMMARY_PER_TIER {
            format!("（等共 {} 只）", rows.len())
        } else {
            String::new()
        };
        body.push(format!("{label}：{}{more}", listed.join("；")));
    }
    if view.rates.misses > 0 {
        let mut codes = std::collections::HashSet::new();
        for m in &view.misses {
            codes.insert(m.event.stock_code.as_str());
        }
        body.push(format!(
            "以上 {} 条漏检事件（涉及 {} 只票）：已写入待补分析队列（每日 06:00 补分析后进入反思），             下一轮智能荐股会把它们作为候选重新评估",
            view.rates.misses,
            codes.len()
        ));
    } else if judgeable_tiers == 0 {
        body.push("本链本窗没有可判定档位 ⇒ 不适用，不是「零漏检」".to_string());
    }
    // 表头也不许印误导性的 0：没有可判档 / 没有达标事件时，漏检数用「—」并注明原因。
    let miss_cell = if judgeable_tiers == 0 {
        "—（无可判定档）".to_string()
    } else if view.rates.events == 0 {
        "—（本窗无达标事件）".to_string()
    } else {
        view.rates.misses.to_string()
    };
    let mut out = vec![format!(
        "[{}] 区间 {}~{} 达标={} 漏检={miss_cell}",
        view.chain, view.from, view.to, view.rates.events
    )];
    out.extend(body);
    out
}

pub(crate) async fn run_mover_recall_check(
    db: &DatabaseConnection,
    _client: &axagent_astock_data::AStockClient,
    trigger_source: &str,
) -> Result<MoverCheckOutcome, String> {
    let views = analyze_mover_recall_inner(db, None).await?;
    let mut lines: Vec<String> = Vec::new();
    let mut misses = 0usize;
    for view in &views {
        misses += view.rates.misses;
        // 快照落库失败不把整轮判失败（结果已算出），只在那一行留痕
        let persist_msg = match persist_mover_recall_run(db, view, trigger_source).await {
            Ok(run_id) => format!("run_id={run_id}"),
            Err(e) => format!("快照落库失败: {}", crate::commands::error::error_text_for_prose(&e)),
        };
        for line in chain_summary_lines(view) {
            lines.push(line);
        }
        if let Some(last) = lines.last_mut() {
            last.push_str(&format!(" | {persist_msg}"));
        }
    }
    if views.len() > 1 {
        lines.push(
            "注：同一达标事件在智能荐股/趋势智选两条链各计一次；唯一事件数见面板（不是两段相加）"
                .to_string(),
        );
    }
    Ok(MoverCheckOutcome {
        summary: lines.join(
            "
",
        ),
        misses,
    })
}
#[cfg(test)]
mod summary_line_tests {
    use super::*;
    use axagent_analysis_engine::mover_recall::{LayerRow, MissLayer, MoverEvent, TierRule};
    use axagent_harness::holding_period::Period;

    fn rule(period: Period, gain: f64, days: u32) -> TierRule {
        TierRule { period, var_name: "mover_gain_x", gain_pct: gain, window_days: days }
    }

    fn view(
        chain: &str,
        collected_days: usize,
        by_design: usize,
        misses: Vec<(&str, &str, f64, Period)>,
    ) -> MoverRecallView {
        let miss_rows: Vec<MissRow> = misses
            .into_iter()
            .map(|(code, name, gain, period)| MissRow {
                event: MoverEvent {
                    stock_code: code.to_string(),
                    stock_name: name.to_string(),
                    period: period.as_str().to_string(),
                    anchor_date: "2026-10-20".to_string(),
                    window_start: "2026-10-19".to_string(),
                    window_days: period.default_holding_days(),
                    cum_gain_pct: gain,
                    max_daily_pct: 10.0,
                    threshold_pct: 10.0,
                    market_type: "main_sz".to_string(),
                    recommended: false,
                },
                layer: MissLayer::NotInPool,
                picked_any_period: false,
                scored_out_styles: vec![],
            })
            .collect();
        MoverRecallView {
            chain: chain.to_string(),
            from: "2026-09-30".to_string(),
            to: "2026-10-20".to_string(),
            data_since: "2026-09-30".to_string(),
            collected_days,
            universe_size: 681,
            universe_confirmed: 595,
            reported_total: 5925,
            rules: vec![
                rule(Period::UltraShort, 10.0, 2),
                rule(Period::Short, 20.0, 5),
                rule(Period::Mid, 30.0, 28),
                rule(Period::Long, 40.0, 90),
            ],
            rates: axagent_analysis_engine::mover_recall::RecallRates {
                reachability: f64::NAN,
                coverage: f64::NAN,
                unexplained_share: f64::NAN,
                events: 77,
                misses: miss_rows.len(),
                judgeable_events: Some(77),
            },
            layers: if by_design == 0 {
                vec![]
            } else {
                vec![LayerRow {
                    layer: MissLayer::ByDesign,
                    count: by_design,
                    by_market_type: vec![],
                }]
            },
            events_by_tier: None,
            codes_with_close: None,
            judgeable_codes_by_tier: None,
            gate_version: None,
            misses: miss_rows,
        }
    }

    /// 用户口径：逐档列票 + 涨幅；智选「整链按设计不做」必须说「不适用」而不是「漏检=0」。
    #[test]
    fn summary_lists_missed_stocks_per_tier() {
        let lines = chain_summary_lines(&view(
            "reco",
            9,
            0,
            vec![
                ("002696", "百洋股份", 21.3, Period::UltraShort),
                ("300750", "宁德时代", 12.5, Period::UltraShort),
            ],
        ));
        let text = lines.join(
            "
",
        );
        assert!(text.contains("超短期：002696 百洋股份 +21.3%；300750 宁德时代 +12.5%"), "{text}");
        assert!(text.contains("以上 2 条漏检事件（涉及 2 只票）"), "{text}");
        // 未满档必须说「尚不可判定」，不能混成「无漏检」
        assert!(text.contains("中期：窗口未满"), "{text}");
        assert!(!text.contains("NaN"), "摘要里不得出现 NaN：{text}");

        // 智选：短/超短按设计不做，中/长窗口未满 ⇒ 不得出现「漏检=0」或「无漏检」
        let ser = chain_summary_lines(&view("serenity", 9, 77, vec![])).join(
            "
",
        );
        assert!(ser.contains("超短期：按设计不做此档"), "{ser}");
        assert!(ser.contains("短期：按设计不做此档"), "{ser}");
        assert!(ser.contains("没有可判定档位"), "{ser}");
        assert!(!ser.contains("漏检=0"), "{ser}");
        assert!(!ser.contains("无漏检"), "{ser}");
    }

    /// 真·零漏检（有可判事件、判据成立、没有漏检）才允许写「无漏检」。
    #[test]
    fn real_zero_miss_says_no_miss() {
        let mut v = view("reco", 9, 0, vec![]);
        v.rules = vec![rule(Period::UltraShort, 10.0, 2)];
        let text = chain_summary_lines(&v).join(
            "
",
        );
        assert!(text.contains("超短期：无漏检"), "{text}");
        assert!(!text.contains("不适用"), "{text}");
    }
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
        _collected_days: usize,
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
        // 全局交易日历 = 夹具收盘序列的日期并集（`events_for_tier` 用它反推窗口起点；
        // 旧的「凑够 collected_days 个假日期」会让锚点不在日历里 ⇒ 事件恒空）。
        let mut close_dates: Vec<String> = by_stock
            .values()
            .flat_map(|(_, series)| series.iter().map(|(d, _)| d.clone()))
            .collect();
        close_dates.sort();
        close_dates.dedup();
        let collected_days = close_dates.len();
        SharedFacts {
            from: from.to_string(),
            to: to.to_string(),
            data_since: from.to_string(),
            collected_days,
            universe_size: 2,
            universe_confirmed: 2,
            reported_total: 0,
            rules: mover_recall::tier_rules_from_vars(&[]),
            by_stock,
            close_dates,
            picks,
            trim_evidence,
            pool_batches: Vec::new(),
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

    /// A-3 ①：同档在**窗口起点之前**出过票 ⇒ 判 `prior_covered`、不进漏检清单也不进补分析队列。
    /// 它此前恒不可达：`in_window` 只认窗口内的票，早先那次推荐被当成「从没推荐过」，
    /// 于是「推荐时机」的问题被报成了「候选宇宙」的问题（优化方向会去扩池）。
    #[test]
    fn pick_before_window_is_prior_covered_not_a_miss() {
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
        assert!(reco.misses.is_empty(), "窗口前已覆盖不是漏检");
        assert_eq!(reco.rates.misses, 0);
        let late = reco.layers.iter().find(|l| l.layer == MissLayer::PriorCovered);
        assert!(
            late.is_some_and(|l| l.count == 1),
            "layers 必须留下 prior_covered：{:#?}",
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

    /// synthetic=1 是降级占位行：只能当池覆盖证据，不能算「已推荐」。
    #[test]
    fn synthetic_pick_is_pool_evidence_not_hit() {
        let mut p = pick("p1", "trend", "002696", "ultra_short", Some(r#"[["002696","n"]]"#));
        p.synthetic = 1;
        let f = facts(vec![p], vec![]);
        let reco = build_chain_view(RecoChain::Reco, &f);
        assert_eq!(reco.misses.len(), 1, "降级占位行不能把漏检洗成命中");
        assert_eq!(reco.misses[0].layer, MissLayer::Unexplained);
    }

    /// 窗口前「已覆盖」必须有回看下界：一年前推荐过不能永久豁免本窗漏检。
    #[test]
    fn prior_covered_has_a_lookback_bound() {
        let f = facts(
            vec![pick_on(
                "p1",
                "trend",
                "002696",
                "ultra_short",
                Some(r#"[["002696","n"]]"#),
                "2026-08-01T09:00:00",
            )],
            vec![],
        );
        let reco = build_chain_view(RecoChain::Reco, &f);
        assert!(
            reco.layers.iter().all(|l| l.layer != MissLayer::PriorCovered),
            "窗口前很久的推荐不得判成已覆盖：{:#?}",
            reco.layers
        );
        assert_eq!(reco.misses.len(), 1);
    }

    /// 0 票批次也要留痕：没有 pick 行时，空池批次必须能判成 `pool_degraded`。
    #[test]
    fn empty_pool_batch_is_degraded_without_pick_rows() {
        let mut f = facts(vec![], vec![]);
        f.pool_batches.push(PoolBatchView {
            chain: "reco".to_string(),
            generated_at: "2026-10-08T09:00:00".to_string(),
            seed_pool_json: Some("[]".to_string()),
            degraded: true,
        });
        let reco = build_chain_view(RecoChain::Reco, &f);
        assert_eq!(reco.misses.len(), 1);
        assert_eq!(reco.misses[0].layer, MissLayer::PoolDegraded);
    }

    /// 有批次行、池里没有该票 ⇒ 可归因到 `not_in_pool`（不再因「无 pick 行」落 unexplained）。
    #[test]
    fn normal_pool_batch_marks_not_in_pool_without_pick_rows() {
        let mut f = facts(vec![], vec![]);
        f.pool_batches.push(PoolBatchView {
            chain: "reco".to_string(),
            generated_at: "2026-10-08T09:00:00".to_string(),
            seed_pool_json: Some(r#"[["600519","n"]]"#.to_string()),
            degraded: false,
        });
        let reco = build_chain_view(RecoChain::Reco, &f);
        assert_eq!(reco.misses.len(), 1);
        assert_eq!(reco.misses[0].layer, MissLayer::NotInPool);
    }
}
