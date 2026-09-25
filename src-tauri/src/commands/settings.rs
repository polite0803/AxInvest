// SPDX-License-Identifier: AGPL-3.0-only

use crate::AppState;
use axagent_agent_macro::agent_command;
use axagent_harness::types::*;
use tauri::AppHandle;
use tauri::State;

#[tauri::command]
#[agent_command(
    domain = settings,
    safety = Safe,
    call_mode = StateOnly,
    description = "获取应用设置"
)]
pub async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    let mut settings =
        axagent_dao::repo::settings::get_settings(state.harness.db()).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    settings.backup_dir = axagent_storage::path_vars::decode_path_opt(&settings.backup_dir);
    settings.gateway_ssl_cert_path =
        axagent_storage::path_vars::decode_path_opt(&settings.gateway_ssl_cert_path);
    settings.gateway_ssl_key_path =
        axagent_storage::path_vars::decode_path_opt(&settings.gateway_ssl_key_path);

    tracing::debug!(
        "[get_settings] 返回 settings: default_provider_id={:?} default_model_id={:?}",
        settings.default_provider_id,
        settings.default_model_id,
    );

    Ok(settings)
}

#[tauri::command]
#[agent_command(
    domain = settings,
    safety = Caution,
    call_mode = StateOnly,
    description = "保存应用设置"
)]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: AppSettings,
) -> Result<(), String> {
    tracing::debug!(
        "[save_settings] 接收到设置: default_provider_id={:?} default_model_id={:?}",
        settings.default_provider_id,
        settings.default_model_id,
    );

    settings.backup_dir = axagent_storage::path_vars::encode_path_opt(&settings.backup_dir);
    settings.gateway_ssl_cert_path =
        axagent_storage::path_vars::encode_path_opt(&settings.gateway_ssl_cert_path);
    settings.gateway_ssl_key_path =
        axagent_storage::path_vars::encode_path_opt(&settings.gateway_ssl_key_path);
    axagent_dao::repo::settings::save_settings(state.harness.db(), &settings).await.map_err(
        |e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        },
    )?;

    // 2.7 P1:telemetry_level 变更后同步更新共享级别句柄。
    //
    // `FilteringSink` 通过 `level_handle()` 引用同一 `Arc<RwLock<TelemetryLevel>>`,
    // 这里更新后所有正在运行的 sink 都会立即按新级别过滤事件,无需重建 sink 链。
    // 容错:解析失败时回退到 `Off`,保守保护用户隐私。
    {
        let new_level =
            axagent_telemetry::TelemetryLevel::from_str_or_off(&settings.telemetry_level);
        let mut guard = state.telemetry_level_handle.write();
        *guard = new_level;
    }

    // ── OS 级沙箱策略──
    // sandbox_mode 变更后立即更新全局策略，下一次工具调用即生效（无需重启）。
    {
        let workspace =
            settings.default_workspace_dir.as_ref().map(std::path::PathBuf::from).unwrap_or_else(
                || std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            );
        axagent_tools::registry::set_global_sandbox_policy(
            axagent_harness::SandboxPolicy::from_mode_str(&settings.sandbox_mode, workspace),
        );
        axagent_tools::registry::set_global_approval_policy(
            axagent_harness::ApprovalPolicy::from_policy_str(&settings.approval_policy),
        );
        tracing::info!(
            "[save_settings] 沙箱/审批策略已更新: sandbox_mode={} approval_policy={}",
            settings.sandbox_mode,
            settings.approval_policy
        );
    }

    // ── Guardian 审查闸门（PLAN-codex-parity-adoption R3-2）──
    // 开关变更立即生效（装载 / 停用全局桥），与上面的沙箱/审批策略同属「保存即生效」同步点。
    crate::init::guardian_bridge::refresh_global_guardian_bridge(state.inner()).await;

    // ── Graph RAG 实体图谱开关（RAG 设置面板）──
    //
    // 与上面 telemetry / 沙箱策略同属「保存后立即生效」的同步点。
    //
    // ⚠ 2026-09-15 补：此前这个开关**只在启动装配时读一次**（`init::services`），
    // 且注入点是 `OnceLock`（首次生效、之后忽略；也没有清除入口）⇒ 用户改完必须重启，
    // UI 文案只能写「重启后生效」，而「配置了却不即时生效」正是最容易被当成 bug 的形态。
    // 注入点已改为可替换槽位，这里把「开 ↔ 关」都即时反映到检索链路。
    crate::indexing::sync_entity_graph_provider(state.harness.db()).await;

    #[cfg(not(mobile))]
    {
        crate::tray::sync_tray_language(&app, &settings.language).map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
    }
    #[cfg(mobile)]
    {
        let _ = &app;
        Ok(())
    }
}
