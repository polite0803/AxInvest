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

    // ── 多密钥供应商的 key 级 failover（2026-10-28）──────────────────────────
    //
    // 为什么必须逐把试而不是"一行定死"：一家供应商常挂多把 key（实测 Agnes 4 行，其中一行
    //   `last_error='Validation failed'` 且从未验证通过），而解密失败是**行级**故障 —— 撞上
    //   一行坏密文时同供应商其余行完全可用。旧实现直接返回错误，配合下面那个不可重试码，
    //   等于一行坏 key 让整条链的分析师节点全部拿不到模型（实测一次回放批次 55 个节点全灭）。
    //
    // 上限取 `prov.keys.len()`：dao 侧 `repo::provider::get_active_key` 的轮询游标每次调用
    //   前进一格，逐次再取即可覆盖到其它行；取满行数即停 —— master key 与密文都不随重试改变，
    //   全部解不开时继续打转只是白跑。
    let decrypt_attempts = prov.keys.len().max(1);
    let mut candidate = key;
    let mut last_decrypt_err: Option<String> = None;
    let mut resolved: Option<(ProviderKey, String)> = None;
    for attempt in 0..decrypt_attempts {
        match axagent_crypto::crypto::decrypt_key(&candidate.key_encrypted, master_key) {
            Ok(decrypted) => {
                resolved = Some((candidate, decrypted));
                break;
            },
            Err(e) => {
                tracing::warn!(
                    target: "axagent.llm_resolve",
                    provider_id = %prov.id,
                    key_id = %candidate.id,
                    attempt,
                    "[LLM-RESOLVE] 该 key 解密失败 ⇒ 换该供应商的下一把: {e}"
                );
                last_decrypt_err = Some(e.to_string());
                match axagent_harness::repositories::provider_repository()
                    .get_active_key(&prov.id)
                    .await
                {
                    Ok(next) => candidate = next,
                    // 取不到下一把（该供应商只有这一行，或已无启用 key）⇒ 停止轮换
                    Err(_) => break,
                }
            },
        }
    }
    let (key, api_key) = resolved.ok_or_else(|| {
        // ⚠ 码必须是 `API_KEY_DECRYPT_FAILED` 而不是 `UNSUPPORTED_PROVIDER`（2026-10-28）。
        //   同一条故障在缓存路径 `agent_executor.rs` 里早就用的是这个码；这里跟着
        //   `resolve_model_for_node` 的失败一起套 UNSUPPORTED_PROVIDER，于是界面把
        //   「密钥解不开」显示成「不支持的数据供应商」，把用户指向供应商配置而不是凭证。
        //   代价（有意的）：本码在 `is_non_retryable_error` 清单内 ⇒ 该节点**不再重试**。
        //   到这一步重试也不会变好 —— 逐把 key 的轮换已在上面做完，剩下的只有换凭证。
        NodeError::exec_failed(
            error_code::API_KEY_DECRYPT_FAILED,
            format!(
                "API key decryption failed: {}（已轮换该供应商的 {decrypt_attempts} 把 key）",
                last_decrypt_err.unwrap_or_else(|| "无错误详情".to_string())
            ),
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
        let at = src.find("decrypt_key(&candidate.key_encrypted, master_key)")?;
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
    /// 为什么钉源码文本而不是走行为测试：`resolve_provider_and_adapter` 走
    /// `harness::repositories::provider_repository()` 这个**进程级全局**，测试里换一张假表会
    /// 污染同批并行的其它用例；要做得先给 harness 加注入/还原的门面，属另一格改动。
    /// 本组断言钉的是**那次选择与接线**：谁把它改回 `UNSUPPORTED_PROVIDER`、或把轮换拆掉，
    /// 这里立刻红。
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

    /// key 级 failover 的接线缺口（纯函数，入参是源码文本）。
    ///
    /// 抽成函数是为了让负控拆在**同一守卫**上：拿改动过的源文喂进来，必须恰好红对应那一格。
    fn failover_wiring_gaps(src: &str) -> Vec<String> {
        let mut gaps = Vec::new();
        if !src.contains("let decrypt_attempts = prov.keys.len().max(1);") {
            gaps.push("轮换缺上限：应挂在**本供应商的 key 行数**上，避免全部坏时无限打转".into());
        }
        if !src.contains("decrypt_key(&candidate.key_encrypted, master_key)") {
            gaps.push("解密站点没在读 candidate：等于每轮都在解同一行坏密文".into());
        }
        if !src.contains("Ok(next) => candidate = next,") {
            gaps.push("解密失败后没把新取到的 key 赋回 candidate：换不动下一把".into());
        }
        gaps
    }

    /// 多密钥供应商的 **key 级 failover** 必须在位：解密失败先换该供应商的下一把 key，
    /// 换满行数才允许失败。一行坏密文不该让整条链拿不到模型（实测 55 个分析师节点全灭）。
    #[test]
    fn decrypt_site_rotates_keys_before_failing() {
        let src = include_str!("llm_resolve.rs");
        assert!(
            failover_wiring_gaps(src).is_empty(),
            "接线有缺口: {:?}",
            failover_wiring_gaps(src)
        );

        // 负控 A：把「赋回下一把」拆掉 ⇒ 只应红第 ③ 格
        let no_assign = src.replace("Ok(next) => candidate = next,", "Ok(_next) => {}");
        assert_ne!(no_assign, src, "负控 A 锚点腐烂：替换没生效");
        let gaps_a = failover_wiring_gaps(&no_assign);
        assert_eq!(gaps_a.len(), 1, "负控 A 应只红一条，实得 {gaps_a:?}");
        assert!(gaps_a[0].starts_with("解密失败后没把新取到的 key"), "{gaps_a:?}");

        // 负控 B：把上限改成写死的 1（= 不轮换）⇒ 只应红第 ① 格
        let no_cap = src.replace(
            "let decrypt_attempts = prov.keys.len().max(1);",
            "let decrypt_attempts = 1usize;",
        );
        assert_ne!(no_cap, src, "负控 B 锚点腐烂：替换没生效");
        let gaps_b = failover_wiring_gaps(&no_cap);
        assert_eq!(gaps_b.len(), 1, "负控 B 应只红一条，实得 {gaps_b:?}");
        assert!(gaps_b[0].starts_with("轮换缺上限"), "{gaps_b:?}");

        // 负控 C：解密读回最初那把 ⇒ 只应红第 ② 格
        let fixed_key = src.replace(
            "decrypt_key(&candidate.key_encrypted, master_key)",
            "decrypt_key(&key.key_encrypted, master_key)",
        );
        assert_ne!(fixed_key, src, "负控 C 锚点腐烂：替换没生效");
        let gaps_c = failover_wiring_gaps(&fixed_key);
        assert_eq!(gaps_c.len(), 1, "负控 C 应只红一条，实得 {gaps_c:?}");
        assert!(gaps_c[0].starts_with("解密站点没在读 candidate"), "{gaps_c:?}");
    }

    /// 改码的**后果**也必须钉住：本码在不可重试清单内 ⇒ 引擎层不再重复整段运行。
    /// 与「逐把 key 已在上面轮换过」配套 —— 少了任何一半，要么白跑六次，要么一行坏 key 定死全链。
    #[test]
    fn api_key_decrypt_failure_stays_non_retryable() {
        let msg = "API_KEY_DECRYPT_FAILED: API key decryption failed: Crypto error: \
                   Decryption failed: aead::Error";
        assert!(
            crate::work_engine::node_executor_trait::is_non_retryable_error(msg),
            "换完全部 key 仍解不开时是确定性失败：master key 与密文都不随重试改变"
        );
        // 对照（同族但性质不同）：供应商解析失败仍可重试，不得被顺手移进来
        let other = "UNSUPPORTED_PROVIDER: 未找到 ProviderAdapter for type: openai";
        assert!(
            !crate::work_engine::node_executor_trait::is_non_retryable_error(other),
            "UNSUPPORTED_PROVIDER 不在清单内是既有语义，本批没改它"
        );
    }
}
