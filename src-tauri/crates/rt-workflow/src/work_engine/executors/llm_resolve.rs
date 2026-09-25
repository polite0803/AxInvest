// SPDX-License-Identifier: AGPL-3.0-only

//! LLM provider + adapter 公共解析助手。
//!
//! 集中 4 个 executor 重复的 `resolve_model_for_node → decrypt_key → registry.get` 三步。

use axagent_harness::types::{ProviderConfig, ProviderKey};
use axagent_harness::{ProviderAdapter, registry::ProviderRegistry};
use std::sync::Arc;

use crate::work_engine::node_executor_trait::{NodeError, error_code};

/// 解析 provider + key + model + adapter + api_key。
///
/// 调用方传 node_model / session_model / session_provider_id / profile_suggested，
/// helper 内部完成：
/// 1. `axagent_harness::repositories::ProviderRepository::resolve_model_for_node` 拿到 (prov, key, model)
/// 2. `axagent_crypto::crypto::decrypt_key` 解密 api key
/// 3. `provider_registry.get(prov.provider_type.registry_key())` 拿 adapter
///
/// 返回值 `(prov, key, model, adapter, api_key)` 供调用方继续构建 request。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_provider_and_adapter(
    master_key: &[u8; 32],
    provider_registry: Option<&Arc<dyn ProviderRegistry>>,
    node_model: Option<&str>,
    session_model: Option<&str>,
    session_provider_id: Option<&str>,
    profile_suggested_provider: Option<&str>,
    executor_label: &str,
) -> Result<(ProviderConfig, ProviderKey, String, Arc<dyn ProviderAdapter>, String), NodeError> {
    let (prov, key, model) = axagent_harness::repositories::provider_repository()
        .resolve_model_for_node(
            node_model,
            session_model,
            session_provider_id,
            profile_suggested_provider,
        )
        .await
        .map_err(|e| NodeError::exec_failed(error_code::UNSUPPORTED_PROVIDER, e))?;

    tracing::info!(
        target: "axagent.llm_resolve",
        executor = %executor_label,
        provider_id = %prov.id,
        provider_name = %prov.name,
        provider_type = ?prov.provider_type,
        api_host = %prov.api_host,
        model = %model,
        key_id = %key.id,
        "[LLM-RESOLVE] 选中 provider"
    );

    let api_key =
        axagent_crypto::crypto::decrypt_key(&key.key_encrypted, master_key).map_err(|e| {
            NodeError::exec_failed(
                error_code::UNSUPPORTED_PROVIDER,
                format!("API key decryption failed: {e}"),
            )
        })?;

    let registry_key =
        axagent_harness::types::provider_model::provider_registry_key(&prov.provider_type);
    let adapter: Arc<dyn ProviderAdapter> =
        provider_registry.and_then(|reg| reg.get(registry_key)).ok_or_else(|| {
            NodeError::exec_failed(
                error_code::UNSUPPORTED_PROVIDER,
                format!("{executor_label} 未找到 ProviderAdapter for type: {registry_key}"),
            )
        })?;

    Ok((prov, key, model, adapter, api_key))
}

/// 拒绝把决策模型（`ModelType::Decision`，如 TypeSafe Jev）配到生成类节点上。
///
/// 决策模型只返回结构化判定、不做文本生成，UI 的类型过滤（`ModelSelect` 的
/// `modelTypes`）挡不住手写工作流 JSON / 模板导入等路径，因此在调用 provider
/// 之前就在这里快速失败，避免产出一段无法使用的“文本”。
///
/// 仅 `llm` / `agent` 这类生成节点需要调用；`llmClassifier` / `condition` /
/// `switch` 的判断路径本就期望结构化判定，不应调用。
///
/// 判据（`resolve_model_type` / `is_generation_blocked`）定义在 harness 的类型权威层，
/// 聊天发送链路共用同一份，不要在此重写。
pub(crate) fn ensure_generation_model(
    prov: &ProviderConfig,
    model: &str,
    executor_label: &str,
) -> Result<(), NodeError> {
    let model_type = axagent_harness::types::provider_model::resolve_model_type(prov, model);

    if axagent_harness::types::provider_model::is_generation_blocked(&model_type) {
        return Err(NodeError::exec_failed(
            error_code::UNSUPPORTED_PROVIDER,
            format!(
                "{executor_label} 不支持决策模型（{model}）：决策模型只输出结构化判定，不做文本生成。\
                 请改配 chat 模型，或把该判断改由 llmClassifier / condition 节点完成。"
            ),
        ));
    }
    Ok(())
}
