// SPDX-License-Identifier: AGPL-3.0-only

//! 32 个内置节点类型的契约实现（P6.0-① 阶段1）。
//!
//! 由宏批量生成，字段口径与 `AUDIT-workflow-node-open-baseline-2026-11-17.md` 一致。
//! 阶段1 只注册、不替换 enum；本文件提供 descriptor 供共存期与后续迁移使用。

use serde_json::Value;

use crate::workflow_node_dyn::WorkflowNodeDescriptor;
use crate::workflow_types::{NodeKind, *};

/// 生成内置节点 descriptor：config 强类型解析 + 分类 + output_var 提取。
/// 第 5 个参数是 output_var 模式 token（`str` = 有 outputVar 字段，`none` = 无）。
macro_rules! builtin_descriptor {
    // 有 outputVar 字段的节点类型
    ($struct:ident, $tag:literal, $cfg:ty, $kind:expr, str) => {
        pub struct $struct;
        impl WorkflowNodeDescriptor for $struct {
            fn type_name(&self) -> &'static str {
                $tag
            }
            fn kind(&self) -> NodeKind {
                $kind
            }
            fn parse_config(&self, raw: &Value) -> Result<(), String> {
                serde_json::from_value::<$cfg>(raw.clone())
                    .map(|_| ())
                    .map_err(|e| format!("节点 `{}` config 非法: {e}", $tag))
            }
            fn output_var(&self, raw: &Value) -> Option<String> {
                raw.get("outputVar").and_then(|v| v.as_str()).map(|s| s.to_string())
            }
        }
    };
    // 无 outputVar 字段的节点类型
    ($struct:ident, $tag:literal, $cfg:ty, $kind:expr, none) => {
        pub struct $struct;
        impl WorkflowNodeDescriptor for $struct {
            fn type_name(&self) -> &'static str {
                $tag
            }
            fn kind(&self) -> NodeKind {
                $kind
            }
            fn parse_config(&self, raw: &Value) -> Result<(), String> {
                serde_json::from_value::<$cfg>(raw.clone())
                    .map(|_| ())
                    .map_err(|e| format!("节点 `{}` config 非法: {e}", $tag))
            }
            fn output_var(&self, _raw: &Value) -> Option<String> {
                None
            }
        }
    };
}

builtin_descriptor!(TriggerDescriptor, "trigger", TriggerConfig, NodeKind::Input, none);
builtin_descriptor!(AgentDescriptor, "agent", AgentNodeConfig, NodeKind::Agent, str);
builtin_descriptor!(LlmDescriptor, "llm", LLMNodeConfig, NodeKind::Agent, none);
builtin_descriptor!(
    ConditionDescriptor,
    "condition",
    ConditionNodeConfig,
    NodeKind::Condition,
    none
);
builtin_descriptor!(ParallelDescriptor, "parallel", ParallelNodeConfig, NodeKind::Container, none);
builtin_descriptor!(LoopDescriptor, "loop", LoopNodeConfig, NodeKind::Loop, none);
builtin_descriptor!(MergeDescriptor, "merge", MergeNodeConfig, NodeKind::Container, none);
builtin_descriptor!(DelayDescriptor, "delay", DelayNodeConfig, NodeKind::Tool, none);
builtin_descriptor!(ValidationDescriptor, "validation", ValidationNodeConfig, NodeKind::Tool, none);
builtin_descriptor!(
    SubWorkflowDescriptor,
    "subWorkflow",
    SubWorkflowNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(
    WorkflowRefDescriptor,
    "workflowRef",
    WorkflowRefNodeConfig,
    NodeKind::Tool,
    none
);
builtin_descriptor!(
    DocumentParserDescriptor,
    "documentParser",
    DocumentParserNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(
    VectorRetrieveDescriptor,
    "vectorRetrieve",
    VectorRetrieveNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(EndDescriptor, "end", EndNodeConfig, NodeKind::Output, str);
builtin_descriptor!(
    HttpRequestDescriptor,
    "httpRequest",
    HttpRequestNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(SwitchDescriptor, "switch", SwitchNodeConfig, NodeKind::Condition, none);
builtin_descriptor!(
    DatabaseQueryDescriptor,
    "databaseQuery",
    DatabaseQueryNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(
    NotificationDescriptor,
    "notification",
    NotificationNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(ApprovalDescriptor, "approval", ApprovalNodeConfig, NodeKind::Tool, str);
builtin_descriptor!(
    FileOperationDescriptor,
    "fileOperation",
    FileOperationNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(
    DataTransformerDescriptor,
    "dataTransformer",
    DataTransformerNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(
    WebhookSendDescriptor,
    "webhookSend",
    WebhookSendNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(LoggingDescriptor, "logging", LoggingNodeConfig, NodeKind::Tool, str);
builtin_descriptor!(
    LlmClassifierDescriptor,
    "llmClassifier",
    LlmClassifierNodeConfig,
    NodeKind::Tool,
    str
);
builtin_descriptor!(AggregatorDescriptor, "aggregator", AggregatorNodeConfig, NodeKind::Tool, str);
builtin_descriptor!(EmailDescriptor, "email", EmailNodeConfig, NodeKind::Tool, str);
builtin_descriptor!(DebateDescriptor, "debate", DebateNodeConfig, NodeKind::Container, str);
builtin_descriptor!(SwarmDescriptor, "swarm", SwarmNodeConfig, NodeKind::Container, str);
builtin_descriptor!(
    MultiAgentDescriptor,
    "multiAgent",
    MultiAgentNodeConfig,
    NodeKind::Container,
    str
);
builtin_descriptor!(StorageDescriptor, "storage", StorageNodeConfig, NodeKind::Storage, str);
builtin_descriptor!(ToolDescriptor, "tool", ToolNodeConfig, NodeKind::Tool, str);
builtin_descriptor!(CodeDescriptor, "code", CodeNodeConfig, NodeKind::Tool, str);

/// 注册全部 32 个内置节点契约到给定注册表。
pub fn register_builtin_descriptors(registry: &crate::workflow_node_dyn::NodeDescriptorRegistry) {
    registry.register(TriggerDescriptor);
    registry.register(AgentDescriptor);
    registry.register(LlmDescriptor);
    registry.register(ConditionDescriptor);
    registry.register(ParallelDescriptor);
    registry.register(LoopDescriptor);
    registry.register(MergeDescriptor);
    registry.register(DelayDescriptor);
    registry.register(ValidationDescriptor);
    registry.register(SubWorkflowDescriptor);
    registry.register(WorkflowRefDescriptor);
    registry.register(DocumentParserDescriptor);
    registry.register(VectorRetrieveDescriptor);
    registry.register(EndDescriptor);
    registry.register(HttpRequestDescriptor);
    registry.register(SwitchDescriptor);
    registry.register(DatabaseQueryDescriptor);
    registry.register(NotificationDescriptor);
    registry.register(ApprovalDescriptor);
    registry.register(FileOperationDescriptor);
    registry.register(DataTransformerDescriptor);
    registry.register(WebhookSendDescriptor);
    registry.register(LoggingDescriptor);
    registry.register(LlmClassifierDescriptor);
    registry.register(AggregatorDescriptor);
    registry.register(EmailDescriptor);
    registry.register(DebateDescriptor);
    registry.register(SwarmDescriptor);
    registry.register(MultiAgentDescriptor);
    registry.register(StorageDescriptor);
    registry.register(ToolDescriptor);
    registry.register(CodeDescriptor);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_node_dyn::NodeDescriptorRegistry;
    #[test]
    fn registers_all_32_builtin_types() {
        let reg = NodeDescriptorRegistry::new();
        register_builtin_descriptors(&reg);
        let mut names = reg.type_names();
        names.sort();
        let mut expected = vec![
            "trigger",
            "agent",
            "llm",
            "condition",
            "parallel",
            "loop",
            "merge",
            "delay",
            "validation",
            "subWorkflow",
            "workflowRef",
            "documentParser",
            "vectorRetrieve",
            "end",
            "httpRequest",
            "switch",
            "databaseQuery",
            "notification",
            "approval",
            "fileOperation",
            "dataTransformer",
            "webhookSend",
            "logging",
            "llmClassifier",
            "aggregator",
            "email",
            "debate",
            "swarm",
            "multiAgent",
            "storage",
            "tool",
            "code",
        ];
        expected.sort();
        assert_eq!(names, expected, "descriptor 类型集合与序列化基线不符");
    }
    #[test]
    fn descriptors_report_kind_and_type() {
        let reg = NodeDescriptorRegistry::new();
        register_builtin_descriptors(&reg);
        assert_eq!(reg.get("trigger").unwrap().kind(), NodeKind::Input);
        assert_eq!(reg.get("agent").unwrap().kind(), NodeKind::Agent);
        assert_eq!(reg.get("storage").unwrap().kind(), NodeKind::Storage);
    }
}
