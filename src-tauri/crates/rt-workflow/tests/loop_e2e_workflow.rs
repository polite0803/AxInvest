// SPDX-License-Identifier: AGPL-3.0-only

//! Loop 节点端到端集成测试。
//!
//! 验证含 Loop 的工作流在 mock 环境下可以正常运行。
//! 核心修复（compute_ready_nodes 过滤 Loop body 节点）已在单元测试中验证。

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use axagent_harness::repositories::{
    set_loop_checkpoint_repository, set_workflow_execution_repository,
};
use axagent_harness::test_support::{empty_loop_checkpoint_repo, empty_workflow_execution_repo};
use axagent_harness::workflow_types::{
    CodeNode, CodeNodeConfig, EdgeType, EndNodeConfig, LoopNode, LoopNodeConfig, LoopType,
    Position, RetryConfig, ToolNode, ToolNodeConfig, TriggerConfig, TriggerNode, TriggerType,
    WorkflowEdge, WorkflowNode, WorkflowNodeBase,
};

use axagent_rt_workflow::work_engine::{RunOptions, WorkEngine};

use common::EmptyProviderRegistry;

// ── 全局初始化 ───────────────────────────────────────────────────────

static MOCK_REPOS: OnceLock<()> = OnceLock::new();
fn init_mock_repos() {
    MOCK_REPOS.get_or_init(|| {
        set_loop_checkpoint_repository(empty_loop_checkpoint_repo());
        set_workflow_execution_repository(empty_workflow_execution_repo());
    });
}

// ── 节点构造 helper ──────────────────────────────────────────────────

fn make_base(id: &str, title: &str) -> WorkflowNodeBase {
    WorkflowNodeBase {
        id: id.to_string(),
        title: title.to_string(),
        description: None,
        position: Position::default(),
        retry: RetryConfig::default(),
        timeout: Some(30),
        enabled: true,
        parent_id: None,
        compensation: None,
        continue_on_fail: false,
    }
}

fn make_trigger(id: &str) -> WorkflowNode {
    WorkflowNode::Trigger(TriggerNode {
        base: make_base(id, "Trigger"),
        config: TriggerConfig { trigger_type: TriggerType::Manual, config: serde_json::json!({}) },
    })
}

fn make_tool(id: &str, tool_name: &str, output_var: &str) -> WorkflowNode {
    WorkflowNode::Tool(ToolNode {
        base: make_base(id, "Tool"),
        config: ToolNodeConfig {
            tool_name: tool_name.to_string(),
            input_mapping: HashMap::new(),
            output_var: output_var.to_string(),
        },
    })
}

fn make_loop(id: &str, body_steps: Vec<String>) -> WorkflowNode {
    WorkflowNode::Loop(LoopNode {
        base: make_base(id, "Loop"),
        config: LoopNodeConfig {
            loop_type: LoopType::ForEach,
            items_var: None,
            iter_input_var: Some("items".to_string()),
            iteratee_var: Some("item".to_string()),
            iter_output_var: Some("iter_output".to_string()),
            partial_result_var: None,
            max_iterations: None,
            continue_condition: None,
            continue_on_error: false,
            body_steps,
            sub_graph: None,
            interrupt_after_each: false,
            interrupt_nodes: vec![],
        },
    })
}

fn make_end(id: &str) -> WorkflowNode {
    WorkflowNode::End(axagent_harness::workflow_types::EndNode {
        base: make_base(id, "End"),
        config: EndNodeConfig { output_var: None },
    })
}

/// 直接执行的 Rhai CodeNode —— 无需 provider / tool registry 即可产出可控输出。
fn make_code(
    id: &str,
    code: &str,
    output_var: &str,
    input_mapping: HashMap<String, String>,
) -> WorkflowNode {
    WorkflowNode::Code(CodeNode {
        base: make_base(id, "Code"),
        config: CodeNodeConfig {
            language: "rhai".to_string(),
            code: code.to_string(),
            output_var: output_var.to_string(),
            tool_name: None,
            execute_directly: true,
            input_mapping,
        },
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

// ── 测试：创建含 Loop 的工作流 ────────────────────────────────────────

#[tokio::test]
async fn create_workflow_with_loop_node() {
    init_mock_repos();

    let engine = Arc::new(WorkEngine::new([0u8; 32], Arc::new(EmptyProviderRegistry)));

    let nodes = vec![
        make_trigger("t1"),
        make_tool("draft-step", "draft_tool", "draft_out"),
        make_loop("loop1", vec!["draft-step".to_string()]),
        make_end("end1"),
    ];
    let edges = vec![make_edge("t1", "loop1"), make_edge("loop1", "end1")];

    let wf = engine
        .create_workflow("loop_e2e_test", nodes, edges)
        .await
        .expect("创建含 Loop 的工作流应成功");

    // 验证工作流包含所有节点
    assert_eq!(wf.nodes.len(), 4, "工作流应有 4 个节点");

    // 验证 Loop 节点的 body_steps 被正确存储
    let loop_node = wf.nodes.iter().find(|n| n.base_id() == "loop1").expect("应找到 loop1 节点");

    if let WorkflowNode::Loop(l) = loop_node {
        assert_eq!(l.config.body_steps, vec!["draft-step"]);
        assert!(matches!(l.config.loop_type, LoopType::ForEach));
    } else {
        panic!("loop1 应为 Loop 类型");
    }

    // 验证 Loop body 节点 (draft-step) 存在于工作流中
    let body_node =
        wf.nodes.iter().find(|n| n.base_id() == "draft-step").expect("应找到 draft-step 节点");
    assert!(matches!(body_node, WorkflowNode::Tool(_)), "draft-step 应为 Tool 类型");
}

// ── 测试：工作流状态管理 ──────────────────────────────────────────────

#[tokio::test]
async fn workflow_status_transitions() {
    init_mock_repos();

    let engine = Arc::new(WorkEngine::new([0u8; 32], Arc::new(EmptyProviderRegistry)));
    engine.init_dispatcher().await;

    let nodes = vec![make_trigger("t1"), make_end("end1")];
    let edges = vec![make_edge("t1", "end1")];

    let wf = engine.create_workflow("status_test", nodes, edges).await.expect("创建工作流应成功");

    let result = engine
        .run_workflow(&wf.id, axagent_rt_workflow::work_engine::RunOptions::new())
        .await
        .expect("运行工作流应成功");

    assert!(
        matches!(
            result.status,
            axagent_harness::workflow_types::WorkflowStatus::Completed
                | axagent_harness::workflow_types::WorkflowStatus::PartiallyCompleted
        ),
        "简单工作流应完成或部分完成，实际: {:?}",
        result.status
    );
}

// ── 测试：Loop 每一轮都必须真正重跑 body 并消费本轮的 iteratee ──────────
//
// 回归对象（R6）：`build_loop_body_dispatch`（engine/mod.rs）的
// `Some(NodeStatus::Completed)` 分支在 body 节点「已是 Completed 且 results 已有
// 输出」时**直接 return 既有结果**，而 LoopExecutor 在轮次之间不清理 body 节点的
// 执行状态 ⇒ 第 2..N 轮根本不执行 body，每轮都吐第 1 轮的缓存产物。
//
// 为什么既有测试拦不住：`loop_executor_integration.rs` 的所有用例都用
// `make_body_dispatch`（stub 闭包）替换了 `build_loop_body_dispatch`，绕过了
// 缺陷所在的那段引擎代码；本文件的 `create_workflow_with_loop_node` 只断言图的
// 静态结构、从不运行 ⇒ 该分支此前**没有任何测试覆盖**（测试自腐烂的又一例：
// 「有 Loop 测试但没有任何一例跑到真实引擎的 body 调度」）。
//
// 断言口径：body 把本轮 iteratee（chapter）原样回显 ⇒ 修复前得到 ["a","a","a"]。
#[tokio::test(flavor = "multi_thread")]
async fn loop_body_reruns_each_iteration() {
    init_mock_repos();

    let engine = Arc::new(WorkEngine::new([0u8; 32], Arc::new(EmptyProviderRegistry)));
    engine.init_dispatcher().await;

    // body 节点：输出 = 本轮 iteratee（chapter），使各轮结果可区分。
    // 用字符串而非数字，避免 Rhai→JSON 的整数/浮点表示差异干扰断言。
    let mut echo_mapping = HashMap::new();
    echo_mapping.insert("chapter".to_string(), "chapter".to_string());

    let nodes = vec![
        make_trigger("t1"),
        // 上游产出 3 元素数组（模拟 lc-outline 的章节数组，经 output_var 注入后
        // 由 iter_input_var = "items_out.result" 点路径取出）
        make_code("mk-items", r#"["a", "b", "c"]"#, "items_out", HashMap::new()),
        make_code("echo-step", "chapter", "chapter_echo", echo_mapping),
        WorkflowNode::Loop(LoopNode {
            base: make_base("loop1", "Loop"),
            config: LoopNodeConfig {
                loop_type: LoopType::ForEach,
                items_var: None,
                iter_input_var: Some("items_out.result".to_string()),
                iteratee_var: Some("chapter".to_string()),
                iter_output_var: Some("chapters_text".to_string()),
                partial_result_var: Some("chapters_text__partial".to_string()),
                max_iterations: None,
                continue_condition: None,
                continue_on_error: false,
                body_steps: vec!["echo-step".to_string()],
                sub_graph: None,
                interrupt_after_each: false,
                interrupt_nodes: vec![],
            },
        }),
        make_end("end1"),
    ];
    let edges = vec![
        make_edge("t1", "mk-items"),
        make_edge("mk-items", "loop1"),
        make_edge("loop1", "end1"),
    ];

    let wf = engine
        .create_workflow("loop_body_rerun_test", nodes, edges)
        .await
        .expect("创建含 Loop 的工作流应成功");

    let run =
        engine.run_workflow(&wf.id, RunOptions::new()).await.expect("运行含 Loop 的工作流应成功");

    let loop_out = run.results.get("loop1").expect("loop1 应把聚合结果写入 results");
    assert_eq!(
        loop_out.get("iter_count").and_then(|v| v.as_u64()),
        Some(3),
        "forEach 应迭代 3 次，实际输出: {loop_out}"
    );

    let items = loop_out.get("items").and_then(|v| v.as_array()).expect("items 应为数组");
    assert_eq!(items.len(), 3, "每轮产出 1 项");
    let got: Vec<&str> =
        items.iter().map(|v| v.get("result").and_then(|r| r.as_str()).unwrap_or("<无>")).collect();
    assert_eq!(
        got,
        vec!["a", "b", "c"],
        "Loop 每轮必须重新执行 body 并消费本轮 iteratee；若三轮结果相同 \
         （如 [\"a\",\"a\",\"a\"]）说明 body 节点被复用跳过、每轮返回第 1 轮缓存产物"
    );
}
