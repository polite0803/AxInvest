// SPDX-License-Identifier: AGPL-3.0-only

//! 决策事后验证（V55）—— 跑历史回放回测，测算现状 hit_rate 与 9 因子 IC
//!
//! ## 背景
//! 股票分析系统的最大问题是"决策可采信度低"。本模块用历史数据反推当前决策系统的
//! 真实命中率与因子有效性，把"事后验证"做成系统的一等公民。
//!
//! ## 工作流
//! ```text
//! reco_picks 表（历史荐股）
//!     ↓ run_decision_backtest
//! 拉取 T+5/T+20/T+60 实际 K 线 (行情 API)
//!     ↓
//! build_pick_validation() 推断 action + 计算 hit_outcome
//!     ↓
//! 写 decision_validations 表
//!     ↓
//! compute_hit_rate_report() 聚合 hit_rate + 9 因子 IC
//!     ↓
//! 返回 HitRateReport 给前端 / 反馈到 portfolio-mgr.rhai
//! ```
//!
//! ## 关键设计
//! - **dry_run=true**：不写表，只返回 report（用于预览）
//! - **synthetic_filter**：默认排除 synthetic=1 的兜底 pick（这些不是真实决策）
//! - **T+N 默认值**：[5, 20, 60]（短/中/长三个窗口）
//! - **K 线拉取**：复用 astock_client.get_klines，自动 vendor failover + 缓存

use axagent_analysis_engine::hit_rate_backtest::{
    HitRateReport, PickValidation, build_pick_validation, compute_hit_rate_report,
};
use axagent_analysis_engine::recommender::RECO_GENERATION_FLOOR;
use axagent_analysis_engine::recommender::types::RecoPick;
use axagent_entities::decision_validations;
use axagent_entities::reco_picks;
use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;
use axagent_agent_macro::agent_command;

/// 跑决策回测请求参数
///
/// `Default` + `#[serde(default)]`（2026-09-20 加）：cron 任务把本结构序列化进
/// `CronJob.prompt`，未配置时 `prompt` 为空串 ⇒ 需要能从 `{}` / 空串解析出
/// 「全部走默认」的请求。缺这两个属性时，即使每个字段都是 `Option`，
/// `serde_json::from_str::<Self>("{}")` 的行为也依赖 serde 对 `Option` 的隐式容忍，
/// 一旦将来有人给某字段去掉 `Option` 就会在**运行期**（不是编译期）报缺字段。
///
/// `Serialize` 是 2026-09-20 为 B3 自动触发补的：`create_decision_backtest_cron`
/// 要把请求体序列化成 JSON 存进 `CronJob.prompt`（同 batch-reflection /
/// validate-decisions 的配置携带方式）。加性扩宽 —— 原本只有前端 → 命令这一向，
/// 现在多出「命令 → prompt」这一向，`rename_all` 两边一致故往返自洽。
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunDecisionBacktestRequest {
    /// T+N 验证窗口（默认 [5, 20, 60]）
    pub t_plus_n_list: Option<Vec<i32>>,
    /// 是否排除 synthetic=1 的兜底 pick（默认 true）
    pub exclude_synthetic: Option<bool>,
    /// 最大回测 pick 数（默认 200，避免一次跑太多超时）
    pub max_picks: Option<u32>,
    /// 仅生成报告不写库（用于前端预览）。
    /// ⚠ cron 路径**强制 false**：定时任务算完就丢弃是零产物（见 `services.rs` 分支）。
    pub dry_run: Option<bool>,
    /// 仅回测指定周期（"short" | "mid" | "long" | None=全部）
    pub period_filter: Option<String>,
}

/// 跑决策回测响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDecisionBacktestResponse {
    pub report: HitRateReport,
    /// 写库的条数（dry_run 时为 0）
    pub written_count: usize,
    /// 跳过的条数（合成 pick / 缺数据等）
    pub skipped_count: usize,
    /// 数据源（"eastmoney" | "sina" | "xueqiu" | "fallback_seed"）
    pub data_source: String,
    /// #49：本次报告分母的**起算代际**（= `RECO_GENERATION_FLOOR`）
    pub generation_floor: i32,
    /// #49：因「早于起算代」被排除出分母的验证记录数（旧代样本，永久不可比）
    pub excluded_pre_floor_generation: usize,
    /// #49：因「代际未知」被排除出分母的验证记录数（pick 无 `reco_version`，会随写侧补章而减少）
    pub excluded_unknown_generation: usize,
}

/// #49 读侧筛样的**唯一判据**：按荐股代际把样本分成「进分母」与两类排除。
///
/// 入参是每条验证记录所 join 到的 `reco_picks.reco_version`（`None` = 归属未知，
/// 含「pick 行已不存在」的孤儿），返回 `(进分母的下标, 早于起算代, 代际未知)`。
///
/// 两类排除**分开计数**是刻意的：合成一个数就看不出处置差异 —— 前者永久不可比，
/// 后者会随写侧补章而减少（口径同 §七十五 的权重窗口）。
///
/// 筛的是**下限** [`RECO_GENERATION_FLOOR`]，不是「等于当前版」：等号会让每次换代把
/// 整个窗口清零（§五十一-② 已裁过一次，规则原文见 `holding_period.rs`）。
fn screen_by_reco_generation(
    versions: &[Option<i32>],
    generation_floor: i32,
) -> (Vec<usize>, usize, usize) {
    let mut kept = Vec::new();
    let mut pre_floor = 0usize;
    let mut unknown = 0usize;
    for (idx, version) in versions.iter().enumerate() {
        match version {
            Some(v) if *v >= generation_floor => kept.push(idx),
            Some(_) => pre_floor += 1,
            None => unknown += 1,
        }
    }
    debug_assert_eq!(
        kept.len() + pre_floor + unknown,
        versions.len(),
        "三类必须构成全集（漏一类 = 静默丢样本）"
    );
    (kept, pre_floor, unknown)
}

/// 跑决策回测 —— 历史回放回测
///
/// 流程：
/// 1. 读取 reco_picks（默认排除 synthetic 兜底）
/// 2. 对每条 pick 拉取 T+N 窗口的日 K 线
/// 3. 用 `build_pick_validation` 构建 PickValidation
/// 4. 若 dry_run=false，写 decision_validations 表
/// 5. 聚合所有 validations → HitRateReport
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "跑决策回测")]
#[tauri::command]
pub async fn run_decision_backtest(
    state: State<'_, AppState>,
    request: RunDecisionBacktestRequest,
) -> Result<RunDecisionBacktestResponse, String> {
    run_decision_backtest_inner(state.harness.db(), &state.astock_client, request).await
}

/// 决策回测的**实际实现** —— 命令层与 cron 层共用的**唯一入口**。
///
/// # 为什么必须抽出来（2026-09-20，B3 自动触发接线）
///
/// 原实现整段写在 `#[tauri::command]` 里，入参是 `State<'_, AppState>`；
/// 而 `CronExecutor` 的 handler 只捕获数据库句柄与 astock client 的 `Arc`，
/// 且运行在 `tokio::spawn` 里 —— **拿不到 `State`**，于是定时任务无法复用它，
/// `run_decision_backtest` 全仓零调用方 ⇒ `stock_analyses.outcome` 只能靠人手点。
/// 同型先例：`stock_workflow::run_batch_reflection_inner`（反思侧同样为接线而抽出）。
///
/// 抽的是**实现**，不是把命令当普通函数调 —— 后者要求随手能拿到 `AppState`，
/// 会把「cron handler 需要什么」与「命令需要什么」耦死。
pub(crate) async fn run_decision_backtest_inner(
    db: &sea_orm::DatabaseConnection,
    client: &std::sync::Arc<axagent_astock_data::AStockClient>,
    request: RunDecisionBacktestRequest,
) -> Result<RunDecisionBacktestResponse, String> {
    // ── 参数标准化 ──
    let t_plus_n_list = request.t_plus_n_list.unwrap_or_else(|| vec![5, 20, 60]);
    let exclude_synthetic = request.exclude_synthetic.unwrap_or(true);
    let max_picks = request.max_picks.unwrap_or(200).min(2000);
    let dry_run = request.dry_run.unwrap_or(false);

    // ── 1. 读取 reco_picks ──
    let mut query = reco_picks::Entity::find();
    // P1 修复(2026-08-01): 排除 serenity-screening 候选行（style='serenity'，
    // pick_data 的 price 等可能为 0，无决策验证意义；且 seed_pool_json 格式不同）。
    query = query.filter(reco_picks::Column::Style.ne("serenity"));
    if exclude_synthetic {
        query = query.filter(reco_picks::Column::Synthetic.eq(0));
    }
    if let Some(ref period) = request.period_filter {
        query = query.filter(reco_picks::Column::Period.eq(period.as_str()));
    }
    // 按 generated_at 倒序，优先回测最近的数据
    let picks = query.all(db).await.map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("读取 reco_picks 失败: {e}"))
    })?;

    if picks.is_empty() {
        // 空集合：返回空报告，避免前端拿到 None 报错
        return Ok(RunDecisionBacktestResponse {
            report: empty_report(),
            written_count: 0,
            skipped_count: 0,
            data_source: "none".to_string(),
            generation_floor: RECO_GENERATION_FLOOR,
            excluded_pre_floor_generation: 0,
            excluded_unknown_generation: 0,
        });
    }

    // 限制条数：取最近 max_picks 条
    let picks: Vec<_> = picks.into_iter().take(max_picks as usize).collect();

    // ── 2-3. 拉 K 线 + 构建 PickValidation ──
    let mut validations: Vec<PickValidation> = Vec::new();
    // #49：与 `validations` **按下标对齐**的荐股代际（`reco_picks.reco_version`）。
    // 只在下面 `Ok(...)` 分支里与 validations 同处 push ⇒ 两者长度恒等。
    let mut versions: Vec<Option<i32>> = Vec::new();
    let mut skipped = 0usize;
    let mut data_source = "unknown".to_string();

    for pick_model in &picks {
        // 解析 pick_data → RecoPick（pick_data 是 None 或 JSON 解析失败时跳过）
        let Some(ref pick_data_str) = pick_model.pick_data else {
            skipped += 1;
            continue;
        };
        let reco_pick: RecoPick = match serde_json::from_str(pick_data_str) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    pick_id = %pick_model.id,
                    error = %e,
                    "解析 reco_picks.pick_data 失败，跳过"
                );
                skipped += 1;
                continue;
            },
        };

        // 对每个 T+N 跑一次验证
        for &t_plus_n in &t_plus_n_list {
            match fetch_and_validate(
                client,
                &reco_pick,
                &pick_model.id,
                t_plus_n,
                &pick_model.generated_at,
            )
            .await
            {
                Ok((validation, src)) => {
                    if data_source == "unknown" {
                        data_source = src;
                    }
                    versions.push(pick_model.reco_version);
                    validations.push(validation);
                },
                Err(e) => {
                    tracing::warn!(
                        pick_id = %pick_model.id,
                        stock = %reco_pick.stock_code,
                        t_plus_n = t_plus_n,
                        error = %e,
                        "拉取/验证失败，跳过"
                    );
                    skipped += 1;
                },
            }
        }
    }

    // ── 4. 写库（dry_run=false 时）──
    // 写侧**不按代际筛**：旧代/代际未知的 pick 照样要落 T+N 结果（原始事实与统计分母
    // 是两回事，筛这里会让 `stock_analyses.outcome` 回写链断在存量数据上）。
    // 筛样只发生在读侧聚合（步骤 5 与 `compute_validation_report`）。
    let written_count = if dry_run || validations.is_empty() {
        0
    } else {
        write_decision_validations(db, &validations).await.map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("写 decision_validations 失败: {e}"))
        })?
    };

    // ── 4.5 P2-F15: outcome 回写 ──
    // T+N 验证完成后，根据 hit_outcome 反推 stock_analyses.outcome（win/loss），
    // 然后回写 lesson_applications.outcome_at_validation。
    // 这样 run_lesson_validation 就能精确统计 success_count。
    //
    // 匹配策略：通过 stock_code + generated_at 日期匹配 stock_analyses 行。
    // 注意：reco_picks 和 stock_analyses 是两条独立路径，这里用日期近似匹配，
    // 可能存在一对多情况（同一天同一只股票多个 analysis），取最近的一条。
    if !dry_run && !validations.is_empty() {
        let synced = sync_outcomes_to_stock_analyses(db, &validations).await;
        if synced > 0 {
            tracing::info!(
                "[backtest] P2-F15: 从 decision_validations 回写 {synced} 条 stock_analyses.outcome + lesson_applications"
            );
        }
    }

    // ── 5. 聚合报告（#49：按荐股代际筛样后才进分母；写侧不筛，见步骤 4 的说明）──
    let (kept, excluded_pre_floor_generation, excluded_unknown_generation) =
        screen_by_reco_generation(&versions, RECO_GENERATION_FLOOR);
    let screened: Vec<PickValidation> = kept.iter().map(|&i| validations[i].clone()).collect();
    if screened.is_empty() && !validations.is_empty() {
        tracing::warn!(
            "[backtest] 本轮 {} 条验证记录**全部**被代际筛样排除（起算代 {}：早于起算代 {} 条 / 代际未知 {} 条）\
             —— 命中率报告分母为 0 的含义是「没有可比代际的样本」，不是「没有验证数据」",
            validations.len(),
            RECO_GENERATION_FLOOR,
            excluded_pre_floor_generation,
            excluded_unknown_generation
        );
    }
    let report = if screened.is_empty() {
        empty_report()
    } else {
        compute_hit_rate_report(&screened)
    };

    Ok(RunDecisionBacktestResponse {
        report,
        written_count,
        skipped_count: skipped,
        data_source,
        generation_floor: RECO_GENERATION_FLOOR,
        excluded_pre_floor_generation,
        excluded_unknown_generation,
    })
}

/// 拉取 T+N 窗口日 K 线并构建 PickValidation
///
/// 入参从 `&State<'_, AppState>` 改为 `&Arc<AStockClient>`：本函数原本只用到
/// `state.astock_client`，取 `State` 会让它无法被 cron 路径复用（见
/// `run_decision_backtest_inner` 的说明）。
async fn fetch_and_validate(
    client: &std::sync::Arc<axagent_astock_data::AStockClient>,
    reco_pick: &RecoPick,
    pick_id: &str,
    t_plus_n: i32,
    generated_at: &str,
) -> Result<(PickValidation, String), String> {
    // 从 generated_at 提取决策日期（格式 "YYYY-MM-DDTHH:MM:SS.fff" → "YYYY-MM-DD"）
    let decision_date = generated_at.get(..10).unwrap_or(generated_at);

    // 拉取窗口：取 T+N 后 10 个交易日（防止节假日窗口不足）。
    // 修复 P0: 原 fetch_limit 太小（t_plus_n + 10），当 pick 距今较远时
    // 不包含决策日之后的足够数据。改为 500（约 2 个交易年）确保覆盖。
    let fetch_limit = (t_plus_n as u32 + 10).max(500);

    let klines =
        client.get_klines(&reco_pick.stock_code, "daily", fetch_limit).await.map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("K 线拉取失败: {e}"))
        })?;

    if klines.is_empty() {
        return Err("K 线为空".to_string());
    }

    // 修复 P0: 原代码 klines[..n] 取的是最早的 n 根 K 线（决策日之前的数据），
    // 导致用决策前的价格"验证"决策，命中率完全失真。
    // 正确逻辑：找到决策日之后的第一根 K 线，取该位置之后的 n 根。
    // K 线按日期升序排列（vendors/sina.rs 等都 sort_by date）
    let start_idx = klines
        .iter()
        .position(|k| k.date.as_str() > decision_date)
        .ok_or_else(|| format!("决策日 {decision_date} 之后无 K 线数据"))?;

    let valid_klines = &klines[start_idx..];
    let n = (t_plus_n as usize).min(valid_klines.len());
    let closes: Vec<f64> = valid_klines[..n].iter().map(|k| k.close).collect();
    let highs: Vec<f64> = valid_klines[..n].iter().map(|k| k.high).collect();
    let lows: Vec<f64> = valid_klines[..n].iter().map(|k| k.low).collect();

    // 数据源标识：优先用 K 线数据的 vendor 名（这里简化为 "astock_client"，
    // 因为 AStockClient 内部已做 vendor failover，统一标记）
    let data_source = "astock_client".to_string();

    let validation =
        build_pick_validation(reco_pick, pick_id, t_plus_n, &closes, &highs, &lows, &data_source);

    Ok((validation, data_source))
}

/// 批量写 decision_validations（按 (pick_id, t_plus_n) 幂等）
async fn write_decision_validations(
    db: &sea_orm::DatabaseConnection,
    validations: &[PickValidation],
) -> Result<usize, String> {
    let now = Utc::now().to_rfc3339();
    let mut count = 0;

    for v in validations {
        // 幂等：先查 (pick_id, t_plus_n) 是否已存在
        let existing = decision_validations::Entity::find()
            .filter(decision_validations::Column::PickId.eq(&v.pick_id))
            .filter(decision_validations::Column::TPlusN.eq(v.t_plus_n))
            .one(db)
            .await
            .map_err(|e| {
                ErrorResponse::new(wf_err::INTERNAL)
                    .with_detail(format!("查询已存在验证记录失败: {e}"))
            })?;

        if existing.is_some() {
            // 已存在则跳过（避免覆盖前次结果）
            continue;
        }

        let factor_snapshot_json =
            v.factor_snapshot.as_ref().and_then(|m| serde_json::to_string(m).ok());

        let active = decision_validations::ActiveModel {
            id: Set(uuid_v4()),
            pick_id: Set(v.pick_id.clone()),
            stock_code: Set(v.stock_code.clone()),
            stock_name: Set(v.stock_name.clone()),
            style: Set(v.style.clone()),
            period: Set(v.period.clone()),
            t_plus_n: Set(v.t_plus_n),
            generated_at: Set(v.generated_at.clone()),
            validated_at: Set(now.clone()),
            entry_price: Set(v.entry_price),
            target_price: Set(v.target_price),
            stop_loss: Set(v.stop_loss),
            position_pct: Set(v.position_pct),
            confidence: Set(v.confidence),
            inferred_action: Set(v.inferred_action.clone()),
            t_plus_n_price: Set(v.t_plus_n_price),
            max_price: Set(v.max_price),
            min_price: Set(v.min_price),
            max_return_pct: Set(v.max_return_pct),
            max_drawdown_pct: Set(v.max_drawdown_pct),
            final_return_pct: Set(v.final_return_pct),
            hit_stop_loss: Set(v.hit_stop_loss),
            hit_target: Set(v.hit_target),
            hit_outcome: Set(v.hit_outcome.clone()),
            factor_snapshot: Set(factor_snapshot_json),
            data_source: Set(v.data_source.clone()),
            created_at: Set(now.clone()),
        };

        decision_validations::Entity::insert(active).exec(db).await.map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("插入 decision_validations 失败: {e}"))
        })?;
        count += 1;
    }

    Ok(count)
}

/// 列表：已写入的决策验证记录
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionValidationItem {
    pub id: String,
    pub pick_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub style: String,
    pub period: String,
    pub t_plus_n: i32,
    pub generated_at: String,
    pub validated_at: String,
    pub entry_price: f64,
    pub target_price: f64,
    pub stop_loss: f64,
    pub position_pct: f64,
    pub confidence: i32,
    pub inferred_action: String,
    pub t_plus_n_price: Option<f64>,
    pub max_price: Option<f64>,
    pub min_price: Option<f64>,
    pub max_return_pct: Option<f64>,
    pub max_drawdown_pct: Option<f64>,
    pub final_return_pct: Option<f64>,
    pub hit_stop_loss: Option<i32>,
    pub hit_target: Option<i32>,
    pub hit_outcome: Option<String>,
    pub data_source: String,
}

#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "列出决策验证记录")]
#[tauri::command]
pub async fn list_decision_validations(
    state: State<'_, AppState>,
    stock_code: Option<String>,
    hit_outcome: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<DecisionValidationItem>, String> {
    use sea_orm::{PaginatorTrait, QueryOrder};
    let db = state.harness.db();

    let mut query = decision_validations::Entity::find();
    if let Some(ref code) = stock_code {
        query = query.filter(decision_validations::Column::StockCode.eq(code.as_str()));
    }
    if let Some(ref outcome) = hit_outcome {
        query = query.filter(decision_validations::Column::HitOutcome.eq(outcome.as_str()));
    }

    let paginator = query
        .order_by_desc(decision_validations::Column::ValidatedAt)
        .paginate(db, limit.unwrap_or(100) as u64);
    let items = paginator.fetch_page(offset.unwrap_or(0) as u64).await.map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL)
            .with_detail(format!("查询 decision_validations 失败: {e}"))
    })?;

    Ok(items
        .into_iter()
        .map(|m| DecisionValidationItem {
            id: m.id,
            pick_id: m.pick_id,
            stock_code: m.stock_code,
            stock_name: m.stock_name,
            style: m.style,
            period: m.period,
            t_plus_n: m.t_plus_n,
            generated_at: m.generated_at,
            validated_at: m.validated_at,
            entry_price: m.entry_price,
            target_price: m.target_price,
            stop_loss: m.stop_loss,
            position_pct: m.position_pct,
            confidence: m.confidence,
            inferred_action: m.inferred_action,
            t_plus_n_price: m.t_plus_n_price,
            max_price: m.max_price,
            min_price: m.min_price,
            max_return_pct: m.max_return_pct,
            max_drawdown_pct: m.max_drawdown_pct,
            final_return_pct: m.final_return_pct,
            hit_stop_loss: m.hit_stop_loss,
            hit_target: m.hit_target,
            hit_outcome: m.hit_outcome,
            data_source: m.data_source,
        })
        .collect())
}

/// 验证报告响应 —— 报告本体之外还带**代际筛样的账**（#49）。
///
/// 为什么三个数一起给：只回 `report.total` 的话，看到 0 无法区分「没有验证数据」与
/// 「有数据但全部不可比 / 归属未知」—— 后者是结构性缺口，伪装成前者的正常空结果即是
/// 歧义（AGENTS.md「结构性缺口不得在 UI 造成歧义」；形态同 §七十五 权重窗口的排除计数）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReportResponse {
    pub report: HitRateReport,
    /// 本次分母的起算代际（= `RECO_GENERATION_FLOOR`）
    pub generation_floor: i32,
    /// 因「早于起算代」被排除的记录数（旧代样本，永久不可比）
    pub excluded_pre_floor_generation: usize,
    /// 因「代际未知」被排除的记录数（pick 无 `reco_version`，或 pick 行已不存在）
    pub excluded_unknown_generation: usize,
    /// 本次读到的 `decision_validations` 总行数 = 进分母 + 两类排除（自证分母没漏）
    pub total_rows: usize,
}

/// 聚合报告 —— 基于已写入的 decision_validations 重新计算
///
/// ## #49 代际筛样（读侧，PLAN 七十九 A2）
/// 本表自己**没有**版本列，代际在它 join 的 `reco_picks.reco_version` 上。口径是**荐股域**的
/// [`RECO_GENERATION_FLOOR`]，不是工作流域的 `HORIZON_BRANCH_GENERATION_FLOOR` —— 荐股链不跑
/// 工作流模板（见 `recommender::RECO_ALGORITHM_VERSION` 的注释），拿错域的整数比大小会让
/// 筛样恒真或恒假。写侧盖章由 `seed_consistency_tests` 的「每个 `reco_picks::ActiveModel`
/// 必带 `reco_version`」门守着；`NULL`（含 pick 行已不存在）记为代际未知并**排除**。
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "计算验证报告")]
#[tauri::command]
pub async fn compute_validation_report(
    state: State<'_, AppState>,
) -> Result<ValidationReportResponse, String> {
    let db = state.harness.db();
    let all = decision_validations::Entity::find().all(db).await.map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL)
            .with_detail(format!("读取 decision_validations 失败: {e}"))
    })?;
    let total_rows = all.len();

    // 代际来源：整张 reco_picks 只取 (id, reco_version) 两列 —— 不为筛样把 pick_data /
    // seed_pool_json 搬进内存，也不按 pick_id 拼 IN 列表（存量行数会顶到 SQLite 参数上限）
    let pick_versions: HashMap<String, Option<i32>> = reco_picks::Entity::find()
        .select_only()
        .column(reco_picks::Column::Id)
        .column(reco_picks::Column::RecoVersion)
        .into_tuple()
        .all(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("读取 reco_picks 代际失败: {e}"))
        })?
        .into_iter()
        .collect();

    if all.is_empty() {
        return Ok(ValidationReportResponse {
            report: empty_report(),
            generation_floor: RECO_GENERATION_FLOOR,
            excluded_pre_floor_generation: 0,
            excluded_unknown_generation: 0,
            total_rows,
        });
    }

    // DB 行 → (荐股代际, PickValidation)。`get(..).copied().flatten()` 把「pick 行已不存在」
    // 与「pick 有行但没盖章」并成同一个 None —— 两者都是归属未知，处置一致。
    let rows: Vec<(Option<i32>, PickValidation)> = all
        .into_iter()
        .map(|m| {
            let version = pick_versions.get(&m.pick_id).copied().flatten();
            let factor_snapshot: Option<HashMap<String, f64>> =
                m.factor_snapshot.as_ref().and_then(|s| serde_json::from_str(s).ok());
            let validation = PickValidation {
                pick_id: m.pick_id,
                stock_code: m.stock_code,
                stock_name: m.stock_name,
                style: m.style,
                period: m.period,
                generated_at: m.generated_at,
                t_plus_n: m.t_plus_n,
                entry_price: m.entry_price,
                target_price: m.target_price,
                stop_loss: m.stop_loss,
                position_pct: m.position_pct,
                confidence: m.confidence,
                inferred_action: m.inferred_action,
                t_plus_n_price: m.t_plus_n_price,
                max_price: m.max_price,
                min_price: m.min_price,
                max_return_pct: m.max_return_pct,
                max_drawdown_pct: m.max_drawdown_pct,
                final_return_pct: m.final_return_pct,
                hit_stop_loss: m.hit_stop_loss,
                hit_target: m.hit_target,
                hit_outcome: m.hit_outcome,
                factor_snapshot,
                data_source: m.data_source,
            };
            (version, validation)
        })
        .collect();

    // 筛样（判据与 run 侧共用 `screen_by_reco_generation`，避免一个名目两套分母）
    let versions: Vec<Option<i32>> = rows.iter().map(|(v, _)| *v).collect();
    let (kept, excluded_pre_floor_generation, excluded_unknown_generation) =
        screen_by_reco_generation(&versions, RECO_GENERATION_FLOOR);
    let validations: Vec<PickValidation> = kept.iter().map(|&i| rows[i].1.clone()).collect();

    if validations.is_empty() {
        tracing::warn!(
            "[validation_report] decision_validations {} 条**全部**被代际筛样排除\
             （起算代 {}：早于起算代 {} 条 / \
             代际未知 {} 条）\
             —— 报告分母为 0 的含义是「没有可比代际的样本」，不是「没有验证数据」",
            total_rows,
            RECO_GENERATION_FLOOR,
            excluded_pre_floor_generation,
            excluded_unknown_generation
        );
    }

    let report = if validations.is_empty() {
        empty_report()
    } else {
        compute_hit_rate_report(&validations)
    };

    Ok(ValidationReportResponse {
        report,
        generation_floor: RECO_GENERATION_FLOOR,
        excluded_pre_floor_generation,
        excluded_unknown_generation,
        total_rows,
    })
}

/// 空报告（无 pick 时返回，避免前端拿 None 报错）
fn empty_report() -> HitRateReport {
    HitRateReport {
        total: 0,
        generated_at: Utc::now().to_rfc3339(),
        by_action: HashMap::new(),
        by_style: HashMap::new(),
        by_t_plus_n: HashMap::new(),
        factor_ic: HashMap::new(),
        factor_ic_ranked: Vec::new(),
        best_picks: Vec::new(),
        worst_picks: Vec::new(),
    }
}

/// UUID v4 简易实现（避免引入额外依赖）
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    // 时间戳 + 纳秒数 + 进程 ID 拼一个伪 UUID
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (nanos >> 32) as u32,
        ((nanos >> 16) as u16),
        (nanos & 0x0FFF) as u16,
        std::process::id(),
        nanos as u64 & 0xFFFFFFFFFFFF
    )
}

/// P2-F15 切入点 3：从 decision_validations 回写 stock_analyses.outcome + lesson_applications.outcome_at_validation
///
/// 遍历本次 T+N 验证结果（`PickValidation`），根据 `hit_outcome` 推断 win/loss，
/// 然后通过 `stock_code + generated_at` 日期匹配 `stock_analyses` 行，更新其
/// `outcome` 字段，并同步回写 `lesson_applications.outcome_at_validation`。
///
/// ## hit_outcome → outcome 映射
/// - `hit` / `partial` → `win`
/// - `miss` / `false_hit` → `loss`
/// - `insufficient` / `None` → 跳过（数据不足，不做判定）
///
/// ## 匹配策略
/// `reco_picks.generated_at`（ISO 8601）取日期部分，匹配 `stock_analyses.analysis_date`
/// （YYYY-MM-DD）。同一只股票同一天可能有多个 analysis，取 `created_at` 最大（最新）的一条。
///
/// ## 幂等性
/// `update_lesson_application_outcome` 内部有 `outcome_at_validation IS NULL` 守卫，
/// 不会覆盖已验证结果。`stock_analyses.outcome` 用 `update_many` 直接覆盖，
/// 但同一 analysis 的 T+N 验证结果应该是一致的（T+5/T+20/T+60 可能不同，
/// 取最严重的 loss 优先）。
async fn sync_outcomes_to_stock_analyses(
    db: &sea_orm::DatabaseConnection,
    validations: &[PickValidation],
) -> u64 {
    use axagent_entities::stock_analyses;
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};

    let mut synced = 0u64;

    for v in validations {
        // 1. hit_outcome → outcome 映射（口径与离线回测一致，见 hit_rate_backtest 判定源）
        let Some(outcome) =
            axagent_analysis_engine::hit_rate_backtest::hit_outcome_to_binary_outcome(
                v.hit_outcome.as_deref(),
            )
        else {
            continue;
        };

        // 2. 从 generated_at 提取日期（YYYY-MM-DD）
        let decision_date = v.generated_at.get(..10).unwrap_or(&v.generated_at);

        // 3. 查 stock_analyses 中匹配的行（stock_code + analysis_date）
        //    取最新的一条，避免一对多时更新多条
        //    只更新 outcome 为 NULL 或 pending 的行，避免覆盖已验证结果
        let matching = stock_analyses::Entity::find()
            .filter(stock_analyses::Column::StockCode.eq(&v.stock_code))
            .filter(stock_analyses::Column::AnalysisDate.eq(decision_date))
            .filter(
                Condition::any()
                    .add(stock_analyses::Column::Outcome.is_null())
                    .add(stock_analyses::Column::Outcome.eq("pending")),
            )
            .order_by_desc(stock_analyses::Column::CreatedAt)
            .one(db)
            .await;

        let Ok(Some(analysis)) = matching else {
            // 无匹配行或查询失败，跳过
            continue;
        };

        // 4. 更新 stock_analyses.outcome
        let validation_source = match v.t_plus_n {
            5 => "t_plus_5",
            20 => "t_plus_20",
            60 => "t_plus_60",
            _ => "t_plus_n",
        };

        let update_result = stock_analyses::Entity::update_many()
            .col_expr(stock_analyses::Column::Outcome, Expr::value(outcome))
            .col_expr(
                stock_analyses::Column::UpdatedAt,
                Expr::value(chrono::Utc::now().timestamp_millis()),
            )
            .filter(stock_analyses::Column::Id.eq(&analysis.id))
            .exec(db)
            .await;

        if update_result.is_err() {
            continue;
        }

        // 5. 回写 lesson_applications.outcome_at_validation
        //    调用 core::update_lesson_application_outcome
        let affected = crate::commands::stock_workflow::core::update_lesson_application_outcome(
            db,
            &analysis.id,
            outcome,
            validation_source,
        )
        .await;

        if affected > 0 {
            synced += affected;
        }
    }

    synced
}

// ═══════════════════════════════════════════════════════════════════════
// B3 自动触发：决策回测的定时任务（task_type = decision-backtest）
// ═══════════════════════════════════════════════════════════════════════
//
// ## 为什么需要这一组（2026-09-20）
//
// `run_decision_backtest` 的后端逻辑一直完整（T+N 命中率 + 9 因子 IC +
// 回写 `stock_analyses.outcome` + `lesson_applications.outcome_at_validation`），
// 但**全仓零调用方**：前端 `src/**` 搜不到 `runDecisionBacktest` / `hitRate` /
// `decisionValidation`，CronExecutor 里也没有对应分支 ⇒ `stock_analyses.outcome`
// 只能靠人手触发，B3 闭环断在「没人触发」这一环，而不是逻辑缺失。
//
// 因此本组命令先把**自动触发**接上（产品裁决：先接自动触发，前端面板后做）。
// 四件套（create / list / toggle / delete）与其余 task_type 保持一致形态，
// 前端面板将来只需按名调用，不必再改后端。
//
// ⚠ 定时任务三问（缺一即为死配置）：
//   ① 真落库？ —— `create_decision_backtest_cron` 写 `CronJobStore`，配置 JSON
//      进 `CronJob.prompt`（与 batch-reflection / validate-decisions 同形态）；
//   ② 有执行分支？ —— `init/services.rs` 的 `decision-backtest` 分支；
//   ③ 有真产物？ —— 写 `decision_validations` + 回写 `stock_analyses.outcome`。
//   下游核验入口：`list_decision_validations` / `compute_validation_report`。

/// `decision-backtest` 任务类型标识。
///
/// **单点声明**：`init/services.rs` 的分支与本文件的 create 命令都引用它，
/// 不各写一份字面量（同 `pool_scan::POOL_SCAN_TASK_TYPE` 的处置）。
/// 改名会静默切断派发（分支匹配不到 ⇒ 任务到点什么都不发生，且不报错）。
pub const DECISION_BACKTEST_TASK_TYPE: &str = "decision-backtest";

/// 解析 cron 侧的配置（`CronJob.prompt`）。
///
/// 空串 / 纯空白 ⇒ 全默认（T+5/20/60、排除 synthetic、200 条上限、写库）。
/// 解析失败**必须报错**：静默回落默认会让「配置写错」看起来像「配置生效了」。
pub(crate) fn parse_decision_backtest_config(
    prompt: &str,
) -> Result<RunDecisionBacktestRequest, String> {
    if prompt.trim().is_empty() {
        return Ok(RunDecisionBacktestRequest::default());
    }
    serde_json::from_str(prompt)
        .map_err(|e| format!("解析 {} 配置失败: {e}", DECISION_BACKTEST_TASK_TYPE))
}

/// 创建决策回测定时任务。
///
/// - `cron_expression`: 默认 `"0 7 * * *"`（**早于**反思族任务的 06:00 之后、
///   收市结算之前；T+N 验证只读历史 K 线，早跑不会拿到不完整数据）
/// - `t_plus_n_list` / `max_picks` / `period_filter` / `exclude_synthetic`:
///   原样进配置；`dry_run` **不可配**（定时任务算完即弃是零产物，见下）
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "创建决策回测定时任务")]
#[tauri::command]
pub async fn create_decision_backtest_cron(
    state: State<'_, AppState>,
    cron_expression: Option<String>,
    t_plus_n_list: Option<Vec<i32>>,
    max_picks: Option<u32>,
    period_filter: Option<String>,
    enabled: Option<bool>,
) -> Result<crate::commands::stock_analysis::CronJobResponse, String> {
    let id = format!("decbt-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("x"));
    let expr = cron_expression.unwrap_or_else(|| "0 7 * * *".to_string());

    // dry_run **恒 false**（不暴露成参数）：定时任务的价值就是落库与回写，
    // 允许配 true 等于允许建一个"到点跑、算完扔掉"的任务 —— 三问里的第③问直接为否。
    let request = RunDecisionBacktestRequest {
        t_plus_n_list: t_plus_n_list.clone(),
        exclude_synthetic: None, // None ⇒ 默认 true（排除 synthetic 兜底 pick）
        max_picks,
        dry_run: Some(false),
        period_filter: period_filter.clone(),
    };
    let prompt =
        serde_json::to_string(&request).map_err(|e| format!("序列化决策回测配置失败: {e}"))?;

    let windows = t_plus_n_list
        .as_ref()
        .map(|v| v.iter().map(|n| format!("T+{n}")).collect::<Vec<_>>().join("/"))
        .unwrap_or_else(|| "T+5/20/60（默认）".to_string());
    let desc = format!(
        "决策回测：按 reco_picks 回放验证并回写 outcome（窗口 {}，周期 {}，上限 {} 条）",
        windows,
        period_filter.as_deref().unwrap_or("全部"),
        max_picks.unwrap_or(200)
    );

    let mut job = axagent_runtime_core::CronJob::new(&id, &expr, &prompt, &desc)
        .with_task_type(DECISION_BACKTEST_TASK_TYPE);
    if !enabled.unwrap_or(true) {
        job.status = axagent_runtime_core::CronJobStatus::Paused;
    }
    state.cron_job_store.add(job.clone()).await;
    Ok(crate::commands::stock_analysis::CronJobResponse::from(&job))
}

/// 列出所有决策回测定时任务
#[agent_command(domain = "finance", safety = Safe, call_mode = StateOnly, description = "列出决策回测定时任务")]
#[tauri::command]
pub async fn list_decision_backtest_crons(
    state: State<'_, AppState>,
) -> Result<Vec<crate::commands::stock_analysis::CronJobResponse>, String> {
    let jobs = state.cron_job_store.list().await;
    Ok(jobs
        .iter()
        .filter(|j| j.task_type.as_deref() == Some(DECISION_BACKTEST_TASK_TYPE))
        .map(crate::commands::stock_analysis::CronJobResponse::from)
        .collect())
}

/// 启停决策回测定时任务
#[agent_command(domain = "finance", safety = Safe, call_mode = StateOnly, description = "开关决策回测定时任务")]
#[tauri::command]
pub async fn toggle_decision_backtest_cron(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .cron_job_store
        .set_status(
            &id,
            if enabled {
                axagent_runtime_core::CronJobStatus::Active
            } else {
                axagent_runtime_core::CronJobStatus::Paused
            },
        )
        .await;
    Ok(())
}

/// 删除决策回测定时任务
#[agent_command(domain = "finance", safety = Dangerous, call_mode = StateInput, description = "删除决策回测定时任务")]
#[tauri::command]
pub async fn delete_decision_backtest_cron(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state.cron_job_store.remove(&id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uuid_v4_format() {
        let id = uuid_v4();
        // 格式: 8-4-4-4-12 hex
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
        assert!(id.split('-').next().unwrap().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_empty_report() {
        let r = empty_report();
        assert_eq!(r.total, 0);
        assert!(r.by_action.is_empty());
        assert!(r.by_style.is_empty());
        assert!(r.by_t_plus_n.is_empty());
        assert!(r.factor_ic.is_empty());
    }

    /// #49：筛样三类必须构成全集，且**下限不是等号**（>= floor 全进，含更晚的代）
    #[test]
    fn test_screen_by_reco_generation_partitions_into_three() {
        let versions = [Some(0), Some(1), Some(2), None];
        let (kept, pre, unknown) = screen_by_reco_generation(&versions, 1);
        assert_eq!(kept, vec![1, 2], "起算代及其之后都进分母；等号口径会让每次换代清零窗口");
        assert_eq!((pre, unknown), (1, 1), "早于起算代与代际未知必须分开计数");
        assert_eq!(kept.len() + pre + unknown, versions.len());
    }

    /// #49 负控：代际未知**不得**被当成本代（现网存量全是 NULL ⇒ 这条分支决定报告是
    /// 「空分母 + 说明」还是「假装样本都属于当前代」）
    #[test]
    fn test_screen_by_reco_generation_never_counts_unknown_as_current() {
        let versions = [None, None, None];
        let (kept, pre, unknown) = screen_by_reco_generation(&versions, 1);
        assert!(kept.is_empty(), "NULL 不进分母");
        assert_eq!(
            (pre, unknown),
            (0, 3),
            "三个 NULL 只能记成「代际未知」，不得混进「早于起算代」"
        );
    }
}
