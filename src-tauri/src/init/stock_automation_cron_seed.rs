// SPDX-License-Identifier: AGPL-3.0-only

//! 股票自动化闭环内置任务的种子化（PLAN-daily-automation-pipeline C +
//! PLAN-batch-automation-mover-recall P1）
//!
//! 前五条为**交易日**流水线（休市由 executor 的交易日闸门拦截，见
//! `init/services.rs`）；第 9 条（组合指标日更）同样进闸门 —— 它的产物就是当日收盘快照，
//! 休市日跑只会把同一份旧价再落一行。其余三条（妖股核查 / 进化聚合 / 教训验证）
//! 与第 10 条（意图过期）**刻意不进闸门**，各自的执行分支注释里说明了原因：
//!
//! | 时刻 | 任务 | 产物 |
//! |---|---|---|
//! | 00:00 | 智能荐股（四档共享池扫描） | `reco_picks` 四档行 = 真实候选股票池 |
//! | 01:00 | 趋势智选（serenity-screening 工作流） | `reco_picks` style='serenity' 行 |
//! | 02:00 | 候选池逐只深度分析（全池 + 当日未分析判据） | `stock_analyses` + 反思 pending |
//! | 07:00 | 决策回测（T+N 验证历史 pick） | `decision_validations` = 荐股闭环的唯一样本源 |
//! | 07:30 | pending 交易意图自动过期 | `stock_analyses.trade_intent_status` = expired |
//! | 16:00 | 组合指标日更（收盘后） | `portfolio_metrics_daily` + `portfolio_correlation_snapshot` |
//! | 18:00 | 四周期到期反思（due_only，覆盖超短/短/中/长） | `stock_reflections` resolved |
//! | 周六 09:00 | 妖股（窗口涨幅达标）漏检核查 | `mover_recall_runs` 周度快照 |
//! | 工作日 20:00 | 进化计划聚合（反思+验证+漏检） | `stock_evolution_history` pending_approval |
//! | 工作日 21:00 | 教训有效性验证与退役 | `reflection_lessons` 的 success_count/confidence/退役态 |
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
use crate::commands::constants::{
    INTENT_EXPIRY_TASK_TYPE, LESSON_VALIDATION_TASK_TYPE, PORTFOLIO_METRICS_TASK_TYPE,
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
pub const LESSON_JOB_ID: &str = "lesson-validation-daily";
pub const PORTFOLIO_METRICS_JOB_ID: &str = "portfolio-metrics-daily";
pub const INTENT_EXPIRY_JOB_ID: &str = "intent-expiry-daily";

/// 种子化十条内置定时任务（启动时调用，失败不阻塞）。
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

/// 构造十条内置任务（时刻均为**本地时区**口径，与调度器的 Local 匹配对齐）
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

    // 8) 工作日 21:00 教训有效性验证与退役：排在 18:00 到期反思、20:00 进化聚合之后，
    //    吃当天新落的反思行与 `lesson_applications` 引用记录。
    //    **刻意不进交易日闸门**：本任务只读库（反思行 / 引用记录 / 分析 outcome），
    //    不依赖当日行情，休市日跑同样能推进退役判据。
    //    接线前 `run_lesson_validation` **没有任何触发方**（PLAN-reflection-optimization-closure §三：
    //    函数上方注释谎称已接入后台任务，实际全仓无该符号）⇒ 无效教训一直留在注入语料里，
    //    反思链只剩「加教训」没有「删教训」。
    let mut lesson = CronJob::new(
        "教训验证与退役（内置）",
        "0 21 * * 1-5",
        "",
        "统计每条教训被引用后的实际成功率，据此调整 confidence 并退役无效教训",
    )
    .with_task_type(LESSON_VALIDATION_TASK_TYPE);
    lesson.id = LESSON_JOB_ID.to_string();
    jobs.push(lesson);

    // 9) 16:00 组合指标日更：收盘后落 portfolio_metrics_daily + correlation_snapshot。
    //    这条链原先**只有面板「立即刷新」一个触发方** ⇒ 不点按钮就没有当日行，现网组合三表
    //    恒 0 行（PLAN-monitor-chain-closure 裁定 5 ①）；时间旅行读回来的空面板其实是
    //    「没人写过快照」，而不是逻辑缺失。时刻取 16:00 = 收盘（15:00）之后、18:00 反思之前。
    //    **进交易日闸门**：产物就是当日收盘净值序列，休市日跑只会把同一份旧价再落一行，
    //    还会让 90 日回撤窗口里塞满重复样本。
    //    prompt 留空 = 无参数（as_of_date 为 None ⇒ 引擎按北京会话日盖当天）。
    let mut portfolio = CronJob::new(
        "组合指标日更（内置）",
        "0 16 * * *",
        "",
        "收盘后刷新组合快照，落 portfolio_metrics_daily 与相关性矩阵，供面板时间旅行与回撤取序列",
    )
    .with_task_type(PORTFOLIO_METRICS_TASK_TYPE);
    portfolio.id = PORTFOLIO_METRICS_JOB_ID.to_string();
    jobs.push(portfolio);

    // 10) 07:30 pending 交易意图自动过期（裁定 14）。
    //     过期能力原先只挂在下线的 risk_inspection 链上（阶段 F-5 退役），此后**没有任何触发方**
    //     ⇒ pending 只增不减，待办列表里的陈旧意图与新鲜的一起参与决策。
    //     时刻排在 07:00 决策回测之后、开盘（09:30）之前：用户早上审待办时看到的已是最新过期态。
    //     **刻意不进交易日闸门**：本任务只改库里的状态列，不读当日行情，休市日跑同样在推进。
    let mut expiry = CronJob::new(
        "交易意图过期（内置）",
        "30 7 * * *",
        "",
        &format!(
            "把超过 {} 小时仍未审核的 pending 交易意图标记为 expired，避免待办只增不减",
            crate::commands::trade_intent::DEFAULT_INTENT_MAX_AGE_HOURS
        ),
    )
    .with_task_type(INTENT_EXPIRY_TASK_TYPE);
    expiry.id = INTENT_EXPIRY_JOB_ID.to_string();
    jobs.push(expiry);

    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    // 只在测试里用：生产侧只**写** prompt，读回它靠 executor 分支的同一个解析器（下方往返断言）。
    // 放在模块顶部会让 `cargo check --lib`（不含 test target）报 unused_imports。
    use crate::commands::backtest_validation::parse_decision_backtest_config;

    #[tokio::test]
    async fn seed_creates_all_builtin_jobs_and_is_idempotent() {
        let store = Arc::new(CronJobStore::new_ephemeral());
        seed_stock_automation_crons(&store, false).await.unwrap();
        assert_eq!(store.list().await.len(), 10);

        // 再跑两次：不重复建、不改写
        seed_stock_automation_crons(&store, false).await.unwrap();
        seed_stock_automation_crons(&store, false).await.unwrap();
        assert_eq!(store.list().await.len(), 10);
    }

    /// 齐备性判据（PLAN §三.5）：**种出来的每个 task_type 都必须在 executor 源码里被显式处理**。
    ///
    /// 根因形态就是「seed 有任务、services 没有对应判据」⇒ 任务到点什么都不发生且不报错
    /// （教训验证就是这么空转了一个多月）。口径刻意笨：只要求该 task_type 的**字面值或常量名**
    /// 出现在 `init/services.rs` 里（分派分支或交易日闸门判据都算被处理过），
    /// 不做语义推断 —— 会「聪明地」推断分支的判据容易自己恒真。
    #[test]
    fn every_builtin_task_type_is_handled_by_the_executor() {
        /// 把 `.with_task_type(X)` 的实参换算成 services.rs 里必须出现的形式之一。
        fn expected_forms(arg: &str) -> Vec<String> {
            let trimmed = arg.trim().trim_end_matches(')').trim();
            if let Some(rest) = trimmed.strip_prefix('"') {
                let value = rest.trim_end_matches('"').to_string();
                vec![format!("\"{value}\"")]
            } else {
                let last = trimmed.rsplit("::").next().unwrap_or(trimmed).to_string();
                vec![last]
            }
        }
        fn with_task_type_args(seed_src: &str) -> Vec<String> {
            let marker = ".with_task_type(";
            let mut out = Vec::new();
            let mut rest = seed_src;
            while let Some(i) = rest.find(marker) {
                let after = &rest[i + marker.len()..];
                match after.find(')') {
                    Some(end) => out.push(after[..end].to_string()),
                    None => break,
                }
                rest = &after[1.min(after.len())..];
            }
            out
        }

        let seed_src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/init/stock_automation_cron_seed.rs"),
        )
        .expect("读取自身源码失败");
        let services_src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/init/services.rs"),
        )
        .expect("读取 src/init/services.rs 失败");

        // 注释里的历史说明也含 task_type 字面量 ⇒ 先剥注释行（`//` 开头）再判，避免门被注释喂饱
        let strip_comments = |src: &str| -> String {
            src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
        };
        let code_lines = strip_comments(&services_src);

        // seed 侧扫描面 = 生产代码：本测试自身源码里既有 marker 字面量（`".with_task_type("`）
        // 又有文档 comment 提到它 ⇒ 不截到 `#[cfg(test)]` 之前会自指计数
        // （条数比任务数多，且多的那几条是测试自己的字面量）。
        let seed_code = strip_comments(&seed_src);
        let seed_prose = seed_code.split("#[cfg(test)]").next().unwrap_or_default();
        let args = with_task_type_args(seed_prose);
        assert_eq!(args.len(), 10, "内置任务应各有 1 个 with_task_type 实参，实得 {}", args.len());
        for arg in &args {
            let forms = expected_forms(arg);
            let handled = forms.iter().any(|f| code_lines.contains(f));
            assert!(
                handled,
                "task_type 实参 `{arg}` 在 services.rs 的代码里没有任何处置点 ⇒ 该任务到点静默不执行"
            );
        }

        // 负控：一个没人处理的 task_type 必须被判为未处置，否则本门恒真
        let orphan = expected_forms("\"lesson-validation\"");
        assert_eq!(orphan, vec!["\"lesson-validation\""]);
        let synthetic = String::from("if x { do_something() }");
        assert!(
            !orphan.iter().any(|f| synthetic.contains(f)),
            "负控失效：本门应能认出「源码里没人处理这个 task_type」"
        );
    }

    /// 齐备性判据 ②：**每条内置任务都要能在任务中心「立即执行」**。
    ///
    /// 与上一条不同的失效面：task_type 有执行分支（到点自动跑），却既漏登记
    /// `scheduled_task::BUILTIN_TASK_TYPES`、又没有 `workflow_id` ⇒ 手动触发落进分支 3 报
    /// `MANUAL_RUN_NOT_SUPPORTED`。现网 `lesson-validation` 就是这么漏了几个月。
    ///
    /// ⚠ 判据必须覆盖**两条**手动执行路径（`execute_scheduled_task` 的实际分支顺序）：
    ///   分支 1 = 带 `workflow_id`（趋势智选两条链走这条；它的 `trend-screening` task_type
    ///          只是前端归链标记，executor 没有对应分支 —— 见 `recommendation_cron.rs` 的设计注）；
    ///   分支 2 = 无工作流但 task_type 在白名单（荐股 / 池分析 / 反思 / 回测等）。
    /// 只查白名单会把分支 1 的任务判成「无法手动执行」（本版首跑即为此假红），
    /// 而为了让门变绿去把 `trend-screening` 加进白名单更错 —— 那是宣称 executor 有它
    /// 并不存在的分支，门从此失去意义。
    #[test]
    fn every_builtin_job_is_manually_executable() {
        let jobs = build_builtin_jobs().expect("内置任务构造失败");
        let allowed = crate::commands::scheduled_task::builtin_task_types();
        assert!(allowed.len() >= 10, "白名单下限异常（{allowed:?}）");
        // 两条路径的谓词 = 生产分支顺序的镜像，负控也拆在同一处守卫上。
        let manually_executable = |j: &CronJob| -> bool {
            j.workflow_id.as_deref().is_some_and(|w| !w.is_empty())
                || j.task_type.as_deref().is_some_and(|tt| allowed.contains(&tt))
        };
        for job in &jobs {
            let tt = job.task_type.as_deref().expect("内置任务必须带 task_type");
            assert!(
                manually_executable(job),
                "任务 '{}' 的 task_type `{tt}` 既不在 BUILTIN_TASK_TYPES，\
                 又没有 workflow_id 可走分支 1 ⇒ 任务中心无法手动执行",
                job.name
            );
        }

        // 负控：两条路径都拿不到 ⇒ 必须被判为不可手动执行，否则本门恒真。
        let mut orphan =
            CronJob::new("负控空壳", "0 0 * * *", "既无工作流也无内置 handler", "负控夹具")
                .with_task_type("no-such-builtin-type");
        assert!(!manually_executable(&orphan), "两条路径皆无的任务必须被拦下");
        orphan = orphan.with_workflow_id("some-workflow".to_string());
        assert!(manually_executable(&orphan), "补上 workflow_id 后必须放行（分支 1）");
        assert!(!allowed.contains(&"no-such-builtin-type"));
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

        // 教训验证：工作日 21:00、无参数、task_type 与 executor 分支同源
        // （本任务的根因正是「有实现无触发」⇒ 常量必须两侧共用一份，见 §三.1）
        let lesson = find(LESSON_JOB_ID);
        assert_eq!(lesson.task_type.as_deref(), Some(LESSON_VALIDATION_TASK_TYPE));
        assert_eq!(lesson.schedule, "0 21 * * 1-5");
        assert!(lesson.prompt.is_empty(), "教训验证无参数，prompt 应留空");

        // 组合指标日更：收盘后 16:00、无参数、task_type 与 executor 分支同源
        let portfolio = find(PORTFOLIO_METRICS_JOB_ID);
        assert_eq!(portfolio.task_type.as_deref(), Some(PORTFOLIO_METRICS_TASK_TYPE));
        assert_eq!(portfolio.schedule, "0 16 * * *", "必须在收盘（15:00）之后");
        assert!(portfolio.prompt.is_empty(), "组合日更无参数，prompt 应留空");

        // 意图过期：开盘（09:30）之前、无参数、时长取自命令面同一个常量（不另抄一份 72）
        let expiry = find(INTENT_EXPIRY_JOB_ID);
        assert_eq!(expiry.task_type.as_deref(), Some(INTENT_EXPIRY_TASK_TYPE));
        assert_eq!(expiry.schedule, "30 7 * * *");
        assert!(expiry.prompt.is_empty(), "意图过期无参数，prompt 应留空（时长由执行分支取常量）");
        assert!(
            expiry
                .description
                .contains(&crate::commands::trade_intent::DEFAULT_INTENT_MAX_AGE_HOURS.to_string()),
            "描述里的时长必须与命令面常量一致，实得 {:?}",
            expiry.description
        );
    }
}
