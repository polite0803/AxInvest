// SPDX-License-Identifier: AGPL-3.0-only

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;
use tokio::net::lookup_host;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};

pub struct HttpRequestExecutor;

impl HttpRequestExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HttpRequestExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// P0-4: SSRF 防护 —— 在 HTTP 请求前做 IP 黑名单校验。
/// 拒绝：loopback、私网、link-local、IPv6 link-local、IPv4 映射的 IPv6、云元数据地址。
fn is_blocked_ip(ip: &IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            // 私网 + link-local (169.254.0.0/16) + 云元数据 (169.254.169.254)
            if v4.is_private() || v4.is_link_local() {
                return true;
            }
            // 0.0.0.0/8、100.64.0.0/10（CGNAT）、224.0.0.0/4（multicast）
            if v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || (v4.octets()[0] == 100 && (v4.octets()[1] >= 64 && v4.octets()[1] <= 127))
                || v4.octets()[0] >= 240
            {
                return true;
            }
        },
        IpAddr::V6(v6) => {
            // fe80::/10 (link-local) + fc00::/7 (unique local)
            if v6.is_unicast_link_local()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || v6.is_unspecified()
                || v6.is_multicast()
            {
                return true;
            }
            // IPv4-mapped IPv6 (::ffff:0:0/96) 也要校验内嵌的 v4
            if let Some(v4) = ipv4_mapped_from_v6(v6)
                && is_blocked_ip(&IpAddr::V4(v4))
            {
                return true;
            }
        },
    }
    false
}

fn ipv4_mapped_from_v6(v6: &Ipv6Addr) -> Option<Ipv4Addr> {
    let seg = v6.segments();
    if seg[0] == 0 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0 && seg[4] == 0 && seg[5] == 0xffff {
        let octets = v6.octets();
        return Some(Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]));
    }
    None
}

/// P0-4: 解析 URL 主机名，校验所有解析到的 IP 都在白名单（此处为非黑名单）。
/// 同时拒绝空主机名、纯 IP 字面量解析失败。
async fn assert_url_safe(url: &str) -> Result<(), NodeError> {
    let parsed = url::Url::parse(url)
        .map_err(|e| NodeError::exec_failed("http_error", format!("Invalid URL: {e}")))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| NodeError::exec_failed("http_error", "URL has no host".to_string()))?;

    // scheme 必须是 http/https
    match parsed.scheme() {
        "http" | "https" => {},
        other => {
            return Err(NodeError::exec_failed(
                "http_error",
                format!("URL scheme '{other}' is not allowed (only http/https)"),
            ));
        },
    }

    // 解析所有 A/AAAA 记录，任一落在黑名单就拒绝
    let addrs: Vec<IpAddr> = lookup_host((host, 0u16))
        .await
        .map_err(|e| {
            NodeError::exec_failed("http_error", format!("DNS lookup failed for {host}: {e}"))
        })?
        .map(|sa| sa.ip())
        .collect();

    if addrs.is_empty() {
        return Err(NodeError::exec_failed(
            "http_error",
            format!("DNS lookup for {host} returned no addresses"),
        ));
    }

    for ip in &addrs {
        if is_blocked_ip(ip) {
            return Err(NodeError::exec_failed(
                "http_error",
                format!(
                    "SSRF blocked: {host} resolves to blocked IP {ip} \
                     (loopback / private / link-local / cloud-metadata / multicast)"
                ),
            ));
        }
    }
    Ok(())
}

#[async_trait]
impl NodeExecutorTrait for HttpRequestExecutor {
    fn node_type(&self) -> &'static str {
        "httpRequest"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::HttpRequest(http_node) = node else {
            return Err(NodeError::type_mismatch(
                "httpRequest".to_string(),
                crate::work_engine::node_executor_trait::node_type_name(node).to_string(),
            ));
        };

        let config = &http_node.config;
        if config.url.trim().is_empty() {
            return Err(NodeError::exec_failed(
                "http_error",
                "HTTP Request URL is empty".to_string(),
            ));
        }

        // ── Dry Run 短路 ──
        // 单步调试模式下不发真实请求，返回 200 OK + mock body，
        // 完全避免 SSRF/网络副作用（DNS 解析、TCP 连接、TLS 握手等均不触发）。
        if context.dry_run {
            return Ok(NodeOutput {
                output: serde_json::json!({
                    "status": 200,
                    "status_text": "success",
                    "headers": {},
                    "body": "[DRY RUN] HTTP 200 OK 模拟响应",
                    "dry_run": true,
                    "node_id": node.base_id(),
                }),
                output_var: if config.output_var.is_empty() {
                    None
                } else {
                    Some(config.output_var.clone())
                },
                control: None,
            });
        }

        // P0-4: SSRF 校验必须在发请求前做（IP 解析后再发，避免 DNS rebinding）
        assert_url_safe(&config.url).await?;

        // P6.0-⑥ 批 E：协议细节（reqwest / 超时 / 重定向策略）归实现层；
        // 本执行器保留的是**策略**——方法白名单、body_type 语义、SSRF 校验与输出包装。
        let seam = context.http_client.as_ref().ok_or_else(|| {
            NodeError::exec_failed(
                "WORKFLOW_HTTP_NOT_INJECTED",
                "workflow http client not injected into ExecutionState",
            )
        })?;

        let method = config.method.to_uppercase();
        if !matches!(
            method.as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
        ) {
            return Err(NodeError::exec_failed(
                "http_error",
                format!("Unsupported HTTP method: {}", config.method),
            ));
        }

        let mut headers = config.headers.clone();
        let body = config.body.clone();
        let mut body_is_json = false;
        // body_type 只在 POST 上生效（与原语义一致）；PUT / PATCH 按原文发送
        if method == "POST" && body.is_some() {
            match config.body_type.as_str() {
                "json" => body_is_json = true,
                "form" => {
                    headers.insert(
                        "content-type".to_string(),
                        "application/x-www-form-urlencoded".to_string(),
                    );
                },
                _ => {},
            }
        }

        let response = seam
            .send(axagent_harness::WorkflowHttpRequest {
                method,
                url: config.url.clone(),
                headers,
                body,
                body_is_json,
                timeout_secs: config.timeout_secs,
                // P0-4: 禁止跟随重定向 —— 攻击者可重定向到内网绕过校验
                follow_redirects: false,
            })
            .await
            .map_err(|e| NodeError::exec_failed("http_error", e.to_string()))?;

        let status = response.status;
        let output = serde_json::json!({
            "status": status,
            "status_text": if (200..300).contains(&status) { "success" } else { "error" },
            "headers": response.headers,
            "body": response.body,
            "node_id": node.base_id(),
        });

        Ok(NodeOutput {
            output,
            output_var: if config.output_var.is_empty() {
                None
            } else {
                Some(config.output_var.clone())
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

    /// 测试替身：记录请求并回投固定响应。
    struct MockHttp {
        calls: StdMutex<Vec<axagent_harness::WorkflowHttpRequest>>,
        status: u16,
        body: String,
    }

    #[async_trait]
    impl axagent_harness::WorkflowHttpClient for MockHttp {
        async fn send(
            &self,
            request: axagent_harness::WorkflowHttpRequest,
        ) -> Result<axagent_harness::WorkflowHttpResponse, String> {
            self.calls.lock().unwrap().push(request);
            Ok(axagent_harness::WorkflowHttpResponse {
                status: self.status,
                headers: std::collections::HashMap::new(),
                body: self.body.clone(),
            })
        }
    }

    fn node_with(method: &str, body_type: &str, body: Option<&str>) -> WorkflowNode {
        serde_json::from_value(json!({
            "type": "httpRequest",
            "id": "h1",
            "title": "HTTP",
            "position": {"x": 0.0, "y": 0.0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {
                "url": "https://example.com/api",
                "method": method,
                "body_type": body_type,
                "body": body,
                "headers": {"x-k": "v"},
                "timeout_secs": 20,
                "output_var": "h_out"
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
        let err =
            HttpRequestExecutor.execute(&node_with("GET", "json", None), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "WORKFLOW_HTTP_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn seam_receives_request_and_output_contract_holds() {
        let client = Arc::new(MockHttp {
            calls: StdMutex::new(vec![]),
            status: 201,
            body: "{\"ok\":true}".to_string(),
        });
        let ctx = ctx_with(Some(client.clone()));
        let out = HttpRequestExecutor
            .execute(&node_with("post", "json", Some("{\"a\":1}")), &ctx)
            .await
            .unwrap();
        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].url, "https://example.com/api");
        assert!(calls[0].body_is_json, "POST + body_type=json ⇒ body_is_json");
        assert_eq!(calls[0].headers.get("x-k").map(String::as_str), Some("v"));
        assert!(!calls[0].follow_redirects, "SSRF 策略：禁止跟随重定向");
        assert_eq!(calls[0].timeout_secs, 20);
        drop(calls);
        assert_eq!(out.output["status"], 201);
        assert_eq!(out.output["status_text"], "success");
        assert_eq!(out.output["body"], "{\"ok\":true}");
        assert_eq!(out.output_var, Some("h_out".to_string()));
    }

    #[tokio::test]
    async fn form_body_type_sets_content_type() {
        let client = Arc::new(MockHttp {
            calls: StdMutex::new(vec![]),
            status: 200,
            body: "ok".to_string(),
        });
        let ctx = ctx_with(Some(client.clone()));
        HttpRequestExecutor
            .execute(&node_with("POST", "form", Some("a=1&b=2")), &ctx)
            .await
            .unwrap();
        let calls = client.calls.lock().unwrap();
        assert_eq!(
            calls[0].headers.get("content-type").map(String::as_str),
            Some("application/x-www-form-urlencoded")
        );
        assert!(!calls[0].body_is_json);
    }

    #[tokio::test]
    async fn unsupported_method_rejected_before_seam() {
        let client =
            Arc::new(MockHttp { calls: StdMutex::new(vec![]), status: 200, body: String::new() });
        let ctx = ctx_with(Some(client.clone()));
        let err =
            HttpRequestExecutor.execute(&node_with("TRACE", "raw", None), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => assert_eq!(code, "http_error"),
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
        assert_eq!(client.calls.lock().unwrap().len(), 0, "白名单拒绝不得触达接缝");
    }

    #[tokio::test]
    async fn dry_run_short_circuits_without_seam() {
        let mut ctx = ctx_with(None);
        ctx.dry_run = true;
        let out = HttpRequestExecutor.execute(&node_with("GET", "raw", None), &ctx).await.unwrap();
        assert_eq!(out.output["dry_run"], true);
        assert_eq!(out.output["status"], 200);
    }
}
