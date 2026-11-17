// SPDX-License-Identifier: AGPL-3.0-only

//! Deserializer for WorkflowNode — decoupled from the DTO definition.
//!
//! The custom `Deserialize` impl for WorkflowNode is a ~90-line dispatcher
//! over 31 variants. This module isolates that logic so that
//! `workflow_types.rs` remains a pure data definition.
//!
//! 阶段2（P6.0-①）：未知类型白名单改为查 `NodeDescriptorRegistry`，消除
//! "match 臂 + unknown_variant 名单"双表。合法类型仍走原强类型 wrapper 解析，
//! 序列化/反序列化行为逐字不变。

use std::sync::OnceLock;

use serde::Deserialize;

use crate::workflow_node_descriptors::register_builtin_descriptors;
use crate::workflow_node_dyn::NodeDescriptorRegistry;
use crate::workflow_types::*;

/// 内置节点类型注册表（懒初始化一次，供 deserializer 查表）。
fn builtin_registry() -> &'static NodeDescriptorRegistry {
    static REGISTRY: OnceLock<NodeDescriptorRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry = NodeDescriptorRegistry::new();
        register_builtin_descriptors(&registry);
        registry
    })
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<WorkflowNode, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let type_str = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| serde::de::Error::missing_field("type"))?;

    macro_rules! try_from_value {
        ($variant:ident, $inner:ty) => {
            WorkflowNode::$variant(
                serde_json::from_value::<$inner>(value).map_err(serde::de::Error::custom)?,
            )
        };
    }

    match type_str {
        "trigger" => Ok(try_from_value!(Trigger, TriggerNode)),
        "agent" => Ok(try_from_value!(Agent, AgentNode)),
        "llm" => Ok(try_from_value!(Llm, LLMNode)),
        "condition" => Ok(try_from_value!(Condition, ConditionNode)),
        "parallel" => Ok(try_from_value!(Parallel, ParallelNode)),
        "loop" => Ok(try_from_value!(Loop, LoopNode)),
        "merge" => Ok(try_from_value!(Merge, MergeNode)),
        "delay" => Ok(try_from_value!(Delay, DelayNode)),
        "validation" => Ok(try_from_value!(Validation, ValidationNode)),
        "subWorkflow" => Ok(try_from_value!(SubWorkflow, SubWorkflowNode)),
        "documentParser" => Ok(try_from_value!(DocumentParser, DocumentParserNode)),
        "vectorRetrieve" => Ok(try_from_value!(VectorRetrieve, VectorRetrieveNode)),
        "httpRequest" => Ok(try_from_value!(HttpRequest, HttpRequestNode)),
        "switch" => Ok(try_from_value!(Switch, SwitchNode)),
        "databaseQuery" => Ok(try_from_value!(DatabaseQuery, DatabaseQueryNode)),
        "notification" => Ok(try_from_value!(Notification, NotificationNode)),
        "approval" => Ok(try_from_value!(Approval, ApprovalNode)),
        "fileOperation" => Ok(try_from_value!(FileOperation, FileOperationNode)),
        "dataTransformer" => Ok(try_from_value!(DataTransformer, DataTransformerNode)),
        "webhookSend" => Ok(try_from_value!(WebhookSend, WebhookSendNode)),
        "logging" => Ok(try_from_value!(Logging, LoggingNode)),
        "llmClassifier" => Ok(try_from_value!(LlmClassifier, LlmClassifierNode)),
        "aggregator" => Ok(try_from_value!(Aggregator, AggregatorNode)),
        "email" => Ok(try_from_value!(Email, EmailNode)),
        "debate" => Ok(try_from_value!(Debate, DebateNode)),
        "swarm" => Ok(try_from_value!(Swarm, SwarmNode)),
        "multiAgent" => Ok(try_from_value!(MultiAgent, MultiAgentNode)),
        "storage" => Ok(try_from_value!(Storage, StorageNode)),
        "workflowRef" => Ok(try_from_value!(WorkflowRef, WorkflowRefNode)),
        "end" => Ok(try_from_value!(End, EndNode)),
        "tool" => Ok(try_from_value!(Tool, ToolNode)),
        "code" => Ok(try_from_value!(Code, CodeNode)),
        other => {
            // 阶段2：未知类型判据只认注册表，消除原 32 名手写白名单（双表）。
            // unknown_variant 要求 &'static [&'static str]，而注册表名单是运行期
            // 动态分配的 ⇒ 改用 custom 拼出同款文本，保持错误可读且不留静态双表。
            let known = builtin_registry().type_names();
            Err(serde::de::Error::custom(format!(
                "unknown variant `{other}`, expected one of `{}`",
                known.join("`, `")
            )))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表白名单与 32 臂 match 必须同集：新增 descriptor 忘加 match 臂时，
    /// 本测试通过"注册但解析失败"暴露；match 有臂但 descriptor 缺失时，
    /// 第二个断言通过"合法 type 不在名单"暴露。
    #[test]
    fn registry_and_match_arms_cover_same_types() {
        let registry = builtin_registry();
        let names = registry.type_names();
        assert_eq!(names.len(), 32, "内置 descriptor 应恰好 32 个");
        for name in &names {
            // 任意最小合法 payload 都行：这里只验证 match 臂存在（不会落 unknown 分支）。
            // 解析失败（config 字段不足）是预期，未知类型错误则不是。
            let raw = serde_json::json!({"type": name, "id": "x", "title": "t",
                "position": {"x":0,"y":0}, "config": {}});
            let parsed = serde_json::from_value::<WorkflowNode>(raw);
            if let Err(e) = parsed {
                assert!(
                    !e.to_string().contains("unknown variant"),
                    "已注册类型 `{name}` 在 match 中无对应臂"
                );
            }
        }
    }
}
