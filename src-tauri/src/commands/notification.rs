// SPDX-License-Identifier: AGPL-3.0-only

//! 出站推送渠道配置命令（判据③：关一个渠道不动主壳代码路径）。
//!
//! 渠道凭据随 `AppSettings.notification_channels` 落 DB；前端保存设置后调用
//! [`notification_apply_channel_config`] 让共享 dispatcher 即时重装（无需重启），
//! [`notification_list_channels`] 供设置页展示「已配置 / 已启用 / 当前生效」三态。

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::storage as storage_err;
use axagent_agent_macro::agent_command;
use axagent_harness::types::{NOTIFICATION_CHANNEL_IDS, NotificationChannelConfig};
use serde::Serialize;
use tauri::State;

/// 单个渠道的三态投影（供前端设置页渲染）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationChannelStatus {
    pub id: String,
    /// 用户配置里的启用位。
    pub enabled: bool,
    /// 凭据是否齐全（缺凭据的启用渠道不会真正装配）。
    pub configured: bool,
    /// 当前是否已注册到 dispatcher（装配成功的才算）。
    pub active: bool,
}

/// 按渠道 id 判断凭据是否齐全——与 `axagent_notification::config_assembly` 的装配判据同一口径。
fn has_required_credentials(cfg: &NotificationChannelConfig) -> bool {
    match cfg.id.as_str() {
        "pushplus" | "serverchan" => !cfg.token.trim().is_empty(),
        "ntfy" => cfg.api_url.as_deref().is_some_and(|s| !s.trim().is_empty()),
        "gotify" => {
            !cfg.token.trim().is_empty()
                && cfg.api_url.as_deref().is_some_and(|s| !s.trim().is_empty())
        },
        _ => false,
    }
}

#[agent_command(
    domain = settings,
    safety = Safe,
    call_mode = StateOnly,
    description = "列出出站推送渠道的三态"
)]
#[tauri::command]
pub async fn notification_list_channels(
    state: State<'_, AppState>,
) -> Result<Vec<NotificationChannelStatus>, String> {
    let settings = axagent_dao::repo::settings::get_settings(state.harness.db()).await.map_err(
        |e| -> String {
            ErrorResponse::new(storage_err::READ_FILE_FAILED)
                .with_detail(format!("读取渠道配置失败: {e}"))
                .into()
        },
    )?;
    let active_names = state.notification_dispatcher.list_channel_names().await;

    // 以四个权威 id 为骨架，合并用户配置与运行时注册态；配置里多出的未知 id 也如实列出。
    let mut statuses: Vec<NotificationChannelStatus> = NOTIFICATION_CHANNEL_IDS
        .iter()
        .map(|id| {
            let cfg = settings.notification_channels.iter().find(|c| c.id == *id);
            NotificationChannelStatus {
                id: (*id).to_string(),
                enabled: cfg.map(|c| c.enabled).unwrap_or(false),
                configured: cfg.map(has_required_credentials).unwrap_or(false),
                active: active_names.iter().any(|n| n == id),
            }
        })
        .collect();
    for cfg in &settings.notification_channels {
        if !NOTIFICATION_CHANNEL_IDS.contains(&cfg.id.as_str()) {
            statuses.push(NotificationChannelStatus {
                id: cfg.id.clone(),
                enabled: cfg.enabled,
                configured: false,
                active: false,
            });
        }
    }
    Ok(statuses)
}

#[agent_command(
    domain = settings,
    safety = Caution,
    call_mode = StateOnly,
    description = "按 settings 即时重装出站推送渠道"
)]
#[tauri::command]
pub async fn notification_apply_channel_config(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let settings = axagent_dao::repo::settings::get_settings(state.harness.db()).await.map_err(
        |e| -> String {
            ErrorResponse::new(storage_err::READ_FILE_FAILED)
                .with_detail(format!("读取渠道配置失败: {e}"))
                .into()
        },
    )?;
    let active = axagent_notification::config_assembly::apply_channel_configs(
        &state.notification_dispatcher,
        &settings.notification_channels,
    )
    .await;
    tracing::info!(
        target: "axagent_notification::assembly",
        count = active.len(),
        channels = ?active,
        "出站推送渠道已按 settings 重装"
    );
    Ok(active)
}
