// SPDX-License-Identifier: AGPL-3.0-only

//! Notification executor — 经通知直推接缝发送节点消息。
//!
//! P6.0-⑥（PLAN §211）：渠道 payload 格式知识（slack/wecom/dingtalk/feishu）
//! 与 HTTP 发送已并入通知能力层（notification crate `incoming_webhook`，实现
//! harness `IncomingWebhookSender`）——本执行器只解析节点配置并调接缝，不得
//! 自带 reqwest / payload 构造。

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};

pub struct NotificationExecutor;

impl NotificationExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NotificationExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeExecutorTrait for NotificationExecutor {
    fn node_type(&self) -> &'static str {
        "notification"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        ctx: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::Notification(n) = node else {
            return Err(NodeError::type_mismatch("notification", self.node_type()));
        };
        let c = &n.config;

        // P0 修复(2026-07-22): channel="system" 时不需要 webhook_url，
        // 直接返回成功（系统通知在前端展示，无需 HTTP 调用）。
        // 原逻辑无条件检查 webhook_url，导致 channel="system" + webhook_url=None
        // 时返回 NOTIFICATION_CONFIG_INVALID，工作流状态变为 PartiallyCompleted。
        if c.channel == "system" {
            tracing::info!(
                channel = %c.channel,
                message = %c.message,
                "NotificationExecutor: system channel, skip webhook send"
            );
            return Ok(NodeOutput {
                output: serde_json::json!({
                    "channel": "system",
                    "message": &c.message,
                    "sent": false,
                    "reason": "system channel does not require webhook",
                }),
                output_var: Some(c.output_var.clone()),
                control: None,
            });
        }

        if c.webhook_url.as_deref().unwrap_or("").trim().is_empty() {
            return Err(NodeError::exec_failed(
                "NOTIFICATION_CONFIG_INVALID",
                "webhook_url is empty",
            ));
        }

        let sink = ctx.notification_sink.as_ref().ok_or_else(|| {
            NodeError::exec_failed(
                "NOTIFICATION_SINK_NOT_INJECTED",
                "notification sink not injected into ExecutionState",
            )
        })?;

        tracing::info!(
            channel = %c.channel,
            url = %c.webhook_url.as_deref().unwrap_or(""),
            "NotificationExecutor: sending via notification sink"
        );

        let receipt = sink
            .send(&c.channel, c.webhook_url.as_deref().unwrap_or_default(), &c.message)
            .await
            .map_err(|e| NodeError::exec_failed("NOTIFICATION_SEND_FAILED", e))?;

        if !receipt.sent {
            tracing::warn!(
                status = receipt.status,
                body = %receipt.response_body,
                "NotificationExecutor: non-2xx response"
            );
        }

        Ok(NodeOutput {
            output: serde_json::json!({
                "sent": receipt.sent,
                "status": receipt.status,
                "channel": c.channel,
                "response_body": receipt.response_body,
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
    // SAFETY: 测试替身 MockSink 的 std::sync::Mutex 不跨 await 使用（send 内
    // push 后即释放），临界区仅同步操作；整 mod 的用点同型，故 mod 级豁免。
    #![allow(clippy::disallowed_types)]
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    /// 测试替身：记录三元组并可编排回执。calls 锁不跨 await（push 后即释放）。
    struct MockSink {
        calls: StdMutex<Vec<(String, String, String)>>,
        result: Result<(bool, u16, String), String>,
    }

    #[async_trait::async_trait]
    impl axagent_harness::IncomingWebhookSender for MockSink {
        async fn send(
            &self,
            channel: &str,
            webhook_url: &str,
            message: &str,
        ) -> Result<axagent_harness::IncomingWebhookReceipt, String> {
            self.calls.lock().unwrap().push((
                channel.to_string(),
                webhook_url.to_string(),
                message.to_string(),
            ));
            match &self.result {
                Ok((sent, status, body)) => Ok(axagent_harness::IncomingWebhookReceipt {
                    sent: *sent,
                    status: *status,
                    response_body: body.clone(),
                }),
                Err(e) => Err(e.clone()),
            }
        }
    }

    fn node_with(channel: &str, webhook_url: Option<&str>) -> WorkflowNode {
        serde_json::from_value(json!({
            "type": "notification",
            "id": "n1",
            "title": "通知",
            "position": {"x": 0.0, "y": 0.0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {
                "channel": channel,
                "message": "hello",
                "webhook_url": webhook_url,
                "enabled": true,
                "output_var": "notify_out"
            }
        }))
        .unwrap()
    }

    async fn ctx_with_sink(
        sink: Option<Arc<dyn axagent_harness::IncomingWebhookSender>>,
    ) -> ExecutionState {
        let mut ctx = ExecutionState::new("e1".into(), "w1".into(), json!({}));
        ctx.notification_sink = sink;
        ctx
    }

    #[tokio::test]
    async fn system_channel_skips_sink_even_when_missing() {
        let ctx = ctx_with_sink(None).await;
        let node = node_with("system", None);
        let out = NotificationExecutor.execute(&node, &ctx).await.unwrap();
        assert_eq!(out.output["channel"], "system");
        assert_eq!(out.output["sent"], false);
        assert_eq!(out.output_var, Some("notify_out".to_string()));
    }

    #[tokio::test]
    async fn empty_webhook_url_is_config_invalid() {
        let sink = Arc::new(MockSink {
            calls: StdMutex::new(vec![]),
            result: Ok((true, 200, "ok".into())),
        });
        let ctx = ctx_with_sink(Some(sink)).await;
        let node = node_with("slack", Some("  "));
        let err = NotificationExecutor.execute(&node, &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "NOTIFICATION_CONFIG_INVALID")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_sink_reports_not_injected() {
        let ctx = ctx_with_sink(None).await;
        let node = node_with("slack", Some("https://hooks.example.com/x"));
        let err = NotificationExecutor.execute(&node, &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "NOTIFICATION_SINK_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn sink_receives_triple_and_output_contract_holds() {
        let sink = Arc::new(MockSink {
            calls: StdMutex::new(vec![]),
            result: Ok((true, 200, "{\"ok\":true}".into())),
        });
        let ctx = ctx_with_sink(Some(sink.clone())).await;
        let node = node_with("wecom", Some("https://qyapi.weixin.qq.com/hook"));
        let out = NotificationExecutor.execute(&node, &ctx).await.unwrap();
        let calls = sink.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            (
                "wecom".to_string(),
                "https://qyapi.weixin.qq.com/hook".to_string(),
                "hello".to_string()
            )
        );
        drop(calls);
        assert_eq!(out.output["sent"], true);
        assert_eq!(out.output["status"], 200);
        assert_eq!(out.output["channel"], "wecom");
        assert_eq!(out.output["response_body"], "{\"ok\":true}");
        assert_eq!(out.output["node_id"], "n1");
        assert_eq!(out.output_var, Some("notify_out".to_string()));
    }

    #[tokio::test]
    async fn sink_error_maps_to_send_failed() {
        let sink =
            Arc::new(MockSink { calls: StdMutex::new(vec![]), result: Err("网络断了".into()) });
        let ctx = ctx_with_sink(Some(sink)).await;
        let node = node_with("slack", Some("https://hooks.example.com/x"));
        let err = NotificationExecutor.execute(&node, &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => assert_eq!(code, "NOTIFICATION_SEND_FAILED"),
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }
}
