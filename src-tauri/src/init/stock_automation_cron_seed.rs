// SPDX-License-Identifier: AGPL-3.0-only

//! 股票自动化闭环内置任务的种子化（PLAN-daily-automation-pipeline C +
//! PLAN-batch-automation-mover-recall P1）
//!
//! 前五条为**交易日**流水线（休市由 executor 的交易日闸门拦截，见
//! `init/services.rs`）；后两条（妖股核查 / 进化聚合）**刻意不进交易日闸门**，
//! 各自的执行分支注释里说明了原因：
//!
//! | 时刻 | 任务 | 产物 |
//! |---|---|---|
//! | 00:00 | 智能荐股（四档共享池扫描） | `reco_picks` 四档行 = 真实候选股票池 |
//! | 01:00 | 趋势智选（serenity-screening 工作流） | `reco_picks` style='serenity' 行 |
//! | 02:00 | 候选池逐只深度分析（全池 + 当日未分析判据） | `stock_analyses` + 反思 pending |
//! | 07:00 | 决策回测（T+N 验证历史 pick） | `decision_validations` = 荐股闭环的唯一样本源 |
//! | 18:00 | 四周期到期反思（due_only，覆盖超短/短/中/长） | `stock_reflections` resolved |
//! | 周六 09:00 | 妖股（窗口涨幅达标）漏检核查 | `mover_recall_runs` 周度快照 |
//! | 工作日 20:00 | 进化计划聚合（反思+验证+漏检） | `stock_evolution_history` pending_approval |
//!
//! 种子化语义：**存在即跳过，不改**（不同于 `seed_opc_cron` 的 upsert-overwrite）——
//! 用户在定时任务面板改过时刻/阈值后，重启不得被静默回滚。
//! 幂等按固定 job id 判（`overwrite=true` 仅供测试重置）。

use axagent_analysis_engine::recommender::Period;
use axagent_runtime_core::{CronJob, CronJobStore};
use std::sync::Arc;

use crate::commands::backtest_validation::{
    DECISION_BACKTEST_TASK_TYPE, RunDecisionBacktestRequest,
};
use crate::commands::evolution_proposal::EVOLUTION_PROPOSAL_TASK_TYPE;
use crate::commands::mover_recall::MOVER_RECALL_TASK_TYPE;
use crate::commands::pool_scan::{POOL_SCAN_TASK_TYPE, PoolScanConfig};
use crate::commands::recommendation_cron::{
    RecoCronConfig, TREND_SCREENING_TASK_TYPE, TREND_SCREENING_WORKFLOW_ID,
};
use crate::commands::stock_workflow::BatchReflectionConfig;

/// 任务固定 ID（幂等键，勿改 —— 改了会在用户库里长出第二份）
pub const RECO_JOB_ID: &str = "stock-reco-daily";
pub const TREND_JOB_ID: &str = "trend-screening-daily";
pub const POOL_SCAN_JOB_ID: &str = "pool-scan-daily";
pub const REFLECTION_JOB_ID: &str = "reflection-due-daily";
pub const DECISION_BACKTEST_JOB_ID: &str = "decision-backtest-daily";
pub const MOVER_RECALL_JOB_ID: &str = "mover-recall-weekly";
pub const EVOLUTION_PROPOSAL_JOB_ID: &str = "evolution-proposal-daily";

/// 种子化七条内置定时任务（启动时调用，失败不阻塞）。
///
/// `overwrite = false`（生产）：已存在即跳过；
/// `overwrite = true`（测试重置用）：先删同名再建。
pub async fn seed_stock_automation_crons(
    store: &Arc<CronJobStore>,
    overwrite: bool,
) -> Result<(), String> {
    let jobs = build_builtin_jobs()?;
    for job in jobs {
        if store.get(&job.id).await.is_some() {
            if !overwrite {
                tracing::info!("[stock-automation-cron] 任务已存在，保留用户配置: {}", job.id);
                continue;
            }
            store.remove(&job.id).await;
        }
        let id = job.id.clone();
        store.add(job).await;
        tracing::info!("[stock-automation-cron] 已创建内置任务 {id}");
    }
    Ok(())
}

/// 构造七条内置任务（时刻均为**本地时区**口径，与调度器的 Local 匹配对齐）
///
/// ⚠ `CronJob::new` 的首参是 **name**，`id` 由 uuid 现生成（crates/runtime-core/src/cron_job.rs:196-199）⇒
/// 幂等键必须显式赋 `job.id`（同 `seed_opc_cron.rs` 的形态），否则每次启动长出新的任务。
fn build_builtin_jobs() -> Result<Vec<CronJob>, String> {
    let mut jobs = Vec::new();

    // 1) 00:00 智能荐股：四档全跑，逐档落 reco_picks（候选池的持久化形态）
    let reco_config = RecoCronConfig {
        periods: vec![Period::UltraShort, Period::Short, Period::Mid, Period::Long],
        // 与模板变量 `reco_min_confidence` 出厂默认一致（超短档置信经反身性折扣落在
        // 50~64 区间，60 会静默挡掉整档，见 recommendation_cron.rs 同款注释）
        min_confidence: 50,
        top_n: 5,
    };
    let mut reco = CronJob::new(
        "智能荐股（内置）",
        "0 0 * * *",
        &serde_json::to_string(&reco_config).map_err(|e| e.to_string())?,
        "四档（超短/短/中/长）共享池扫描，产物逐档落候选池 reco_picks",
    )
    .with_task_type("stock-recommendation");
    reco.id = RECO_JOB_ID.to_string();
    jobs.push(reco);

    // 2) 01:00 趋势智选：executor 走 workflow_id 兜底分支（trend-screening
    //    task_type 只是前端列表归链标记），产物落 reco_picks style='serenity'
    let mut trend = CronJob::new(
        "趋势智选（内置）",
        "0 1 * * *",
        "定时运行趋势智选工作流，产物写入候选池（reco_picks）",
        "运行 serenity-screening 工作流，产物进候选池供 02:00 池分析消费",
    )
    .with_task_type(TREND_SCREENING_TASK_TYPE)
    .with_workflow_id(TREND_SCREENING_WORKFLOW_ID.to_string());
    trend.id = TREND_JOB_ID.to_string();
    jobs.push(trend);

    // 3) 02:00 候选池逐只深度分析：全池 + 「当日未分析」判据 + 不截断
    //    （PoolScanConfig::default() 即 {lookbackDays:0, maxStocks:0, minConfidence:50}）
    let mut pool = CronJob::new(
        "候选池分析（内置）",
        "0 2 * * *",
        &PoolScanConfig::default().to_json()?,
        "对候选池中当日尚未分析的股票依次执行完整分析工作流（跨日积压自动补）",
    )
    .with_task_type(POOL_SCAN_TASK_TYPE);
    pool.id = POOL_SCAN_JOB_ID.to_string();
    jobs.push(pool);

    // 4) 18:00 四周期到期反思：period=None + dueOnly=true ⇒ 一条任务覆盖四档
    //    （到期判据 hindsight_date<=今日 已下推到 SQL，配额不被未到期积压消耗）
    let reflection_config = BatchReflectionConfig {
        period: None,
        due_only: true,
        // 配额给足：日流量（四档×topN + serenity）远小于此，且到期集合已被 SQL 收窄
        max_count: Some(200),
    };
    let mut reflection = CronJob::new(
        "到期反思（内置）",
        "0 18 * * *",
        &reflection_config.to_json()?,
        "遍历达到超短/短/中/长各档持有期且未反思的历史分析记录，逐条反思",
    )
    .with_task_type("batch-reflection");
    reflection.id = REFLECTION_JOB_ID.to_string();
    jobs.push(reflection);

    // 5) 07:00 决策回测：按 T+N 验证历史 pick，写 `decision_validations`。
    //    这张表是荐股链数值闭环（rank IC → 逐档权重）的**唯一**样本来源，而写入方
    //    只有 `run_decision_backtest` 一条路。此前该任务只能由人在面板手工创建 ⇒
    //    现网 `cron_jobs` 里根本没有它，`decision_validations` 恒 0 行、闭环空转
    //    （不是逻辑缺失：2026-09-20 已把执行分支接好，缺的是没人被播种出来）。
    //    时刻取 07:00 = `create_decision_backtest_cron` 的出厂默认，避免两种口径。
    let backtest_config = RunDecisionBacktestRequest {
        // 窗口 / 上限 / 排除 synthetic 均留 None ⇒ 用命令层的出厂默认（T+5/20/60、200 条）
        t_plus_n_list: None,
        exclude_synthetic: None,
        max_picks: None,
        // dry_run 恒 false（与 create 命令同口径）：定时任务算完不落库 = 零产物
        dry_run: Some(false),
        period_filter: None,
    };
    let mut backtest = CronJob::new(
        "决策回测（内置）",
        "0 7 * * *",
        &serde_json::to_string(&backtest_config).map_err(|e| e.to_string())?,
        "按 reco_picks 做 T+N 验证并落 decision_validations，供荐股 IC 与逐档权重取样本",
    )
    .with_task_type(DECISION_BACKTEST_TASK_TYPE);
    backtest.id = DECISION_BACKTEST_JOB_ID.to_string();
    jobs.push(backtest);

    // 6) 周六 09:00 妖股漏检核查：消费周五已落库的收盘快照，不依赖当日行情。
    //    **不走交易日闸门**（周六非交易日，进闸门必被拦，见 services.rs 分支注释）。
    //    prompt 留空：核查无参数（终点取最新快照日，全档位全窗口）。
    let mut mover = CronJob::new(
        "妖股漏检核查（内置）",
        "0 9 * * 6",
        "",
        "核查窗口涨幅达标事件被各档荐股漏检的比例，落 mover_recall_runs 周度快照",
    )
    .with_task_type(MOVER_RECALL_TASK_TYPE);
    mover.id = MOVER_RECALL_JOB_ID.to_string();
    jobs.push(mover);

    // 7) 工作日 20:00 进化计划聚合：在 18:00 到期反思之后，聚合近 24h 的
    //    反思 / 决策验证 / 妖股漏检样本，有样本才产出 pending_approval 计划，
    //    **不执行任何变异**（审批接受在 P4）。prompt 留空 = 默认 24h 窗口。
    let mut proposal = CronJob::new(
        "进化计划聚合（内置）",
        "0 20 * * 1-5",
        "",
        "聚合近 24h 反思/验证/漏检样本，生成待审批的参数进化计划（不自动执行）",
    )
    .with_task_type(EVOLUTION_PROPOSAL_TASK_TYPE);
    proposal.id = EVOLUTION_PROPOSAL_JOB_ID.to_string();
    jobs.push(proposal);

    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    // 只在测试里用：生产侧只**写** prompt，读回它靠 executor 分支的同一个解析器（下方往返断言）。
    // 放在模块顶部会让 `cargo check --lib`（不含 test target）报 unused_imports。
    use crate::commands::backtest_validation::parse_decision_backtest_config;

    #[tokio::test]
    async fn seed_creates_seven_jobs_and_is_idempotent() {
        let store = Arc::new(CronJobStore::new_ephemeral());
        seed_stock_automation_crons(&store, false).await.unwrap();
        assert_eq!(store.list().await.len(), 7);

        // 再跑两次：不重复建、不改写
        seed_stock_automation_crons(&store, false).await.unwrap();
        seed_stock_automation_crons(&store, false).await.unwrap();
        assert_eq!(store.list().await.len(), 7);
    }

    #[tokio::test]
    async fn seed_preserves_user_modifications() {
        let store = Arc::new(CronJobStore::new_ephemeral());
        seed_stock_automation_crons(&store, false).await.unwrap();
        // 用户把池分析改到 03:00
        let job_id = POOL_SCAN_JOB_ID.to_string();
        store.update(&job_id, |j| j.schedule = "0 3 * * *".to_string()).await;
        // 重启种子化：不得回滚
        seed_stock_automation_crons(&store, false).await.unwrap();
        let job = store.get(&job_id).await.expect("任务应存在");
        assert_eq!(job.schedule, "0 3 * * *", "用户配置被种子化回滚");
    }

    #[tokio::test]
    async fn job_configs_roundtrip_through_executor_parsers() {
        let jobs = build_builtin_jobs().unwrap();
        let find = |id: &str| jobs.iter().find(|j| j.id == id).expect("任务存在");

        let reco = find(RECO_JOB_ID);
        let cfg = RecoCronConfig::from_json(&reco.prompt).unwrap();
        assert_eq!(cfg.periods.len(), 4);
        assert_eq!(cfg.min_confidence, 50);

        let pool = find(POOL_SCAN_JOB_ID);
        let cfg = PoolScanConfig::from_json(&pool.prompt).unwrap();
        assert_eq!(cfg.lookback_days, 0);
        assert_eq!(cfg.max_stocks, 0);

        let refl = find(REFLECTION_JOB_ID);
        let cfg = BatchReflectionConfig::from_json(&refl.prompt).unwrap();
        assert!(cfg.due_only);
        assert!(cfg.period.is_none());
        assert_eq!(cfg.max_count, Some(200));

        let trend = find(TREND_JOB_ID);
        assert_eq!(trend.workflow_id.as_deref(), Some(TREND_SCREENING_WORKFLOW_ID));
        assert_eq!(trend.task_type.as_deref(), Some(TREND_SCREENING_TASK_TYPE));

        // 决策回测：配置必须能被 executor 分支的解析器读回，且 dryRun 显式为 false
        // （定时任务算完不落库 = 零产物，三问里的第③问）
        let backtest = find(DECISION_BACKTEST_JOB_ID);
        assert_eq!(backtest.task_type.as_deref(), Some(DECISION_BACKTEST_TASK_TYPE));
        let cfg = parse_decision_backtest_config(&backtest.prompt).unwrap();
        assert_eq!(cfg.dry_run, Some(false), "内置决策回测任务不得配成零产物");
        assert!(
            cfg.t_plus_n_list.is_none() && cfg.max_picks.is_none() && cfg.period_filter.is_none(),
            "内置任务用出厂默认，播种处不该夹带口径覆写"
        );

        // 妖股核查：周六 09:00、无配置、task_type 与 executor 分支对齐
        let mover = find(MOVER_RECALL_JOB_ID);
        assert_eq!(mover.task_type.as_deref(), Some(MOVER_RECALL_TASK_TYPE));
        assert_eq!(mover.schedule, "0 9 * * 6");
        assert!(mover.prompt.is_empty(), "妖股核查无参数，prompt 应留空");

        // 进化聚合：工作日 20:00、空配置可被解析器读成默认 24h 窗口
        let proposal = find(EVOLUTION_PROPOSAL_JOB_ID);
        assert_eq!(proposal.task_type.as_deref(), Some(EVOLUTION_PROPOSAL_TASK_TYPE));
        assert_eq!(proposal.schedule, "0 20 * * 1-5");
        let cfg = crate::commands::evolution_proposal::EvolutionProposalConfig::from_json(
            &proposal.prompt,
        )
        .unwrap();
        assert_eq!(cfg.window_hours_or_default(), 24);
    }
}
