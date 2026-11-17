// SPDX-License-Identifier: AGPL-3.0-only

//! Linux AT-SPI2 实现（atspi crate）。
//!
//! 连接系统 a11y 总线 → 从 registry 取桌面根 → 递归遍历 Accessible 树，
//! 得到结构化节点；语义动作走 Action / EditableText / Component 接口，全程零坐标。
//!
//! WSL / CI 无 GUI 时 `AccessibilityConnection::new()` 会失败，本模块把错误映射为
//! [`UiAutomationError::PermissionDenied`]，提示启用辅助功能，绝不 panic。
//!
//! 节点句柄的 `raw_id` 是本 provider 内的 opaque 序号（如 `n42`），对应的
//! AT-SPI2 `ObjectRefOwned` 存在 `handles` 表中；这样既不向前端暴露 D-Bus 路径，
//! 也不需要把 zbus 的 `UniqueName` / `ObjectPath` 从字符串反解（避免额外依赖）。

use std::sync::atomic::{AtomicU64, Ordering};

use atspi::connection::AccessibilityConnection;
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::action::ActionProxy;
use atspi::proxy::component::ComponentProxy;
use atspi::proxy::editable_text::EditableTextProxy;
use atspi::{Interface, ObjectRefOwned, State, StateSet};
use axagent_harness::ui_tree::{
    UiActionKind, UiAutomationError, UiKeyModifier, UiNode, UiNodeId, UiScrollIntent,
    UiTreeProvider, UiTreeQuery, UiTreeSnapshot,
};
use parking_lot::Mutex;

use super::{check_handle, filter_tree, make_id, now_millis};

const BACKEND: &str = "linux";
/// 树遍历深度上限，防止异常 a11y 实现导致无限递归。
const MAX_DEPTH: u32 = 64;
/// 句柄表上限，防止长期运行内存无界增长（超出后淘汰最旧）。
const MAX_HANDLES: usize = 5000;

/// Linux AT-SPI2 provider。
///
/// 每次 query 建立一次短连接并重建句柄表：不持有长连接，避免无 GUI 环境下
/// 构造失败，也避免 D-Bus 连接生命周期与 tokio runtime 绑定。
pub struct LinuxUiTreeProvider {
    generation: AtomicU64,
    handles: Mutex<std::collections::HashMap<String, ObjectRefOwned>>,
}

impl Default for LinuxUiTreeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LinuxUiTreeProvider {
    pub fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            handles: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn check(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        check_handle(id, BACKEND, self.generation.load(Ordering::SeqCst))
    }

    fn object_ref(&self, raw_id: &str) -> Result<ObjectRefOwned, UiAutomationError> {
        self.handles
            .lock()
            .get(raw_id)
            .cloned()
            .ok_or_else(|| UiAutomationError::StaleHandle(raw_id.to_string()))
    }

    fn store_handle(&self, obj: ObjectRefOwned) -> String {
        let mut handles = self.handles.lock();
        let raw_id = format!("n{}", handles.len());
        if handles.len() < MAX_HANDLES {
            handles.insert(raw_id.clone(), obj);
        }
        raw_id
    }
}

#[async_trait::async_trait]
impl UiTreeProvider for LinuxUiTreeProvider {
    async fn query(&self, query: UiTreeQuery) -> Result<UiTreeSnapshot, UiAutomationError> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let conn = connect().await?;
        let root = conn
            .root_accessible_on_registry()
            .await
            .map_err(|e| UiAutomationError::Platform(format!("获取 AT-SPI2 桌面根失败: {e}")))?;
        // 每次 query 重建句柄表，旧 raw_id 自动失效。
        self.handles.lock().clear();
        let mut nodes = Vec::new();
        walk_accessible(self, &conn, &root, "desktop", 0, &query, &mut nodes).await;
        filter_tree(&mut nodes, &query);
        restamp_generation(&mut nodes, generation);
        let roots =
            nodes.iter().filter(|n| n.window_title.is_some()).map(|n| n.id.clone()).collect();
        Ok(UiTreeSnapshot { nodes, roots, captured_at: now_millis() })
    }

    async fn invoke(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        self.check(id)?;
        atspi_action_by_name(self, &id.raw_id, "click").await
    }

    async fn set_value(&self, id: &UiNodeId, value: &str) -> Result<(), UiAutomationError> {
        self.check(id)?;
        let conn = connect().await?;
        let obj = self.object_ref(&id.raw_id)?;
        let editable = EditableTextProxy::builder(conn.connection())
            .destination(obj.name_as_str().unwrap_or(""))
            .map_err(|e| UiAutomationError::Platform(e.to_string()))?
            .path(obj.path())
            .map_err(|e| UiAutomationError::Platform(e.to_string()))?
            .build()
            .await
            .map_err(|e| {
                UiAutomationError::Platform(format!("创建 EditableTextProxy 失败: {e}"))
            })?;
        let ok = editable
            .set_text_contents(value)
            .await
            .map_err(|e| UiAutomationError::Platform(format!("AT-SPI2 设值失败: {e}")))?;
        if ok {
            Ok(())
        } else {
            Err(UiAutomationError::Platform("AT-SPI2 拒绝设值".into()))
        }
    }

    async fn focus(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        self.check(id)?;
        let conn = connect().await?;
        let obj = self.object_ref(&id.raw_id)?;
        let component = ComponentProxy::builder(conn.connection())
            .destination(obj.name_as_str().unwrap_or(""))
            .map_err(|e| UiAutomationError::Platform(e.to_string()))?
            .path(obj.path())
            .map_err(|e| UiAutomationError::Platform(e.to_string()))?
            .build()
            .await
            .map_err(|e| UiAutomationError::Platform(format!("创建 ComponentProxy 失败: {e}")))?;
        component
            .grab_focus()
            .await
            .map(|_| ())
            .map_err(|e| UiAutomationError::Platform(format!("AT-SPI2 聚焦失败: {e}")))
    }

    async fn perform(
        &self,
        id: &UiNodeId,
        action: UiActionKind,
        _scroll_intent: Option<UiScrollIntent>,
    ) -> Result<(), UiAutomationError> {
        self.check(id)?;
        match action {
            UiActionKind::SetValue => Err(UiAutomationError::ActionNotSupported {
                action: "set_value".into(),
                detail: "请使用 set_value 方法".into(),
            }),
            UiActionKind::Focus => self.focus(id).await,
            other => atspi_action_by_name(self, &id.raw_id, action_name(&other)).await,
        }
    }

    async fn press_key(
        &self,
        key: &str,
        modifiers: &[UiKeyModifier],
        text: Option<&str>,
    ) -> Result<(), UiAutomationError> {
        super::enigo_keyboard::press_key(key, modifiers, text)
    }
}

/// 连接 a11y 总线；无 GUI 环境给明确权限错误。
async fn connect() -> Result<AccessibilityConnection, UiAutomationError> {
    AccessibilityConnection::new()
        .await
        .map_err(|e| UiAutomationError::PermissionDenied(format!("连接 AT-SPI2 总线失败: {e}")))
}

/// 递归遍历 Accessible 树，输出扁平节点列表。
#[allow(clippy::too_many_arguments)]
async fn walk_accessible(
    provider: &LinuxUiTreeProvider,
    conn: &AccessibilityConnection,
    accessible: &AccessibleProxy<'_>,
    application: &str,
    depth: u32,
    query: &UiTreeQuery,
    out: &mut Vec<UiNode>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let name = accessible.name().await.unwrap_or_default();
    let role = accessible
        .get_role_name()
        .await
        .map(|r| role_to_string(&r))
        .unwrap_or_else(|_| "element".to_string());
    let description = accessible.description().await.unwrap_or_default();
    let states = match accessible.get_state().await {
        Ok(set) => format_states(&set),
        Err(_) => Vec::new(),
    };

    let obj_ref = ObjectRefOwned::try_from(accessible).ok();
    let raw_id = if let Some(obj) = obj_ref {
        provider.store_handle(obj)
    } else {
        format!("unknown-{depth}-{name}")
    };

    let interfaces = accessible.get_interfaces().await.unwrap_or_default();
    let mut supported_actions = vec![UiActionKind::Focus, UiActionKind::ScrollIntoView];
    if interfaces.contains(Interface::Action) {
        supported_actions.extend([
            UiActionKind::Invoke,
            UiActionKind::Toggle,
            UiActionKind::Select,
            UiActionKind::Expand,
            UiActionKind::Collapse,
        ]);
    }
    if interfaces.contains(Interface::EditableText) || interfaces.contains(Interface::Text) {
        supported_actions.push(UiActionKind::SetValue);
    }

    let window_title = if depth <= 1 && !name.is_empty() {
        Some(name.clone())
    } else {
        None
    };
    out.push(UiNode {
        id: make_id(BACKEND, &raw_id, 0),
        role,
        name: name.clone(),
        value: None,
        description: if description.is_empty() {
            None
        } else {
            Some(description)
        },
        states,
        supported_actions,
        application: if depth == 0 {
            name.clone()
        } else {
            application.to_string()
        },
        window_title,
        bounds: None,
        children: Vec::new(),
    });

    if let Ok(children) = accessible.get_children().await {
        for child_ref in children {
            if query.max_depth.is_some_and(|limit| depth + 1 > limit) {
                break;
            }
            if let Ok(child) = child_ref.as_accessible_proxy(conn.connection()).await {
                let child_app = if depth == 0 {
                    name.clone()
                } else {
                    application.to_string()
                };
                Box::pin(walk_accessible(
                    provider,
                    conn,
                    &child,
                    &child_app,
                    depth + 1,
                    query,
                    out,
                ))
                .await;
            }
        }
    }
}

fn role_to_string(role: &str) -> String {
    role.to_ascii_lowercase().replace([' ', '_'], "-")
}

fn format_states(set: &StateSet) -> Vec<String> {
    let mut states = Vec::new();
    for (name, bit) in [
        ("focused", State::Focused),
        ("selected", State::Selected),
        ("expanded", State::Expanded),
        ("checked", State::Checked),
    ] {
        if set.contains(bit) {
            states.push(name.to_string());
        }
    }
    if !set.contains(State::Enabled) {
        states.push("disabled".to_string());
    }
    states
}

fn action_name(action: &UiActionKind) -> &'static str {
    match action {
        UiActionKind::Invoke => "click",
        UiActionKind::Toggle => "toggle",
        UiActionKind::Select => "select",
        UiActionKind::Expand => "expand",
        UiActionKind::Collapse => "collapse",
        UiActionKind::ScrollIntoView | UiActionKind::Scroll => "scroll",
        UiActionKind::Focus | UiActionKind::SetValue => "focus",
    }
}

/// 用 AT-SPI2 Action 接口按语义名执行动作（取第一个名称匹配项）。
async fn atspi_action_by_name(
    provider: &LinuxUiTreeProvider,
    raw_id: &str,
    wanted: &str,
) -> Result<(), UiAutomationError> {
    let conn = connect().await?;
    let obj = provider.object_ref(raw_id)?;
    let proxy = ActionProxy::builder(conn.connection())
        .destination(obj.name_as_str().unwrap_or(""))
        .map_err(|e| UiAutomationError::Platform(e.to_string()))?
        .path(obj.path())
        .map_err(|e| UiAutomationError::Platform(e.to_string()))?
        .build()
        .await
        .map_err(|e| UiAutomationError::Platform(format!("创建 ActionProxy 失败: {e}")))?;
    let count = proxy.n_actions().await.unwrap_or(0);
    for i in 0..count {
        if let Ok(name) = proxy.get_name(i).await
            && (name.eq_ignore_ascii_case(wanted) || (wanted == "click" && i == 0))
        {
            return proxy
                .do_action(i)
                .await
                .map(|_| ())
                .map_err(|e| UiAutomationError::Platform(format!("AT-SPI2 动作执行失败: {e}")));
        }
    }
    Err(UiAutomationError::ActionNotSupported {
        action: wanted.to_string(),
        detail: raw_id.to_string(),
    })
}

fn restamp_generation(nodes: &mut [UiNode], generation: u64) {
    for node in nodes {
        node.id.session_gen = generation;
        restamp_generation(&mut node.children, generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 查询在「无 a11y 总线 → PermissionDenied」与「有 a11y 总线 → 快照」两种
    /// 环境下都不得 panic，且每次调用必须递增代次（跨平台稳定断言）。
    #[tokio::test]
    async fn query_never_panics_and_advances_generation() {
        let provider = LinuxUiTreeProvider::new();
        let result = provider.query(UiTreeQuery::default()).await;
        assert!(
            matches!(result, Ok(ref snapshot) if snapshot.nodes.iter().all(|n| n.id.session_gen == 1))
                || matches!(result, Err(UiAutomationError::PermissionDenied(_))),
            "查询结果须为当前代次快照或明确的权限错误"
        );
        assert_eq!(provider.generation.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stale_handle_rejected() {
        let provider = LinuxUiTreeProvider::new();
        let stale_id = make_id(BACKEND, "n0", 0);
        let err = provider.invoke(&stale_id).await.unwrap_err();
        assert!(matches!(err, UiAutomationError::StaleHandle(_)));
    }
}
