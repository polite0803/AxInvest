// SPDX-License-Identifier: AGPL-3.0-only

//! 无障碍树（UI Automation）契约层 —— computer-use 语义化的统一抽象。
//!
//! 这是「截图 + 坐标点击」迁移到「读无障碍树 + 对语义节点发动作」的权威契约：
//! - 观察层：[`UiTreeProvider::query`] 返回结构化 [`UiTreeSnapshot`]
//!   （role / name / value / states / supported_actions），不再返回截图；
//! - 操作层：[`UiTreeProvider::invoke`] / [`set_value`] / [`focus`] / [`scroll`]
//!   只接受不透明节点句柄 [`UiNodeId`]，任何写操作都不接受坐标；
//! - 键盘层：[`UiTreeProvider::press_key`] 保留纯键盘注入（无坐标参数）。
//!
//! 平台实现由 `axagent-kit`（implementor）提供：Windows UIA / macOS AX /
//! Linux AT-SPI2；本模块零 `axagent-*` 依赖，遵守 foundation 层铁律。
//!
//! 设计约束（见 `PROPOSAL-computer-use-a11y-semantic.md` §3）：
//! - DTO 字段 snake_case + `#[serde(rename_all = "camelCase")]`（禁区13）；
//! - [`UiNode::bounds`] 仅为前端展示用的**可选诊断字段**，任何语义动作拒绝坐标；
//! - [`UiNodeId::session_gen`] 由实现层每次 query 递增，防旧句柄误操作。

use serde::{Deserialize, Serialize};

/// 语义节点的不透明句柄。
///
/// `raw_id` 是平台原始 id（UIA RuntimeId / AXUIElement 指针 / AT-SPI2 bus path）；
/// `session_gen` 由实现层每次 query 递增，操作时应拒绝旧会话的句柄。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct UiNodeId {
    /// 后端标识：`windows` / `macos` / `linux` / `mock`。
    pub backend: String,
    /// 平台原始节点 id（不透明字符串，前端不得解析）。
    pub raw_id: String,
    /// 句柄所属查询代次；实现层操作前须校验与当前代次一致。
    pub session_gen: u64,
}

/// 节点屏幕矩形（**仅诊断展示**，不参与任何语义动作）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct UiBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 语义动作种类（节点 `supported_actions` 与命令入参共用）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "camelCase")]
pub enum UiActionKind {
    Invoke,
    SetValue,
    Focus,
    Toggle,
    Select,
    Expand,
    Collapse,
    ScrollIntoView,
    Scroll,
}

/// 无障碍树节点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct UiNode {
    /// 不透明句柄（含查询代次）。
    pub id: UiNodeId,
    /// 平台角色（如 `button` / `edit` / `window` / `AXTextField`）。
    pub role: String,
    /// 可访问名称（用户可见标签）。
    pub name: String,
    /// 当前值（输入框文本 / 滑块数值等，可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// 详细描述（可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 状态集合（如 `focused` / `selected` / `expanded` / `disabled`）。
    pub states: Vec<String>,
    /// 该节点支持的语义动作（前端据此条件渲染操作按钮）。
    pub supported_actions: Vec<UiActionKind>,
    /// 所属应用名。
    pub application: String,
    /// 所属窗口标题（可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    /// 屏幕矩形，**仅诊断展示**；语义动作禁止使用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<UiBounds>,
    /// 子节点（按平台树顺序）。
    pub children: Vec<UiNode>,
}

/// 一次查询得到的无障碍树快照（扁平表 + 根 id 列表，前端据此拼树）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct UiTreeSnapshot {
    /// 扁平节点表（含所有层级的节点）。
    pub nodes: Vec<UiNode>,
    /// 根节点 id 列表。
    pub roots: Vec<UiNodeId>,
    /// 采集时间戳（Unix 毫秒）。
    pub captured_at: i64,
}

/// 无障碍树查询条件（全部可选；空条件 = 查当前桌面全部应用）。
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "camelCase")]
pub struct UiTreeQuery {
    /// 按应用名过滤（子串匹配，可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    /// 按窗口标题过滤（子串匹配，可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    /// 按角色过滤（精确匹配，可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// 按名称过滤（子串匹配，可空）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_contains: Option<String>,
    /// 最大展开深度（`None` = 不限）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
    /// 是否包含屏幕外 / 最小化窗口的节点。
    pub include_offscreen: bool,
}

/// 滚动语义（语义动作 `Scroll` 的方向参数，无坐标）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "camelCase")]
pub enum UiScrollIntent {
    Up,
    Down,
    Left,
    Right,
}

/// 键盘修饰键（与既有键盘注入语义对齐，无坐标）。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "camelCase")]
pub enum UiKeyModifier {
    Ctrl,
    Alt,
    Shift,
    Meta,
}

/// 无障碍树提供器（三平台实现 + mock 实现共同遵守的契约）。
///
/// 所有方法只接受 [`UiNodeId`] 句柄或纯语义参数，**不接受任何坐标**。
#[async_trait::async_trait]
pub trait UiTreeProvider: Send + Sync {
    /// 查询无障碍树，返回结构化快照。实现层每次调用应递增 `session_gen`。
    async fn query(&self, query: UiTreeQuery) -> Result<UiTreeSnapshot, UiAutomationError>;

    /// 激活节点（按钮 / 菜单项等）。
    async fn invoke(&self, id: &UiNodeId) -> Result<(), UiAutomationError>;

    /// 设置节点值（输入框文本等）。
    async fn set_value(&self, id: &UiNodeId, value: &str) -> Result<(), UiAutomationError>;

    /// 聚焦节点。
    async fn focus(&self, id: &UiNodeId) -> Result<(), UiAutomationError>;

    /// 按语义动作操作节点（Toggle / Select / Expand / Collapse / ScrollIntoView / Scroll）。
    async fn perform(
        &self,
        id: &UiNodeId,
        action: UiActionKind,
        scroll_intent: Option<UiScrollIntent>,
    ) -> Result<(), UiAutomationError>;

    /// 纯键盘注入（无坐标）：按键名 + 修饰键 + 可选文本。
    async fn press_key(
        &self,
        key: &str,
        modifiers: &[UiKeyModifier],
        text: Option<&str>,
    ) -> Result<(), UiAutomationError>;
}

/// UI 自动化错误（按类别供前端 i18n 映射）。
#[derive(Debug, thiserror::Error)]
pub enum UiAutomationError {
    /// 平台不支持（如 Android / 无 GUI 环境）。
    #[error("当前平台不支持无障碍树操作：{0}")]
    Unsupported(String),
    /// 权限未授予（macOS AX / Windows UIA 权限）。
    #[error("无障碍权限未授予：{0}")]
    PermissionDenied(String),
    /// 句柄失效（代次过期 / 节点已销毁）。
    #[error("节点句柄已失效：{0}")]
    StaleHandle(String),
    /// 节点不支持请求的动作。
    #[error("节点不支持动作 {action}：{detail}")]
    ActionNotSupported { action: String, detail: String },
    /// 平台 API 调用失败。
    #[error("无障碍树操作失败：{0}")]
    Platform(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_node() -> UiNode {
        UiNode {
            id: UiNodeId { backend: "mock".into(), raw_id: "n1".into(), session_gen: 1 },
            role: "button".into(),
            name: "确定".into(),
            value: None,
            description: None,
            states: vec!["focused".into()],
            supported_actions: vec![UiActionKind::Invoke, UiActionKind::ScrollIntoView],
            application: "记事本".into(),
            window_title: Some("未命名".into()),
            bounds: Some(UiBounds { x: 1.0, y: 2.0, width: 3.0, height: 4.0 }),
            children: vec![],
        }
    }

    /// DTO 下发前端：所有键必须 camelCase，禁止 snake_case 残留（禁区13）。
    #[test]
    fn dto_serializes_to_camel_case() {
        let snapshot = UiTreeSnapshot {
            nodes: vec![sample_node()],
            roots: vec![UiNodeId { backend: "mock".into(), raw_id: "n1".into(), session_gen: 1 }],
            captured_at: 1_700_000_000_000,
        };
        let value = serde_json::to_value(&snapshot).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("capturedAt"), "顶层键应 camelCase");
        assert!(obj.get("captured_at").is_none(), "不得残留 snake_case 键");
        let node = &obj["nodes"][0];
        for good in [
            "id",
            "role",
            "name",
            "states",
            "supportedActions",
            "application",
            "windowTitle",
            "bounds",
            "children",
        ] {
            assert!(node.get(good).is_some(), "节点键 {good} 缺失");
        }
        for bad in ["window_title", "supported_actions", "raw_id", "session_gen"] {
            assert!(node.get(bad).is_none(), "不得残留 snake_case 键 {bad}");
        }
        assert!(node.get("value").is_none(), "None 字段应被跳过");
        assert!(node.get("description").is_none(), "None 字段应被跳过");
        // enum 值也应为 camelCase。
        assert_eq!(node["supportedActions"][0], "invoke");
        assert_eq!(node["supportedActions"][1], "scrollIntoView");
    }

    /// 错误 display 插值正确（前端按 category 映射 i18n）。
    #[test]
    fn error_display_interpolates_fields() {
        let err = UiAutomationError::ActionNotSupported {
            action: "set_value".into(),
            detail: "按钮不支持设值".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("set_value") && msg.contains("按钮不支持设值"));
    }

    /// mock 全链路：查询 → 按 id 操作 → 陈旧句柄拒绝 → 错误传播。
    #[tokio::test]
    async fn mock_provider_query_act_and_error_propagation() {
        use crate::test_support::MockUiTreeProvider;

        let root_id = UiNodeId { backend: "mock".into(), raw_id: "n1".into(), session_gen: 0 };
        let snapshot = UiTreeSnapshot {
            nodes: vec![UiNode {
                id: root_id.clone(),
                role: "button".into(),
                name: "确定".into(),
                value: None,
                description: None,
                states: vec![],
                supported_actions: vec![UiActionKind::Invoke],
                application: "test".into(),
                window_title: None,
                bounds: None,
                children: vec![],
            }],
            roots: vec![root_id.clone()],
            captured_at: 0,
        };
        let provider = MockUiTreeProvider::new(snapshot);

        // 查询后 id 带当前代次（1）。
        let fresh = provider.query(UiTreeQuery::default()).await.unwrap();
        let fresh_id = &fresh.roots[0].clone();
        assert_eq!(fresh_id.session_gen, 1);
        assert_eq!(provider.session_gen(), 1);

        // 用当前句柄操作成功。
        provider.invoke(fresh_id).await.unwrap();
        provider.set_value(fresh_id, "hello").await.unwrap();
        assert_eq!(provider.calls().len(), 2);

        // 旧句柄（gen=0）被拒绝。
        let err = provider.invoke(&root_id).await.unwrap_err();
        assert!(matches!(err, UiAutomationError::StaleHandle(_)));

        // 注入的下一个错误如实传播。
        provider.set_next_error("平台炸了");
        let err = provider.focus(fresh_id).await.unwrap_err();
        assert!(matches!(err, UiAutomationError::Platform(msg) if msg.contains("平台炸了")));
    }
}
