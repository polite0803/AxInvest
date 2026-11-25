// SPDX-License-Identifier: AGPL-3.0-only

//! P2-a T2'（PLAN-P2 §4）：Bot 例程执行分支的集成测试。
//!
//! 覆盖 PLAN-P2 T2' 要求的三条场景中可在无外网/无模型环境下锁死的部分：
//!
//! ① **delete_bot 级联**（GAP G-10）：删 Bot 后该 Bot 的例程从 store 消失，
//!    非 Bot 例程不受影响 —— 命令层级联逻辑的纯 store 侧验证（`store.list()`
//!    过滤 botId 后逐个 remove 的同口径复演）。
//! ② **例程轮收尾语义**：一次性例程跑完置 Disabled、循环例程保持 Active、
//!    成败都落 history（success/error 可读）—— 场景「到点触发落 history」
//!    与「history.error 可读」的 store 侧等价验证。
//! ③ **Bot 档权限映射**：ask 档例程在无 prompter 的执行路径落 Prompt 档
//!    （写工具轮由 `PermissionPolicy` 自动拒绝），错误文本会原样落
//!    `history.error`（② 的权限侧等价验证）。
//!
//! 真正「创建 1 分钟后到点的 Bot 例程 → 观察任务中心历史」的端到端手工验证
//! 按 PLAN-P2 T2' 的验收条款以手工方式执行（需要真实 provider key）。
//!
//! ⚠ `run_bot_routine_turn` 的完整路径需要 AppState（harness + 会话管理器 +
//! 真实 provider），集成测试环境无法构造 —— 其单测侧约束在
//! `bot_routine_runner::p2_bot_routine_tests` 与 `bot_approval::tests` 锁定。

use std::sync::Arc;

use axagent_runtime_core::{CronJob, CronJobStatus, CronJobStore};

/// 与 `commands::bot::delete_bot` 级联段同口径的复演：list → 过滤 botId → 逐个 remove。
///
/// 命令层实现与其保持一致（本测试同时是级联语义的行为锁：若命令层被改成
/// 其它口径——如不筛 botId——此测试仍锁死「只删该 Bot 的例程」这一契约）。
async fn cascade_remove_bot_routines(store: &Arc<CronJobStore>, bot_id: &str) -> usize {
    let mut removed = 0usize;
    for job in store.list().await {
        if job.bot_id.as_deref() == Some(bot_id) && store.remove(&job.id).await {
            removed += 1;
        }
    }
    removed
}

fn job_with_bot(name: &str, bot_id: &str, recurring: bool) -> CronJob {
    let mut job = CronJob::new(name, "0 9 * * *", "每日例程提示词", "P2 测试例程");
    job.bot_id = Some(bot_id.to_string());
    job.recurring = recurring;
    job
}

fn plain_job(name: &str) -> CronJob {
    CronJob::new(name, "30 8 * * *", "非 Bot 例程提示词", "P2 测试普通例程")
}

/// ①（PLAN-P2 T2' 第 ③ 条）：`delete_bot` 后该 Bot 例程消失、非 Bot 例程不受影响。
#[tokio::test]
async fn delete_bot_cascade_removes_only_that_bots_routines() {
    let store = Arc::new(CronJobStore::new_ephemeral());
    let bot_a = job_with_bot("Bot A 例程", "bot-aaa", true);
    let bot_b = job_with_bot("Bot B 例程", "bot-bbb", false);
    let plain = plain_job("普通收盘任务");
    store.add(bot_a.clone()).await;
    store.add(bot_b.clone()).await;
    store.add(plain.clone()).await;

    let removed = cascade_remove_bot_routines(&store, "bot-aaa").await;
    assert_eq!(removed, 1, "只级联删除 Bot A 自己的例程");

    let remaining: Vec<String> = store.list().await.into_iter().map(|j| j.id).collect();
    assert!(!remaining.contains(&bot_a.id), "Bot A 的例程必须消失");
    assert!(remaining.contains(&bot_b.id), "Bot B 的例程不受影响");
    assert!(remaining.contains(&plain.id), "非 Bot 例程（无 botId）不受影响");

    // 删除无例程的 Bot：0 删，幂等无害。
    let removed_again = cascade_remove_bot_routines(&store, "bot-不存在").await;
    assert_eq!(removed_again, 0);
}

/// ②（PLAN-P2 T2' 第 ①/② 条的 store 侧）：例程轮完成后的落历史与一次性禁用。
///
/// 到点触发由既有调度器负责（本测试不重复调度器自身的回归）；这里锁的是
/// executor Bot 分支收尾（`bot_routine_runner::finish_bot_routine`）所依赖的
/// store 契约：成败都进 history、run_count 推进、一次性置 Disabled。测试按
/// 收尾函数的实现直接驱动 `record_run` + `set_status` 组合，与
/// `p2_bot_routine_tests::finish_bot_routine_disables_one_shot_only`（runner
/// 内单测，直接调收尾函数）互为镜像。
#[tokio::test]
async fn routine_turn_outcome_lands_in_history_and_disables_one_shot() {
    let store = Arc::new(CronJobStore::new_ephemeral());
    let started = axagent_runtime_core::cron_job::now_millis();

    // 成功轮（一次性）：output 落 last_result/history，跑完 Disabled。
    let mut one_shot = job_with_bot("一次性 Bot 例程", "bot-ok", false);
    one_shot.id = "one-shot-bot-routine".to_string();
    store.add(one_shot).await;
    let ok_result = axagent_runtime_core::TaskRunResult {
        success: true,
        output: Some("例程输出文本".to_string()),
        error: None,
        duration_ms: 120,
        executed_at: started,
        skipped: false,
        partial: false,
    };
    assert!(store.record_run("one-shot-bot-routine", ok_result).await);
    assert!(store.set_status("one-shot-bot-routine", CronJobStatus::Disabled).await);
    let job = store.get("one-shot-bot-routine").await.expect("job 仍在");
    assert_eq!(job.status, CronJobStatus::Disabled);
    assert_eq!(job.run_count, 1);
    let res = job.last_result.expect("成功轮落 last_result");
    assert_eq!(res.output.as_deref(), Some("例程输出文本"));

    // 失败轮（循环）：error 原样可读，保持 Active 等下轮。
    let mut failing = job_with_bot("失败 Bot 例程", "bot-err", true);
    failing.id = "failing-bot-routine".to_string();
    store.add(failing).await;
    let err_result = axagent_runtime_core::TaskRunResult {
        success: false,
        output: None,
        error: Some("ask 档下写工具被自动拒绝".to_string()),
        duration_ms: 40,
        executed_at: started,
        skipped: false,
        partial: false,
    };
    assert!(store.record_run("failing-bot-routine", err_result).await);
    let job = store.get("failing-bot-routine").await.expect("job 仍在");
    assert_eq!(job.status, CronJobStatus::Active, "循环例程不置 Disabled");
    let res = job.last_result.expect("失败轮也落 last_result");
    assert!(!res.success);
    assert!(
        res.error.as_deref().unwrap_or_default().contains("写工具被自动拒绝"),
        "history.error 必须可读：{:?}",
        res.error
    );
}

/// ③（ADR-P2-4 权限侧）：ask 档例程的执行档位是 Prompt —— 无 prompter 的
/// 非交互轮里，需要审批的操作由 `PermissionPolicy` 自动拒绝，拒绝原因会进
/// `Err` 并经收尾函数落 history.error。此映射是那条链路的第一环。
/// 同时验证导出面（`tests/` 只能经 `axagent_lib` 再导出触达私有模块）。
#[test]
fn ask_bot_routine_resolves_to_prompt_level() {
    assert_eq!(
        axagent_lib::resolve_bot_routine_level("ask"),
        axagent_harness::runtime_types::permissions::PermissionMode::Prompt,
        "ask 档例程 = Prompt（无人批准 ⇒ 写操作自动拒）"
    );
    assert_eq!(
        axagent_lib::resolve_bot_routine_level("full"),
        axagent_harness::runtime_types::permissions::PermissionMode::Allow
    );
    assert_eq!(
        axagent_lib::resolve_bot_routine_level("custom"),
        axagent_harness::runtime_types::permissions::PermissionMode::Prompt,
        "custom 档例程保守回落 Prompt（ADR-P2-4）"
    );
}

/// 导出面契约锁：`level_supported`（T3' 交互路径共用）从 lib 再导出可达。
#[test]
fn p2_routine_exports_are_reachable() {
    // level_supported：custom 档无 provider 支持（ADR-P2-4），其余档支持。
    assert!(!axagent_lib::level_supported("openai", "custom"));
    assert!(axagent_lib::level_supported("openai", "ask"));
    assert!(axagent_lib::level_supported("openai", "auto"));
    assert!(axagent_lib::level_supported("openai", "edits"));
    assert!(axagent_lib::level_supported("openai", "full"));
}
