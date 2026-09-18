// SPDX-License-Identifier: AGPL-3.0-only

//! Kimi（月之暗面 Moonshot）原生适配器。
//!
//! Kimi 提供 OpenAI 兼容的 Chat Completions 端点
//! (`https://api.moonshot.cn/v1/chat/completions`)，
//! 因此 chat / chat_stream / embed 委托给 [`OpenAIAdapter`]。
//!
//! Kimi 无特殊思考字段，但支持超长上下文（最高 128k）。
//!
//! 本适配器重写：
//! - **`list_models`** — 返回 Kimi 官方模型（moonshot-v1 系列）。
//! - **`validate_key`** — 使用 Kimi 的 base URL 调用 `/models` 端点校验鉴权。

use std::sync::Arc;

use crate::compat::openai_compat_cloud_adapter;
use crate::openai::OpenAIAdapter;
use crate::{ProviderAdapter, ProviderRequestContext};
use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::*;
use futures::Stream;
use std::pin::Pin;

/// Kimi Moonshot 默认 API 端点
const DEFAULT_BASE_URL: &str = "https://api.moonshot.cn";

/// Kimi 适配器。
///
/// chat / chat_stream / embed 委托给内部 OpenAI 适配器，
/// 因为 Moonshot 在 `/v1/` 前缀下使用 OpenAI 兼容协议。
/// 模型列表与鉴权校验使用 Kimi 官方端点。
pub struct KimiAdapter {
    inner: OpenAIAdapter,
}

// ── 委托样板由宏生成（new / Default / base_url / get_client +
//    trait 的 chat / chat_stream / list_models / validate_key / embed）──
//    与其余 4 家云厂商逐字一致，改 trait 签名时不会漏改某一家（见 compat.rs）。
openai_compat_cloud_adapter!(
    KimiAdapter,
    default_base_url = DEFAULT_BASE_URL,
    validate_path = "/v1/models",
);

impl KimiAdapter {
    /// 返回 Kimi 官方模型列表。
    fn builtin_models(provider_id: &str) -> Vec<Model> {
        vec![
            Model {
                provider_id: provider_id.to_string(),
                model_id: "moonshot-v1-8k".to_string(),
                name: "Moonshot V1 8K".to_string(),
                group_name: Some("Moonshot".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(8192),
                max_output_tokens: None,
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "moonshot-v1-32k".to_string(),
                name: "Moonshot V1 32K".to_string(),
                group_name: Some("Moonshot".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(32768),
                max_output_tokens: None,
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "moonshot-v1-128k".to_string(),
                name: "Moonshot V1 128K".to_string(),
                group_name: Some("Moonshot".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(131072),
                max_output_tokens: None,
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
        ]
    }
}
