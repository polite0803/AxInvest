// SPDX-License-Identifier: AGPL-3.0-only

//! 智谱 GLM 原生适配器。
//!
//! 智谱 GLM 通过 BigModel 开放平台
//! (`https://open.bigmodel.cn/api/paas/v4/chat/completions`)
//! 提供 OpenAI 兼容的 Chat Completions 端点，
//! 因此 chat / chat_stream / embed 委托给 [`OpenAIAdapter`]。
//!
//! 特有能力：glm-4 系列模型在响应中返回 `thinking` 字段，表示思考过程。
//! 该字段已被 [`OpenAIAdapter`] 的 `extract_thinking` 函数解析并映射到
//! harness 的 `thinking` 字段，无需在此重复实现。
//!
//! 本适配器重写：
//! - **`list_models`** — 返回智谱 GLM 官方模型。
//! - **`validate_key`** — 使用智谱 GLM 的 base URL 调用 `/models` 端点校验鉴权。

use std::sync::Arc;

use crate::compat::openai_compat_cloud_adapter;
use crate::openai::OpenAIAdapter;
use crate::{ProviderAdapter, ProviderRequestContext};
use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::*;
use futures::Stream;
use std::pin::Pin;

/// 智谱 GLM BigModel 开放平台默认端点
const DEFAULT_BASE_URL: &str = "https://open.bigmodel.cn/api/paas/v4";

/// 智谱 GLM 适配器。
///
/// chat / chat_stream / embed 委托给内部 OpenAI 适配器，
/// 因为 BigModel 在 `/api/paas/v4` 前缀下使用 OpenAI 兼容协议。
/// 模型列表与鉴权校验使用智谱 GLM 官方端点。
pub struct GlmAdapter {
    inner: OpenAIAdapter,
}

// ── 委托样板由宏生成（new / Default / base_url / get_client +
//    trait 的 chat / chat_stream / list_models / validate_key / embed）──
//    与其余 4 家云厂商逐字一致，改 trait 签名时不会漏改某一家（见 compat.rs）。
openai_compat_cloud_adapter!(
    GlmAdapter,
    default_base_url = DEFAULT_BASE_URL,
    validate_path = "/models",
);

impl GlmAdapter {
    /// 返回智谱 GLM 官方模型列表。
    fn builtin_models(provider_id: &str) -> Vec<Model> {
        vec![
            Model {
                provider_id: provider_id.to_string(),
                model_id: "glm-4".to_string(),
                name: "GLM-4".to_string(),
                group_name: Some("GLM".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![
                    ModelCapability::TextChat,
                    ModelCapability::FunctionCalling,
                    ModelCapability::Vision,
                ],
                max_tokens: Some(131072),
                max_output_tokens: Some(4096),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "glm-4-plus".to_string(),
                name: "GLM-4 Plus".to_string(),
                group_name: Some("GLM".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![
                    ModelCapability::TextChat,
                    ModelCapability::FunctionCalling,
                    ModelCapability::Vision,
                ],
                max_tokens: Some(131072),
                max_output_tokens: Some(4096),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "glm-4-air".to_string(),
                name: "GLM-4 Air".to_string(),
                group_name: Some("GLM".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(131072),
                max_output_tokens: Some(4096),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
            Model {
                provider_id: provider_id.to_string(),
                model_id: "glm-4-flash".to_string(),
                name: "GLM-4 Flash".to_string(),
                group_name: Some("GLM".to_string()),
                model_type: ModelType::Chat,
                capabilities: vec![ModelCapability::TextChat, ModelCapability::FunctionCalling],
                max_tokens: Some(131072),
                max_output_tokens: Some(4096),
                enabled: true,
                param_overrides: None,
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            },
        ]
    }
}
