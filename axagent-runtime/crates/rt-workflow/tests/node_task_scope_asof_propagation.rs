// SPDX-License-Identifier: AGPL-3.0-only

//! 回归测试：`RunOptions.node_task_scope` 是**唯一**能把 per-run 上下文送进节点任务的接线。
//!
//! 背景（2026-10-11 实测）：趋势智选（serenity）命令解析了 `as_of_date`，却只在**父任务**上
//! 用裸 `task_local::scope` 包住 `run_workflow`。引擎为每个节点单独 `JoinSet::spawn`
//! （`work_engine/engine/mod.rs`），而 `tokio::task_local` **不跨 spawn 传播** ⇒ 节点内
//! `current_as_of()` 读不到截止日 ⇒ 回放静默按 live 取数。主分析链 2026-10-03（P1-8）补了
//! `node_task_scope`，智选链当时没跟上，而**全仓没有任何测试锁过这条机制** —— 于是断裂
//! 一周无人可读。本文件把机制本身钉住。
//!
//! 分层约束：`rt-workflow` 不得依赖 `axagent-astock-data`（harness 之外的实现层），
//! 所以这里用本文件内自定义的 `ASOF_PROBE` task-local 作为 as-of 上下文的**同型替身** ——
//! 被测对象是「钩子能否跨 spawn 传递 task_local」这一机制，不是 as-of 语义本身。
//!
//! 两条测试互为正负控：
//!   1. 挂钩子 ⇒ 每个节点都读得到本轮值；
//!   2. **只**在父任务上 scope（断裂原形态）⇒ 每个节点都读不到。
//!      若这条哪天变成「读得到」，说明钩子之外的隐式传播被引入了，那份隐式正是并发串台之源。

mod common;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axagent_harness::repositories::set_workflow_execution_repository;
use axagent_harness::workflow_types::{
    EdgeType, Position, RetryConfig, ToolNode, ToolNodeConfig, TriggerConfig, TriggerNode,
    TriggerType, WorkflowEdge, WorkflowNode, WorkflowNodeBase,
};
use axagent_harness::{ToolCategory, ToolContext, ToolError, ToolInfo, ToolRegistry, ToolResult};

use axagent_rt_workflow::work_engine::engine::node_task_scope_hook;
use axagent_rt_workflow::work_engine::{RunOptions, WorkEngine};

use common::{EmptyProviderRegistry, RecordingWorkflowExecutionRepo};

tokio::task_local! {
    /// 本轮运行上下文的替身（真实实现里就是 `axagent_astock_data::as_of::AS_OF`）
    static ASOF_PROBE: Option<String>;
}

const RUN_VALUE: &str = "2026-07-21";

/// 节点内读到替身上下文的状态：`visible=` / `invisible`。
fn probe_state() -> String {
    match ASOF_PROBE.try_with(|v| v.clone()) {
        Ok(Some(v)) => format!("visible={v}"),
        Ok(None) => "visible=None".to_string(),
        // task_local 不可见 == 跨了 spawn 边界且没人重挂作用域
        Err(_) => "invisible".to_string(),
    }
}

#[derive(Default)]
struct ProbeRegistry {
    seen: tokio::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl ToolRegistry for ProbeRegistry {
    fn get(&self, _name: &str) -> Option<Arc<dyn axagent_harness::Tool>> {
        None
    }
    fn find(&self, _name: &str) -> Option<Arc<dyn axagent_harness::Tool>> {
        None
    }
    fn list(&self) -> Vec<ToolInfo> {
        Vec::new()
    }
    fn list_by_category(&self, _category: ToolCategory) -> Vec<ToolInfo> {
        Vec::new()
    }
    fn is_disabled(&self, _name: &str) -> bool {
        false
    }
    async fn execute_tool(
        &self,
        name: &str,
        _input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        self.seen.lock().await.push(format!("{name}:{}", probe_state()));
        Ok(ToolResult::success("probed"))
    }
}

impl ProbeRegistry {
    async fn seen(&self) -> Vec<String> {
        self.seen.lock().await.clone()
    }
}

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

fn make_tool(id: &str, tool_name: &str) -> WorkflowNode {
    WorkflowNode::Tool(ToolNode {
        base: make_base(id, "Tool"),
        config: ToolNodeConfig {
            tool_name: tool_name.to_string(),
            input_mapping: HashMap::new(),
            output_var: "result".to_string(),
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

static REPO_ONCE: OnceLock<()> = OnceLock::new();

/// trigger -> tool_a -> tool_b 两个节点都探针（钩子必须罩住**每个**节点任务，
/// 只罩住第一个节点同样算断裂）。
async fn two_tool_engine() -> (Arc<WorkEngine>, String, Arc<ProbeRegistry>) {
    REPO_ONCE.get_or_init(|| {
        set_workflow_execution_repository(Arc::new(RecordingWorkflowExecutionRepo::default()));
    });
    let engine = Arc::new(WorkEngine::new([0u8; 32], Arc::new(EmptyProviderRegistry)));
    engine.init_dispatcher().await;
    let reg = Arc::new(ProbeRegistry::default());
    engine.set_tool_registry(reg.clone()).await;

    let wf = engine
        .create_workflow(
            "node_task_scope_probe",
            vec![
                make_trigger("trigger"),
                make_tool("tool_a", "probe_a"),
                make_tool("tool_b", "probe_b"),
            ],
            vec![make_edge("trigger", "tool_a"), make_edge("tool_a", "tool_b")],
        )
        .await
        .expect("create_workflow");
    (engine, wf.id, reg)
}

/// 正控：挂了 `node_task_scope` ⇒ 每个节点任务都读得到本轮值。
#[tokio::test]
async fn node_task_scope_carries_per_run_context_across_spawn() {
    let (engine, wf_id, reg) = two_tool_engine().await;

    let mut opts = RunOptions::new();
    opts.node_task_scope =
        Some(node_task_scope_hook(move |fut| ASOF_PROBE.scope(Some(RUN_VALUE.to_string()), fut)));

    engine.run_workflow(&wf_id, opts).await.expect("run_workflow");

    let seen = reg.seen().await;
    assert_eq!(seen.len(), 2, "两个 tool 节点都应被调用，实际: {seen:?}");
    // 钩子罩住**每个**节点任务：两个探针都必须读到本轮值
    for tool in ["probe_a", "probe_b"] {
        assert!(
            seen.contains(&format!("{tool}:visible={RUN_VALUE}")),
            "节点 {tool} 没读到本轮上下文（断裂形态）：{seen:?}"
        );
    }
}

/// 负控（断裂原形态）：只在**父任务**上 scope、没挂钩子 ⇒ 节点一律读不到。
/// 这条同时证明上一条不是恒真断言：去掉钩子就必须变红。
#[tokio::test]
async fn parent_task_scope_alone_does_not_reach_node_tasks() {
    let (engine, wf_id, reg) = two_tool_engine().await;

    let opts = RunOptions::new();
    let _ = ASOF_PROBE
        .scope(Some(RUN_VALUE.to_string()), engine.run_workflow(&wf_id, opts))
        .await
        .expect("run_workflow");

    let seen = reg.seen().await;
    assert_eq!(seen.len(), 2, "两个 tool 节点都应被调用，实际: {seen:?}");
    for entry in &seen {
        assert!(
            entry.ends_with(":invisible"),
            "没挂 node_task_scope 时节点不得读到父任务的 task_local（读到说明存在隐式传播）：{entry}"
        );
    }
}
