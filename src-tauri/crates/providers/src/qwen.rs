// SPDX-License-Identifier: AGPL-3.0-only

//! 通义千问（Qwen）原生适配器。
//!
//! 通义千问通过阿里云 DashScope 的 OpenAI 兼容模式
//! (`https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions`)
//! 提供服务，因此 chat / chat_stream / embed 委托给 [`OpenAIAdapter`]。
//!
//! 特有能力：`qwen-thinking` 模型在响应中返回 `thinking` 字段，表示思考过程。
//! 该字段已被 [`OpenAIAdapter`] 的 `extract_thinking` 函数解析并映射到
//! harness 的 `thinking` 字段，无需在此重复实现。
//!
//! 本适配器重写：
//! - **`list_models`** — 返回通义千问官方模型。
//! - **`validate_key`** — 使用通义千问的 base URL 调用 `/models` 端点校验鉴权。

use std::sync::Arc;

use crate::compat::openai_compat_cloud_adapter;
use crate::openai::OpenAIAdapter;
use crate::{ProviderAdapter, ProviderRequestContext};
use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::*;
use futures::Stream;
use std::pin::Pin;

/// 通义千问 DashScope OpenAI 兼容模式默认端点
const DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode";

/// 通义千问适配器。
///
/// chat / chat_stream / embed 委托给内部 OpenAI 适配器，
/// 因为 DashScope 在 `/v1/` 前缀下使用 OpenAI 兼容协议。
/// 模型列表与鉴权校验使用通义千问官方端点。
pub struct QwenAdapter {
    inner: OpenAIAdapter,
}

// ── 委托样板由宏生成（new / Default / base_url / get_client +
//    trait 的 chat / chat_stream / list_models / validate_key / embed）──
//    与其余 4 家云厂商逐字一致，改 trait 签名时不会漏改某一家（见 compat.rs）。
openai_compat_cloud_adapter!(
    QwenAdapter,
    default_base_url = DEFAULT_BASE_URL,
    validate_path = "/v1/models",
);

impl QwenAdapter {
    /// 返回通义千问官方模型列表。
    fn builtin_models(provider_id: &str) -> Vec<Model> {
        vec![
            Model {
                provider_id: provider_id.to_string(),
                model_id: "qwen-max".to_string(),
                name: "Qwen Max".to_string(),
                group_name: Some("Qwen".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(32768),
                max_output_tokens: Some(8192),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "qwen-plus".to_string(),
                name: "Qwen Plus".to_string(),
                group_name: Some("Qwen".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(131072),
                max_output_tokens: Some(8192),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "qwen-turbo".to_string(),
                name: "Qwen Turbo".to_string(),
                group_name: Some("Qwen".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(1000000),
                max_output_tokens: Some(8192),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "qwen-thinking".to_string(),
                name: "Qwen Thinking".to_string(),
                group_name: Some("Qwen".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![
                    ModelCapability::TextChat,
                    ModelCapability::FunctionCalling,
                    ModelCapability::Reasoning,
                ],
                max_tokens: Some(38400),
                max_output_tokens: Some(16384),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
        ]
    }
}
