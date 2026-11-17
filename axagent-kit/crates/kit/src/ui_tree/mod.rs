// SPDX-License-Identifier: AGPL-3.0-only

//! 无障碍树语义化实现（kit 层，implementor）。
//!
//! `harness::ui_tree` 定义契约与 DTO，本模块提供三平台 provider：
//! - Windows：UI Automation（UIA）；
//! - macOS：Accessibility API（AX）；
//! - Linux：AT-SPI2（atspi crate）。
//!
//! 观察走树、操作走语义动作，全链路零坐标；键盘注入由 enigo 提供但仅保留
//! 纯键盘路径（按键 + 修饰键 + 可选文本），不接受任何坐标参数。

use axagent_harness::ui_tree::{
    UiActionKind, UiAutomationError, UiKeyModifier, UiNode, UiNodeId, UiScrollIntent,
    UiTreeProvider, UiTreeQuery, UiTreeSnapshot,
};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::WindowsUiTreeProvider;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacosUiTreeProvider;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxUiTreeProvider;

/// 创建当前平台的无障碍树 provider。
///
/// 三平台均返回具体实现；其它平台（Android / iOS 等）返回 `Unsupported` 实现，
/// 所有方法给出明确错误，而不是 panic。
pub fn platform_ui_tree_provider() -> std::sync::Arc<dyn UiTreeProvider> {
    #[cfg(target_os = "windows")]
    {
        std::sync::Arc::new(WindowsUiTreeProvider::new())
    }
    #[cfg(target_os = "macos")]
    {
        std::sync::Arc::new(MacosUiTreeProvider::new())
    }
    #[cfg(target_os = "linux")]
    {
        std::sync::Arc::new(LinuxUiTreeProvider::new())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        std::sync::Arc::new(UnsupportedUiTreeProvider)
    }
}

/// 平台无关句柄工厂：生成带后端标识与当前代次的节点 id。
pub(crate) fn make_id(backend: &str, raw_id: &str, generation: u64) -> UiNodeId {
    UiNodeId { backend: backend.to_string(), raw_id: raw_id.to_string(), session_gen: generation }
}

/// 校验句柄代次一致，且 `backend` 与 provider 后端匹配。
pub(crate) fn check_handle(
    id: &UiNodeId,
    backend: &str,
    generation: u64,
) -> Result<(), UiAutomationError> {
    if id.backend != backend {
        return Err(UiAutomationError::Platform(format!(
            "节点句柄后端不匹配：期望 {backend}，实际 {}",
            id.backend
        )));
    }
    if id.session_gen != generation {
        return Err(UiAutomationError::StaleHandle(format!(
            "句柄代次 {} 已过期，当前代次 {}",
            id.session_gen, generation
        )));
    }
    Ok(())
}

/// 按查询条件过滤节点（平台实现统一调用，避免重复逻辑）。
///
/// 保留匹配节点自身；父节点因子女命中而保留，保证树结构可定位。
pub(crate) fn filter_tree(nodes: &mut Vec<UiNode>, query: &UiTreeQuery) {
    let application = query.application.as_deref();
    let window_title = query.window_title.as_deref();
    let role = query.role.as_deref();
    let name_contains = query.name_contains.as_deref();
    let max_depth = query.max_depth;

    fn keep(
        node: &UiNode,
        app: Option<&str>,
        title: Option<&str>,
        role: Option<&str>,
        name: Option<&str>,
    ) -> bool {
        if app.is_some_and(|a| !node.application.contains(a)) {
            return false;
        }
        if title.is_some_and(|t| !node.window_title.as_deref().unwrap_or_default().contains(t)) {
            return false;
        }
        if role.is_some_and(|r| node.role != r) {
            return false;
        }
        if name.is_some_and(|n| !node.name.contains(n)) {
            return false;
        }
        true
    }

    fn prune(
        node: &mut UiNode,
        depth: u32,
        app: Option<&str>,
        title: Option<&str>,
        role: Option<&str>,
        name: Option<&str>,
        max_depth: Option<u32>,
    ) -> bool {
        if max_depth.is_some_and(|limit| depth >= limit) {
            node.children.clear();
            return keep(node, app, title, role, name);
        }
        node.children.retain_mut(|child| {
            let child_kept = prune(child, depth + 1, app, title, role, name, max_depth);
            keep(child, app, title, role, name) || child_kept
        });
        keep(node, app, title, role, name) || !node.children.is_empty()
    }

    nodes.retain_mut(|node| {
        prune(node, 0, application, window_title, role, name_contains, max_depth)
    });
}

/// 纯键盘注入（无坐标）：复用 enigo 键盘 API。
///
/// 保留 enigo 是为了 Q1「保留纯键盘注入」裁定；这里绝不调用坐标相关 API，
/// 也不读取屏幕像素，因此不会引入 xcap / pipewire 依赖链。
///
/// enigo 是 `ui-automation` feature 下的 optional 依赖：feature 关闭时明确报
/// `Unsupported`，而不是编译失败或静默吞掉按键。
pub(crate) mod enigo_keyboard {
    use axagent_harness::ui_tree::{UiAutomationError, UiKeyModifier};

    pub(crate) fn press_key(
        key: &str,
        modifiers: &[UiKeyModifier],
        text: Option<&str>,
    ) -> Result<(), UiAutomationError> {
        #[cfg(not(feature = "ui-automation"))]
        {
            let _ = (key, modifiers, text);
            Err(UiAutomationError::Unsupported(
                "键盘注入需要启用 ui-automation 能力（当前构建未启用 enigo）".into(),
            ))
        }

        #[cfg(feature = "ui-automation")]
        {
            use enigo::{Direction, Enigo, Key, Keyboard, Settings};

            let mut enigo = Enigo::new(&Settings::default())
                .map_err(|e| UiAutomationError::Platform(format!("Enigo 初始化失败: {e}")))?;

            if let Some(text) = text {
                enigo
                    .text(text)
                    .map_err(|e| UiAutomationError::Platform(format!("文本注入失败: {e}")))?;
                return Ok(());
            }

            let mut pressed = Vec::new();
            for m in modifiers {
                let key = match m {
                    UiKeyModifier::Ctrl => Key::Control,
                    UiKeyModifier::Alt => Key::Alt,
                    UiKeyModifier::Shift => Key::Shift,
                    UiKeyModifier::Meta => Key::Meta,
                };
                enigo
                    .key(key, Direction::Press)
                    .map_err(|e| UiAutomationError::Platform(format!("按下修饰键失败: {e}")))?;
                pressed.push(key);
            }

            let key_enum = map_key(key);
            let result = enigo
                .key(key_enum, Direction::Click)
                .map_err(|e| UiAutomationError::Platform(format!("按键 {key} 失败: {e}")));

            for key in pressed.into_iter().rev() {
                let _ = enigo.key(key, Direction::Release);
            }
            result
        }
    }

    #[cfg(feature = "ui-automation")]
    fn map_key(key: &str) -> enigo::Key {
        use enigo::Key;
        match key.to_ascii_lowercase().as_str() {
            "enter" | "return" => Key::Return,
            "tab" => Key::Tab,
            "escape" | "esc" => Key::Escape,
            "backspace" => Key::Backspace,
            "delete" => Key::Delete,
            "space" => Key::Space,
            c if c.chars().count() == 1 => Key::Unicode(c.chars().next().unwrap()),
            _ => Key::Unicode('?'),
        }
    }
}

pub(crate) fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 非桌面平台的显式降级实现（编译期保底，不 panic）。
pub struct UnsupportedUiTreeProvider;

#[async_trait::async_trait]
impl UiTreeProvider for UnsupportedUiTreeProvider {
    async fn query(&self, _query: UiTreeQuery) -> Result<UiTreeSnapshot, UiAutomationError> {
        Err(UiAutomationError::Unsupported("当前平台不支持无障碍树".into()))
    }

    async fn invoke(&self, _id: &UiNodeId) -> Result<(), UiAutomationError> {
        Err(UiAutomationError::Unsupported("当前平台不支持无障碍树操作".into()))
    }

    async fn set_value(&self, _id: &UiNodeId, _value: &str) -> Result<(), UiAutomationError> {
        Err(UiAutomationError::Unsupported("当前平台不支持无障碍树操作".into()))
    }

    async fn focus(&self, _id: &UiNodeId) -> Result<(), UiAutomationError> {
        Err(UiAutomationError::Unsupported("当前平台不支持无障碍树操作".into()))
    }

    async fn perform(
        &self,
        _id: &UiNodeId,
        _action: UiActionKind,
        _scroll_intent: Option<UiScrollIntent>,
    ) -> Result<(), UiAutomationError> {
        Err(UiAutomationError::Unsupported("当前平台不支持无障碍树操作".into()))
    }

    async fn press_key(
        &self,
        _key: &str,
        _modifiers: &[UiKeyModifier],
        _text: Option<&str>,
    ) -> Result<(), UiAutomationError> {
        Err(UiAutomationError::Unsupported("当前平台不支持键盘注入".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> Vec<UiNode> {
        fn node(id: &str, app: &str, role: &str, name: &str, children: Vec<UiNode>) -> UiNode {
            UiNode {
                id: make_id("mock", id, 1),
                role: role.into(),
                name: name.into(),
                value: None,
                description: None,
                states: vec![],
                supported_actions: vec![UiActionKind::Invoke],
                application: app.into(),
                window_title: Some("窗口".into()),
                bounds: None,
                children,
            }
        }
        vec![
            node(
                "root1",
                "记事本",
                "window",
                "未命名",
                vec![
                    node("btn1", "记事本", "button", "确定", vec![]),
                    node("edit1", "记事本", "edit", "输入", vec![]),
                ],
            ),
            node(
                "root2",
                "浏览器",
                "window",
                "主页",
                vec![node("btn2", "浏览器", "button", "搜索", vec![])],
            ),
        ]
    }

    #[test]
    fn filter_by_application_keeps_ancestors() {
        let mut nodes = sample_tree();
        filter_tree(
            &mut nodes,
            &UiTreeQuery { application: Some("记事本".into()), ..Default::default() },
        );
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "未命名");
        assert_eq!(nodes[0].children.len(), 2);
    }

    #[test]
    fn filter_by_role_and_name() {
        let mut nodes = sample_tree();
        filter_tree(
            &mut nodes,
            &UiTreeQuery {
                role: Some("button".into()),
                name_contains: Some("搜索".into()),
                ..Default::default()
            },
        );
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].children.len(), 1);
        assert_eq!(nodes[0].children[0].name, "搜索");
    }

    #[test]
    fn filter_by_max_depth() {
        let mut nodes = sample_tree();
        filter_tree(&mut nodes, &UiTreeQuery { max_depth: Some(0), ..Default::default() });
        assert_eq!(nodes.len(), 2);
        assert!(nodes.iter().all(|n| n.children.is_empty()));
    }

    #[test]
    fn check_handle_rejects_mismatch_and_stale() {
        let fresh = make_id("linux", "n1", 1);
        assert!(check_handle(&fresh, "linux", 1).is_ok());
        assert!(matches!(
            check_handle(&make_id("windows", "n1", 1), "linux", 1),
            Err(UiAutomationError::Platform(_))
        ));
        assert!(matches!(
            check_handle(&make_id("linux", "n1", 0), "linux", 1),
            Err(UiAutomationError::StaleHandle(_))
        ));
    }
}
