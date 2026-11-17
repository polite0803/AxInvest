// SPDX-License-Identifier: AGPL-3.0-only

//! 主行情源设置命令（判据③：换一个行情源不动主壳代码路径）。
//!
//! 写入 `AppSettings.market_data_primary_vendor` 并即时应用到共享 `AStockClient`：
//! 把该源提到 `quote` / `klines` 链首（只重排、不增删注册源；as_of 回放不受影响）。

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::storage as storage_err;
use axagent_agent_macro::agent_command;
use tauri::State;

#[agent_command(
    domain = "finance",
    safety = Caution,
    call_mode = StateInput,
    description = "设置主行情源（运行时重排，无需重编主壳）"
)]
#[tauri::command]
pub async fn stock_set_market_data_primary_vendor(
    state: State<'_, AppState>,
    vendor: Option<String>,
) -> Result<(), String> {
    let db = state.harness.db();
    let mut settings =
        axagent_dao::repo::settings::get_settings(db).await.map_err(|e| -> String {
            ErrorResponse::new(storage_err::READ_FILE_FAILED)
                .with_detail(format!("读取设置失败: {e}"))
                .into()
        })?;
    let normalized = vendor.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    settings.market_data_primary_vendor = normalized.clone();
    axagent_dao::repo::settings::save_settings(db, &settings).await.map_err(|e| -> String {
        ErrorResponse::new(storage_err::WRITE_FILE_FAILED)
            .with_detail(format!("保存设置失败: {e}"))
            .into()
    })?;

    state.stock().astock_client.set_primary_market_vendor(normalized.as_deref());
    tracing::info!(
        target: "astock-data::priority",
        primary = ?normalized,
        "主行情源已更新（quote/klines 链首重排）"
    );
    Ok(())
}
