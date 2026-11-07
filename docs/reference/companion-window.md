> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Companion 悬浮窗可行性调研

## 概述

nomifun-tauri 的 `nomifun-companion` 实现了桌面悬浮 AI 助手窗口（类似 Claude Desktop 的悬浮助手），macOS 用 `macos-private-api` 实现透明点击穿透窗口。本报告评估 AxAgent 实现类似功能的可行性。

## Tauri v2 窗口能力支持

### 透明窗口

Tauri v2 支持 `window.create()` 的 `transparent: true` 参数。目前 AxAgent 的 `tauri.conf.json` 中主窗口已使用 `titleBarStyle: "Overlay"` + `hiddenTitle: true`，但未启用 `transparent`。

**可行性**：✅ Tauri v2 原生支持。在创建 companion 窗口时传入 `transparent: true` 即可。

### 点击穿透（mouse ignore）

Tauri v2 **没有原生 API** 实现窗口级别的鼠标事件穿透。需要平台特定的处理：

| 平台 | 方案 | 可行性 |
|------|------|--------|
| macOS | `NSWindow.ignoresMouseEvents = true`，通过 `objc2` 运行时调用 | 高 — 项目中已有 `objc2` 依赖（`Cargo.toml` 第 174 行 `target_vendor = "apple"`） |
| Windows | `WS_EX_TRANSPARENT + WS_EX_LAYERED` 扩展窗口样式 | 高 — 项目中已有 `windows-sys` 依赖（`Cargo.toml` 第 183 行） |
| Linux | 部分 X11  Compositor 支持 `_NET_WM_WINDOW_OPACITY` 和 `_NET_WM_STATE_SKIP_TASKBAR` | 低 — 各 compositor 差异大，缺少通用方案 |

### 窗口钉住（always-on-top + skip taskbar）

Tauri v2 支持 `WindowBuilder::always_on_top(true)`。`skip_taskbar` 需要平台特定代码。

### Companion 功能实现方式

推荐两种架构：

**方案 A：Tauri 多窗口（推荐）**

在现有 AxAgent 进程中创建第二个 `WebviewWindow`：
```rust
use tauri::WebviewWindowBuilder;
let companion = WebviewWindowBuilder::new(app, "companion", tauri::WebviewUrl::App("companion.html".into()))
    .inner_size(400.0, 600.0)
    .transparent(true)
    .always_on_top(true)
    .decorations(false)
    .build()?;
// macOS: set ignoresMouseEvents
// Windows: set WS_EX_TRANSPARENT
```

优点：共享 Tauri IPC、共享 Rust 后端、开发简单
缺点：占用渲染进程资源

**方案 B：独立窗口 + IPC**

创建单独的轻量窗口，通过 WebSocket 或 stdio 与主进程通信。
优点：资源独立，主窗口崩溃不影响
缺点：IPC 复杂度高，需要独立的通信协议

## 关键依赖

- **macOS**: `objc2`（已在 `Cargo.toml` 第 174 行 `[target.'cfg(target_vendor = "apple")'.dependencies]` 中）
- **Windows**: `windows-sys`（已在 `Cargo.toml` 第 182 行 `[target.'cfg(target_os = "windows")'.dependencies]` 中）
- **Tauri**: `tray-icon`（已在 `Cargo.toml` 第 148 行 `tauri` features 中）

## 建议实现路径

### Phase 1：基础窗口（~2 天）
1. 创建 `crates/companion/` 新 crate
2. 实现 `create_companion_window()` 函数，创建透明无边框窗口
3. 集成到 `init/state.rs` 的条件初始化

### Phase 2：点击穿透（~3 天）
1. macOS：通过 `objc2` 调用 `setIgnoresMouseEvents:`
2. Windows：通过 `windows-sys` 设置 `WS_EX_TRANSPARENT`
3. 快捷键切换穿透模式（⌘⇧Space 或 Ctrl⇧Space）

### Phase 3：AI 交互集成（~5 天）
1. 加载 companion UI 页面（React 组件）
2. 通过 Tauri events (`app.emit()` / `app.listen()`) 与主窗口通信
3. 精简模式：仅显示当前对话摘要 + 快速输入

## 不推荐的做法

1. **macOS `macos-private-api`**：Tauri v2 的 `macos-private-api` feature 主要用于透明磁铁效果（traffic light 按钮穿透）。Companion 窗口不需要此 API。
2. **独立进程**：AxAgent 不是 Electron，Tauri 的进程模型不适合拆分 UI 窗口为独立进程。

## 结论

**可行，中等复杂度**。Tauri v2 提供了足够的底层支持，且项目中已有 `objc2` 和 `windows-sys` 依赖可直接复用。建议采用方案 A（Tauri 多窗口），从基础透明窗口开始逐步迭代。
