// SPDX-License-Identifier: AGPL-3.0-only

//! WebhookSend executor — sends HTTP requests with optional credential-based
//! authentication.
//!
//! Reuses `reqwest` (already depended upon by HttpRequestExecutor). When
//! `credential_id` is set, the executor resolves the credential via
//! `CredentialManager` and injects the appropriate auth headers.

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};

pub struct WebhookSendExecutor;

impl WebhookSendExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebhookSendExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeExecutorTrait for WebhookSendExecutor {
    fn node_type(&self) -> &'static str {
        "webhookSend"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        ctx: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::WebhookSend(n) = node else {
            return Err(NodeError::type_mismatch("webhookSend", self.node_type()));
        };
        let c = &n.config;

        if c.url.trim().is_empty() {
            return Err(NodeError::exec_failed("WEBHOOK_CONFIG_INVALID", "webhook URL is empty"));
        }

        let method = c.method.to_uppercase();
        if !matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "GET" | "DELETE") {
            return Err(NodeError::exec_failed(
                "WEBHOOK_METHOD_UNSUPPORTED",
                format!("unsupported method: {method}"),
            ));
        }

        // P6.0-⑥ 批 E：协议细节（reqwest / 超时）归实现层；本执行器保留策略——
        // 方法白名单、凭据注入（读 ctx.credential_manager）、头覆盖顺序与输出包装。
        let seam = ctx.http_client.as_ref().ok_or_else(|| {
            NodeError::exec_failed(
                "WORKFLOW_HTTP_NOT_INJECTED",
                "workflow http client not injected into ExecutionState",
            )
        })?;

        let mut headers = std::collections::HashMap::new();
        // Inject credential-based auth headers if credential_id is set
        if let Some(cid) = c.credential_id.as_deref()
            && let Some(cm) = &ctx.credential_manager
        {
            match cm.get_auth_headers(cid).await {
                Ok(hs) => {
                    for (key, value) in hs {
                        headers.insert(key, value);
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        credential_id = cid,
                        error = %e,
                        "WebhookSend: failed to load credential, proceeding without auth"
                    );
                },
            }
        }
        // Add explicit headers from config (override credential-injected ones)
        for (key, value) in &c.headers {
            headers.insert(key.clone(), value.clone());
        }

        let body = c.body.clone().filter(|b| !b.trim().is_empty());

        let start = std::time::Instant::now();
        let response = seam
            .send(axagent_harness::WorkflowHttpRequest {
                method: method.clone(),
                url: c.url.clone(),
                headers,
                body,
                body_is_json: true,
                timeout_secs: 30,
                follow_redirects: true,
            })
            .await
            .map_err(|e| NodeError::exec_failed("WEBHOOK_SEND_FAILED", e.to_string()))?;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let status = response.status;
        let output = serde_json::json!({
            "url": c.url,
            "method": method,
            "status": status,
            "status_text": if (200..300).contains(&status) { "success" } else { "error" },
            "body": response.body,
            "elapsed_ms": elapsed_ms,
            "node_id": node.base_id(),
        });

        Ok(NodeOutput {
            output,
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

    struct MockHttp {
        calls: StdMutex<Vec<axagent_harness::WorkflowHttpRequest>>,
    }

    #[async_trait]
    impl axagent_harness::WorkflowHttpClient for MockHttp {
        async fn send(
            &self,
            request: axagent_harness::WorkflowHttpRequest,
        ) -> Result<axagent_harness::WorkflowHttpResponse, String> {
            self.calls.lock().unwrap().push(request);
            Ok(axagent_harness::WorkflowHttpResponse {
                status: 204,
                headers: std::collections::HashMap::new(),
                body: String::new(),
            })
        }
    }

    fn node_with(method: &str) -> WorkflowNode {
        serde_json::from_value(json!({
            "type": "webhookSend",
            "id": "w1",
            "title": "Webhook",
            "position": {"x": 0.0, "y": 0.0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {
                "url": "https://hooks.example.com/x",
                "method": method,
                "body": "{\"k\":1}",
                "headers": {"x-a": "1"},
                "output_var": "wh_out"
            }
        }))
        .unwrap()
    }

    fn ctx_with(client: Option<Arc<dyn axagent_harness::WorkflowHttpClient>>) -> ExecutionState {
        let mut ctx = ExecutionState::new("e1".into(), "w1".into(), json!({}));
        ctx.http_client = client;
        ctx
    }

    #[tokio::test]
    async fn missing_seam_reports_not_injected() {
        let ctx = ctx_with(None);
        let err = WebhookSendExecutor.execute(&node_with("POST"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "WORKFLOW_HTTP_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn seam_receives_json_body_and_output_contract_holds() {
        let client = Arc::new(MockHttp { calls: StdMutex::new(vec![]) });
        let ctx = ctx_with(Some(client.clone()));
        let out = WebhookSendExecutor.execute(&node_with("post"), &ctx).await.unwrap();
        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].body.as_deref(), Some("{\"k\":1}"));
        assert!(calls[0].body_is_json);
        assert_eq!(calls[0].timeout_secs, 30);
        assert_eq!(calls[0].headers.get("x-a").map(String::as_str), Some("1"));
        drop(calls);
        assert_eq!(out.output["status"], 204);
        assert_eq!(out.output["status_text"], "success");
        assert_eq!(out.output["method"], "POST");
        assert_eq!(out.output_var, Some("wh_out".to_string()));
    }

    #[tokio::test]
    async fn unsupported_method_rejected_before_seam() {
        let client = Arc::new(MockHttp { calls: StdMutex::new(vec![]) });
        let ctx = ctx_with(Some(client.clone()));
        let err = WebhookSendExecutor.execute(&node_with("TRACE"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "WEBHOOK_METHOD_UNSUPPORTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
        assert_eq!(client.calls.lock().unwrap().len(), 0);
    }
}
