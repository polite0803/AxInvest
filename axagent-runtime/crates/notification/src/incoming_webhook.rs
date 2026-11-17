// SPDX-License-Identifier: AGPL-3.0-only

//! Incoming webhook 直推通道（P6.0-⑥：工作流 notification 节点的能力实现层）。
//!
//! 渠道格式知识（slack / wecom / dingtalk / feishu 的 payload 形状与中文别名）
//! 从 rt-workflow 的 notification_executor 搬入本模块——executor 只能是接缝薄壳
//! （PLAN §0.8.4 P6.0-⑥ / §211），格式构造与 HTTP 发送归通知能力层。
//! POST 行为与原执行器逐项一致：JSON 体、15s 超时、非 2xx 返回 sent=false。

use axagent_harness::{IncomingWebhookReceipt, IncomingWebhookSender};

/// 按渠道名构造 payload（与原 notification_executor::build_payload 逐字等价，含中文别名）。
pub fn build_incoming_payload(channel: &str, message: &str) -> serde_json::Value {
    match channel.to_lowercase().as_str() {
        "slack" => serde_json::json!({
            "text": message,
        }),
        "wecom" | "wechat_work" | "企业微信" => serde_json::json!({
            "msgtype": "markdown",
            "markdown": {
                "content": message,
            },
        }),
        "dingtalk" | "ding" | "钉钉" => serde_json::json!({
            "msgtype": "markdown",
            "markdown": {
                "title": "Notification",
                "text": message,
            },
        }),
        "feishu" | "lark" | "飞书" => serde_json::json!({
            "msg_type": "interactive",
            "card": {
                "header": {
                    "title": {
                        "tag": "plain_text",
                        "content": "Notification",
                    },
                },
                "elements": [
                    {
                        "tag": "markdown",
                        "content": message,
                    },
                ],
            },
        }),
        _ => serde_json::json!({"text": message}),
    }
}

/// [`IncomingWebhookSender`] 的默认实现：无状态（webhook_url 由调用方传入），
/// 可全局共享一份。
pub struct IncomingWebhookSenderImpl;

#[async_trait::async_trait]
impl IncomingWebhookSender for IncomingWebhookSenderImpl {
    async fn send(
        &self,
        channel: &str,
        webhook_url: &str,
        message: &str,
    ) -> Result<IncomingWebhookReceipt, String> {
        let payload = build_incoming_payload(channel, message);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;
        let response = client
            .post(webhook_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("发送通知失败: {e}"))?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        Ok(IncomingWebhookReceipt {
            sent: (200..300).contains(&status),
            status,
            response_body: body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_payload_has_text() {
        assert_eq!(build_incoming_payload("slack", "hello"), serde_json::json!({"text": "hello"}));
    }

    #[test]
    fn wecom_payload_is_markdown_and_accepts_chinese_alias() {
        for channel in ["wecom", "wechat_work", "企业微信"] {
            assert_eq!(
                build_incoming_payload(channel, "m"),
                serde_json::json!({"msgtype": "markdown", "markdown": {"content": "m"}}),
            );
        }
    }

    #[test]
    fn dingtalk_payload_has_title_and_accepts_chinese_alias() {
        for channel in ["dingtalk", "ding", "钉钉"] {
            assert_eq!(
                build_incoming_payload(channel, "m"),
                serde_json::json!({"msgtype": "markdown", "markdown": {"title": "Notification", "text": "m"}}),
            );
        }
    }

    #[test]
    fn feishu_payload_is_interactive_card_and_accepts_chinese_alias() {
        for channel in ["feishu", "lark", "飞书"] {
            assert_eq!(
                build_incoming_payload(channel, "m"),
                serde_json::json!({
                    "msg_type": "interactive",
                    "card": {
                        "header": {"title": {"tag": "plain_text", "content": "Notification"}},
                        "elements": [{"tag": "markdown", "content": "m"}],
                    },
                }),
            );
        }
    }

    #[test]
    fn unknown_channel_falls_back_to_text() {
        assert_eq!(build_incoming_payload("telegram", "m"), serde_json::json!({"text": "m"}));
    }
}
