// SPDX-License-Identifier: AGPL-3.0-only

//! Node executor trait and related types

use async_trait::async_trait;
use axagent_harness::workflow_types::{ExecutionStatus, NodeKind, WorkflowNode};
use serde::{Deserialize, Serialize};

/// Output from a node execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    #[serde(default)]
    pub output: serde_json::Value,
    pub output_var: Option<String>,
    /// 可选控制指令（如 Suspend 挂起）。默认 None = 正常完成。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<NodeControl>,
}

/// 节点对引擎发出的控制指令（挂起/恢复等）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeControl {
    /// 挂起整个工作流，等待外部审批/信号后恢复
    Suspend { resume_token: String, approval: ApprovalRequest },
}

/// 审批请求数据（由 ApprovalExecutor 产生，引擎通过 ApprovalOps 持久化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub execution_id: String,
    pub node_id: String,
    pub title: String,
    pub message: String,
    pub approver: Option<String>,
    pub channels: Vec<String>,
    pub payload: serde_json::Value,
    pub timeout_secs: u64,
    pub timeout_action: String,
}

// ── Error codes ──

/// 节点执行错误码（前端根据 code 查 i18n 翻译）
pub mod error_code {
    pub const PROVIDER_QUERY_FAILED: &str = "PROVIDER_QUERY_FAILED";
    pub const NO_AVAILABLE_PROVIDER: &str = "NO_AVAILABLE_PROVIDER";
    // 「密钥解不开」在供应商页与工作流两条链上是同一条故障，必须同码；码的单一权威在
    // harness::error_codes::provider（前端 11 语言的 error 段按该码收录）。
    pub use axagent_harness::error_codes::provider::KEY_DECRYPT_FAILED;
    pub const UNSUPPORTED_PROVIDER: &str = "UNSUPPORTED_PROVIDER";
    pub const LLM_CALL_FAILED: &str = "LLM_CALL_FAILED";
    pub const AGENT_PROFILE_NOT_FOUND: &str = "AGENT_PROFILE_NOT_FOUND";
    pub const TOOL_CALL_FAILED: &str = "TOOL_CALL_FAILED";
    pub const TOOL_NOT_CONFIGURED: &str = "TOOL_NOT_CONFIGURED";
    pub const SUBWORKFLOW_FAILED: &str = "SUBWORKFLOW_FAILED";
    pub const SUBWORKFLOW_NOT_CONFIGURED: &str = "SUBWORKFLOW_NOT_CONFIGURED";
    pub const VECTOR_RETRIEVE_FAILED: &str = "VECTOR_RETRIEVE_FAILED";
    pub const VECTOR_NOT_CONFIGURED: &str = "VECTOR_NOT_CONFIGURED";
    pub const VARIABLE_NOT_FOUND: &str = "VARIABLE_NOT_FOUND";
    pub const VALIDATION_FAILED: &str = "VALIDATION_FAILED";
    pub const PERMISSION_DENIED: &str = "PERMISSION_DENIED";
    pub const TIMEOUT: &str = "TIMEOUT";
    pub const CIRCUIT_BREAKER_OPEN: &str = "CIRCUIT_BREAKER_OPEN";
    pub const NODE_TYPE_MISMATCH: &str = "NODE_TYPE_MISMATCH";
    pub const UNSUPPORTED_NODE_TYPE: &str = "UNSUPPORTED_NODE_TYPE";
    pub const IO_ERROR: &str = "IO_ERROR";
    pub const CACHE_DESERIALIZE_FAILED: &str = "CACHE_DESERIALIZE_FAILED";
    pub const MODEL_NOT_CONFIGURED: &str = "MODEL_NOT_CONFIGURED";
    pub const NODE_NOT_FOUND: &str = "NODE_NOT_FOUND";
    pub const EXECUTION_CANCELLED: &str = "EXECUTION_CANCELLED";
}

/// Error types for node execution
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("{code}: {detail}")]
    ExecutionFailed { code: &'static str, detail: String },

    #[error("{code}: {detail}")]
    Timeout { code: &'static str, detail: String },

    #[error("{code}: expected {expected}, got {got}")]
    InvalidNodeType { code: &'static str, expected: String, got: String },

    #[error("VARIABLE_NOT_FOUND: {0}")]
    VariableNotFound(String),

    #[error("VALIDATION_FAILED: {0}")]
    Validation(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl NodeError {
    /// 创建执行失败错误
    pub fn exec_failed(code: &'static str, detail: impl std::fmt::Display) -> Self {
        Self::ExecutionFailed { code, detail: detail.to_string() }
    }
    /// 创建超时错误
    pub fn timed_out(code: &'static str, detail: impl std::fmt::Display) -> Self {
        Self::Timeout { code, detail: detail.to_string() }
    }
    /// 创建节点类型不匹配错误
    pub fn type_mismatch(expected: impl Into<String>, got: impl Into<String>) -> Self {
        Self::InvalidNodeType {
            code: error_code::NODE_TYPE_MISMATCH,
            expected: expected.into(),
            got: got.into(),
        }
    }

    /// 获取错误码（前端 i18n 映射 key）
    pub fn code(&self) -> &str {
        match self {
            Self::ExecutionFailed { code, .. } => code,
            Self::Timeout { code, .. } => code,
            Self::InvalidNodeType { code, .. } => code,
            Self::VariableNotFound(_) => error_code::VARIABLE_NOT_FOUND,
            Self::Validation(_) => error_code::VALIDATION_FAILED,
            Self::Io(_) => error_code::IO_ERROR,
            // EXECUTION_CANCELLED 通过 ExecutionFailed 变体传递
        }
    }
}

impl From<NodeError> for serde_json::Value {
    fn from(err: NodeError) -> Self {
        serde_json::json!({
            "code": err.code(),
            "message": err.to_string(),
        })
    }
}

/// 判断错误是否为不可重试的确定性失败。
///
/// 这些错误码代表模板/配置/权限类的确定性问题：同样的输入重试必然复现同样的失败，
/// 重试只会拉长整体耗时并放大供应商限流（2026-09-08 实证：a-fundamentals 因
/// `{{market_regime}}` 变量缺失连续 4 次重试全废，浪费 ~90s 并把分析师波拉长到 182s）。
///
/// 匹配规则：错误消息形如 `"CODE: detail"`，取首个 `:` 前的码段精确比对。
/// 超时（TIMEOUT）与网络/供应商类错误（LLM_CALL_FAILED 等）不在清单内，仍可重试。
pub fn is_non_retryable_error(err_msg: &str) -> bool {
    const NON_RETRYABLE: &[&str] = &[
        error_code::VARIABLE_NOT_FOUND,
        error_code::VALIDATION_FAILED,
        error_code::PERMISSION_DENIED,
        error_code::AGENT_PROFILE_NOT_FOUND,
        error_code::MODEL_NOT_CONFIGURED,
        error_code::KEY_DECRYPT_FAILED,
        error_code::TOOL_NOT_CONFIGURED,
        error_code::NODE_NOT_FOUND,
        error_code::NODE_TYPE_MISMATCH,
        error_code::UNSUPPORTED_NODE_TYPE,
        error_code::EXECUTION_CANCELLED,
    ];
    let code = err_msg.split(':').next().unwrap_or("").trim();
    NON_RETRYABLE.contains(&code)
}

// ── Trait ──

/// 检查执行是否被取消或暂停。
///
/// 所有执行器在长时间运行的循环/操作前应调用此函数，以支持：
/// 1. 硬取消（cancel_token 被触发时立即中止）
/// 2. 软暂停（status == Paused 时挂起等待恢复信号）
///
/// # 返回值
/// - `Ok(())`: 可以继续执行
/// - `Err(NodeError)`: 应中止执行
pub async fn check_cancellation_or_pause(context: &super::ExecutionState) -> Result<(), NodeError> {
    // 1. 检查硬取消
    if let Some(ref token) = context.cancel_token
        && token.is_cancelled()
    {
        return Err(NodeError::exec_failed(error_code::EXECUTION_CANCELLED, "节点执行已取消"));
    }

    // 2. 检查软暂停（循环等待恢复信号，同时定期检查取消状态）
    // 注意：context.status 由外部恢复操作修改，此处通过 pause_signal 通知
    // 不直接修改 status 字段
    loop {
        // 先检查是否已暂停
        if context.status != ExecutionStatus::Paused {
            break;
        }

        // 尝试获取暂停信号并等待
        if let Some(signal) = context.pause_signal() {
            tokio::select! {
                _ = signal.notified() => {
                    // 恢复信号触发，跳出循环继续执行
                    tracing::debug!(
                        "[check_cancellation_or_pause] 暂停状态已恢复，继续执行"
                    );
                    break;
                }
                // 每 500ms 检查一次取消状态
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(500)) => {
                    if let Some(ref token) = context.cancel_token
                        && token.is_cancelled()
                    {
                        return Err(NodeError::exec_failed(
                            error_code::EXECUTION_CANCELLED,
                            "节点执行已取消（暂停期间检测到取消）",
                        ));
                    }
                    // 继续循环等待
                    continue;
                }
            }
        } else {
            // 没有暂停信号但状态是 Paused，直接跳出
            break;
        }
    }

    Ok(())
}

#[async_trait]
pub trait NodeExecutorTrait: Send + Sync {
    fn node_type(&self) -> &'static str;
    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &super::ExecutionState,
    ) -> Result<NodeOutput, NodeError>;
}

/// 阶段3：两处 32 臂 match 收口到节点契约注册表。
/// 共存期 enum 仍是权威，但 descriptor 的 type_name/kind 与原 match 逐字同集
/// （阶段2 测试 `registry_and_match_arms_cover_same_types` 已锁），故查表等价。
/// 未注册类型在此 panic 而非静默降级——enum 变体全覆盖，正常路径不可达。
fn descriptor_of(
    node: &WorkflowNode,
) -> std::sync::Arc<dyn axagent_harness::workflow_node_dyn::WorkflowNodeDescriptor> {
    axagent_harness::workflow_node_descriptors::descriptor_for_node(node)
        .expect("WorkflowNode 类型必须已注册 descriptor（阶段3 不应静默降级）")
}

pub fn node_type_name(node: &WorkflowNode) -> &'static str {
    descriptor_of(node).type_name()
}

/// 获取节点的高级分类（NodeKind）
pub fn node_kind(node: &WorkflowNode) -> NodeKind {
    descriptor_of(node).kind()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 阶段3 桥接回归：从 enum 序列化标签查 descriptor，返回值必须与旧 match 一致。
    #[test]
    fn descriptor_bridge_matches_legacy_type_and_kind() {
        let node: WorkflowNode = serde_json::from_value(serde_json::json!({
            "type": "storage", "id": "s1", "title": "存储",
            "position": {"x": 0, "y": 0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {"backend": "sqlite", "operation": "upsert", "input_var": "v", "collection": "c"}
        }))
        .unwrap();
        assert_eq!(node_type_name(&node), "storage");
        assert_eq!(node_kind(&node), NodeKind::Storage);
    }
}
