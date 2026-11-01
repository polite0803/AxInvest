// SPDX-License-Identifier: AGPL-3.0-only
//! 插件类型定义 —— 清单、配置、权限、安装源等。
//!
//! 部分 helper 常量/import 仅在测试或可选特性中使用，因此本文件放宽 dead_code 检查。

#![allow(dead_code, unused_imports)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use axagent_harness::{NpmRegistryService, parse_npm_package_spec};

use crate::manager::PluginError;
use crate::sandbox::{SandboxConfig, apply_env_to_command, check_subprocess_permission};

// 插件市场常量不再本地定义（AGENTS.md 禁区 12）：权威源在 core.rs。
// 本文件只引用实际用到的 4 个市场标识；原第 5~9 个（settings/installed/plugin.json/相对路径/SKILL.md）
// 在本文件从未被使用，属死定义，一并移除。
use crate::core::{
    BUILTIN_MARKETPLACE, BUNDLED_MARKETPLACE, EXTERNAL_MARKETPLACE, OPENCLAW_MARKETPLACE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginKind {
    Builtin,
    Bundled,
    External,
    OpenClaw,
}

impl Display for PluginKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => write!(f, "builtin"),
            Self::Bundled => write!(f, "bundled"),
            Self::External => write!(f, "external"),
            Self::OpenClaw => write!(f, "openclaw"),
        }
    }
}

impl PluginKind {
    #[must_use]
    pub fn marketplace(self) -> &'static str {
        match self {
            Self::Builtin => BUILTIN_MARKETPLACE,
            Self::Bundled => BUNDLED_MARKETPLACE,
            Self::External => EXTERNAL_MARKETPLACE,
            Self::OpenClaw => OPENCLAW_MARKETPLACE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub kind: PluginKind,
    pub source: String,
    pub default_enabled: bool,
    pub root: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginHooks {
    #[serde(rename = "PreToolUse", default)]
    pub pre_tool_use: Vec<String>,
    #[serde(rename = "PostToolUse", default)]
    pub post_tool_use: Vec<String>,
    #[serde(rename = "PostToolUseFailure", default)]
    pub post_tool_use_failure: Vec<String>,
}

impl PluginHooks {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pre_tool_use.is_empty()
            && self.post_tool_use.is_empty()
            && self.post_tool_use_failure.is_empty()
    }

    #[must_use]
    pub fn merged_with(&self, other: &Self) -> Self {
        let mut merged = self.clone();
        merged.pre_tool_use.extend(other.pre_tool_use.iter().cloned());
        merged.post_tool_use.extend(other.post_tool_use.iter().cloned());
        merged.post_tool_use_failure.extend(other.post_tool_use_failure.iter().cloned());
        merged
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLifecycle {
    #[serde(rename = "Init", default)]
    pub init: Vec<String>,
    #[serde(rename = "Shutdown", default)]
    pub shutdown: Vec<String>,
}

impl PluginLifecycle {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.init.is_empty() && self.shutdown.is_empty()
    }
}

/// 仪表盘面板的落位区域（合流自 rt-dashboard 的强类型声明，见 `PLAN-plugin-gap-closure.md` §3）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginDashboardPanelPosition {
    #[default]
    Main,
    Sidebar,
    Header,
    Footer,
}

/// 仪表盘面板尺寸。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginDashboardPanelSize {
    Small,
    #[default]
    Medium,
    Large,
    #[serde(rename = "fullWidth")]
    FullWidth,
}

/// 插件贡献的仪表盘面板声明（`dashboard_panels`，manifest 唯一权威声明结构）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDashboardPanel {
    pub id: String,
    pub title: String,
    pub component_name: String,
    #[serde(default)]
    pub position: PluginDashboardPanelPosition,
    #[serde(default)]
    pub size: PluginDashboardPanelSize,
    #[serde(default)]
    pub props: HashMap<String, Value>,
    /// 前端入口标识（合流吸收 rt-dashboard `frontend_entry`；挂载消费点待前端贡献点立项）。
    #[serde(default)]
    pub frontend_entry: Option<String>,
}

/// 仪表盘面板清单条目 —— `dashboard_list_plugins` 门面的返回形状
/// （PluginManager 是权威，本结构只是其清单的只读投影）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardPluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
    pub enabled: bool,
    pub panels: Vec<PluginDashboardPanel>,
}

/// `plugin_read_panel_asset` 的返回形状：面板沙箱 iframe 的内容文本。
///
/// 只应交给 `sandbox="allow-scripts"` 的 iframe，**不得**进主文档执行
/// （见 `PLAN-dashboard-consolidation.md` §6.4 变体 C 的隔离前提）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPanelAsset {
    pub plugin_id: String,
    pub panel_id: String,
    /// 相对插件安装目录的入口路径（已含扩展名白名单与 canonicalize 校验）。
    pub entry: String,
    pub source: String,
}

/// L1 功能包逐页声明的装载形态 —— `PLAN-shell-plus-plugins.md` §0.12.5 裁定 3 的载体。
///
/// 两个取值各自对应一条**已在仓里**的装载通道，本批不新造第三条：
/// - `bundle`：插件自带的前端产物（打包 JS/HTML）。`entry` 是**相对插件安装目录**的资产路径，
///   由宿主解析成绝对路径后经 asset 协议装载（允许根见 `tauri.conf.json` 的
///   `assetProtocol.scope`，只放行插件安装目录）。适合重交互页（股票图表类）。
/// - `schema`：页面内容是一张动态 UI Schema。`entry` 是 **schema id**，不是文件路径，
///   走 `DynamicPageViewer` + `dynamicUIStore`（schema 由插件以 `origin="plugin"`、
///   `owner_id = pluginId` 注入，卸载撤销面见 `commands/dynamic_ui.rs::revoke_schemas_owned_by`）。
///   适合表单 / 列表类页面。
///
/// serde `rename_all = "lowercase"` 与 [`PluginDashboardPanelPosition`] 同形。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginPageLoading {
    Bundle,
    Schema,
}

impl PluginPageLoading {
    /// 清单词汇表（错误文案与 [`Self::parse`] 同源，勿在别处另写字面串）。
    pub const LABELS: [&'static str; 2] = ["bundle", "schema"];

    /// 本变体的清单字面量（与 serde `rename_all = "lowercase"` 的序列化值一致）。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bundle => "bundle",
            Self::Schema => "schema",
        }
    }

    /// 解析 `plugin.json` 声明的 `loading` 值。
    ///
    /// 只接受精确小写（与 [`PluginPermission::parse`]、[`PluginToolPermission::parse`] 同口径，
    /// 不做大小写宽容）；不在词汇表内返回 `None`，由 `build_manifest_pages` 报清单校验错误。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "bundle" => Some(Self::Bundle),
            "schema" => Some(Self::Schema),
            _ => None,
        }
    }
}

/// L1 功能包逐页声明的清单条目（`plugin.json` 的 `pages[]`，§0.10.2 T3）。
///
/// 形状与 `dashboard_panels` 的 [`PluginDashboardPanel`] 同族，共用两条既有约定：
/// 1. **宿主绑定寻址** —— 前端只传 `(pluginId, pageId)`，路径一律由后端按清单解析
///    （见 `PluginManager::resolve_page_asset`）；声明里的 `entry` 是相对定位符，
///    前端拿它拼路径即为违规。
/// 2. **声明面只此一份** —— `plugins` 是面板/页面声明的唯一权威（AGENTS.md 附登记区），
///    不得另建平行 manifest 或第二份路由表。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPage {
    /// 页面标识（宿主 tab / 路由的键），在同一插件内必须唯一。
    pub id: String,
    /// 显示标题。**不做宿主侧 i18n** —— 页面文案归插件产物自己负责。
    pub title: String,
    /// 内容定位符，语义由 [`Self::loading`] 决定：
    /// `bundle` ⇒ 相对插件安装目录的资产路径（扩展名走资产白名单）；
    /// `schema` ⇒ 动态 UI Schema 的 id（不做资产形状校验）。
    pub entry: String,
    /// 装载形态。**必填**：未声明即清单校验错误 —— §0.12.5 裁定 3「不许混用却不声明」的落点。
    pub loading: PluginPageLoading,
}

/// `plugin.json` 中 `pages[]` 的**未校验**形态。
///
/// 照抄 [`RawPluginToolManifest`] 的先例：枚举字段在载入期收成字符串，好让
/// 「缺失 / 非法值」都走**清单校验错误**（`EmptyEntryField` / `InvalidPageLoading`），
/// 而不是让整份 `plugin.json` 反序列化失败 —— 后者只能报出 serde 的行列号，
/// 指不出是哪一页、哪个键。校验与转写见 `manager.rs::build_manifest_pages`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPluginPage {
    pub id: String,
    pub title: String,
    pub entry: String,
    /// `None` = 键缺失（未声明）；`Some("")` = 声明了空值。两者都按「未声明」处理。
    #[serde(default)]
    pub loading: Option<String>,
}

/// `plugin_resolve_page_asset` 的返回形状：宿主绑定的 `(pluginId, pageId)` 解析出的
/// **绝对资产路径**（canonicalize 过，且已校验落在插件安装目录与 asset 协议作用域之内）。
///
/// ⚠ 本结构只回答「这个页面声明的内容在哪」，`loading = "schema"` 的页面**没有**文件资产，
/// 解析时直接报错，不返回空路径。前端拿到 `absolute_path` 后应交给 Tauri 的
/// `convertFileSrc` 生成 asset URL，**不得**自行拼接 `~/.axagent/plugins/installed/…`
/// （那条链与 `external_dirs` 发现形态对不上，见 `manager.rs::resolve_page_asset` 文档）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPageAssetLocation {
    pub plugin_id: String,
    pub page_id: String,
    /// 清单里声明的相对定位符，原样回带**仅供展示与排错**，不参与路径构造。
    pub entry: String,
    pub loading: PluginPageLoading,
    /// canonicalize 后的绝对路径（Windows 已去掉 `\\?\` verbatim 前缀，见
    /// `manager.rs::path_for_asset_protocol`）。
    pub absolute_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    /// 作者（合流吸收 rt-dashboard `DashboardPluginManifest.author`）。
    #[serde(default)]
    pub author: Option<String>,
    pub permissions: Vec<PluginPermission>,
    #[serde(rename = "defaultEnabled", default)]
    pub default_enabled: bool,
    #[serde(default)]
    pub hooks: PluginHooks,
    #[serde(default)]
    pub lifecycle: PluginLifecycle,
    #[serde(default)]
    pub tools: Vec<PluginToolManifest>,
    #[serde(default)]
    pub commands: Vec<PluginCommandManifest>,
    #[serde(default)]
    pub scenarios: Vec<String>,
    pub mcp_servers: Vec<PluginMcpServer>,
    pub skills: Vec<PluginSkillEntry>,
    pub agents: Vec<PluginAgentDefInternal>,
    #[serde(default)]
    pub dashboard_panels: Vec<PluginDashboardPanel>,
    /// L1 功能包逐页声明（§0.10.2 T3）；缺省 = 该包不贡献页面。
    ///
    /// 与 `dashboard_panels` 一样是本 crate 内该表面的**唯一**声明权威：
    /// 前端只做只读投影，不得另建路由表或平行 manifest。
    #[serde(default)]
    pub pages: Vec<PluginPage>,
    #[serde(default)]
    pub dependencies: Vec<PluginDependency>,
    #[serde(default)]
    pub integrity: Option<PluginIntegrity>,
    /// 插件声明的能力（P3 外部插件注册：启用时注册到能力注册表，禁用/卸载时可逆回滚）。
    #[serde(default)]
    pub capabilities: Vec<PluginCapabilityDecl>,
    /// B 层 worker 进程声明（PLAN §5.3）；缺省 = 不启动子进程，只贡献声明式资产。
    #[serde(default)]
    pub worker: Option<PluginWorkerDecl>,
}

/// B 层插件 worker 进程声明（PLAN §5.3「声明获取」/ §10.5-2「action 回流」）。
///
/// 声明本字段的插件在**启用时** spawn 一个长驻子进程，双方以长度前缀 JSON 帧通信
/// （协议在 `crates/plugin-proto`）；未声明则只贡献声明式资产
/// （hooks / tools / MCP / 技能 / 能力声明），不启动任何进程。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginWorkerDecl {
    /// worker 可执行文件路径，**必须相对插件安装目录**。
    ///
    /// 绝对路径与 `..` 逃逸一律拒绝：安装目录是插件的信任边界，
    /// 允许绝对路径等于允许插件把自己指向任意系统程序。
    pub program: String,
    /// 传给 worker 的命令行参数。
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDependency {
    pub plugin_name: String,
    pub min_version: Option<String>,
}

/// 插件声明的能力（P3 外部插件注册）。
///
/// 脚本插件无法跨进程提供 Rust trait 对象，因此以「声明」形式描述插件
/// 提供的能力接缝；`PluginManager` 在启用插件时将这些声明以
/// `CapabilityOrigin::ExternalPlugin` 注册进能力注册表，禁用 / 卸载时可逆回滚。
///
/// 扩展的检索元数据字段（`name`/`kind`/`domain`/`tags`/`visibility`/`discoverable`/
/// `evolvable`）用于把声明映射为能力护照（`CapabilityPassportDto`），接入能力发现。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCapabilityDecl {
    /// 能力接缝 ID（如 `"platform.adapter.telegram"`、`"tool.set.myplugin"`）。
    pub seam: String,
    /// 能力类型标识（如 `"platform_adapter"`、`"tool_set"`）。
    #[serde(default)]
    pub capability_type: String,
    /// 该能力的执行 op（PLAN §5.4 映射表的 `op` 列）。
    ///
    /// 单 op 接缝（如 `workflow.sandbox` → `"execute"`）由宿主门面按此名转发；
    /// 多 op 接缝（`session.log.invariant` / `platform.adapter`）的 op 词汇由接缝固定，
    /// 本字段仅作声明标记、不参与路由（见 `worker.rs` 的远程门面说明）。
    #[serde(default)]
    pub op: String,
    /// 契约版本（默认 `"1.0"`）。
    #[serde(default = "default_capability_version")]
    pub version: String,
    /// 人类可读描述。
    #[serde(default)]
    pub description: String,
    /// 护照显示名（默认回退到 `seam`）。
    #[serde(default)]
    pub name: String,
    /// 能力载体类型（`tool`/`workflow`/`knowledge_base`/`agent`/`skill`），映射 `CapabilityKind`。
    #[serde(default)]
    pub kind: String,
    /// 能力域（如 `general`/`finance`/`automation`），映射 `CapabilityDomain`。
    #[serde(default)]
    pub domain: String,
    /// 检索标签（增强能力发现匹配）。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 负面场景（防止误匹配）。
    #[serde(default)]
    pub negative_scenarios: Vec<String>,
    /// 可见性（`public`/`system_only`），映射 `Visibility`。
    #[serde(default)]
    pub visibility: String,
    /// 是否参与能力发现（默认 true；false 时仅注册运行时能力注册表，不进发现索引）。
    #[serde(default = "default_discoverable")]
    pub discoverable: bool,
    /// 可进化性（`none`/`local`/`derived`），映射 `CapabilityEvolvability`。
    #[serde(default)]
    pub evolvable: String,
}

fn default_discoverable() -> bool {
    true
}

fn default_capability_version() -> String {
    "1.0".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginIntegrity {
    pub algorithm: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginMcpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSkillEntry {
    pub name: String,
    pub path: String,
}

/// 插件内部 Agent 定义（反序列化后转换为 agent_provider::PluginAgentDef）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginAgentDefInternal {
    pub agent_type: String,
    pub description: String,
    pub tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub model: Option<String>,
    pub background: bool,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPermission {
    Read,
    Write,
    Execute,
    FileSystemRead,
    FileSystemWrite,
    NetworkAccess,
    SubprocessExecution,
    ClipboardAccess,
    NotificationAccess,
}

impl PluginPermission {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Execute => "execute",
            Self::FileSystemRead => "file_system_read",
            Self::FileSystemWrite => "file_system_write",
            Self::NetworkAccess => "network_access",
            Self::SubprocessExecution => "subprocess_execution",
            Self::ClipboardAccess => "clipboard_access",
            Self::NotificationAccess => "notification_access",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "execute" => Some(Self::Execute),
            "file_system_read" => Some(Self::FileSystemRead),
            "file_system_write" => Some(Self::FileSystemWrite),
            "network_access" => Some(Self::NetworkAccess),
            "subprocess_execution" => Some(Self::SubprocessExecution),
            "clipboard_access" => Some(Self::ClipboardAccess),
            "notification_access" => Some(Self::NotificationAccess),
            _ => None,
        }
    }

    #[must_use]
    pub fn implies_read(&self) -> bool {
        matches!(
            self,
            Self::Read
                | Self::FileSystemRead
                | Self::Write
                | Self::FileSystemWrite
                | Self::Execute
                | Self::SubprocessExecution
        )
    }

    #[must_use]
    pub fn implies_write(&self) -> bool {
        matches!(
            self,
            Self::Write | Self::FileSystemWrite | Self::Execute | Self::SubprocessExecution
        )
    }

    #[must_use]
    pub fn implies_execute(&self) -> bool {
        matches!(self, Self::Execute | Self::SubprocessExecution)
    }
}

impl AsRef<str> for PluginPermission {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginToolManifest {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub required_permission: PluginToolPermission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginToolPermission {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl PluginToolPermission {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read-only" => Some(Self::ReadOnly),
            "workspace-write" => Some(Self::WorkspaceWrite),
            "danger-full-access" => Some(Self::DangerFullAccess),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginToolDefinition {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// 插件声明的命名命令（`plugin.json` 的 `commands[]`）。
///
/// `input_schema` / `required_permission` 是**宿主侧授权门槛的声明面**，不是执行隔离能力
/// （见 `manager.rs::execute_plugin_command` 的文档：沙箱只做 `subprocess_execution`
/// 门槛 + ENV 白名单，不约束子进程实际能写什么）。两者都是**加法**且带 serde 默认值：
/// 老清单里没有这两个键时按「未声明」处理，行为与添加本字段之前逐字节一致。
///
/// `required_permission` 故意用 `Option<String>` 而非 `PluginToolPermission`：
/// 非法值必须走**清单校验错误**（`InvalidCommandRequiredPermission`），
/// 不能让整份 `plugin.json` 反序列化失败 —— 照抄 tool 侧 `RawPluginToolManifest`
/// 同名字段的先例（见本文件 `RawPluginToolManifest::required_permission`）。
///
/// ⚠ `input_schema` 是 `serde_json::Value`（非 `Eq`），故本结构只能 derive `PartialEq`；
/// 连带 `core::PluginSummary` 也必须去掉 `Eq`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginCommandManifest {
    pub name: String,
    pub description: String,
    pub command: String,
    /// 命令输入的 JSON Schema（宿主侧只做最小形状校验：input 必须是 object，
    /// `required` 列出的键必须存在；其余关键字不校验）。
    ///
    /// 未声明 ⇒ 空 object `{}`，等价于「不约束输入形状」（今天的既有行为）。
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Option<Value>,
    /// 执行该命令要求的宿主侧授权档位标签（`read-only` / `workspace-write` / `danger-full-access`）。
    ///
    /// 未声明 ⇒ `ReadOnly`。判定用**具体权限名**（`write` / `file_system_write` / `execute`），
    /// 不走 `PluginPermission::implies_*` —— 后者把 `SubprocessExecution` 判为满足，
    /// 用它做门槛必然恒过（详见 `manager.rs::check_command_gate`）。
    #[serde(rename = "requiredPermission", default)]
    pub required_permission: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPluginMcpServer {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPluginSkillEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPluginAgentDef {
    #[serde(rename = "agentType")]
    pub agent_type: String,
    pub description: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(rename = "disallowedTools", default)]
    pub disallowed_tools: Vec<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub background: bool,
    #[serde(rename = "systemPrompt")]
    pub system_prompt: Option<String>,
}

/// `plugin.json` 的 `author` 容错解析：**字符串与对象两种写法都收**。
///
/// Claude Code 的清单契约（即 `.claude-plugin/plugin.json`）允许 `author` 写成对象
/// （`{"name": …, "email": …, "url": …}`），而本仓原先把它声明为 `Option<String>` ⇒
/// 遇到对象写法 serde 直接报 `invalid type: map, expected a string`，**整份清单加载失败**。
/// 实测（2026-09-30）：`~/.agents/skills/connect-apps-plugin/.claude-plugin/plugin.json`
/// 的 `author` 就是对象，该插件因此永远进不了注册表（`Skill load failure` WARN）。
///
/// 判据：`author` 在加载链上**只用于展示**（`PluginMetadata` 甚至不携带它），
/// 不该成为「能不能加载」的门 —— 故这里收敛两种写法：优先 `name`，退化到 `email`，
/// 都没有则记 `None`；数字 / 布尔 / 数组等其余形态同样按「未声明」处理。
fn deserialize_author_lenient<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name),
        Some(Value::Object(map)) => match (
            map.get("name").and_then(Value::as_str),
            map.get("email").and_then(Value::as_str),
        ) {
            (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
            (Some(name), None) => Some(name.to_string()),
            (None, Some(email)) => Some(email.to_string()),
            (None, None) => None,
        },
        Some(_) => None,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RawPluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default, deserialize_with = "deserialize_author_lenient")]
    pub author: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(rename = "defaultEnabled", default)]
    pub default_enabled: bool,
    #[serde(default)]
    pub hooks: PluginHooks,
    #[serde(default)]
    pub lifecycle: PluginLifecycle,
    #[serde(default)]
    pub tools: Vec<RawPluginToolManifest>,
    #[serde(default)]
    pub commands: Vec<PluginCommandManifest>,
    #[serde(default)]
    pub scenarios: Vec<String>,
    #[serde(default, alias = "mcpServers")]
    pub mcp_servers: Vec<RawPluginMcpServer>,
    #[serde(default)]
    pub skills: Vec<RawPluginSkillEntry>,
    #[serde(default)]
    pub agents: Vec<RawPluginAgentDef>,
    #[serde(default)]
    pub dashboard_panels: Vec<PluginDashboardPanel>,
    /// 逐页声明的未校验形态；必须显式转写进 [`PluginManifest::pages`] ——
    /// serde 默认忽略未知字段，漏了这一步清单里的 `pages` 会被静默丢弃。
    #[serde(default)]
    pub pages: Vec<RawPluginPage>,
    #[serde(default)]
    pub dependencies: Vec<PluginDependency>,
    #[serde(default)]
    pub integrity: Option<PluginIntegrity>,
    #[serde(default)]
    pub capabilities: Vec<PluginCapabilityDecl>,
    /// B 层 worker 进程声明（PLAN §5.3）；缺省 = 不启动子进程。
    ///
    /// 必须显式转写进 [`PluginManifest`]：serde 默认忽略未知字段，漏了这一步
    /// `plugin.json` 里声明的 `worker` 会被静默丢弃，插件永远起不来子进程。
    #[serde(default)]
    pub worker: Option<PluginWorkerDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPluginToolManifest {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(rename = "requiredPermission", default = "default_tool_permission_label")]
    pub required_permission: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginTool {
    plugin_id: String,
    plugin_name: String,
    definition: PluginToolDefinition,
    pub command: String,
    args: Vec<String>,
    required_permission: PluginToolPermission,
    root: Option<PathBuf>,
}

impl PluginTool {
    #[must_use]
    pub fn new(
        plugin_id: impl Into<String>,
        plugin_name: impl Into<String>,
        definition: PluginToolDefinition,
        command: impl Into<String>,
        args: Vec<String>,
        required_permission: PluginToolPermission,
        root: Option<PathBuf>,
    ) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            plugin_name: plugin_name.into(),
            definition,
            command: command.into(),
            args,
            required_permission,
            root,
        }
    }

    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    #[must_use]
    pub fn definition(&self) -> &PluginToolDefinition {
        &self.definition
    }

    #[must_use]
    pub fn required_permission(&self) -> &str {
        self.required_permission.as_str()
    }

    pub fn check_permission(
        &self,
        declared_permissions: &[PluginPermission],
    ) -> Result<(), PluginError> {
        match self.required_permission {
            PluginToolPermission::ReadOnly => {
                if !declared_permissions.iter().any(|p| p.implies_read()) {
                    return Err(PluginError::CommandFailed(format!(
                        "plugin tool `{}` requires read permission, but plugin `{}` only declares {:?}",
                        self.definition.name,
                        self.plugin_id,
                        declared_permissions.iter().map(|p| p.as_str()).collect::<Vec<_>>()
                    )));
                }
                Ok(())
            },
            PluginToolPermission::WorkspaceWrite => {
                if !declared_permissions.iter().any(|p| p.implies_write()) {
                    return Err(PluginError::CommandFailed(format!(
                        "plugin tool `{}` requires workspace-write permission, but plugin `{}` only declares {:?}",
                        self.definition.name,
                        self.plugin_id,
                        declared_permissions.iter().map(|p| p.as_str()).collect::<Vec<_>>()
                    )));
                }
                Ok(())
            },
            PluginToolPermission::DangerFullAccess => {
                if !declared_permissions.iter().any(|p| p.implies_execute()) {
                    return Err(PluginError::CommandFailed(format!(
                        "plugin tool `{}` requires danger-full-access permission, but plugin `{}` only declares {:?} (needs 'execute')",
                        self.definition.name,
                        self.plugin_id,
                        declared_permissions.iter().map(|p| p.as_str()).collect::<Vec<_>>()
                    )));
                }
                Ok(())
            },
        }
    }

    pub fn execute_with_permission_check(
        &self,
        input: &Value,
        declared_permissions: &[PluginPermission],
    ) -> Result<String, PluginError> {
        self.check_permission(declared_permissions)?;
        self.execute(input)
    }

    const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 60;

    pub fn execute(&self, input: &Value) -> Result<String, PluginError> {
        let input_json = input.to_string();
        let mut process = Command::new(&self.command);
        process
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("CLAWD_PLUGIN_ID", &self.plugin_id)
            .env("CLAWD_PLUGIN_NAME", &self.plugin_name)
            .env("CLAWD_TOOL_NAME", &self.definition.name)
            .env("CLAWD_TOOL_INPUT", &input_json);
        // Windows: 隐藏控制台窗口（插件命令可能是 node/python 等控制台程序）
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x08000000);
        }
        if let Some(root) = &self.root {
            process.current_dir(root).env("CLAWD_PLUGIN_ROOT", root.display().to_string());
        }

        let mut child = process.spawn()?;
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write as _;
            stdin.write_all(input_json.as_bytes())?;
        }

        let timeout = Duration::from_secs(Self::DEFAULT_TOOL_TIMEOUT_SECS);
        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let output = child.wait_with_output()?;
                    if status.success() {
                        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        return Err(PluginError::CommandFailed(format!(
                            "plugin tool `{}` from `{}` failed for `{}`: {}",
                            self.definition.name,
                            self.plugin_id,
                            self.command,
                            if stderr.is_empty() {
                                format!("exit status {}", status)
                            } else {
                                stderr
                            }
                        )));
                    }
                },
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(PluginError::CommandFailed(format!(
                            "plugin tool `{}` from `{}` timed out after {}s",
                            self.definition.name,
                            self.plugin_id,
                            timeout.as_secs()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                },
                Err(e) => return Err(PluginError::Io(e)),
            }
        }
    }

    /// 在沙箱约束下执行工具命令。
    ///
    /// 与 [`execute`](Self::execute) 的区别：
    /// 1. 执行前调用 [`check_subprocess_permission`]，未声明 `subprocess_execution`
    ///    权限时直接返回 [`PluginError::PermissionDenied`]，不启动子进程。
    /// 2. 通过 [`apply_env_to_command`] 对子进程 ENV 做白名单过滤，屏蔽
    ///    API Key / Token / Secret 等敏感变量。
    ///
    /// 显式设置的 `CLAWD_PLUGIN_ID` 等专用变量在 `apply_env_to_command` 之后
    /// 写入，不受白名单约束。
    pub fn execute_sandboxed(
        &self,
        input: &Value,
        sandbox: &SandboxConfig,
    ) -> Result<String, PluginError> {
        // 沙箱检查：未声明 subprocess_execution 权限时禁止启动子进程
        check_subprocess_permission(sandbox)?;

        let input_json = input.to_string();
        let mut process = Command::new(&self.command);
        // 沙箱：ENV 白名单过滤（env_clear + 回填白名单变量）
        apply_env_to_command(&mut process, sandbox);
        process
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("CLAWD_PLUGIN_ID", &self.plugin_id)
            .env("CLAWD_PLUGIN_NAME", &self.plugin_name)
            .env("CLAWD_TOOL_NAME", &self.definition.name)
            .env("CLAWD_TOOL_INPUT", &input_json);
        // Windows: 隐藏控制台窗口（插件命令可能是 node/python 等控制台程序）
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x08000000);
        }
        if let Some(root) = &self.root {
            process.current_dir(root).env("CLAWD_PLUGIN_ROOT", root.display().to_string());
        }

        let mut child = process.spawn()?;
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write as _;
            stdin.write_all(input_json.as_bytes())?;
        }

        let timeout = Duration::from_secs(Self::DEFAULT_TOOL_TIMEOUT_SECS);
        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let output = child.wait_with_output()?;
                    if status.success() {
                        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        return Err(PluginError::CommandFailed(format!(
                            "plugin tool `{}` from `{}` failed for `{}`: {}",
                            self.definition.name,
                            self.plugin_id,
                            self.command,
                            if stderr.is_empty() {
                                format!("exit status {}", status)
                            } else {
                                stderr
                            }
                        )));
                    }
                },
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(PluginError::CommandFailed(format!(
                            "plugin tool `{}` from `{}` timed out after {}s",
                            self.definition.name,
                            self.plugin_id,
                            timeout.as_secs()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                },
                Err(e) => return Err(PluginError::Io(e)),
            }
        }
    }
}

fn default_tool_permission_label() -> String {
    "read-only".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginInstallSource {
    LocalPath { path: PathBuf },
    GitUrl { url: String },
    NpmPackage { name: String, version: Option<String> },
    OpenClaw { package_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPluginRecord {
    #[serde(default = "default_plugin_kind")]
    pub kind: PluginKind,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub install_path: PathBuf,
    pub source: PluginInstallSource,
    pub installed_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    /// 安装完成时对**整个安装目录**算出的哈希（trust-on-install 钉死）。
    ///
    /// 为什么不复用 manifest 里的 `PluginIntegrity`：那个字段写在被哈希的目录内，
    /// 而 `hash_plugin_directory` 会把 manifest 自身算进去 ⇒ 自指，任何声明都无法自证。
    /// 锚点必须落在被哈希范围之外，registry 记录正是这样一个外部锚。
    /// `#[serde(default)]` 保证旧 registry（无此字段）仍可读取，读取时按「未钉死」处理。
    #[serde(default)]
    pub integrity: Option<PluginIntegrity>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPluginRegistry {
    #[serde(default)]
    pub plugins: BTreeMap<String, InstalledPluginRecord>,
}

fn default_plugin_kind() -> PluginKind {
    PluginKind::External
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinPlugin {
    pub metadata: PluginMetadata,
    pub hooks: PluginHooks,
    pub lifecycle: PluginLifecycle,
    pub tools: Vec<PluginTool>,
    pub mcp_servers: Vec<PluginMcpServer>,
    pub skills: Vec<PluginSkillEntry>,
    /// 插件声明的权限集合（来自 manifest），用于沙箱 capability 检查。
    pub permissions: Vec<PluginPermission>,
    /// 插件声明的命名命令（阶段3-① 执行分发，`PLAN-plugin-gap-closure.md` §2）。
    pub commands: Vec<PluginCommandManifest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BundledPlugin {
    pub metadata: PluginMetadata,
    pub hooks: PluginHooks,
    pub lifecycle: PluginLifecycle,
    pub tools: Vec<PluginTool>,
    pub mcp_servers: Vec<PluginMcpServer>,
    pub skills: Vec<PluginSkillEntry>,
    /// 插件声明的权限集合（来自 manifest），用于沙箱 capability 检查。
    pub permissions: Vec<PluginPermission>,
    /// 插件声明的命名命令（阶段3-① 执行分发，`PLAN-plugin-gap-closure.md` §2）。
    pub commands: Vec<PluginCommandManifest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalPlugin {
    pub metadata: PluginMetadata,
    pub hooks: PluginHooks,
    pub lifecycle: PluginLifecycle,
    pub tools: Vec<PluginTool>,
    pub mcp_servers: Vec<PluginMcpServer>,
    pub skills: Vec<PluginSkillEntry>,
    /// 插件声明的权限集合（来自 manifest），用于沙箱 capability 检查。
    pub permissions: Vec<PluginPermission>,
    /// 插件声明的命名命令（阶段3-① 执行分发，`PLAN-plugin-gap-closure.md` §2）。
    pub commands: Vec<PluginCommandManifest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenClawPlugin {
    pub metadata: PluginMetadata,
    pub hooks: PluginHooks,
    pub lifecycle: PluginLifecycle,
    pub tools: Vec<PluginTool>,
    pub mcp_servers: Vec<PluginMcpServer>,
    pub skills: Vec<PluginSkillEntry>,
    /// 插件声明的权限集合（来自 manifest），用于沙箱 capability 检查。
    pub permissions: Vec<PluginPermission>,
    /// 插件声明的命名命令（阶段3-① 执行分发，`PLAN-plugin-gap-closure.md` §2）。
    pub commands: Vec<PluginCommandManifest>,
}
