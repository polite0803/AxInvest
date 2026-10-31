// SPDX-License-Identifier: AGPL-3.0-only

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::provider as provider_err;
use axagent_agent_macro::agent_command;
use axagent_harness::core_error::AxAgentError;
use axagent_harness::types::*;
use std::time::Instant;
use tauri::State;

/// 取该供应商的启用密钥 —— dao 侧「没有启用的密钥」与「取行失败」是两种失败，必须分码：
/// 前者用户能自己处置（去添加/启用密钥），后者是库侧故障。二者原本一起塌进 `COMMON_INTERNAL`，
/// 界面只能靠 `errMsg.includes("key")` 猜（ProviderDetail 旧写法）。
fn active_key_error(e: AxAgentError) -> String {
    // 写成 `matches!(&e, …)`：若 `match &e` 后在分支里消费 `e`，值仍被 scrutinee 借用 ⇒ E0507。
    if matches!(&e, AxAgentError::NotFound(_)) {
        String::from(ErrorResponse::new(provider_err::NO_ACTIVE_KEY).with_detail(e.to_string()))
    } else {
        String::from(ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    }
}

/// 解密提供商密钥行 —— 失败必须报 `PROVIDER_KEY_DECRYPT_FAILED`，不许塌进 `COMMON_INTERNAL`：
/// 「内部错误」指不到凭证，而这类失败重试也不会变好（master key 与密文都不随重试改变），
/// 用户唯一能做的动作是重新录入那一把密钥。
///
/// 日志带 provider_id / key_id / 密文长度 / 当前 master key 指纹：2026-10-28 与 10-31 两次
/// 现场都因为「不知道是哪一行、被哪把 key 加密」而无法结案（10-31 那次坏行在报错后被人重存覆盖）。
fn decrypt_provider_key(key_row: &ProviderKey, master_key: &[u8; 32]) -> Result<String, String> {
    axagent_crypto::decrypt_key(&key_row.key_encrypted, master_key).map_err(|e| {
        tracing::error!(
            target: "axagent.providers",
            provider_id = %key_row.provider_id,
            key_id = %key_row.id,
            cipher_len = key_row.key_encrypted.len(),
            master_key_fp = %axagent_crypto::master_key_fingerprint(master_key),
            "提供商密钥解密失败: {e}"
        );
        String::from(
            ErrorResponse::new(provider_err::KEY_DECRYPT_FAILED)
                .with_detail(format!("密钥行 {} 解密失败: {e}", key_row.id)),
        )
    })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Safe,
    call_mode = StateOnly,
    description = "列出所有可用的 LLM 提供商"
)]
pub async fn list_providers(state: State<'_, AppState>) -> Result<Vec<ProviderConfig>, String> {
    axagent_dao::repo::provider::list_providers_merged(state.harness.db()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "创建新的 LLM 提供商配置"
)]
pub async fn create_provider(
    state: State<'_, AppState>,
    input: CreateProviderInput,
) -> Result<ProviderConfig, String> {
    axagent_dao::repo::provider::create_provider(state.harness.db(), input).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "更新提供商配置"
)]
pub async fn update_provider(
    state: State<'_, AppState>,
    id: String,
    input: UpdateProviderInput,
) -> Result<ProviderConfig, String> {
    let real_id = axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), &id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    axagent_dao::repo::provider::update_provider(state.harness.db(), &real_id, input).await.map_err(
        |e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        },
    )
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Dangerous,
    call_mode = StateInput,
    description = "删除提供商配置"
)]
pub async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<(), String> {
    // Virtual built-in providers have no DB row — deletion is a no-op (they'll reappear)
    if id.starts_with("builtin_") {
        return Ok(());
    }
    axagent_dao::repo::provider::delete_provider(state.harness.db(), &id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "切换提供商启用状态"
)]
pub async fn toggle_provider(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let real_id = axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), &id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    axagent_dao::repo::provider::toggle_provider(state.harness.db(), &real_id, enabled)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "添加提供商 API 密钥"
)]
pub async fn add_provider_key(
    state: State<'_, AppState>,
    provider_id: String,
    raw_key: String,
) -> Result<ProviderKey, String> {
    let real_id =
        axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), &provider_id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
    let encrypted =
        axagent_crypto::encrypt_key(&raw_key, state.harness.master_key()).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    // SECURITY: 与 update_provider_key 保持一致，使用 SHA-256 哈希前 8 字符作为不可逆标识。
    // 旧实现用 `&raw_key[..8]` 按字节切片：key 含多字节 UTF-8 字符时若第 8 字节不是字符边界会
    // panic（Tauri 命令 panic 会被转成 IPC 错误，前端表现为无信息的「保存失败」），
    // 同时会把明文 key 前 8 位落库。改用哈希后两者同时消除。
    let prefix = format!("{}...", &axagent_crypto::sha256_hash(&raw_key)[..8]);
    axagent_dao::repo::provider::add_provider_key(state.harness.db(), &real_id, &encrypted, &prefix)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "更新提供商 API 密钥"
)]
pub async fn update_provider_key(
    state: State<'_, AppState>,
    key_id: String,
    raw_key: String,
) -> Result<ProviderKey, String> {
    let encrypted =
        axagent_crypto::encrypt_key(&raw_key, state.harness.master_key()).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    // SECURITY: 使用 SHA-256 哈希前 8 字符作为不可逆标识，避免明文 key 前 8 字符泄露
    let prefix = format!("{}...", &axagent_crypto::sha256_hash(&raw_key)[..8]);
    axagent_dao::repo::provider::update_provider_key(
        state.harness.db(),
        &key_id,
        &encrypted,
        &prefix,
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Dangerous,
    call_mode = StateInput,
    description = "删除提供商 API 密钥"
)]
pub async fn delete_provider_key(state: State<'_, AppState>, key_id: String) -> Result<(), String> {
    axagent_dao::repo::provider::delete_provider_key(state.harness.db(), &key_id).await.map_err(
        |e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        },
    )
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "切换密钥启用状态"
)]
pub async fn toggle_provider_key(
    state: State<'_, AppState>,
    key_id: String,
    enabled: bool,
) -> Result<(), String> {
    axagent_dao::repo::provider::toggle_provider_key(state.harness.db(), &key_id, enabled)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

/// SECURITY (C1): 仅返回密钥前缀用于 UI 展示，禁止返回完整明文密钥。
/// 若前端需要验证密钥有效性，请使用 `validate_provider_key` 命令。
#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Safe,
    call_mode = StateInput,
    description = "获取密钥前缀用于 UI 展示"
)]
pub async fn get_decrypted_provider_key(
    state: State<'_, AppState>,
    key_id: String,
) -> Result<String, String> {
    let key_row = axagent_dao::repo::provider::get_provider_key(state.harness.db(), &key_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let decrypted = decrypt_provider_key(&key_row, state.harness.master_key())?;
    Ok(axagent_crypto::key_prefix(&decrypted))
}

/// SECURITY (C1 修订): 完整明文密钥仅在 UI 显式请求（「显示」按钮）时返回，
/// 供用户查看/复制自己的密钥。与 `get_decrypted_provider_key`（仅前缀，Safe）区分：
/// 本命令经 agent 桥暴露为 Caution（需确认），避免智能体静默读取完整密钥。
#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "获取提供商完整 API 密钥（用户在 UI 显式请求显示时使用）"
)]
pub async fn reveal_provider_key(
    state: State<'_, AppState>,
    key_id: String,
) -> Result<String, String> {
    let key_row = axagent_dao::repo::provider::get_provider_key(state.harness.db(), &key_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    decrypt_provider_key(&key_row, state.harness.master_key())
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Safe,
    call_mode = StateInput,
    description = "验证提供商密钥有效性"
)]
pub async fn validate_provider_key(
    state: State<'_, AppState>,
    key_id: String,
) -> Result<bool, String> {
    let key_row = axagent_dao::repo::provider::get_provider_key(state.harness.db(), &key_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let decrypted = decrypt_provider_key(&key_row, state.harness.master_key())?;
    let provider =
        axagent_dao::repo::provider::get_provider(state.harness.db(), &key_row.provider_id)
            .await
            .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    // Use the registry to validate by listing models

    let provider_type_str = axagent_harness::types::provider_registry_key(&provider.provider_type);
    let adapter = state
        .harness
        .provider_registry()
        .get(provider_type_str)
        .ok_or_else(|| format!("No adapter for provider type: {}", provider_type_str))?;
    let global_settings = axagent_dao::repo::settings::get_settings(state.harness.db())
        .await
        .inspect_err(|e| {
            tracing::warn!("Failed to read global settings, falling back to defaults: {}", e)
        })
        .unwrap_or_default();
    let resolved_proxy = axagent_harness::types::provider_model::resolve_provider_proxy(
        &provider.proxy_config,
        &global_settings,
    );
    let ctx = axagent_harness::ProviderRequestContext {
        header_timeout_secs: None,
        api_key: decrypted,
        key_id: key_id.clone(),
        provider_id: provider.id.clone(),
        base_url: Some(axagent_harness::resolve_base_url_for_type(
            &provider.api_host,
            &provider.provider_type,
        )),
        api_path: provider.api_path.clone(),
        proxy_config: resolved_proxy,
        custom_headers: provider.custom_headers.as_ref().and_then(|s| serde_json::from_str(s).ok()),
        api_mode: None,
        conversation: None,
        previous_response_id: None,
        store_response: None,
    };
    let valid = match adapter.validate_key(&ctx).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Key validation failed for key {}: {}", key_id, e);
            // Update as invalid, then return the error
            let _ = axagent_dao::repo::provider::update_key_validation(
                state.harness.db(),
                &key_id,
                false,
            )
            .await;
            // C-3: 迁移到 ErrorResponse，保留 Retryable 分类便于前端引导重试
            return Err(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Retryable,
            )
            .to_string());
        },
    };
    // Update validation timestamp
    axagent_dao::repo::provider::update_key_validation(state.harness.db(), &key_id, valid)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    Ok(valid)
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "保存提供商模型列表"
)]
pub async fn save_models(
    state: State<'_, AppState>,
    provider_id: String,
    models: Vec<Model>,
) -> Result<(), String> {
    let real_id =
        axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), &provider_id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
    axagent_dao::repo::provider::save_models(state.harness.db(), &real_id, &models).await.map_err(
        |e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        },
    )
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "切换模型启用状态"
)]
pub async fn toggle_model(
    state: State<'_, AppState>,
    provider_id: String,
    model_id: String,
    enabled: bool,
) -> Result<Model, String> {
    let real_id =
        axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), &provider_id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
    axagent_dao::repo::provider::toggle_model(state.harness.db(), &real_id, &model_id, enabled)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "更新模型参数配置"
)]
pub async fn update_model_params(
    state: State<'_, AppState>,
    provider_id: String,
    model_id: String,
    overrides: ModelParamOverrides,
) -> Result<Model, String> {
    let real_id =
        axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), &provider_id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
    axagent_dao::repo::provider::update_model_params(
        state.harness.db(),
        &real_id,
        &model_id,
        overrides,
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Safe,
    call_mode = StateInput,
    description = "从远端获取可用模型列表"
)]
pub async fn fetch_remote_models(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<Vec<Model>, String> {
    let real_id =
        axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), &provider_id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
    let provider = axagent_dao::repo::provider::get_provider(state.harness.db(), &real_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    // llama.cpp 供应商：模型列表 = 下载目录中的 GGUF 文件（无需远端拉取）
    if provider.provider_type == ProviderType::LlamaCpp {
        let dir = crate::commands::local_model::download_dir(state.harness.db()).await;
        return Ok(crate::commands::local_model::scan_gguf_models(&real_id, &dir));
    }
    // Get an enabled key for the provider
    let key_row = axagent_dao::repo::provider::get_active_key(state.harness.db(), &real_id)
        .await
        .map_err(active_key_error)?;
    let decrypted = decrypt_provider_key(&key_row, state.harness.master_key())?;

    let provider_type_str = axagent_harness::types::provider_registry_key(&provider.provider_type);
    let adapter = state
        .harness
        .provider_registry()
        .get(provider_type_str)
        .ok_or_else(|| format!("No adapter for provider type: {}", provider_type_str))?;
    let global_settings = axagent_dao::repo::settings::get_settings(state.harness.db())
        .await
        .inspect_err(|e| {
            tracing::warn!("Failed to read global settings, falling back to defaults: {}", e)
        })
        .unwrap_or_default();
    let resolved_proxy = axagent_harness::types::provider_model::resolve_provider_proxy(
        &provider.proxy_config,
        &global_settings,
    );
    let ctx = axagent_harness::ProviderRequestContext {
        header_timeout_secs: None,
        api_key: decrypted,
        key_id: key_row.id.clone(),
        provider_id: provider.id.clone(),
        base_url: Some(axagent_harness::resolve_base_url_for_type(
            &provider.api_host,
            &provider.provider_type,
        )),
        api_path: provider.api_path.clone(),
        proxy_config: resolved_proxy,
        custom_headers: provider.custom_headers.as_ref().and_then(|s| serde_json::from_str(s).ok()),
        api_mode: None,
        conversation: None,
        previous_response_id: None,
        store_response: None,
    };
    // Android 网络延迟更高，超时放宽到 60s；桌面端保持 30s
    let model_timeout_secs = if cfg!(target_os = "android") { 60 } else { 30 };
    let mut models = tokio::time::timeout(
        std::time::Duration::from_secs(model_timeout_secs),
        adapter.list_models(&ctx),
    )
    .await
    .map_err(|_| {
        ErrorResponse::new(provider_err::MODEL_LIST_TIMEOUT).with_detail(format!(
            "获取模型列表超时 ({}s)。请检查网络连接和 API 地址是否正确。",
            model_timeout_secs
        ))
    })?
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    for model in &mut models {
        if model.max_tokens.is_none() {
            model.max_tokens =
                axagent_kit::model_knowledge::get_model_context_window(&model.model_id);
        }
    }
    // Deduplicate by model_id (keep last occurrence)
    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<Model> = Vec::with_capacity(models.len());
    for model in models.into_iter().rev() {
        if seen.insert(model.model_id.clone()) {
            deduped.push(model);
        }
    }
    deduped.reverse();
    Ok(deduped)
}

/// Test a single model's availability by sending a minimal chat request.
/// Returns latency in milliseconds on success.
#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Safe,
    call_mode = StateInput,
    description = "测试模型可用性并返回延迟"
)]
pub async fn test_model(
    state: State<'_, AppState>,
    provider_id: String,
    model_id: String,
) -> Result<u64, String> {
    let real_id =
        axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), &provider_id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
    let provider = axagent_dao::repo::provider::get_provider(state.harness.db(), &real_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let key_row = axagent_dao::repo::provider::get_active_key(state.harness.db(), &real_id)
        .await
        .map_err(active_key_error)?;
    let decrypted = decrypt_provider_key(&key_row, state.harness.master_key())?;

    let provider_type_str = axagent_harness::types::provider_registry_key(&provider.provider_type);
    let adapter = state
        .harness
        .provider_registry()
        .get(provider_type_str)
        .ok_or_else(|| format!("No adapter for provider type: {}", provider_type_str))?;
    let global_settings = axagent_dao::repo::settings::get_settings(state.harness.db())
        .await
        .inspect_err(|e| {
            tracing::warn!("Failed to read global settings, falling back to defaults: {}", e)
        })
        .unwrap_or_default();
    let resolved_proxy = axagent_harness::types::provider_model::resolve_provider_proxy(
        &provider.proxy_config,
        &global_settings,
    );
    let ctx = axagent_harness::ProviderRequestContext {
        header_timeout_secs: None,
        api_key: decrypted,
        key_id: key_row.id.clone(),
        provider_id: provider.id.clone(),
        base_url: Some(axagent_harness::resolve_base_url_for_type(
            &provider.api_host,
            &provider.provider_type,
        )),
        api_path: provider.api_path.clone(),
        proxy_config: resolved_proxy,
        custom_headers: provider.custom_headers.as_ref().and_then(|s| serde_json::from_str(s).ok()),
        api_mode: None,
        conversation: None,
        previous_response_id: None,
        store_response: None,
    };
    let request = ChatRequest {
        model: model_id,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: ChatContent::Text("hi".into()),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
        }],
        stream: false,
        temperature: None,
        top_p: None,
        max_tokens: Some(1),
        tools: None,
        thinking_budget: None,
        use_max_completion_tokens: None,
        thinking_param_style: None,
        api_mode: None,
        instructions: None,
        conversation: None,
        previous_response_id: None,
        store: None,
        response_format: None,
    };
    let start = Instant::now();
    adapter.chat(&ctx, request.into()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    Ok(start.elapsed().as_millis() as u64)
}

#[tauri::command]
#[agent_command(
    domain = provider,
    safety = Caution,
    call_mode = StateInput,
    description = "重新排序提供商列表"
)]
pub async fn reorder_providers(
    state: State<'_, AppState>,
    provider_ids: Vec<String>,
) -> Result<(), String> {
    // Materialize any virtual built-in providers so sort_order can be persisted
    let mut real_ids = Vec::with_capacity(provider_ids.len());
    for id in &provider_ids {
        let real_id = axagent_dao::repo::provider::resolve_provider_id(state.harness.db(), id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
        real_ids.push(real_id);
    }
    axagent_dao::repo::provider::reorder_providers(state.harness.db(), &real_ids).await.map_err(
        |e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        },
    )
}

#[cfg(test)]
mod key_error_code_gate_tests {
    /// 「提供商密钥失败分码」的站点普查（纯函数，入参是源码文本）。
    ///
    /// 返回违规清单而不是直接 assert：一次能看全缺口，负控也能拆在同一条守卫上。
    /// 为什么钉源码文本而不是走行为测试：这些命令都要 `State<'_, AppState>`（进程级
    /// harness + 真库），拿不到假表；而 2026-10-28 与 10-31 两次现场都是「解密失败塌进
    /// COMMON_INTERNAL ⇒ 界面指不到凭证」，改动者把站点换回就地 `decrypt_key` 时不会碰到别的门。
    fn decrypt_sites_lose_their_codes(full_src: &str) -> Vec<String> {
        // 只普查 `#[cfg(test)]` 之前的生产代码：本模块里负控用的锚点字面量（以及上面那句
        // 注释）本身就含这些子串，按整文件计数必然假红。
        let src = full_src.split("#[cfg(test)]").next().unwrap_or(full_src);
        let mut gaps = Vec::new();
        // 就地 `axagent_crypto::decrypt_key(` 只允许出现在 helper 内部这一处。
        let inline = src.matches("axagent_crypto::decrypt_key(").count();
        if inline != 1 {
            gaps.push(format!("就地解密站点应为 1 处（helper 内），现为 {inline} 处：多出来的会重新塌进 COMMON_INTERNAL"));
        }
        if src.matches("decrypt_provider_key(&key_row, state.harness.master_key())").count() != 5 {
            gaps.push("五个界面入口没都走 decrypt_provider_key".to_string());
        }
        if src.matches("map_err(active_key_error)?").count() != 2 {
            gaps.push("取密钥的两站没都按码分支（fetch_remote_models / test_model）".to_string());
        }
        gaps
    }

    #[test]
    fn provider_key_failures_keep_their_own_codes() {
        let gaps = decrypt_sites_lose_their_codes(include_str!("providers.rs"));
        assert!(gaps.is_empty(), "密钥失败必须各归其码：{gaps:?}");
    }

    /// 负控 A：把四处入口换回就地解密 + `from_error` ⇒ 前两条守卫必须同时翻红。
    #[test]
    fn gate_is_live_for_decrypt_sites() {
        let broken = include_str!("providers.rs").replace(
            "let decrypted = decrypt_provider_key(&key_row, state.harness.master_key())?;",
            "let decrypted = axagent_crypto::decrypt_key(&key_row.key_encrypted, state.harness.master_key())\n        .map_err(|e| String::from(crate::commands::error::ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)))?;",
        );
        assert_ne!(
            broken,
            include_str!("providers.rs"),
            "负控锚点腐烂：替换没生效，本用例等于没通电"
        );
        assert_eq!(decrypt_sites_lose_their_codes(&broken).len(), 2);
    }

    /// 负控 B：把取密钥两站换回 `from_error` ⇒ 只有第三条守卫翻红（证明三条各管各的）。
    #[test]
    fn gate_is_live_for_active_key_sites() {
        let broken = include_str!("providers.rs").replace(
            "map_err(active_key_error)?",
            "map_err(|e| String::from(crate::commands::error::ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)))?",
        );
        assert_ne!(
            broken,
            include_str!("providers.rs"),
            "负控锚点腐烂：替换没生效，本用例等于没通电"
        );
        assert_eq!(decrypt_sites_lose_their_codes(&broken).len(), 1);
    }
}
