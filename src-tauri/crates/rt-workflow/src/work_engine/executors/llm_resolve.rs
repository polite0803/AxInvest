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
            // ⚠ 码必须是 `API_KEY_DECRYPT_FAILED` 而不是 `UNSUPPORTED_PROVIDER`（2026-10-28）。
            //   同一条故障在缓存路径 `agent_executor.rs` 里早就用的是这个码；这里跟着
            //   `resolve_model_for_node` 的失败一起套 UNSUPPORTED_PROVIDER，于是界面把
            //   「密钥解不开」显示成「不支持的数据供应商」，把用户指向供应商配置而不是凭证。
            //   代价（有意的）：本码在 `is_non_retryable_error` 清单内 ⇒ 该节点**不再重试**。
            //   这是正确的：密文与 master key 都不随重试改变，重试必然复现同一失败
            //   （实测一次回放批次为此白跑 235 次尝试 / 55 个分析师节点）。
            NodeError::exec_failed(
                error_code::API_KEY_DECRYPT_FAILED,
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

#[cfg(test)]
mod tests {
    /// 解密站点上**先出现**的是哪个码。
    ///
    /// 判据用「谁在前」而不是「固定窗口里有没有」：本站下面就是 adapter 缺失那一站，它**本来
    /// 就该**保留 `UNSUPPORTED_PROVIDER` —— 按字符窗口取段要么切在汉字中间、要么把下一站一起
    /// 吞进来（第一版即因此假红：600 字符正好截在 `error_code::API_KEY_DECRY` 上）。
    fn decrypt_site_code(src: &str) -> Option<&'static str> {
        let at = src.find("decrypt_key(&key.key_encrypted, master_key)")?;
        let rest = &src[at..];
        let decrypt = rest.find("error_code::API_KEY_DECRYPT_FAILED");
        let unsupported = rest.find("error_code::UNSUPPORTED_PROVIDER");
        match (decrypt, unsupported) {
            (Some(i), Some(j)) if j < i => Some("UNSUPPORTED_PROVIDER"),
            (Some(_), _) => Some("API_KEY_DECRYPT_FAILED"),
            (None, _) => None,
        }
    }

    /// 解密失败的错误码必须留在 `API_KEY_DECRYPT_FAILED`（2026-10-28 归因收口）。
    ///
    /// 为什么钉源码文本而不是走行为测试：这条链要真库 + provider 注册表 + 一行解不开的密文
    /// 才能走到，而在 `resolve_provider_and_adapter` 里为测试造一张坏密钥表 = 引入生产里
    /// 不存在的形态。本断言钉的是**那次选择**：谁把它改回 `UNSUPPORTED_PROVIDER`，这里立刻红。
    #[test]
    fn decrypt_failure_keeps_the_api_key_decrypt_code() {
        let src = include_str!("llm_resolve.rs");
        assert_eq!(
            decrypt_site_code(src),
            Some("API_KEY_DECRYPT_FAILED"),
            "解密失败必须报 API_KEY_DECRYPT_FAILED：界面标签才指得到凭证，且与缓存路径同码"
        );
        // 负控：把本站的码改回旧值 ⇒ 判据必须翻成 UNSUPPORTED_PROVIDER（否则本用例恒真）
        let broken =
            src.replace("error_code::API_KEY_DECRYPT_FAILED,", "error_code::UNSUPPORTED_PROVIDER,");
        assert_ne!(broken, src, "负控锚点腐烂：替换没生效，本用例等于没通电");
        assert_eq!(decrypt_site_code(&broken), Some("UNSUPPORTED_PROVIDER"));
    }

    /// 改码的**后果**也必须钉住：本码在不可重试清单内 ⇒ 该节点一次即定，不再空跑六次。
    /// 清单若被改动（把本码移出），这条会红，逼人来同步上面的取舍注释。
    #[test]
    fn api_key_decrypt_failure_stays_non_retryable() {
        let msg = "API_KEY_DECRYPT_FAILED: API key decryption failed: Crypto error: \
                   Decryption failed: aead::Error";
        assert!(
            crate::work_engine::node_executor_trait::is_non_retryable_error(msg),
            "密钥解密失败是确定性失败：master key 与密文都不随重试改变，重试只会白跑"
        );
        // 对照（同族但性质不同）：供应商解析失败仍可重试，不得被顺手移进来
        let other = "UNSUPPORTED_PROVIDER: 未找到 ProviderAdapter for type: openai";
        assert!(
            !crate::work_engine::node_executor_trait::is_non_retryable_error(other),
            "UNSUPPORTED_PROVIDER 不在清单内是既有语义，本批没改它"
        );
    }
}
