// SPDX-License-Identifier: AGPL-3.0-only

//! Email executor — sends emails via SMTP using `lettre`.
//!
//! SMTP credentials are resolved from `credential_id` (via `CredentialManager`)
//! when available, falling back to inline `smtp_*` fields for backwards compatibility.

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};

pub struct EmailExecutor;

impl EmailExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EmailExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// SMTP configuration, resolved from credential or inline fields.
struct SmtpConfig {
    host: String,
    port: u16,
    user: String,
    pass: String,
    tls: bool,
}

async fn resolve_smtp_config(
    ctx: &ExecutionState,
    credential_id: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
    user: Option<&str>,
    pass: Option<&str>,
) -> Result<SmtpConfig, NodeError> {
    if let Some(cid) = credential_id {
        let cm = ctx.credential_manager.as_ref().ok_or_else(|| {
            NodeError::exec_failed(
                "EMAIL_CREDENTIAL_UNAVAILABLE",
                "credential_manager not injected into ExecutionState",
            )
        })?;
        let sc = cm
            .get_smtp_config(cid)
            .await
            .map_err(|e| NodeError::exec_failed("EMAIL_CREDENTIAL_FAILED", e.to_string()))?;
        Ok(SmtpConfig { host: sc.host, port: sc.port, user: sc.user, pass: sc.pass, tls: sc.tls })
    } else {
        let host = host
            .filter(|h| !h.is_empty())
            .ok_or_else(|| {
                NodeError::exec_failed(
                    "EMAIL_CONFIG_INCOMPLETE",
                    "smtp_host is required when credential_id is not set",
                )
            })?
            .to_string();
        let port = port.unwrap_or(587);
        let user = user.unwrap_or("").to_string();
        let pass = pass.unwrap_or("").to_string();
        // Default to TLS for port 587, plain for others
        let tls = port == 587;
        Ok(SmtpConfig { host, port, user, pass, tls })
    }
}

#[async_trait]
impl NodeExecutorTrait for EmailExecutor {
    fn node_type(&self) -> &'static str {
        "email"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        ctx: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::Email(n) = node else {
            return Err(NodeError::type_mismatch("email", self.node_type()));
        };
        let c = &n.config;

        let smtp = resolve_smtp_config(
            ctx,
            c.credential_id.as_deref(),
            c.smtp_host.as_deref(),
            c.smtp_port,
            c.smtp_user.as_deref(),
            c.smtp_pass.as_deref(),
        )
        .await?;

        tracing::info!(
            host = %smtp.host,
            port = smtp.port,
            to = ?c.to,
            subject = %c.subject,
            "EmailExecutor: sending email"
        );

        // P6.0-⑥ 批 E：lettre（构建/传输/投递）归实现层；本执行器保留
        // 凭据解析策略（credential_id 优先、内联字段回退）与输出包装。
        let seam = ctx.smtp_sender.as_ref().ok_or_else(|| {
            NodeError::exec_failed(
                "WORKFLOW_SMTP_NOT_INJECTED",
                "workflow smtp sender not injected into ExecutionState",
            )
        })?;

        seam.send(axagent_harness::WorkflowSmtpRequest {
            host: smtp.host.clone(),
            port: smtp.port,
            user: smtp.user.clone(),
            pass: smtp.pass.clone(),
            tls: smtp.tls,
            from: smtp.user.clone(),
            to: c.to.clone(),
            subject: c.subject.clone(),
            body: c.body.clone(),
        })
        .await
        .map_err(|e| NodeError::exec_failed("EMAIL_SEND_FAILED", e))?;
        tracing::info!("EmailExecutor: sent successfully");

        Ok(NodeOutput {
            output: serde_json::json!({
                "to": c.to,
                "subject": c.subject,
                "sent": true,
                "node_id": node.base_id(),
            }),
            output_var: if c.output_var.is_empty() {
                None
            } else {
                Some(c.output_var.clone())
            },
            control: None,
        })
    }
}

#[cfg(test)]
mod tests {
    // SAFETY: 测试替身的 std::sync::Mutex 不跨 await 使用（push 后即释放，
    // 锁持有不跨越 await 点），临界区仅同步操作；整 mod 的用点同型，故 mod 级豁免。
    #![allow(clippy::disallowed_types)]
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    struct MockSmtp {
        calls: StdMutex<Vec<axagent_harness::WorkflowSmtpRequest>>,
    }

    #[async_trait]
    impl axagent_harness::WorkflowSmtpSender for MockSmtp {
        async fn send(&self, request: axagent_harness::WorkflowSmtpRequest) -> Result<(), String> {
            self.calls.lock().unwrap().push(request);
            Ok(())
        }
    }

    fn node_with() -> WorkflowNode {
        serde_json::from_value(json!({
            "type": "email",
            "id": "m1",
            "title": "邮件",
            "position": {"x": 0.0, "y": 0.0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {
                "to": ["a@example.com", "b@example.com"],
                "subject": "测试",
                "body": "正文",
                "smtp_host": "smtp.example.com",
                "smtp_port": 587,
                "smtp_user": "me@example.com",
                "smtp_pass": "secret",
                "output_var": "mail_out"
            }
        }))
        .unwrap()
    }

    fn ctx_with(sender: Option<Arc<dyn axagent_harness::WorkflowSmtpSender>>) -> ExecutionState {
        let mut ctx = ExecutionState::new("e1".into(), "w1".into(), json!({}));
        ctx.smtp_sender = sender;
        ctx
    }

    #[tokio::test]
    async fn missing_seam_reports_not_injected() {
        let ctx = ctx_with(None);
        let err = EmailExecutor.execute(&node_with(), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "WORKFLOW_SMTP_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn inline_smtp_config_goes_through_seam() {
        let sender = Arc::new(MockSmtp { calls: StdMutex::new(vec![]) });
        let ctx = ctx_with(Some(sender.clone()));
        let out = EmailExecutor.execute(&node_with(), &ctx).await.unwrap();
        let calls = sender.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].host, "smtp.example.com");
        assert_eq!(calls[0].port, 587);
        assert!(calls[0].tls, "587 ⇒ STARTTLS（原语义）");
        assert_eq!(calls[0].from, "me@example.com");
        assert_eq!(calls[0].to.len(), 2);
        assert_eq!(calls[0].subject, "测试");
        drop(calls);
        assert_eq!(out.output["sent"], true);
        assert_eq!(out.output_var, Some("mail_out".to_string()));
    }

    #[tokio::test]
    async fn non_587_port_defaults_to_plain() {
        let sender = Arc::new(MockSmtp { calls: StdMutex::new(vec![]) });
        let ctx = ctx_with(Some(sender.clone()));
        let mut node = node_with();
        if let WorkflowNode::Email(ref mut n) = node {
            n.config.smtp_port = Some(25);
        }
        EmailExecutor.execute(&node, &ctx).await.unwrap();
        let calls = sender.calls.lock().unwrap();
        assert!(!calls[0].tls, "非 587 ⇒ 明文（原语义）");
    }
}
