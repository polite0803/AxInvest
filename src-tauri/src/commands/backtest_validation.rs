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
//! compute_period_matched_report() 聚合 hit_rate + 9 因子 IC（每档只计自己的窗口）
//!     ↓
//! 返回 HitRateReport 给前端 / 反馈到 portfolio-mgr.rhai
//! ```
//!
//! ## 关键设计
//! - **dry_run=true**：不写表，只返回 report（用于预览）
//! - **synthetic_filter**：默认排除 synthetic=1 的兜底 pick（这些不是真实决策）
//! - **T+N 默认值**：由四档权威现场推导（`Period::validation_windows()` = 2/5/28/90 交易日），
//!   不再是手抄的 `[5, 20, 60]`。旧窗口集的存量行**不回填**，三个统计读点按
//!   `Period::is_validation_window` 把它们挡在分母外并计数（拍板 6/7）。
//! - **K 线拉取**：复用 astock_client.get_klines，自动 vendor failover + 缓存

use axagent_analysis_engine::hit_rate_backtest::{
    HitRateReport, PRICE_DIVERGENCE_TOLERANCE_PCT, PickValidation, build_pick_validation,
    compute_period_matched_report, price_divergence_pct, price_is_divergent,
};
use axagent_analysis_engine::recommender::RECO_GENERATION_FLOOR;
use axagent_analysis_engine::recommender::types::RecoPick;
use axagent_analysis_engine::serenity_hit_rate_backtest::{
    SerenityHitRateReport, SerenityPickValidation, build_serenity_validation,
    compute_serenity_hit_rate_report, serenity_validation_to_row,
};
use axagent_entities::decision_validations;
use axagent_entities::reco_picks;
use chrono::Utc;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::common;
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
    /// T+N 验证窗口（默认 = 四档期望持有交易日，权威见 `harness::holding_period::Period::validation_windows`；
    /// 手抄一份数字就是第五套参数，见拍板 7）
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
    /// 是否把**回放代**（`reco_picks.as_of_date IS NOT NULL`）一并验证（默认 false）。
    ///
    /// ## 为什么默认 false
    ///
    /// 默认路径是**实盘业绩**的分母：回放样本混进来会把净值虚高
    /// （2026-10-25 实测：混入回放后 T+2 净均值 +3.76% → 仅实盘 −2.55%，差 6.3pp）。
    ///
    /// ## 为什么需要 true
    ///
    /// 回放代目前**没有任何验证入口**：全库 974 条回放 pick 对应
    /// `decision_validations` 只有 12 行、且 0 行判定过。
    /// 要回答「回放能不能替代等成熟期」就必须能单独验证它。
    ///
    /// ⇒ 打开后写入的行仍带 `as_of_date`，与实盘行**始终可分**，
    /// 消费侧（`list_decision_validations` / `reco_loop`）默认仍只看实盘。
    pub include_replay: Option<bool>,
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
    /// 数据源（"eastmoney" | "sina" | "xueqiu" | "fallback_seed"；两趟都零产物时为 "none"）
    pub data_source: String,
    /// #49：本次报告分母的**起算代际**（= `RECO_GENERATION_FLOOR`）
    pub generation_floor: i32,
    /// #49：因「早于起算代」被排除出分母的验证记录数（旧代样本，永久不可比）
    pub excluded_pre_floor_generation: usize,
    /// #49：因「代际未知」被排除出分母的验证记录数（pick 无 `reco_version`，会随写侧补章而减少）
    pub excluded_unknown_generation: usize,
    /// 智选（serenity）候选的 T+N **超额**命中报表（判据是「跑赢基准多少」，
    /// 与上面 `report` 的「是否到达目标价」是两个不同的计数域，**不得合并显示成一个命中率**）。
    ///
    /// `None` 只在智选一趟没跑到时出现；跑到就恒 `Some`（哪怕零样本 —— 零样本与没跑
    /// 是两种缺席，见 `serenity_hit_rate_backtest::SERENITY_METRIC_NOT_PERSISTED`）。
    pub serenity: Option<SerenityHitRateReport>,
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
///
/// # 为什么是 `pub` 而不是 `pub(crate)`（2026-10-25，A-2 回补验证）
///
/// `src/bin/axagent-validate-backfill.rs` 要用它：前端没有触发入口、
/// cron 分支又强制 `dryRun=false`，带 `dryRun` 的探测只能从 CLI 驱动。
/// 同形先例：`run_batch_reflection_inner` 也是 `pub` + 在 `lib.rs` re-export，
/// 供 `axagent-batch-rerun` 调用 —— 这里沿用同一形态，**不新增第二份实现**。
pub async fn run_decision_backtest_inner(
    db: &sea_orm::DatabaseConnection,
    client: &std::sync::Arc<axagent_astock_data::AStockClient>,
    request: RunDecisionBacktestRequest,
) -> Result<RunDecisionBacktestResponse, String> {
    // ── 参数标准化 ──
    let t_plus_n_list = request
        .t_plus_n_list
        .unwrap_or_else(axagent_harness::holding_period::Period::validation_windows);
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
    // **只验证实盘代**（2026-10-25 S0 实测发现，缺这一行会让分母混代）：
    //
    // 生产库实测：`synthetic=0 AND style<>'serenity'` 共 1,706 行，其中 **968 行
    // （56.8%）是回放代**（`as_of_date` 非空）。不过滤就会把「实盘出票」与
    // 「历史回放出票」混进同一个命中率分母 —— 两者市场环境不同、可执行性不同，
    // 混算得出的胜率既不代表实盘，也不代表回放。
    //
    // 与同表其他读点同口径：`reco_loop.rs:358`（权重闭环）、
    // `list_decision_validations` 的 `generation` 分支、`_shared_reco_pool.rs` 文档
    // 「只读实盘的下游必须显式带 `as_of_date IS NULL`，漏加由 check-reco-generation.mjs 拦」。
    // 回放样本的验证走各自的 as-of 入口（如 `mover_recall_analysis` 批次），不经本函数。
    //
    // 2026-10-25 R-1 修正：上面那条「回放走各自入口」在**荐股层并不成立** ——
    // 实测 974 条回放 pick 只对应 12 行 `decision_validations`、0 行判定过。
    // ⇒ 加 `include_replay` 开关（默认 false，行为与上面完全一致），
    //   让「回放代能不能替代等成熟期」这个问题第一次可以单独验证。
    //   打开后写入行仍带 `as_of_date`，实盘/回放始终可分。
    let include_replay = request.include_replay.unwrap_or(false);
    if !include_replay {
        query = query.filter(reco_picks::Column::AsOfDate.is_null());
    }
    // ⚠ `order_by_desc` **必须有**（2026-10-25 A-1 修复）：
    //   原注释就写着「按 generated_at 倒序，优先回测最近的数据」，但**代码里没有 order_by**
    //   —— 直接在数据库的返回顺序上 `take(max_picks)`。两个可观测后果（生产库实测）：
    //   ① 127 单成熟样本只覆盖 **7 个交易日**（拿到的总是同一批偶然靠前的行）；
    //   ② 同一「股票×风格×周期×日」出现 6 行重复（同一逻辑批次的多个副本都被选中）。
    //   下面的去重依赖「先见即最新」，所以排序是去重的前置条件，缺一不可。
    let picks =
        query.order_by_desc(reco_picks::Column::GeneratedAt).all(db).await.map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("读取 reco_picks 失败: {e}"))
        })?;

    if picks.is_empty() {
        // 不早退：智选（serenity）行走独立一趟（见下方 §6），这里只是通用通道无候选。
        tracing::info!("[backtest] 通用通道无候选 pick（style≠serenity），本轮只跑智选超额验证");
    }

    // **业务键去重**（2026-10-25 A-1）：同一「股票 × 风格 × 周期 × 日 × 代际」只验证一次，
    // 保留 `generated_at` 最新的一份（依赖上面的倒序）。
    //
    // 为什么必须有：`reco_picks` 没有业务唯一约束，同一逻辑批次会被重复执行数十次
    // （生产库实测 2026-10-19 一天 32 个批次、每批 440 行且 `pick_data` 逐字节相同，
    // 单格最多 30 份副本）。不去重则补跑验证会把同一张票验证几十遍，
    // 既浪费 K 线请求，又会把重复样本灌进胜率分母 —— 这正是 127 单里那 6 行重复的来源。
    //
    // 键里的 `day` 取 `generated_at` 的日期段：同一天内的多次刷新视为同一批（保留最新），
    // 跨天则各自成样本（不同市场环境，本来就是不同证据）。
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let picks: Vec<_> = picks
        .into_iter()
        .filter(|p| {
            let day = p.generated_at.split('T').next().unwrap_or(p.generated_at.as_str());
            seen_keys.insert(format!(
                "{}|{}|{}|{}|{}",
                p.stock_code,
                p.style,
                p.period,
                day,
                p.as_of_date.as_deref().unwrap_or("live"),
            ))
        })
        .take(max_picks as usize)
        .collect();

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
                pick_model.as_of_date.as_deref(),
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
        // 回测命令的**对外报告**同样必须按档匹配窗口；此前这里仍走
        // `compute_hit_rate_report`（跨 2/5/28/90 池化），于是「回测页显示 50%、
        // 验证报告显示 42%」两套口径并存 —— 2026-10-24 修。
        compute_period_matched_report(&screened)
    };

    // ── 6. 智选（serenity）超额命中通道 ──
    // 单独一趟、单独报表：它的判据词表与通用通道不同，混进上面的 `compute_hit_rate_report`
    // 会把智选显示成「命中率 0%」（全部落进 insufficient）。见 `run_serenity_backtest_pass`。
    let (serenity_rows, serenity_report, serenity_skipped) =
        run_serenity_backtest_pass(db, client, &t_plus_n_list, exclude_synthetic, max_picks)
            .await?;
    let serenity_written = if dry_run || serenity_rows.is_empty() {
        0
    } else {
        write_decision_validations(db, &serenity_rows).await.map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("写智选超额验证行失败: {e}"))
        })?
    };
    tracing::info!(
        "[backtest] 智选超额命中：判过 {} 条 / 跑赢基准 {} 条 / 超额命中率 {:.3} / 平均超额 {:.2}% \
         / 跳过 {} 条 / 落库 {serenity_written}",
        serenity_report.validated_picks,
        serenity_report.hit_count,
        serenity_report.hit_rate,
        serenity_report.avg_excess_return,
        serenity_skipped,
    );
    if data_source == "unknown" {
        // 两趟都零产物 ⇒ 沿用旧口径的 "none"（前端按它判「本轮没取到行情」）；
        // 只有智选趟产了行 ⇒ 标 astock_client（智选趟固定走 client 的 vendor failover）
        data_source = if serenity_rows.is_empty() {
            "none".to_string()
        } else {
            "astock_client".to_string()
        };
    }

    Ok(RunDecisionBacktestResponse {
        report,
        written_count: written_count + serenity_written,
        skipped_count: skipped + serenity_skipped,
        data_source,
        generation_floor: RECO_GENERATION_FLOOR,
        excluded_pre_floor_generation,
        excluded_unknown_generation,
        serenity: Some(serenity_report),
    })
}

/// 决策日期：回放 pick 用**它的 as-of 截止日**，实盘 pick 用入库时刻的日期部分
/// （格式 "YYYY-MM-DDTHH:MM:SS.fff" → "YYYY-MM-DD"）。回放的入库时刻是"今天"，
/// 拿它当决策日会去验证一个过去决策之后的行情 ⇒ 命中率完全失真。
fn decision_date_of<'a>(generated_at: &'a str, as_of_date: Option<&'a str>) -> &'a str {
    match as_of_date {
        Some(d) => d,
        None => generated_at.get(..10).unwrap_or(generated_at),
    }
}

/// 取「决策日之后 N 个交易日」的窗口序列 —— 通用通道与智选通道**共用**，
/// 不各写一遍取数与日期定位（两份实现曾经只在其中一侧修过下标口径的 P0）。
///
/// K 线按日期升序（各 vendor 均 sort_by date）。
/// 返回 `(closes, highs, lows, 窗口末日, 锚定收盘价)`。
///
/// **锚定收盘价** = 决策日（或决策日之前最近一个交易日）的收盘价，取自**同一份 K 线序列**。
/// 它是 `RecoPick.price` 的唯一可比基准：两者不同源，正是「进错出对」的成因
/// （出场价取自 K 线 ⇒ 100% 正确；入场价取自 as-of 报价通道 ⇒ 实测会串代码）。
/// 决策日早于全部 K 线时返回 `None`（无法判定，不等于一致）。
async fn fetch_window_series(
    client: &std::sync::Arc<axagent_astock_data::AStockClient>,
    stock_code: &str,
    decision_date: &str,
    t_plus_n: i32,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>, String, Option<f64>), String> {
    // 拉取窗口：取 T+N 后 10 个交易日（防止节假日窗口不足）。
    // 修复 P0: 原 fetch_limit 太小（t_plus_n + 10），当 pick 距今较远时
    // 不包含决策日之后的足够数据。改为 500（约 2 个交易年）确保覆盖。
    let fetch_limit = (t_plus_n as u32 + 10).max(500);
    let klines = client.get_klines(stock_code, "daily", fetch_limit).await.map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("K 线拉取失败: {e}"))
    })?;
    if klines.is_empty() {
        return Err("K 线为空".to_string());
    }
    // 修复 P0: 原代码 klines[..n] 取的是最早的 n 根 K 线（决策日之前的数据），
    // 导致用决策前的价格"验证"决策，命中率完全失真。
    // 正确逻辑：找到决策日之后的第一根 K 线，取该位置之后的 n 根。
    let start_idx = klines
        .iter()
        .position(|k| k.date.as_str() > decision_date)
        .ok_or_else(|| format!("决策日 {decision_date} 之后无 K 线数据"))?;
    let valid_klines = &klines[start_idx..];
    // 修复 2026-10-24：窗口不足时旧实现 `min(t_plus_n, len)` 会把「只有 30 根」的
    // T+90 当成 T+90 写进 decision_validations（final_return 实际是 30 日收益），
    // 这是回测里最隐蔽的错标。宁可不写，也不把不同持有期混进同一列。
    if valid_klines.len() < t_plus_n as usize {
        return Err(format!(
            "T+{t_plus_n} 窗口不足：决策日 {decision_date} 之后只有 {} 根 K 线",
            valid_klines.len()
        ));
    }
    let n = t_plus_n as usize;
    let closes: Vec<f64> = valid_klines[..n].iter().map(|k| k.close).collect();
    let highs: Vec<f64> = valid_klines[..n].iter().map(|k| k.high).collect();
    let lows: Vec<f64> = valid_klines[..n].iter().map(|k| k.low).collect();
    let end_date = valid_klines[n - 1].date.to_string();
    // 锚定收盘价：决策日（或之前最近交易日）的收盘价。`start_idx == 0` 表示决策日
    // 早于全部 K 线 ⇒ 没有可比基准，如实返回 `None`（不拿首根收盘价冒充）。
    let anchor_close = if start_idx > 0 {
        Some(klines[start_idx - 1].close)
    } else {
        None
    };
    Ok((closes, highs, lows, end_date, anchor_close))
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
    as_of_date: Option<&str>,
) -> Result<(PickValidation, String), String> {
    let decision_date = decision_date_of(generated_at, as_of_date);
    let (closes, highs, lows, _end_date, anchor_close) =
        fetch_window_series(client, &reco_pick.stock_code, decision_date, t_plus_n).await?;

    // ── R-3a 价源平价校验（2026-10-25）──
    // `RecoPick.price` 走 as-of 报价通道，与库内真实收盘可能不符（实测 000779 记 1.63 /
    // 真实 10.60，且 1.63 恰是 000980 当日收盘 ⇒ 批量取价行错位）。而 `targetPrice` /
    // `stopLoss` 都是按那个错价算的 ⇒ **一行价格体系错了，整行不可用**，不能靠替换入场价修补。
    // 剔除而非修补的理由见 `PRICE_DIVERGENCE_TOLERANCE_PCT` 的文档。
    if let Some(anchor) = anchor_close {
        if price_is_divergent(reco_pick.price, anchor) {
            let dev = price_divergence_pct(reco_pick.price, anchor).unwrap_or(f64::NAN);
            // 回放侧历史上有 3/173 条属此形态；实盘 311 条实测最大偏离仅 4.4%（0 条命中）
            // ⇒ 这条分支命中即意味着该条 pick 的价格来源有问题，必须留痕而非静默丢弃。
            tracing::warn!(
                pick_id = %pick_id,
                stock = %reco_pick.stock_code,
                t_plus_n = t_plus_n,
                decision_date = %decision_date,
                pick_price = reco_pick.price,
                anchor_close = anchor,
                dev_pct = dev,
                tolerance_pct = PRICE_DIVERGENCE_TOLERANCE_PCT,
                "入场价与决策日真实收盘偏离超阈值 ⇒ 整行价格体系不可信，跳过（不修补）"
            );
            return Err(format!(
                "入场价偏离真实收盘 {dev:.1}%（>{PRICE_DIVERGENCE_TOLERANCE_PCT}%）：\
                 pick={} 真实={anchor}，目标价/止损价同源污染，整行不可用",
                reco_pick.price
            ));
        }
    } else {
        // 决策日早于全部 K 线 ⇒ 没有可比基准。如实告警，不静默视为「校验通过」。
        tracing::debug!(
            pick_id = %pick_id, stock = %reco_pick.stock_code,
            decision_date = %decision_date,
            "无锚定收盘价（决策日早于全部 K 线），本条跳过价源校验"
        );
    }
    // 数据源标识：AStockClient 内部已做 vendor failover，统一标记
    let data_source = "astock_client".to_string();
    let mut validation =
        build_pick_validation(reco_pick, pick_id, t_plus_n, &closes, &highs, &lows, &data_source);
    // build_pick_validation 只拿到 pick 的**价格**，没有生成时刻；它默认写 `now()`
    // ⇒ 每条验证行的 generated_at 都变成「回测跑批时刻」。闭环的 `predicted_at_ms`
    // （DQI 熔断按「出生时证据面」标注）与按日期匹配的分析回写都会因此错位。
    // 调用方本来就有真实 generated_at，这里必须覆盖回它。
    validation.generated_at = generated_at.to_string();
    validation.as_of_date = as_of_date.map(str::to_string);
    Ok((validation, data_source))
}

/// 智选候选行里的诊断维度（`serenity_score` / `bottleneck_composite` / `data_reliability`）。
///
/// 为什么现读现取而不是加进 `RecoPick`：现网实测 75 条 serenity 行的 `pick_data`
/// **一个都没有**这三个键（2026-10-15），而 `RecoPick` 的构造点有 20 处（多数是夹具）。
/// 本函数按 camelCase 与 snake_case 两种拼写都试 ⇒ 生产侧哪天把键写进来了，
/// 分层报表自动开始有数据，不需要再改这里；今天拿不到就如实 `None`。
fn serenity_metrics(detail: &serde_json::Value) -> (Option<f64>, Option<f64>, Option<String>) {
    let num = |keys: [&str; 2]| keys.iter().find_map(|k| detail.get(*k).and_then(|v| v.as_f64()));
    (
        num(["serenityScore", "serenity_score"]),
        num(["bottleneckComposite", "bottleneck_composite"]),
        ["dataReliability", "data_reliability"]
            .iter()
            .find_map(|k| detail.get(*k).and_then(|v| v.as_str()).map(str::to_string)),
    )
}

/// 智选（serenity）候选的 T+N **超额**命中通道（2026-10-15，PLAN-reflection-optimization-closure §四）。
///
/// 为什么它此前被一刀切排除（2026-08-01 的 P1）：智选候选没有 target_price / stop_loss，
/// 通用判据「是否到达目标价」对它不成立。但排除之后智选候选**再没有任何 T+N 数值消费者**
/// —— 连 `serenity_hit_rate_backtest` 整份模块都不在编译清单里。本通道用该模块的判据补回：
/// 同窗口收益 − 基准（沪深300）涨跌幅 ⇒ 四档超额标签，落同一张 `decision_validations`。
///
/// 产出的行**不进** `compute_hit_rate_report`：那张报表的分母只认
/// `{hit, miss, false_hit}`，智选标签会全部落进 `insufficient` 并把命中率显示成 0%，
/// 等于把「判据不适用」伪装成「一条都没命中」。它们的消费者是：
/// ① 荐股 IC 矩阵的 serenity 格（`reco_ic_stats` 读表算 rank IC —— 该格此前恒为空观测）；
/// ② 本函数返回的分层报表，由决策回测的运行摘要显示（任务中心）。
///
/// 基准取不到的行**不写**：不让「没取到基准」退化成「基准涨 0%」（那会把绝对收益
/// 冒充成超额收益）。
async fn run_serenity_backtest_pass(
    db: &sea_orm::DatabaseConnection,
    client: &std::sync::Arc<axagent_astock_data::AStockClient>,
    t_plus_n_list: &[i32],
    exclude_synthetic: bool,
    max_picks: u32,
) -> Result<(Vec<PickValidation>, SerenityHitRateReport, usize), String> {
    let mut query = reco_picks::Entity::find().filter(reco_picks::Column::Style.eq("serenity"));
    if exclude_synthetic {
        query = query.filter(reco_picks::Column::Synthetic.eq(0));
    }
    // 与通用通道同因同修（2026-10-25 A-1）：必须显式排序，去重才能「先见即最新」；
    // 否则 `take(max_picks)` 取到哪批由数据库返回顺序决定，既重复又只覆盖少数日期。
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let picks: Vec<_> = query
        .order_by_desc(reco_picks::Column::GeneratedAt)
        .all(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("读取智选候选失败: {e}"))
        })?
        .into_iter()
        .filter(|p| {
            let day = p.generated_at.split('T').next().unwrap_or(p.generated_at.as_str());
            seen_keys.insert(format!(
                "{}|{}|{}|{}",
                p.stock_code,
                p.period,
                day,
                p.as_of_date.as_deref().unwrap_or("live"),
            ))
        })
        .take(max_picks as usize)
        .collect();

    let mut rows: Vec<PickValidation> = Vec::new();
    let mut validations: Vec<SerenityPickValidation> = Vec::new();
    let mut skipped = 0usize;
    for pick_model in &picks {
        let Some(ref pick_data_str) = pick_model.pick_data else {
            skipped += 1;
            continue;
        };
        let Ok(detail) = serde_json::from_str::<serde_json::Value>(pick_data_str) else {
            tracing::warn!(pick_id = %pick_model.id, "智选 pick_data 不是 JSON ⇒ 本条不做超额验证");
            skipped += 1;
            continue;
        };
        let Ok(reco_pick) = serde_json::from_value::<RecoPick>(detail.clone()) else {
            tracing::warn!(pick_id = %pick_model.id, "智选 pick_data 不符合 RecoPick ⇒ 跳过");
            skipped += 1;
            continue;
        };
        if !reco_pick.price.is_finite() || reco_pick.price <= 0.0 {
            // 入场价非法 ⇒ 收益率无从计算（不是「收益 0」）；NaN / INF 同判非法
            skipped += 1;
            continue;
        }
        let decision_date =
            decision_date_of(&pick_model.generated_at, pick_model.as_of_date.as_deref());
        let (score, composite, reliability) = serenity_metrics(&detail);
        for &t_plus_n in t_plus_n_list {
            let window =
                fetch_window_series(client, &reco_pick.stock_code, decision_date, t_plus_n).await;
            let (closes, _highs, _lows, end_date, anchor_close) = match window {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(pick_id = %pick_model.id, t_plus_n, error = %e, "智选窗口行情不可得");
                    skipped += 1;
                    continue;
                },
            };
            // 与通用通道同一条平价校验：智选也吃同一份 `pick_data.price`，
            // 价源串代码时它的「超额」同样无意义。这里只判不修，命中即跳过。
            if anchor_close.is_some_and(|a| price_is_divergent(reco_pick.price, a)) {
                let dev = price_divergence_pct(reco_pick.price, anchor_close.unwrap_or(0.0))
                    .unwrap_or(f64::NAN);
                tracing::warn!(
                    pick_id = %pick_model.id, t_plus_n, dev_pct = dev,
                    "智选入场价与决策日真实收盘偏离超阈值 ⇒ 跳过（超额无意义）"
                );
                skipped += 1;
                continue;
            }
            let benchmark = crate::commands::stock_workflow::reflection::compute_benchmark_change(
                client,
                decision_date,
                &end_date,
            )
            .await;
            let benchmark = match benchmark {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        pick_id = %pick_model.id, t_plus_n, error = %e,
                        "基准涨跌幅不可得 ⇒ 本条不判超额（不让绝对收益冒充超额）"
                    );
                    skipped += 1;
                    continue;
                },
            };
            let v = build_serenity_validation(
                &pick_model.id,
                &reco_pick.stock_code,
                &reco_pick.stock_name,
                &pick_model.generated_at,
                t_plus_n,
                score,
                composite,
                reliability.as_deref(),
                false,
                &closes,
                Some(benchmark),
                "astock_client",
            );
            rows.push(serenity_validation_to_row(
                &v,
                reco_pick.price,
                i32::from(reco_pick.confidence),
                reco_pick.position_pct,
                &pick_model.period,
                pick_model.as_of_date.as_deref(),
            ));
            validations.push(v);
        }
    }
    let report = compute_serenity_hit_rate_report(&validations);
    Ok((rows, report, skipped))
}

/// 批量写 decision_validations（按 `(pick_id, t_plus_n, generated_at)` 幂等）
///
/// ## 幂等键为什么必须含 `generated_at`（2026-10-26 修正）
///
/// 原键是 `(pick_id, t_plus_n)`。**过粗**：同一只票、同一窗口、但**不同决策日**的样本
/// 会被判成「已存在」而跳过 ⇒ 不同决策日的独立样本被吞掉。
///
/// 实证（2026-10-26 生产库）：`decision_validations` 里同 `(pick_id, t_plus_n)`
/// 存在 402 对**决策日不同**的样本（其中 195 对收益完全相同、207 对不同）——
/// 前者是纯重复，后者是**两个不同决策日的真实样本**，被粗键混为一谈。
///
/// 反向的坑也存在：`reco_picks` 的同业务键覆盖会换掉 `pick_id`
/// （历史上是删旧行插新行 ⇒ 旧 id 消失 ⇒ 引用它的验证行全部悬空，
/// 实测 448 行 / 20.2%，且孤儿行读数系统性偏乐观）。
/// 修法在 `dao::repo::reco_pool::insert_picks`（同键覆盖**保留 id**），
/// 本函数只需保证**同一决策日**不重复写。
///
/// ⇒ 键 = `(pick_id, t_plus_n, generated_at)`：
/// `pick_id` 定位「哪条荐股」·`t_plus_n` 定位「哪个窗口」·`generated_at` 定位「哪次决策」。
async fn write_decision_validations(
    db: &sea_orm::DatabaseConnection,
    validations: &[PickValidation],
) -> Result<usize, String> {
    let now = Utc::now().to_rfc3339();
    let mut count = 0;

    for v in validations {
        // 幂等：先查 (pick_id, t_plus_n, generated_at) 是否已存在
        // ⚠ `generated_at` 是**决策日**（live = 批次键；replay = as_of 决策日），
        //   它才是「哪一次决策」的唯一标识 ⇒ 缺它会把不同决策日的样本判成重复。
        let existing = decision_validations::Entity::find()
            .filter(decision_validations::Column::PickId.eq(&v.pick_id))
            .filter(decision_validations::Column::TPlusN.eq(v.t_plus_n))
            .filter(decision_validations::Column::GeneratedAt.eq(&v.generated_at))
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
            as_of_date: Set(v.as_of_date.clone()),
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
    generation: Option<String>,
) -> Result<Vec<DecisionValidationItem>, String> {
    use sea_orm::{PaginatorTrait, QueryOrder};
    let db = state.harness.db();
    let generation = axagent_astock_data::as_of::parse_generation_param(generation.as_deref())
        .map_err(|e| ErrorResponse::new(common::INVALID_INPUT).with_detail(e))?;

    let mut query = decision_validations::Entity::find()
        // 代际过滤：默认只列**实盘**验证记录（与权重闭环同一口径）；要看某次回放的
        // 验证结果，传 `generation`=那批的截止日。两套样本不并排显示。
        .filter(match generation.as_deref() {
            Some(d) => decision_validations::Column::AsOfDate.eq(d),
            None => decision_validations::Column::AsOfDate.is_null(),
        });
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
    /// 本次读到的 `decision_validations` **全表**行数（跨代，含本轮没统计的那些代）。
    /// 本轮实际参与筛样的行数 = 进分母 + 两类排除；两者之差就是被代际切分挡在外面的行数。
    /// （「本字段 = 进分母 + 两类排除」那句自证在 `generation` 入参引入后不再成立。）
    pub total_rows: usize,
    /// 本轮报表算的是哪一代：`None` = 实盘出票，`Some(YYYY-MM-DD)` = 那一次回放的出票。
    /// 一次只出一代的报表 —— 实盘与回放的 T+N 不是同一种命中，并成一个分母会得出
    /// 「回放提升了命中率」这类假结论（AGENTS.md「两套计数域不得并排显示成可比」）。
    pub generation: Option<String>,
    /// 库里**另有**多少条回放样本没进本轮分母（`generation=None` 时才可能非零）。
    /// 给这个数是为了让「我没算它」看得见，而不是让报表看起来像「全都算了」。
    pub replay_sample_rows: usize,
    /// B3 窗口换代：多少条记录的 T+N **不属于当前四档窗口集**（旧默认 `[5,20,60]` 产的 20/60 行）
    /// 因而被挡在分母外。行不删、不回填 —— 与 `excluded_pre_floor_generation` 一样是「有数据但不可比」，
    /// 不是「没有数据」。
    pub excluded_pre_window_set_rows: usize,
}

/// 趋势智选后验的**只读**汇总（B1，`PLAN-chain-closure-remediation.md` §3）。
///
/// ## 为什么只计数、不重新判定
/// 智选行的 `hit_outcome` 由 07:00 回测里的智选趟按「T+N 对基准的超额」判完写库
/// （`excess_outperform` / `outperform` / `underperform` / `excess_underperform`）。
/// 本命令**只按已存标签聚合**：在这里再判一次命中就是第二套判据，两边一旦漂移会得到
/// 两个都自称正确的智选命中率 —— 与 `hit_outcome_to_binary_outcome` 必须做唯一判定源同理。
///
/// ## 刻意不给的数
/// 平均超额收益**不落库**（只出现在回测当轮的 tracing 里），所以本结构体没有那个字段，
/// 由界面按「未落库」如实声明。宁可少一个数，也不摆一个既没算过也没存着的数。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerenityLabelCount {
    pub label: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerenityValidationSummary {
    /// `style='serenity'` 的后验行数。同一候选按多个 T+N 窗口各落一行 ⇒ **行数 ≠ 候选数**。
    pub rows: i64,
    /// 去重后的 `pick_id` 数（与 `rows` 并排显示时这两个计数域不可互换）。
    pub candidates: i64,
    pub judged: i64,
    pub outperform: i64,
    /// `judged == 0` 时为 `None` —— 「没有可比样本」不该被显示成一枚 0% 的命中率。
    pub hit_rate: Option<f64>,
    pub label_counts: Vec<SerenityLabelCount>,
    pub latest_validated_at: Option<String>,
    pub generation: Option<String>,
    /// 恒 `false`：智选按设计**不**进荐股 IC 闭环（超额标签被 `hit_outcome_to_binary_outcome` 拒收）。
    /// 面板据此显示一句声明，而不是留一格看起来像漏算。
    pub in_weight_loop: bool,
}

#[agent_command(
    domain = "finance",
    safety = Safe,
    call_mode = StateInput,
    description = "汇总智选后验样本"
)]
#[tauri::command]
pub async fn serenity_validation_summary(
    state: State<'_, AppState>,
    generation: Option<String>,
) -> Result<SerenityValidationSummary, String> {
    let db = state.harness.db();
    let generation = axagent_astock_data::as_of::parse_generation_param(generation.as_deref())
        .map_err(|e| ErrorResponse::new(common::INVALID_INPUT).with_detail(e))?;

    let rows: Vec<(String, Option<String>, String)> = decision_validations::Entity::find()
        .filter(decision_validations::Column::Style.eq("serenity"))
        // 代际口径与 `list_decision_validations` / 权重闭环一致：默认只算实盘代，
        // 要看某次回放的验证结果就传那批的截止日。两套样本不并排比。
        .filter(match generation.as_deref() {
            Some(d) => decision_validations::Column::AsOfDate.eq(d),
            None => decision_validations::Column::AsOfDate.is_null(),
        })
        .select_only()
        .column(decision_validations::Column::PickId)
        .column(decision_validations::Column::HitOutcome)
        .column(decision_validations::Column::ValidatedAt)
        .into_tuple()
        .all(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("读取智选后验样本失败: {e}"))
        })?;

    let mut labels: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut candidates: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut judged = 0i64;
    let mut outperform = 0i64;
    let mut latest: Option<&String> = None;
    for (pick_id, outcome, validated_at) in &rows {
        candidates.insert(pick_id.clone());
        if let Some(label) = outcome.as_ref() {
            *labels.entry(label.clone()).or_insert(0) += 1;
            judged += 1;
            if label == "excess_outperform" || label == "outperform" {
                outperform += 1;
            }
        }
        if latest.map(String::as_str) < Some(validated_at.as_str()) {
            latest = Some(validated_at);
        }
    }

    Ok(SerenityValidationSummary {
        rows: rows.len() as i64,
        candidates: candidates.len() as i64,
        judged,
        outperform,
        // 分母为 0 不出数（出 0.0 会被读成「一条都没跑赢」，而那是两回事）
        hit_rate: if judged > 0 {
            Some(outperform as f64 / judged as f64)
        } else {
            None
        },
        label_counts: labels
            .into_iter()
            .map(|(label, count)| SerenityLabelCount { label, count })
            .collect(),
        latest_validated_at: latest.cloned(),
        generation,
        in_weight_loop: false,
    })
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
    generation: Option<String>,
) -> Result<ValidationReportResponse, String> {
    let db = state.harness.db();
    let generation = axagent_astock_data::as_of::parse_generation_param(generation.as_deref())
        .map_err(|e| ErrorResponse::new(common::INVALID_INPUT).with_detail(e))?;
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
            generation,
            replay_sample_rows: 0,
            excluded_pre_window_set_rows: 0,
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
                // 代际随样本一起读出：报表侧要能把回放样本与实盘样本分开算，
                // 两套计数域并排比会得出「回放提升了命中率」这类假结论。
                as_of_date: m.as_of_date,
            };
            (version, validation)
        })
        .collect();

    // 代际切分（先切代际、再筛算法代际）：报表一次只算一代。反过来会让「本轮分母从哪来」
    // 说不清 —— 两类排除的计数必须与本轮样本集同源才自证得起来。
    let replay_sample_rows = rows.iter().filter(|(_, v)| v.as_of_date.is_some()).count();
    let rows: Vec<(Option<i32>, PickValidation)> = rows
        .into_iter()
        .filter(|(_, v)| match generation.as_deref() {
            Some(d) => v.as_of_date.as_deref() == Some(d),
            None => v.as_of_date.is_none(),
        })
        .collect();
    if generation.is_none() && replay_sample_rows > 0 {
        tracing::info!(
            "[validation_report] 本轮按**实盘**口径统计；库里另有 {replay_sample_rows} 条回放\
             样本未计入分母（要那一代就传 generation=<回放截止日>）"
        );
    }

    // B3 窗口换代（拍板 6 + 7）：旧窗口集 [5,20,60] 产出的 20/60 行**不进本轮分母**。
    // 先按窗口集筛掉旧窗口；随后 `compute_period_matched_report` 再把每档限定到
    // **自己的**持有窗口（ultra_short=2 / short=5 / mid=28 / long=90），避免 2/5/28/90
    // 混成一个 by_style 分母（2026-10-24 生产实证：池化会把 capital_short 的 +3.41%
    // 与 T+28/T+90 的负收益平均成负值）。
    let excluded_pre_window_set_rows = rows
        .iter()
        .filter(|(_, v)| !axagent_harness::holding_period::Period::is_validation_window(v.t_plus_n))
        .count();
    let rows: Vec<(Option<i32>, PickValidation)> = rows
        .into_iter()
        .filter(|(_, v)| axagent_harness::holding_period::Period::is_validation_window(v.t_plus_n))
        .collect();
    if excluded_pre_window_set_rows > 0 {
        tracing::info!(
            "[validation_report] {excluded_pre_window_set_rows} 条属**上线前窗口集**（T+N 不在 \
             {:?}）⇒ 未计入分母；这些行不回填也不删，只是与当代窗口不可比",
            axagent_harness::holding_period::Period::validation_windows()
        );
    }

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
        compute_period_matched_report(&validations)
    };

    Ok(ValidationReportResponse {
        report,
        generation_floor: RECO_GENERATION_FLOOR,
        excluded_pre_floor_generation,
        excluded_unknown_generation,
        total_rows,
        generation,
        replay_sample_rows,
        excluded_pre_window_set_rows,
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
    serde_json::from_str(prompt).map_err(|e| {
        ErrorResponse::new(common::INVALID_INPUT)
            .with_detail(format!("解析 {} 配置失败: {e}", DECISION_BACKTEST_TASK_TYPE))
            .to_string()
    })
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
        // cron 路径**恒 false**：定时任务是实盘业绩的分母，混回放会把净值虚高。
        // 回放验证只在显式命令行场景（`--include-replay`）打开。
        include_replay: None,
    };
    let prompt = serde_json::to_string(&request).map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("序列化决策回测配置失败: {e}"))
    })?;

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

    let name = format!("决策回测（{windows}）");
    let mut job = axagent_runtime_core::CronJob::new(&name, &expr, &prompt, &desc)
        .with_id(id.clone())
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
