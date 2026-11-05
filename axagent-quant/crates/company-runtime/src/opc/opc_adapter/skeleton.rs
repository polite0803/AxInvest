// SPDX-License-Identifier: AGPL-3.0-only

//! 域包动态编排统一骨架
//!
//! 复用 `axagent-harness` 的 `DynamicSubGraph` 动态编排：14 个域包各有一个手写
//! `CapabilityPackAdapter`，本模块收敛它们**重复的编排样板**：
//! - `build_plan`：`DecompositionPlan` 构造 + `DynamicSubGraph::generate`（把股票
//!   `build_analysis_pipeline`/`build_debate_strategy` 的 plan+generate 逻辑抽取至此）
//! - `to_workflow_template_data`：`GeneratedSubGraph` → `WorkflowTemplateData`（平铺
//!   子图 nodes/edges 进模板，供 `upsert_template` 落库、`run_template_via_engine` 执行）
//! - `select_strategy_keywords`：按关键词路由 `OrchestrationStrategy` 的通用实现
//!
//! 各域 adapter `decompose_mission` 构造硬编码 `SubTask` 列表（含 role/dependencies），
//! 调用 `build_plan`；反射/进化约束等用 `..Default::default()` 降样板，按域覆写。

use axagent_harness::capability::Visibility;
use axagent_harness::capability_pack_orchestration::{
    DecompositionPlan, DynamicSubGraph, GeneratedSubGraph, OrchestrationError,
    OrchestrationStrategy, SubTask,
};
use axagent_harness::util_fns::now_ts;
use axagent_harness::workflow_types::{TriggerConfig, TriggerType, WorkflowTemplateData};

/// 从硬编码 `SubTask` 列表按策略动态生成 DAG 子图。
pub fn build_plan(
    mission: &str,
    strategy: OrchestrationStrategy,
    sub_tasks: Vec<SubTask>,
    max_parallel: u32,
    max_replans: u32,
) -> Result<GeneratedSubGraph, OrchestrationError> {
    let plan = DecompositionPlan {
        mission: mission.to_string(),
        strategy,
        sub_tasks,
        max_parallel,
        max_replans,
        replan_count: 0,
        created_at: chrono::Utc::now(),
    };
    DynamicSubGraph::new().generate(&plan)
}

/// 把动态子图平铺为可落库执行的 `WorkflowTemplateData`。
///
/// `id` 使用 `subgraph.id`（每次运行时生成、天然唯一），与固定模板
/// `{domain_pack_id}_harness_workflow` 不冲突。子图 nodes/edges 平铺进模板字段。
pub fn to_workflow_template_data(
    subgraph: &GeneratedSubGraph,
    domain_pack_id: &str,
    name: &str,
) -> WorkflowTemplateData {
    let sub = subgraph.to_workflow();
    let now = now_ts();
    WorkflowTemplateData {
        id: subgraph.id.clone(),
        name: name.to_string(),
        description: Some(format!("{domain_pack_id} 任务动态编排工作流（mission 驱动）")),
        icon: "⚙️".to_string(),
        tags: vec![domain_pack_id.to_string(), "opc".to_string()],
        version: 1,
        is_preset: false,
        is_editable: false,
        is_public: false,
        visibility: Visibility::Public,
        trigger_config: Some(TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({}),
        }),
        nodes: sub.nodes,
        edges: sub.edges,
        input_schema: None,
        output_schema: None,
        variables: Vec::new(),
        error_config: None,
        error_workflow_id: None,
        tool_defs: Vec::new(),
        mission_hash: None,
        cluster_id: None,
        route_path: None,
        hooks_config: None,
        created_at: now,
        updated_at: now,
    }
}

/// 按关键词路由编排策略（各域 adapter 可复用或按需定制）。
///
/// 命中 `辩论/多空/bull/bear/debate/争议` → `Debate`；否则默认 `Pipeline`。
pub fn select_strategy_keywords(mission: &str, debate_keywords: &[&str]) -> OrchestrationStrategy {
    let lower = mission.to_lowercase();
    for kw in debate_keywords {
        if lower.contains(kw) {
            return OrchestrationStrategy::Debate;
        }
    }
    OrchestrationStrategy::Pipeline
}

/// 默认辩论关键词表（可被各域 adapter 覆用）。
pub const DEFAULT_DEBATE_KEYWORDS: &[&str] = &["辩论", "多空", "bull", "bear", "debate", "争议"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_strategy_detects_debate() {
        assert_eq!(
            select_strategy_keywords("对方案进行多空辩论", DEFAULT_DEBATE_KEYWORDS),
            OrchestrationStrategy::Debate
        );
        assert_eq!(
            select_strategy_keywords("分析基本面", DEFAULT_DEBATE_KEYWORDS),
            OrchestrationStrategy::Pipeline
        );
    }

    #[test]
    fn build_plan_generates_subgraph() {
        let tasks =
            vec![SubTask::new("s1".into(), "步骤一".into(), "任务描述".into(), "analyst".into())];
        let g =
            build_plan("任务", OrchestrationStrategy::Ordered, tasks, 2, 1).expect("分解应成功");
        assert_eq!(g.nodes.len(), 1);
    }

    #[test]
    fn to_template_carries_subgraph_and_flat_nodes() {
        let tasks =
            vec![SubTask::new("s1".into(), "步骤一".into(), "任务描述".into(), "analyst".into())];
        let g = build_plan("任务", OrchestrationStrategy::Ordered, tasks, 2, 1).unwrap();
        let t = to_workflow_template_data(&g, "software_dev", "动态流程");
        assert_eq!(t.nodes.len(), 1);
        assert_eq!(t.id, g.id);
        assert!(t.tags.contains(&"software_dev".to_string()));
    }
}
