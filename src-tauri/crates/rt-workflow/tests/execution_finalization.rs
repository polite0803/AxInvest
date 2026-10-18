// SPDX-License-Identifier: AGPL-3.0-only

//! 执行层收尾集成测试。
//!
//! 覆盖本轮修复的两个回归点：
//!  1) 子工作流收尾：此前子工作流 run_workflow 走"收尾捷径"时只构建 output 就 return，
//!     不调用 complete_execution，导致其 DB workflow_execution 记录永远停留在初始态
//!     （孤儿 running 记录）。修复后在捷径内补一次纯 repo 的 update_workflow_execution_status。
//!  2) PartiallyCompleted 的 total_time_ms：workflow 级 completed_at 曾有一处误用毫秒
//!     （timestamp_millis），与 created_at（秒）不一致，导致
//!     total_time_ms = (completed_at - created_at) * 1000 计算出天文数字。修复后统一为秒。

mod common;

use std::sync::Arc;

use axagent_harness::repositories::{
    set_loop_checkpoint_repository, set_workflow_execution_repository,
};
use axagent_harness::test_support::empty_loop_checkpoint_repo;
use axagent_harness::workflow_types::{
    EdgeType, EndNode, EndNodeConfig, ExecutionStatus, Position, RetryConfig, TriggerConfig,
    TriggerNode, TriggerType, WorkflowEdge, WorkflowNode, WorkflowNodeBase, WorkflowStatus,
};

use axagent_rt_workflow::work_engine::{RunOptions, WorkEngine};
use common::{EmptyProviderRegistry, RecordingWorkflowExecutionRepo};

/// 本文件每个用例都要**进程级**注册自己的替身仓储
/// （`set_workflow_execution_repository` 是全局替换，不是 per-test 注入）。
/// 两个用例并行时，后注册的一方会把前一方换成自己的记录器 ⇒ 断言拿到的
/// `updates` 永远是空的（实测红在「执行行必须收到一次终态写入」）。
/// 用 `tokio::sync::Mutex`（持锁跨 await，且 `clippy.toml` 禁 std 锁）。
static REGISTRY_GUARD: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

fn registry_guard() -> &'static tokio::sync::Mutex<()> {
    REGISTRY_GUARD.get_or_init(|| tokio::sync::Mutex::new(()))
}

// ── 节点构造 helpers ─────────────────────────────────────────────────
//
// `RecordingWorkflowExecutionRepo` / `EmptyProviderRegistry` 两处 mock 原先在本文件
// 逐字重复定义，已收敛到 `tests/common/mod.rs`（去重 2026-09-14）。

fn make_base(id: &str, title: &str, enabled: bool) -> WorkflowNodeBase {
    WorkflowNodeBase {
        id: id.to_string(),
        title: title.to_string(),
        description: None,
        position: Position::default(),
        retry: RetryConfig::default(),
        timeout: Some(30),
        enabled,
        parent_id: None,
        compensation: None,
        continue_on_fail: false,
    }
}

fn make_trigger(id: &str) -> WorkflowNode {
    WorkflowNode::Trigger(TriggerNode {
        base: make_base(id, "Trigger", true),
        config: TriggerConfig { trigger_type: TriggerType::Manual, config: serde_json::json!({}) },
    })
}

fn make_end(id: &str) -> WorkflowNode {
    WorkflowNode::End(EndNode {
        base: make_base(id, "End", true),
        config: EndNodeConfig { output_var: None },
    })
}

fn make_edge(source: &str, target: &str) -> WorkflowEdge {
    WorkflowEdge {
        id: format!("e__{source}__{target}"),
        source: source.to_string(),
        source_handle: None,
        target: target.to_string(),
        target_handle: None,
        edge_type: EdgeType::Direct,
        label: None,
    }
}

// ── 测试 ─────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn sub_workflow_and_partial_finalize_persist_terminal_status() {
    // 串行化见 `REGISTRY_GUARD` 文档。
    let _registry = registry_guard().lock().await;
    let repo = Arc::new(RecordingWorkflowExecutionRepo::default());
    let updates = repo.updates.clone();
    // 全局 registry 覆盖式注入；单测试函数内顺序执行，无并行 set 冲突。
    set_workflow_execution_repository(repo);
    set_loop_checkpoint_repository(empty_loop_checkpoint_repo());

    let engine = Arc::new(WorkEngine::new([0u8; 32], Arc::new(EmptyProviderRegistry)));
    engine.init_dispatcher().await;

    // ── 场景 A：子工作流收尾（parent_execution_id 非空 → 收尾捷径）──
    let sub_wf = engine
        .create_workflow(
            "sub_finalize_test",
            vec![make_trigger("t1"), make_end("end1")],
            vec![make_edge("t1", "end1")],
        )
        .await
        .expect("创建子工作流测试用工作流应成功");
    let sub_result = engine
        .run_workflow(
            &sub_wf.id,
            RunOptions { parent_execution_id: Some("parent-1".to_string()), ..RunOptions::new() },
        )
        .await
        .expect("子工作流运行应成功");
    assert!(
        matches!(sub_result.status, WorkflowStatus::Completed | WorkflowStatus::PartiallyCompleted),
        "简单子工作流应完成/部分完成，实际: {:?}",
        sub_result.status
    );

    // 断言子工作流的 DB 收尾确实写入终态（而非停留在初始 running → 无 update 记录）。
    let mut sub_seen_final = false;
    for (_, status, total_time) in updates.lock().await.iter() {
        if status == "completed" {
            let t = total_time.expect("completed 收尾必须写入 total_time_ms");
            assert!((0..3_600_000).contains(&t), "total_time_ms 应为合理毫秒值，实际: {t}");
            sub_seen_final = true;
        }
    }
    assert!(sub_seen_final, "子工作流 DB 记录应收到 completed 终态");

    // ── 场景 B：PartiallyCompleted 时 total_time_ms 合理 ──
    // 正常 t1→end1 外加一个孤立 disabled 节点。disabled 节点初始 Pending 但不参与
    // DAG 就绪调度，主链完成后死锁检测将其置 Skipped，从而 all_ok && any_skipped。
    let disabled_trigger = WorkflowNode::Trigger(TriggerNode {
        base: make_base("disabled1", "Disabled", false),
        config: TriggerConfig { trigger_type: TriggerType::Manual, config: serde_json::json!({}) },
    });
    let partial_wf = engine
        .create_workflow(
            "partial_finalize_test",
            vec![make_trigger("t1"), make_end("end1"), disabled_trigger],
            vec![make_edge("t1", "end1")],
        )
        .await
        .expect("创建部分完成测试用工作流应成功");
    let partial_result = engine
        .run_workflow(&partial_wf.id, RunOptions::new())
        .await
        .expect("部分完成工作流运行应成功");
    assert_eq!(
        partial_result.status,
        WorkflowStatus::PartiallyCompleted,
        "含孤立 disabled 节点的工作流应部分完成，实际: {:?}",
        partial_result.status
    );

    // 断言 PartiallyCompleted 的 DB 收尾 total_time_ms 合理（非天文数字，即非毫秒×秒混算）。
    let mut partial_seen_final = false;
    for (_, status, total_time) in updates.lock().await.iter() {
        if status == "partially_completed" {
            let t = total_time.expect("partially_completed 收尾必须写入 total_time_ms");
            assert!(
                (0..3_600_000).contains(&t),
                "PartiallyCompleted total_time_ms 应为合理毫秒值，实际: {t}"
            );
            partial_seen_final = true;
        }
    }
    assert!(partial_seen_final, "PartiallyCompleted DB 记录应收到 partially_completed 终态");
}

/// 内存里没有这次执行 ⇒ **仍必须把终态写进库**。旧写法在这里返回 `NotFound`、调用点又用
/// `.ok()` 吞掉，于是没人再写这行的终态 —— 现网后果：一条分析已完成并落库，它对应的
/// `workflow_executions` 行却永久停在 `running`、`total_time_ms=0`（全表累积 85 条），
/// 历史列表一直转圈，批量评估还会把这些行当样本抽走。
///
/// 反向锁：把 `complete_execution` 的 else 分支改回 `Err(NotFound)` ⇒ 本条既拿不到
/// `Ok(())`、也看不到任何 update 记录，两处都红。
#[tokio::test]
async fn complete_execution_persists_terminal_status_when_memory_state_is_gone() {
    // 串行化见 `REGISTRY_GUARD` 文档。
    let _registry = registry_guard().lock().await;
    let repo = Arc::new(RecordingWorkflowExecutionRepo::default());
    let updates = repo.updates.clone();
    set_workflow_execution_repository(repo);
    set_loop_checkpoint_repository(empty_loop_checkpoint_repo());
    let engine = WorkEngine::new([0u8; 32], Arc::new(EmptyProviderRegistry));

    // 关键前提：这个 id 从未在本引擎登记过（不调 run_workflow）⇒ 内存 map 里必不存在，
    // 走的正是「内存丢了、库里还在」那条现实路径。
    engine
        .complete_execution(
            "ghost-execution-id",
            &serde_json::json!({ "ok": true }),
            4_321,
            ExecutionStatus::Completed,
        )
        .await
        .expect("内存缺失时也要写库终态，不得再返回 NotFound");

    let log = updates.lock().await;
    let hit = log.iter().find(|(id, _, _)| id == "ghost-execution-id");
    let (_, status, total_time) = hit.expect("执行行必须收到一次终态写入");
    assert_eq!(status, "completed", "终态应为 completed，实际: {status}");
    assert_eq!(*total_time, Some(4_321), "耗时必须照写 —— 留 0 就是现网幽灵行的那个形态");
}
