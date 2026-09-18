// SPDX-License-Identifier: AGPL-3.0-only

pub mod anthropic;
pub mod chat_completions;
pub mod responses;

pub use anthropic::AnthropicTransport;
pub use chat_completions::ChatCompletionsTransport;
pub use responses::ResponsesTransport;

use async_trait::async_trait;

/// 从一行 SSE 文本中取出 `data:` 载荷。
///
/// 规则：跳过空行与 `event:` 行；`data: `（含空格）与 `data:`（无空格）两种前缀都接受。
///
/// ⚠ 与 `chat_completions::parse_sse_chunk` **不是同一形态、不可互换**：
/// 后者接收**整块**文本并在内部按 `'\n'` 切分，不做「跨 chunk 残留行」缓冲；
/// 本函数只处理**调用方已切好的完整单行**。
///
/// [2026-09-13] 去重审计 P1-8：`anthropic.rs` 与 `gemini.rs` 的 `chat_stream` 中该判定块
/// **逐字相同**（各 10 行），已收敛至此。
///
/// **不适用者**（勿合并，行为不同）：`openai.rs` 与 `openai_responses.rs` 实现的是完整
/// SSE 事件语义 —— 空行是**事件边界**、多行 `data:` 需 `join("\n")`、并记录 `event:` 类型；
/// 它们与本函数的「单行独立解析」语义不同，机械合并会改变流式解析行为。
#[inline]
pub(crate) fn sse_data_payload(line: &str) -> Option<&str> {
    if line.is_empty() || line.starts_with("event:") {
        return None;
    }
    if let Some(d) = line.strip_prefix("data: ") {
        Some(d)
    } else {
        line.strip_prefix("data:")
    }
}

#[derive(Debug, Clone)]
pub struct TransportRequest {
    pub model: String,
    pub messages: Vec<TransportMessage>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub tools: Option<serde_json::Value>,
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct TransportMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<serde_json::Value>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TransportResponse {
    pub content: String,
    pub tool_calls: Option<Vec<TransportToolCall>>,
    pub usage: TransportUsage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TransportToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default)]
pub struct TransportUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[async_trait]
pub trait TransportProvider: Send + Sync {
    fn provider_name(&self) -> &'static str;

    async fn send(
        &self,
        request: TransportRequest,
        api_key: &str,
        base_url: Option<&str>,
    ) -> anyhow::Result<TransportResponse>;

    async fn send_streaming(
        &self,
        request: TransportRequest,
        api_key: &str,
        base_url: Option<&str>,
    ) -> anyhow::Result<
        Box<dyn futures::Stream<Item = anyhow::Result<TransportStreamChunk>> + Send + Unpin>,
    >;
}

#[derive(Debug, Clone)]
pub struct TransportStreamChunk {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<TransportToolCall>>,
    pub finish_reason: Option<String>,
    pub usage: Option<TransportUsage>,
}
