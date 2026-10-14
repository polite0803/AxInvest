// SPDX-License-Identifier: AGPL-3.0-only

#[cfg(all(debug_assertions, not(target_os = "android")))]
use tauri_plugin_mcp_bridge;

pub fn register_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init());

    // 桌面端插件：autostart 和 global_shortcut 仅在桌面平台可用。
    // global_shortcut 此前漏注册：Cargo 依赖 / capabilities 授权 / 前端调用均就绪，
    // 但此处未 .plugin(...)，导致前端报 `plugin global-shortcut not found`，
    // 全局唤起快捷键（默认 Ctrl+Shift+A）静默失效。
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&'static str>>,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build());

    let builder = builder
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(feature = "notification")]
    let builder = builder.plugin(tauri_plugin_notification::init());

    #[cfg(all(debug_assertions, not(target_os = "android")))]
    let builder = builder.plugin(tauri_plugin_mcp_bridge::init());

    builder
}
