// SPDX-License-Identifier: AGPL-3.0-only

//! 文心一言（百度千帆 ERNIE）原生适配器。
//!
//! 文心一言通过百度千帆 v2 的 OpenAI 兼容端点
//! (`https://qianfan.baidubce.com/v2/chat/completions`)
//! 提供服务，因此 chat / chat_stream / embed 委托给 [`OpenAIAdapter`]。
//!
//! 文心一言无特殊思考字段。
//!
//! 本适配器重写：
//! - **`list_models`** — 返回文心一言官方模型（ernie 系列）。
//! - **`validate_key`** — 使用文心一言的 base URL 调用 `/models` 端点校验鉴权。

use std::sync::Arc;

use crate::compat::openai_compat_cloud_adapter;
use crate::openai::OpenAIAdapter;
use crate::{ProviderAdapter, ProviderRequestContext};
use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::*;
use futures::Stream;
use std::pin::Pin;

/// 百度千帆 v2 默认 API 端点
const DEFAULT_BASE_URL: &str = "https://qianfan.baidubce.com/v2";

/// 文心一言适配器。
///
/// chat / chat_stream / embed 委托给内部 OpenAI 适配器，
/// 因为千帆 v2 在 `/v2` 前缀下使用 OpenAI 兼容协议。
/// 模型列表与鉴权校验使用文心一言官方端点。
pub struct WenxinAdapter {
    inner: OpenAIAdapter,
}

// ── 委托样板由宏生成（new / Default / base_url / get_client +
//    trait 的 chat / chat_stream / list_models / validate_key / embed）──
//    与其余 4 家云厂商逐字一致，改 trait 签名时不会漏改某一家（见 compat.rs）。
openai_compat_cloud_adapter!(
    WenxinAdapter,
    default_base_url = DEFAULT_BASE_URL,
    validate_path = "/models",
);

impl WenxinAdapter {
    /// 返回文心一言官方模型列表。
    fn builtin_models(provider_id: &str) -> Vec<Model> {
        vec![
            Model {
                provider_id: provider_id.to_string(),
                model_id: "ernie-4.0-8k".to_string(),
                name: "ERNIE 4.0 8K".to_string(),
                group_name: Some("ERNIE".to_string()),
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
                model_id: "ernie-4.0-turbo-8k".to_string(),
                name: "ERNIE 4.0 Turbo 8K".to_string(),
                group_name: Some("ERNIE".to_string()),
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
                model_id: "ernie-3.5-8k".to_string(),
                name: "ERNIE 3.5 8K".to_string(),
                group_name: Some("ERNIE".to_string()),
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
                model_id: "ernie-speed-128k".to_string(),
                name: "ERNIE Speed 128K".to_string(),
                group_name: Some("ERNIE".to_string()),
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
