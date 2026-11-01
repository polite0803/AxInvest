// SPDX-License-Identifier: AGPL-3.0-only

//! `build_provider_request_context` — 公共 ProviderRequestContext 构造助手。
//!
//! 之前在多处（`llm_resolve.rs::resolve_provider_and_adapter`、
//! `platform_bridge.rs::call_llm`、以及若干 `commands/*.rs` 内的 4 处 3 步链）字节级
//! 重复构造 `ProviderRequestContext`（12 字段）。现统一收敛到本函数。
//!
//! 设计：本函数只关心"从已解密的 api_key + provider/key 配置 → ctx"这一纯数据变换，
//! 不涉及 `sea-orm` / `axagent-providers` 依赖，可放在 axagent-harness
//! （axagent-core 的 trait 抽象层），供所有 crate 复用。
//!
//! 注：URL 解析直接调用同 crate 的 `url_utils::resolve_base_url_for_type`，
//! 避免 harness → providers 反向依赖。

use crate::types::{ProviderConfig, ProviderKey};

use crate::provider::ProviderRequestContext;
use crate::types::model_param_overrides;
use crate::url_utils::resolve_base_url_for_type;

/// 从 `ProviderConfig + ProviderKey + 已解密的 api_key` 构建 `ProviderRequestContext`。
///
/// 字段映射规则（与各调用方原逻辑保持一致）：
/// - `api_key` = 传入的明文 api_key
/// - `key_id` = `key.id`
/// - `provider_id` = `prov.id`
/// - `base_url` = `resolve_base_url_for_type(prov.api_host, prov.provider_type)`
/// - `api_path` = `prov.api_path.clone()`
/// - `proxy_config` = `prov.proxy_config.clone()`
/// - `custom_headers` = `prov.custom_headers` 解析 JSON 后的 Option
/// - 其余 5 个字段（api_mode/conversation/previous_response_id/store_response）= None
pub fn build_provider_request_context(
    prov: &ProviderConfig,
    key: &ProviderKey,
    api_key: String,
) -> ProviderRequestContext {
    build_provider_request_context_for_model(prov, key, api_key, None)
}

/// 同 [`build_provider_request_context`]，但额外解析**模型级**首字节预算。
///
/// `model_id` 指向设置页「供应商-模型 → 首字节超时」那一格
/// （`ModelParamOverrides::header_timeout_secs`）；模型未登记 override 或未填该格
/// ⇒ `header_timeout_secs = None` ⇒ provider 适配器用默认预算（软 45s / 硬 90s）。
///
/// 调用方必须传**本次实际使用的 model_id**：同一 provider 下不同模型的排队时延差异很大，
/// 预算按模型粒度才准。
pub fn build_provider_request_context_for_model(
    prov: &ProviderConfig,
    key: &ProviderKey,
    api_key: String,
    model_id: Option<&str>,
) -> ProviderRequestContext {
    ProviderRequestContext {
        api_key,
        key_id: key.id.clone(),
        provider_id: prov.id.clone(),
        base_url: Some(resolve_base_url_for_type(&prov.api_host, &prov.provider_type)),
        api_path: prov.api_path.clone(),
        proxy_config: prov.proxy_config.clone(),
        custom_headers: prov.custom_headers.as_ref().and_then(|s| serde_json::from_str(s).ok()),
        api_mode: None,
        conversation: None,
        previous_response_id: None,
        store_response: None,
        header_timeout_secs: model_id
            .and_then(|m| model_param_overrides(prov, m))
            .and_then(|o| o.header_timeout_secs),
    }
}

#[cfg(test)]
mod context_builder_tests {
    use super::*;
    use crate::types::{Model, ModelParamOverrides, ModelType, ProviderKey, ProviderType};

    fn prov_with_model(model_id: &str, header_timeout_secs: Option<u64>) -> ProviderConfig {
        ProviderConfig {
            id: "p1".to_string(),
            name: "P1".to_string(),
            provider_type: ProviderType::OpenAI,
            api_host: "https://example.invalid/v1".to_string(),
            api_path: None,
            enabled: true,
            models: vec![Model {
                provider_id: "p1".to_string(),
                model_id: model_id.to_string(),
                name: model_id.to_string(),
                group_name: None,
                model_type: ModelType::Chat,
                capabilities: vec![],
                max_tokens: None,
                enabled: true,
                param_overrides: Some(ModelParamOverrides {
                    header_timeout_secs,
                    ..Default::default()
                }),
                input_price_per_mtok: None,
                output_price_per_mtok: None,
            }],
            keys: vec![],
            proxy_config: None,
            tool_adaptation: None,
            tool_adaptation_marker_prefix: None,
            custom_headers: None,
            icon: None,
            builtin_id: None,
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn key() -> ProviderKey {
        ProviderKey {
            id: "k1".to_string(),
            provider_id: "p1".to_string(),
            key_encrypted: "enc".to_string(),
            key_prefix: "sk-".to_string(),
            enabled: true,
            last_validated_at: None,
            last_error: None,
            rotation_index: 0,
            created_at: 0,
        }
    }

    /// 正控：模型登记了首字节预算 ⇒ ctx 带上该值。
    #[test]
    fn picks_up_the_model_level_header_timeout() {
        let prov = prov_with_model("fast", Some(120));
        let ctx =
            build_provider_request_context_for_model(&prov, &key(), "sk-x".into(), Some("fast"));
        assert_eq!(ctx.header_timeout_secs, Some(120));
    }

    /// 负控一：模型没登记那一格 ⇒ None（适配器用默认预算），不得凭空造值。
    #[test]
    fn no_override_means_no_override() {
        let prov = prov_with_model("fast", None);
        let ctx =
            build_provider_request_context_for_model(&prov, &key(), "sk-x".into(), Some("fast"));
        assert_eq!(ctx.header_timeout_secs, None);
    }

    /// 负控二：取值必须按 **model_id** 定位 —— 传别的模型名不得命中这一格。
    #[test]
    fn another_model_id_does_not_borrow_the_value() {
        let prov = prov_with_model("fast", Some(120));
        let ctx =
            build_provider_request_context_for_model(&prov, &key(), "sk-x".into(), Some("slow"));
        assert_eq!(ctx.header_timeout_secs, None);
    }

    /// 负控三：不传 model_id（老调用方）⇒ None，不回退到别的模型的格。
    #[test]
    fn without_model_id_keeps_the_default_budget_path() {
        let prov = prov_with_model("fast", Some(120));
        let ctx = build_provider_request_context(&prov, &key(), "sk-x".to_string());
        assert_eq!(ctx.header_timeout_secs, None);
    }
}
