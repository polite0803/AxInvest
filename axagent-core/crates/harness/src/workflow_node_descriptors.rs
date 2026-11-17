// SPDX-License-Identifier: AGPL-3.0-only

//! 32 个内置节点类型的契约实现（P6.0-① 阶段1）。
//!
//! 由宏批量生成，字段口径与 `AUDIT-workflow-node-open-baseline-2026-11-17.md` 一致。
//! 阶段1 只注册、不替换 enum；本文件提供 descriptor 供共存期与后续迁移使用。

use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::workflow_node_dyn::{NodeDescriptorRegistry, WorkflowNodeDescriptor};
use crate::workflow_types::{NodeKind, *};

// ── 展示元数据映射（P6.0-① 阶段5）────────────────────────────────────────
//
// `kind()` 是引擎调度分类，`display_kind()` 是编辑器配色 / 容器判定分类，
// 二者是两个不同维度。下表逐字对齐原前端 `NODE_KIND_MAP` 与 `NODE_TYPE_MAP`：
// - 显式列出 32 类型的展示分类（含与引擎 kind 相同的项，便于逐行比对旧表），
//   未命中回退 `kind()`；单测锁住 32 类型完整形状。
//
// ⚠ 展示分类 ≠ 引擎分类：11 个类型二者不同（如 `aggregator` 引擎=Tool、
//   展示=Container；`approval` 引擎=Tool、展示=Output）。不可用 `kind()` 代替。

/// 展示分类映射：显式列出展示分类≠引擎分类的类型 + 少量需固定展示分类的类型，
/// 未命中回退 `kind()`（如 `tool` / `code` / `delay` 等引擎=Tool 且展示=Tool 的类型）。
static BUILTIN_DISPLAY_KIND: &[(&str, NodeKind)] = &[
    ("agent", NodeKind::Agent),
    ("llm", NodeKind::Agent),
    ("llmClassifier", NodeKind::Agent),
    ("multiAgent", NodeKind::Agent),
    ("vectorRetrieve", NodeKind::Storage),
    ("storage", NodeKind::Storage),
    ("databaseQuery", NodeKind::Storage),
    ("condition", NodeKind::Condition),
    ("switch", NodeKind::Condition),
    ("loop", NodeKind::Loop),
    ("parallel", NodeKind::Container),
    ("debate", NodeKind::Container),
    ("swarm", NodeKind::Container),
    ("subWorkflow", NodeKind::Container),
    ("workflowRef", NodeKind::Container),
    ("aggregator", NodeKind::Container),
    ("merge", NodeKind::Container),
    ("end", NodeKind::Output),
    ("notification", NodeKind::Output),
    ("approval", NodeKind::Output),
    ("email", NodeKind::Output),
    ("webhookSend", NodeKind::Output),
    ("trigger", NodeKind::Input),
];

/// 左侧面板分组映射：仅列非 `flow` 分组；未命中回退 `"flow"`。
static BUILTIN_CATEGORY: &[(&str, &str)] = &[
    ("trigger", "trigger"),
    ("agent", "agent"),
    ("llm", "llm"),
    ("llmClassifier", "llm"),
    ("multiAgent", "agent"),
    ("subWorkflow", "integration"),
    ("workflowRef", "integration"),
    ("aggregator", "execution"),
    ("tool", "execution"),
    ("code", "execution"),
    ("documentParser", "integration"),
    ("httpRequest", "integration"),
    ("fileOperation", "execution"),
    ("dataTransformer", "execution"),
    ("notification", "integration"),
    ("email", "integration"),
    ("webhookSend", "integration"),
    ("vectorRetrieve", "agent"),
    ("storage", "integration"),
    ("databaseQuery", "integration"),
];

fn display_kind_for(type_name: &str, engine_kind: NodeKind) -> NodeKind {
    BUILTIN_DISPLAY_KIND
        .iter()
        .find(|(name, _)| *name == type_name)
        .map(|(_, kind)| kind.clone())
        .unwrap_or(engine_kind)
}

fn category_for(type_name: &str) -> &'static str {
    BUILTIN_CATEGORY
        .iter()
        .find(|(name, _)| *name == type_name)
        .map(|(_, category)| *category)
        .unwrap_or("flow")
}

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
            fn display_kind(&self) -> NodeKind {
                display_kind_for($tag, $kind)
            }
            fn display_category(&self) -> &'static str {
                category_for($tag)
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
            fn display_kind(&self) -> NodeKind {
                display_kind_for($tag, $kind)
            }
            fn display_category(&self) -> &'static str {
                category_for($tag)
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

/// 全局节点类型注册表（懒初始化：内置 32 种 + 外部功能包动态注册）。
///
/// 这是「类型字符串 → 契约」的单一权威：
/// - 阶段2 供 deserializer 判未知类型；
/// - 阶段3 供 rt-workflow 三处 match 查表；
/// - 阶段4 供 dispatcher 遍历自动装配 executor。
///
/// 外部新增节点类型无需改本文件：在启动期调用 `register_node_descriptor` 即可。
/// 模式对齐 `capability_registry.rs` 的 `get_capability_registry()`。
pub fn builtin_descriptors() -> &'static NodeDescriptorRegistry {
    static REGISTRY: OnceLock<NodeDescriptorRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry = NodeDescriptorRegistry::new();
        register_builtin_descriptors(&registry);
        registry
    })
}

/// 外部注册一个节点类型契约（功能包 / 插件 / 测试节点用）。
///
/// 返回 `false` 表示同名类型已注册（保留先注册者，不覆盖）——
/// 避免第三方覆盖内置节点行为。新增节点类型 = 实现 descriptor + 调用本函数。
pub fn register_node_descriptor<D: WorkflowNodeDescriptor + 'static>(descriptor: D) -> bool {
    let before = builtin_descriptors().type_names().len();
    builtin_descriptors().register(descriptor);
    builtin_descriptors().type_names().len() > before
}

/// 按类型串查契约（阶段3 三处 match / 阶段4 dispatcher 入口）。
pub fn builtin_descriptor_for(type_name: &str) -> Option<Arc<dyn WorkflowNodeDescriptor>> {
    builtin_descriptors().get(type_name)
}

/// 从 `WorkflowNode` 的序列化标签查内置契约（共存期桥接）。
///
/// enum 仍是权威时，三处 match 需要「node → type 串 → descriptor」；
/// 阶段6 删除 enum 后，调用方直接持有 `WorkflowNodeDyn.node_type` 即可。
pub fn descriptor_for_node(node: &WorkflowNode) -> Option<Arc<dyn WorkflowNodeDescriptor>> {
    let value = serde_json::to_value(node).ok()?;
    builtin_descriptor_for(value.get("type")?.as_str()?)
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

    /// 阶段4 验收判据1（harness 侧）：外部类型仅靠实现 descriptor + 一次注册
    /// 即进入注册表，无需改主壳任何既有文件；同名重复注册不覆盖。
    /// 用局部注册表验证语义，避免污染进程级的 32 内置类型集合（其它测试依赖它）。
    #[test]
    fn external_descriptor_registers_into_registry() {
        struct TestOnlyDescriptor;
        impl WorkflowNodeDescriptor for TestOnlyDescriptor {
            fn type_name(&self) -> &'static str {
                "testOnlyExternalNode"
            }
            fn kind(&self) -> NodeKind {
                NodeKind::Tool
            }
            fn parse_config(&self, _raw: &Value) -> Result<(), String> {
                Ok(())
            }
            fn output_var(&self, _raw: &Value) -> Option<String> {
                None
            }
        }

        let reg = NodeDescriptorRegistry::new();
        reg.register(TestOnlyDescriptor);
        let found = reg.get("testOnlyExternalNode").unwrap();
        assert_eq!(found.type_name(), "testOnlyExternalNode");
        assert_eq!(found.kind(), NodeKind::Tool);
        // 重复注册不覆盖、不新增
        let before = reg.type_names().len();
        reg.register(TestOnlyDescriptor);
        assert_eq!(reg.type_names().len(), before);
    }

    /// 阶段5：展示元数据必须与旧前端 `NODE_KIND_MAP` / `NODE_TYPE_MAP` 逐字等价，
    /// 否则 11 个「引擎分类 ≠ 展示分类」的节点会在数据驱动化后改变配色 / 容器行为。
    #[test]
    fn display_meta_matches_legacy_frontend_tables() {
        let reg = super::builtin_descriptors();
        // (type, 展示分类, 分组, isContainer) —— 取自阶段5实施前的 workflow.types.ts
        let expected: &[(&str, NodeKind, &str, bool)] = &[
            ("trigger", NodeKind::Input, "trigger", false),
            ("agent", NodeKind::Agent, "agent", false),
            ("llm", NodeKind::Agent, "llm", false),
            ("llmClassifier", NodeKind::Agent, "llm", false),
            ("multiAgent", NodeKind::Agent, "agent", false),
            ("condition", NodeKind::Condition, "flow", false),
            ("switch", NodeKind::Condition, "flow", false),
            ("loop", NodeKind::Loop, "flow", true),
            ("parallel", NodeKind::Container, "flow", true),
            ("debate", NodeKind::Container, "flow", true),
            ("swarm", NodeKind::Container, "flow", true),
            ("subWorkflow", NodeKind::Container, "integration", true),
            ("workflowRef", NodeKind::Container, "integration", true),
            ("aggregator", NodeKind::Container, "execution", true),
            ("merge", NodeKind::Container, "flow", true),
            ("tool", NodeKind::Tool, "execution", false),
            ("code", NodeKind::Tool, "execution", false),
            ("delay", NodeKind::Tool, "flow", false),
            ("validation", NodeKind::Tool, "flow", false),
            ("documentParser", NodeKind::Tool, "integration", false),
            ("httpRequest", NodeKind::Tool, "integration", false),
            ("fileOperation", NodeKind::Tool, "execution", false),
            ("dataTransformer", NodeKind::Tool, "execution", false),
            ("logging", NodeKind::Tool, "flow", false),
            ("end", NodeKind::Output, "flow", false),
            ("notification", NodeKind::Output, "integration", false),
            ("approval", NodeKind::Output, "flow", false),
            ("email", NodeKind::Output, "integration", false),
            ("webhookSend", NodeKind::Output, "integration", false),
            ("vectorRetrieve", NodeKind::Storage, "agent", false),
            ("storage", NodeKind::Storage, "integration", false),
            ("databaseQuery", NodeKind::Storage, "integration", false),
        ];
        assert_eq!(expected.len(), 32);
        for (type_name, display_kind, category, is_container) in expected {
            let meta = reg
                .get(type_name)
                .unwrap_or_else(|| panic!("缺少内置 descriptor: {type_name}"))
                .display_meta();
            assert_eq!(meta.type_name, *type_name);
            assert_eq!(&meta.display_kind, display_kind, "{type_name} display_kind");
            assert_eq!(meta.category, *category, "{type_name} category");
            assert_eq!(meta.is_container, *is_container, "{type_name} isContainer");
            assert_eq!(
                meta.label_key,
                format!("workflow.nodeTypes.{type_name}"),
                "{type_name} labelKey"
            );
            let expected_hue = match display_kind {
                NodeKind::Input => "yellow",
                NodeKind::Output => "red",
                NodeKind::Tool => "green",
                NodeKind::Agent => "blue",
                NodeKind::Condition => "orange",
                NodeKind::Loop => "purple",
                NodeKind::Container => "cyan",
                NodeKind::Storage => "pink",
            };
            assert_eq!(meta.hue, expected_hue, "{type_name} hue");
        }
        // 11 个双维度类型：引擎分类必须保持原值（不可被展示分类覆盖）。
        let dual = [
            ("aggregator", NodeKind::Tool, NodeKind::Container),
            ("approval", NodeKind::Tool, NodeKind::Output),
            ("databaseQuery", NodeKind::Tool, NodeKind::Storage),
            ("email", NodeKind::Tool, NodeKind::Output),
            ("llmClassifier", NodeKind::Tool, NodeKind::Agent),
            ("multiAgent", NodeKind::Container, NodeKind::Agent),
            ("notification", NodeKind::Tool, NodeKind::Output),
            ("subWorkflow", NodeKind::Tool, NodeKind::Container),
            ("vectorRetrieve", NodeKind::Tool, NodeKind::Storage),
            ("webhookSend", NodeKind::Tool, NodeKind::Output),
            ("workflowRef", NodeKind::Tool, NodeKind::Container),
        ];
        for (type_name, engine, display) in dual {
            let d = reg.get(type_name).unwrap();
            assert_eq!(d.kind(), engine, "{type_name} 引擎分类不可变");
            assert_eq!(d.display_kind(), display, "{type_name} 展示分类");
        }
    }
}
