// SPDX-License-Identifier: AGPL-3.0-only

//! macOS Accessibility（AX）实现。
//!
//! 通过 `osascript` 驱动 System Events 的 AX 树遍历（`entire contents`）与语义动作
//! （`click` / `set value` / `set focused`），全程使用角色/名称/值，不读屏幕像素、
//! 不发送坐标点击。System Events 是 macOS 内置 AX 客户端，首次访问会触发
//! 「辅助功能」授权；未授权时 osascript 返回非零退出码，映射为 PermissionDenied。

use std::sync::atomic::{AtomicU64, Ordering};

use axagent_harness::ui_tree::{
    UiActionKind, UiAutomationError, UiKeyModifier, UiNode, UiNodeId, UiScrollIntent,
    UiTreeProvider, UiTreeQuery, UiTreeSnapshot,
};

use super::{check_handle, filter_tree, make_id, now_millis};

const BACKEND: &str = "macos";

pub struct MacosUiTreeProvider {
    generation: AtomicU64,
}

impl Default for MacosUiTreeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MacosUiTreeProvider {
    pub fn new() -> Self {
        Self { generation: AtomicU64::new(0) }
    }

    fn check(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        check_handle(id, BACKEND, self.generation.load(Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
impl UiTreeProvider for MacosUiTreeProvider {
    async fn query(&self, query: UiTreeQuery) -> Result<UiTreeSnapshot, UiAutomationError> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut nodes = run_query_script().await?;
        filter_tree(&mut nodes, &query);
        restamp_generation(&mut nodes, generation);
        let roots =
            nodes.iter().filter(|n| n.window_title.is_some()).map(|n| n.id.clone()).collect();
        Ok(UiTreeSnapshot { nodes, roots, captured_at: now_millis() })
    }

    async fn invoke(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        self.check(id)?;
        let (app, pid, index) = parse_handle(&id.raw_id)?;
        run_action_script(&app, pid, index, "click", None)
    }

    async fn set_value(&self, id: &UiNodeId, value: &str) -> Result<(), UiAutomationError> {
        self.check(id)?;
        let (app, pid, index) = parse_handle(&id.raw_id)?;
        run_action_script(&app, pid, index, "set_value", Some(value))
    }

    async fn focus(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        self.check(id)?;
        let (app, pid, index) = parse_handle(&id.raw_id)?;
        run_action_script(&app, pid, index, "focus", None)
    }

    async fn perform(
        &self,
        id: &UiNodeId,
        action: UiActionKind,
        _scroll_intent: Option<UiScrollIntent>,
    ) -> Result<(), UiAutomationError> {
        self.check(id)?;
        let (app, pid, index) = parse_handle(&id.raw_id)?;
        let action = match action {
            UiActionKind::Invoke
            | UiActionKind::Toggle
            | UiActionKind::Select
            | UiActionKind::Expand
            | UiActionKind::Collapse => "click",
            UiActionKind::ScrollIntoView | UiActionKind::Scroll => "scroll",
            UiActionKind::Focus => "focus",
            UiActionKind::SetValue => {
                return Err(UiAutomationError::ActionNotSupported {
                    action: "set_value".into(),
                    detail: "请使用 set_value 方法".into(),
                });
            },
        };
        run_action_script(&app, pid, index, action, None)
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

/// raw_id 格式：`{appName}\u{1f}{pid}\u{1f}{index}`（app 名仅用于脚本重建，pid+index 定位元素）。
fn parse_handle(raw_id: &str) -> Result<(&str, &str, &str), UiAutomationError> {
    let mut parts = raw_id.splitn(3, '\u{1f}');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(app), Some(pid), Some(index))
            if pid.chars().all(|c| c.is_ascii_digit())
                && index.chars().all(|c| c.is_ascii_digit()) =>
        {
            Ok((app, pid, index))
        },
        _ => Err(UiAutomationError::StaleHandle(raw_id.to_string())),
    }
}

/// 查询脚本：遍历可见进程的窗口 AX 树，输出「应用名|pid|索引|角色|名称|值」逐行文本。
async fn run_query_script() -> Result<Vec<UiNode>, UiAutomationError> {
    let script = r#"
tell application "System Events"
    set output to ""
    repeat with proc in (every process whose visible is true)
        set pName to name of proc
        set pId to unix id of proc
        try
            set winList to every window of proc
            repeat with w in winList
                try
                    set elems to entire contents of w
                    set idx to 0
                    repeat with e in elems
                        set idx to idx + 1
                        try
                            set r to role of e
                        on error
                            set r to "AXUnknown"
                        end try
                        try
                            set n to name of e
                        on error
                            set n to ""
                        end try
                        try
                            set v to value of e
                        on error
                            set v to ""
                        end try
                        set output to output & pName & "|||" & pId & "|||" & idx & "|||" & r & "|||" & n & "|||" & v & linefeed
                    end repeat
                end try
            end repeat
        end try
    end repeat
end tell
return output
"#;
    let output = tokio::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .await
        .map_err(|e| UiAutomationError::Platform(format!("启动 osascript 失败: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(UiAutomationError::PermissionDenied(format!(
            "macOS AX 查询失败（请授权辅助功能）: {}",
            stderr.trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut nodes = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(6, "|||").collect();
        if parts.len() < 6 {
            continue;
        }
        let app = parts[0].trim();
        let pid = parts[1].trim();
        let index = parts[2].trim();
        let role = parts[3].trim();
        let name = parts[4].trim();
        let value = parts[5].trim();
        let raw_id = format!("{app}\u{1f}{pid}\u{1f}{index}");
        let mut actions =
            vec![UiActionKind::Focus, UiActionKind::ScrollIntoView, UiActionKind::Invoke];
        if !value.is_empty() {
            actions.push(UiActionKind::SetValue);
        }
        nodes.push(UiNode {
            id: make_id(BACKEND, &raw_id, 0),
            role: role.to_string(),
            name: name.to_string(),
            value: if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            },
            description: None,
            states: Vec::new(),
            supported_actions: actions,
            application: app.to_string(),
            window_title: if role.contains("window") {
                Some(name.to_string())
            } else {
                None
            },
            bounds: None,
            children: Vec::new(),
        });
    }
    Ok(nodes)
}

/// 动作脚本：按 pid + 索引重建 System Events 元素并执行语义动作。
fn run_action_script(
    app: &str,
    pid: &str,
    index: &str,
    action: &str,
    value: Option<&str>,
) -> Result<(), UiAutomationError> {
    let action_code = match action {
        "click" => "click target".to_string(),
        "set_value" => format!(
            "set value of target to \"{}\"",
            value.unwrap_or("").replace('\\', "\\\\").replace('"', "\\\"")
        ),
        "focus" => "set focused of target to true".to_string(),
        "scroll" => "perform action \"AXScrollToVisible\" of target".to_string(),
        other => {
            return Err(UiAutomationError::ActionNotSupported {
                action: other.into(),
                detail: app.into(),
            });
        },
    };
    let script = format!(
        r#"tell application "System Events"
    set proc to first process whose unix id is {pid}
    set target to item {index} of entire contents of window 1 of proc
    {action_code}
end tell"#,
    );
    let output = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| UiAutomationError::Platform(format!("执行 AX 动作失败: {e}")))?;
    if !output.status.success() {
        return Err(UiAutomationError::Platform(format!(
            "AX 动作失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn restamp_generation(nodes: &mut [UiNode], generation: u64) {
    for node in nodes {
        node.id.session_gen = generation;
        restamp_generation(&mut node.children, generation);
    }
}
