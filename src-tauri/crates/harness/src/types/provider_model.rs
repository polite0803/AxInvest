// SPDX-License-Identifier: AGPL-3.0-only

use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use super::settings_chat::AppSettings;
use crate::constants;

/// Deserialize `Option<Option<T>>` so that a JSON `null` becomes `Some(None)`
/// while a missing field (via `#[serde(default)]`) stays `None`.
pub(crate) fn deserialize_double_option<'de, T, D>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

// === Provider System ===

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub provider_type: ProviderType,
    pub api_host: String,
    pub api_path: Option<String>,
    pub enabled: bool,
    pub models: Vec<Model>,
    pub keys: Vec<ProviderKey>,
    pub proxy_config: Option<ProviderProxyConfig>,
    /// 工具调用模式：None=按 provider_type 推断；"native"=模型原生 function calling；
    /// "managed"=由 AxAgent 通过提示词注入 + 文本解析模拟（用于 Chat2API 等无原生 tool 接口的网关）
    pub tool_adaptation: Option<String>,
    /// 托管模式下的 marker 前缀（仅 tool_adaptation="managed" 时生效）。
    /// None 或空字符串 = 使用默认值 "CHAT2API"。
    pub tool_adaptation_marker_prefix: Option<String>,
    pub custom_headers: Option<String>,
    pub icon: Option<String>,
    pub builtin_id: Option<String>,
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    OpenAI,
    #[serde(rename = "openai_responses")]
    OpenAIResponses,
    Anthropic,
    Gemini,
    OpenClaw,
    Hermes,
    Ollama,
    #[serde(rename = "llama_cpp")]
    LlamaCpp,
    /// TypeSafe Jev 决策模型（decisions 端点，非 chat 兼容协议）。
    TypeSafe,
}

impl ProviderType {
    // Business methods extracted to free functions below.
}

/// ProviderType → ProviderRegistry 内部 key 的唯一权威映射。
///
/// 注册表 key 由 `axagent_providers::registry::ProviderRegistry::create_default`
/// 决定；新增 ProviderType 变体时必须同步在此实现，否则编译期立刻暴露
/// (match 必须穷尽所有变体)。所有调用方统一用 `provider_registry_key(pt)`，
/// 不允许再定义私有副本。
pub fn provider_registry_key(pt: &ProviderType) -> &'static str {
    match pt {
        ProviderType::OpenAI => "openai",
        ProviderType::OpenAIResponses => "openai_responses",
        ProviderType::Anthropic => "anthropic",
        ProviderType::Gemini => "gemini",
        ProviderType::OpenClaw => "openclaw",
        ProviderType::Hermes => "hermes",
        ProviderType::Ollama => "ollama",
        ProviderType::LlamaCpp => "llama_cpp",
        ProviderType::TypeSafe => "typesafe",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKey {
    pub id: String,
    pub provider_id: String,
    pub key_encrypted: String,
    pub key_prefix: String,
    pub enabled: bool,
    pub last_validated_at: Option<i64>,
    pub last_error: Option<String>,
    pub rotation_index: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProxyConfig {
    pub proxy_type: Option<String>,
    pub proxy_address: Option<String>,
    pub proxy_port: Option<u16>,
}

impl ProviderProxyConfig {
    // Business methods extracted to free functions below.
}

/// Resolve effective proxy: provider-level overrides global.
/// If provider has explicit proxy_type, use it (even "none" to disable).
/// Otherwise fall back to global settings.
pub fn resolve_provider_proxy(
    provider: &Option<ProviderProxyConfig>,
    global_settings: &AppSettings,
) -> Option<ProviderProxyConfig> {
    if let Some(config) = provider
        && config.proxy_type.is_some()
    {
        if config.proxy_type.as_deref() == Some("none") {
            return None;
        }
        return Some(config.clone());
    }
    // Fall back to global proxy
    match global_settings.proxy_type.as_deref() {
        Some("none") | None => None,
        Some("system") => Some(ProviderProxyConfig {
            proxy_type: Some(constants::role::SYSTEM.to_string()),
            proxy_address: None,
            proxy_port: None,
        }),
        _ => Some(ProviderProxyConfig {
            proxy_type: global_settings.proxy_type.clone(),
            proxy_address: global_settings.proxy_address.clone(),
            proxy_port: global_settings.proxy_port,
        }),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProviderInput {
    pub name: String,
    pub provider_type: ProviderType,
    pub api_host: String,
    pub api_path: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub builtin_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProviderInput {
    pub name: Option<String>,
    pub provider_type: Option<ProviderType>,
    pub api_host: Option<String>,
    pub api_path: Option<Option<String>>,
    pub enabled: Option<bool>,
    pub proxy_config: Option<ProviderProxyConfig>,
    #[serde(default)]
    pub tool_adaptation: Option<Option<String>>,
    #[serde(default)]
    pub tool_adaptation_marker_prefix: Option<Option<String>>,
    pub custom_headers: Option<Option<String>>,
    pub icon: Option<Option<String>>,
    pub sort_order: Option<i32>,
}

// === Model System ===

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub provider_id: String,
    pub model_id: String,
    pub name: String,
    pub group_name: Option<String>,
    pub model_type: ModelType,
    pub capabilities: Vec<ModelCapability>,
    pub max_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub enabled: bool,
    pub param_overrides: Option<ModelParamOverrides>,
    /// Input price per million tokens (USD). When set, used for accurate cost calculation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_price_per_mtok: Option<f64>,
    /// Output price per million tokens (USD). When set, used for accurate cost calculation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_price_per_mtok: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, TS)]
pub enum ModelType {
    #[default]
    Chat,
    Voice,
    Embedding,
    /// 决策模型（如 TypeSafe Jev）：只接收 state + 类型化问题，返回带概率的结构化判定，
    /// **不做文本生成**。故不能被 LLMNode / Agent 这类生成节点选中 —— 见
    /// `rt-workflow` 的 `llm_resolve.rs` 里的节点类型硬校验。
    Decision,
}

impl ModelType {
    // Business methods extracted to free functions below.
}

/// Auto-detect model type from model_id string.
///
/// 使用更精确的匹配策略避免误判：
/// - Embedding：text-embedding-*、embedding-* 前缀，以及 bge 家族（bge-m3 等本地 GGUF）
/// - Voice：tts-*, whisper-*, realtime 等明确语音模型标识
/// - Decision：TypeSafe Jev 家族（`typesafe/jev-*`）
/// - 其余为 Chat 类型
pub fn detect_model_type(model_id: &str) -> ModelType {
    let id = model_id.to_lowercase();
    if id.contains("text-embedding")
        || id.starts_with("embedding")
        || id.contains("-embedding")
        || id.contains("bge-")
        || id.starts_with("bge")
    {
        ModelType::Embedding
    } else if id.contains("tts-") || id.contains("whisper-") || id.contains("realtime") {
        ModelType::Voice
    } else if id.contains("jev-") {
        // 决策模型即便被手工挂到某个 chat provider 下，也应保留 Decision 类型，
        // 否则会被 LLMNode / Agent 误选为生成模型。
        ModelType::Decision
    } else {
        ModelType::Chat
    }
}

/// 该模型类型是否**禁止用于生成路径**（对话 / LLMNode / Agent）。
///
/// 决策模型（`Decision`，如 TypeSafe Jev）只接收 state + 类型化问题、返回带概率的
/// 结构化判定，**不做文本生成** —— 被塞进生成路径只会产出一段无法使用的“文本”。
/// 它的正确去处是工作流的 `llmClassifier` / `condition`（LLM 动态路由）节点。
///
/// 本判据是全仓唯一权威定义：`rt-workflow` 的 `llm_resolve.rs` 与聊天发送链路
/// （`commands/conversations/`）都复用它，不要各自重写 `== ModelType::Decision`。
///
/// 注意：`Voice` / `Embedding` 各有自己的合法通道（实时语音 / 向量检索），
/// 不属于本判据范围。
pub fn is_generation_blocked(model_type: &ModelType) -> bool {
    matches!(model_type, ModelType::Decision)
}

/// 解析某 provider 下某模型的 `ModelType`。
///
/// 优先取 provider 登记的类型；模型未登记（手工填入 model id、模板导入等路径）
/// 时回落到按命名推断（`detect_model_type`），避免漏判。
pub fn resolve_model_type(prov: &ProviderConfig, model_id: &str) -> ModelType {
    prov.models
        .iter()
        .find(|m| m.model_id == model_id)
        .map(|m| m.model_type.clone())
        .unwrap_or_else(|| detect_model_type(model_id))
}

impl std::fmt::Display for ModelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelType::Chat => write!(f, "chat"),
            ModelType::Voice => write!(f, "voice"),
            ModelType::Embedding => write!(f, "embedding"),
            ModelType::Decision => write!(f, "decision"),
        }
    }
}

impl std::str::FromStr for ModelType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "chat" => Ok(ModelType::Chat),
            "voice" => Ok(ModelType::Voice),
            "embedding" => Ok(ModelType::Embedding),
            "decision" => Ok(ModelType::Decision),
            _ => Ok(ModelType::Chat),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub enum ModelCapability {
    TextChat,
    Vision,
    FunctionCalling,
    Reasoning,
    RealtimeVoice,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModelParamOverrides {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
    pub frequency_penalty: Option<f32>,
    /// When true, the provider adapter should send `max_completion_tokens`
    /// instead of `max_tokens` (required by OpenAI o-series models).
    pub use_max_completion_tokens: Option<bool>,
    /// When true, system messages are converted to user messages
    /// (for models that don't support the system role).
    pub no_system_role: Option<bool>,
    /// When true, always include max_tokens in the request
    /// (falls back to 4096 if conversation.max_tokens is not set).
    pub force_max_tokens: Option<bool>,
    /// Thinking parameter format for the provider API.
    /// "reasoning_effort" (default/OpenAI) or "enable_thinking" (SiliconFlow).
    pub thinking_param_style: Option<String>,
    /// Delay in milliseconds before each API request to this model.
    /// Used to avoid hitting rate limits (e.g. 429 errors) on providers
    /// with strict per-model rate quotas.
    pub request_delay_ms: Option<u64>,
}

// === Conversation & Message ===
