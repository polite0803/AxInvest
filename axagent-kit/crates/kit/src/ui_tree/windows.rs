// SPDX-License-Identifier: AGPL-3.0-only

//! Windows UI Automation（UIA）实现。
//!
//! 通过 PowerShell 调用 .NET `UIAutomationClient`（Windows 内置），读取语义树并输出
//! JSON；语义动作同样通过 UIA Pattern 执行。全程不读取屏幕像素、不发送坐标点击。
//! UIA 在 PowerShell 进程内自然运行于 STA 环境，无需在 Rust 侧管理 COM apartment。

use std::sync::atomic::{AtomicU64, Ordering};

use axagent_harness::ui_tree::{
    UiActionKind, UiAutomationError, UiKeyModifier, UiNode, UiNodeId, UiScrollIntent,
    UiTreeProvider, UiTreeQuery, UiTreeSnapshot,
};

use super::{check_handle, filter_tree, make_id, now_millis};

const BACKEND: &str = "windows";

pub struct WindowsUiTreeProvider {
    generation: AtomicU64,
}

impl Default for WindowsUiTreeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsUiTreeProvider {
    pub fn new() -> Self {
        Self { generation: AtomicU64::new(0) }
    }

    fn check(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        check_handle(id, BACKEND, self.generation.load(Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
impl UiTreeProvider for WindowsUiTreeProvider {
    async fn query(&self, query: UiTreeQuery) -> Result<UiTreeSnapshot, UiAutomationError> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut nodes = run_query_script(&query).await?;
        filter_tree(&mut nodes, &query);
        restamp_generation(&mut nodes, generation);
        let roots = nodes.iter().take(1).map(|n| n.id.clone()).collect();
        Ok(UiTreeSnapshot { nodes, roots, captured_at: now_millis() })
    }

    async fn invoke(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        self.check(id)?;
        run_action_script(&id.raw_id, "invoke", None)
    }

    async fn set_value(&self, id: &UiNodeId, value: &str) -> Result<(), UiAutomationError> {
        self.check(id)?;
        run_action_script(&id.raw_id, "set_value", Some(value))
    }

    async fn focus(&self, id: &UiNodeId) -> Result<(), UiAutomationError> {
        self.check(id)?;
        run_action_script(&id.raw_id, "focus", None)
    }

    async fn perform(
        &self,
        id: &UiNodeId,
        action: UiActionKind,
        _scroll_intent: Option<UiScrollIntent>,
    ) -> Result<(), UiAutomationError> {
        self.check(id)?;
        let action = match action {
            UiActionKind::SetValue => {
                return Err(UiAutomationError::ActionNotSupported {
                    action: "set_value".into(),
                    detail: "请使用 set_value 方法".into(),
                });
            },
            UiActionKind::Invoke => "invoke",
            UiActionKind::Focus => "focus",
            UiActionKind::Toggle => "toggle",
            UiActionKind::Select => "select",
            UiActionKind::Expand => "expand",
            UiActionKind::Collapse => "collapse",
            UiActionKind::ScrollIntoView | UiActionKind::Scroll => "scroll",
        };
        run_action_script(&id.raw_id, action, None)
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

/// 查询脚本：按 UIA 控件类型遍历语义树，输出含 runtimeId（句柄）/role/name/value 的 JSON。
async fn run_query_script(query: &UiTreeQuery) -> Result<Vec<UiNode>, UiAutomationError> {
    let script = r#"
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::RootElement
$elements = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
$results = @()
foreach ($el in $elements) {
    try {
        $rid = $el.GetRuntimeId() -join '.'
        $value = $null
        try { $value = [string]$el.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value } catch {}
        $results += @{
            runtimeId = $rid
            role = $el.Current.ControlType.ProgrammaticName
            name = $el.Current.Name
            value = $value
            app = $el.Current.ClassName
            offscreen = $el.Current.IsOffscreen
        }
        if ($results.Count -ge 500) { break }
    } catch {}
}
$results | ConvertTo-Json -Compress -Depth 5
"#;
    let output = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
        .await
        .map_err(|e| UiAutomationError::Platform(format!("启动 PowerShell 失败: {e}")))?;
    if !output.status.success() {
        return Err(UiAutomationError::Platform(format!(
            "UIA 查询失败: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let json = String::from_utf8_lossy(&output.stdout);
    let mut raw: Vec<serde_json::Value> = serde_json::from_str(json.trim()).unwrap_or_default();

    // 应用名过滤（在 Rust 侧做，避免 PowerShell 脚本注入风险）。
    let app_filter = query.application.as_deref();
    if let Some(app) = app_filter {
        raw.retain(|v| v["app"].as_str().unwrap_or_default().contains(app));
    }
    let include_offscreen = query.include_offscreen;
    if !include_offscreen {
        raw.retain(|v| v["offscreen"].as_bool().unwrap_or(false) == false);
    }

    Ok(raw
        .into_iter()
        .map(|v| {
            let name = v["name"].as_str().unwrap_or_default().to_string();
            let role = normalize_role(v["role"].as_str().unwrap_or("ControlType.Custom"));
            let value = v["value"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
            let app = v["app"].as_str().unwrap_or_default().to_string();
            let mut actions =
                vec![UiActionKind::Focus, UiActionKind::ScrollIntoView, UiActionKind::Invoke];
            if value.is_some() {
                actions.push(UiActionKind::SetValue);
            }
            UiNode {
                id: make_id(BACKEND, v["runtimeId"].as_str().unwrap_or("0"), 0),
                role,
                name,
                value,
                description: None,
                states: Vec::new(),
                supported_actions: actions,
                application: app.clone(),
                window_title: Some(app),
                bounds: None,
                children: Vec::new(),
            }
        })
        .collect())
}

/// 动作脚本：按 runtimeId 找回 UIA 元素并执行 Pattern 动作。
fn run_action_script(
    runtime_id: &str,
    action: &str,
    value: Option<&str>,
) -> Result<(), UiAutomationError> {
    // runtimeId 是脚本自身生成的点分十进制串，只含数字和点，杜绝脚本注入。
    if !runtime_id.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(UiAutomationError::StaleHandle(runtime_id.to_string()));
    }
    let value_literal = match value {
        Some(v) => format!("'{}'", v.replace('\'', "''")),
        None => String::new(),
    };
    let action_code = match action {
        "invoke" => "$el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()".to_string(),
        "set_value" => format!(
            "$el.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue({value_literal})"
        ),
        "focus" => "$el.SetFocus()".to_string(),
        "toggle" => "$el.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()".to_string(),
        "select" => "$el.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()".to_string(),
        "expand" => "$el.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()".to_string(),
        "collapse" => "$el.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern).Collapse()".to_string(),
        "scroll" => "$el.GetCurrentPattern([System.Windows.Automation.ScrollItemPattern]::Pattern).ScrollIntoView()".to_string(),
        other => {
            return Err(UiAutomationError::ActionNotSupported {
                action: other.into(),
                detail: runtime_id.to_string(),
            });
        }
    };
    let script = format!(
        r#"Add-Type -AssemblyName UIAutomationClient
$rid = @({parts}) | ForEach-Object {{ [int]$_ }}
$cond = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::RuntimeIdProperty, $rid)
$el = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
if ($null -eq $el) {{ exit 2 }}
{action_code}
"#,
        parts = runtime_id.split('.').map(|p| p.to_string()).collect::<Vec<_>>().join(","),
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| UiAutomationError::Platform(format!("执行 UIA 动作失败: {e}")))?;
    if output.status.code() == Some(2) {
        return Err(UiAutomationError::StaleHandle(runtime_id.to_string()));
    }
    if !output.status.success() {
        return Err(UiAutomationError::Platform(format!(
            "UIA 动作失败: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn normalize_role(raw: &str) -> String {
    raw.strip_prefix("ControlType.").unwrap_or(raw).to_ascii_lowercase()
}

fn restamp_generation(nodes: &mut [UiNode], generation: u64) {
    for node in nodes {
        node.id.session_gen = generation;
        restamp_generation(&mut node.children, generation);
    }
}
