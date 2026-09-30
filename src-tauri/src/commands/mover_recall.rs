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
use serde::Serialize;

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;

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

/// 主入口：跑一次核查。
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
    let db = state.harness.db();
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
    let picks = mover_recall::load_picks_in_range(db, &from, &upper).await?;
    let trim_evidence = mover_recall::load_trim_evidence(db, &from, &upper).await?;
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
            let anchor_idx = match close_dates.iter().position(|d| *d == ev.anchor_date) {
                Some(i) => i,
                None => continue,
            };
            if anchor_idx + 1 < rule.window_days as usize {
                continue;
            }
            let window_start = close_dates[anchor_idx + 1 - rule.window_days as usize].clone();
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
    misses.truncate(200);

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
