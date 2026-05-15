//! AgentRunner 实现 — 通过 ProviderAdapter 调用真实的 LLM。
//!
//! 股票分析专家只需文本生成，无需工具调用，因此直接使用
//! `ProviderAdapter::chat()` 进行一次性 LLM 调用，避免
//! SessionManager / AxAgentApiClient 的复杂接线。

use std::sync::Arc;

use async_trait::async_trait;
use axagent_core::types::{ChatContent, ChatMessage, ChatRequest};
use axagent_providers::{ProviderAdapter, ProviderRequestContext};

use crate::decision::AgentRunner;

/// 基于 ProviderAdapter 的 AgentRunner 实现。
///
/// 每个 `run_agent` 调用构建一个包含 system + user 消息的 ChatRequest，
/// 通过 provider adapter 直接调用 LLM，返回文本响应。
pub struct SessionManagerRunner {
    /// LLM provider 适配器（OpenAI / Anthropic / Gemini / ...）
    adapter: Arc<dyn ProviderAdapter>,
    /// provider 请求上下文（API key、base URL 等）
    ctx: ProviderRequestContext,
    /// 模型 ID
    model: String,
    /// 温度参数 (0.0 = 确定性, 1.0 = 创造性)
    temperature: Option<f64>,
    /// 最大输出 token 数
    max_tokens: Option<u32>,
}

impl SessionManagerRunner {
    /// 创建新的 SessionManagerRunner
    pub fn new(
        adapter: Arc<dyn ProviderAdapter>,
        ctx: ProviderRequestContext,
        model: String,
    ) -> Self {
        Self {
            adapter,
            ctx,
            model,
            temperature: Some(0.3), // 股票分析偏向确定性
            max_tokens: Some(4096),
        }
    }

    /// 设置温度参数
    pub fn with_temperature(mut self, temperature: Option<f64>) -> Self {
        self.temperature = temperature;
        self
    }

    /// 设置最大 token 数
    pub fn with_max_tokens(mut self, max_tokens: Option<u32>) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

#[async_trait]
impl AgentRunner for SessionManagerRunner {
    async fn run_agent(
        &self,
        expert_id: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<String, String> {
        let _ = expert_id; // 用于日志，当前通过 system/user prompt 区分专家

        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: ChatContent::Text(system_prompt.to_string()),
                tool_calls: None,
                tool_call_id: None,
                thinking: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Text(user_prompt.to_string()),
                tool_calls: None,
                tool_call_id: None,
                thinking: None,
            },
        ];

        let request = ChatRequest {
            model: self.model.clone(),
            messages,
            stream: false, // 非流式，一次性获取完整响应
            temperature: self.temperature,
            top_p: None,
            max_tokens: self.max_tokens,
            tools: None,
            thinking_budget: None,
            use_max_completion_tokens: None,
            thinking_param_style: None,
            api_mode: None,
            instructions: None,
            conversation: None,
            previous_response_id: None,
            store: None,
        };

        let response = self
            .adapter
            .chat(&self.ctx, request)
            .await
            .map_err(|e| format!("LLM 调用失败: {e}"))?;

        if response.content.is_empty() {
            Err(format!("[{expert_id}] LLM 返回空响应"))
        } else {
            Ok(response.content)
        }
    }
}
