//! Twitter / X 扫描器
//!
//! 通过 **Twitter API v2** (`GET /2/tweets/search/recent`) 采集 AI 相关讨论与趋势信号。
//!
//! ## 合规约束
//!
//! 原实现把 `base_url` 指向第三方 Nitter 镜像并逐行扫描 HTML 找关键词，
//! 这既是规避官方访问途径，也会把页面里的 JS/CSS 当成正文。现已改为：
//! - 只请求官方 API 端点，响应按 JSON 解析；
//! - 未配置 `TWITTER_BEARER_TOKEN` 时直接跳过，不发起任何请求；
//! - 使用真实 UA，不伪造浏览器指纹。

use super::marketplace_scanner::{MarketplaceScanner, RawLead};
use crate::tools::scanner_common;
use async_trait::async_trait;

/// Twitter 扫描器
pub struct TwitterScanner {
    http: reqwest::Client,
    /// 可选的 Bearer Token，用于官方 API
    api_token: Option<String>,
    /// 基础 URL，默认为 Twitter 官方搜索 API
    base_url: String,
}

impl TwitterScanner {
    pub fn new() -> Self {
        Self::with_config(None, None)
    }

    /// 携带官方 API 凭证构造
    pub fn with_token(api_token: Option<String>) -> Self {
        Self::with_config(api_token, None)
    }

    /// 从配置创建（凭证 + 端点透传）
    ///
    /// `api_token` 未提供时回退读环境变量（桌面 GUI 进程通常不带环境变量，
    /// 平台配置里的 token 由路由层经本方法直接注入 —— 凭证三层断链修复）。
    pub fn with_config(api_token: Option<String>, base_url: Option<String>) -> Self {
        let http = scanner_common::build_http_client(scanner_common::DEFAULT_TIMEOUT_SECS);
        let api_token = api_token.or_else(|| std::env::var("TWITTER_BEARER_TOKEN").ok());
        Self {
            http,
            api_token,
            base_url: base_url.unwrap_or_else(|| "https://api.twitter.com/2".to_string()),
        }
    }

    /// 构建搜索 URL（Twitter API v2 recent search）
    fn build_search_url(&self, query: &str, max_results: u32) -> String {
        let encoded_query = scanner_common::encode_query(query);
        let max_results = max_results.clamp(10, 100);
        format!(
            "{}/tweets/search/recent?query={}&max_results={}&tweet.fields=public_metrics,created_at",
            self.base_url, encoded_query, max_results
        )
    }

    /// 构建请求头（真实身份 + Bearer 认证）
    fn build_headers(&self) -> reqwest::header::HeaderMap {
        scanner_common::build_headers(self.api_token.as_deref(), "application/json")
    }

    /// AI/技术趋势关键词
    fn trend_keywords() -> Vec<&'static str> {
        vec![
            "llm",
            "gpt",
            "ai agent",
            "agentic",
            "rag",
            "vector database",
            "embedding",
            "fine-tuning",
            "opensource alternative",
            "how to integrate",
            "struggling with",
            "looking for",
            "need help",
        ]
    }

    /// 检查推文是否包含趋势关键词或需求信号
    fn extract_signals(tweet_text: &str) -> Option<Vec<String>> {
        let text_lower = tweet_text.to_lowercase();
        let mut detected = Vec::new();

        // 技术趋势
        for kw in Self::trend_keywords() {
            if text_lower.contains(kw) {
                detected.push(format!("trend:{}", kw));
            }
        }

        // 需求模式
        let demand_patterns = [
            (
                "demand:problem",
                vec![
                    "how to",
                    "how do i",
                    "trying to",
                    "struggling",
                    "issue with",
                    "bug in",
                    "doesn't work",
                    "not working",
                ],
            ),
            (
                "demand:integration",
                vec![
                    "integrate",
                    "integration with",
                    "connect to",
                    "works with",
                    "supports",
                    "plugin for",
                    "sdk for",
                ],
            ),
            (
                "demand:feature_request",
                vec![
                    "would love",
                    "would be great if",
                    "is there a way",
                    "any plans",
                    "feature request",
                    "need a",
                    "looking for a",
                ],
            ),
            (
                "demand:comparison",
                vec![
                    "vs",
                    "versus",
                    "compare",
                    "better than",
                    "alternative to",
                    "why use",
                    "which is best",
                ],
            ),
        ];

        for (tag, patterns) in &demand_patterns {
            if patterns.iter().any(|p| text_lower.contains(p)) {
                detected.push(tag.to_string());
            }
        }

        if detected.is_empty() {
            None
        } else {
            Some(detected)
        }
    }

    /// 从推文中提取核心需求描述
    fn extract_summary(tweet_text: &str) -> String {
        let text = tweet_text.replace('\n', " ").trim().to_string();
        scanner_common::truncate_chars(&text, 150)
    }
}

impl Default for TwitterScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MarketplaceScanner for TwitterScanner {
    fn platform(&self) -> String {
        "twitter".to_string()
    }

    async fn search(&self, q: &str) -> Result<Vec<RawLead>, String> {
        if q.is_empty() {
            return Ok(Vec::new());
        }

        // 合规门禁：无 Bearer Token 直接跳过，绝不退化为 Nitter 镜像抓取
        scanner_common::require_official_api_credential(
            "twitter",
            self.api_token.as_deref(),
            &self.base_url,
        )?;

        let url = self.build_search_url(q, 20);
        let headers = self.build_headers();

        tracing::info!(query = q, "[TwitterScanner] 发起搜索请求");

        let response = self.http.get(&url).headers(headers).send().await;

        let mut leads = Vec::new();

        match response {
            Ok(resp) if resp.status().is_success() => {
                // Twitter v2 返回 JSON：{ "data": [ { id, text, public_metrics } ] }
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    let Some(tweets) = scanner_common::pick_items(&body, &["data"]) else {
                        tracing::debug!("[TwitterScanner] 响应中无 data 数组，跳过");
                        return Ok(leads);
                    };

                    for tweet in tweets {
                        let text = scanner_common::pick_str(tweet, &["text"]).unwrap_or_default();
                        if text.is_empty() {
                            continue;
                        }
                        let Some(signals) = Self::extract_signals(text) else {
                            continue;
                        };

                        let tweet_id = scanner_common::pick_str(tweet, &["id"]).unwrap_or("");
                        let metrics = tweet.get("public_metrics").cloned().unwrap_or_default();

                        leads.push(RawLead {
                            platform: "twitter".to_string(),
                            title: format!("Twitter Signal: {}", signals.join(", ")),
                            description: Self::extract_summary(text),
                            url: if tweet_id.is_empty() {
                                self.base_url.clone()
                            } else {
                                format!("https://twitter.com/i/web/status/{}", tweet_id)
                            },
                            price_text: None,
                            contact: None,
                            contact_email: None,
                            contact_phone: None,
                            snapshot: serde_json::json!({
                                "source": "twitter_scanner",
                                "signals": signals,
                                "raw_text": text,
                                "public_metrics": metrics,
                            }),
                        });
                    }
                }
            },
            Ok(resp) => {
                let status = resp.status();
                tracing::warn!(status = status.as_u16(), "[TwitterScanner] 请求失败");
                // 如果是速率限制，返回错误
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err("Twitter API 速率限制".to_string());
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "[TwitterScanner] 网络请求异常，返回空结果");
                // 网络错误时不中断流程，返回空结果
            },
        }

        tracing::info!(query = q, filtered = leads.len(), "[TwitterScanner] 搜索完成");

        Ok(leads)
    }
}

// 预留的 Twitter API 响应结构体
// structs are reserved for future use with official API

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform() {
        let scanner = TwitterScanner::new();
        assert_eq!(scanner.platform(), "twitter");
    }

    #[test]
    fn test_extract_signals_demand() {
        // 包含需求信号
        let signals =
            TwitterScanner::extract_signals("How to integrate this LLM with my existing API?");
        assert!(signals.is_some());
        let sigs = signals.unwrap();
        assert!(sigs.iter().any(|s| s.contains("integration")));
    }

    #[test]
    fn test_extract_signals_trend() {
        // 包含技术趋势
        let signals = TwitterScanner::extract_signals(
            "Just tried the new RAG approach with vector databases, game changer!",
        );
        assert!(signals.is_some());
        assert!(signals.unwrap().iter().any(|s| s.contains("vector database")));
    }

    #[test]
    fn test_extract_signals_noise() {
        // 无关内容
        let signals = TwitterScanner::extract_signals("Had a great lunch today.");
        assert!(signals.is_none());
    }

    #[test]
    fn test_build_search_url() {
        let scanner = TwitterScanner::new();
        let url = scanner.build_search_url("ai agent", 10);
        // 空格编码为 %20（不是 +）
        scanner_common::assert_url_query_param(&url, "query", "ai agent");
        assert!(url.contains("search"));
    }

    #[tokio::test]
    async fn test_search_empty_query() {
        let scanner = TwitterScanner::new();
        let result = scanner.search("").await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
