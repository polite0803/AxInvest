// SPDX-License-Identifier: AGPL-3.0-only

//! DeepSeek 原生适配器。
//!
//! DeepSeek 提供与 OpenAI Chat Completions 兼容的端点 (`/v1/chat/completions`)，
//! 因此 chat / chat_stream / embed 委托给 [`OpenAIAdapter`]。
//!
//! 特有能力：`deepseek-reasoner` 模型在响应中返回 `reasoning_content` 字段，
//! 表示深度思考过程。该字段已被 [`OpenAIAdapter`] 的 `extract_thinking` 函数
//! 解析并映射到 harness 的 `thinking` 字段，无需在此重复实现。
//!
//! 本适配器重写：
//! - **`list_models`** — 返回 DeepSeek 官方模型（`deepseek-chat` / `deepseek-reasoner`）。
//! - **`validate_key`** — 使用 DeepSeek 的 base URL 调用 `/models` 端点校验鉴权。

use std::sync::Arc;

use crate::compat::openai_compat_cloud_adapter;
use crate::openai::OpenAIAdapter;
use crate::{ProviderAdapter, ProviderRequestContext};
use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::*;
use futures::Stream;
use std::pin::Pin;

/// DeepSeek 默认 API 端点
const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";

/// DeepSeek 适配器。
///
/// chat / chat_stream / embed 委托给内部 OpenAI 适配器，
/// 因为 DeepSeek 在 `/v1/` 前缀下使用 OpenAI 兼容协议。
/// 模型列表与鉴权校验使用 DeepSeek 官方端点。
pub struct DeepSeekAdapter {
    inner: OpenAIAdapter,
}

// ── 委托样板由宏生成（new / Default / base_url / get_client +
//    trait 的 chat / chat_stream / list_models / validate_key / embed）──
//    与其余 4 家云厂商逐字一致，改 trait 签名时不会漏改某一家（见 compat.rs）。
openai_compat_cloud_adapter!(
    DeepSeekAdapter,
    default_base_url = DEFAULT_BASE_URL,
    validate_path = "/models",
);

impl DeepSeekAdapter {
    /// 返回 DeepSeek 官方模型列表。
    fn builtin_models(provider_id: &str) -> Vec<Model> {
        vec![
            Model {
                provider_id: provider_id.to_string(),
                model_id: "deepseek-chat".to_string(),
                name: "DeepSeek Chat".to_string(),
                group_name: Some("DeepSeek".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(65536),
                max_output_tokens: Some(8192),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "deepseek-reasoner".to_string(),
                name: "DeepSeek Reasoner".to_string(),
                group_name: Some("DeepSeek".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![
                    ModelCapability::TextChat,
                    ModelCapability::FunctionCalling,
                    ModelCapability::Reasoning,
                ],
                max_tokens: Some(65536),
                max_output_tokens: Some(32768),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
        ]
    }
}
