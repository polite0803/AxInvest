// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;
use std::sync::Arc;

use crate::ProviderAdapter;
use crate::anthropic::AnthropicAdapter;
use crate::compat::impl_default_via_new;
use crate::deepseek::DeepSeekAdapter;
use crate::gemini::GeminiAdapter;
use crate::glm::GlmAdapter;
use crate::hermes::HermesAdapter;
use crate::kimi::KimiAdapter;
use crate::llama_cpp::LlamaCppAdapter;
use crate::ollama::OllamaAdapter;
use crate::openai::OpenAIAdapter;
use crate::openai_responses::OpenAIResponsesAdapter;
use crate::openclaw::OpenClawAdapter;
use crate::qwen::QwenAdapter;
use crate::typesafe::TypeSafeAdapter;
use crate::wenxin::WenxinAdapter;

pub struct ProviderRegistry {
    adapters: HashMap<String, Arc<dyn ProviderAdapter>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self { adapters: HashMap::new() }
    }

    /// Register a provider adapter (takes `Arc` to support the harness `ProviderRegistry` trait)
    pub fn register(&mut self, provider_type: &str, adapter: Arc<dyn ProviderAdapter>) {
        if self.adapters.contains_key(provider_type) {
            tracing::warn!(
                provider_type,
                "Provider adapter already registered; overwriting existing entry"
            );
        }
        self.adapters.insert(provider_type.to_string(), adapter);
    }

    /// Get a registered adapter by provider type name
    pub fn get(&self, provider_type: &str) -> Option<&Arc<dyn ProviderAdapter>> {
        self.adapters.get(provider_type)
    }

    /// Creates a registry pre-populated with built-in provider adapters.
    ///
    /// 内置适配器不再在此硬编码构造函数清单：它们经 `model.provider.{name}`
    /// 接缝注册进能力注册表，再由本函数从接缝构建（外部插件注册的适配器
    /// 同样出现在返回的注册表中 —— 内置与插件平权）。
    ///
    /// 纯构造：除补注册内置接缝（幂等）外不产生其他全局副作用。
    pub fn create_default() -> Self {
        register_builtin_providers();
        Self::from_capability_registry()
    }

    /// 从能力注册表构建适配器表（`model.provider.*` 接缝为**唯一来源**）。
    pub fn from_capability_registry() -> Self {
        let capability_registry = axagent_harness::get_capability_registry();
        let mut registry = Self::new();
        for provider_type in capability_registry.list_model_providers() {
            if let Some(adapter) = capability_registry.get_model_provider(&provider_type) {
                registry.adapters.insert(provider_type, adapter);
            }
        }
        registry
    }
}

/// 内置 Provider 适配器清单 —— **全仓唯一的硬编码点**。
///
/// [`register_builtin_providers`] 与单测共用本清单；新增内置适配器只需在此追加一行。
fn builtin_adapters() -> Vec<(&'static str, Arc<dyn ProviderAdapter>)> {
    vec![
        ("openai", Arc::new(OpenAIAdapter::new())),
        ("openai_responses", Arc::new(OpenAIResponsesAdapter::new())),
        ("anthropic", Arc::new(AnthropicAdapter::new())),
        ("gemini", Arc::new(GeminiAdapter::new())),
        ("openclaw", Arc::new(OpenClawAdapter::new())),
        ("hermes", Arc::new(HermesAdapter::new())),
        ("ollama", Arc::new(OllamaAdapter::new())),
        ("llama_cpp", Arc::new(LlamaCppAdapter::new())),
        // 国内 LLM 厂商原生适配器
        ("deepseek", Arc::new(DeepSeekAdapter::new())),
        ("qwen", Arc::new(QwenAdapter::new())),
        ("glm", Arc::new(GlmAdapter::new())),
        ("kimi", Arc::new(KimiAdapter::new())),
        ("wenxin", Arc::new(WenxinAdapter::new())),
        // 决策模型：TypeSafe Jev（decisions 端点，非 chat 兼容）
        ("typesafe", Arc::new(TypeSafeAdapter::new())),
    ]
}

/// 把内置适配器注册进能力注册表（`model.provider.{name}` 接缝）。
///
/// **幂等**：已注册的类型名跳过（注册表对重复 ID 返回 `Duplicate`）。
/// 启动装配时调用一次；[`ProviderRegistry::create_default`] 亦会调用，
/// 使未走启动装配的路径（如单测）同样拿得到内置适配器。
pub fn register_builtin_providers() {
    let capability_registry = axagent_harness::get_capability_registry();
    for (provider_type, adapter) in builtin_adapters() {
        if capability_registry.contains(&format!("model.provider.{provider_type}")) {
            continue;
        }
        if let Err(e) = capability_registry.register_model_provider(provider_type, adapter) {
            tracing::warn!(
                provider_type,
                error = %e,
                "内置 Provider 适配器注册进能力注册表失败"
            );
        }
    }
}

impl_default_via_new!(ProviderRegistry);

// ============================================================
// Harness ProviderRegistry trait 实现
// ============================================================

impl axagent_harness::registry::ProviderRegistry for ProviderRegistry {
    fn get(&self, provider_type: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters.get(provider_type).cloned()
    }
}
