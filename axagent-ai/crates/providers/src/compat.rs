// SPDX-License-Identifier: AGPL-3.0-only

//! OpenAI 兼容「薄包装」适配器的委托样板（`macro_rules`）。
//!
//! # 背景
//!
//! `deepseek` / `glm` / `kimi` / `qwen` / `wenxin` 五家云端厂商，以及
//! `llama_cpp` / `ollama` 两家本地推理后端，做法完全相同：内部持有一个
//! `OpenAIAdapter`，把绝大多数 trait 方法**纯转发**给它。
//! 这 7 个文件里约 340 行代码逐字相同，差异仅三处：
//!
//! 1. 默认 base URL 常量（`DEFAULT_BASE_URL` / `DEFAULT_OLLAMA_HOST`）；
//! 2. `validate_key` 探测的相对路径（`/models` 或 `/v1/models`）；
//! 3. 厂商特有方法（`builtin_models`、llama.cpp 的 `meta` 解析等）。
//!
//! # 为什么必须由宏生成整个 `impl` 块
//!
//! **不能在 `#[async_trait] impl` 内部调用宏来生成 `async fn`。**
//! Rust 的属性宏展开先于同一项内部的 `macro_rules!`：`#[async_trait]`
//! 解析 impl 时看到的是宏调用（`syn::ImplItem::Macro`），会原样透传；
//! 之后 `macro_rules!` 才展开出 `async fn`，而 `async_trait` 已经错过了它，
//! 最终得到「trait 期望 `Pin<Box<dyn Future>>`、实际却是 `async fn`」的编译错误。
//!
//! 因此本模块的宏把 `#[async_trait]` 写进**宏输出**：`openai_compat_*_adapter!`
//! 先在项位置展开，输出一个完整的 `#[async_trait] impl ProviderAdapter for T { .. }`，
//! 属性宏随后才作用到完整的方法列表上，顺序正确。
//!
//! 同理，厂商独有的 trait 方法通过 `extra: { ... }` 参数**原样透传**，
//! 它们仍是宏输出的一部分，`#[async_trait]` 一样能看到并处理。
//!
//! # 使用约束
//!
//! 宏体内使用**裸名**（`ProviderAdapter` / `ProviderRequestContext` / `Result` /
//! `Arc` / `ChatRequest` / `AsyncTrait`…），依赖调用文件已有的 `use`。
//! 调用方必须包含：
//!
//! ```ignore
//! use std::sync::Arc;
//! use crate::openai::OpenAIAdapter;
//! use crate::{ProviderAdapter, ProviderRequestContext};
//! use async_trait::async_trait;
//! use axagent_harness::core_error::{AxAgentError, Result};
//! use axagent_harness::types::*;
//! use futures::Stream;
//! use std::pin::Pin;
//! ```
//!
//! 这样写（而不是全用 `$crate::` 路径）是为了让调用文件的 `use` 保持被使用，
//! 避免 `unused_imports` 告警 —— 本 crate 的 CI 用 `-D warnings` 把它当错误。
//! 只有本 crate 自身的项（`apply_request_headers`）用 `$crate::` 前缀。

/// 生成 `Default`，委托给 `Self::new()`。
///
/// 本项目 15 个适配器 / 注册表都是这个形态（`Self::new()` 完全通用），
/// 用于消除 15 份 5 行的重复。要求 `$t::new()` 存在。
macro_rules! impl_default_via_new {
    ($t:ident) => {
        impl Default for $t {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

/// 云厂商薄包装样板（`deepseek` / `glm` / `kimi` / `qwen` / `wenxin`）。
///
/// 生成 `impl $t { new, base_url, get_client }` + `Default` + 整个
/// `#[async_trait] impl ProviderAdapter`（`chat` / `chat_stream` / `list_models` /
/// `validate_key` / `embed`）。
///
/// - `default_base_url = <常量或字面量>` —— 调用方未配置时回落的默认端点；
/// - `validate_path = "<相对路径>"` —— `validate_key` 探测用的路径；
/// - `extra: { ... }` —— 可选，追加该厂商独有的 trait 方法。
///
/// 生成的 `list_models` 调用 `Self::builtin_models(&ctx.provider_id)`，
/// 因此调用文件必须提供该固有方法。
#[rustfmt::skip]
macro_rules! openai_compat_cloud_adapter {
    (
        $t:ident,
        default_base_url = $base:expr,
        validate_path = $vp:literal
        $(, extra: { $($extra:tt)* })?
        $(,)?
    ) => {
        impl $t {
            /// 构造适配器：内部持有一个 OpenAI 兼容适配器并纯转发。
            pub fn new() -> Self {
                Self { inner: OpenAIAdapter::new() }
            }

            /// 解析有效的 base URL（未配置时回落到厂商默认端点）。
            fn base_url(ctx: &ProviderRequestContext) -> String {
                ctx.base_url.clone().unwrap_or_else(|| $base.to_string())
            }

            /// 构建带代理支持的 HTTP 客户端，委托给内部 OpenAI 适配器。
            #[allow(clippy::result_large_err)]
            fn get_client(&self, ctx: &ProviderRequestContext) -> Result<reqwest::Client> {
                self.inner.get_client(ctx)
            }
        }

        impl Default for $t {
            fn default() -> Self {
                Self::new()
            }
        }

        #[async_trait]
        impl ProviderAdapter for $t {
            /// 委托给 OpenAI 适配器：思考字段已由 `extract_thinking` 解析。
            async fn chat(
                &self,
                ctx: &ProviderRequestContext,
                request: Arc<ChatRequest>,
            ) -> Result<ChatResponse> {
                self.inner.chat(ctx, request).await
            }

            /// 委托给 OpenAI 适配器：SSE 分片格式与 OpenAI 兼容。
            fn chat_stream(
                &self,
                ctx: &ProviderRequestContext,
                request: ChatRequest,
                cancel_token: Option<Arc<std::sync::atomic::AtomicBool>>,
            ) -> Pin<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
                self.inner.chat_stream(ctx, request, cancel_token)
            }

            /// 返回厂商内置模型列表（不调用 API）。
            async fn list_models(&self, ctx: &ProviderRequestContext) -> Result<Vec<Model>> {
                Ok(Self::builtin_models(&ctx.provider_id))
            }

            /// 通过 `<base><validate_path>` 端点校验 API Key 有效性。
            async fn validate_key(&self, ctx: &ProviderRequestContext) -> Result<bool> {
                let url = format!("{}{}", Self::base_url(ctx), $vp);
                let resp = $crate::apply_request_headers(
                    self.get_client(ctx)?
                        .get(&url)
                        .header("Authorization", format!("Bearer {}", ctx.api_key)),
                    ctx,
                )
                .send()
                .await
                .map_err(|e| AxAgentError::Provider(format!("Request failed: {e}")))?;
                let status = resp.status().as_u16();
                Ok(status != 401 && status != 403)
            }

            /// 委托给 OpenAI 适配器。
            async fn embed(
                &self,
                ctx: &ProviderRequestContext,
                request: EmbedRequest,
            ) -> Result<EmbedResponse> {
                self.inner.embed(ctx, request).await
            }

            $($($extra)*)?
        }
    };
}

/// 本地推理后端薄包装样板（`llama_cpp` / `ollama`）。
///
/// 生成 `impl $t { new }` + `Default` + 整个 `#[async_trait] impl`，
/// 其中 `chat` / `chat_stream` / `embed` 为纯转发；`list_models` /
/// `validate_key` 以及其他厂商独有方法由 `extra: { ... }` 原样透传
/// （这两家都改用各自的本地端点，如 `/health`、`/api/tags`，无法复用云样板）。
///
/// 注意：本宏**只**生成 `new`，其余固有方法（`root_url` / `api_url` /
/// `get_client` / `detect_llama_model_type` …）仍写在调用文件的
/// `impl $t { ... }` 块里 —— Rust 允许多个固有 impl 块。
#[rustfmt::skip]
macro_rules! openai_compat_local_adapter {
    (
        $t:ident,
        extra: { $($extra:tt)* }
        $(,)?
    ) => {
        impl $t {
            /// 构造适配器：内部持有一个 OpenAI 兼容适配器并纯转发。
            pub fn new() -> Self {
                Self { inner: OpenAIAdapter::new() }
            }
        }

        impl Default for $t {
            fn default() -> Self {
                Self::new()
            }
        }

        #[async_trait]
        impl ProviderAdapter for $t {
            /// 委托给 OpenAI 适配器。
            async fn chat(
                &self,
                ctx: &ProviderRequestContext,
                request: Arc<ChatRequest>,
            ) -> Result<ChatResponse> {
                self.inner.chat(ctx, request).await
            }

            /// 委托给 OpenAI 适配器：SSE 分片格式与 OpenAI 兼容。
            fn chat_stream(
                &self,
                ctx: &ProviderRequestContext,
                request: ChatRequest,
                cancel_token: Option<Arc<std::sync::atomic::AtomicBool>>,
            ) -> Pin<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
                self.inner.chat_stream(ctx, request, cancel_token)
            }

            /// 委托给 OpenAI 适配器。
            async fn embed(
                &self,
                ctx: &ProviderRequestContext,
                request: EmbedRequest,
            ) -> Result<EmbedResponse> {
                self.inner.embed(ctx, request).await
            }

            $($extra)*
        }
    };
}

pub(crate) use impl_default_via_new;
pub(crate) use openai_compat_cloud_adapter;
pub(crate) use openai_compat_local_adapter;
