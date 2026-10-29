// SPDX-License-Identifier: AGPL-3.0-only

// 1.97 起 clippy::items_after_test_module 升级为 warn(在 `-D warnings` 下变 deny),
// 历史上把测试模块放在文件中间以贴近被测代码,这里显式 allow 保留现有排版。
#![allow(clippy::items_after_test_module)]

use std::sync::Arc;

use async_trait::async_trait;
use axagent_harness::constants::default_url;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::*;
use futures::Stream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::compat::impl_default_via_new;
use crate::transport::sse_data_payload;
use crate::{ProviderAdapter, ProviderRequestContext, build_http_client, parse_base64_data_url};

const DEFAULT_BASE_URL: &str = default_url::GEMINI_BASE;

pub struct GeminiAdapter {
    client: reqwest::Client,
}

impl_default_via_new!(GeminiAdapter);

impl GeminiAdapter {
    pub fn new() -> Self {
        Self {
            client: crate::build_default_http_client().unwrap_or_else(|e| {
                tracing::warn!(
                    "{}",
                    axagent_harness::i18n::fmt_msg(
                        axagent_harness::i18n::I18nKey::ProviderHttpClientBuildFailed,
                        &format!("Gemini: {e}")
                    )
                );
                reqwest::Client::new()
            }),
        }
    }

    fn base_url(ctx: &ProviderRequestContext) -> String {
        ctx.base_url.clone().unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
    }

    #[allow(clippy::result_large_err)]
    fn get_client(&self, ctx: &ProviderRequestContext) -> Result<reqwest::Client> {
        match &ctx.proxy_config {
            Some(c) if c.proxy_type.as_deref() != Some("none") => build_http_client(Some(c)),
            _ => Ok(self.client.clone()),
        }
    }
}

// --- Internal types ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiToolDeclaration>>,
}

#[derive(Serialize, Deserialize)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inline_data: Option<GeminiInlineData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thought: Option<bool>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiInlineData {
    mime_type: String,
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_config: Option<GeminiThinkingConfig>,
    /// Structured Output: "application/json"
    #[serde(skip_serializing_if = "Option::is_none")]
    response_mime_type: Option<String>,
    /// Structured Output: JSON Schema 约束
    #[serde(skip_serializing_if = "Option::is_none")]
    response_schema: Option<serde_json::Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiThinkingConfig {
    thinking_budget: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    prompt_token_count: Option<u32>,
    candidates_token_count: Option<u32>,
    /// Gemini 上下文缓存命中 token 数 (cachedContentTokenCount).
    #[serde(default)]
    cached_content_token_count: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiModelsResponse {
    models: Option<Vec<GeminiModel>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiModel {
    name: String,
    display_name: Option<String>,
    supported_generation_methods: Option<Vec<String>>,
    #[serde(rename = "inputTokenLimit")]
    input_token_limit: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiToolDeclaration {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

fn convert_tools_to_gemini(tools: &Option<Vec<ChatTool>>) -> Option<Vec<GeminiToolDeclaration>> {
    tools.as_ref().map(|ts| {
        vec![GeminiToolDeclaration {
            function_declarations: ts
                .iter()
                .map(|t| GeminiFunctionDeclaration {
                    name: t.function.name.clone(),
                    description: t.function.description.clone().unwrap_or_default(),
                    parameters: t.function.parameters.clone(),
                })
                .collect(),
        }]
    })
}

fn convert_messages(messages: &[ChatMessage]) -> (Option<GeminiContent>, Vec<GeminiContent>) {
    // 与 OpenAI 路径同一套净化判据：空名调用/无响应调用/孤儿 functionResponse 剔除，
    // 空 id 合成并让配对消息回写同一 id（判据见 crate::tool_calls）。
    match crate::tool_calls::sanitize_messages(messages) {
        Some(sanitized) => convert_raw_messages(&sanitized),
        None => convert_raw_messages(messages),
    }
}

fn convert_raw_messages(messages: &[ChatMessage]) -> (Option<GeminiContent>, Vec<GeminiContent>) {
    let mut system_instruction = None;
    let mut contents = Vec::new();

    // Pre-build a map from tool_call_id to function name for Gemini's functionResponse
    let mut tool_id_to_name: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for msg in messages {
        if let Some(ref tcs) = msg.tool_calls {
            for tc in tcs {
                tool_id_to_name.insert(tc.id.clone(), tc.function.name.clone());
            }
        }
    }

    for msg in messages {
        if msg.role == "system" {
            let parts = match &msg.content {
                ChatContent::Text(text) => vec![GeminiPart {
                    text: Some(text.clone()),
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                    thought: None,
                }],
                ChatContent::Multipart(parts) => parts
                    .iter()
                    .filter_map(|p| {
                        if let Some(text) = &p.text {
                            Some(GeminiPart {
                                text: Some(text.clone()),
                                inline_data: None,
                                function_call: None,
                                function_response: None,
                                thought: None,
                            })
                        } else if let Some(img) = &p.image_url {
                            parse_base64_data_url(&img.url).map(|(mime_type, data)| GeminiPart {
                                text: None,
                                inline_data: Some(GeminiInlineData { mime_type, data }),
                                function_call: None,
                                function_response: None,
                                thought: None,
                            })
                        } else {
                            None
                        }
                    })
                    .collect(),
            };
            system_instruction = Some(GeminiContent { role: None, parts });
            continue;
        }

        match msg.role.as_str() {
            "tool" => {
                // Gemini needs the function NAME, not the call ID
                // Look up the actual name from the tool_call_id → name map.
                // 若查不到 (异常数据 / tool 已被截断),跳过此消息并 warn,
                // 避免把 unknown 投递给上游引发 400。
                let tool_name = match msg
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| tool_id_to_name.get(id).map(|s| s.as_str()))
                {
                    Some(name) => name.to_string(),
                    None => {
                        tracing::warn!(
                            "gemini: tool message dropped — tool_call_id {:?} not in id→name map",
                            msg.tool_call_id
                        );
                        continue;
                    },
                };
                let result_value: serde_json::Value =
                    serde_json::from_str(&crate::extract_text_content(&msg.content)).unwrap_or(
                        serde_json::json!({ "result": crate::extract_text_content(&msg.content) }),
                    );
                contents.push(GeminiContent {
                    role: Some("user".to_string()),
                    parts: vec![GeminiPart {
                        text: None,
                        inline_data: None,
                        function_call: None,
                        function_response: Some(GeminiFunctionResponse {
                            name: tool_name.to_string(),
                            response: result_value,
                        }),
                        thought: None,
                    }],
                });
            },
            "assistant" if msg.tool_calls.is_some() => {
                let mut parts = Vec::new();
                let text = crate::extract_text_content(&msg.content);
                let (visible_text, reasoning) = crate::extract_reasoning_from_text(&text);
                if let Some(ref r) = reasoning {
                    parts.push(GeminiPart {
                        text: Some(r.clone()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                        thought: Some(true),
                    });
                }
                if !visible_text.is_empty() {
                    parts.push(GeminiPart {
                        text: Some(visible_text),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                        thought: None,
                    });
                }
                if let Some(ref tcs) = msg.tool_calls {
                    for tc in tcs {
                        // `args` 只接受 JSON 对象：非法/非对象参数统一降级为 {}（带 warn）
                        let args =
                            crate::tool_calls::normalize_arguments_object(&tc.function.arguments);
                        parts.push(GeminiPart {
                            text: None,
                            inline_data: None,
                            function_call: Some(GeminiFunctionCall {
                                name: tc.function.name.clone(),
                                args,
                            }),
                            function_response: None,
                            thought: None,
                        });
                    }
                }
                contents.push(GeminiContent { role: Some("model".to_string()), parts });
            },
            _ => {
                let mut parts = Vec::new();
                match &msg.content {
                    ChatContent::Text(text) => {
                        let (visible, reasoning) = crate::extract_reasoning_from_text(text);
                        if let Some(ref r) = reasoning {
                            parts.push(GeminiPart {
                                text: Some(r.clone()),
                                inline_data: None,
                                function_call: None,
                                function_response: None,
                                thought: Some(true),
                            });
                        }
                        if !visible.is_empty() {
                            parts.push(GeminiPart {
                                text: Some(visible),
                                inline_data: None,
                                function_call: None,
                                function_response: None,
                                thought: None,
                            });
                        }
                    },
                    ChatContent::Multipart(multipart) => {
                        for p in multipart {
                            if let Some(text) = &p.text {
                                let (visible, reasoning) = crate::extract_reasoning_from_text(text);
                                if let Some(ref r) = reasoning {
                                    parts.push(GeminiPart {
                                        text: Some(r.clone()),
                                        inline_data: None,
                                        function_call: None,
                                        function_response: None,
                                        thought: Some(true),
                                    });
                                }
                                if !visible.is_empty() {
                                    parts.push(GeminiPart {
                                        text: Some(visible),
                                        inline_data: None,
                                        function_call: None,
                                        function_response: None,
                                        thought: None,
                                    });
                                }
                            } else if let Some(img) = &p.image_url
                                && let Some((mime_type, data)) = parse_base64_data_url(&img.url)
                            {
                                parts.push(GeminiPart {
                                    text: None,
                                    inline_data: Some(GeminiInlineData { mime_type, data }),
                                    function_call: None,
                                    function_response: None,
                                    thought: None,
                                });
                            }
                        }
                    },
                }

                let role = match msg.role.as_str() {
                    "assistant" => "model",
                    other => other,
                };

                contents.push(GeminiContent { role: Some(role.to_string()), parts });
            },
        }
    }

    (system_instruction, contents)
}

fn make_gen_config(request: &ChatRequest) -> Option<GeminiGenerationConfig> {
    let thinking_config =
        request.thinking_budget.map(|b| GeminiThinkingConfig { thinking_budget: b });

    // Structured Output 转换（ResponseFormat → Gemini responseMimeType + responseSchema）
    let (response_mime_type, response_schema) = match &request.response_format {
        Some(ResponseFormat::JsonObject) => (Some("application/json".to_string()), None),
        Some(ResponseFormat::JsonSchema { schema, .. }) => {
            (Some("application/json".to_string()), Some(schema.clone()))
        },
        None => (None, None),
    };

    if request.temperature.is_some()
        || request.top_p.is_some()
        || request.max_tokens.is_some()
        || thinking_config.is_some()
        || response_mime_type.is_some()
    {
        Some(GeminiGenerationConfig {
            temperature: request.temperature,
            top_p: request.top_p,
            max_output_tokens: request.max_tokens,
            thinking_config,
            response_mime_type,
            response_schema,
        })
    } else {
        None
    }
}

fn usage_from_meta(meta: Option<GeminiUsageMetadata>) -> TokenUsage {
    meta.map(|u| TokenUsage {
        input_tokens: u.prompt_token_count.unwrap_or(0),
        output_tokens: u.candidates_token_count.unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: u.cached_content_token_count.unwrap_or(0),
        cache_miss_input_tokens: None,
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn convert_messages_keeps_inline_image_parts() {
        let (_, contents) = convert_messages(&[ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Multipart(vec![
                ContentPart {
                    r#type: "text".to_string(),
                    text: Some("Describe this image".to_string()),
                    image_url: None,
                },
                ContentPart {
                    r#type: "image_url".to_string(),
                    text: None,
                    image_url: Some(ImageUrl { url: "data:image/png;base64,YWJj".to_string() }),
                },
            ]),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
        }]);

        assert_eq!(
            serde_json::to_value(&contents[0]).expect("测试应成功"),
            json!({
                "role": "user",
                "parts": [
                    { "text": "Describe this image" },
                    {
                        "inlineData": {
                            "mimeType": "image/png",
                            "data": "YWJj"
                        }
                    }
                ]
            })
        );
    }

    #[test]
    fn function_call_without_name_is_dropped() {
        let fc = GeminiFunctionCall { name: "  ".to_string(), args: json!({}) };
        assert!(tool_call_from_function_call(&fc).is_none());
    }

    #[test]
    fn function_call_args_are_forced_to_object() {
        let fc = GeminiFunctionCall { name: "read_file".to_string(), args: json!([1, 2]) };
        let call = tool_call_from_function_call(&fc).expect("测试：应保留");
        assert_eq!(call.function.name, "read_file");
        assert_eq!(call.function.arguments, "{}");
    }

    /// 序列化接线测试：空名 functionCall 与无承接的 functionResponse 都不上网。
    #[test]
    fn convert_messages_drops_nameless_function_call_and_orphan_response() {
        let assistant = ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::Text(String::new()),
            tool_calls: Some(vec![ToolCall {
                id: "call_a".to_string(),
                call_type: "function".to_string(),
                function: ToolCallFunction { name: String::new(), arguments: "{}".to_string() },
            }]),
            tool_call_id: None,
            thinking: None,
        };
        let orphan = ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::Text("ok".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_a".to_string()),
            thinking: None,
        };

        let (_, contents) = convert_messages(&[assistant, orphan]);
        assert!(contents.is_empty(), "空名调用与其结果都不应出现在 contents");
    }
}

fn simple_id() -> String {
    format!("gemini-{}", uuid::Uuid::new_v4())
}

fn tool_call_id() -> String {
    format!("gemini-fc-{}", uuid::Uuid::new_v4())
}

/// 把 Gemini 的 `functionCall` 归一化为内部 ToolCall：空名丢弃、`args` 强制 JSON 对象。
///
/// Gemini 没有调用 id（按函数名配对），所以 id 由本地生成；空名既无法执行、也无法
/// 写回历史（回写会被上游判「无效函数调用」），只有丢弃这一条路。
fn tool_call_from_function_call(
    fc: &GeminiFunctionCall,
) -> Option<axagent_harness::types::ToolCall> {
    let name = fc.name.trim();
    if name.is_empty() {
        tracing::warn!(
            target: "axagent.providers.tool_call",
            "gemini: functionCall 缺少函数名，已丢弃（无法执行）"
        );
        return None;
    }
    let args = match &fc.args {
        serde_json::Value::Object(_) => fc.args.clone(),
        other => {
            tracing::warn!(
                target: "axagent.providers.tool_call",
                args = %other,
                "gemini: functionCall args 不是 JSON 对象，降级为 {{}}"
            );
            serde_json::Value::Object(serde_json::Map::new())
        },
    };
    let arguments = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string());
    Some(axagent_harness::types::ToolCall {
        id: tool_call_id(),
        call_type: "function".to_string(),
        function: axagent_harness::types::ToolCallFunction { name: name.to_string(), arguments },
    })
}

#[async_trait]
impl ProviderAdapter for GeminiAdapter {
    async fn chat(
        &self,
        ctx: &ProviderRequestContext,
        request: Arc<ChatRequest>,
    ) -> Result<ChatResponse> {
        let base_url = Self::base_url(ctx);
        // 不在 URL 中传递 API key,改用 x-goog-api-key header (更安全,且避免 URL 日志泄露)
        let url = format!("{}/models/{}:generateContent", base_url, request.model);

        let (system_instruction, contents) = convert_messages(&request.messages);
        let body = GeminiRequest {
            contents,
            system_instruction,
            generation_config: make_gen_config(&request),
            tools: convert_tools_to_gemini(&request.tools),
        };

        let resp = crate::apply_request_headers(
            self.get_client(ctx)?.post(&url).header("x-goog-api-key", &ctx.api_key).json(&body),
            ctx,
        )
        .send()
        .await
        .map_err(|e| AxAgentError::Provider(format!("Request to {url} failed: {e}")))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let t = resp.text().await.unwrap_or_default();
            return Err(AxAgentError::Provider(format!("Gemini API error {s}: {t}")));
        }

        let gr: GeminiResponse =
            resp.json().await.map_err(|e| AxAgentError::Provider(format!("Parse error: {e}")))?;

        let parts = gr
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.content.as_ref())
            .map(|c| &c.parts);

        let mut content = String::new();
        let mut thinking = String::new();
        let mut tool_calls: Vec<axagent_harness::types::ToolCall> = Vec::new();

        if let Some(parts) = parts {
            for part in parts {
                if let Some(ref text) = part.text {
                    if part.thought == Some(true) {
                        thinking.push_str(text);
                    } else {
                        content.push_str(text);
                    }
                }
                if let Some(ref fc) = part.function_call
                    && let Some(call) = tool_call_from_function_call(fc)
                {
                    tool_calls.push(call);
                }
            }
        }

        Ok(ChatResponse {
            id: simple_id(),
            model: request.model.clone(),
            content,
            thinking: if thinking.is_empty() {
                None
            } else {
                Some(thinking)
            },
            usage: usage_from_meta(gr.usage_metadata),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
        })
    }

    fn chat_stream(
        &self,
        ctx: &ProviderRequestContext,
        request: ChatRequest,
        cancel_token: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Pin<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        let client = self.get_client(ctx).unwrap_or_else(|e| {
            tracing::warn!("Failed to build proxy-aware HTTP client, falling back to default: {e}");
            self.client.clone()
        });
        let api_key = ctx.api_key.clone();
        let custom_headers = ctx.custom_headers.clone();
        let provider_id = ctx.provider_id.clone();
        let header_timeout_secs = ctx.header_timeout_secs;
        let base_url = Self::base_url(ctx);
        // 不在 URL 中传递 API key,改用 x-goog-api-key header (更安全,且避免 URL 日志泄露)
        // Gemini 请求体里没有 model 字段（模型在 URL 上），单独留一份供诊断日志用。
        let model = request.model.clone();
        let url = format!("{}/models/{}:streamGenerateContent?alt=sse", base_url, request.model);

        let (system_instruction, contents) = convert_messages(&request.messages);
        let body = GeminiRequest {
            contents,
            system_instruction,
            generation_config: make_gen_config(&request),
            tools: convert_tools_to_gemini(&request.tools),
        };

        let (mut tx, rx) = futures::channel::mpsc::channel(256);

        tokio::spawn(async move {
            // 首字节预算与其它协议路径同口径（见 stream_budget）。
            let body_size = serde_json::to_string(&body).ok().map(|s| s.len()).unwrap_or(0);
            let budget =
                crate::stream_budget::resolve_header_budget(header_timeout_secs, body_size);
            let header_ctx = crate::stream_budget::HeaderBudgetContext {
                provider_id: provider_id.clone(),
                model: model.clone(),
                url: url.clone(),
            };
            let send_result =
                crate::stream_budget::send_with_header_budget(budget, &header_ctx, || {
                    crate::apply_stream_headers_to_request(
                        client.post(&url).header("x-goog-api-key", &api_key).json(&body),
                        &custom_headers,
                    )
                })
                .await;

            let resp = match send_result {
                Ok(outcome) => outcome.response,
                Err(crate::stream_budget::HeaderSendError::Hung { waited, budget, .. }) => {
                    let _ = tx.try_send(Err(AxAgentError::execution_with_source(
                        crate::stream_budget::hung_error_message("Gemini", waited, budget),
                        anyhow::anyhow!(
                            "response headers timeout after {}s (gateway hung)",
                            waited.as_secs()
                        ),
                    )));
                    return;
                },
                Err(crate::stream_budget::HeaderSendError::Request(e)) => {
                    let _ = tx.try_send(Err(AxAgentError::execution_with_source(
                        format!("Request to {url} failed: {}", super::diagnose_reqwest_error(&e)),
                        e,
                    )));
                    return;
                },
            };

            if !resp.status().is_success() {
                let s = resp.status();
                let t = resp.text().await.unwrap_or_default();
                let _ = tx.try_send(Err(AxAgentError::execution_with_source(
                    super::diagnose_http_status("Gemini", s, &t),
                    anyhow::anyhow!("HTTP {s}: {t}"),
                )));
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut buf = String::new();

            while let Some(chunk) = byte_stream.next().await {
                if let Some(ref token) = cancel_token
                    && token.load(std::sync::atomic::Ordering::Relaxed)
                {
                    let _ =
                        tx.try_send(Err(AxAgentError::Provider("Stream cancelled".to_string())));
                    return;
                }
                match chunk {
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(pos) = buf.find('\n') {
                            let line = buf[..pos].trim_end().to_string();
                            buf = buf[pos + 1..].to_string();

                            let Some(data) = sse_data_payload(&line) else {
                                continue;
                            };

                            match serde_json::from_str::<GeminiResponse>(data) {
                                Ok(gr) => {
                                    let parts = gr
                                        .candidates
                                        .as_ref()
                                        .and_then(|c| c.first())
                                        .and_then(|c| c.content.as_ref())
                                        .map(|c| &c.parts);

                                    let mut content: Option<String> = None;
                                    let mut thinking_chunk: Option<String> = None;
                                    let mut tool_calls_vec: Vec<axagent_harness::types::ToolCall> =
                                        Vec::new();

                                    if let Some(parts) = parts {
                                        for part in parts {
                                            if let Some(ref text) = part.text {
                                                if part.thought == Some(true) {
                                                    thinking_chunk = Some(text.clone());
                                                } else {
                                                    content = Some(text.clone());
                                                }
                                            }
                                            if let Some(ref fc) = part.function_call
                                                && let Some(call) = tool_call_from_function_call(fc)
                                            {
                                                tool_calls_vec.push(call);
                                            }
                                        }
                                    }

                                    let tool_calls = if tool_calls_vec.is_empty() {
                                        None
                                    } else {
                                        Some(tool_calls_vec)
                                    };

                                    let usage = gr.usage_metadata.map(|u| TokenUsage {
                                        input_tokens: u.prompt_token_count.unwrap_or(0),
                                        output_tokens: u.candidates_token_count.unwrap_or(0),
                                        cache_creation_input_tokens: 0,
                                        cache_read_input_tokens: u
                                            .cached_content_token_count
                                            .unwrap_or(0),
                                        cache_miss_input_tokens: None,
                                    });

                                    let _ = tx.try_send(Ok(ChatStreamChunk {
                                        content,
                                        thinking: thinking_chunk,
                                        done: false,
                                        is_final: None,
                                        usage,
                                        tool_calls,
                                    }));
                                },
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to parse SSE event JSON: {}. Data: {}",
                                        e,
                                        &data[..data.len().min(200)]
                                    );
                                },
                            }
                        }
                    },
                    Err(e) => {
                        let _ = tx.try_send(Err(AxAgentError::Provider(format!(
                            "Stream error: {e}. This may be caused by network instability, proxy issues, or the provider terminating the connection. Please try again."
                        ))));
                        return;
                    },
                }
            }

            let _ = tx.try_send(Ok(ChatStreamChunk {
                content: None,
                thinking: None,
                done: true,
                is_final: None,
                usage: None,
                tool_calls: None,
            }));
        });

        Box::pin(rx)
    }

    async fn list_models(&self, ctx: &ProviderRequestContext) -> Result<Vec<Model>> {
        let url = format!("{}/models", Self::base_url(ctx));

        let resp = crate::apply_request_headers(
            self.get_client(ctx)?.get(&url).header("x-goog-api-key", &ctx.api_key),
            ctx,
        )
        .send()
        .await
        .map_err(|e| AxAgentError::Provider(format!("Request to {url} failed: {e}")))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let t = resp.text().await.unwrap_or_default();
            return Err(AxAgentError::Provider(format!("Gemini API error {s}: {t}")));
        }

        let models: GeminiModelsResponse =
            resp.json().await.map_err(|e| AxAgentError::Provider(format!("Parse error: {e}")))?;

        Ok(models
            .models
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                m.supported_generation_methods.as_ref().is_none_or(|methods| {
                    methods.iter().any(|m| m == "generateContent" || m == "generateAnswer")
                })
            })
            .map(|m| {
                let model_id = m.name.strip_prefix("models/").unwrap_or(&m.name).to_string();
                let name = m.display_name.unwrap_or_else(|| model_id.clone());
                let model_type =
                    axagent_harness::types::provider_model::detect_model_type(&model_id);
                let mut caps = match model_type {
                    ModelType::Chat => vec![ModelCapability::TextChat],
                    ModelType::Embedding => vec![],
                    ModelType::Voice => vec![ModelCapability::RealtimeVoice],
                    ModelType::Decision => vec![],
                };
                if model_id.contains("pro") || model_id.contains("flash") {
                    caps.push(ModelCapability::Vision);
                }
                Model {
                    provider_id: ctx.provider_id.clone(),
                    model_id: model_id.clone(),
                    name,
                    group_name: None,
                    model_type,
                    capabilities: caps,
                    max_tokens: m.input_token_limit,
                    enabled: true,
                    param_overrides: None,
                    input_price_per_mtok: None,
                    output_price_per_mtok: None,
                }
            })
            .collect())
    }

    async fn embed(
        &self,
        ctx: &ProviderRequestContext,
        request: EmbedRequest,
    ) -> Result<EmbedResponse> {
        let base_url = Self::base_url(ctx);
        // 不在 URL 中传递 API key,改用 x-goog-api-key header
        let url = format!("{}/models/{}:batchEmbedContents", base_url, request.model);

        let requests: Vec<serde_json::Value> = request
            .input
            .iter()
            .map(|text| {
                let mut req = serde_json::json!({
                    "model": format!("models/{}", request.model),
                    "content": { "parts": [{ "text": text }] }
                });
                if let Some(dims) = request.dimensions {
                    req["outputDimensionality"] = serde_json::json!(dims);
                }
                req
            })
            .collect();

        let body = serde_json::json!({ "requests": requests });

        let resp = crate::apply_request_headers(
            self.get_client(ctx)?.post(&url).header("x-goog-api-key", &ctx.api_key).json(&body),
            ctx,
        )
        .send()
        .await
        .map_err(|e| AxAgentError::Provider(format!("Gemini embed request failed: {e}")))?;

        if !resp.status().is_success() {
            let s = resp.status();
            let t = resp.text().await.unwrap_or_default();
            return Err(AxAgentError::Provider(format!("Gemini embed API error {s}: {t}")));
        }

        #[derive(Deserialize)]
        struct GeminiBatchEmbedResponse {
            embeddings: Vec<GeminiEmbedValues>,
        }
        #[derive(Deserialize)]
        struct GeminiEmbedValues {
            values: Vec<f32>,
        }

        let result: GeminiBatchEmbedResponse = resp
            .json()
            .await
            .map_err(|e| AxAgentError::Provider(format!("Gemini embed parse error: {e}")))?;

        let dimensions = result.embeddings.first().map(|e| e.values.len()).unwrap_or(0);
        let embeddings: Vec<Vec<f32>> = result.embeddings.into_iter().map(|e| e.values).collect();

        Ok(EmbedResponse { embeddings, dimensions })
    }
}
