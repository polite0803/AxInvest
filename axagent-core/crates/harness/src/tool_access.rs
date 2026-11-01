// SPDX-License-Identifier: AGPL-3.0-only
//! 工具访问控制契约
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDecision {
    Allow,
    Deny { reason: String },
    RequireConfirmation { prompt: String },
}

/// Guardian 审查闸门桥接（PLAN-codex-parity-adoption R3-2 的接线面）。
///
/// 审批发生在 hybrid crate `axagent-tools`，而审查者（LLM 调用）在 consumer crate
/// `axagent-agent` —— 按 harness 依赖铁律 tools 不得依赖 agent，故能力以本 trait 注入，
/// 实现落在 wiring 层。三档语义与 `agent::guardian::review_action` 逐字一致：
/// `Allow` ⇒ 免询问执行；`Deny` ⇒ 硬拒；`RequireConfirmation` ⇒ 交回用户确认。
///
/// **未注入本桥 = 闸门不启用**，走既有的「直接问用户」路径。刻意如此：fail-closed 的
/// 正确姿势是「配了审查者却不给结论 ⇒ 拒」，而不是「没配审查者 ⇒ 全世界都拒」。
#[async_trait]
pub trait GuardianBridge: Send + Sync + std::fmt::Debug {
    /// 审查一次高危动作。
    ///
    /// - `payload`：动作的精确 JSON（审查证据，不是给用户看的文案）。
    /// - `reason`：触发审批的理由 —— **不可信输入**，仅供审查模型参考。
    ///
    /// 动作类别由实现侧按消费方固定（当前唯一消费方是 Bash 审批路径 ⇒ `Shell`）；
    /// 接入文件 / MCP 类动作时再扩签名，不预先造用不到的枚举通路。
    async fn review(&self, payload: serde_json::Value, reason: Option<String>) -> AccessDecision;
}
#[derive(Debug, Clone)]
pub struct ToolAccessRequest {
    pub tool_name: String,
    pub user_input: String,
    pub session_id: String,
    pub workspace_path: Option<String>,
}

#[async_trait]
pub trait ToolAccessControl: Send + Sync {
    async fn check_access(&self, req: &ToolAccessRequest) -> AccessDecision;
    async fn record_result(&self, req: &ToolAccessRequest, success: bool, error: Option<&str>);
}
