// SPDX-License-Identifier: AGPL-3.0-only

//! 工作流节点类型清单命令（P6.0-① 阶段5）。
//!
//! 把 harness 的 `NodeDescriptorRegistry` 展示元数据下发给前端，供
//! `NODE_TYPE_MAP` / 属性面板 / 左侧面板按清单渲染。新增节点类型只需
//! 注册 descriptor，主壳零改。

use axagent_agent_macro::agent_command;
use axagent_harness::workflow_node_descriptors::builtin_descriptors;
use axagent_harness::workflow_node_dyn::WorkflowNodeDescriptorDto;

/// 列出已注册的所有工作流节点类型及其展示元数据。
///
/// 只读、无外部输入 ⇒ 无失败路径；返回 `Result<_, String>` 只为满足
/// Tauri 命令签名惯例。
#[agent_command(domain = workflow, safety = Safe, call_mode = StateOnly, description = "列出已注册的工作流节点类型")]
#[tauri::command]
pub async fn list_workflow_node_descriptors() -> Result<Vec<WorkflowNodeDescriptorDto>, String> {
    Ok(builtin_descriptors()
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.display_meta())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_returns_builtin_types_with_display_meta() {
        let list = list_workflow_node_descriptors().await.unwrap();
        assert!(list.len() >= 32, "至少包含 32 个内置节点类型");
        let storage = list.iter().find(|d| d.type_name == "storage").unwrap();
        assert_eq!(storage.label_key, "workflow.nodeTypes.storage");
        assert_eq!(storage.category, "integration");
        assert_eq!(storage.hue, "pink");
        assert!(!storage.is_container);
        // 双维度类型：展示分类与引擎分类不同（引擎=Tool，展示=Container）。
        let aggregator = list.iter().find(|d| d.type_name == "aggregator").unwrap();
        assert_eq!(aggregator.kind, axagent_harness::workflow_types::NodeKind::Tool);
        assert_eq!(aggregator.display_kind, axagent_harness::workflow_types::NodeKind::Container);
        assert!(aggregator.is_container);
    }
}
