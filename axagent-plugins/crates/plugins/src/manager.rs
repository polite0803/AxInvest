// SPDX-License-Identifier: AGPL-3.0-only
//! 插件管理器：安装/卸载/启用/禁用/查询 + 清单加载与校验。
//!
//! 部分 helper 常量/import 仅在测试或可选特性中使用，因此本文件放宽 dead_code 检查。

#![allow(dead_code, unused_imports)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing;

use axagent_harness::{
    CapabilityPassportDto, EffectHandle, NpmRegistryService, parse_npm_package_spec,
};
use axagent_plugin_proto::{CommandFrame, DEFAULT_COMMAND_TIMEOUT_MS, error_codes};

// 插件市场常量不再本地定义：由下方 `use crate::core::*;` 统一引入（core.rs 为唯一定义点）。
// ⚠ 此处原先的本地定义会**静默遮蔽** glob 导入（编译器不报错），故必须删除才能真正复用同一来源。

use crate::core::*;
use crate::mcp_launcher::McpLauncher;
use crate::sandbox::{
    SandboxConfig, apply_env_to_command, build_sandbox_from_manifest,
    build_sandbox_from_permissions, check_subprocess_permission,
};
use crate::skill_installer::SkillInstaller;
use crate::types::*;
use crate::ui_schema::is_declared_key_shape;
use crate::worker::{LoadedPlugin, PluginWorkerConfig, WorkerError};

#[derive(Debug, Clone)]
pub struct PluginManagerConfig {
    pub config_home: PathBuf,
    pub enabled_plugins: BTreeMap<String, bool>,
    pub external_dirs: Vec<PathBuf>,
    pub install_root: Option<PathBuf>,
    pub registry_path: Option<PathBuf>,
    pub bundled_root: Option<PathBuf>,
}

impl PluginManagerConfig {
    #[must_use]
    pub fn new(config_home: impl Into<PathBuf>) -> Self {
        Self {
            config_home: config_home.into(),
            enabled_plugins: BTreeMap::new(),
            external_dirs: Vec::new(),
            install_root: None,
            registry_path: None,
            bundled_root: None,
        }
    }

    /// 解析「内置插件」根：优先用**应用资源目录**里的 `plugins/bundled`（发布版），
    /// 不存在时退回源码树（开发期）。
    ///
    /// 为什么必须可覆盖：默认值来自 `env!("CARGO_MANIFEST_DIR")` —— 那是**编译机**的源码路径，
    /// 被打进二进制后，用户机器上根本不存在 ⇒ 发布版默认「零内置插件」且不报错
    /// （`discover_plugin_dirs` 对 `NotFound` 返回空表）。传 `resource_dir()` 即可修掉。
    #[must_use]
    pub fn resolve_bundled_root(resource_dir: Option<&Path>) -> PathBuf {
        if let Some(dir) = resource_dir {
            let candidate = dir.join("plugins").join("bundled");
            if candidate.is_dir() {
                return candidate;
            }
        }
        PluginManager::bundled_root()
    }
}

#[derive(Debug)]
pub struct PluginManager {
    config: PluginManagerConfig,
    mcp_launcher: McpLauncher,
    skill_installer: SkillInstaller,
    npm_registry: Option<Arc<dyn NpmRegistryService>>,
    /// 运行时能力注册表（P3 外部插件注册入口）。启用插件时把声明能力注册进去，
    /// 禁用 / 卸载时经 `EffectHandle` 可逆回滚。
    capability_registry: Option<Arc<axagent_harness::CapabilityRegistry>>,
    /// 各插件已注册能力的可逆句柄（键 = 插件 ID，值 = 撤销句柄列表）。
    active_capability_handles: HashMap<String, Vec<EffectHandle>>,
    /// 各插件已注册护照的 capability_id（键 = 插件 ID，值 = 护照 ID 列表）。
    /// 启用时记录，禁用 / 卸载时回滚索引的依据（索引写入由命令层 async 完成）。
    active_passport_ids: HashMap<String, Vec<String>>,
    /// 已载入的 B 层 worker（键 = 插件 ID）。仅 manifest 声明了 `worker` 才有条目。
    ///
    /// 持有 [`LoadedPlugin`] 即同时持有「能力注册句柄」与「子进程」两样东西，
    /// 移除条目 = 能力 LIFO 回滚 + 停进程，故**只能经 [`Self::stop_plugin_worker`] 移除**。
    loaded_workers: HashMap<String, LoadedPlugin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub plugin_id: String,
    pub version: String,
    pub install_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOutcome {
    pub plugin_id: String,
    pub old_version: String,
    pub new_version: String,
    pub install_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginManifestValidationError {
    EmptyField {
        field: &'static str,
    },
    EmptyEntryField {
        kind: &'static str,
        field: &'static str,
        name: Option<String>,
    },
    InvalidPermission {
        permission: String,
    },
    DuplicatePermission {
        permission: String,
    },
    DuplicateEntry {
        kind: &'static str,
        name: String,
    },
    MissingPath {
        kind: &'static str,
        path: PathBuf,
    },
    PathIsDirectory {
        kind: &'static str,
        path: PathBuf,
    },
    InvalidToolInputSchema {
        tool_name: String,
    },
    InvalidToolRequiredPermission {
        tool_name: String,
        permission: String,
    },
    /// 命名命令声明了 `inputSchema` 但不是 JSON object。
    ///
    /// 与 tool 侧 `InvalidToolInputSchema` **分开两个变体**：Display 文案里的
    /// `plugin tool` / `plugin command` 是写死的，复用会让用户去改错的那一份声明。
    InvalidCommandInputSchema {
        command_name: String,
    },
    /// 命名命令声明了 `requiredPermission`，但值不在三档标签之内。
    InvalidCommandRequiredPermission {
        command_name: String,
        permission: String,
    },
    UnsupportedManifestContract {
        detail: String,
    },
    DependencyNotSatisfied {
        plugin_name: String,
        min_version: Option<String>,
    },
    IntegrityCheckFailed {
        algorithm: String,
        expected: String,
        actual: String,
    },
    /// 面板声明的前端资产（`frontendEntry`）形状非法：绝对路径 / 逃逸 / 扩展名不在白名单
    InvalidPanelAsset {
        panel: String,
        detail: String,
    },
    /// 页面声明的 `loading` 缺失或不在词汇表（`bundle` / `schema`）内。
    ///
    /// 与 [`Self::InvalidCommandRequiredPermission`] 同族：都是「枚举型声明字段必须走清单
    /// 校验错误、而不是让整份 manifest 反序列化失败」的先例，错误文案指到**具体页 id**。
    InvalidPageLoading {
        page: String,
        loading: String,
    },
    /// 页面声明的前端资产（`loading = "bundle"` 时的 `entry`）形状非法。
    InvalidPageAsset {
        page: String,
        detail: String,
    },
    /// 任务源声明的 `id` 形状非法（任务 #148）。
    ///
    /// 源 id 会进宿主 `SourceId` 与 `runIdOf(source, id)` 的拼接，带 `:` / 空格 / 大写
    /// 都会破坏「run id 全局唯一」这条不变量 ⇒ 必须在装载期拒，而不是运行期拼出怪键。
    InvalidTaskSourceId {
        source: String,
        detail: String,
    },
    /// 任务源声明的 `command` 不在同一份清单的 `commands[]` 里。
    TaskSourceCommandUnknown {
        source: String,
        command: String,
    },
    /// `ui.json` 读取 / 解析 / 校验失败（插件 UI 贡献的声明文件，任务 `#149` 第 2 步）。
    ///
    /// 与 `pages[]` / `taskSources[]` 的字段级错误分开：这份文件的缺陷是**整份**的
    /// （格式版本、JSON 语法、key 形状），逐条报到字段上会让用户改十遍。
    InvalidUiSchemaDeclaration {
        detail: String,
    },
    /// 页面声明 `loading = "schema"` 时，`entry` 必须是**本插件 `ui.json`** 里声明的 key。
    ///
    /// 今天这条路径零归属校验：`entry` 被前端直接当 schemaId 用，指向谁的 schema 都行
    /// ⇒ 插件可以挂一张宿主/别的插件/用户建的表格页。改动后只能指自己的 key。
    PageSchemaKeyUnknown {
        page: String,
        key: String,
    },
    /// 任务源声明 `detailSchema` 时，值必须是**本插件 `ui.json`** 里声明的 key。
    ///
    /// 与 [`Self::PageSchemaKeyUnknown`] 同源判据：值是 key 而不是行 id，
    /// 宿主用 `ui_schema::row_id` 拼全 ⇒ 跨插件引用在声明面上写不出来。
    TaskSourceSchemaKeyUnknown {
        source: String,
        key: String,
    },
    /// 卡片声明的 `id` 形状非法（任务 #216 前置 ②）。
    ///
    /// 与 [`Self::InvalidTaskSourceId`] 同源判据：id 会进前端注册表键 `${pluginId}:${id}`，
    /// 带 `:` / 空格 / 大写会让「同一张卡只有一种写法」这条不变量失效 ⇒ 装载期就拒。
    InvalidTaskCardId {
        card: String,
        detail: String,
    },
    /// 卡片声明的 `surface` 缺失或不在词汇表（`chat` / `task_card`）内。
    ///
    /// 与 [`Self::InvalidPageLoading`] 同族：枚举型声明字段必须走清单校验错误，
    /// 不许默认兜底到某个 surface（插在界面上错误的位置比不显示更难查）。
    InvalidTaskCardSurface {
        card: String,
        surface: String,
    },
    /// 卡片的 `summaryCommand` 不在同一份清单的 `commands[]` 里。
    TaskCardCommandUnknown {
        card: String,
        command: String,
    },
    /// 卡片的 `detailSchema` 不是**本插件 `ui.json`** 里声明的 key。
    TaskCardSchemaKeyUnknown {
        card: String,
        key: String,
    },
    /// 卡片动作的 `id` 形状非法（同一张卡内动作 id 是宿主按钮的键）。
    InvalidTaskCardActionId {
        card: String,
        action: String,
        detail: String,
    },
    /// 卡片动作引用的 `command` 不在同一份清单的 `commands[]` 里。
    ///
    /// **不允许写成宿主命令名**：这条判据就是「插件只能调自己的命令」在声明面上的落点。
    TaskCardActionCommandUnknown {
        card: String,
        action: String,
        command: String,
    },
    /// 卡片动作声明了 `mutating: true`，但清单 `permissions[]` 没有 `write`。
    ///
    /// 权限判定**只在后端**做（`PluginSummaryDto` 不带 permissions，前端判不了），
    /// 且必须在**装载期**拦：放它装上的话，界面上会出现一颗前端无法判权、后端也不知道
    /// 该不该放行的按钮 —— 静默降级成「点了没反应」或「点了就写」，两种都不可接受。
    TaskCardActionWritePermissionMissing {
        card: String,
        action: String,
    },
}

impl Display for PluginManifestValidationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField { field } => {
                write!(f, "plugin manifest {field} cannot be empty")
            },
            Self::EmptyEntryField { kind, field, name } => match name {
                Some(name) if !name.is_empty() => {
                    write!(f, "plugin {kind} `{name}` {field} cannot be empty")
                },
                _ => write!(f, "plugin {kind} {field} cannot be empty"),
            },
            Self::InvalidPermission { permission } => {
                write!(
                    f,
                    "plugin manifest permission `{permission}` must be one of read, write, execute, file_system_read, file_system_write, network_access, subprocess_execution, clipboard_access, or notification_access"
                )
            },
            Self::DuplicatePermission { permission } => {
                write!(f, "plugin manifest permission `{permission}` is duplicated")
            },
            Self::DuplicateEntry { kind, name } => {
                write!(f, "plugin {kind} `{name}` is duplicated")
            },
            Self::MissingPath { kind, path } => {
                write!(f, "{kind} path `{}` does not exist", path.display())
            },
            Self::PathIsDirectory { kind, path } => {
                write!(f, "{kind} path `{}` must point to a file", path.display())
            },
            Self::InvalidToolInputSchema { tool_name } => {
                write!(f, "plugin tool `{tool_name}` inputSchema must be a JSON object")
            },
            Self::InvalidToolRequiredPermission { tool_name, permission } => write!(
                f,
                "plugin tool `{tool_name}` requiredPermission `{permission}` must be read-only, workspace-write, or danger-full-access"
            ),
            Self::InvalidCommandInputSchema { command_name } => {
                write!(f, "plugin command `{command_name}` inputSchema must be a JSON object")
            },
            Self::InvalidCommandRequiredPermission { command_name, permission } => write!(
                f,
                "plugin command `{command_name}` requiredPermission `{permission}` must be read-only, workspace-write, or danger-full-access"
            ),
            Self::UnsupportedManifestContract { detail } => f.write_str(detail),
            Self::DependencyNotSatisfied { plugin_name, min_version } => match min_version {
                Some(ver) => write!(
                    f,
                    "plugin dependency `{plugin_name}` (min version {ver}) is not satisfied"
                ),
                None => write!(f, "plugin dependency `{plugin_name}` is not installed"),
            },
            Self::IntegrityCheckFailed { algorithm, expected, actual } => {
                write!(
                    f,
                    "plugin integrity check failed ({algorithm}): expected {expected}, got {actual}"
                )
            },
            Self::InvalidPanelAsset { panel, detail } => {
                write!(f, "plugin dashboard panel `{panel}` has an invalid frontendEntry: {detail}")
            },
            Self::InvalidPageLoading { page, loading } => write!(
                f,
                "plugin page `{page}` loading `{loading}` must be one of {}",
                crate::types::PluginPageLoading::LABELS.join(", ")
            ),
            Self::InvalidPageAsset { page, detail } => {
                write!(f, "plugin page `{page}` has an invalid entry: {detail}")
            },
            Self::InvalidTaskSourceId { source, detail } => {
                write!(f, "plugin task source `{source}` has an invalid id: {detail}")
            },
            Self::TaskSourceCommandUnknown { source, command } => write!(
                f,
                "plugin task source `{source}` references command `{command}`, which is not declared in this manifest's commands[]"
            ),
            Self::InvalidUiSchemaDeclaration { detail } => {
                write!(f, "plugin ui declaration is invalid: {detail}")
            },
            Self::PageSchemaKeyUnknown { page, key } => write!(
                f,
                "plugin page `{page}` references UI Schema key `{key}`, which is not declared in this plugin's {file}",
                file = crate::ui_schema::UI_SCHEMA_FILE_NAME
            ),
            Self::TaskSourceSchemaKeyUnknown { source, key } => write!(
                f,
                "plugin task source `{source}` references UI Schema key `{key}`, which is not declared in this plugin's {file}",
                file = crate::ui_schema::UI_SCHEMA_FILE_NAME
            ),
            Self::InvalidTaskCardId { card, detail } => {
                write!(f, "plugin task card `{card}` has an invalid id: {detail}")
            },
            Self::InvalidTaskCardSurface { card, surface } => write!(
                f,
                "plugin task card `{card}` declares surface `{surface}`, which must be one of {}",
                PluginCardSurface::LABELS.join(", ")
            ),
            Self::TaskCardCommandUnknown { card, command } => write!(
                f,
                "plugin task card `{card}` references command `{command}`, which is not declared in this manifest's commands[]"
            ),
            Self::TaskCardSchemaKeyUnknown { card, key } => write!(
                f,
                "plugin task card `{card}` references UI Schema key `{key}`, which is not declared in this plugin's {file}",
                file = crate::ui_schema::UI_SCHEMA_FILE_NAME
            ),
            Self::InvalidTaskCardActionId { card, action, detail } => {
                write!(f, "plugin task card `{card}` action `{action}` has an invalid id: {detail}")
            },
            Self::TaskCardActionCommandUnknown { card, action, command } => write!(
                f,
                "plugin task card `{card}` action `{action}` references command `{command}`, which is not declared in this manifest's commands[]"
            ),
            Self::TaskCardActionWritePermissionMissing { card, action } => write!(
                f,
                "plugin task card `{card}` action `{action}` is declared mutating but the manifest's permissions[] does not include `write`"
            ),
        }
    }
}

#[derive(Debug)]
pub enum PluginError {
    Io(std::io::Error),
    Json(serde_json::Error),
    ManifestValidation(Vec<PluginManifestValidationError>),
    LoadFailures(Vec<PluginLoadFailure>),
    InvalidManifest(String),
    NotFound(String),
    CommandFailed(String),
    /// 插件命令超过宿主等待上限（帧预算 + 收帧宽限）后被放弃等待（任务 #49）。
    ///
    /// 命令帧走**长驻 worker**：插件僵死不再级联卡死宿主调用线程。与 `CommandFailed`
    /// 分开是必须的 —— 前者「没等到」（调用方可按预算重试），后者「跑出了错」
    /// （重试大概率复现），前端对两者的处置相反。
    CommandTimeout(String),
    /// 沙箱权限拦截：插件执行前 capability 检查未通过（路径越权 / 缺少
    /// `subprocess_execution` 权限等）。与 `CommandFailed` 区分以便上层
    /// 做权限引导而非通用错误处理。
    PermissionDenied(String),
    /// 面板资产的完整性锚点缺失：该插件安装时没钉下目录哈希（早于此机制安装，
    /// 或安装当场哈希失败）。用户动作 = 重装一次；**不得**报成「内容被篡改」。
    IntegrityUnpinned(String),
    /// 安装目录内容与安装时钉下的哈希不一致（被改动）。用户动作 = 移除或重装，
    /// 性质是安全拒绝，与「插件本来没声明内容」相反，故两个码分开。
    IntegrityMismatch(String),
    /// 插件已安装冲突 — install 时检测到 plugin_id 已存在,
    /// 调用方应提示用户使用 update 而非 install。
    AlreadyExists(String),
}

impl Display for PluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::ManifestValidation(errors) => {
                for (index, error) in errors.iter().enumerate() {
                    if index > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{error}")?;
                }
                Ok(())
            },
            Self::LoadFailures(failures) => {
                for (index, failure) in failures.iter().enumerate() {
                    if index > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{failure}")?;
                }
                Ok(())
            },
            Self::InvalidManifest(message)
            | Self::NotFound(message)
            | Self::CommandFailed(message)
            | Self::CommandTimeout(message)
            | Self::PermissionDenied(message)
            | Self::IntegrityUnpinned(message)
            | Self::IntegrityMismatch(message)
            | Self::AlreadyExists(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<std::io::Error> for PluginError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for PluginError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl PluginManager {
    #[must_use]
    pub fn new(config: PluginManagerConfig) -> Self {
        let skill_installer = SkillInstaller::new(config.config_home.join("skills"));
        Self {
            config,
            mcp_launcher: McpLauncher::new(),
            skill_installer,
            npm_registry: None,
            capability_registry: None,
            active_capability_handles: HashMap::new(),
            active_passport_ids: HashMap::new(),
            loaded_workers: HashMap::new(),
        }
    }

    /// 注入 NPM Registry 服务（用于下载 npm 包）
    #[must_use]
    pub fn with_npm_registry(mut self, registry: Arc<dyn NpmRegistryService>) -> Self {
        self.npm_registry = Some(registry);
        self
    }

    /// 注入运行时能力注册表（P3 外部插件注册）。
    ///
    /// 启用插件时，插件声明的能力将以 `CapabilityOrigin::ExternalPlugin`
    /// 注册进注册表；禁用 / 卸载插件时经可逆句柄回滚。
    ///
    /// 接收 `&CapabilityRegistry` 并做一次浅克隆（共享底层存储），
    /// 便于 wiring 层直接传入全局能力注册表单例引用。
    #[must_use]
    pub fn with_capability_registry(
        mut self,
        registry: &axagent_harness::CapabilityRegistry,
    ) -> Self {
        self.capability_registry = Some(Arc::new(registry.clone()));
        self
    }

    #[must_use]
    pub fn bundled_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bundled")
    }

    #[must_use]
    pub fn install_root(&self) -> PathBuf {
        self.config
            .install_root
            .clone()
            .unwrap_or_else(|| self.config.config_home.join("plugins").join("installed"))
    }

    /// 内容寻址的构建产物根目录（PLAN §13.2：`<appdata>/plugins/<plugin_id>/<hash>/`）。
    ///
    /// 与 [`Self::install_root`] 同级但**不是**同一目录：`install_root` 下是已安装插件的
    /// 单层目录（`installed/<plugin-id>/`），本目录下按 `<plugin_id>/<artifact_hash>/` 分层，
    /// 供 §12.3 的隔离构建落盘可复用的产物（同 hash 命中即跳过编译）。
    #[must_use]
    pub fn plugin_build_root(&self) -> PathBuf {
        self.config.config_home.join("plugins")
    }

    #[must_use]
    pub fn registry_path(&self) -> PathBuf {
        self.config
            .registry_path
            .clone()
            .unwrap_or_else(|| self.config.config_home.join("plugins").join(REGISTRY_FILE_NAME))
    }

    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.config.config_home.join(SETTINGS_FILE_NAME)
    }

    pub fn plugin_registry(&self) -> Result<PluginRegistry, PluginError> {
        self.plugin_registry_report()?.into_registry()
    }

    pub fn validate_dependencies(&self, manifest: &PluginManifest) -> Result<(), PluginError> {
        if manifest.dependencies.is_empty() {
            return Ok(());
        }
        let registry = self.plugin_registry()?;
        let available: BTreeMap<&str, &str> = registry
            .plugins()
            .iter()
            .map(|p| (p.metadata().name.as_str(), p.metadata().version.as_str()))
            .collect();

        let mut errors = Vec::new();
        for dep in &manifest.dependencies {
            match available.get(dep.plugin_name.as_str()) {
                None => {
                    errors.push(PluginManifestValidationError::DependencyNotSatisfied {
                        plugin_name: dep.plugin_name.clone(),
                        min_version: dep.min_version.clone(),
                    });
                },
                Some(&installed_version) => {
                    if let Some(ref min_ver) = dep.min_version
                        && !version_satisfies(installed_version, min_ver)
                    {
                        errors.push(PluginManifestValidationError::DependencyNotSatisfied {
                            plugin_name: dep.plugin_name.clone(),
                            min_version: Some(min_ver.clone()),
                        });
                    }
                },
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(PluginError::ManifestValidation(errors))
        }
    }

    pub fn verify_integrity(
        &self,
        plugin_root: &Path,
        integrity: &PluginIntegrity,
    ) -> Result<(), PluginError> {
        verify_declared_integrity(plugin_root, integrity)
    }

    /// 读取某个仪表盘面板声明的前端资产（沙箱 iframe 的内容来源）。
    ///
    /// 四道防线，各挡一类：manifest 载入期的形状/ id 校验（坏声明进仓）→
    /// registry 钉死哈希核对（安装后被改动）→ canonicalize 包含性（符号链接逃逸）→
    /// 扩展名 + 大小上限（白名单外与内存放大）。
    ///
    /// fail-closed：安装时没钉下哈希的插件（早于此机制安装）**一律拒绝**，需重装才恢复 ——
    /// 与「插件未声明内容」是两个不同错误，不可混成一个提示。
    pub fn read_panel_asset(
        &self,
        plugin_id: &str,
        panel_id: &str,
    ) -> Result<PluginPanelAsset, PluginError> {
        let registry = self.load_registry()?;
        let record = registry.plugins.get(plugin_id).ok_or_else(|| {
            PluginError::NotFound(format!("plugin `{plugin_id}` is not installed"))
        })?;
        // 走完整载入链而非直接读盘：这样完整性核对无法被绕过
        let manifest = load_plugin_from_directory(&record.install_path)?;
        let panel =
            manifest.dashboard_panels.iter().find(|p| p.id == panel_id).ok_or_else(|| {
                PluginError::NotFound(format!(
                    "plugin `{plugin_id}` has no dashboard panel `{panel_id}`"
                ))
            })?;
        let entry = panel.frontend_entry.as_deref().ok_or_else(|| {
            PluginError::NotFound(format!(
                "dashboard panel `{panel_id}` declares no frontendEntry (metadata-only panel)"
            ))
        })?;
        if let Some(detail) = asset_entry_shape_error(entry, "frontendEntry") {
            return Err(PluginError::PermissionDenied(detail));
        }
        // 完整性核对（锚点在 registry，不在 manifest —— 后者自指不成立）
        verify_install_anchor(record, plugin_id, "dashboard panel assets")?;
        let canonical = canonical_asset_in_dir(&record.install_path, entry, "panel")?;
        let size_bytes = fs::metadata(&canonical)?.len();
        if size_bytes > PANEL_ASSET_MAX_BYTES {
            return Err(PluginError::PermissionDenied(format!(
                "panel asset `{entry}` is {size_bytes} bytes, over the {PANEL_ASSET_MAX_BYTES}-byte cap"
            )));
        }
        Ok(PluginPanelAsset {
            plugin_id: plugin_id.to_string(),
            panel_id: panel_id.to_string(),
            entry: entry.to_string(),
            source: fs::read_to_string(&canonical)?,
        })
    }

    /// 解析某个逐页声明条目（`pages[]`，§0.10.2 T3）的**绝对资产路径**。
    ///
    /// 与 [`Self::read_panel_asset`] 的分工：那条读**内容**（文本资产 → iframe `srcdoc`），
    /// 本条只给**位置**（不读文件，故没有大小上限那一层；bundle 的子资源由 asset 协议
    /// 在作用域内自取）。两条共用同一套防线，且**都不接收前端传来的路径**：
    /// 入参只有宿主绑定的 `(plugin_id, page_id)`，路径一律在此按清单解析 ——
    /// 让前端自己拼 `~/.axagent/plugins/installed/<id>` 会与 `external_dirs` 的发现形态
    /// 打架（那条链上的插件根本不在安装根里），等于给页面容器开第二条寻址链。
    ///
    /// 防线与面板同口径，另加一道作用域校验：清单载入期的形状校验（坏声明进仓）→
    /// registry 钉死哈希核对（安装后被改动）→ canonicalize 包含性（符号链接逃逸）→
    /// **解析结果必须落在 [`Self::install_root`] 内**，因为 `tauri.conf.json` 的
    /// `assetProtocol.scope` 只放行安装根；手写 registry 或改过 `install_root` 时，
    /// 宁可在这里报出原因，也不给前端一个「路径合法但 webview 必拒」的串。
    ///
    /// `loading = "schema"` 的页面**没有文件资产**（`entry` 是动态 UI Schema 的 id），
    /// 属契约误用，报错而不返回空路径。
    pub fn resolve_page_asset(
        &self,
        plugin_id: &str,
        page_id: &str,
    ) -> Result<PluginPageAssetLocation, PluginError> {
        let registry = self.load_registry()?;
        let record = registry.plugins.get(plugin_id).ok_or_else(|| {
            PluginError::NotFound(format!("plugin `{plugin_id}` is not installed"))
        })?;
        // 走完整载入链而非直接读盘：清单校验与完整性核对都无法被绕过（同 read_panel_asset）。
        let manifest = load_plugin_from_directory(&record.install_path)?;
        // 判据③（可关）：与命令执行链同口径，未启用的插件不得解析其页面资产。
        // 前端 flattenPages 只做 UI 过滤，这里补执行层闸——脚本直接 invoke 本命令时
        // 不能绕过「关掉插件」的语义（否则页面已从清单消失，资产却仍可被取到）。
        // 直接读 config.enabled_plugins（is_enabled 的权威源），默认口径与 is_enabled 一致：
        // External 未记录时视为关闭，其余按 manifest 默认值——不触发 plugin_registry 全量重扫。
        let enabled =
            self.config.enabled_plugins.get(plugin_id).copied().unwrap_or(match record.kind {
                PluginKind::External => false,
                PluginKind::Builtin | PluginKind::Bundled | PluginKind::OpenClaw => {
                    manifest.default_enabled
                },
            });
        if !enabled {
            return Err(PluginError::CommandFailed(format!(
                "插件 `{plugin_id}` 未启用，页面 `{page_id}` 资产不可解析"
            )));
        }
        let page = manifest.pages.iter().find(|p| p.id == page_id).ok_or_else(|| {
            PluginError::NotFound(format!("plugin `{plugin_id}` has no page `{page_id}`"))
        })?;
        if page.loading != PluginPageLoading::Bundle {
            return Err(PluginError::NotFound(format!(
                "page `{page_id}` 走 UI Schema 通道（loading = `{}`），没有可解析的文件资产",
                page.loading.as_str()
            )));
        }
        if let Some(detail) = asset_entry_shape_error(&page.entry, "entry") {
            return Err(PluginError::PermissionDenied(detail));
        }
        verify_install_anchor(record, plugin_id, "page assets")?;
        let canonical = canonical_asset_in_dir(&record.install_path, &page.entry, "page")?;
        // 安装根不存在时退回未规范化路径继续比对：比对不过就是「不在作用域内」，
        // 结论仍然正确，只是少了那次符号链接归一。
        let install_root = self.install_root();
        let asset_root = install_root.canonicalize().unwrap_or_else(|_| install_root.clone());
        if !canonical.starts_with(&asset_root) {
            return Err(PluginError::PermissionDenied(format!(
                "Access denied: page asset is outside the asset-scoped install root ({})",
                asset_root.display()
            )));
        }
        Ok(PluginPageAssetLocation {
            plugin_id: plugin_id.to_string(),
            page_id: page_id.to_string(),
            entry: page.entry.clone(),
            loading: page.loading,
            absolute_path: path_for_asset_protocol(&canonical),
        })
    }

    pub fn plugin_registry_report(&self) -> Result<PluginRegistryReport, PluginError> {
        self.sync_bundled_plugins()?;

        let mut discovery = PluginDiscovery::default();
        discovery.plugins.extend(builtin_plugins());

        let installed = self.discover_installed_plugins_with_failures()?;
        discovery.extend(installed);

        let external =
            self.discover_external_directory_plugins_with_failures(&discovery.plugins)?;
        discovery.extend(external);

        Ok(self.build_registry_report(discovery))
    }

    pub fn list_plugins(&self) -> Result<Vec<PluginSummary>, PluginError> {
        Ok(self.plugin_registry()?.summaries())
    }

    pub fn list_installed_plugins(&self) -> Result<Vec<PluginSummary>, PluginError> {
        Ok(self.installed_plugin_registry()?.summaries())
    }

    /// 仪表盘面板清单 —— dashboard 合流后的只读投影（`PLAN-plugin-gap-closure.md` §3）。
    ///
    /// 真相源是 `PluginManifest.dashboard_panels`：声明了非空面板的插件进入清单，
    /// 启停位跟随 PluginManager；清单每次调用实时扫描，与 rt-dashboard 时代的
    /// 「常驻 entries + 手动 reload」相比不再有第二份真相。
    pub fn list_dashboard_plugins(
        &self,
    ) -> Result<Vec<crate::types::DashboardPluginInfo>, PluginError> {
        let mut out = Vec::new();
        for summary in self.list_plugins()? {
            let Some(root) = summary.metadata.root.as_deref() else { continue };
            // 单个插件清单读取失败不影响整体列表（与 registry 容错加载同口径）。
            let Ok(manifest) = load_plugin_from_directory(root) else {
                tracing::warn!(root = %root.display(), "仪表盘面板清单：插件 manifest 读取失败，跳过");
                continue;
            };
            if manifest.dashboard_panels.is_empty() {
                continue;
            }
            out.push(crate::types::DashboardPluginInfo {
                id: summary.metadata.id,
                name: summary.metadata.name,
                version: summary.metadata.version,
                description: summary.metadata.description,
                author: manifest.author,
                enabled: summary.enabled,
                panels: manifest.dashboard_panels,
            });
        }
        Ok(out)
    }

    /// 插件页面清单 —— 逐页声明（`manifest.pages[]`，§0.10.2 T3）的只读投影（任务 #75）。
    ///
    /// 真相源是 `PluginManifest.pages`：声明了非空页面的插件进入清单，启停位跟随
    /// PluginManager；清单每次调用实时扫描，不缓存、不建第二份路由表 —— 与
    /// [`Self::list_dashboard_plugins`] 同形（禁区 4、§0.7.6 的教训）。
    pub fn list_plugin_pages(&self) -> Result<Vec<crate::types::PluginPagesInfo>, PluginError> {
        let mut out = Vec::new();
        for summary in self.list_plugins()? {
            let Some(root) = summary.metadata.root.as_deref() else { continue };
            // 单个插件清单读取失败不影响整体列表（与 registry 容错加载同口径）。
            let Ok(manifest) = load_plugin_from_directory(root) else {
                tracing::warn!(root = %root.display(), "插件页面清单：插件 manifest 读取失败，跳过");
                continue;
            };
            if manifest.pages.is_empty() {
                continue;
            }
            out.push(crate::types::PluginPagesInfo {
                id: summary.metadata.id,
                name: summary.metadata.name,
                version: summary.metadata.version,
                description: summary.metadata.description,
                author: manifest.author,
                enabled: summary.enabled,
                pages: manifest.pages,
            });
        }
        Ok(out)
    }

    /// OPC 域包清单投影 —— `manifest.domainPacks[]` 指向的相对 JSON 文件。
    ///
    /// 真相源是插件安装目录里的域包 logical manifest；每次调用实时扫描，不缓存、
    /// 不建第二套注册表。单份清单读失败只跳过该域包，不让整张清单失效。
    pub fn list_plugin_domain_packs(
        &self,
    ) -> Result<Vec<crate::types::PluginDomainPacksInfo>, PluginError> {
        let mut out = Vec::new();
        for summary in self.list_plugins()? {
            let Some(root) = summary.metadata.root.as_deref() else { continue };
            let Ok(manifest) = load_plugin_from_directory(root) else { continue };
            if manifest.domain_packs.is_empty() {
                continue;
            }
            let mut packs = Vec::new();
            for rel in &manifest.domain_packs {
                let Some(file) = Self::safe_relative_path(root, rel) else {
                    tracing::warn!(plugin = %summary.metadata.id, rel = %rel, "域包清单路径逃逸，跳过");
                    continue;
                };
                let Ok(contents) = fs::read_to_string(&file) else {
                    tracing::warn!(plugin = %summary.metadata.id, rel = %rel, "域包清单读取失败，跳过");
                    continue;
                };
                match serde_json::from_str::<crate::types::PluginDomainPackDecl>(&contents) {
                    Ok(decl) => packs.push(decl),
                    Err(error) => tracing::warn!(
                        plugin = %summary.metadata.id, rel = %rel, error = %error,
                        "域包清单解析失败，跳过"
                    ),
                }
            }
            if packs.is_empty() {
                continue;
            }
            out.push(crate::types::PluginDomainPacksInfo {
                id: summary.metadata.id,
                name: summary.metadata.name,
                version: summary.metadata.version,
                description: summary.metadata.description,
                author: manifest.author,
                enabled: summary.enabled,
                domain_packs: packs,
            });
        }
        Ok(out)
    }

    /// 把清单里的相对路径限制在安装目录内；不解析符号链接（域包 JSON 是只读声明）。
    fn safe_relative_path(root: &Path, rel: &str) -> Option<PathBuf> {
        let candidate = Path::new(rel);
        if candidate.is_absolute()
            || candidate.components().any(|c| matches!(c, Component::ParentDir))
        {
            return None;
        }
        Some(root.join(candidate))
    }

    /// 任务中心任务源清单 —— `manifest.taskSources[]` 的只读投影（任务 #148）。
    ///
    /// 与 [`Self::list_plugin_pages`] 同形：每次调用实时扫描、不缓存、不建第二份注册表
    /// （禁区 4 / §0.7.6），并带 `enabled` 位把启停过滤**留给消费侧**（任务中心只挂已启用
    /// 插件的源）。单插件清单读失败按跳过处理，不拖垮整张清单。
    pub fn list_plugin_task_sources(
        &self,
    ) -> Result<Vec<crate::types::PluginTaskSourcesInfo>, PluginError> {
        let mut out = Vec::new();
        for summary in self.list_plugins()? {
            let Some(root) = summary.metadata.root.as_deref() else { continue };
            let Ok(manifest) = load_plugin_from_directory(root) else {
                tracing::warn!(root = %root.display(), "插件任务源清单：插件 manifest 读取失败，跳过");
                continue;
            };
            if manifest.task_sources.is_empty() {
                continue;
            }
            out.push(crate::types::PluginTaskSourcesInfo {
                id: summary.metadata.id,
                name: summary.metadata.name,
                version: summary.metadata.version,
                description: summary.metadata.description,
                author: manifest.author,
                enabled: summary.enabled,
                task_sources: manifest.task_sources,
            });
        }
        Ok(out)
    }

    /// 插件贡献的卡片清单 —— `manifest.taskCards[]` 的只读投影（任务 #216 前置 ②）。
    ///
    /// 与 [`Self::list_plugin_task_sources`] 同形、逐条对齐：每次调用实时扫描已安装插件清单、
    /// 不缓存、不建第二份注册表（禁区 4），并带 `enabled` 位把启停过滤**留给消费侧**
    /// （前端注册表只挂已启用插件的卡）。单插件清单读失败按跳过处理，不拖垮整张清单。
    ///
    /// 与任务源投影的**唯一**差异：本函数不额外过滤 `actions[]`。写权限闸在装载期做
    /// （`build_manifest_task_cards` 的 `TaskCardActionWritePermissionMissing`），
    /// 已装上的插件里不可能存在越权的 `mutating` 动作 ⇒ 投影期再判一遍只会长出第二套口径
    /// （且两处判定一旦漂移，用户看到的是「装了却不显示」而不是「装不上」）。
    pub fn list_plugin_task_cards(
        &self,
    ) -> Result<Vec<crate::types::PluginTaskCardsInfo>, PluginError> {
        let mut out = Vec::new();
        for summary in self.list_plugins()? {
            let Some(root) = summary.metadata.root.as_deref() else { continue };
            let Ok(manifest) = load_plugin_from_directory(root) else {
                tracing::warn!(root = %root.display(), "插件卡片清单：插件 manifest 读取失败，跳过");
                continue;
            };
            if manifest.task_cards.is_empty() {
                continue;
            }
            out.push(crate::types::PluginTaskCardsInfo {
                id: summary.metadata.id,
                name: summary.metadata.name,
                version: summary.metadata.version,
                description: summary.metadata.description,
                author: manifest.author,
                enabled: summary.enabled,
                task_cards: manifest.task_cards,
            });
        }
        Ok(out)
    }

    /// 已启用插件贡献的**命名命令**在智能体命令面上的投影（B10-1）。
    ///
    /// 只收 `enabled` 的插件；清单读取失败按单插件跳过（与 [`Self::list_plugin_pages`]
    /// 同口径）。**不做名称去重**：与宿主 `#[agent_command]` 重名时的处置在消费侧
    /// （桥按「宿主优先」过滤），这里只如实投影声明面。
    pub fn agent_command_specs(
        &self,
    ) -> Result<Vec<crate::types::PluginAgentCommand>, PluginError> {
        let mut out = Vec::new();
        for summary in self.list_plugins()? {
            if !summary.enabled {
                continue;
            }
            let Some(root) = summary.metadata.root.as_deref() else { continue };
            let Ok(manifest) = load_plugin_from_directory(root) else {
                tracing::warn!(root = %root.display(), "插件命令清单：插件 manifest 读取失败，跳过");
                continue;
            };
            for command in manifest.commands {
                out.push(crate::types::PluginAgentCommand {
                    plugin_id: summary.metadata.id.clone(),
                    name: command.name,
                    description: command.description,
                    required_permission: command.required_permission,
                    // 参数面原样带出（未声明保持 `None`，不在这里落空 object）：
                    // 命令面要区分「没声明」与「声明了空 object」，见 types.rs 该字段的文档。
                    input_schema: command.input_schema,
                });
            }
        }
        // 稳定顺序：清单枚举顺序随目录遍历变化，索引文案不该跟着抖。
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.plugin_id.cmp(&b.plugin_id)));
        Ok(out)
    }

    pub fn discover_plugins(&self) -> Result<Vec<PluginDefinition>, PluginError> {
        Ok(self.plugin_registry()?.plugins.into_iter().map(|plugin| plugin.definition).collect())
    }

    pub fn aggregated_hooks(&self) -> Result<PluginHooks, PluginError> {
        self.plugin_registry()?.aggregated_hooks()
    }

    pub fn aggregated_tools(&self) -> Result<Vec<PluginTool>, PluginError> {
        self.plugin_registry()?.aggregated_tools()
    }

    /// 执行插件声明的命名命令（阶段3-① 分发入口，`PLAN-plugin-gap-closure.md` §2；
    /// 任务 #49 升级为**双通道**）。
    ///
    /// ## 逻辑名兼容路由（P6.2 批 1）
    ///
    /// 域包拆分后同一逻辑 OPC 命令可能由公共 worker 或独立域 worker 承接；
    /// 传 `"opc"` 这类不带 `@` 的名字已不再是完整注册表键。此时按命令名找
    /// **唯一启用 owner**，返回完整注册表 id 后再走原有精确分派。
    ///
    /// ## 通道选择（本次改动的全部增量）
    ///
    /// 插件**已长驻 worker**（manifest 声明 `worker` 且启用成功，见 `loaded_workers`）时，
    /// 命令改走**命令帧**派发（[`axagent_plugin_proto::ops::COMMAND`]）：复用启用时已 spawn 的
    /// worker 进程，一次调用 = 一次帧往返，且带宿主强制的等待上限（预算 + 收帧宽限），
    /// 插件僵死不再级联卡死宿主调用线程（D1）。**未长驻 worker 的插件维持原
    /// spawn-on-call 通道，行为逐字节不变**。
    ///
    /// ## 回退面（任务 #73 S-4：穷举式，不给原则化措辞留想象空间）
    ///
    /// **只有下面三类**错误会退回通道二，且每一类都以「worker 从未执行过这条命令」为判据：
    /// 1. worker 对命令帧回**能力拒绝** —— 错误码 [`error_codes::PLUGIN_UNKNOWN_OP`]
    ///    （未实现 `ops::COMMAND` 的存量旧二进制，任务 #59 修 M1）；
    /// 2. **worker 进程已不在** ⇒ 命令帧**未送达**（[`WorkerError::NotDelivered`]，
    ///    任务 #73 修 M-1：进程已退出 / 管道已断 / 写帧失败）—— 这是契约
    ///    「worker 中途异常退出也退回原通道」的落地，此前盘上没有这条路；
    /// 3. （隐含）插件根本没长驻 worker —— `loaded_workers` 无表项，直接走通道二，
    ///    与加本通道前的行为逐字节相同，不算「回退」。
    ///
    /// **明确不回退的也是三类**，方向都是「报错而不是重跑」：
    /// a. **超时**（[`WorkerError::Timeout`]）：超时不等于取消，worker 可能仍在执行，
    ///    重发 = 并发二次执行副作用；
    /// b. **帧已送达之后**才坏（连接断裂、回执解不开、成功信封装 Fault、worker 自报命令级
    ///    失败）：同样无法排除命令已被执行；
    /// c. **drain 窗口拒绝** 与 **pluginId 绑定不符**：两者也是「派发前 + 零副作用」，
    ///    但刻意**不**纳入回退 —— drain 说明本插件正在停用（回退等于绕过停机语义），
    ///    绑定不符说明宿主自己的选通道逻辑有缺陷（回退会把它掩盖成「插件旧」）。
    ///    原则化措辞「派发前被拒、无副作用 ⇒ 回退」**宽于实现**（审查稿 S-4），
    ///    以本清单为准；反过来也不许为了对齐原则去放宽回退面。
    ///
    /// 回退**只发生一次**：回退臂是通道一的唯一出口，走出 `if let` 后直落通道二并返回，
    /// 结构上不存在「帧 ⇄ spawn」回路（锁：三臂穷举 + e2e 的三个计数）。
    ///
    /// ## 两条通道共享的前置（判定顺序：先授权、后形状、再选通道，全部在派发之前）
    ///
    /// 授权门槛见 [`Self::check_command_gate`]；声明了 `inputSchema` 时再做最小形状校验
    /// （[`Self::check_command_input_shape`]），未声明 ⇒ 不约束输入形状，与加字段前一致。
    /// 命令帧路径的 pluginId 绑定 / drain 闸门 / 超时收口在
    /// [`crate::worker::WorkerInvoker::execute_command`]（写帧前拒绝不合规请求）。
    ///
    /// ## 输出契约
    ///
    /// spawn 通道：stdout 原文（见下方「实测事实」）。
    /// 命令帧通道：worker 成功回执的 `value` 转字符串 —— JSON 文本块原样返回，
    /// 其余（对象/数组/数字等）序列化为 JSON 文本。**与 spawn 通道不同，这是新语义**
    /// （worker 形态的命名命令此前不存在，无既有消费方）。
    ///
    /// ## spawn-on-call 通道的实测事实（未长驻 worker 时；改注释前本段写的是
    /// 「按空白切分为程序 + 参数」，那是假的）
    ///
    /// manifest 的 `commands[].command` **整团作为程序名**交给 `Command::new`，
    /// `args` 恒为空 `Vec` —— 没有任何按空白切分，也不经 shell 解析。
    /// 声明里写成 `prog a b` 时，Windows / POSIX 都会把整个字符串当作可执行文件名找，
    /// 找不到即 `Io` 错误；带参数需求请声明成 tool（`tools[].command` + `tools[].args`）。
    ///
    /// 程序名的**解析**与 tool 侧共用 [`resolve_hook_entry`]（任务 #47 修的既存缺陷）：
    /// 相对路径声明（`./` 或 `../` 前缀）先按插件安装目录拼成绝对路径再 spawn，
    /// 因为可执行文件查找只看宿主进程的 cwd、不看子进程 `current_dir`，原样传下去必然失败。
    /// ⚠ **不带 `./` 前缀的串仍按字面命令交给 PATH 查找**（`commands/greet.sh` 不会被当成
    /// 目录内文件）—— 那是 tool 侧既有语义，本次刻意不扩，否则老插件里同名的 PATH 命令
    /// 会静默改指向安装目录。绝对路径声明逐字保留，不回归。
    ///
    /// 沙箱（[`PluginTool::execute_sandboxed`]）只做两件事：`subprocess_execution`
    /// 权限门槛 + ENV 白名单（`env_clear` 后回填白名单变量）。它**不调用**
    /// `check_path_permission`，因此对子进程实际能读写什么**没有任何限制**。
    ///
    /// 所以本函数的权限档位语义是**宿主侧授权门槛（authorization gate）**，
    /// 不是执行隔离（containment）：档位只决定「宿主是否允许这个插件把这条命令跑起来」，
    /// 跑起来之后能干什么是子进程/worker 自己的事。spawn 通道输入 JSON 经 stdin 与
    /// `CLAWD_TOOL_INPUT` 注入；命令帧通道输入装在 [`CommandFrame::input`] 里经帧注入。
    pub fn execute_plugin_command(
        &self,
        plugin_id: &str,
        command_name: &str,
        input: &Value,
    ) -> Result<String, PluginError> {
        let plugin_id = if plugin_id.contains('@') {
            plugin_id.to_string()
        } else {
            self.resolve_command_owner(plugin_id, command_name)?
        };
        self.execute_plugin_command_with_timeout(
            &plugin_id,
            command_name,
            input,
            Duration::from_millis(DEFAULT_COMMAND_TIMEOUT_MS),
        )
    }

    /// 按「逻辑插件名 + 命令名」解析唯一启用 owner，返回完整注册表 id。
    ///
    /// 只扫描 enabled 插件；0 个 owner 报 `NotFound`，多个 enabled owner 报
    /// `CommandFailed`（**禁止静默取第一个**）。逻辑名本身只用于收窄候选：公共 OPC
    /// 命令与域包命令都带 `opc_` 前缀，域包 worker 名带 `opc-` 前缀。
    pub fn resolve_command_owner(
        &self,
        logical_name: &str,
        command_name: &str,
    ) -> Result<String, PluginError> {
        let registry = self.plugin_registry()?;
        let owners: Vec<String> = registry
            .plugins()
            .iter()
            .filter(|plugin| plugin.is_enabled())
            .filter(|plugin| {
                let id = plugin.metadata().id.as_str();
                id == logical_name
                    || id.starts_with(&format!("{logical_name}@"))
                    || (logical_name == "opc" && id.starts_with("opc-"))
            })
            .filter(|plugin| plugin.commands().iter().any(|c| c.name == command_name))
            .map(|plugin| plugin.metadata().id.clone())
            .collect();
        match owners.as_slice() {
            [only] => Ok(only.clone()),
            [] => Err(PluginError::NotFound(format!(
                "没有启用的插件声明命令 `{command_name}`（逻辑名 `{logical_name}`）"
            ))),
            many => Err(PluginError::CommandFailed(format!(
                "命令 `{command_name}` 被多个启用插件声明：{}；禁止静默选第一个",
                many.join(", ")
            ))),
        }
    }

    /// [`Self::execute_plugin_command`] 的显式预算变体（任务 #49）。
    ///
    /// `timeout` 只作用于**命令帧通道**（worker 执行预算随帧下发，宿主等待 =
    /// 预算 + 1s 收帧宽限，见 `worker.rs::COMMAND_FRAME_GRACE_MS`）；
    /// spawn-on-call 通道维持既存语义（**无超时**，进程自己跑完才返回）—— 给旧通道
    /// 补超时属于改造既存行为，不在本批范围，已在交接报告里列为遗留项。
    pub fn execute_plugin_command_with_timeout(
        &self,
        plugin_id: &str,
        command_name: &str,
        input: &Value,
        timeout: Duration,
    ) -> Result<String, PluginError> {
        let registry = self.plugin_registry()?;
        let plugin = registry
            .get(plugin_id)
            .ok_or_else(|| PluginError::NotFound(format!("plugin `{plugin_id}` not found")))?;
        if !plugin.is_enabled() {
            return Err(PluginError::CommandFailed(format!(
                "插件 `{plugin_id}` 未启用，命令 `{command_name}` 不可执行"
            )));
        }
        let entry = plugin.commands().iter().find(|c| c.name == command_name).ok_or_else(|| {
            PluginError::NotFound(format!("插件 `{plugin_id}` 没有声明命令 `{command_name}`"))
        })?;
        // 授权门槛先于输入形状校验：档位不满足时连「输入是否合规」都不该告知调用方。
        let required_permission = command_required_permission(entry)?;
        Self::check_command_gate(required_permission, &entry.name, plugin.permissions())?;
        if let Some(schema) = &entry.input_schema {
            Self::check_command_input_shape(&entry.name, schema, input)?;
        }
        // ── 通道一：长驻 worker（命令帧，任务 #49 新增） ──
        // 绑定语义：`plugin_id` 用注册表权威键（上面查到的就是它），worker 侧还会拿
        // 自身身份二次校验（`execute_command` 的防线②），双向都对不上才放行不了。
        if let Some(loaded) = self.loaded_workers.get(plugin_id) {
            let frame = CommandFrame::new(
                plugin_id,
                entry.name.clone(),
                input.clone(),
                timeout.as_millis().min(u64::MAX as u128) as u64,
            );
            match loaded.execute_command(&frame) {
                Ok(reply) => return Ok(command_value_to_text(&reply.value)),
                // 只有「worker 从未执行过这条命令」的两类错误才回退（穷举见本函数文档的
                // 「回退面」三类 + 「明确不回退」三类）：① 旧 worker 对命令帧的能力拒绝
                // `PLUGIN_UNKNOWN_OP`；② worker 进程已不在 ⇒ 帧**未送达**（`NotDelivered`，
                // 任务 #73 修 M-1 —— 此前契约文档承诺了这条但盘上没有，命令凭空失败）。
                // 其余（超时 / 帧送达后断裂 / drain 拒绝 / 绑定不符 / worker 自报命令级失败 /
                // 脏连接上的立即拒绝）一律原样上抛，**不回退**。
                Err(error) if !command_error_allows_fallback(&error) => {
                    return Err(command_worker_error_to_plugin_error(error));
                },
                // 回退只发生一次：本分支是通道一的**唯一**回退出口，走出 `if let` 后一路走
                // spawn 并直接 return，既不会再回到通道一（不存在「帧失败 ⇒ spawn ⇒ 又帧」的
                // 双跑循环），也不会把回退后的 spawn 失败再喂回命令帧。
                Err(error) => {
                    // 为什么敢重发这条命令：两类命中的「零副作用」依据不同，分开写 ——
                    // 判据合并成一个 bool 就是审查稿 M-1/M-2 的共同根。
                    // ① `PLUGIN_UNKNOWN_OP` 来自 worker 的 handler 兜底分支（plugin-proto
                    //    handler 模板），即 worker **还没进入任何命令逻辑**就回了能力拒绝；
                    // ② [`WorkerError::NotDelivered`] 连帧都没出去（进程已退出 / 管道已断 /
                    //    写帧失败）⇒ 对端不可能执行过任何东西。
                    // 命令**已开始执行**之后的失败一律带别的码位或是 `Transport`，不会落进
                    // 本分支。判据读**结构化 `code` 字段**（任务 #73 S-1；旧的「取 `reason`
                    // 文本首段」已删，不留双判据）⇒ 任何人改错误文案都不会再静默关掉回退。
                    // 锁住该边界的用例：`unknown_op_in_the_message_text_does_not_fall_back`
                    // （单元，反向锁）+ `e2e_legacy_worker_falls_back_to_spawn_once`（端到端
                    // 三标计数）+ `e2e_dead_worker_command_falls_back_to_spawn_once`（M-1）。
                    let fallback_reason = if command_error_is_unknown_op(&error) {
                        "未实现命令帧 op"
                    } else {
                        "进程已不在，命令帧未送达"
                    };
                    tracing::warn!(
                        plugin_id = %plugin_id,
                        // 结构化码位：能力拒绝时是 `PLUGIN_UNKNOWN_OP`；进程不在时没有协议码
                        // ⇒ 空串。本字段是判据的输入本体（S-1），不是文案的副产品。
                        error_code = %error.error_code().unwrap_or_default(),
                        "插件 `{plugin_id}` 的长驻 worker {fallback_reason}，命名命令 `{}` 回退到 \
                         spawn-on-call 通道执行；worker 错误原文：{}",
                        entry.name,
                        error,
                    );
                },
            }
        }
        // ── 通道二：spawn-on-call（未长驻 worker 的既存路径，逐字保留） ──
        let metadata = plugin.metadata();
        // 相对路径声明必须在这里解析成绝对路径，不能原样交给 `Command::new`：
        // 可执行文件查找发生在**宿主进程的 cwd**，子进程的 `current_dir` 不参与
        // （Windows 报 `OS error 3`、POSIX 报 ENOENT）⇒ `./bin/greet.cmd` 这类声明
        // 在修复前必然 spawn 失败。解析复用 tool 侧的同一条 [`resolve_hook_entry`]
        // （禁区 12：不造第二套解析），三条语义因此与 tool 声明逐字同形：
        // 字面命令（`node` 等：不带 `./`、`../` 且非绝对）原样交给 PATH 查找；
        // 绝对路径原样保留（老插件不回归）；`./`、`../` 前缀按安装根 join。
        // `root` 为 `None`（无安装目录的来源，如内置插件）时不解析 —— 与修复前一致。
        let program = match metadata.root.as_deref() {
            Some(root) => resolve_hook_entry(root, &entry.command),
            None => entry.command.clone(),
        };
        let tool = PluginTool::new(
            plugin_id,
            metadata.name.clone(),
            PluginToolDefinition {
                name: entry.name.clone(),
                description: Some(entry.description.clone()),
                input_schema: resolved_command_input_schema(entry),
            },
            program,
            Vec::new(),
            required_permission,
            metadata.root.clone(),
        );
        let sandbox = build_sandbox_from_permissions(plugin.permissions());
        tool.execute_sandboxed(input, &sandbox)
    }

    /// 命名命令的授权门槛：档位需要哪个**具体权限名**，插件声明里就必须有哪个。
    ///
    /// 刻意不调 `PluginPermission::implies_read/implies_write/implies_execute`
    /// （`types.rs`）：那三个函数都把 `SubprocessExecution` 判为满足，而「能起子进程」
    /// 本来就是命令执行的前提交（`sandbox.rs::build_sandbox_from_permissions`）
    /// ⇒ 用 `implies_*` 做门槛必然恒过，零安全增益。
    ///
    /// 也不改 `implies_*` 本身：那是 tool 侧在用的语义，动它会破坏现有插件。
    ///
    /// 真值表（未声明档位按 `ReadOnly` 处理，与加字段前一致）：
    /// `ReadOnly` 无额外要求，只受沙箱 `subprocess_execution` 门槛约束。
    /// `WorkspaceWrite` 需要 `write` 或 `file_system_write`；`execute` 与
    /// `subprocess_execution` 都不算。
    /// `DangerFullAccess` 需要 `execute`；`subprocess_execution` 不算。
    fn check_command_gate(
        required_permission: PluginToolPermission,
        command_name: &str,
        declared: &[PluginPermission],
    ) -> Result<(), PluginError> {
        let declared_labels: Vec<&str> = declared.iter().map(|p| p.as_str()).collect();
        let satisfied = match required_permission {
            PluginToolPermission::ReadOnly => true,
            PluginToolPermission::WorkspaceWrite => declared.iter().any(|permission| {
                matches!(permission, PluginPermission::Write | PluginPermission::FileSystemWrite)
            }),
            PluginToolPermission::DangerFullAccess => {
                declared.iter().any(|permission| matches!(permission, PluginPermission::Execute))
            },
        };
        if satisfied {
            return Ok(());
        }
        Err(PluginError::PermissionDenied(format!(
            "插件命令 `{command_name}` 要求档位 `{}`，但插件声明权限 {declared_labels:?} 中没有该档位要求的具体权限名（`subprocess_execution` 不能顶上），拒绝启动子进程",
            required_permission.as_str()
        )))
    }

    /// 命名命令输入的最小形状校验（自建，**不引第三方 JSON Schema 校验器**）。
    ///
    /// 校验范围到此为止，别把它当 JSON Schema 实现。只做两件事：`input` 必须是 JSON
    /// object；`schema["required"]` 若存在，其中每个字符串键都必须在 `input` 里出现。
    ///
    /// `type` / `properties` / `enum` 等其余 JSON Schema 关键字不校验（要约束值形态请声明
    /// 成 tool 并在实现侧自证）。`required` 写成非数组是声明方的 bug，按「无法判定即拒绝」
    /// 处理，不静默放行。不满足时返回错误且**不启动子进程**。
    fn check_command_input_shape(
        command_name: &str,
        schema: &Value,
        input: &Value,
    ) -> Result<(), PluginError> {
        let reject = |detail: String| {
            PluginError::CommandFailed(format!(
                "插件命令 `{command_name}` 输入校验未通过：{detail}（未启动子进程、未派发命令帧）"
            ))
        };
        let Some(input_object) = input.as_object() else {
            return Err(reject(format!(
                "input 必须是 JSON object，实际是 {}",
                json_value_kind(input)
            )));
        };
        let Some(required) = schema.get("required") else {
            return Ok(());
        };
        let Some(required_keys) = required.as_array() else {
            return Err(reject(format!(
                "inputSchema.required 必须是数组，实际是 {}",
                json_value_kind(required)
            )));
        };
        for key in required_keys {
            let Some(key) = key.as_str() else {
                return Err(reject(format!(
                    "inputSchema.required 数组元素必须是字符串，实际是 {}",
                    json_value_kind(key)
                )));
            };
            if !input_object.contains_key(key) {
                return Err(reject(format!("缺少必填字段 `{key}`")));
            }
        }
        Ok(())
    }

    pub fn validate_plugin_source(&self, source: &str) -> Result<PluginManifest, PluginError> {
        let install_source = parse_install_source(source)?;
        match install_source {
            PluginInstallSource::LocalPath { path } => load_plugin_from_directory(&path),
            _ => {
                let temp_root = self.install_root().join(".tmp");
                let staged =
                    materialize_source(&install_source, &temp_root, self.npm_registry.as_ref())?;
                let manifest = load_plugin_from_directory(&staged)?;
                // 清理临时目录
                if staged.starts_with(&temp_root) {
                    let _ = std::fs::remove_dir_all(&staged);
                }
                Ok(manifest)
            },
        }
    }

    pub fn install(&mut self, source: &str) -> Result<InstallOutcome, PluginError> {
        let install_source = parse_install_source(source)?;
        let temp_root = self.install_root().join(".tmp");
        let staged_source =
            materialize_source(&install_source, &temp_root, self.npm_registry.as_ref())?;
        let cleanup_source = matches!(install_source, PluginInstallSource::GitUrl { .. });
        let manifest = load_plugin_from_directory(&staged_source)?;

        let plugin_id = plugin_id(&manifest.name, EXTERNAL_MARKETPLACE);
        let install_path = self.install_root().join(sanitize_plugin_id(&plugin_id));

        // P3 安全:install 冲突检测 — 若 plugin_id 已存在于 registry,
        // 返回 AlreadyExists 错误,提示用户使用 update 而非 install。
        // 防止恶意插件冒充合法插件(同名覆盖)。
        {
            let registry = self.load_registry()?;
            if registry.plugins.contains_key(&plugin_id) {
                return Err(PluginError::AlreadyExists(format!(
                    "plugin `{plugin_id}` is already installed, use `update` instead of `install`"
                )));
            }
        }

        if install_path.exists() {
            remove_dir_all_with_retry(&install_path, 5)?;
        }
        copy_dir_all(&staged_source, &install_path)?;
        if cleanup_source {
            let _ = fs::remove_dir_all(&staged_source);
        }

        let now = unix_time_ms();
        // trust-on-install 钉死：拷贝完成后立刻对**整个安装目录**算哈希并写进 registry。
        // 锚点必须落在被保护范围之外 —— manifest 自身就在目录里，`manifest.integrity`
        // 因此是自指、永远无法自证（见 PLAN-dashboard-consolidation.md §6.5）。
        // 算不出哈希不阻断安装，但记录留空，面板资产读取会 fail-closed 拒绝，需重装才恢复。
        let pinned_integrity = match hash_plugin_directory(&install_path) {
            Ok(hash) => Some(PluginIntegrity { algorithm: "sha256".to_string(), hash }),
            Err(error) => {
                tracing::warn!(
                    plugin_id = %plugin_id,
                    error = %error,
                    "插件安装哈希失败：该插件的仪表盘面板资产将被拒绝，直到重新安装"
                );
                None
            },
        };
        let record = InstalledPluginRecord {
            kind: PluginKind::External,
            id: plugin_id.clone(),
            name: manifest.name,
            version: manifest.version.clone(),
            description: manifest.description,
            install_path: install_path.clone(),
            source: install_source,
            installed_at_unix_ms: now,
            updated_at_unix_ms: now,
            integrity: pinned_integrity,
        };

        let mut registry = self.load_registry()?;
        registry.plugins.insert(plugin_id.clone(), record);
        self.store_registry(&registry)?;
        self.write_enabled_state(&plugin_id, Some(true))?;
        self.config.enabled_plugins.insert(plugin_id.clone(), true);

        Ok(InstallOutcome { plugin_id, version: manifest.version, install_path })
    }

    pub fn enable(&mut self, plugin_id: &str) -> Result<(), PluginError> {
        self.ensure_known_plugin(plugin_id)?;
        self.write_enabled_state(plugin_id, Some(true))?;
        self.config.enabled_plugins.insert(plugin_id.to_string(), true);
        let registry = self.load_registry()?;
        if let Some(record) = registry.plugins.get(plugin_id) {
            let manifest = load_plugin_from_directory(&record.install_path)?;
            self.start_plugin_locked(plugin_id, record, &manifest)?;
        }
        Ok(())
    }

    fn start_plugin_locked(
        &mut self,
        plugin_id: &str,
        record: &crate::types::InstalledPluginRecord,
        manifest: &crate::types::PluginManifest,
    ) -> Result<(), PluginError> {
        if !manifest.mcp_servers.is_empty() {
            self.mcp_launcher
                .start_plugin_mcps(plugin_id, &manifest.mcp_servers, &record.install_path)
                .map_err(|e| PluginError::CommandFailed(e.to_string()))?;
        }
        if !manifest.skills.is_empty() {
            self.skill_installer
                .install_plugin_skills(plugin_id, &manifest.skills, &record.install_path)
                .map_err(|e| PluginError::CommandFailed(e.to_string()))?;
        }
        if !manifest.agents.is_empty() {
            crate::agent_provider::register_plugin_agents_sync(plugin_id, &manifest.agents);
        }
        if !manifest.capabilities.is_empty() {
            let errors = self.register_plugin_capabilities(plugin_id, manifest);
            if !errors.is_empty() {
                tracing::warn!(
                    "Plugin `{plugin_id}` had capability registration errors: {}",
                    errors.join("; ")
                );
            }
        }
        // 能力发现护照：构造并记录（索引写入由命令层 async 完成，见 `passports_for_plugin`）。
        if !manifest.skills.is_empty()
            || !manifest.agents.is_empty()
            || !manifest.capabilities.is_empty()
        {
            let count = self.register_plugin_passports(plugin_id, manifest, record.kind);
            tracing::info!("Plugin `{plugin_id}` registered {count} capability passport(s)");
        }
        // B 层 worker：声明了 `worker` 才 spawn（长驻子进程，PLAN §5.3）。
        // 失败不阻断启用 —— 声明式资产（hooks / tools / 技能）仍然可用；但必须留可见日志，
        // 否则插件「启用了却一个远程能力都没有」会无从排查。
        if let Some(decl) = &manifest.worker {
            match self.start_plugin_worker(plugin_id, &record.install_path, decl, manifest) {
                Ok(()) => tracing::info!("Started worker for plugin `{plugin_id}`"),
                Err(e) => tracing::warn!("Failed to start worker for plugin `{plugin_id}`: {e}"),
            }
        }
        Ok(())
    }

    /// 启动（或热替换）插件的 B 层 worker 子进程。
    ///
    /// 热替换取**先停后起**形态：旧进程的停机已走 drain 优雅收尾
    /// （[`LoadedPlugin::unload`]，PLAN §13.3）；尚未实现的只剩**双进程并存切换**
    /// （起新验活后再摘旧，进一步消除接缝空窗）。
    fn start_plugin_worker(
        &mut self,
        plugin_id: &str,
        install_path: &Path,
        decl: &PluginWorkerDecl,
        manifest: &PluginManifest,
    ) -> Result<(), PluginError> {
        self.stop_plugin_worker(plugin_id);
        let program = resolve_worker_program(install_path, &decl.program)?;
        let config = PluginWorkerConfig {
            args: decl.args.clone(),
            ..PluginWorkerConfig::new(plugin_id, program)
                .with_sandbox(build_sandbox_from_manifest(manifest))
                .with_working_dir(install_path.to_path_buf())
                // 清单权限原样交给 worker 连接：接缝写 op 的准入闸读它（见
                // `worker::required_permission_for`）。不在运行期协商。
                .with_permissions(manifest.permissions.clone())
        };
        let loaded =
            LoadedPlugin::load(config).map_err(|e| PluginError::CommandFailed(e.to_string()))?;
        self.loaded_workers.insert(plugin_id.to_string(), loaded);
        Ok(())
    }

    /// 停止插件的 worker：能力 LIFO 回滚 + 进程停机（[`LoadedPlugin::unload`]）。
    ///
    /// 无 worker 时是空操作 —— 便于在 disable / uninstall / stop_all 里无条件调用。
    fn stop_plugin_worker(&mut self, plugin_id: &str) {
        if let Some(loaded) = self.loaded_workers.remove(plugin_id) {
            loaded.unload();
        }
    }

    /// 取指定插件 worker 的调用面（`None` = 该插件未声明 worker 或当前未启用）。
    ///
    /// 宿主侧唯一的用途是**反向回调**（如 UI action 回流，PLAN §10.5-2）：
    /// 正向（插件 → 宿主）一律走能力接缝门面，不需要本方法。
    #[must_use]
    pub fn plugin_invoker(&self, plugin_id: &str) -> Option<Arc<dyn axagent_harness::SeamInvoker>> {
        self.loaded_workers.get(plugin_id).map(LoadedPlugin::invoker)
    }

    /// 该插件当前是否有一个**活着**的长驻 worker（任务 #73 M-1 判据的对外投影）。
    ///
    /// `false` 的三种形态：未声明 worker / 未启用成功（`loaded_workers` 无表项）、
    /// 表项还在但子进程已退出或管道已断（正是 M-1 要回退的那种状态）。
    /// 本方法**不改状态**，判据与 `execute_command` 的存活预检逐字同源
    /// （都走 [`LoadedPlugin::worker_is_running`]），不会出现「探针说活着、派发说死了」。
    #[must_use]
    pub fn plugin_worker_is_running(&self, plugin_id: &str) -> bool {
        self.loaded_workers.get(plugin_id).is_some_and(LoadedPlugin::worker_is_running)
    }

    /// 该插件的事件投递计数（任务 #135，B3「丢弃计数」的宿主侧读口）。
    ///
    /// 与 [`Self::plugin_worker_is_running`] 同一条取数链（都从 `loaded_workers` 现取，
    /// 不另存一份），所以不会出现「探针说有、派发说没」。
    /// `None` = 这个插件当前没有挂着的事件订阅（判据见
    /// [`LoadedPlugin::event_stats`]）；未启用的插件同样返回 `None`。
    #[must_use]
    pub fn plugin_event_stats(&self, plugin_id: &str) -> Option<crate::worker::EventDeliveryStats> {
        self.loaded_workers.get(plugin_id).and_then(LoadedPlugin::event_stats)
    }

    /// 把插件声明的能力注册进能力注册表（P3 外部插件注册）。
    ///
    /// 以 `CapabilityOrigin::ExternalPlugin` 来源注册，返回失败的能力接缝列表
    /// （不中断插件启用）；成功句柄存入 `active_capability_handles`，
    /// 供禁用 / 卸载时回滚。
    fn register_plugin_capabilities(
        &mut self,
        plugin_id: &str,
        manifest: &crate::types::PluginManifest,
    ) -> Vec<String> {
        let Some(registry) = self.capability_registry.clone() else {
            return Vec::new();
        };
        let mut errors = Vec::new();
        for decl in &manifest.capabilities {
            let descriptor = axagent_harness::PluginCapabilityDescriptor::new(
                &decl.seam,
                plugin_id,
                &decl.capability_type,
                &decl.version,
                &decl.description,
            );
            match registry.register_plugin_capability(descriptor) {
                Ok(handle) => {
                    self.active_capability_handles
                        .entry(plugin_id.to_string())
                        .or_default()
                        .push(handle);
                    tracing::info!(
                        "Registered plugin capability `{}` from `{plugin_id}`",
                        decl.seam
                    );
                },
                Err(e) => {
                    tracing::warn!(
                        "Failed to register plugin capability `{}` from `{plugin_id}`: {e}",
                        decl.seam
                    );
                    errors.push(format!("{}: {e}", decl.seam));
                },
            }
        }
        errors
    }

    /// 回滚插件注册的全部能力（P3 外部插件注册：禁用 / 卸载时调用）。
    fn unregister_plugin_capabilities(&mut self, plugin_id: &str) {
        if let Some(handles) = self.active_capability_handles.remove(plugin_id) {
            let count = handles.len();
            for handle in handles {
                handle.undo();
            }
            tracing::info!("Rolled back {count} plugin capability(ies) from `{plugin_id}`");
        }
    }

    /// 启用插件时构造并记录其能力护照（索引写入由命令层 async 完成）。
    ///
    /// 护照统一标记 `CapabilitySource::Plugin`，`evolvable` 由载体决定：
    /// 技能 / Agent 本地可写 → `Local`；声明的能力默认 `Derived`（进化产出副本、原护照不变）。
    /// 返回构造的护照数量。
    fn register_plugin_passports(
        &mut self,
        plugin_id: &str,
        manifest: &crate::types::PluginManifest,
        kind: PluginKind,
    ) -> usize {
        let passports = self.collect_plugin_passports(plugin_id, manifest, kind, None);
        let ids: Vec<String> = passports.iter().map(|p| p.capability_id.clone()).collect();
        if !ids.is_empty() {
            self.active_passport_ids.insert(plugin_id.to_string(), ids);
        }
        passports.len()
    }

    /// 取出插件已注册的护照 ID（禁用 / 卸载时回滚索引的依据）。
    fn take_plugin_passport_ids(&mut self, plugin_id: &str) -> Vec<String> {
        self.active_passport_ids.remove(plugin_id).unwrap_or_default()
    }

    /// 读取指定插件的能力护照（从 registry 加载 manifest 同步构造）。
    ///
    /// 纯构造、无副作用：启用后注册索引、禁用 / 卸载前回滚索引、启动收集均复用。
    pub fn passports_for_plugin(&self, plugin_id: &str) -> Vec<CapabilityPassportDto> {
        let Ok(registry) = self.plugin_registry() else {
            return Vec::new();
        };
        self.passports_for_plugin_in(&registry, plugin_id)
    }

    /// 与 [`passports_for_plugin`](Self::passports_for_plugin) 同源，但**复用调用方已加载的
    /// registry**。
    ///
    /// 为什么必须有这个入口（2026-10-01 启动期调度饥饿实锤）：`passports_for_plugin` 内部
    /// 会调 `plugin_registry()`，而那是**全量重扫**（读所有插件目录 + 解析所有 manifest）；
    /// 启动收集（`init/state.rs::register_all_capabilities` 第 4/4b 步）是**逐插件**调用它
    /// ⇒ 每个插件都多付一次 N 倍全量扫描，总体 **O(N²)**，20+ 秒同步文件 IO 把 tokio worker
    /// 占满（探针实测 40 秒调度空档，并连带 `index_queue` 报 `Connection pool timed out` ——
    /// 池里当时有 12 个空闲连接）。本入口把 registry 交给调用方加载一次，降为 **O(N)**。
    ///
    /// 单个插件的 manifest 仍会重解析一次：`PluginMetadata` 不含 skills/agents，无法从
    /// registry 复用（要做只能扩展 `PluginDefinition` 保存整份 manifest，影响面更大）。
    ///
    /// 原方法保持"每次与磁盘即时一致"的语义不动 —— 启用/禁用/卸载路径依赖它。
    pub fn passports_for_plugin_in(
        &self,
        registry: &PluginRegistry,
        plugin_id: &str,
    ) -> Vec<CapabilityPassportDto> {
        let Some(record) = registry.get(plugin_id) else {
            return Vec::new();
        };
        let meta = record.metadata();
        let Some(root) = meta.root.as_ref() else {
            return Vec::new();
        };
        let Ok(manifest) = load_plugin_from_directory(root) else {
            return Vec::new();
        };
        self.collect_plugin_passports(plugin_id, &manifest, meta.kind, Some(root))
    }

    /// 同步构造某插件的全部能力护照（技能 + Agent + 声明的能力）。
    ///
    /// `plugin_root` 是插件安装目录——用于读取 Skill 引用的 SKILL.md 文件，
    /// 把 markdown 正文填入 `prompt_body`（运行时 AgentNode.system_prompt）。
    /// 传 `None` 时跳过文件读取，description 用占位文本（保持向后兼容）。
    ///
    /// 同一 manifest + 同一 root 输出稳定；同一 manifest + None root 也稳定。
    fn collect_plugin_passports(
        &self,
        plugin_id: &str,
        manifest: &crate::types::PluginManifest,
        kind: PluginKind,
        plugin_root: Option<&Path>,
    ) -> Vec<CapabilityPassportDto> {
        use axagent_harness::{
            CapabilityDomain, CapabilityEvolvability, CapabilityKind, CapabilityPassportDto,
            CapabilitySource, Visibility,
        };
        let mut passports = Vec::new();
        // 技能（本地 SKILL.md，本地可写 → Local 进化）
        for skill in &manifest.skills {
            // 读 SKILL.md 文件 → 提取 description + prompt_body
            let (skill_desc, skill_prompt_body) = match plugin_root {
                Some(root) => {
                    let skill_path = root.join(&skill.path);
                    match fs::read_to_string(&skill_path) {
                        Ok(content) => parse_skill_md(&content),
                        Err(e) => {
                            tracing::debug!(
                                plugin_id,
                                skill_name = %skill.name,
                                path = %skill_path.display(),
                                error = %e,
                                "读取 SKILL.md 失败，使用占位 description"
                            );
                            (format!("插件 `{plugin_id}` 提供的技能 {}", skill.name), None)
                        },
                    }
                },
                None => (format!("插件 `{plugin_id}` 提供的技能 {}", skill.name), None),
            };
            passports.push(CapabilityPassportDto {
                capability_id: format!("plugin:{plugin_id}:skill:{}", skill.name),
                name: skill.name.clone(),
                description: skill_desc,
                kind: CapabilityKind::Skill,
                domain: CapabilityDomain::General,
                source: CapabilitySource::Plugin,
                evolvable: CapabilityEvolvability::Local,
                sub_category: "plugin_skill".to_string(),
                visibility: Visibility::Public,
                tags: vec!["plugin".to_string(), plugin_id.to_string(), "skill".to_string()],
                prompt_body: skill_prompt_body,
                ..Default::default()
            });
        }
        // Agent（本地 agent 定义，本地可写 → Local 进化）
        for agent in &manifest.agents {
            passports.push(CapabilityPassportDto {
                capability_id: format!("plugin:{plugin_id}:agent:{}", agent.agent_type),
                name: agent.agent_type.clone(),
                description: agent.description.clone(),
                kind: CapabilityKind::Agent,
                domain: CapabilityDomain::General,
                source: CapabilitySource::Plugin,
                evolvable: CapabilityEvolvability::Local,
                sub_category: "plugin_agent".to_string(),
                visibility: Visibility::Public,
                tags: vec!["plugin".to_string(), plugin_id.to_string(), "agent".to_string()],
                ..Default::default()
            });
        }
        // 声明的能力（按 decl 检索元数据映射；默认 Derived 进化 → 不影响原能力）
        for decl in &manifest.capabilities {
            if !decl.discoverable {
                continue;
            }
            let mut dto = CapabilityPassportDto {
                capability_id: format!("plugin:{plugin_id}:cap:{}", decl.seam),
                name: if decl.name.is_empty() {
                    decl.seam.clone()
                } else {
                    decl.name.clone()
                },
                description: decl.description.clone(),
                kind: parse_capability_kind(&decl.kind).unwrap_or(CapabilityKind::Tool),
                domain: decl.domain.parse().unwrap_or(CapabilityDomain::General),
                source: CapabilitySource::Plugin,
                evolvable: parse_capability_evolvability(&decl.evolvable)
                    .unwrap_or(CapabilityEvolvability::Derived),
                sub_category: decl.capability_type.clone(),
                visibility: parse_visibility(&decl.visibility),
                tags: decl.tags.clone(),
                negative_scenarios: decl.negative_scenarios.clone(),
                ..Default::default()
            };
            if dto.tags.is_empty() {
                dto.tags = vec!["plugin".to_string(), plugin_id.to_string()];
            }
            passports.push(dto);
        }
        // 非内置插件在能力发现中始终标记为插件来源（OpenClaw 等也参与发现/进化判断）
        let _ = kind;
        passports
    }

    pub fn disable(&mut self, plugin_id: &str) -> Result<(), PluginError> {
        self.mcp_launcher.stop_plugin_mcps(plugin_id);
        crate::agent_provider::unregister_plugin_agents_sync(plugin_id);
        self.skill_installer.remove_plugin_skills(plugin_id).ok();
        self.unregister_plugin_capabilities(plugin_id);
        let _ = self.take_plugin_passport_ids(plugin_id);
        self.stop_plugin_worker(plugin_id);
        self.ensure_known_plugin(plugin_id)?;
        self.write_enabled_state(plugin_id, Some(false))?;
        self.config.enabled_plugins.insert(plugin_id.to_string(), false);
        Ok(())
    }

    pub fn start_enabled_plugins(&mut self) -> Result<Vec<String>, PluginError> {
        let registry = self.load_registry()?;
        let mut started = Vec::new();
        let mut errors = Vec::new();

        let enabled_ids: Vec<String> = self
            .config
            .enabled_plugins
            .iter()
            .filter(|entry| *entry.1)
            .map(|(id, _)| id.clone())
            .collect();

        for plugin_id in &enabled_ids {
            let Some(record) = registry.plugins.get(plugin_id) else {
                continue;
            };
            match load_plugin_from_directory(&record.install_path) {
                Ok(manifest) => {
                    if let Err(e) = self.start_plugin_locked(plugin_id, record, &manifest) {
                        tracing::warn!("Failed to start plugin `{plugin_id}`: {e}");
                        errors.push(format!("{plugin_id}: {e}"));
                    } else {
                        started.push(plugin_id.clone());
                        tracing::info!("Started enabled plugin: {plugin_id} v{}", record.version);
                    }
                },
                Err(e) => {
                    tracing::warn!("Failed to load manifest for enabled plugin `{plugin_id}`: {e}");
                    errors.push(format!("{plugin_id}: {e}"));
                },
            }
        }

        if errors.is_empty() {
            Ok(started)
        } else {
            tracing::warn!("Some plugins failed to start: {}", errors.join("; "));
            Ok(started)
        }
    }

    pub fn stop_all_plugins(&mut self) {
        let plugin_ids: Vec<String> = self
            .config
            .enabled_plugins
            .iter()
            .filter(|entry| *entry.1)
            .map(|(id, _)| id.clone())
            .collect();
        for plugin_id in plugin_ids {
            self.mcp_launcher.stop_plugin_mcps(&plugin_id);
            crate::agent_provider::unregister_plugin_agents_sync(&plugin_id);
            self.skill_installer.remove_plugin_skills(&plugin_id).ok();
            self.unregister_plugin_capabilities(&plugin_id);
            let _ = self.take_plugin_passport_ids(&plugin_id);
            self.stop_plugin_worker(&plugin_id);
            tracing::info!("Stopped plugin: {plugin_id}");
        }
    }

    pub fn uninstall(&mut self, plugin_id: &str) -> Result<(), PluginError> {
        let mut registry = self.load_registry()?;
        let record = registry.plugins.remove(plugin_id).ok_or_else(|| {
            PluginError::NotFound(format!("plugin `{plugin_id}` is not installed"))
        })?;
        let remaining_plugins: Vec<&InstalledPluginRecord> =
            registry.plugins.values().filter(|r| r.id != plugin_id).collect();
        let mut dependents = Vec::new();
        for other in &remaining_plugins {
            if let Ok(other_manifest) = load_plugin_from_directory(&other.install_path) {
                for dep in &other_manifest.dependencies {
                    if dep.plugin_name == record.name {
                        dependents.push(other.name.clone());
                        break;
                    }
                }
            }
        }
        if !dependents.is_empty() {
            registry.plugins.insert(plugin_id.to_string(), record);
            return Err(PluginError::CommandFailed(format!(
                "cannot uninstall plugin `{plugin_id}`: other plugins depend on it: {}",
                dependents.join(", ")
            )));
        }
        if record.kind == PluginKind::Bundled {
            registry.plugins.insert(plugin_id.to_string(), record);
            return Err(PluginError::CommandFailed(format!(
                "plugin `{plugin_id}` is bundled and managed automatically; disable it instead"
            )));
        }
        self.mcp_launcher.stop_plugin_mcps(plugin_id);
        crate::agent_provider::unregister_plugin_agents_sync(plugin_id);
        self.skill_installer.remove_plugin_skills(plugin_id).ok();
        self.unregister_plugin_capabilities(plugin_id);
        let _ = self.take_plugin_passport_ids(plugin_id);
        // 卸载：先停 worker（能力回滚 + 进程停机），再删安装目录 —— 顺序反了会留下
        // 指向已删除目录的常驻进程。
        self.stop_plugin_worker(plugin_id);
        if record.install_path.exists() {
            remove_dir_all_with_retry(&record.install_path, 5)?;
        }
        self.store_registry(&registry)?;
        self.write_enabled_state(plugin_id, None)?;
        self.config.enabled_plugins.remove(plugin_id);
        Ok(())
    }

    pub fn update(&mut self, plugin_id: &str) -> Result<UpdateOutcome, PluginError> {
        let mut registry = self.load_registry()?;
        let record = registry.plugins.get(plugin_id).cloned().ok_or_else(|| {
            PluginError::NotFound(format!("plugin `{plugin_id}` is not installed"))
        })?;

        let temp_root = self.install_root().join(".tmp");
        let staged_source =
            materialize_source(&record.source, &temp_root, self.npm_registry.as_ref())?;
        let cleanup_source = matches!(record.source, PluginInstallSource::GitUrl { .. });
        let manifest = load_plugin_from_directory(&staged_source)?;
        self.validate_dependencies(&manifest)?;

        // P3 安全:升级备份机制 — 在覆盖前备份旧版本到 `.bak` 目录,
        // 保留用户对 plugin.json / SKILL.md / hooks 的本地修改。
        // 备份失败不阻断升级(降级到原行为),仅 warn 日志。
        // 备份目录命名:{install_path}.bak,仅保留最近 1 个版本。
        // 换目录前必须停掉 worker：Windows 上正在运行的可执行文件无法删除，
        // 删除失败会直接让升级失败（且报错指向文件占用，很难联想到 worker）。
        self.stop_plugin_worker(plugin_id);
        let backup_path = record.install_path.with_extension("bak");
        if record.install_path.exists() {
            // 先清理旧的备份目录(若存在)
            if backup_path.exists() {
                let _ = fs::remove_dir_all(&backup_path);
            }
            match copy_dir_all(&record.install_path, &backup_path) {
                Ok(()) => {
                    tracing::info!(
                        "[plugin_update] plugin `{}` 旧版本已备份到 {}",
                        plugin_id,
                        backup_path.display()
                    );
                },
                Err(e) => {
                    tracing::warn!(
                        "[plugin_update] plugin `{}` 备份失败: {} — 继续升级,旧版本将被覆盖",
                        plugin_id,
                        e
                    );
                },
            }
            remove_dir_all_with_retry(&record.install_path, 5)?;
        }
        copy_dir_all(&staged_source, &record.install_path)?;
        if cleanup_source {
            let _ = fs::remove_dir_all(&staged_source);
        }

        // P3 缺陷修复：若插件处于启用状态，update 后需回滚旧版本已注册的能力，
        // 再用新 manifest 重新注册，避免旧描述残留、新能力不生效。
        // 放在 updated_record 构造之前执行，此时 manifest 字段尚未被 move。
        let enabled = self.config.enabled_plugins.get(plugin_id).copied().unwrap_or(false);
        if enabled {
            self.unregister_plugin_capabilities(plugin_id);
            let errors = self.register_plugin_capabilities(plugin_id, &manifest);
            if !errors.is_empty() {
                tracing::warn!(
                    "[plugin_update] plugin `{plugin_id}` capability re-registration had errors: {}",
                    errors.join("; ")
                );
            }
            // worker 同样要换到新目录的产物上（旧的指向刚被替换掉的可执行文件）。
            if let Some(decl) = &manifest.worker {
                match self.start_plugin_worker(plugin_id, &record.install_path, decl, &manifest) {
                    Ok(()) => {},
                    Err(e) => {
                        tracing::warn!("[plugin_update] plugin `{plugin_id}` worker 重启失败: {e}");
                    },
                }
            }
        }

        // 更新会整目录替换 ⇒ 旧钉死值立刻失效，必须对新目录重算（否则把新内容判成篡改）。
        let repinned_integrity = match hash_plugin_directory(&record.install_path) {
            Ok(hash) => Some(PluginIntegrity { algorithm: "sha256".to_string(), hash }),
            Err(error) => {
                tracing::warn!(
                    plugin_id = %plugin_id,
                    error = %error,
                    "插件更新后哈希重算失败：该插件的仪表盘面板资产将被拒绝，直到重新安装"
                );
                None
            },
        };
        let updated_record = InstalledPluginRecord {
            version: manifest.version.clone(),
            description: manifest.description,
            updated_at_unix_ms: unix_time_ms(),
            integrity: repinned_integrity,
            ..record.clone()
        };
        registry.plugins.insert(plugin_id.to_string(), updated_record);
        self.store_registry(&registry)?;

        Ok(UpdateOutcome {
            plugin_id: plugin_id.to_string(),
            old_version: record.version,
            new_version: manifest.version,
            install_path: record.install_path,
        })
    }

    fn discover_installed_plugins_with_failures(&self) -> Result<PluginDiscovery, PluginError> {
        let mut registry = self.load_registry()?;
        let mut discovery = PluginDiscovery::default();
        let mut seen_ids = BTreeSet::<String>::new();
        let mut seen_paths = BTreeSet::<PathBuf>::new();
        let mut stale_registry_ids = Vec::new();

        for install_path in discover_plugin_dirs(&self.install_root())? {
            let matched_record =
                registry.plugins.values().find(|record| record.install_path == install_path);
            let kind = matched_record.map_or(PluginKind::External, |record| record.kind);
            let source = matched_record.map_or_else(
                || install_path.display().to_string(),
                |record| describe_install_source(&record.source),
            );
            match load_plugin_definition(&install_path, kind, source.clone(), kind.marketplace()) {
                Ok(plugin) => {
                    if seen_ids.insert(plugin.metadata().id.clone()) {
                        seen_paths.insert(install_path);
                        discovery.push_plugin(plugin);
                    }
                },
                Err(error) => {
                    discovery.push_failure(PluginLoadFailure::new(
                        install_path,
                        kind,
                        source,
                        error,
                    ));
                },
            }
        }

        for record in registry.plugins.values() {
            if seen_paths.contains(&record.install_path) {
                continue;
            }
            if !record.install_path.exists() || plugin_manifest_path(&record.install_path).is_err()
            {
                stale_registry_ids.push(record.id.clone());
                continue;
            }
            let source = describe_install_source(&record.source);
            match load_plugin_definition(
                &record.install_path,
                record.kind,
                source.clone(),
                record.kind.marketplace(),
            ) {
                Ok(plugin) => {
                    if seen_ids.insert(plugin.metadata().id.clone()) {
                        seen_paths.insert(record.install_path.clone());
                        discovery.push_plugin(plugin);
                    }
                },
                Err(error) => {
                    discovery.push_failure(PluginLoadFailure::new(
                        record.install_path.clone(),
                        record.kind,
                        source,
                        error,
                    ));
                },
            }
        }

        if !stale_registry_ids.is_empty() {
            for plugin_id in stale_registry_ids {
                registry.plugins.remove(&plugin_id);
            }
            self.store_registry(&registry)?;
        }

        Ok(discovery)
    }

    fn discover_external_directory_plugins_with_failures(
        &self,
        existing_plugins: &[PluginDefinition],
    ) -> Result<PluginDiscovery, PluginError> {
        let mut discovery = PluginDiscovery::default();

        for directory in &self.config.external_dirs {
            let source = Self::derive_source_from_external_dir(directory);
            for root in discover_plugin_dirs(directory)? {
                match load_plugin_definition(
                    &root,
                    PluginKind::External,
                    source.clone(),
                    EXTERNAL_MARKETPLACE,
                ) {
                    Ok(plugin) => {
                        if existing_plugins
                            .iter()
                            .chain(discovery.plugins.iter())
                            .all(|existing| existing.metadata().id != plugin.metadata().id)
                        {
                            discovery.push_plugin(plugin);
                        }
                    },
                    Err(error) => {
                        discovery.push_failure(PluginLoadFailure::new(
                            root,
                            PluginKind::External,
                            source.clone(),
                            error,
                        ));
                    },
                }
            }
        }

        Ok(discovery)
    }

    fn derive_source_from_external_dir(directory: &Path) -> String {
        let dir_str = directory.to_string_lossy().to_lowercase();
        // 按优先级匹配，避免 .codebuddy 被 .claude 误匹配
        if dir_str.contains(".axagent") {
            "axagent".to_string()
        } else if dir_str.contains(".claude") {
            "claude".to_string()
        } else if dir_str.contains(".agents") {
            "agents".to_string()
        } else if dir_str.contains(".codebuddy") {
            "codebuddy".to_string()
        } else if dir_str.contains(".trae") {
            "trae".to_string()
        } else if dir_str.contains(".workbuddy") {
            "workbuddy".to_string()
        } else {
            // Fallback: use the parent directory name (e.g. ".custom" from "~/.custom/skills")
            directory
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        }
    }

    pub fn installed_plugin_registry_report(&self) -> Result<PluginRegistryReport, PluginError> {
        self.sync_bundled_plugins()?;
        Ok(self.build_registry_report(self.discover_installed_plugins_with_failures()?))
    }

    fn sync_bundled_plugins(&self) -> Result<(), PluginError> {
        let bundled_root = self.config.bundled_root.clone().unwrap_or_else(Self::bundled_root);
        let bundled_plugins = discover_plugin_dirs(&bundled_root)?;
        let mut registry = self.load_registry()?;
        let mut changed = false;
        let install_root = self.install_root();
        let mut active_bundled_ids = BTreeSet::new();

        for source_root in bundled_plugins {
            let manifest = load_plugin_from_directory(&source_root)?;
            let plugin_id = plugin_id(&manifest.name, BUNDLED_MARKETPLACE);
            active_bundled_ids.insert(plugin_id.clone());
            let install_path = install_root.join(sanitize_plugin_id(&plugin_id));
            let now = unix_time_ms();
            let existing_record = registry.plugins.get(&plugin_id);
            let installed_copy_is_valid =
                install_path.exists() && load_plugin_from_directory(&install_path).is_ok();
            let needs_sync = existing_record.is_none_or(|record| {
                record.kind != PluginKind::Bundled
                    || record.version != manifest.version
                    || record.name != manifest.name
                    || record.description != manifest.description
                    || record.install_path != install_path
                    || !record.install_path.exists()
                    || !installed_copy_is_valid
            });

            if !needs_sync {
                continue;
            }

            if install_path.exists() {
                remove_dir_all_with_retry(&install_path, 3)?;
            }
            copy_dir_all(&source_root, &install_path)?;
            // bundled 同步同样是「整目录替换」⇒ 与 install/update 一致地重钉哈希，
            // 否则三类来源里只有 bundled 的面板资产没有完整性锚点。
            let pinned_integrity = match hash_plugin_directory(&install_path) {
                Ok(hash) => Some(PluginIntegrity { algorithm: "sha256".to_string(), hash }),
                Err(error) => {
                    tracing::warn!(path = %install_path.display(), error = %error, "bundled 插件哈希失败");
                    None
                },
            };

            let installed_at_unix_ms =
                existing_record.map_or(now, |record| record.installed_at_unix_ms);
            registry.plugins.insert(
                plugin_id.clone(),
                InstalledPluginRecord {
                    kind: PluginKind::Bundled,
                    id: plugin_id,
                    name: manifest.name,
                    version: manifest.version,
                    description: manifest.description,
                    install_path,
                    source: PluginInstallSource::LocalPath { path: source_root },
                    installed_at_unix_ms,
                    updated_at_unix_ms: now,
                    integrity: pinned_integrity,
                },
            );
            changed = true;
        }

        let stale_bundled_ids = registry
            .plugins
            .iter()
            .filter_map(|(plugin_id, record)| {
                (record.kind == PluginKind::Bundled && !active_bundled_ids.contains(plugin_id))
                    .then_some(plugin_id.clone())
            })
            .collect::<Vec<_>>();

        for plugin_id in stale_bundled_ids {
            if let Some(record) = registry.plugins.remove(&plugin_id) {
                if record.install_path.exists() {
                    remove_dir_all_with_retry(&record.install_path, 3)?;
                }
                changed = true;
            }
        }

        if changed {
            self.store_registry(&registry)?;
        }

        Ok(())
    }

    fn is_enabled(&self, metadata: &PluginMetadata) -> bool {
        self.config.enabled_plugins.get(&metadata.id).copied().unwrap_or(match metadata.kind {
            PluginKind::External => false,
            PluginKind::Builtin | PluginKind::Bundled | PluginKind::OpenClaw => {
                metadata.default_enabled
            },
        })
    }

    fn ensure_known_plugin(&self, plugin_id: &str) -> Result<(), PluginError> {
        if self.plugin_registry()?.contains(plugin_id) {
            Ok(())
        } else {
            Err(PluginError::NotFound(format!(
                "plugin `{plugin_id}` is not installed or discoverable"
            )))
        }
    }

    pub(crate) fn load_registry(&self) -> Result<InstalledPluginRegistry, PluginError> {
        let path = self.registry_path();
        match fs::read_to_string(&path) {
            Ok(contents) if contents.trim().is_empty() => Ok(InstalledPluginRegistry::default()),
            Ok(contents) => Ok(serde_json::from_str(&contents)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(InstalledPluginRegistry::default())
            },
            Err(error) => Err(PluginError::Io(error)),
        }
    }

    pub(crate) fn store_registry(
        &self,
        registry: &InstalledPluginRegistry,
    ) -> Result<(), PluginError> {
        let path = self.registry_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(registry)?)?;
        Ok(())
    }

    pub(crate) fn write_enabled_state(
        &self,
        plugin_id: &str,
        enabled: Option<bool>,
    ) -> Result<(), PluginError> {
        update_settings_json(&self.settings_path(), |root| {
            let enabled_plugins = ensure_object(root, "enabledPlugins");
            match enabled {
                Some(value) => {
                    enabled_plugins.insert(plugin_id.to_string(), Value::Bool(value));
                },
                None => {
                    enabled_plugins.remove(plugin_id);
                },
            }
        })
    }

    fn installed_plugin_registry(&self) -> Result<PluginRegistry, PluginError> {
        self.installed_plugin_registry_report()?.into_registry()
    }

    fn build_registry_report(&self, discovery: PluginDiscovery) -> PluginRegistryReport {
        PluginRegistryReport::new(
            PluginRegistry::new(
                discovery
                    .plugins
                    .into_iter()
                    .map(|plugin| {
                        let enabled = self.is_enabled(plugin.metadata());
                        RegisteredPlugin::new(plugin, enabled)
                    })
                    .collect(),
            ),
            discovery.failures,
        )
    }
}

// ── 护照元数据映射 helper ─────────────────────────

/// 将插件声明的能力类型字符串映射为 `CapabilityKind`（未知回退 `Tool`）。
fn parse_capability_kind(s: &str) -> Option<axagent_harness::CapabilityKind> {
    match s.to_lowercase().as_str() {
        "tool" => Some(axagent_harness::CapabilityKind::Tool),
        "workflow" => Some(axagent_harness::CapabilityKind::Workflow),
        "knowledge_base" | "knowledgebase" => Some(axagent_harness::CapabilityKind::KnowledgeBase),
        "agent" => Some(axagent_harness::CapabilityKind::Agent),
        "skill" => Some(axagent_harness::CapabilityKind::Skill),
        _ => None,
    }
}

/// 将插件声明的可进化性字符串映射为 `CapabilityEvolvability`。
fn parse_capability_evolvability(s: &str) -> Option<axagent_harness::CapabilityEvolvability> {
    match s.to_lowercase().as_str() {
        "local" => Some(axagent_harness::CapabilityEvolvability::Local),
        "derived" => Some(axagent_harness::CapabilityEvolvability::Derived),
        "none" => Some(axagent_harness::CapabilityEvolvability::None),
        _ => None,
    }
}

/// 将插件声明的可见性字符串映射为 `Visibility`（未知回退 `Public`）。
fn parse_visibility(s: &str) -> axagent_harness::Visibility {
    match s.to_lowercase().as_str() {
        "system_only" => axagent_harness::Visibility::SystemOnly,
        "privileged_only" => axagent_harness::Visibility::PrivilegedOnly,
        "hidden" => axagent_harness::Visibility::Hidden,
        _ => axagent_harness::Visibility::Public,
    }
}

#[must_use]
pub fn builtin_plugins() -> Vec<PluginDefinition> {
    vec![PluginDefinition::Builtin(BuiltinPlugin {
        metadata: PluginMetadata {
            id: plugin_id("example-builtin", BUILTIN_MARKETPLACE),
            name: "example-builtin".to_string(),
            version: "0.1.0".to_string(),
            description: "Example built-in plugin scaffold for the Rust plugin system".to_string(),
            kind: PluginKind::Builtin,
            source: BUILTIN_MARKETPLACE.to_string(),
            default_enabled: false,
            root: None,
        },
        hooks: PluginHooks::default(),
        lifecycle: PluginLifecycle::default(),
        tools: Vec::new(),
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        permissions: Vec::new(),
        commands: Vec::new(),
    })]
}

fn load_plugin_definition(
    root: &Path,
    kind: PluginKind,
    source: String,
    marketplace: &str,
) -> Result<PluginDefinition, PluginError> {
    let manifest = load_plugin_from_directory(root)?;
    let metadata = PluginMetadata {
        id: plugin_id(&manifest.name, marketplace),
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        kind,
        source,
        default_enabled: manifest.default_enabled,
        root: Some(root.to_path_buf()),
    };
    let hooks = resolve_hooks(root, &manifest.hooks);
    let lifecycle = resolve_lifecycle(root, &manifest.lifecycle);
    let tools = resolve_tools(root, &metadata.id, &metadata.name, &manifest.tools);
    let mcp_servers = manifest.mcp_servers;
    let skills = manifest.skills;
    let permissions = manifest.permissions;
    // 阶段3-①：commands 必须一路带进 PluginDefinition —— 历史上此处丢弃字段，
    // 导致 manifest.commands[] 只有加载校验、零执行分发（「有 schema 无分发」缺口）。
    let commands = manifest.commands;
    Ok(match kind {
        PluginKind::Builtin => PluginDefinition::Builtin(BuiltinPlugin {
            metadata,
            hooks,
            lifecycle,
            tools,
            mcp_servers,
            skills,
            permissions,
            commands,
        }),
        PluginKind::Bundled => PluginDefinition::Bundled(BundledPlugin {
            metadata,
            hooks,
            lifecycle,
            tools,
            mcp_servers,
            skills,
            permissions,
            commands,
        }),
        PluginKind::External => PluginDefinition::External(ExternalPlugin {
            metadata,
            hooks,
            lifecycle,
            tools,
            mcp_servers,
            skills,
            permissions,
            commands,
        }),
        PluginKind::OpenClaw => PluginDefinition::OpenClaw(OpenClawPlugin {
            metadata,
            hooks,
            lifecycle,
            tools,
            mcp_servers,
            skills,
            permissions,
            commands,
        }),
    })
}

pub fn load_plugin_from_directory(root: &Path) -> Result<PluginManifest, PluginError> {
    load_manifest_from_directory(root)
}

fn load_manifest_from_directory(root: &Path) -> Result<PluginManifest, PluginError> {
    let manifest_path = plugin_manifest_path(root)?;
    if manifest_path.ends_with("SKILL.md") {
        load_manifest_from_skill_md(root, &manifest_path)
    } else {
        load_manifest_from_path(root, &manifest_path)
    }
}

// ── SKILL.md 解析 (Claude Code 兼容) ──

fn load_manifest_from_skill_md(
    root: &Path,
    manifest_path: &Path,
) -> Result<PluginManifest, PluginError> {
    let contents = fs::read_to_string(manifest_path).map_err(|error| {
        PluginError::NotFound(format!("SKILL.md not found at {}: {error}", manifest_path.display()))
    })?;

    let dir_name = root.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");

    let mut name = String::from(dir_name);
    let mut description = String::new();
    let mut version = String::from("1.0.0");
    let mut permissions: Vec<String> = Vec::new();
    let mut pre_tool_use: Vec<String> = Vec::new();
    let mut post_tool_use: Vec<String> = Vec::new();
    let mut post_tool_use_failure: Vec<String> = Vec::new();
    let mut init_commands: Vec<String> = Vec::new();
    let mut shutdown_commands: Vec<String> = Vec::new();
    let mut default_enabled = true;

    if let Some(frontmatter) = extract_yaml_frontmatter(&contents) {
        let parsed = parse_yaml_frontmatter(&frontmatter);

        if let Some(v) = parsed.get("name").and_then(|v| v.as_str()) {
            name = v.to_string();
        }
        if let Some(v) = parsed.get("description").and_then(|v| v.as_str()) {
            description = v.to_string();
        }
        if let Some(v) = parsed.get("version").and_then(|v| v.as_str()) {
            version = v.to_string();
        }
        if let Some(v) = parsed.get("default_enabled").or(parsed.get("defaultEnabled")) {
            default_enabled =
                v.as_bool().unwrap_or(v.as_str().is_none_or(|s| s.eq_ignore_ascii_case("true")));
        }

        for key in &["permissions"] {
            if let Some(items) = parsed.get(*key).and_then(|v| v.as_array()) {
                for item in items {
                    if let Some(s) = item.as_str() {
                        permissions.push(s.to_string());
                    }
                }
            }
        }

        for (target, key_names) in [
            (&mut pre_tool_use, vec!["pre_tool_use", "PreToolUse"]),
            (&mut post_tool_use, vec!["post_tool_use", "PostToolUse"]),
            (&mut post_tool_use_failure, vec!["post_tool_use_failure", "PostToolUseFailure"]),
            (&mut init_commands, vec!["init", "Init"]),
            (&mut shutdown_commands, vec!["shutdown", "Shutdown"]),
        ] {
            for key_name in key_names {
                if let Some(items) = parsed.get(key_name).and_then(|v| v.as_array()) {
                    for item in items {
                        if let Some(s) = item.as_str() {
                            target.push(s.to_string());
                        }
                    }
                }
            }
        }
    }

    let manifest = PluginManifest {
        name,
        description: if description.is_empty() {
            format!("SKILL.md 技能: {}", dir_name)
        } else {
            description
        },
        version,
        author: None,
        permissions: permissions.iter().filter_map(|p| PluginPermission::parse(p)).collect(),
        default_enabled,
        hooks: PluginHooks { pre_tool_use, post_tool_use, post_tool_use_failure },
        lifecycle: PluginLifecycle { init: init_commands, shutdown: shutdown_commands },
        tools: Vec::new(),
        commands: Vec::new(),
        scenarios: Vec::new(),
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        agents: Vec::new(),
        dashboard_panels: Vec::new(),
        // SKILL.md 派生的清单同样没有页面/任务源声明（二者只能来自 plugin.json）。
        pages: Vec::new(),
        task_sources: Vec::new(),
        task_cards: Vec::new(),
        dependencies: Vec::new(),
        integrity: None,
        capabilities: Vec::new(),
        // SKILL.md 派生的清单没有 worker 声明（该声明只能来自 plugin.json）。
        worker: None,
        domain_packs: Vec::new(),
    };
    Ok(manifest)
}

/// 解析 SKILL.md 内容 → (description, prompt_body)
///
/// - 有 YAML frontmatter（首尾 `---` 分隔）：
///   * `description` 取自 frontmatter 的 `description:` 字段
///   * `prompt_body` 为 frontmatter 之后的 markdown 正文（运行时 AgentNode.system_prompt）
/// - 无 frontmatter：
///   * `description` 为空串（由调用方用占位文本兜底）
///   * `prompt_body` 为整个文件内容
fn parse_skill_md(content: &str) -> (String, Option<String>) {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|l| l.trim()) == Some("---")
        && let Some(end_idx) =
            lines.iter().enumerate().skip(1).find(|(_, l)| l.trim() == "---").map(|(i, _)| i)
    {
        let frontmatter = lines[1..end_idx].join("\n");
        let body_start = end_idx + 1;
        let body = lines[body_start..].join("\n").trim().to_string();
        let parsed = parse_yaml_frontmatter(&frontmatter);
        let description =
            parsed.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let prompt_body = if body.is_empty() { None } else { Some(body) };
        return (description, prompt_body);
    }
    let trimmed = content.trim().to_string();
    (
        "".to_string(),
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        },
    )
}

fn extract_yaml_frontmatter(contents: &str) -> Option<String> {
    let lines: Vec<&str> = contents.lines().collect();
    if lines.first().map(|l| l.trim()) != Some("---") {
        return None;
    }
    let mut end_idx = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            end_idx = Some(i);
            break;
        }
    }
    let end = end_idx?;
    Some(lines[1..end].join("\n"))
}

#[derive(Debug, Clone)]
enum YamlValue {
    String(String),
    Bool(bool),
    Array(Vec<YamlValue>),
}

impl YamlValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            YamlValue::String(s) => Some(s),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            YamlValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<&Vec<YamlValue>> {
        match self {
            YamlValue::Array(arr) => Some(arr),
            _ => None,
        }
    }
}

fn parse_yaml_frontmatter(frontmatter: &str) -> std::collections::HashMap<String, YamlValue> {
    let mut result = std::collections::HashMap::new();
    let mut current_key: Option<String> = None;
    let mut current_array: Vec<YamlValue> = Vec::new();

    for line in frontmatter.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            if let Some(key) = current_key.take()
                && !current_array.is_empty()
            {
                result.insert(key, YamlValue::Array(std::mem::take(&mut current_array)));
            }
            continue;
        }

        if let Some(stripped) = trimmed.strip_prefix("- ") {
            if current_key.is_some() {
                let item = parse_yaml_scalar(stripped.trim());
                current_array.push(item);
            }
            continue;
        }

        if let Some(key) = current_key.take()
            && !current_array.is_empty()
        {
            result.insert(key, YamlValue::Array(std::mem::take(&mut current_array)));
        }

        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim().to_string();
            let value_part = value.trim();
            if value_part.is_empty() {
                current_key = Some(key);
                current_array.clear();
            } else {
                result.insert(key, parse_yaml_scalar(value_part));
            }
        }
    }

    if let Some(key) = current_key.take()
        && !current_array.is_empty()
    {
        result.insert(key, YamlValue::Array(current_array));
    }

    result
}

fn parse_yaml_scalar(value: &str) -> YamlValue {
    let unquoted = value.trim_matches('"').trim_matches('\'');
    if unquoted.eq_ignore_ascii_case("true") {
        YamlValue::Bool(true)
    } else if unquoted.eq_ignore_ascii_case("false") {
        YamlValue::Bool(false)
    } else {
        YamlValue::String(unquoted.to_string())
    }
}

fn load_manifest_from_path(
    root: &Path,
    manifest_path: &Path,
) -> Result<PluginManifest, PluginError> {
    let contents = fs::read_to_string(manifest_path).map_err(|error| {
        PluginError::NotFound(format!(
            "plugin manifest not found at {}: {error}",
            manifest_path.display()
        ))
    })?;
    let raw_json: Value = serde_json::from_str(&contents)?;
    let compatibility_errors = detect_claude_code_manifest_contract_gaps(&raw_json);
    if !compatibility_errors.is_empty() {
        return Err(PluginError::ManifestValidation(compatibility_errors));
    }
    let raw_manifest: RawPluginManifest = serde_json::from_value(raw_json)?;
    build_plugin_manifest(root, raw_manifest)
}

pub(crate) fn detect_claude_code_manifest_contract_gaps(
    raw_manifest: &Value,
) -> Vec<PluginManifestValidationError> {
    let Some(root) = raw_manifest.as_object() else {
        return Vec::new();
    };

    let mut errors = Vec::new();

    if root
        .get("commands")
        .and_then(Value::as_array)
        .is_some_and(|commands| commands.iter().any(Value::is_string))
    {
        errors.push(PluginManifestValidationError::UnsupportedManifestContract {
            detail: "plugin manifest field `commands` uses Claude Code-style directory globs; `claw` slash dispatch is still built-in and does not load plugin slash command markdown files.".to_string(),
        });
    }

    if let Some(hooks) = root.get("hooks").and_then(Value::as_object) {
        for hook_name in hooks.keys() {
            if !matches!(hook_name.as_str(), "PreToolUse" | "PostToolUse" | "PostToolUseFailure") {
                errors.push(PluginManifestValidationError::UnsupportedManifestContract {
                    detail: format!(
                        "plugin hook `{hook_name}` uses the Claude Code lifecycle contract; `claw` plugins currently support only PreToolUse, PostToolUse, and PostToolUseFailure."
                    ),
                });
            }
        }
    }

    errors
}

fn plugin_manifest_path(root: &Path) -> Result<PathBuf, PluginError> {
    let direct_path = root.join(MANIFEST_FILE_NAME);
    if direct_path.exists() {
        return Ok(direct_path);
    }

    let packaged_path = root.join(MANIFEST_RELATIVE_PATH);
    if packaged_path.exists() {
        return Ok(packaged_path);
    }

    // Claude Code 兼容：检查 SKILL.md
    let skill_md_path = root.join(SKILL_MD_FILE_NAME);
    if skill_md_path.exists() {
        return Ok(skill_md_path);
    }

    Err(PluginError::NotFound(format!(
        "plugin manifest not found at {} or {}",
        direct_path.display(),
        packaged_path.display()
    )))
}

fn build_plugin_manifest(
    root: &Path,
    raw: RawPluginManifest,
) -> Result<PluginManifest, PluginError> {
    let mut errors = Vec::new();

    validate_required_manifest_field("name", &raw.name, &mut errors);
    validate_required_manifest_field("version", &raw.version, &mut errors);
    validate_required_manifest_field("description", &raw.description, &mut errors);

    let permissions = build_manifest_permissions(&raw.permissions, &mut errors);
    validate_command_entries(root, raw.hooks.pre_tool_use.iter(), "hook", &mut errors);
    validate_command_entries(root, raw.hooks.post_tool_use.iter(), "hook", &mut errors);
    validate_command_entries(root, raw.hooks.post_tool_use_failure.iter(), "hook", &mut errors);
    validate_command_entries(root, raw.lifecycle.init.iter(), "lifecycle command", &mut errors);
    validate_command_entries(root, raw.lifecycle.shutdown.iter(), "lifecycle command", &mut errors);
    let tools = build_manifest_tools(root, raw.tools, &mut errors);
    let commands = build_manifest_commands(root, raw.commands, &mut errors);
    validate_dashboard_panels(&raw.dashboard_panels, &mut errors);
    // UI 贡献的声明面（任务 #149 第 2 步）：`ui.json` 是本插件 schema 的**唯一**归属来源。
    // 先读成 key 集合，再让 `pages[]` / `taskSources[]` 做 membership 校验 —— 顺序不能反：
    // 没有集合就只能「不校验」，等于把跨插件/跨来源引用放行。
    let ui_keys = load_declared_ui_keys(root, &mut errors);
    let pages = build_manifest_pages(raw.pages, &ui_keys, &mut errors);
    // 任务源自洽性判据：`command` 必须指向**同一份清单**里已声明的命名命令。
    // 用已校验的 `commands`（而非 raw）取名 ⇒ 指向一条本身非法的命令也算「不存在」，
    // 不会出现「任务源指向一条装不出来的命令」这种只在运行期才暴露的形状。
    let command_names: BTreeSet<&str> = commands.iter().map(|c| c.name.as_str()).collect();
    let task_sources =
        build_manifest_task_sources(raw.task_sources, &command_names, &ui_keys, &mut errors);
    // 卡片声明的自洽性判据与任务源同源（命令 membership + ui.json membership），
    // 额外多一条**写权限闸**：`mutating` 动作要求 `permissions[]` 含 `write`。
    // 权限在这里判而不在前端：`PluginSummaryDto` 不带 permissions，前端无从判断。
    let task_cards = build_manifest_task_cards(
        raw.task_cards,
        &command_names,
        &ui_keys,
        &permissions,
        &mut errors,
    );

    if !errors.is_empty() {
        return Err(PluginError::ManifestValidation(errors));
    }

    Ok(PluginManifest {
        name: raw.name,
        version: raw.version,
        description: raw.description,
        author: raw.author,
        permissions,
        default_enabled: raw.default_enabled,
        hooks: raw.hooks,
        lifecycle: raw.lifecycle,
        tools,
        commands,
        scenarios: raw.scenarios,
        mcp_servers: raw
            .mcp_servers
            .into_iter()
            .map(|r| PluginMcpServer {
                name: r.name,
                command: r.command,
                args: r.args,
                env: r.env,
                cwd: r.cwd,
            })
            .collect(),
        skills: raw
            .skills
            .into_iter()
            .map(|r| PluginSkillEntry { name: r.name, path: r.path })
            .collect(),
        agents: raw
            .agents
            .into_iter()
            .map(|r| PluginAgentDefInternal {
                agent_type: r.agent_type,
                description: r.description,
                tools: r.tools,
                disallowed_tools: r.disallowed_tools,
                model: r.model,
                background: r.background,
                system_prompt: r.system_prompt,
            })
            .collect(),
        dashboard_panels: raw.dashboard_panels,
        pages,
        task_sources,
        task_cards,
        dependencies: raw.dependencies,
        integrity: raw.integrity,
        capabilities: raw.capabilities,
        worker: raw.worker,
        domain_packs: raw.domain_packs,
    })
}

fn validate_required_manifest_field(
    field: &'static str,
    value: &str,
    errors: &mut Vec<PluginManifestValidationError>,
) {
    if value.trim().is_empty() {
        errors.push(PluginManifestValidationError::EmptyField { field });
    }
}

fn build_manifest_permissions(
    permissions: &[String],
    errors: &mut Vec<PluginManifestValidationError>,
) -> Vec<PluginPermission> {
    let mut seen = BTreeSet::new();
    let mut validated = Vec::new();

    for permission in permissions {
        let permission = permission.trim();
        if permission.is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "permission",
                field: "value",
                name: None,
            });
            continue;
        }
        if !seen.insert(permission.to_string()) {
            errors.push(PluginManifestValidationError::DuplicatePermission {
                permission: permission.to_string(),
            });
            continue;
        }
        match PluginPermission::parse(permission) {
            Some(permission) => validated.push(permission),
            None => errors.push(PluginManifestValidationError::InvalidPermission {
                permission: permission.to_string(),
            }),
        }
    }

    validated
}

fn build_manifest_tools(
    root: &Path,
    tools: Vec<RawPluginToolManifest>,
    errors: &mut Vec<PluginManifestValidationError>,
) -> Vec<PluginToolManifest> {
    let mut seen = BTreeSet::new();
    let mut validated = Vec::new();

    for tool in tools {
        let name = tool.name.trim().to_string();
        if name.is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "tool",
                field: "name",
                name: None,
            });
            continue;
        }
        if !seen.insert(name.clone()) {
            errors.push(PluginManifestValidationError::DuplicateEntry { kind: "tool", name });
            continue;
        }
        if tool.description.trim().is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "tool",
                field: "description",
                name: Some(name.clone()),
            });
        }
        if tool.command.trim().is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "tool",
                field: "command",
                name: Some(name.clone()),
            });
        } else {
            validate_command_entry(root, &tool.command, "tool", errors);
        }
        if !tool.input_schema.is_object() {
            errors.push(PluginManifestValidationError::InvalidToolInputSchema {
                tool_name: name.clone(),
            });
        }
        let Some(required_permission) =
            PluginToolPermission::parse(tool.required_permission.trim())
        else {
            errors.push(PluginManifestValidationError::InvalidToolRequiredPermission {
                tool_name: name.clone(),
                permission: tool.required_permission.trim().to_string(),
            });
            continue;
        };

        validated.push(PluginToolManifest {
            name,
            description: tool.description,
            input_schema: tool.input_schema,
            command: tool.command,
            args: tool.args,
            required_permission,
        });
    }

    validated
}

/// 命令帧通道错误 → 宿主错误的归类（任务 #49）。
///
/// 只分两档：[`WorkerError::Timeout`]（等待被放弃，调用方可按预算重试）落
/// [`PluginError::CommandTimeout`]；其余（传输断裂、worker 错误帧自带协议码、回执形状
/// 非法等）统一 [`PluginError::CommandFailed`] —— WorkerError 的 Display 原文保留
/// `code: message` 前缀（`exchange_with_timeout` 的既有格式），上层据此归类错误码。
fn command_worker_error_to_plugin_error(error: WorkerError) -> PluginError {
    match error {
        WorkerError::Timeout { .. } => PluginError::CommandTimeout(error.to_string()),
        other => PluginError::CommandFailed(other.to_string()),
    }
}

/// 命令帧错误是否为「worker **未实现** `ops::COMMAND`」的派发前能力拒绝（任务 #59 修 M1）。
///
/// 判据**只读结构化错误码字段** [`WorkerError::error_code`]（任务 #73 S-1）——
/// 旧写法取 `WorkerError::Transport.reason` 文本首个 `:` 之前的段，任何人在信封成形处
/// 加前缀 / 换分隔符 / 把码挪到尾部都会让首段变成中文句子 ⇒ **回退整条静默消失**
/// （= M1 原病复发），而唯一会红的锁在子进程门控内、CI 恒 SKIP（审查稿 A2.3 / S-2）。
/// 文本判据已删除，不留双判据。
fn command_error_is_unknown_op(error: &WorkerError) -> bool {
    error.error_code() == Some(error_codes::PLUGIN_UNKNOWN_OP)
}

/// 命令帧错误是否为「**worker 进程已不在** ⇒ 帧从未送达」（任务 #73 修 M-1）。
///
/// 兑现 [`PluginManager::execute_plugin_command`] 契约里「worker 中途异常退出也退回原通道」
/// 那半句 —— 此前盘上没有这条路：`loaded_workers` 的表项只在 enable/unload 时增删，
/// 子进程自己死了表项还在，命令走通道一拿到 transport 错误、不命中 `UNKNOWN_OP` ⇒
/// **凭空失败**（= M1 换了触发条件）。
///
/// ⚠ 与 [`command_error_is_seam_suspect`] **分开判**（派单硬约束）：
/// 「进程不在」= 帧没出去 = 零副作用 = **回退**；
/// 「进程在但曾超时」= 连接被实证装死 = **拒绝**（不回退，等显式 unload/disable 复位）。
/// 把两者塞进同一个 bool 正是 M-1/M-2/M-3 的共同根（PLAN §26.3）。
fn command_error_is_worker_gone(error: &WorkerError) -> bool {
    matches!(error, WorkerError::NotDelivered { .. })
}

/// 该次失败是否出自「接缝脏闸」（进程还活着，只是曾被实证装死）。
///
/// 单列出来是为了**负向**锁定：`command_error_allows_fallback` 必须对它返回 `false`，
/// 否则「脏连接上的命令」会被回退成 spawn，等于把 D1 的止血又换回「两条通道各等一轮」。
/// 生产路径不读它（拒绝发生在 `worker.rs::execute_command` 里，压根不会走到判据），
/// 故只在测试构建里编译 —— 不加 `cfg(test)` 会成为 dead_code。
#[cfg(test)]
fn command_error_is_seam_suspect(error: &WorkerError) -> bool {
    error.error_code() == Some(error_codes::SEAM_PROVIDER_UNAVAILABLE)
}

/// 命名命令能否从通道一回退到通道二（任务 #73 收口后的**唯一**回退判据入口）。
///
/// = 「旧 worker 明确回了能力拒绝」∪「worker 进程已不在、帧未送达」，两条各自有独立的
/// 零副作用依据（见上面两个函数），其余一律不回退。
fn command_error_allows_fallback(error: &WorkerError) -> bool {
    command_error_is_unknown_op(error) || command_error_is_worker_gone(error)
}

/// 命令回执 `value` 的字符串输出口径（与 spawn 通道的 stdout 原文区分开）。
///
/// JSON 文本块（worker 侧把「纯文本输出」用字符串值承载时）原样返回，其余一律
/// 序列化为 JSON 文本。序列化对来自帧内的 [`Value`] 不会失败（无环、键均为字符串），
/// 兜底分支仅防御性存在。
fn command_value_to_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".to_string()),
    }
}

/// 命令声明的 `requiredPermission` 落定值。
///
/// 未声明 ⇒ `ReadOnly`（与加字段前逐字节一致，现有插件照常加载与执行）。
/// 声明了但解析不出 ⇒ 报错：清单加载链上 `build_manifest_commands` 已拦过一遍，
/// 走到这里还拿到非法值说明绕过了校验（如手工构造的 registry），fail-closed 而不是
/// 静默降成 `ReadOnly` —— 静默降档等于把「写权限命令」当只读放行。
fn command_required_permission(
    entry: &PluginCommandManifest,
) -> Result<PluginToolPermission, PluginError> {
    match entry.required_permission.as_deref() {
        // 未声明 ⇒ ReadOnly，与加字段前逐字节一致（现有插件照常执行）。
        None => Ok(PluginToolPermission::ReadOnly),
        Some(label) => PluginToolPermission::parse(label.trim()).ok_or_else(|| {
            PluginError::InvalidManifest(format!(
                "插件命令 `{}` 的 requiredPermission `{label}` 非法（须为 read-only / workspace-write / danger-full-access）",
                entry.name
            ))
        }),
    }
}

/// 命令声明的 `inputSchema` 落定值（用于构造执行链上的 `PluginToolDefinition`）。
///
/// 未声明 ⇒ 空 object，与加字段前写死的 `Value::Object(Default::default())` 同一个值。
fn resolved_command_input_schema(entry: &PluginCommandManifest) -> Value {
    entry.input_schema.clone().unwrap_or_else(|| Value::Object(Map::new()))
}

/// JSON 值的类型名，只用于报错文案（`serde_json::Value::type_str` 的等价简写）。
fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn build_manifest_commands(
    root: &Path,
    commands: Vec<PluginCommandManifest>,
    errors: &mut Vec<PluginManifestValidationError>,
) -> Vec<PluginCommandManifest> {
    let mut seen = BTreeSet::new();
    let mut validated = Vec::new();

    for command in commands {
        let name = command.name.trim().to_string();
        if name.is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "command",
                field: "name",
                name: None,
            });
            continue;
        }
        if !seen.insert(name.clone()) {
            errors.push(PluginManifestValidationError::DuplicateEntry { kind: "command", name });
            continue;
        }
        if command.description.trim().is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "command",
                field: "description",
                name: Some(name.clone()),
            });
        }
        if command.command.trim().is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "command",
                field: "command",
                name: Some(name.clone()),
            });
        } else {
            validate_command_entry(root, &command.command, "command", errors);
        }
        // 以下两项**只对「声明了」的插件生效**：缺省键不报错，保证现有插件照常加载
        // （与 tool 侧不同 —— tool 的 inputSchema/requiredPermission 一直是必填）。
        // 显式声明成空串/空白也算「声明了非法值」，与 tool 侧口径一致。
        if command.input_schema.as_ref().is_some_and(|schema| !schema.is_object()) {
            errors.push(PluginManifestValidationError::InvalidCommandInputSchema {
                command_name: name.clone(),
            });
        }
        let invalid_permission = command
            .required_permission
            .as_deref()
            .filter(|label| PluginToolPermission::parse(label.trim()).is_none());
        if let Some(permission) = invalid_permission {
            errors.push(PluginManifestValidationError::InvalidCommandRequiredPermission {
                command_name: name.clone(),
                permission: permission.trim().to_string(),
            });
            continue;
        }

        validated.push(command);
    }

    validated
}

/// 逐页声明（`pages[]`）的校验与转写。
///
/// 形状照抄 [`build_manifest_tools`]：raw 收字符串 → 校验 → 转成强类型清单，
/// 于是「装载形态缺失 / 非法」都是**带页 id 的清单校验错误**，而不是 serde 的行列号。
///
/// 与面板声明（[`validate_dashboard_panels`]）刻意保持同一口径：**不做 I/O** ——
/// `entry` 只验形状不验存在性，存在性由读取期的 canonicalize + 包含性把关
/// （见 [`PluginManager::resolve_page_asset`]）。两边若一边验存在、一边不验，
/// 同一份资产声明就会出现「面板能装、页面不能装」这种无法解释的分叉。
fn build_manifest_pages(
    pages: Vec<RawPluginPage>,
    ui_keys: &BTreeSet<String>,
    errors: &mut Vec<PluginManifestValidationError>,
) -> Vec<PluginPage> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut validated = Vec::new();

    for page in pages {
        let id = page.id.trim().to_string();
        if id.is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "page",
                field: "id",
                name: None,
            });
            continue;
        }
        if !seen.insert(id.clone()) {
            errors.push(PluginManifestValidationError::DuplicateEntry { kind: "page", name: id });
            continue;
        }
        if page.title.trim().is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "page",
                field: "title",
                name: Some(id.clone()),
            });
        }
        let entry_missing = page.entry.trim().is_empty();
        if entry_missing {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "page",
                field: "entry",
                name: Some(id.clone()),
            });
        }
        // 装载形态是 §0.12.5 裁定 3「不许混用却不声明」的落点：空白串与缺键同判「未声明」。
        let declared = page.loading.as_deref().map(str::trim).filter(|value| !value.is_empty());
        let loading = match declared {
            None => {
                errors.push(PluginManifestValidationError::EmptyEntryField {
                    kind: "page",
                    field: "loading",
                    name: Some(id.clone()),
                });
                continue;
            },
            Some(value) => match PluginPageLoading::parse(value) {
                Some(loading) => loading,
                None => {
                    errors.push(PluginManifestValidationError::InvalidPageLoading {
                        page: id.clone(),
                        loading: value.to_string(),
                    });
                    continue;
                },
            },
        };
        // 资产形状白名单**只适用于 bundle**：schema 页的 `entry` 是本插件 `ui.json` 的
        // key（形如 `order-form`），按文件路径去验它必然被判成「缺扩展名」。
        // 空 `entry` 已单独报过（上面那条），这里不再叠报同一个缺陷 —— 与
        // `build_manifest_commands` 的「空值只报 EmptyEntryField」保持一致。
        if !entry_missing
            && loading == PluginPageLoading::Bundle
            && let Some(detail) = asset_entry_shape_error(&page.entry, "entry")
        {
            errors
                .push(PluginManifestValidationError::InvalidPageAsset { page: id.clone(), detail });
        }
        // schema 页的 `entry` 是**本插件** `ui.json` 的 key（任务 #149 第 2 步）。
        // 这条 membership 校验是今天缺失的那道归属闸：此前 `entry` 被前端直接当 schemaId
        // 送进加载器，插件可以指到宿主的、别的插件的、乃至用户自建的任意 schema。
        if !entry_missing && loading == PluginPageLoading::Schema {
            let key = page.entry.trim();
            if !ui_keys.contains(key) {
                errors.push(PluginManifestValidationError::PageSchemaKeyUnknown {
                    page: id.clone(),
                    key: key.to_string(),
                });
            }
        }

        validated.push(PluginPage { id, title: page.title, entry: page.entry, loading });
    }

    validated
}

/// 校验并转写 `plugin.json` 的 `taskSources[]`（任务 #148，PLAN §97.4）。
///
/// 三条硬判据，全部在**装载期**拦下，不留到任务中心点开才失败：
/// 1. `id` 形状 = 小写字母数字与 `-`（首字符须是字母数字），且**同一清单内唯一**。
///    源 id 会直接进宿主 `SourceId` 与 `runIdOf(source, id)` 的拼接，带 `:` / 空格 /
///    大写都会让「全局唯一 run id」这条不变量失效。
/// 2. `command` 必须是**同一份清单** `commands[]` 里已声明的命令（`command_names`）——
///    指一条不存在的命令 = 声明一个永远为空的源，比报错更坏。
/// 3. `title` 非空（纯文本，不走宿主 i18n；空标题会让任务中心源列表出现空标签）。
/// 4. `detailSchema`（任务 `#149` 第 2 步）若声明，必须是**本插件 `ui.json`** 里已声明的 key
///    （`ui_keys`）—— 声明面只能引用自己的 schema，跨插件引用写不出来。
///
/// ⚠ **不与宿主源清单比名字**：宿主源是前端常量，清单校验期看不到 ⇒ 重名裁决在消费侧
/// （`adapters/registry.ts::registerAdapter` 幂等，宿主先占位即胜出），与 B10-1 的
/// 「宿主优先」同一条判据。
fn build_manifest_task_sources(
    sources: Vec<RawPluginTaskSource>,
    command_names: &BTreeSet<&str>,
    ui_keys: &BTreeSet<String>,
    errors: &mut Vec<PluginManifestValidationError>,
) -> Vec<PluginTaskSource> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut validated = Vec::new();

    for source in sources {
        let id = source.id.trim().to_string();
        if id.is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "task source",
                field: "id",
                name: None,
            });
            continue;
        }
        if !is_declared_key_shape(&id) {
            errors.push(PluginManifestValidationError::InvalidTaskSourceId {
                source: id.clone(),
                detail: "只允许小写字母、数字与 `-`，且首字符须是字母或数字".to_string(),
            });
            continue;
        }
        if !seen.insert(id.clone()) {
            errors.push(PluginManifestValidationError::DuplicateEntry {
                kind: "task source",
                name: id,
            });
            continue;
        }
        if source.title.trim().is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "task source",
                field: "title",
                name: Some(id.clone()),
            });
            continue;
        }
        let command = source.command.trim().to_string();
        if command.is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "task source",
                field: "command",
                name: Some(id.clone()),
            });
            continue;
        }
        if !command_names.contains(command.as_str()) {
            errors.push(PluginManifestValidationError::TaskSourceCommandUnknown {
                source: id,
                command,
            });
            continue;
        }
        // 空串色名按「未声明」处理：与 loading / requiredPermission 的空白同判。
        let color = source.color.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
        // `detailSchema` 同理：空白串按「未声明」处理；非空则必须是本插件 `ui.json` 的 key。
        // 注意存的是 **key**（不是行 id）—— 拼 `plugin.{id}.{key}` 只在宿主侧发生，
        // 于是「引到别的插件的 schema」在声明面上就没有写法。
        let detail_schema =
            source.detail_schema.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        if let Some(key) = &detail_schema
            && !ui_keys.contains(key)
        {
            errors.push(PluginManifestValidationError::TaskSourceSchemaKeyUnknown {
                source: id.clone(),
                key: key.clone(),
            });
            continue;
        }
        validated.push(PluginTaskSource { id, title: source.title, command, color, detail_schema });
    }

    validated
}

/// 校验并转写 `plugin.json` 的 `taskCards[]`（任务 #216 前置 ②）。
///
/// 逐条对齐 [`build_manifest_task_sources`]，差异点写在下面（交付报告也会逐点列）。
///
/// 1. `id` 形状 = [`is_declared_key_shape`]（小写字母数字与 `-`，首字符须是字母数字，≤64），
///    **同一份清单内唯一**；重名 ⇒ **fail-closed**（leader 裁定 3）：报
///    `DuplicateEntry{kind: "task card"}`，整份 `plugin.json` 校验失败、插件装不上。
///    与 `task_sources` 的区别：源重名裁决在消费侧（宿主优先），卡片重名在装载期直接拒
///    —— 卡片键是前端注册表的身份，静默丢一张卡等于让用户看到「少了一张卡」却查不到原因。
/// 2. `title` 非空（纯文本，不走宿主 i18n）。
/// 3. `surface` 必须精确命中 [`PluginCardSurface::LABELS`]；缺失 / 表外值一律报错，不兜底。
/// 4. `summaryCommand` 必须是**同一份清单** `commands[]` 里已声明的命令（用已校验的 commands 取名，
///    指一条本身非法的命令也算「不存在」）。
/// 5. `detailSchema` 若声明，必须是**本插件 `ui.json`** 的 key（`ui_keys`）。
/// 6. `actions[]` 的 `id` 形状同上 + **同卡内唯一**，`command` 也必须是本包命令
///    （⇒ 宿主命令名在声明面上写不出来）。
/// 7. `actions[].mutating = true` ⇒ 清单 `permissions[]` 必须含 `write`
///    （[`PluginPermission::implies_write`]，`execute` / `subprocess_execution` 也算）。
///    只读动作不要求 `write`（leader 裁定 2）。
fn build_manifest_task_cards(
    cards: Vec<RawPluginTaskCard>,
    command_names: &BTreeSet<&str>,
    ui_keys: &BTreeSet<String>,
    permissions: &[PluginPermission],
    errors: &mut Vec<PluginManifestValidationError>,
) -> Vec<PluginTaskCard> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut validated = Vec::new();
    let write_allowed = permissions.iter().any(PluginPermission::implies_write);

    for card in cards {
        let id = card.id.trim().to_string();
        if id.is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "task card",
                field: "id",
                name: None,
            });
            continue;
        }
        if !is_declared_key_shape(&id) {
            errors.push(PluginManifestValidationError::InvalidTaskCardId {
                card: id.clone(),
                detail: "只允许小写字母、数字与 `-`，且首字符须是字母或数字".to_string(),
            });
            continue;
        }
        // fail-closed：重名不「先到先得」，而是报错让整份清单装不上（leader 裁定 3）。
        if !seen.insert(id.clone()) {
            errors.push(PluginManifestValidationError::DuplicateEntry {
                kind: "task card",
                name: id,
            });
            continue;
        }
        if card.title.trim().is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "task card",
                field: "title",
                name: Some(id.clone()),
            });
            continue;
        }
        let surface_label = card.surface.as_deref().unwrap_or_default().trim().to_string();
        let Some(surface) = PluginCardSurface::parse(&surface_label) else {
            errors.push(PluginManifestValidationError::InvalidTaskCardSurface {
                card: id,
                surface: surface_label,
            });
            continue;
        };
        let summary_command = card.summary_command.trim().to_string();
        if summary_command.is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "task card",
                field: "summaryCommand",
                name: Some(id.clone()),
            });
            continue;
        }
        if !command_names.contains(summary_command.as_str()) {
            errors.push(PluginManifestValidationError::TaskCardCommandUnknown {
                card: id,
                command: summary_command,
            });
            continue;
        }
        // 空白串按「未声明」处理（与 color / loading / detailSchema 同判）。
        let detail_schema =
            card.detail_schema.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        if let Some(key) = &detail_schema
            && !ui_keys.contains(key)
        {
            errors.push(PluginManifestValidationError::TaskCardSchemaKeyUnknown {
                card: id.clone(),
                key: key.clone(),
            });
            continue;
        }
        let tone = card.tone.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());

        // 动作逐条校验。任一条非法 ⇒ 整张卡被丢弃（下面的 errors 非空即整份清单校验失败，
        // 故「丢卡」只是不再往 validated 里塞，不会真的放行一份残缺声明）。
        let mut actions = Vec::with_capacity(card.actions.len());
        let mut seen_actions: BTreeSet<String> = BTreeSet::new();
        let mut actions_ok = true;
        for action in card.actions {
            let action_id = action.id.trim().to_string();
            let label = if action_id.is_empty() {
                format!("{id}#{}", actions.len())
            } else {
                format!("{id}.{action_id}")
            };
            if action_id.is_empty() || !is_declared_key_shape(&action_id) {
                errors.push(PluginManifestValidationError::InvalidTaskCardActionId {
                    card: id.clone(),
                    action: label,
                    detail: "只允许小写字母、数字与 `-`，且首字符须是字母或数字".to_string(),
                });
                actions_ok = false;
                continue;
            }
            if !seen_actions.insert(action_id.clone()) {
                errors.push(PluginManifestValidationError::DuplicateEntry {
                    kind: "task card action",
                    name: label,
                });
                actions_ok = false;
                continue;
            }
            if action.title.trim().is_empty() {
                errors.push(PluginManifestValidationError::EmptyEntryField {
                    kind: "task card action",
                    field: "title",
                    name: Some(label),
                });
                actions_ok = false;
                continue;
            }
            let command = action.command.trim().to_string();
            if command.is_empty() || !command_names.contains(command.as_str()) {
                errors.push(PluginManifestValidationError::TaskCardActionCommandUnknown {
                    card: id.clone(),
                    action: label,
                    command,
                });
                actions_ok = false;
                continue;
            }
            // 写权限闸（leader 裁定 2）：只对 `mutating: true` 生效，只读动作一概不要求 write。
            if action.mutating && !write_allowed {
                errors.push(PluginManifestValidationError::TaskCardActionWritePermissionMissing {
                    card: id.clone(),
                    action: label,
                });
                actions_ok = false;
                continue;
            }
            actions.push(PluginTaskCardAction {
                id: action_id,
                title: action.title,
                command,
                mutating: action.mutating,
            });
        }
        if !actions_ok {
            continue;
        }

        validated.push(PluginTaskCard {
            id,
            title: card.title,
            surface,
            summary_command,
            detail_schema,
            tone,
            order: card.order,
            actions,
        });
    }

    validated
}

/// 读取 `<root>/ui.json` 并返回其声明的 key 集合（任务 `#149` 第 2 步）。
///
/// 三种情形分开处置，不能混：
/// - **文件不存在** ⇒ 空集合（没声明任何 schema，`pages[]` / `taskSources[]` 也就无从引用）；
/// - **读取失败（非 NotFound）** ⇒ 清单校验错误（IO 错误不能静默当成「无声明」，
///   否则一次权限/句柄问题会让引用了 schema 的页面被误判成「引用了不存在的 key」）；
/// - **解析失败** ⇒ 清单校验错误，且错误来自 `ui_schema::parse` 的逐条定位。
fn load_declared_ui_keys(
    root: &Path,
    errors: &mut Vec<PluginManifestValidationError>,
) -> BTreeSet<String> {
    let path = root.join(crate::ui_schema::UI_SCHEMA_FILE_NAME);
    // #213：读取前先用 metadata 挡体积（避免把超大 ui.json 读进内存）。metadata 读不到时不在这里报错，
    // 交给下面的读取分支保持既有 NotFound / IO 错误语义。
    if let Ok(meta) = fs::metadata(&path)
        && meta.len() > crate::ui_schema::UI_SCHEMA_FILE_MAX_BYTES
    {
        errors.push(PluginManifestValidationError::InvalidUiSchemaDeclaration {
            detail: format!(
                "{} 体积 {} 字节超过上限 {} 字节",
                path.display(),
                meta.len(),
                crate::ui_schema::UI_SCHEMA_FILE_MAX_BYTES
            ),
        });
        return BTreeSet::new();
    }
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return BTreeSet::new();
        },
        Err(error) => {
            errors.push(PluginManifestValidationError::InvalidUiSchemaDeclaration {
                detail: format!("{} 读取失败：{error}", path.display()),
            });
            return BTreeSet::new();
        },
    };
    match crate::ui_schema::parse(&source) {
        Ok(entries) => entries.into_iter().map(|entry| entry.key).collect(),
        Err(detail) => {
            errors.push(PluginManifestValidationError::InvalidUiSchemaDeclaration {
                detail: format!("{} 非法：{detail}", path.display()),
            });
            BTreeSet::new()
        },
    }
}

fn validate_command_entries<'a>(
    root: &Path,
    entries: impl Iterator<Item = &'a String>,
    kind: &'static str,
    errors: &mut Vec<PluginManifestValidationError>,
) {
    for entry in entries {
        validate_command_entry(root, entry, kind, errors);
    }
}

fn validate_command_entry(
    root: &Path,
    entry: &str,
    kind: &'static str,
    errors: &mut Vec<PluginManifestValidationError>,
) {
    if entry.trim().is_empty() {
        errors.push(PluginManifestValidationError::EmptyEntryField {
            kind,
            field: "command",
            name: None,
        });
        return;
    }
    if is_literal_command(entry) {
        return;
    }

    let path = if Path::new(entry).is_absolute() {
        PathBuf::from(entry)
    } else {
        root.join(entry)
    };
    if !path.exists() {
        errors.push(PluginManifestValidationError::MissingPath { kind, path });
    } else if !path.is_file() {
        errors.push(PluginManifestValidationError::PathIsDirectory { kind, path });
    }
}

fn resolve_hooks(root: &Path, hooks: &PluginHooks) -> PluginHooks {
    PluginHooks {
        pre_tool_use: hooks
            .pre_tool_use
            .iter()
            .map(|entry| resolve_hook_entry(root, entry))
            .collect(),
        post_tool_use: hooks
            .post_tool_use
            .iter()
            .map(|entry| resolve_hook_entry(root, entry))
            .collect(),
        post_tool_use_failure: hooks
            .post_tool_use_failure
            .iter()
            .map(|entry| resolve_hook_entry(root, entry))
            .collect(),
    }
}

fn resolve_lifecycle(root: &Path, lifecycle: &PluginLifecycle) -> PluginLifecycle {
    PluginLifecycle {
        init: lifecycle.init.iter().map(|entry| resolve_hook_entry(root, entry)).collect(),
        shutdown: lifecycle.shutdown.iter().map(|entry| resolve_hook_entry(root, entry)).collect(),
    }
}

fn resolve_tools(
    root: &Path,
    plugin_id: &str,
    plugin_name: &str,
    tools: &[PluginToolManifest],
) -> Vec<PluginTool> {
    tools
        .iter()
        .map(|tool| {
            PluginTool::new(
                plugin_id,
                plugin_name,
                PluginToolDefinition {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    input_schema: tool.input_schema.clone(),
                },
                resolve_hook_entry(root, &tool.command),
                tool.args.clone(),
                tool.required_permission,
                Some(root.to_path_buf()),
            )
        })
        .collect()
}

pub fn validate_hook_paths(root: Option<&Path>, hooks: &PluginHooks) -> Result<(), PluginError> {
    let Some(root) = root else {
        return Ok(());
    };
    for entry in hooks
        .pre_tool_use
        .iter()
        .chain(hooks.post_tool_use.iter())
        .chain(hooks.post_tool_use_failure.iter())
    {
        validate_command_path(root, entry, "hook")?;
    }
    Ok(())
}

pub fn validate_lifecycle_paths(
    root: Option<&Path>,
    lifecycle: &PluginLifecycle,
) -> Result<(), PluginError> {
    let Some(root) = root else {
        return Ok(());
    };
    for entry in lifecycle.init.iter().chain(lifecycle.shutdown.iter()) {
        validate_command_path(root, entry, "lifecycle command")?;
    }
    Ok(())
}

pub fn validate_tool_paths(root: Option<&Path>, tools: &[PluginTool]) -> Result<(), PluginError> {
    let Some(root) = root else {
        return Ok(());
    };
    for tool in tools {
        validate_command_path(root, &tool.command, "tool")?;
    }
    Ok(())
}

fn validate_command_path(root: &Path, entry: &str, kind: &str) -> Result<(), PluginError> {
    if is_literal_command(entry) {
        return Ok(());
    }
    let path = if Path::new(entry).is_absolute() {
        PathBuf::from(entry)
    } else {
        root.join(entry)
    };
    if !path.exists() {
        return Err(PluginError::InvalidManifest(format!(
            "{kind} path `{}` does not exist",
            path.display()
        )));
    }
    if !path.is_file() {
        return Err(PluginError::InvalidManifest(format!(
            "{kind} path `{}` must point to a file",
            path.display()
        )));
    }
    Ok(())
}

fn resolve_hook_entry(root: &Path, entry: &str) -> String {
    if is_literal_command(entry) {
        entry.to_string()
    } else {
        root.join(entry).display().to_string()
    }
}

fn is_literal_command(entry: &str) -> bool {
    !entry.starts_with("./") && !entry.starts_with("../") && !Path::new(entry).is_absolute()
}

/// 解析 worker 可执行文件路径：必须是**安装目录内的相对路径**。
///
/// 与 hook / tool 路径（[`validate_command_path`]）的宽松口径**有意不同**：那两者是
/// 「命令字符串」，可以是系统程序；worker 是插件自己的二进制，只应来自插件安装目录 ——
/// 否则声明里一个绝对路径就能让宿主去跑任意可执行文件。
fn resolve_worker_program(install_path: &Path, entry: &str) -> Result<PathBuf, PluginError> {
    let relative = Path::new(entry);
    if relative.is_absolute() {
        return Err(PluginError::InvalidManifest(format!(
            "worker program `{entry}` must be relative to the plugin install directory"
        )));
    }
    if relative.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(PluginError::InvalidManifest(format!(
            "worker program `{entry}` must not escape the plugin install directory"
        )));
    }
    let path = install_path.join(relative);
    if !path.is_file() {
        return Err(PluginError::InvalidManifest(format!(
            "worker program `{}` does not exist or is not a file",
            path.display()
        )));
    }
    Ok(path)
}

pub fn run_lifecycle_commands(
    metadata: &PluginMetadata,
    lifecycle: &PluginLifecycle,
    phase: &str,
    commands: &[String],
) -> Result<(), PluginError> {
    if lifecycle.is_empty() || commands.is_empty() {
        return Ok(());
    }

    for command in commands {
        // SECURITY: 不再通过 `cmd /C` 或 `sh -lc` 执行命令，避免 shell 解析
        // `&`、`|`、`$()`、反引号等特殊字符导致命令注入。
        // 将命令字符串按空白分割为程序 + 参数数组，直接 exec。
        // 插件 lifecycle 命令来自插件配置文件（plugin.json），虽然用户安装时会确认，
        // 但配置文件可能被篡改，因此仍需防护。
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let executable = parts[0];
        let args = &parts[1..];

        let mut process = Command::new(executable);
        process.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        if let Some(root) = &metadata.root {
            process.current_dir(root);
        }
        let output = process.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(PluginError::CommandFailed(format!(
                "plugin `{}` {} failed for `{}`: {}",
                metadata.id,
                phase,
                command,
                if stderr.is_empty() {
                    format!("exit status {}", output.status)
                } else {
                    stderr
                }
            )));
        }
    }

    Ok(())
}

/// 在沙箱约束下执行 lifecycle 命令（init / shutdown）。
///
/// 与 [`run_lifecycle_commands`] 的区别：
/// 1. 执行前调用 [`check_subprocess_permission`]，未声明 `subprocess_execution`
///    权限时直接返回 [`PluginError::PermissionDenied`]，不启动子进程。
/// 2. 通过 [`apply_env_to_command`] 对子进程 ENV 做白名单过滤，屏蔽
///    API Key / Token / Secret 等敏感变量。
///
/// 原始 [`run_lifecycle_commands`] 保持不变以维持向后兼容；需要沙箱隔离的
/// 调用方（如基于 manifest 权限构建沙箱后执行 lifecycle）应使用本函数。
pub fn run_lifecycle_commands_sandboxed(
    metadata: &PluginMetadata,
    lifecycle: &PluginLifecycle,
    phase: &str,
    commands: &[String],
    sandbox: &SandboxConfig,
) -> Result<(), PluginError> {
    if lifecycle.is_empty() || commands.is_empty() {
        return Ok(());
    }

    // 沙箱检查：未声明 subprocess_execution 权限时禁止执行 lifecycle 命令
    check_subprocess_permission(sandbox)?;

    for command in commands {
        // SECURITY: 同 run_lifecycle_commands，直接 exec 避免 shell 注入
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let executable = parts[0];
        let args = &parts[1..];

        let mut process = Command::new(executable);
        process.args(args);
        // 沙箱：ENV 白名单过滤（env_clear + 回填白名单变量）
        apply_env_to_command(&mut process, sandbox);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        if let Some(root) = &metadata.root {
            process.current_dir(root);
        }
        let output = process.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(PluginError::CommandFailed(format!(
                "plugin `{}` {} failed for `{}`: {}",
                metadata.id,
                phase,
                command,
                if stderr.is_empty() {
                    format!("exit status {}", output.status)
                } else {
                    stderr
                }
            )));
        }
    }

    Ok(())
}

fn resolve_local_source(source: &str) -> Result<PathBuf, PluginError> {
    let path = PathBuf::from(source);
    if path.exists() {
        Ok(path)
    } else {
        Err(PluginError::NotFound(format!("plugin source `{source}` was not found")))
    }
}

fn looks_like_npm_spec(source: &str) -> bool {
    // 排除明显是其他格式的源
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("git@")
        || source.starts_with('/')
        || source.starts_with('.')
        || source.starts_with('\\')
        || source.contains("://")
    {
        return false;
    }
    // npm 包：@scope/name 或 name@version 或裸包名
    // 裸包名不含路径分隔符 / 和 \
    if source.starts_with('@') || source.contains('@') {
        return true;
    }
    // 裸包名：不含路径分隔符且不含空格
    !source.contains('/') && !source.contains('\\') && !source.contains(' ')
}

pub(crate) fn parse_install_source(source: &str) -> Result<PluginInstallSource, PluginError> {
    // OpenClaw 包检测: openclaw:package-name
    if let Some(package_id) = source.strip_prefix("openclaw:") {
        let pkg = package_id.trim();
        if pkg.is_empty() {
            return Err(PluginError::InvalidManifest(
                "OpenClaw package ID must not be empty".to_string(),
            ));
        }
        return Ok(PluginInstallSource::OpenClaw { package_id: pkg.to_string() });
    }
    // npm 包检测
    if looks_like_npm_spec(source) {
        let (name, version) = parse_npm_package_spec(source);
        return Ok(PluginInstallSource::NpmPackage {
            name: name.to_string(),
            version: version.map(|v| v.to_string()),
        });
    }
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("git@")
        || Path::new(source)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("git"))
    {
        Ok(PluginInstallSource::GitUrl { url: source.to_string() })
    } else {
        Ok(PluginInstallSource::LocalPath { path: resolve_local_source(source)? })
    }
}

fn materialize_source(
    source: &PluginInstallSource,
    temp_root: &Path,
    npm_registry: Option<&Arc<dyn NpmRegistryService>>,
) -> Result<PathBuf, PluginError> {
    fs::create_dir_all(temp_root)?;
    match source {
        PluginInstallSource::LocalPath { path } => Ok(path.clone()),
        PluginInstallSource::GitUrl { url } => {
            // URL 合法性校验：阻止 `git clone` 把 `--upload-pack=...` 之类的 flag
            // 误当作 URL 参数解析（任何以 `-` 开头的 token 都不是合法 URL）。
            if url.is_empty() || url.starts_with('-') {
                return Err(PluginError::InvalidManifest(format!(
                    "git url must not be empty or start with '-': `{url}`"
                )));
            }
            static MATERIALIZE_COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = MATERIALIZE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time is after UNIX epoch")
                .as_nanos();
            let destination = temp_root.join(format!("plugin-{nanos}-{unique}"));
            // `--` 终止 git 的 option 解析，后续 token 强制视为位置参数（URL/dest）
            let output = Command::new("git")
                .arg("clone")
                .arg("--depth")
                .arg("1")
                .arg("--")
                .arg(url)
                .arg(&destination)
                .output()?;
            if !output.status.success() {
                return Err(PluginError::CommandFailed(format!(
                    "git clone failed for `{url}`: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            Ok(destination)
        },
        PluginInstallSource::NpmPackage { name, version } => {
            let name = name.clone();
            let version = version.clone();
            let dest = temp_root.join(format!("npm-{}", sanitize_plugin_id(&name)));
            let dest_clone = dest.clone();
            let service = npm_registry.cloned();
            match service {
                Some(registry) => {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| {
                            PluginError::CommandFailed(format!(
                                "Failed to create runtime for npm download: {e}"
                            ))
                        })?;
                    rt.block_on(async {
                        registry
                            .download_package(&name, version.as_deref(), &dest)
                            .await
                            .map_err(PluginError::CommandFailed)?;
                        Ok(dest_clone)
                    })
                },
                None => Err(PluginError::CommandFailed(
                    "NPM registry service is not configured".to_string(),
                )),
            }
        },
        PluginInstallSource::OpenClaw { package_id } => {
            let dest = temp_root.join(format!("openclaw-{}", sanitize_plugin_id(package_id)));
            let dest_clone = dest.clone();
            let pid = package_id.clone();
            let service = npm_registry.cloned();
            match service {
                Some(registry) => {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| {
                            PluginError::CommandFailed(format!(
                                "Failed to create runtime for OpenClaw plugin download: {e}"
                            ))
                        })?;
                    rt.block_on(async {
                        registry
                            .download_package(&pid, None, &dest)
                            .await
                            .map_err(PluginError::CommandFailed)?;
                        Ok(dest_clone)
                    })
                },
                None => Err(PluginError::CommandFailed(
                    "NPM registry service is not configured".to_string(),
                )),
            }
        },
    }
}

fn discover_plugin_dirs(root: &Path) -> Result<Vec<PathBuf>, PluginError> {
    match fs::read_dir(root) {
        Ok(entries) => {
            let mut paths = Vec::new();
            for entry in entries {
                let path = entry?.path();
                // 跳过 .bak 备份目录（update 遗留），避免被误当作插件
                if path.extension().is_some_and(|ext| ext == "bak") {
                    continue;
                }
                if path.is_dir() && plugin_manifest_path(&path).is_ok() {
                    paths.push(path);
                }
            }
            paths.sort();
            Ok(paths)
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(PluginError::Io(error)),
    }
}

fn plugin_id(name: &str, marketplace: &str) -> String {
    let normalized = name.trim().to_lowercase();
    format!("{normalized}@{marketplace}")
}

pub(crate) fn sanitize_plugin_id(plugin_id: &str) -> String {
    plugin_id
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | '@' | ':' => '-',
            other => other,
        })
        .collect()
}

fn describe_install_source(source: &PluginInstallSource) -> String {
    match source {
        PluginInstallSource::LocalPath { path } => path.display().to_string(),
        PluginInstallSource::GitUrl { url } => url.clone(),
        PluginInstallSource::NpmPackage { name, version } => match version {
            Some(version) => format!("{name}@{version}"),
            None => name.clone(),
        },
        PluginInstallSource::OpenClaw { package_id } => format!("openclaw:{package_id}"),
    }
}

fn unix_time_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("time should be after epoch").as_millis()
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<(), PluginError> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn update_settings_json(
    path: &Path,
    mut update: impl FnMut(&mut Map<String, Value>),
) -> Result<(), PluginError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut root = match fs::read_to_string(path) {
        Ok(contents) if !contents.trim().is_empty() => serde_json::from_str::<Value>(&contents)?,
        Ok(_) => Value::Object(Map::new()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(error) => return Err(PluginError::Io(error)),
    };

    let object = root.as_object_mut().ok_or_else(|| {
        PluginError::InvalidManifest(format!(
            "settings file {} must contain a JSON object",
            path.display()
        ))
    })?;
    update(object);
    fs::write(path, serde_json::to_string_pretty(&root)?)?;
    Ok(())
}

fn ensure_object<'a>(root: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    if !root.get(key).is_some_and(Value::is_object) {
        root.insert(key.to_string(), Value::Object(Map::new()));
    }
    root.get_mut(key).and_then(Value::as_object_mut).expect("object should exist")
}

/// Environment variable lock for test isolation.
/// Guards against concurrent modification of `CLAW_CONFIG_HOME`.
#[cfg(test)]
pub(crate) fn env_lock() -> &'static parking_lot::Mutex<()> {
    static ENV_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    &ENV_LOCK
}

fn version_satisfies(installed: &str, required: &str) -> bool {
    let installed_clean = strip_prerelease(installed);
    let required_clean = strip_prerelease(required);

    let installed_parts: Vec<u32> =
        installed_clean.split('.').filter_map(|s| s.parse().ok()).collect();
    let required_parts: Vec<u32> =
        required_clean.split('.').filter_map(|s| s.parse().ok()).collect();

    for i in 0..required_parts.len().max(installed_parts.len()) {
        let installed_val = installed_parts.get(i).copied().unwrap_or(0);
        let required_val = required_parts.get(i).copied().unwrap_or(0);
        if installed_val > required_val {
            return true;
        }
        if installed_val < required_val {
            return false;
        }
    }

    let installed_has_prerelease = installed.contains('-');
    let required_has_prerelease = required.contains('-');

    if installed_has_prerelease && !required_has_prerelease {
        let installed_base_eq = installed_parts == required_parts;
        return !installed_base_eq;
    }

    true
}

fn strip_prerelease(version: &str) -> &str {
    version.split('-').next().unwrap_or(version)
}

/// 校验 manifest 声明的完整性（`PluginIntegrity`）。
///
/// 由 `build_plugin_manifest` 在**每次** manifest 载入时调用 ⇒ 安装后遭篡改会在下一次
/// 加载即失败，而不是只在 install 当场校验一次。
pub(crate) fn verify_declared_integrity(
    plugin_root: &Path,
    integrity: &PluginIntegrity,
) -> Result<(), PluginError> {
    match integrity.algorithm.as_str() {
        "sha256" => {
            let hash = hash_plugin_directory(plugin_root)?;
            if hash.eq_ignore_ascii_case(&integrity.hash) {
                Ok(())
            } else {
                Err(PluginError::ManifestValidation(vec![
                    PluginManifestValidationError::IntegrityCheckFailed {
                        algorithm: integrity.algorithm.clone(),
                        expected: integrity.hash.clone(),
                        actual: hash,
                    },
                ]))
            }
        },
        other => {
            Err(PluginError::CommandFailed(format!("unsupported integrity algorithm: {other}")))
        },
    }
}

/// 插件面板前端资产的允许扩展名 —— 沙箱 iframe 只需这四种文本资产。
pub(crate) const PANEL_ASSET_ALLOWED_EXTS: &[&str] = &["html", "htm", "js", "css"];

/// 面板资产大小上限。`skill_read_asset` 无上限，此处**刻意收紧**：
/// 读的是第三方代码，巨型文件既无正当用途又是内存放大面。
pub(crate) const PANEL_ASSET_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// 资产声明的**形状**校验（不做 I/O）：绝对路径 / 逃逸 / 扩展名。
///
/// `field` 是清单里的键名（面板是 `frontendEntry`、页面是 `entry`），**只用于错误文案**：
/// 判定口径由面板与页面两条通道共用同一份，不许各写一套 —— 否则同一份产物会出现
/// 「面板能装、页面不能装」这类无法解释的分叉。
///
/// 真正的越界防线在读取期的 canonicalize + 包含性校验（[`canonical_asset_in_dir`]），
/// 这里是加载期的第一道。
fn asset_entry_shape_error(entry: &str, field: &str) -> Option<String> {
    if entry.trim().is_empty() {
        return Some(format!("{field} cannot be empty"));
    }
    if entry.starts_with('/') || entry.starts_with('\\') {
        return Some(format!("{field} must be relative to the plugin directory: {entry}"));
    }
    let bytes = entry.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some(format!("{field} must not be an absolute path: {entry}"));
    }
    if entry.split(['/', '\\']).any(|seg| seg == "..") {
        return Some(format!("{field} must not escape the plugin directory: {entry}"));
    }
    match Path::new(entry).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) {
        Some(ext) if PANEL_ASSET_ALLOWED_EXTS.contains(&ext.as_str()) => None,
        Some(ext) => Some(format!(
            "{field} extension `.{ext}` is not allowed (expected one of {})",
            PANEL_ASSET_ALLOWED_EXTS.join(", ")
        )),
        None => Some(format!("{field} must have a file extension: {entry}")),
    }
}

/// 读取期共用的完整性防线：安装期锚点存在 + 目录内容与锚点一致。
///
/// 面板与页面两条通道**必须走同一条**：锚点缺失（[`PluginError::IntegrityUnpinned`]）与
/// 内容被改（[`PluginError::IntegrityMismatch`]）的用户动作不同（重装 vs 移除），
/// 塌成一个提示就等于把安全事件说成配置缺失。`purpose` 只进文案。
fn verify_install_anchor(
    record: &InstalledPluginRecord,
    plugin_id: &str,
    purpose: &str,
) -> Result<(), PluginError> {
    let pinned = record.integrity.as_ref().ok_or_else(|| {
        PluginError::IntegrityUnpinned(format!(
            "plugin `{plugin_id}` has no pinned install hash; reinstall it to enable {purpose}"
        ))
    })?;
    verify_declared_integrity(&record.install_path, pinned).map_err(|error| {
        PluginError::IntegrityMismatch(format!(
            "plugin `{plugin_id}` install directory no longer matches its pinned hash: {error}"
        ))
    })
}

/// 把「已按形状校验过的相对资产路径」解析成**安装目录内**的规范路径。
///
/// `canonicalize` 之后再判包含性，因此符号链接指向目录外同样被拒；`kind` 只进错误文案
/// （`panel` / `page`）。本函数**不读文件**，故不判大小上限（那是内容通道
/// [`PluginManager::read_panel_asset`] 的防线），也不判扩展名（那是
/// [`asset_entry_shape_error`] 的职责）。
fn canonical_asset_in_dir(
    install_dir: &Path,
    entry: &str,
    kind: &str,
) -> Result<PathBuf, PluginError> {
    let install_dir = install_dir.canonicalize()?;
    let requested = install_dir.join(entry);
    let canonical = requested.canonicalize().map_err(|_| {
        PluginError::NotFound(format!("{kind} asset not found: {}", requested.display()))
    })?;
    if !canonical.starts_with(&install_dir) {
        return Err(PluginError::PermissionDenied(format!(
            "Access denied: {kind} asset is outside plugin directory ({entry})"
        )));
    }
    Ok(canonical)
}

/// 把 canonicalize 结果转成可交给 asset 协议的**普通**绝对路径。
///
/// Windows 的 `std::fs::canonicalize` 会给路径加 `\\?\`（网络路径是 `\\?\UNC\server\share`）
/// verbatim 前缀，而 tauri 的 `assetProtocol.scope` 按**普通路径**做 glob 匹配 ⇒
/// 前缀不去掉就会变成「后端给了路径、前端 `convertFileSrc` 之后被作用域拒绝」这种
/// 只在运行期出现且极难定位的故障。非 Windows 原样返回。
fn path_for_asset_protocol(path: &Path) -> String {
    let raw = path.display().to_string();
    #[cfg(windows)]
    {
        if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = raw.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    raw
}

fn validate_dashboard_panels(
    panels: &[PluginDashboardPanel],
    errors: &mut Vec<PluginManifestValidationError>,
) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for panel in panels {
        if panel.id.trim().is_empty() {
            errors.push(PluginManifestValidationError::EmptyEntryField {
                kind: "dashboard panel",
                field: "id",
                name: None,
            });
            continue;
        }
        if !seen.insert(panel.id.as_str()) {
            errors.push(PluginManifestValidationError::DuplicateEntry {
                kind: "dashboard panel",
                name: panel.id.clone(),
            });
        }
        if let Some(entry) = &panel.frontend_entry
            && let Some(detail) = asset_entry_shape_error(entry, "frontendEntry")
        {
            errors.push(PluginManifestValidationError::InvalidPanelAsset {
                panel: panel.id.clone(),
                detail,
            });
        }
    }
}

fn hash_plugin_directory(plugin_root: &Path) -> Result<String, PluginError> {
    let mut hasher = sha2::Sha256::new();
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files_recursive(plugin_root, &mut files)?;
    files.sort();

    for file_path in &files {
        let data = fs::read(file_path).map_err(PluginError::Io)?;
        use sha2::Digest;
        hasher.update(
            file_path.strip_prefix(plugin_root).unwrap_or(file_path).to_string_lossy().as_bytes(),
        );
        hasher.update(&data);
    }

    use sha2::Digest;
    let result = hasher.finalize();
    let mut hash_str = String::with_capacity(result.len() * 2);
    for byte in result {
        use std::fmt::Write;
        write!(hash_str, "{byte:02x}").expect("writing to String should never fail");
    }
    Ok(hash_str)
}

fn collect_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), PluginError> {
    let entries = fs::read_dir(dir).map_err(PluginError::Io)?;
    for entry in entries {
        let entry = entry.map_err(PluginError::Io)?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

/// Windows 上杀毒软件/索引服务/文件句柄占用导致 `remove_dir_all` 偶发失败。
/// 带退避重试的安全删除工具，默认 5 次，间隔递增（100/200/400/800/1600 ms）。
pub(super) fn remove_dir_all_with_retry(path: &Path, max_retries: u32) -> Result<(), PluginError> {
    for attempt in 0..max_retries {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) if attempt < max_retries - 1 => {
                tracing::debug!(
                    "remove_dir_all attempt {} failed for {}: {}, retrying...",
                    attempt + 1,
                    path.display(),
                    e
                );
                std::thread::sleep(Duration::from_millis(100 * (attempt as u64 + 1)));
            },
            Err(e) => return Err(PluginError::Io(e)),
        }
    }
    Ok(())
}

/// 测试辅助（#189）：**唯一**的 `PluginManagerConfig` 构造入口。
///
/// 为什么需要：`PluginManagerConfig::new` 的缺省 `bundled_root` 是
/// `env!("CARGO_MANIFEST_DIR")/bundled`，而 `npm run plugin:bundle:dev` 会把 stock/opc 的
/// dev 产物**暂存到同一目录**（宿主 dev 模式也从这里装载）⇒ 任何「插件数量」类断言都会随盘上
/// 产物涨落。测试必须把 `bundled_root` 钉到一个空目录，否则本地绿/红取决于有没有跑过打包脚本。
///
/// 放在 `manager` 下而不是各测试模块内：`lib.rs` / `hooks.rs` / `manager.rs` 三处测试共用**同一份**
/// 实现（禁区 12：禁止重复定义）。
#[cfg(test)]
pub(crate) mod test_support {
    use super::PluginManagerConfig;
    use std::path::Path;

    pub(crate) fn isolated_config(config_home: impl AsRef<Path>) -> PluginManagerConfig {
        let config_home = config_home.as_ref().to_path_buf();
        let mut config = PluginManagerConfig::new(config_home.clone());
        let empty_bundled = config_home.join("bundled-isolated");
        std::fs::create_dir_all(&empty_bundled).expect("建空的 bundled 隔离目录");
        config.bundled_root = Some(empty_bundled);
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::test_support::isolated_config;
    use axagent_harness::{
        CapabilityDomain, CapabilityEvolvability, CapabilityKind, CapabilityRegistry,
        CapabilitySource,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("plugins-{label}-{nanos}"))
    }

    /// P6.2 批 1：owner 解析正负控。用已安装 External 插件夹具，命令名相同、插件名不同。
    #[test]
    fn resolve_command_owner_enforces_unique_enabled_owner() {
        let temp = temp_dir("owner-resolve");
        let source = temp.join("source");
        write_command_plugin_source(
            &source,
            "opc-accounting",
            r#"["subprocess_execution"]"#,
            r#"{ "name": "opc_list_invoices", "description": "accounting", "command": "cmd.sh" }"#,
        );
        let mut manager = PluginManager::new(isolated_config(temp.join("home")));
        let installed = manager.install(source.to_str().expect("utf8")).expect("install 成功");
        manager.write_enabled_state(&installed.plugin_id, Some(true)).expect("启用");
        manager.config.enabled_plugins.insert(installed.plugin_id.clone(), true);

        let resolved = manager
            .resolve_command_owner("opc", "opc_list_invoices")
            .expect("唯一 owner 应解析为完整注册表 id");
        assert_eq!(resolved, installed.plugin_id);

        // 零 owner：命令名不存在。
        let err = manager.resolve_command_owner("opc", "opc_not_declared").unwrap_err();
        assert!(matches!(err, PluginError::NotFound(_)), "期望 NotFound，实际 {err:?}");

        // disabled owner 不算命中：关掉后同一命令应变成零 owner。
        manager.write_enabled_state(&installed.plugin_id, Some(false)).expect("禁用");
        manager.config.enabled_plugins.insert(installed.plugin_id.clone(), false);
        let err = manager.resolve_command_owner("opc", "opc_list_invoices").unwrap_err();
        assert!(matches!(err, PluginError::NotFound(_)), "禁用后应零 owner，实际 {err:?}");

        // 多 owner：再装一个同样声明该命令的插件并启用，必须报冲突而不是取第一个。
        let conflict_source = temp.join("conflict");
        write_command_plugin_source(
            &conflict_source,
            "opc-legacy",
            r#"["subprocess_execution"]"#,
            r#"{ "name": "opc_list_invoices", "description": "legacy", "command": "cmd.sh" }"#,
        );
        let conflict =
            manager.install(conflict_source.to_str().expect("utf8")).expect("install 成功");
        manager.write_enabled_state(&conflict.plugin_id, Some(true)).expect("启用");
        manager.config.enabled_plugins.insert(conflict.plugin_id.clone(), true);
        manager.write_enabled_state(&installed.plugin_id, Some(true)).expect("恢复启用");
        manager.config.enabled_plugins.insert(installed.plugin_id.clone(), true);
        let err = manager.resolve_command_owner("opc", "opc_list_invoices").unwrap_err();
        assert!(matches!(err, PluginError::CommandFailed(_)), "多 owner 应冲突，实际 {err:?}");

        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 测试专用配置：把 bundled 根**钉到一个空目录**。
    ///
    /// 为什么必须钉：生产缺省的 bundled 根就是 `env!("CARGO_MANIFEST_DIR")/bundled`
    /// （见 [`PluginManager::bundled_root`]，`PluginManagerConfig::resolve_bundled_root(None)` 与它同源），
    /// 而本机 `npm run plugin:bundle:dev` 会把 stock/opc 的 dev 产物暂存进同一个目录
    /// （宿主 dev 模式正从这里装载）⇒ 不钉住，任何「投影条数 / 名单集合」断言都随盘上暂存产物涨落。
    /// 实测：暂存在位时 `agent_command_specs` 从 1 条读成 7 条、任务源投影从 1 变 2，三条用例红，
    /// 而产品代码零改动 —— 红的是测试对环境的耦合，不是插件运行器的缺陷。
    ///
    /// 空目录落在 `config_home` 之下，随各用例既有的 cleanup 一并删除，不额外留残渣。
    fn demo_manifest_with_capabilities() -> PluginManifest {
        PluginManifest {
            name: "demo".into(),
            version: "1.0.0".into(),
            description: "demo".into(),
            author: None,
            permissions: Vec::new(),
            default_enabled: true,
            hooks: PluginHooks::default(),
            lifecycle: PluginLifecycle::default(),
            tools: Vec::new(),
            commands: Vec::new(),
            scenarios: Vec::new(),
            mcp_servers: Vec::new(),
            skills: Vec::new(),
            agents: Vec::new(),
            dashboard_panels: Vec::new(),
            pages: Vec::new(),
            task_sources: Vec::new(),
            task_cards: Vec::new(),
            dependencies: Vec::new(),
            integrity: None,
            capabilities: vec![PluginCapabilityDecl {
                seam: "platform.adapter.telegram".into(),
                capability_type: "platform_adapter".into(),
                op: "start".into(),
                version: "1.0".into(),
                description: "demo telegram adapter".into(),
                name: "Telegram 适配器".into(),
                kind: "tool".into(),
                domain: "communication".into(),
                tags: vec!["telegram".into()],
                negative_scenarios: Vec::new(),
                visibility: "public".into(),
                discoverable: true,
                evolvable: "derived".into(),
            }],
            worker: None,
            domain_packs: Vec::new(),
        }
    }

    /// dashboard 合流（PLAN-plugin-gap-closure §3）：`dashboard_panels` 由已安装插件
    /// 的 manifest 实时投影进 `list_dashboard_plugins` —— 不再有第二套 registry/目录。
    #[test]
    fn list_dashboard_plugins_projects_installed_manifest_panels() {
        let temp = temp_dir("dashproj");
        let installed = temp.join("plugins").join("installed").join("chart-pack");
        std::fs::create_dir_all(&installed).expect("创建插件目录");
        std::fs::write(
            installed.join(MANIFEST_FILE_NAME),
            r#"{
                "name": "chart-pack",
                "version": "1.0.0",
                "description": "charts",
                "author": "ax",
                "dashboard_panels": [
                    {
                        "id": "heatmap",
                        "title": "Heatmap",
                        "componentName": "chart-pack.heatmap",
                        "position": "sidebar",
                        "size": "fullWidth",
                        "props": {"limit": 10},
                        "frontendEntry": "main.js"
                    }
                ]
            }"#,
        )
        .expect("写入 manifest");
        let manager = PluginManager::new(isolated_config(temp));
        let list = manager.list_dashboard_plugins().expect("清单投影应成功");
        let entry = list
            .iter()
            .find(|plugin| plugin.name == "chart-pack")
            .expect("声明了面板的已安装插件应进入清单");
        assert_eq!(entry.panels.len(), 1);
        assert_eq!(entry.panels[0].component_name, "chart-pack.heatmap");
        assert_eq!(entry.panels[0].position, crate::types::PluginDashboardPanelPosition::Sidebar);
        assert_eq!(entry.panels[0].size, crate::types::PluginDashboardPanelSize::FullWidth);
        assert_eq!(entry.panels[0].props["limit"], serde_json::json!(10));
        assert_eq!(entry.panels[0].frontend_entry.as_deref(), Some("main.js"));
        assert_eq!(entry.author.as_deref(), Some("ax"));
        // 无面板声明的内置/bundled 插件不得混入投影。
        assert!(list.iter().all(|plugin| !plugin.panels.is_empty()));
    }

    /// 写一份只含两个面板（一有资产、一无资产）的已安装插件 + registry，返回 manager。
    fn manager_with_panel_pack(temp: &Path, panel_json: &str) -> PluginManager {
        let installed = temp.join("plugins").join("installed").join("ui-pack");
        std::fs::create_dir_all(installed.join("ui")).expect("创建插件目录");
        std::fs::write(
            installed.join(MANIFEST_FILE_NAME),
            format!(
                r#"{{
                    "name": "ui-pack",
                    "version": "1.0.0",
                    "description": "ui",
                    "dashboard_panels": {panel_json}
                }}"#
            ),
        )
        .expect("写入 manifest");
        std::fs::write(installed.join("ui").join("panel.html"), "<html>hi</html>")
            .expect("写入面板资产");

        let manager = PluginManager::new(isolated_config(temp));
        let mut registry = InstalledPluginRegistry::default();
        // 与真实 install 同形：钉下目录哈希。不钉则 read_panel_asset 按 fail-closed 拒绝，
        // 夹具就会把「未建立锚点」误验成「资产读取本身可用」。
        let pinned = hash_plugin_directory(&installed)
            .ok()
            .map(|hash| PluginIntegrity { algorithm: "sha256".to_string(), hash });
        registry.plugins.insert(
            "ui-pack".to_string(),
            InstalledPluginRecord {
                kind: PluginKind::External,
                id: "ui-pack".to_string(),
                name: "ui-pack".to_string(),
                version: "1.0.0".to_string(),
                description: "ui".to_string(),
                install_path: installed.clone(),
                source: PluginInstallSource::LocalPath { path: installed },
                installed_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                integrity: pinned,
            },
        );
        manager.store_registry(&registry).expect("写 registry");
        manager
    }

    /// 面板 `frontendEntry` 的越界/绝对/非法扩展名形态必须在 **manifest 载入期**即被拒：
    /// manifest 是长期驻留的声明面，等到前端取资产才拦就等于允许坏声明进仓。
    #[test]
    fn load_rejects_invalid_panel_frontend_entry() {
        for (label, entry) in [
            ("escape", "../evil.html"),
            ("absolute", "/etc/passwd.html"),
            ("drive", "C:/Windows/x.html"),
            ("ext", "panel.exe"),
        ] {
            let temp = temp_dir(&format!("panel-{label}"));
            let manager = manager_with_panel_pack(
                &temp,
                r#"[{ "id": "p1", "title": "P1", "componentName": "ui-pack.p1", "frontendEntry": "PLACEHOLDER" }]"#,
            );
            // 用 manager_with_panel_pack 建好目录后，改写 manifest 里的 entry 为本轮形态
            let manifest_path =
                temp.join("plugins").join("installed").join("ui-pack").join(MANIFEST_FILE_NAME);
            let body = std::fs::read_to_string(&manifest_path).expect("读 manifest");
            std::fs::write(&manifest_path, body.replace("PLACEHOLDER", entry))
                .expect("改写 manifest");

            let err = load_plugin_from_directory(manifest_path.parent().expect("插件目录"))
                .expect_err(&format!("{label} 形态的 frontendEntry 应被拒绝"));
            let text = err.to_string();
            assert!(text.contains("frontendEntry"), "{label}: 实际错误 = {text}");
            // 面板资产读取同样不得放行（形状校验是同一函数，两处共用）
            assert!(manager.read_panel_asset("ui-pack", "p1").is_err());
            let _ = std::fs::remove_dir_all(&temp);
        }
    }

    /// 空 id / 重复 id 在载入期即拒 —— 前端渲染与资产寻址都以 panel.id 为键。
    #[test]
    fn load_rejects_duplicate_or_empty_panel_id() {
        for (label, panel_json, expect) in [
            (
                "dup",
                r#"[
                    { "id": "p1", "title": "A", "componentName": "c.a" },
                    { "id": "p1", "title": "B", "componentName": "c.b" }
                ]"#,
                "is duplicated",
            ),
            (
                "empty",
                r#"[{ "id": "  ", "title": "A", "componentName": "c.a" }]"#,
                "cannot be empty",
            ),
        ] {
            let temp = temp_dir(&format!("panel-{label}"));
            manager_with_panel_pack(&temp, panel_json);
            let dir = temp.join("plugins").join("installed").join("ui-pack");
            let err = load_plugin_from_directory(&dir).expect_err(&format!("{label} 应被拒绝"));
            let text = err.to_string();
            assert!(
                text.contains("dashboard panel") && text.contains(expect),
                "{label}: 实际 = {text}"
            );
            let _ = std::fs::remove_dir_all(&temp);
        }
    }

    /// 合法声明能读到资产文本；未声明 `frontendEntry` 的面板是**元数据面板**，
    /// 必须显式报「无内容来源」而不是返回空串（空串会被前端渲染成空白面板冒充正常结果）。
    #[test]
    fn read_panel_asset_serves_declared_source_and_flags_metadata_only() {
        let temp = temp_dir("panel-read");
        let manager = manager_with_panel_pack(
            &temp,
            r#"[
                { "id": "chart", "title": "C", "componentName": "ui-pack.chart", "frontendEntry": "ui/panel.html" },
                { "id": "meta", "title": "M", "componentName": "ui-pack.meta" }
            ]"#,
        );

        let asset = manager.read_panel_asset("ui-pack", "chart").expect("应读到面板资产");
        assert_eq!(asset.source, "<html>hi</html>");
        assert_eq!(asset.entry, "ui/panel.html");
        assert_eq!(asset.plugin_id, "ui-pack");
        assert_eq!(asset.panel_id, "chart");

        let meta_err = manager.read_panel_asset("ui-pack", "meta").expect_err("元数据面板没有资产");
        assert!(meta_err.to_string().contains("no frontendEntry"), "实际 = {meta_err}");
        assert!(manager.read_panel_asset("ui-pack", "absent").is_err());
        assert!(manager.read_panel_asset("no-such-plugin", "chart").is_err());
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 钉死锚点后篡改目录内容必须被拒，且报的是「被改动」而非「没声明内容」——
    /// 两者的用户动作不同（重装 vs 移除），塌成一个提示就等于把安全事件说成配置缺失。
    #[test]
    fn read_panel_asset_refuses_tampered_install() {
        let temp = temp_dir("panel-tamper");
        let manager = manager_with_panel_pack(
            &temp,
            r#"[{ "id": "chart", "title": "C", "componentName": "ui-pack.chart", "frontendEntry": "ui/panel.html" }]"#,
        );
        let asset_dir = temp.join("plugins").join("installed").join("ui-pack");
        std::fs::write(asset_dir.join("ui").join("panel.html"), "<html>tampered</html>")
            .expect("改写资产");

        let err = manager.read_panel_asset("ui-pack", "chart").expect_err("篡改后应拒绝");
        assert!(
            matches!(err, PluginError::IntegrityMismatch(_)),
            "应报 IntegrityMismatch，实际 = {err:?}"
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 早于本机制安装（或安装当场哈希失败）的插件没有锚点 ⇒ fail-closed 拒绝，
    /// 但必须与「被篡改」区分开：内容其实没变，只是无从证明。
    #[test]
    fn read_panel_asset_refuses_unpinned_install() {
        let temp = temp_dir("panel-unpinned");
        let manager = manager_with_panel_pack(
            &temp,
            r#"[{ "id": "chart", "title": "C", "componentName": "ui-pack.chart", "frontendEntry": "ui/panel.html" }]"#,
        );
        let mut registry = manager.load_registry().expect("读 registry");
        registry.plugins.get_mut("ui-pack").expect("夹具应有该插件").integrity = None;
        manager.store_registry(&registry).expect("改写 registry");

        let err = manager.read_panel_asset("ui-pack", "chart").expect_err("无锚点应拒绝");
        assert!(
            matches!(err, PluginError::IntegrityUnpinned(_)),
            "应报 IntegrityUnpinned，实际 = {err:?}"
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 2026-09-30 实测缺陷回归：Claude Code 清单的 `author` 是**对象**写法
    /// （`{"name": …, "email": …}`）时，原先 `RawPluginManifest.author: Option<String>`
    /// 让**整份清单**反序列化失败（`invalid type: map, expected a string`），
    /// 插件从此再也加载不出来。复现源为
    /// `~/.agents/skills/connect-apps-plugin/.claude-plugin/plugin.json`。
    #[test]
    fn load_plugin_from_directory_accepts_object_author_in_claude_code_manifest() {
        let root = temp_dir("authorobj");
        let manifest_path = root.join(MANIFEST_RELATIVE_PATH);
        std::fs::create_dir_all(manifest_path.parent().expect("清单路径应有父目录"))
            .expect("创建 .claude-plugin 目录");
        std::fs::write(
            &manifest_path,
            r#"{
                "name": "connect-apps",
                "version": "1.0.0",
                "description": "Manage auth and connect to apps.",
                "author": {"name": "Composio", "email": "support@composio.dev"}
            }"#,
        )
        .expect("写入 manifest");

        let manifest = load_plugin_from_directory(&root).expect("对象写法 author 不应导致加载失败");
        assert_eq!(manifest.author.as_deref(), Some("Composio <support@composio.dev>"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plugin_capabilities_register_and_rollback() {
        let registry = CapabilityRegistry::new();
        let config = isolated_config(temp_dir("p3cap"));
        let mut manager = PluginManager::new(config).with_capability_registry(&registry);
        let manifest = demo_manifest_with_capabilities();

        let errors = manager.register_plugin_capabilities("external:demo", &manifest);
        assert!(errors.is_empty(), "注册不应失败: {errors:?}");
        // 声明不构成实现：不占实现表（避免「看起来有提供者、运行时取不到」）
        assert!(!registry.contains("platform.adapter.telegram"));
        assert!(registry.is_empty());
        // 但可在检视视图中看到，并标注「仅声明、未实现」
        let details = registry.list_with_details();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].definition.id, "platform.adapter.telegram");
        assert_eq!(details[0].plugin_id.as_deref(), Some("external:demo"));
        assert!(!details[0].implemented);

        // 回滚后声明被移除
        manager.unregister_plugin_capabilities("external:demo");
        assert!(registry.list_with_details().is_empty());
    }

    #[test]
    fn plugin_capabilities_without_registry_is_noop() {
        let config = isolated_config(temp_dir("p3noop"));
        let mut manager = PluginManager::new(config);
        let manifest = demo_manifest_with_capabilities();

        let errors = manager.register_plugin_capabilities("external:noop", &manifest);
        assert!(errors.is_empty());
        // 未注入 registry 时不报错、不崩溃
        manager.unregister_plugin_capabilities("external:noop");
    }

    /// 构造含技能 + Agent + 声明能力（含不可发现项）的 manifest，验证护照映射。
    fn manifest_with_passport_sources() -> PluginManifest {
        let mut manifest = demo_manifest_with_capabilities();
        manifest.skills =
            vec![PluginSkillEntry { name: "web-auto".into(), path: "skills/web-auto".into() }];
        manifest.agents = vec![PluginAgentDefInternal {
            agent_type: "helper".into(),
            description: "demo helper agent".into(),
            tools: Vec::new(),
            disallowed_tools: Vec::new(),
            model: None,
            background: false,
            system_prompt: None,
        }];
        // 追加一个 discoverable=false 的声明能力（应被护照映射跳过）
        manifest.capabilities.push(PluginCapabilityDecl {
            seam: "hidden.seam".into(),
            capability_type: "internal".into(),
            op: String::new(),
            version: "1.0".into(),
            description: String::new(),
            name: String::new(),
            kind: "tool".into(),
            domain: "general".into(),
            tags: Vec::new(),
            negative_scenarios: Vec::new(),
            visibility: "public".into(),
            discoverable: false,
            evolvable: "none".into(),
        });
        manifest
    }

    #[test]
    fn plugin_passports_mapping_marks_source_and_evolvability() {
        let config = isolated_config(temp_dir("p3passmap"));
        let manager = PluginManager::new(config);
        let manifest = manifest_with_passport_sources();

        let passports = manager.collect_plugin_passports(
            "external:demo",
            &manifest,
            PluginKind::External,
            None,
        );
        // 1 技能 + 1 Agent + 1 可发现声明能力（hidden.seam 被跳过）
        assert_eq!(passports.len(), 3, "护照应跳过 discoverable=false 的声明能力");

        // 技能：插件来源 + 本地可写（Local 进化）
        let skill = passports
            .iter()
            .find(|p| p.capability_id == "plugin:external:demo:skill:web-auto")
            .expect("技能护照应存在");
        assert_eq!(skill.source, CapabilitySource::Plugin);
        assert_eq!(skill.evolvable, CapabilityEvolvability::Local);
        assert_eq!(skill.kind, CapabilityKind::Skill);
        assert!(skill.tags.contains(&"plugin".to_string()));

        // Agent：插件来源 + 本地可写（Local 进化）
        let agent = passports
            .iter()
            .find(|p| p.capability_id == "plugin:external:demo:agent:helper")
            .expect("Agent 护照应存在");
        assert_eq!(agent.source, CapabilitySource::Plugin);
        assert_eq!(agent.evolvable, CapabilityEvolvability::Local);
        assert_eq!(agent.kind, CapabilityKind::Agent);

        // 声明能力：插件来源 + 按 decl 解析（derived → 进化产出副本、原护照不变）
        let cap = passports
            .iter()
            .find(|p| p.capability_id == "plugin:external:demo:cap:platform.adapter.telegram")
            .expect("声明能力护照应存在");
        assert_eq!(cap.source, CapabilitySource::Plugin);
        assert_eq!(cap.evolvable, CapabilityEvolvability::Derived);
        assert_eq!(cap.kind, CapabilityKind::Tool);
        assert_eq!(cap.domain, CapabilityDomain::Communication);

        // 不可发现的能力不应产出护照
        assert!(
            !passports.iter().any(|p| p.capability_id == "plugin:external:demo:cap:hidden.seam"),
            "discoverable=false 不应产出护照"
        );
    }

    #[test]
    fn plugin_passports_register_tracks_ids_and_rollback() {
        let config = isolated_config(temp_dir("p3passlife"));
        let mut manager = PluginManager::new(config);
        let manifest = manifest_with_passport_sources();

        // 注册护照：记录 ID 供回滚
        let count =
            manager.register_plugin_passports("external:demo", &manifest, PluginKind::External);
        assert_eq!(count, 3);
        let tracked = manager.active_passport_ids.get("external:demo").cloned();
        let tracked = tracked.expect("应记录护照 ID 集合");
        assert_eq!(tracked.len(), 3);
        assert!(tracked.contains(&"plugin:external:demo:skill:web-auto".to_string()));

        // 回滚：取出 ID 并清空状态
        let ids = manager.take_plugin_passport_ids("external:demo");
        assert_eq!(ids.len(), 3);
        assert!(!manager.active_passport_ids.contains_key("external:demo"));
        // 重复回滚为空（幂等）
        assert!(manager.take_plugin_passport_ids("external:demo").is_empty());
    }

    // ==== 命名命令通道（inputSchema / requiredPermission 声明面 + 执行门禁）====

    /// 构造清单条目。`command` 用 `echo` 这类**字面命令**（`is_literal_command` 为真），
    /// 好让 `build_manifest_commands` 里的路径存在性校验不干扰本组断言。
    fn command_manifest_entry(
        name: &str,
        input_schema: Option<Value>,
        required_permission: Option<&str>,
    ) -> PluginCommandManifest {
        PluginCommandManifest {
            name: name.to_string(),
            description: "test command".to_string(),
            command: "echo".to_string(),
            input_schema,
            required_permission: required_permission.map(str::to_string),
        }
    }

    fn permissions_of(labels: &[&str]) -> Vec<PluginPermission> {
        labels.iter().filter_map(|label| PluginPermission::parse(label)).collect()
    }

    /// 命令脚本的文件名（随平台变）。
    fn command_script_name() -> &'static str {
        if cfg!(windows) {
            "greet.cmd"
        } else {
            "greet.sh"
        }
    }

    /// 拼一条 `commands[]` 声明 JSON。
    ///
    /// `relative = false` ⇒ 脚本按**源目录里的绝对路径**声明。这是任务 #47 修复前唯一跑得通的
    /// 形态，保留作**回归**用例（绝对路径声明必须继续原样交给 `Command::new`）。
    /// `relative = true` ⇒ 声明成 `./commands/<脚本>`，即相对插件安装目录的形态。修复前这种声明
    /// 必然 spawn 失败：命令串整团交给 `Command::new`，可执行文件查找只看**宿主进程的 cwd**
    /// （子进程 `current_dir` 不参与）⇒ Windows `OS error 3`、POSIX ENOENT。
    /// ⚠ 相对形态必须带 `./` 前缀 —— 不带前缀的串按字面命令交给 PATH 查找（tool 侧既有语义）。
    fn command_decl_json(source: &Path, extra_fields_json: &str, relative: bool) -> String {
        let script = source.join("commands").join(command_script_name());
        let declared = if relative {
            format!("./commands/{}", command_script_name())
        } else {
            script.display().to_string()
        };
        let command_json = serde_json::to_string(&declared).expect("脚本路径应可 JSON 化");
        if extra_fields_json.is_empty() {
            format!(r#"{{ "name": "greet", "description": "Greet", "command": {command_json} }}"#)
        } else {
            format!(
                r#"{{ "name": "greet", "description": "Greet", "command": {command_json}, {extra_fields_json} }}"#
            )
        }
    }

    /// 写一个只声明命名命令的插件源目录。
    ///
    /// 脚本会在工作目录（= 插件安装目录）落一个 `ran.marker`，用来**客观证明**
    /// 「拒绝发生在 spawn 之前」而不是只靠错误类型推断。
    fn write_command_plugin_source(
        root: &Path,
        name: &str,
        permissions_json: &str,
        command_decl_json: &str,
    ) {
        let script_content = if cfg!(windows) {
            "@echo off\r\necho ran> ran.marker\r\necho command=%CLAWD_TOOL_NAME%\r\n"
        } else {
            "#!/bin/sh\ncat >/dev/null\nprintf 'ran' > ran.marker\nprintf 'command=%s\\n' \"$CLAWD_TOOL_NAME\"\n"
        };

        let script_path = root.join("commands").join(command_script_name());
        std::fs::create_dir_all(root.join("commands")).expect("创建 commands 目录");
        std::fs::write(&script_path, script_content).expect("写入命令脚本");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).expect("metadata").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).expect("chmod");
        }
        let manifest_path = root.join(MANIFEST_RELATIVE_PATH);
        std::fs::create_dir_all(manifest_path.parent().expect("manifest 声明的目录路径应有父级"))
            .expect("创建 manifest 目录");
        std::fs::write(
            &manifest_path,
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \
                 \"description\": \"command gate plugin\",\n  \"defaultEnabled\": true,\n  \
                 \"permissions\": {permissions_json},\n  \
                 \"commands\": [ {command_decl_json} ]\n}}"
            ),
        )
        .expect("写入 manifest");
    }

    /// 命令通道端到端夹具：已安装的插件 + 需要清理的临时目录。
    struct CommandPluginFixture {
        manager: PluginManager,
        config_home: PathBuf,
        source: PathBuf,
        install_path: PathBuf,
        plugin_id: String,
    }

    impl CommandPluginFixture {
        /// 装一个只声明命名命令的插件。`extra_fields_json` 是 `commands[]` 条目的附加声明
        /// （`requiredPermission` / `inputSchema`），传空串即「未声明」的老清单形状。
        /// `relative` 决定 `command` 声明成相对安装目录还是绝对路径（见 [`command_decl_json`]）。
        fn new(
            label: &str,
            permissions_json: &str,
            extra_fields_json: &str,
            relative: bool,
        ) -> Self {
            let source = temp_dir(&format!("{label}-source"));
            let config_home = temp_dir(&format!("{label}-home"));
            let decl = command_decl_json(&source, extra_fields_json, relative);
            write_command_plugin_source(&source, &format!("{label}-demo"), permissions_json, &decl);
            let mut manager = PluginManager::new(isolated_config(&config_home));
            let outcome = manager
                .install(source.to_str().expect("utf8 path"))
                .unwrap_or_else(|e| panic!("install 应成功: {e}"));
            Self {
                manager,
                config_home,
                source,
                install_path: outcome.install_path.clone(),
                plugin_id: outcome.plugin_id,
            }
        }

        /// 脚本自报的「真的跑过」标记落在插件安装目录（执行链把 cwd 设成安装根）。
        fn marker(&self) -> PathBuf {
            self.install_path.join("ran.marker")
        }

        fn cleanup(self) {
            let _ = std::fs::remove_dir_all(&self.config_home);
            let _ = std::fs::remove_dir_all(&self.source);
        }
    }

    /// 契约 E-1：老清单（两个新键都不存在）照常加载，且落定值与加字段前逐字节一致。
    #[test]
    fn undeclared_command_declarations_keep_todays_defaults() {
        let raw = vec![command_manifest_entry("greet", None, None)];
        let mut errors = Vec::new();

        let validated = build_manifest_commands(Path::new("."), raw, &mut errors);

        assert!(errors.is_empty(), "未声明新字段不得报错，实际 {errors:?}");
        assert_eq!(validated.len(), 1);
        // schema 落定 = 空 object（今天写死的那个值）；档位落定 = ReadOnly。
        assert_eq!(resolved_command_input_schema(&validated[0]), Value::Object(Map::new()));
        assert_eq!(
            command_required_permission(&validated[0]).expect("默认档位应可解析"),
            PluginToolPermission::ReadOnly
        );
        // 未声明 ⇒ 执行链**跳过**形状校验；一旦声明，非 object 输入即拒（对照断言）。
        let declared_schema = serde_json::json!({ "type": "object" });
        assert!(matches!(
            PluginManager::check_command_input_shape("greet", &declared_schema, &Value::Null),
            Err(PluginError::CommandFailed(_))
        ));

        // 同形状走完整加载链（真实 manifest JSON 里没有新键）也必须能装、能分发。
        let fixture = CommandPluginFixture::new("cmd-default", r#"["read"]"#, "", false);
        let summaries = fixture.manager.list_plugins().expect("清单应可枚举");
        let summary = summaries
            .iter()
            .find(|s| s.metadata.id == fixture.plugin_id)
            .expect("已安装插件应出现在清单");
        assert_eq!(summary.commands.len(), 1);
        assert_eq!(summary.commands[0].input_schema, None);
        assert_eq!(summary.commands[0].required_permission, None);
        // 未声明 schema 的 `null` 输入必须一路走到沙箱门槛（= 加字段前的行为），
        // 不能被新增的形状校验半路拦成 CommandFailed。
        let err = fixture
            .manager
            .execute_plugin_command(&fixture.plugin_id, "greet", &Value::Null)
            .expect_err("缺 subprocess_execution 应被沙箱拒绝");
        assert!(
            matches!(&err, PluginError::PermissionDenied(msg) if msg.contains("subprocess_execution")),
            "期望沙箱的 PermissionDenied（证明形状关未介入），实际 {err:?}"
        );
        assert!(!fixture.marker().exists(), "被拒绝的命令不该跑过脚本");
        fixture.cleanup();
    }

    /// 契约 E-2：声明了非 object 的 `inputSchema` ⇒ 清单校验报 InvalidCommandInputSchema。
    #[test]
    fn non_object_command_input_schema_is_a_manifest_error() {
        let raw =
            vec![command_manifest_entry("greet", Some(serde_json::json!("type=object")), None)];
        let mut errors = Vec::new();

        let validated = build_manifest_commands(Path::new("."), raw, &mut errors);

        assert!(
            errors.contains(&PluginManifestValidationError::InvalidCommandInputSchema {
                command_name: "greet".to_string()
            }),
            "应报 InvalidCommandInputSchema，实际 {errors:?}"
        );
        // 档位非法才 continue，schema 非法仍保留条目（与 tool 侧只 continue 档位一致）。
        assert_eq!(validated.len(), 1);
    }

    /// 契约 E-3：声明了不在三档之内的 `requiredPermission` ⇒ InvalidCommandRequiredPermission。
    #[test]
    fn unknown_command_required_permission_is_a_manifest_error() {
        let raw = vec![command_manifest_entry("greet", None, Some("yolo-all-access"))];
        let mut errors = Vec::new();

        let validated = build_manifest_commands(Path::new("."), raw, &mut errors);

        assert!(
            errors.contains(&PluginManifestValidationError::InvalidCommandRequiredPermission {
                command_name: "greet".to_string(),
                permission: "yolo-all-access".to_string()
            }),
            "应报 InvalidCommandRequiredPermission，实际 {errors:?}"
        );
        assert!(validated.is_empty(), "非法档位的条目不得进入已校验清单");
        // 文案必须是 `plugin command`，不能误报成 tool。
        let message = errors
            .iter()
            .find(|e| {
                matches!(e, PluginManifestValidationError::InvalidCommandRequiredPermission { .. })
            })
            .expect("变体应在 errors 里")
            .to_string();
        assert!(message.contains("plugin command"), "文案应指向 command，实际 {message}");
        assert!(!message.contains("plugin tool"), "不得复用 tool 侧文案：{message}");
    }

    /// 契约 C 的真值表：档位按**具体权限名**判定，`subprocess_execution` 不得顶上。
    #[test]
    fn command_gate_requires_specific_permission_names() {
        let read_only = PluginToolPermission::ReadOnly;
        let write = PluginToolPermission::WorkspaceWrite;
        let full = PluginToolPermission::DangerFullAccess;
        let gate = |level: PluginToolPermission, labels: &[&str]| {
            PluginManager::check_command_gate(level, "greet", &permissions_of(labels))
        };

        // ReadOnly：与今天一致，只要沙箱允许起进程就放行。
        assert!(gate(read_only, &["subprocess_execution"]).is_ok());
        assert!(gate(read_only, &[]).is_ok());

        // WorkspaceWrite：write / file_system_write 都算。
        assert!(gate(write, &["write"]).is_ok());
        assert!(gate(write, &["file_system_write"]).is_ok());
        // ……但 execute 与 subprocess_execution 不算（用 implies_* 会恒过）。
        assert!(matches!(
            gate(write, &["subprocess_execution"]),
            Err(PluginError::PermissionDenied(_))
        ));
        assert!(matches!(gate(write, &["execute"]), Err(PluginError::PermissionDenied(_))));
        assert!(matches!(gate(write, &["read"]), Err(PluginError::PermissionDenied(_))));

        // DangerFullAccess：只有 execute 算，subprocess_execution + write 也不算。
        assert!(gate(full, &["execute"]).is_ok());
        assert!(matches!(
            gate(full, &["subprocess_execution", "write"]),
            Err(PluginError::PermissionDenied(_))
        ));
    }

    /// 契约 C 的最小形状校验：只认 object + `required` 键存在性。
    #[test]
    fn command_input_shape_check_is_minimal() {
        let schema = serde_json::json!({ "type": "object", "required": ["who"] });
        let shape =
            |input: Value| PluginManager::check_command_input_shape("greet", &schema, &input);

        assert!(shape(serde_json::json!({ "who": "world" })).is_ok());
        assert!(matches!(shape(serde_json::json!({})), Err(PluginError::CommandFailed(_))));
        assert!(matches!(shape(Value::Null), Err(PluginError::CommandFailed(_))));
        assert!(matches!(shape(serde_json::json!([1, 2])), Err(PluginError::CommandFailed(_))));
        // 只校验键存在，不校验值形态：null 也算「在」。
        assert!(shape(serde_json::json!({ "who": null })).is_ok());
        // `required` 非数组 ⇒ 无法判定即拒绝。
        let broken = serde_json::json!({ "required": "who" });
        assert!(matches!(
            PluginManager::check_command_input_shape(
                "greet",
                &broken,
                &serde_json::json!({ "who": 1 })
            ),
            Err(PluginError::CommandFailed(_))
        ));
    }

    /// 契约 E-4：命令声明 `workspace-write`、插件只声明 `subprocess_execution`
    /// ⇒ `PermissionDenied`，且**确实没起子进程**（marker 文件不存在）。
    #[test]
    fn workspace_write_command_is_denied_when_backed_only_by_subprocess_execution() {
        let fixture = CommandPluginFixture::new(
            "cmd-deny",
            r#"["subprocess_execution"]"#,
            r#""requiredPermission": "workspace-write""#,
            false,
        );

        let err = fixture
            .manager
            .execute_plugin_command(&fixture.plugin_id, "greet", &serde_json::json!({}))
            .expect_err("subprocess_execution 不能顶替 workspace-write");
        assert!(
            matches!(err, PluginError::PermissionDenied(_)),
            "期望 PermissionDenied，实际 {err:?}"
        );
        assert!(!fixture.marker().exists(), "拒绝必须发生在 spawn 之前：marker 文件不该存在");

        fixture.cleanup();
    }

    /// 契约 E-5：命令声明 `workspace-write`、插件声明 `write` ⇒ 放行到执行链。
    ///
    /// 门禁本身永远断言（不起进程）；端到端真跑沿用既有门控
    /// `AXAGENT_TEST_PLUGIN_SUBPROCESS=1`，缺环境时显式跳过、不伪造通过。
    ///
    /// 夹具声明的是**绝对路径**命令，于是本用例同时是「任务 #47 修复不破坏绝对路径声明」
    /// 的端到端回归锁；相对路径的正例见同批新增的 `relative_command_declaration_runs_after_fix`。
    #[test]
    fn workspace_write_command_is_allowed_when_plugin_declares_write() {
        assert!(
            PluginManager::check_command_gate(
                PluginToolPermission::WorkspaceWrite,
                "greet",
                &permissions_of(&["read", "write", "subprocess_execution"])
            )
            .is_ok(),
            "声明了 write 应放行到执行链"
        );

        if std::env::var("AXAGENT_TEST_PLUGIN_SUBPROCESS").as_deref() != Ok("1") {
            eprintln!(
                "SKIP: 未设置 AXAGENT_TEST_PLUGIN_SUBPROCESS=1，跳过真实 spawn 的端到端断言。\
                 \n      本用例的后半段**未通过，也未被验证** —— 前半段的门禁断言已执行。"
            );
            return;
        }

        let fixture = CommandPluginFixture::new(
            "cmd-allow",
            r#"["read", "write", "subprocess_execution"]"#,
            r#""requiredPermission": "workspace-write", "inputSchema": { "type": "object", "required": ["who"] }"#,
            false,
        );

        let output = fixture
            .manager
            .execute_plugin_command(
                &fixture.plugin_id,
                "greet",
                &serde_json::json!({ "who": "world" }),
            )
            .expect("放行后命令应执行成功");
        assert!(output.contains("command=greet"), "命令名应注入执行环境：{output}");
        assert!(fixture.marker().exists(), "脚本应真的跑过");

        // 声明了 schema ⇒ 缺必填字段被挡在 spawn 之前（marker 不新增）。
        let err = fixture
            .manager
            .execute_plugin_command(&fixture.plugin_id, "greet", &serde_json::json!({}))
            .expect_err("声明 required 后缺字段必须拒绝");
        assert!(matches!(err, PluginError::CommandFailed(_)), "期望 CommandFailed，实际 {err:?}");
        assert!(err.to_string().contains("who"), "错误应点出缺失字段：{err}");

        fixture.cleanup();
    }

    // ==== 任务 #47：命令声明的相对路径解析（与 tool 侧共用 resolve_hook_entry）====

    /// 解析语义逐情形锁定。断言对象是 tool 侧那条 `resolve_hook_entry` —— 命令通道修复后
    /// 复用的就是它，故这里成立即等于两条通道的解析语义不分叉（禁区 12）。
    ///
    /// 安装根用系统临时目录、绝对用例也在其内构造：Windows 上「有根无盘」的串
    /// （`/usr/bin/node`）经 `Path::push` 只替换盘符之后的部分，跨平台结论不同，不能拿来断言。
    #[test]
    fn command_program_resolution_matches_tool_side_semantics() {
        let root = std::env::temp_dir();
        let absolute = root.join("bin").join("greet.sh");
        // 绝对路径声明原样保留（老插件不回归）。
        assert_eq!(
            resolve_hook_entry(&root, &absolute.display().to_string()),
            absolute.display().to_string()
        );
        // 相对形态（带 `./` 前缀）按安装根解析 —— 本次修复的正是这一支。
        assert_eq!(
            resolve_hook_entry(&root, "./bin/greet.sh"),
            root.join("./bin/greet.sh").display().to_string()
        );
        // 字面命令交给 PATH 查找；**不带 `./` 前缀的相对串也走这一支**（tool 侧既有语义，
        // 本修复刻意不扩，否则老插件里同名的 PATH 命令会静默改指向安装目录）。
        assert_eq!(resolve_hook_entry(&root, "node"), "node");
        assert_eq!(resolve_hook_entry(&root, "bin/greet.sh"), "bin/greet.sh");
    }

    /// 正例：`./commands/<脚本>` 形态的命名命令在修复后**真的能跑**（修复前必然 spawn 失败）。
    ///
    /// 沿用既有门控 `AXAGENT_TEST_PLUGIN_SUBPROCESS=1`：不设该变量时显式跳过并写明「未验证」，
    /// 不伪造通过（与 `workspace_write_command_is_allowed_when_plugin_declares_write` 同一口径）。
    #[test]
    fn relative_command_declaration_runs_after_fix() {
        if std::env::var("AXAGENT_TEST_PLUGIN_SUBPROCESS").as_deref() != Ok("1") {
            eprintln!(
                "SKIP: 未设置 AXAGENT_TEST_PLUGIN_SUBPROCESS=1，跳过相对路径命令的真 spawn 断言。\
                 \n      本用例的后半段**未通过，也未被验证**。"
            );
            return;
        }

        let fixture = CommandPluginFixture::new(
            "cmd-relative",
            r#"["read", "write", "subprocess_execution"]"#,
            r#""requiredPermission": "workspace-write""#,
            true,
        );

        let output = fixture
            .manager
            .execute_plugin_command(
                &fixture.plugin_id,
                "greet",
                &serde_json::json!({ "who": "world" }),
            )
            .expect("相对路径声明修复后应执行成功");
        assert!(output.contains("command=greet"), "命令名应注入执行环境：{output}");
        assert!(fixture.marker().exists(), "脚本应真的跑过（marker 落在安装目录）");

        fixture.cleanup();
    }

    // ==== 任务 #44：逐页声明 `pages[]` 的清单校验 ====

    /// 一份合法的 bundle 页声明，供各用例复用。
    const BUNDLE_PAGE_JSON: &str =
        r#"[{ "id": "main", "title": "Main", "entry": "pages/index.html", "loading": "bundle" }]"#;

    /// 写一份只声明 `pages[]` 的已安装插件目录 + registry（可选钉下安装期哈希）。
    ///
    /// 形状照 `manager_with_panel_pack`：manifest 与资产**先**落盘、再算目录哈希，
    /// 否则锚点与内容对不上，夹具会把「未建立锚点」误验成「资产解析本身不可用」。
    /// `installed_dir` 允许落在安装根之外，用来验 asset 作用域那道防线。
    fn manager_with_page_pack_at(
        temp: &Path,
        installed_dir: &Path,
        plugin_id: &str,
        pages_json: &str,
        ui_json: Option<&str>,
        pinned: bool,
    ) -> PluginManager {
        std::fs::create_dir_all(installed_dir.join("pages")).expect("创建页面目录");
        std::fs::write(installed_dir.join("pages").join("index.html"), "<html>page</html>")
            .expect("写入页面资产");
        std::fs::write(
            installed_dir.join(MANIFEST_FILE_NAME),
            format!(
                r#"{{ "name": "{plugin_id}", "version": "1.0.0", "description": "pages ui",
                     "pages": {pages_json} }}"#
            ),
        )
        .expect("写入 manifest");
        // `ui.json` 必须与 manifest 一起在算哈希**之前**落盘：schema 页的 `entry`
        // 是 `ui.json` 的 key（任务 #149 第 2 步），而锚点是整目录哈希 ——
        // 后补文件会让「已钉锚点」的夹具在资产解析时被判成内容被篡改。
        if let Some(ui_json) = ui_json {
            std::fs::write(installed_dir.join(crate::ui_schema::UI_SCHEMA_FILE_NAME), ui_json)
                .expect("写入 ui.json");
        }

        let mut manager = PluginManager::new(isolated_config(temp));
        let integrity = if pinned {
            hash_plugin_directory(installed_dir)
                .ok()
                .map(|hash| PluginIntegrity { algorithm: "sha256".to_string(), hash })
        } else {
            None
        };
        let mut registry = InstalledPluginRegistry::default();
        registry.plugins.insert(
            plugin_id.to_string(),
            InstalledPluginRecord {
                kind: PluginKind::External,
                id: plugin_id.to_string(),
                name: plugin_id.to_string(),
                version: "1.0.0".to_string(),
                description: "pages ui".to_string(),
                install_path: installed_dir.to_path_buf(),
                source: PluginInstallSource::LocalPath { path: installed_dir.to_path_buf() },
                installed_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                integrity,
            },
        );
        manager.store_registry(&registry).expect("写 registry");
        // 判据③：assets 正例模拟「已安装且已启用」态；External 默认 disabled，
        // 不显式置 enabled 会让所有资产正例在新增的执行层闸前失败。
        // 直接写权威源，不走 enable()——后者要 plugin_registry 全量重扫认可该夹具。
        manager.write_enabled_state(plugin_id, Some(true)).expect("写启用态");
        manager.config.enabled_plugins.insert(plugin_id.to_string(), true);
        manager
    }

    /// 默认形态：插件落在安装根内、锚点已钉死（即 `plugin_install` 之后的真实状态）。
    fn manager_with_page_pack(temp: &Path, pages_json: &str) -> PluginManager {
        let installed = temp.join("plugins").join("installed").join("ui-pages");
        manager_with_page_pack_at(temp, &installed, "ui-pages", pages_json, None, true)
    }

    /// `ui.json` 的夹具内容：只声明一份表单，够 schema 页与 `detailSchema` 引用。
    const SCHEMA_PAGE_UI_JSON: &str = r#"{
        "version": 1,
        "schemas": [
            { "key": "order-form", "title": "Order", "version": "1.0.0", "schema": { "type": "object" } }
        ]
    }"#;

    /// 与 [`manager_with_page_pack`] 同形，但带上 `ui.json`（schema 页的 `entry` 指到其中 key）。
    fn manager_with_schema_page_pack(temp: &Path, pages_json: &str) -> PluginManager {
        let installed = temp.join("plugins").join("installed").join("ui-pages");
        manager_with_page_pack_at(
            temp,
            &installed,
            "ui-pages",
            pages_json,
            Some(SCHEMA_PAGE_UI_JSON),
            true,
        )
    }

    /// 未声明 `loading` ⇒ 清单校验错误并指到具体页 —— §0.12.5 裁定 3「不许混用却不声明」。
    ///
    /// 这是任务 #43/#46 的前置判据：装载形态不声明就只能靠猜，而猜出来的通道
    /// 一旦和产物不符，故障会落在 webview 里（asset 403 或空白 srcdoc），无从归因。
    #[test]
    fn load_rejects_page_without_declared_loading() {
        let temp = temp_dir("page-noloading");
        let dir = temp.join("plugins").join("installed").join("ui-pages");
        std::fs::create_dir_all(dir.join("pages")).expect("创建页面目录");
        std::fs::write(
            dir.join(MANIFEST_FILE_NAME),
            r#"{ "name": "ui-pages", "version": "1.0.0", "description": "d",
                "pages": [ { "id": "main", "title": "Main", "entry": "pages/index.html" } ] }"#,
        )
        .expect("写入 manifest");

        let err = load_plugin_from_directory(&dir).expect_err("未声明 loading 的页面必须被拒");
        let text = err.to_string();
        assert!(
            text.contains("plugin page `main` loading cannot be empty"),
            "应指到页 id 与缺失字段，实际 = {text}"
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 声明了不在词汇表内的 `loading` ⇒ `InvalidPageLoading`，且文案列出两个合法值。
    #[test]
    fn load_rejects_unknown_page_loading() {
        let temp = temp_dir("page-badloading");
        let dir = temp.join("plugins").join("installed").join("ui-pages");
        std::fs::create_dir_all(dir.join("pages")).expect("创建页面目录");
        std::fs::write(
            dir.join(MANIFEST_FILE_NAME),
            r#"{ "name": "ui-pages", "version": "1.0.0", "description": "d",
                "pages": [ { "id": "main", "title": "Main", "entry": "pages/index.html",
                             "loading": "webview" } ] }"#,
        )
        .expect("写入 manifest");

        let text = load_plugin_from_directory(&dir).expect_err("非法 loading 必须被拒").to_string();
        assert!(text.contains("plugin page `main` loading `webview`"), "实际 = {text}");
        assert!(text.contains("bundle, schema"), "文案应列出合法词汇，实际 = {text}");
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 空 id / 重复 id 在载入期即拒 —— 前端渲染与资产寻址都以 `page.id` 为键（面板同口径）。
    #[test]
    fn load_rejects_duplicate_or_empty_page_id() {
        for (label, pages_json, expect) in [
            (
                "dup",
                r#"[
                    { "id": "a", "title": "A", "entry": "pages/index.html", "loading": "bundle" },
                    { "id": "a", "title": "B", "entry": "pages/index.html", "loading": "bundle" }
                ]"#,
                "is duplicated",
            ),
            (
                "empty",
                r#"[{ "id": "  ", "title": "A", "entry": "pages/index.html", "loading": "bundle" }]"#,
                "cannot be empty",
            ),
        ] {
            let temp = temp_dir(&format!("page-{label}"));
            manager_with_page_pack(&temp, pages_json);
            let dir = temp.join("plugins").join("installed").join("ui-pages");
            let text = load_plugin_from_directory(&dir)
                .expect_err(&format!("{label} 应被拒绝"))
                .to_string();
            assert!(
                text.contains("plugin page") && text.contains(expect),
                "{label}: 实际 = {text}"
            );
            let _ = std::fs::remove_dir_all(&temp);
        }
    }

    /// `entry` 的资产形状校验**只适用于 bundle**；schema 页的 `entry` 是本插件 `ui.json` 的 key。
    ///
    /// 两个方向都要断言：把 schema key 按文件路径验 ⇒ 合法声明被拒（`order-form` 会被
    /// 判成「缺扩展名」）；bundle 的越界声明不验 ⇒ 坏声明进仓。后者与面板共用
    /// [`asset_entry_shape_error`]，故合法值与拒因都应与面板逐字同源。
    #[test]
    fn page_entry_validation_follows_declared_loading() {
        let temp = temp_dir("page-schema-ok");
        let manager = manager_with_schema_page_pack(
            &temp,
            r#"[{ "id": "order", "title": "Order", "entry": "order-form", "loading": "schema" }]"#,
        );
        let dir = temp.join("plugins").join("installed").join("ui-pages");
        let manifest =
            load_plugin_from_directory(&dir).expect("schema 页的 entry 不得按文件路径校验");
        assert_eq!(manifest.pages.len(), 1);
        assert_eq!(manifest.pages[0].loading, PluginPageLoading::Schema);
        assert_eq!(manifest.pages[0].entry, "order-form");
        // schema 页没有文件资产，解析命令必须拒绝（见 resolve_page_asset_refuses_schema_page）。
        assert!(manager.resolve_page_asset("ui-pages", "order").is_err());
        let _ = std::fs::remove_dir_all(&temp);

        for (label, entry, expect) in [
            ("escape", "../outside.html", "must not escape the plugin directory"),
            ("absolute", "/etc/passwd.html", "must be relative to the plugin directory"),
            ("drive", "C:/Windows/x.html", "must not be an absolute path"),
            ("ext", "page.exe", "is not allowed"),
        ] {
            let temp = temp_dir(&format!("page-{label}"));
            let dir = temp.join("plugins").join("installed").join("ui-pages");
            std::fs::create_dir_all(dir.join("pages")).expect("创建页面目录");
            std::fs::write(
                dir.join(MANIFEST_FILE_NAME),
                format!(
                    r#"{{ "name": "ui-pages", "version": "1.0.0", "description": "d",
                         "pages": [ {{ "id": "main", "title": "Main", "entry": "{entry}",
                                       "loading": "bundle" }} ] }}"#
                ),
            )
            .expect("写入 manifest");

            let text = load_plugin_from_directory(&dir)
                .expect_err(&format!("{label} 形态的 entry 应被拒"))
                .to_string();
            assert!(text.contains("plugin page `main` has an invalid entry"), "{label}: {text}");
            assert!(text.contains(expect), "{label}: 期望含 `{expect}`，实际 = {text}");
            let _ = std::fs::remove_dir_all(&temp);
        }
    }

    // ==== 任务 #43：(pluginId, pageId) → 绝对资产路径 ====

    /// 正例：宿主绑定的 `(pluginId, pageId)` 解析出**安装目录内**的绝对路径。
    ///
    /// `absolutePath` 必须能直接打开（这就是给前端 `convertFileSrc` 用的那个值），
    /// 且 Windows 上不得带 `\\?\` verbatim 前缀 —— tauri 的 `assetProtocol.scope` 按
    /// 普通路径做 glob 匹配，带着前缀会退化成一个只在运行期出现、极难定位的 403。
    #[test]
    fn resolve_page_asset_returns_canonical_path_inside_install_root() {
        let temp = temp_dir("page-resolve");
        let manager = manager_with_page_pack(&temp, BUNDLE_PAGE_JSON);

        let located = manager.resolve_page_asset("ui-pages", "main").expect("应解析出页面资产路径");
        assert_eq!(located.plugin_id, "ui-pages");
        assert_eq!(located.page_id, "main");
        // entry 原样回带，仅供展示与排错；路径不来自它，而来自后端解析。
        assert_eq!(located.entry, "pages/index.html");
        assert_eq!(located.loading, PluginPageLoading::Bundle);
        assert!(
            !located.absolute_path.starts_with(r"\\?\"),
            "verbatim 前缀应已剥掉：{}",
            located.absolute_path
        );

        let path = Path::new(&located.absolute_path);
        assert!(path.is_absolute(), "必须是绝对路径：{}", located.absolute_path);
        assert_eq!(
            std::fs::read_to_string(path).expect("解析出的路径应可直接打开"),
            "<html>page</html>"
        );
        // 与解析结果同形比对：两边都要经 `path_for_asset_protocol` 剥掉 verbatim 前缀，
        // 否则「已剥」的路径永远不是「未剥」的安装根的子路径，断言会假失败。
        let install_root = path_for_asset_protocol(
            &temp.join("plugins").join("installed").canonicalize().expect("规范化安装根"),
        );
        assert!(
            located.absolute_path.starts_with(&install_root),
            "解析结果应落在安装根内（asset 协议作用域）：{} 不在 {install_root} 内",
            located.absolute_path
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// schema 页**没有文件资产** ⇒ 报错而不是返回空路径。
    /// 空串会被前端当成合法 `absolutePath` 拼进 asset URL，故障又回到运行期。
    #[test]
    fn resolve_page_asset_refuses_schema_page() {
        let temp = temp_dir("page-schema-resolve");
        let manager = manager_with_schema_page_pack(
            &temp,
            r#"[{ "id": "order", "title": "Order", "entry": "order-form", "loading": "schema" }]"#,
        );

        let err = manager
            .resolve_page_asset("ui-pages", "order")
            .expect_err("schema 页没有可解析的文件资产");
        assert!(matches!(err, PluginError::NotFound(_)), "期望 NotFound，实际 = {err:?}");
        assert!(err.to_string().contains("UI Schema"), "应说明走的是 schema 通道：{err}");
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// asset 作用域防线：registry 记录的安装目录落在 `install_root` 之外
    /// （`external_dirs` 发现形态、或手写 registry）⇒ 解析当场拒绝。
    ///
    /// 这一条是「不许前端自己拼 `~/.axagent/plugins/installed/<id>`」的另一半：
    /// 后端也不认安装根之外的路径，否则前端拿到一个 webview 必拒的串，
    /// 而唯一的线索是一条没有上下文的网络错误。
    #[test]
    fn resolve_page_asset_refuses_install_outside_asset_scope() {
        let temp = temp_dir("page-scope");
        let outside = temp.join("external-dev").join("ui-pages");
        let manager =
            manager_with_page_pack_at(&temp, &outside, "ui-pages", BUNDLE_PAGE_JSON, None, true);

        let err =
            manager.resolve_page_asset("ui-pages", "main").expect_err("安装根外的资产应被拒绝");
        assert!(matches!(err, PluginError::PermissionDenied(_)), "实际 = {err:?}");
        assert!(err.to_string().contains("install root"), "应点出作用域，实际 = {err}");
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 篡改与「没锚点」不得塌成一个提示（与面板共用 [`verify_install_anchor`]）。
    ///
    /// 两者的用户动作相反：锚点缺失是「重装一次就好」，内容被改是「移除或重装」；
    /// 面板那条已经分了两个错误码，页面这条复用同一份实现，不允许在此处合并。
    #[test]
    fn resolve_page_asset_distinguishes_tampered_and_unpinned() {
        let temp = temp_dir("page-tamper");
        let manager = manager_with_page_pack(&temp, BUNDLE_PAGE_JSON);
        let installed = temp.join("plugins").join("installed").join("ui-pages");
        std::fs::write(installed.join("pages").join("index.html"), "<html>tampered</html>")
            .expect("改写资产");
        let err = manager.resolve_page_asset("ui-pages", "main").expect_err("篡改后应拒绝");
        assert!(matches!(err, PluginError::IntegrityMismatch(_)), "实际 = {err:?}");
        let _ = std::fs::remove_dir_all(&temp);

        let temp = temp_dir("page-unpinned");
        let manager = manager_with_page_pack(&temp, BUNDLE_PAGE_JSON);
        // 直接把锚点抹掉：等价于「早于此机制安装」的插件状态。
        let mut registry = manager.load_registry().expect("读 registry");
        let record = registry.plugins.get_mut("ui-pages").expect("夹具应有 ui-pages 记录");
        record.integrity = None;
        manager.store_registry(&registry).expect("改写 registry");
        let err = manager.resolve_page_asset("ui-pages", "main").expect_err("无锚点应 fail-closed");
        assert!(matches!(err, PluginError::IntegrityUnpinned(_)), "实际 = {err:?}");
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 判据③：关掉插件后，资产解析在执行层拒绝（前端 flattenPages 只做 UI 过滤，
    /// 脚本绕过 UI 直接 invoke 本命令时不能拿到已关插件的页面资产）。
    #[test]
    fn resolve_page_asset_refuses_disabled_plugin() {
        let temp = temp_dir("page-disabled");
        let mut manager = manager_with_page_pack(&temp, BUNDLE_PAGE_JSON);
        // 先确认为启用态能解析，再将同一夹具关掉——差异只来自 enabled 开关。
        manager.resolve_page_asset("ui-pages", "main").expect("启用时应可解析");
        manager.write_enabled_state("ui-pages", Some(false)).expect("写关闭态");
        manager.config.enabled_plugins.insert("ui-pages".to_string(), false);

        let err = manager.resolve_page_asset("ui-pages", "main").expect_err("关闭后应拒绝");
        let text = err.to_string();
        assert!(matches!(err, PluginError::CommandFailed(_)), "实际 = {text}");
        assert!(text.contains("ui-pages") && text.contains("未启用"), "实际 = {text}");
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// 词汇表与序列化形状的自证（禁区 13 的 Rust 半边；TS 半边由比对脚本覆盖）。
    #[test]
    fn page_loading_vocabulary_and_dto_shape_are_self_consistent() {
        for label in PluginPageLoading::LABELS {
            let parsed =
                PluginPageLoading::parse(label).unwrap_or_else(|| panic!("`{label}` 应可解析"));
            assert_eq!(parsed.as_str(), label, "`{label}` 的 as_str 与词汇表分叉");
            assert_eq!(
                serde_json::to_value(parsed).expect("应可序列化"),
                serde_json::Value::String(label.to_string()),
                "serde 输出必须与 LABELS 一致"
            );
        }
        // 大小写与空串都不在词汇表内：不做宽容，宽容处即分叉处。
        assert!(PluginPageLoading::parse("Bundle").is_none());
        assert!(PluginPageLoading::parse("").is_none());

        let page = PluginPage {
            id: "main".to_string(),
            title: "Main".to_string(),
            entry: "pages/index.html".to_string(),
            loading: PluginPageLoading::Bundle,
        };
        let json = serde_json::to_value(&page).expect("页面声明应可序列化");
        assert_eq!(sorted_keys(&json), ["entry", "id", "loading", "title"]);
        assert_eq!(json["loading"], serde_json::json!("bundle"));

        let located = PluginPageAssetLocation {
            plugin_id: "ui-pages".to_string(),
            page_id: "main".to_string(),
            entry: page.entry.clone(),
            loading: PluginPageLoading::Bundle,
            absolute_path: "C:\\plugins\\installed\\ui-pages\\pages\\index.html".to_string(),
        };
        let json = serde_json::to_value(&located).expect("解析结果应可序列化");
        assert_eq!(sorted_keys(&json), ["absolutePath", "entry", "loading", "pageId", "pluginId"]);

        // `PluginManifest` 结构体**没有** rename_all ⇒ 清单键就是字段名 `pages`（与
        // `dashboard_panels` 同形）；前端投影 DTO 才用 camelCase。这条不对称写在这里，
        // 是为了让「给 manifest 加 rename_all」这种改动必须经过本断言。
        let manifest = demo_manifest_with_capabilities();
        let json = serde_json::to_value(&manifest).expect("manifest 应可序列化");
        assert_eq!(json["pages"], serde_json::json!([]), "未声明 pages 应序列化成空数组");
        assert!(json.get("dashboard_panels").is_some(), "清单键应保持 snake_case");
    }

    /// 取 JSON object 的键并排序（`serde_json::Map` 默认按序，显式排序以免 feature 变化）。
    fn sorted_keys(value: &Value) -> Vec<&str> {
        value.as_object().expect("应是 JSON object").keys().map(String::as_str).collect()
    }

    // ==== 任务 #59 + #73：命名命令回退判据（S-1 起判据读**结构化码字段**）====

    /// 命令帧在宿主侧成形的错误。
    ///
    /// `reason` 与 `code` 的关系逐字照 `worker.rs::transport_error_with_code`：带码时
    /// `reason` 形如 `{code}: {message}`，纯 I/O 断裂没有码 ⇒ `reason` 是中文句子本体。
    /// 任务 #73 S-1 之后判据**只读 `code`**，`reason` 的形态不再参与判定。
    fn command_transport(code: Option<&str>, message: &str) -> WorkerError {
        WorkerError::Transport {
            plugin_id: "legacy@external".to_string(),
            reason: match code {
                Some(code) => format!("{code}: {message}"),
                None => message.to_string(),
            },
            code: code.map(str::to_string),
        }
    }

    /// 正例：错误码为 `PLUGIN_UNKNOWN_OP` ⇒ 允许回退到 spawn-on-call（存量旧 worker 不回归）。
    #[test]
    fn unknown_op_error_code_allows_legacy_fallback() {
        assert!(
            command_error_allows_fallback(&command_transport(
                Some(error_codes::PLUGIN_UNKNOWN_OP),
                "未知操作：command"
            )),
            "旧 worker 的派发前能力拒绝必须触发回退"
        );
        // 只有裸码、没有说明文本同样命中：判据读字段，与 `reason` 的拼接形态无关。
        assert!(command_error_allows_fallback(&command_transport(
            Some(error_codes::PLUGIN_UNKNOWN_OP),
            ""
        )));
        // 没有码的错误（纯 I/O）不参与本判据。
        assert!(!command_error_allows_fallback(&command_transport(None, "")));
        assert!(!command_error_allows_fallback(&command_transport(
            None,
            "worker 连接已关闭，未收到配对响应"
        )));
    }

    /// S-1 的核心锁：**改文案不会静默关掉回退**。
    ///
    /// 旧判据取 `reason` 首个 `:` 之前的段，在信封成形处加任何前缀（本地化、插件名、
    /// 「通道错误：」）都会让首段变成中文句子 ⇒ 回退整条静默消失（M1 原病复发，且不红不响）。
    /// 现在三种漂移形态都必须照旧回退 —— 这条断言就是「读字段」这句话的可执行版本。
    #[test]
    fn reason_text_drift_does_not_disable_the_fallback() {
        let unknown_op = error_codes::PLUGIN_UNKNOWN_OP;
        for reason_prefix in ["", "通道错误：", "与插件 `legacy@external` 的通道错误："]
        {
            let error = WorkerError::Transport {
                plugin_id: "legacy@external".to_string(),
                reason: format!("{reason_prefix}{unknown_op}: 未知操作：command"),
                code: Some(unknown_op.to_string()),
            };
            assert!(
                command_error_allows_fallback(&error),
                "文案加了前缀 `{reason_prefix}` 后回退不得失效（判据读字段不读文本）"
            );
        }
        // 反向：码位不是 UNKNOWN_OP 时，**即便整串就是旧的可命中形态**也不回退 ——
        // 证明文本已从判据里彻底退出，不存在「字段 + 文本」双判据。
        let text_only = WorkerError::Transport {
            plugin_id: "legacy@external".to_string(),
            reason: format!("{unknown_op}: 未知操作：command"),
            code: Some(error_codes::PLUGIN_COMMAND_FAILED.to_string()),
        };
        assert!(
            !command_error_allows_fallback(&text_only),
            "码位是命令级失败 ⇒ 文本再像也不回退（否则二次执行已跑过的命令）"
        );
    }

    /// 反例（守护「不回退已执行的命令」）：`PLUGIN_UNKNOWN_OP` 只出现在**说明文本**里，
    /// 错误码是命令级 ⇒ 不回退。命令进了 worker 逻辑之后才失败时，worker 可能把下游插件
    /// `call_seam` 的错误原文抄进 message，此时按子串匹配回退等于**二次执行**同一条命令。
    #[test]
    fn unknown_op_in_the_message_text_does_not_fall_back() {
        assert!(
            !command_error_allows_fallback(&command_transport(
                Some(error_codes::PLUGIN_COMMAND_FAILED),
                "下游插件回了 PLUGIN_UNKNOWN_OP，本命令已执行一半"
            )),
            "错误码不是 PLUGIN_UNKNOWN_OP 时，即便文本含该串也不得回退"
        );
        // 命令名恰好叫 PLUGIN_UNKNOWN_OP 之类的干扰形态同样不回退。
        assert!(!command_error_allows_fallback(&command_transport(
            Some(error_codes::PLUGIN_COMMAND_UNKNOWN),
            "没有该命令"
        )));
    }

    /// M-1 与 M-2/S-3 的判据**必须分开**（派单硬约束）：
    /// 「worker 进程不在」⇒ 回退；「worker 在、但曾被实证装死」⇒ 拒绝且等显式复位。
    /// 两者塞进同一个 bool 就是 M-1/M-2/M-3 的共同根（PLAN §26.3）。
    #[test]
    fn worker_gone_falls_back_but_suspect_rejection_does_not() {
        // ① 帧未送达（进程退出 / 管道已断 / 写失败）⇒ 零副作用 ⇒ 回退。
        let gone = WorkerError::NotDelivered {
            plugin_id: "dead@external".to_string(),
            op: axagent_plugin_proto::ops::COMMAND.to_string(),
            reason: "worker 进程已退出（exit code: 0xc0000005）".to_string(),
        };
        assert!(command_error_allows_fallback(&gone), "worker 已死必须退回原通道（M-1 契约）");
        assert!(command_error_is_worker_gone(&gone));
        assert!(!command_error_is_unknown_op(&gone), "两类回退理由各走各的判据，不许互相顶");

        // ② 脏闸拒绝（任务 #73 S-3 在 `execute_command` 里读 `seam_suspect` 后报的错）⇒ 不回退。
        let suspect = command_transport(Some(error_codes::SEAM_PROVIDER_UNAVAILABLE), "连接可疑");
        assert!(
            !command_error_allows_fallback(&suspect),
            "进程还在、只是曾被实证装死 ⇒ 拒绝，不能靠回退把预算再付一遍"
        );
        assert!(command_error_is_seam_suspect(&suspect), "该错误须被识别为脏闸拒绝（负向锁）");

        // ③ 超时同样不回退（超时不等于取消，worker 可能仍在跑）。
        let timeout = WorkerError::Timeout {
            plugin_id: "legacy@external".to_string(),
            op: "command".to_string(),
            timeout_ms: 30_000,
        };
        assert!(!command_error_allows_fallback(&timeout), "超时不得回退（可能仍在执行）");

        // ④ 非命令帧通道的错误不参与回退（形态再像也不命中：判据只认 Transport 的码字段）。
        assert!(
            !command_error_allows_fallback(&WorkerError::Handshake {
                plugin_id: "legacy@external".to_string(),
                reason: "PLUGIN_UNKNOWN_OP: 未知操作：describe".to_string(),
            }),
            "非命令帧通道错误不参与回退"
        );
    }

    /// 任务 #75：`manifest.pages[]` 由已安装插件实时投影进 `list_plugin_pages` ——
    /// 与 `list_dashboard_plugins` 同形，没有第二套 registry / 清单缓存。
    #[test]
    fn list_plugin_pages_projects_installed_manifest_pages() {
        let temp = temp_dir("pageproj");
        let installed_root = temp.join("plugins").join("installed");

        // ① 声明了 pages 的插件（bundle + schema 各一页）。
        let page_pack = installed_root.join("page-pack");
        std::fs::create_dir_all(page_pack.join("web")).expect("创建插件目录");
        std::fs::write(page_pack.join("web").join("index.html"), "<html></html>")
            .expect("写入页面资产");
        std::fs::write(
            page_pack.join(MANIFEST_FILE_NAME),
            r#"{
                "name": "page-pack",
                "version": "2.3.4",
                "description": "pages",
                "author": "ax",
                "pages": [
                    {
                        "id": "chart",
                        "title": "Chart",
                        "entry": "web/index.html",
                        "loading": "bundle"
                    },
                    {
                        "id": "order",
                        "title": "Order",
                        "entry": "order-form",
                        "loading": "schema"
                    }
                ]
            }"#,
        )
        .expect("写入 manifest");
        // schema 页的 `entry` 必须是本插件 `ui.json` 的 key（任务 #149 第 2 步）。
        std::fs::write(page_pack.join(crate::ui_schema::UI_SCHEMA_FILE_NAME), SCHEMA_PAGE_UI_JSON)
            .expect("写入 ui.json");

        // ② 未声明 pages 的插件：即使有 dashboard_panels 也不得混入页面清单。
        let panel_pack = installed_root.join("panel-pack");
        std::fs::create_dir_all(&panel_pack).expect("创建插件目录");
        std::fs::write(
            panel_pack.join(MANIFEST_FILE_NAME),
            r#"{
                "name": "panel-pack",
                "version": "1.0.0",
                "description": "panels",
                "dashboard_panels": [
                    { "id": "p1", "title": "P1", "componentName": "panel-pack.p1" }
                ]
            }"#,
        )
        .expect("写入 manifest");

        // ③ 坏插件：registry 里登记了记录，但安装目录已不存在 ⇒ 发现期按陈旧记录剔除，
        // 不产生加载失败，其余插件照常返回（单个坏插件不拖垮整份清单）。
        let manager = PluginManager::new(isolated_config(temp.clone()));
        let ghost_root = installed_root.join("ghost-pack");
        let mut registry = InstalledPluginRegistry::default();
        registry.plugins.insert(
            "ghost-pack".to_string(),
            InstalledPluginRecord {
                kind: PluginKind::External,
                id: "ghost-pack".to_string(),
                name: "ghost-pack".to_string(),
                version: "0.0.1".to_string(),
                description: "ghost".to_string(),
                install_path: ghost_root.clone(),
                source: PluginInstallSource::LocalPath { path: ghost_root },
                installed_at_unix_ms: 1,
                updated_at_unix_ms: 1,
                integrity: None,
            },
        );
        manager.store_registry(&registry).expect("写 registry");

        let list = manager.list_plugin_pages().expect("页面清单投影应成功");

        // ① 声明了 pages 的插件进入清单，`pages` 逐字段相等。
        let entry = list
            .iter()
            .find(|plugin| plugin.name == "page-pack")
            .expect("声明了页面的已安装插件应进入清单");
        assert_eq!(entry.version, "2.3.4");
        assert_eq!(entry.description, "pages");
        assert_eq!(entry.author.as_deref(), Some("ax"));
        assert_eq!(entry.pages.len(), 2);
        assert_eq!(
            entry.pages[0],
            crate::types::PluginPage {
                id: "chart".to_string(),
                title: "Chart".to_string(),
                entry: "web/index.html".to_string(),
                loading: crate::types::PluginPageLoading::Bundle,
            }
        );
        assert_eq!(
            entry.pages[1],
            crate::types::PluginPage {
                id: "order".to_string(),
                title: "Order".to_string(),
                entry: "order-form".to_string(),
                loading: crate::types::PluginPageLoading::Schema,
            }
        );

        // ② 未声明 pages 的插件不出现；无页面声明的条目不得混入投影。
        assert!(list.iter().all(|plugin| !plugin.pages.is_empty()));
        assert!(!list.iter().any(|plugin| plugin.name == "panel-pack"));

        // ③ 坏插件被跳过，其余插件照常返回。
        assert!(!list.iter().any(|plugin| plugin.name == "ghost-pack"));
        assert!(!list.is_empty(), "坏插件不得拖垮其余插件的页面投影");

        let _ = std::fs::remove_dir_all(&temp);
    }

    /// B10-1：`agent_command_specs` 只投影**已启用**插件的命名命令，并原样带出
    /// `requiredPermission`（安全档位换算由主壳 `plugin_command_safety` 负责，不在这里做）。
    #[test]
    fn agent_command_specs_projects_only_enabled_plugins() {
        let fixture = CommandPluginFixture::new(
            "agent-cmd",
            r#"[ "read", "file_system_write" ]"#,
            r#""requiredPermission": "workspace-write""#,
            true,
        );
        let CommandPluginFixture { mut manager, plugin_id, config_home, source, .. } = fixture;

        let specs = manager.agent_command_specs().expect("投影插件命令");
        assert_eq!(specs.len(), 1, "实际 {specs:?}");
        assert_eq!(specs[0].name, "greet");
        assert_eq!(specs[0].plugin_id, plugin_id);
        assert_eq!(specs[0].required_permission.as_deref(), Some("workspace-write"));
        assert_eq!(specs[0].description, "Greet");
        // 参数面同样要过投影，且**未声明时保持 `None`**（不落空 object）：本夹具只声明了
        // requiredPermission，清单里没有 inputSchema 键。
        assert_eq!(specs[0].input_schema, None, "未声明的参数面不得被投影成某个默认值");

        manager.disable(&plugin_id).expect("禁用插件");
        assert!(
            manager.agent_command_specs().expect("禁用后仍可投影").is_empty(),
            "禁用插件不得再向智能体命令面贡献命令"
        );

        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);
    }

    /// 任务 #171：`agent_command_specs` 必须把清单的 `inputSchema` **原样**带到命令面上。
    ///
    /// 这是三跳投影的第一跳。此前该字段在 `PluginCommandManifest`（已解析）到
    /// `PluginAgentCommand` 的投影里被整段丢弃，导致宿主索引文案无法呈现参数面 ——
    /// LLM 看得见命令名、看不见该传什么。本测试锁住「带出且不改写」：
    /// 声明的 properties / required 逐字保留，不排序、不落定、不补默认值。
    #[test]
    fn agent_command_specs_carries_declared_input_schema() {
        let fixture = CommandPluginFixture::new(
            "agent-cmd-schema",
            r#"[ "read" ]"#,
            r#""inputSchema": { "type": "object", "properties": { "symbol": { "type": "string" }, "count": { "type": "integer" } }, "required": [ "symbol" ] }"#,
            true,
        );
        let CommandPluginFixture { manager, plugin_id, config_home, source, .. } = fixture;

        let specs = manager.agent_command_specs().expect("投影插件命令");
        assert_eq!(specs.len(), 1, "实际 {specs:?}");
        assert_eq!(specs[0].plugin_id, plugin_id);

        let declared =
            specs[0].input_schema.as_ref().expect("清单声明了 inputSchema，投影不得丢弃");
        assert_eq!(declared["type"], "object");
        assert!(declared["properties"].get("symbol").is_some(), "实际 {declared}");
        assert!(declared["properties"].get("count").is_some(), "实际 {declared}");
        assert_eq!(
            declared["required"].as_array().map(|items| items.len()),
            Some(1),
            "required 数组不得被改写，实际 {declared}"
        );

        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);
    }

    // ==== B8-1b：插件贡献任务中心任务源（声明校验 + 只读投影）====

    fn task_source_entry(
        id: &str,
        title: &str,
        command: &str,
        color: Option<&str>,
    ) -> RawPluginTaskSource {
        RawPluginTaskSource {
            id: id.to_string(),
            title: title.to_string(),
            command: command.to_string(),
            color: color.map(str::to_string),
            detail_schema: None,
        }
    }

    /// 带 `detailSchema` 的任务源条目（任务 `#149` 第 2 步的 membership 判据用）。
    fn task_source_entry_with_schema(
        id: &str,
        command: &str,
        detail_schema: &str,
    ) -> RawPluginTaskSource {
        RawPluginTaskSource {
            id: id.to_string(),
            title: "标题".to_string(),
            command: command.to_string(),
            color: None,
            detail_schema: Some(detail_schema.to_string()),
        }
    }

    /// 判据 1：`command` 必须指向**同一份清单**已声明的命令。
    ///
    /// 指一条不存在的命令 = 声明一个永远为空的源（任务中心多一个空标签，且没有任何
    /// 报错指向真因）⇒ 装载期就必须拒，而不是等用户点开。
    #[test]
    fn task_sources_must_reference_declared_commands() {
        let names: BTreeSet<&str> = ["rows"].into_iter().collect();
        let mut errors = Vec::new();

        let validated = build_manifest_task_sources(
            vec![
                task_source_entry("stock-runs", "股票在跑的执行", "rows", Some("gold")),
                task_source_entry("ghost-runs", "指错命令", "not_declared", None),
            ],
            &names,
            &BTreeSet::new(),
            &mut errors,
        );

        assert_eq!(validated.len(), 1, "只有指向已声明命令的条目应通过");
        assert_eq!(validated[0].id, "stock-runs");
        assert_eq!(validated[0].color.as_deref(), Some("gold"));
        assert_eq!(errors.len(), 1, "实际 {errors:?}");
        assert!(
            matches!(errors[0], PluginManifestValidationError::TaskSourceCommandUnknown { .. }),
            "应报 TaskSourceCommandUnknown，实际 {:?}",
            errors[0]
        );
        assert!(
            errors[0].to_string().contains("not_declared"),
            "文案必须点出那条不存在的命令，实际 {}",
            errors[0]
        );
    }

    /// 判据 2：id 形状 + 清单内唯一 + 标题非空，逐条指得出是哪一条源、哪个键。
    #[test]
    fn task_source_ids_are_shape_checked_and_unique() {
        let names: BTreeSet<&str> = ["rows"].into_iter().collect();

        // 形状：带 `:` / 空格 / 大写 / 前导 `-` / 超长一律拒（id 会进 runIdOf 的拼接）。
        for bad_id in ["Stock-Runs", "has space", "with:colon", "-lead", "", "   "] {
            let mut errors = Vec::new();
            let validated = build_manifest_task_sources(
                vec![task_source_entry(bad_id, "标题", "rows", None)],
                &names,
                &BTreeSet::new(),
                &mut errors,
            );
            assert!(validated.is_empty(), "`{bad_id}` 不应通过，实际 {validated:?}");
            assert_eq!(errors.len(), 1, "实际 {errors:?}");
        }

        // 同清单内重名 ⇒ DuplicateEntry（kind 必须写 "task source"，别指到 page/tool 上）。
        let mut errors = Vec::new();
        let validated = build_manifest_task_sources(
            vec![
                task_source_entry("dup", "第一份", "rows", None),
                task_source_entry("dup", "第二份", "rows", None),
            ],
            &names,
            &BTreeSet::new(),
            &mut errors,
        );
        assert_eq!(validated.len(), 1);
        assert!(
            matches!(
                errors[0],
                PluginManifestValidationError::DuplicateEntry { kind: "task source", .. }
            ),
            "实际 {errors:?}"
        );

        // 空标题 ⇒ 指得出键。
        let mut errors = Vec::new();
        let validated = build_manifest_task_sources(
            vec![task_source_entry("no-title", "  ", "rows", None)],
            &names,
            &BTreeSet::new(),
            &mut errors,
        );
        assert!(validated.is_empty());
        assert!(
            matches!(
                errors[0],
                PluginManifestValidationError::EmptyEntryField { field: "title", .. }
            ),
            "实际 {errors:?}"
        );
    }

    /// 空白色名按「未声明」处理 —— 与 `loading` / `requiredPermission` 的空白同判，
    /// 不许留出一个 `color: ""` 让宿主 Tag 渲成无色的色块。
    #[test]
    fn task_source_blank_color_collapses_to_none() {
        let names: BTreeSet<&str> = ["rows"].into_iter().collect();
        let mut errors = Vec::new();

        let validated = build_manifest_task_sources(
            vec![task_source_entry("rows-src", "行", "rows", Some("   "))],
            &names,
            &BTreeSet::new(),
            &mut errors,
        );

        assert!(errors.is_empty(), "实际 {errors:?}");
        assert_eq!(validated.len(), 1);
        assert_eq!(validated[0].color, None);
    }

    // ==== 任务 #149 第 2 步：`ui.json` 声明面（插件 schema 的归属锚）====

    /// 写一个带 `ui.json` 的插件源目录。
    fn write_plugin_source_with_ui(source: &Path, manifest_body: &str, ui_json: Option<&str>) {
        let manifest_path = source.join(MANIFEST_RELATIVE_PATH);
        std::fs::create_dir_all(manifest_path.parent().expect("manifest 有父目录"))
            .expect("创建 manifest 目录");
        std::fs::write(&manifest_path, manifest_body).expect("写入 manifest");
        if let Some(ui_json) = ui_json {
            std::fs::write(source.join(crate::ui_schema::UI_SCHEMA_FILE_NAME), ui_json)
                .expect("写入 ui.json");
        }
    }

    /// `detailSchema` 必须是**本插件 `ui.json`** 的 key：不符即拒，且文案指得出
    /// 「哪条源、哪个 key、哪个文件」—— 这是跨插件引用在声明面上的唯一闸门。
    #[test]
    fn task_source_detail_schema_must_be_declared_in_plugin_ui_json() {
        let names: BTreeSet<&str> = ["rows"].into_iter().collect();
        let ui_keys: BTreeSet<String> = ["order-form".to_string()].into_iter().collect();
        let mut errors = Vec::new();

        let validated = build_manifest_task_sources(
            vec![
                task_source_entry_with_schema("ok-runs", "rows", "order-form"),
                task_source_entry_with_schema("bad-runs", "rows", "other-plugin-form"),
            ],
            &names,
            &ui_keys,
            &mut errors,
        );

        assert_eq!(validated.len(), 1, "实际 {validated:?}");
        assert_eq!(validated[0].id, "ok-runs");
        assert_eq!(validated[0].detail_schema.as_deref(), Some("order-form"));
        assert_eq!(errors.len(), 1, "实际 {errors:?}");
        assert!(
            matches!(errors[0], PluginManifestValidationError::TaskSourceSchemaKeyUnknown { .. }),
            "应报 TaskSourceSchemaKeyUnknown，实际 {:?}",
            errors[0]
        );
        let text = errors[0].to_string();
        assert!(text.contains("bad-runs"), "文案要指出是哪条源：{text}");
        assert!(text.contains("other-plugin-form"), "文案要指出是哪个 key：{text}");
        assert!(text.contains("ui.json"), "文案要指出是哪份声明：{text}");
    }

    /// 空白的 `detailSchema` 按「未声明」处理（与 color / loading 的空白同判）。
    #[test]
    fn task_source_blank_detail_schema_collapses_to_none() {
        let names: BTreeSet<&str> = ["rows"].into_iter().collect();
        let mut errors = Vec::new();
        let validated = build_manifest_task_sources(
            vec![task_source_entry_with_schema("blank-runs", "rows", "   ")],
            &names,
            &BTreeSet::new(),
            &mut errors,
        );
        assert!(errors.is_empty(), "空白串不应触发 membership 校验：{errors:?}");
        assert_eq!(validated[0].detail_schema, None);
    }

    /// 端到端：`ui.json` 声明的 key 同时被 `pages[]`（schema 页）与 `taskSources[].detailSchema`
    /// 引用 ⇒ 装得上；把 key 换成未声明的 ⇒ 装不上且文案点出 key。
    #[test]
    fn ui_json_keys_gate_pages_and_task_source_detail_schema() {
        let ui_json = r#"{
  "version": 1,
  "schemas": [
    { "key": "order-form", "title": "Order", "version": "1.0.0", "schema": { "type": "object" } }
  ]
}"#;
        let manifest = |entry: &str, detail: &str| {
            format!(
                r#"{{
  "name": "ui-decl-demo",
  "version": "1.0.0",
  "description": "ui declaration plugin",
  "defaultEnabled": true,
  "permissions": ["read"],
  "commands": [{{ "name": "rows", "description": "rows", "command": "echo" }}],
  "pages": [{{ "id": "order", "title": "Order", "entry": "{entry}", "loading": "schema" }}],
  "taskSources": [{{ "id": "demo-runs", "title": "Runs", "command": "rows", "detailSchema": "{detail}" }}]
}}"#
            )
        };

        // ① 两个引用都指向已声明的 key ⇒ 装得上。
        let source = temp_dir("ui-decl-ok");
        let config_home = temp_dir("ui-decl-ok-home");
        std::fs::create_dir_all(&source).expect("创建源目录");
        write_plugin_source_with_ui(&source, &manifest("order-form", "order-form"), Some(ui_json));
        let mut manager = PluginManager::new(isolated_config(&config_home));
        manager.install(source.to_str().expect("utf8 path")).expect("引用已声明 key 应装得上");
        let list = manager.list_plugin_task_sources().expect("任务源投影应成功");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].task_sources[0].detail_schema.as_deref(), Some("order-form"));
        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);

        // ② 页面 entry 指到未声明的 key ⇒ 装载期就拒。
        let source = temp_dir("ui-decl-badpage");
        let config_home = temp_dir("ui-decl-badpage-home");
        std::fs::create_dir_all(&source).expect("创建源目录");
        write_plugin_source_with_ui(&source, &manifest("ghost-form", "order-form"), Some(ui_json));
        let mut manager = PluginManager::new(isolated_config(&config_home));
        let text = manager
            .install(source.to_str().expect("utf8 path"))
            .expect_err("引用未声明的 schema key 必须被拒")
            .to_string();
        assert!(text.contains("ghost-form"), "文案要指出是哪个 key：{text}");
        assert!(text.contains("plugin page `order`"), "文案要指出是哪一页：{text}");
        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);

        // ③ 任务源 detailSchema 指到未声明的 key ⇒ 装载期就拒。
        let source = temp_dir("ui-decl-badsrc");
        let config_home = temp_dir("ui-decl-badsrc-home");
        std::fs::create_dir_all(&source).expect("创建源目录");
        write_plugin_source_with_ui(&source, &manifest("order-form", "ghost-form"), Some(ui_json));
        let mut manager = PluginManager::new(isolated_config(&config_home));
        let text = manager
            .install(source.to_str().expect("utf8 path"))
            .expect_err("任务源引用未声明的 schema key 必须被拒")
            .to_string();
        assert!(text.contains("demo-runs"), "文案要指出是哪条源：{text}");
        assert!(text.contains("ghost-form"), "文案要指出是哪个 key：{text}");
        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);

        // ④ 没有 ui.json 却声明了 schema 页 ⇒ 同样拒（空集合 = 无任何可引用的 key）。
        let source = temp_dir("ui-decl-missing");
        let config_home = temp_dir("ui-decl-missing-home");
        std::fs::create_dir_all(&source).expect("创建源目录");
        write_plugin_source_with_ui(&source, &manifest("order-form", "order-form"), None);
        let mut manager = PluginManager::new(isolated_config(&config_home));
        let text = manager
            .install(source.to_str().expect("utf8 path"))
            .expect_err("没有 ui.json 的 schema 页必须被拒")
            .to_string();
        assert!(text.contains("order-form"), "实际 = {text}");
        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);

        // ⑤ ui.json 本身非法（版本号 / key 形状）⇒ 报整份声明非法，指到文件名。
        let source = temp_dir("ui-decl-invalid");
        let config_home = temp_dir("ui-decl-invalid-home");
        std::fs::create_dir_all(&source).expect("创建源目录");
        write_plugin_source_with_ui(
            &source,
            &manifest("order-form", "order-form"),
            Some(r#"{ "version": 9, "schemas": [] }"#),
        );
        let mut manager = PluginManager::new(isolated_config(&config_home));
        let text = manager
            .install(source.to_str().expect("utf8 path"))
            .expect_err("非法 ui.json 必须被拒")
            .to_string();
        assert!(text.contains("ui.json"), "实际 = {text}");
        assert!(text.contains("不受支持"), "实际 = {text}");
        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);
    }

    /// 端到端：装一个声明了任务源的插件 ⇒ 投影出该源；禁用后仍投影但带 `enabled = false`
    /// （与 `list_plugin_pages` 同口径：启停过滤留给消费侧）。
    #[test]
    fn list_plugin_task_sources_projects_installed_declarations() {
        let label = "task-src";
        let source = temp_dir(&format!("{label}-source"));
        let config_home = temp_dir(&format!("{label}-home"));
        std::fs::create_dir_all(&source).expect("创建源目录");
        let manifest_path = source.join(MANIFEST_RELATIVE_PATH);
        std::fs::create_dir_all(manifest_path.parent().expect("manifest 有父目录"))
            .expect("创建 manifest 目录");
        std::fs::write(
            &manifest_path,
            r#"{
  "name": "task-src-demo",
  "version": "1.0.0",
  "description": "task source projection plugin",
  "defaultEnabled": true,
  "permissions": ["read"],
  "commands": [{ "name": "rows", "description": "rows", "command": "echo" }],
  "taskSources": [{ "id": "task-src-demo-runs", "title": "Demo Runs", "command": "rows", "color": "gold" }]
}"#,
        )
        .expect("写入 manifest");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        manager.install(source.to_str().expect("utf8 path")).expect("装带任务源的插件应通过校验");

        let list = manager.list_plugin_task_sources().expect("任务源投影应成功");
        assert_eq!(list.len(), 1, "实际 {list:?}");
        assert!(list[0].enabled, "defaultEnabled = true ⇒ 投影应带启用位");
        assert_eq!(list[0].task_sources.len(), 1);
        assert_eq!(list[0].task_sources[0].id, "task-src-demo-runs");
        assert_eq!(list[0].task_sources[0].command, "rows");

        manager.disable(&list[0].id).expect("禁用插件");
        let after = manager.list_plugin_task_sources().expect("禁用后仍可投影");
        assert_eq!(after.len(), 1, "启停过滤归消费侧，投影面不隐藏条目");
        assert!(!after[0].enabled, "禁用位必须透出，否则任务中心会挂上一个死源");

        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);
    }

    /// 反向：`taskSources[].command` 指到未声明的命令 ⇒ **装载期**就拒，插件装不上。
    #[test]
    fn install_rejects_task_source_referencing_unknown_command() {
        let label = "task-src-bad";
        let source = temp_dir(&format!("{label}-source"));
        let config_home = temp_dir(&format!("{label}-home"));
        std::fs::create_dir_all(&source).expect("创建源目录");
        let manifest_path = source.join(MANIFEST_RELATIVE_PATH);
        std::fs::create_dir_all(manifest_path.parent().expect("manifest 有父目录"))
            .expect("创建 manifest 目录");
        std::fs::write(
            &manifest_path,
            r#"{
  "name": "task-src-bad-demo",
  "version": "1.0.0",
  "description": "bad task source plugin",
  "defaultEnabled": true,
  "permissions": ["read"],
  "taskSources": [{ "id": "ghost-src", "title": "Ghost", "command": "never_declared" }]
}"#,
        )
        .expect("写入 manifest");

        let mut manager = PluginManager::new(isolated_config(&config_home));
        let error = manager
            .install(source.to_str().expect("utf8 path"))
            .expect_err("指向不存在命令的任务源必须被拒");
        let text = error.to_string();
        assert!(
            text.contains("never_declared") || text.contains("task source"),
            "错误要指得出缺陷所在，实际 {text}"
        );

        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);
    }
}

#[cfg(test)]
mod bundled_root_tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("测试：系统时间应晚于 UNIX EPOCH")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("plugin-bundled-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    #[test]
    fn prefers_the_resource_dir_when_it_contains_plugins_bundled() {
        let resource = temp_dir("hit");
        let expected = resource.join("plugins").join("bundled");
        std::fs::create_dir_all(&expected).expect("创建 plugins/bundled");

        assert_eq!(PluginManagerConfig::resolve_bundled_root(Some(&resource)), expected);

        let _ = std::fs::remove_dir_all(&resource);
    }

    #[test]
    fn falls_back_to_the_source_tree_when_the_resource_dir_has_no_plugins() {
        let resource = temp_dir("miss");
        assert_eq!(
            PluginManagerConfig::resolve_bundled_root(Some(&resource)),
            PluginManager::bundled_root(),
            "资源目录里没有 plugins/bundled 时必须退回源码树（开发期语义不变）"
        );
        let _ = std::fs::remove_dir_all(&resource);
    }

    #[test]
    fn falls_back_to_the_source_tree_without_any_resource_dir() {
        assert_eq!(PluginManagerConfig::resolve_bundled_root(None), PluginManager::bundled_root());
    }

    /// #189 反向钉桩：测试代码不得绕过 `test_support::isolated_config` 直接构造配置。
    ///
    /// 缺省 `bundled_root` 指向仓内 `bundled/`，而 `npm run plugin:bundle:dev` 会把 stock/opc 的
    /// dev 产物暂存到那里 ⇒ 一旦有测试直接 `PluginManagerConfig::new`，计数类断言就会随「有没有跑过
    /// 打包脚本」而涨落（本地绿、别人机器红）。
    ///
    /// 判据只看**代码行**（剥 `//` 起首行，避免把注释里的说明算成构造点）；唯一合法构造点是
    /// `test_support` 模块自身那一处，逐文件钉数量（多一处即红）。
    #[test]
    fn tests_do_not_construct_plugin_manager_config_directly() {
        fn code_lines_only(src: &str) -> String {
            src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
        }
        // 拼出靶串，避免本测试自身的源码被算成一次「构造点」（判据被自己的样本喂假）。
        let needle = concat!("PluginManagerConfig", "::new(");
        let lib_raw = include_str!("lib.rs");
        let lib = code_lines_only(lib_raw);
        let hooks = code_lines_only(include_str!("hooks.rs"));
        let manager = code_lines_only(include_str!("manager.rs"));
        // lib.rs 只允许**两个**例外：`discovers_builtin_and_bundled_plugins` 与
        // `default_bundled_root_loads_repo_bundles_as_installed_plugins` —— 它们的判据就是
        // 「缺省 bundled 根能装出 Bundled 插件」，钉到空目录会让断言恒假。两处都必须带 `#189 例外` 标注。
        assert_eq!(
            lib.matches(needle).count(),
            2,
            "lib.rs 只允许两个已标注例外；新用例一律走 `test_support::isolated_config`（#189）"
        );
        assert_eq!(
            lib_raw.matches("#189 例外").count(),
            2,
            "lib.rs 的两个例外必须各带一条 `#189 例外` 标注（数量漂移即说明有人加/删了例外）"
        );
        assert_eq!(
            hooks.matches(needle).count(),
            0,
            "hooks.rs 的测试必须走 `test_support::isolated_config`（#189）"
        );
        assert_eq!(
            manager.matches(needle).count(),
            1,
            "manager.rs 只允许 `test_support` 内出现唯一一处构造点（#189）"
        );
        // 正控：那唯一一处必须落在 `test_support` 模块内（否则说明隔离构造本身被换掉了）
        let module_at = manager.find("mod test_support").expect("test_support 模块必须存在");
        let construct_at = manager.find(needle).expect("必须保留唯一构造点");
        assert!(construct_at > module_at, "唯一构造点必须在 `test_support` 模块内");
    }

    // ==== #216 前置 ②：`taskCards[]` 声明面（surface / 命令 membership / 写权限闸）====

    fn task_card_action_entry(id: &str, command: &str, mutating: bool) -> RawPluginTaskCardAction {
        RawPluginTaskCardAction {
            id: id.to_string(),
            title: "动作".to_string(),
            command: command.to_string(),
            mutating,
        }
    }

    fn task_card_entry(id: &str, surface: Option<&str>, summary: &str) -> RawPluginTaskCard {
        RawPluginTaskCard {
            id: id.to_string(),
            title: "卡片".to_string(),
            surface: surface.map(str::to_string),
            summary_command: summary.to_string(),
            detail_schema: None,
            tone: None,
            order: 0,
            actions: Vec::new(),
        }
    }

    /// surface 词汇表是**封闭**的（leader 裁定 1：只开 `chat` / `task_card`）：
    /// 表外值与缺失一律报错，**不兜底** —— 兜底会让卡片落在界面上错误的位置。
    #[test]
    fn task_card_surface_vocabulary_is_closed() {
        let names: BTreeSet<&str> = ["rows"].into_iter().collect();
        let read_only = [PluginPermission::Read];

        // 两个合法值都通过，且原样落到枚举上（序列化字面量与清单写法逐字一致）。
        for (label, expected) in
            [("chat", PluginCardSurface::Chat), ("task_card", PluginCardSurface::TaskCard)]
        {
            let mut errors = Vec::new();
            let validated = build_manifest_task_cards(
                vec![task_card_entry("surface-ok", Some(label), "rows")],
                &names,
                &BTreeSet::new(),
                &read_only,
                &mut errors,
            );
            assert!(errors.is_empty(), "`{label}` 应通过，实际 {errors:?}");
            assert_eq!(validated[0].surface, expected);
        }
        // 反向控：serde 序列化值必须与 parse 令牌一致（两处口径同源，防漂移）。
        for surface in [PluginCardSurface::Chat, PluginCardSurface::TaskCard] {
            assert_eq!(PluginCardSurface::parse(surface.as_str()), Some(surface));
        }

        for bad in [None, Some("overview"), Some(""), Some("Chat")] {
            let mut errors = Vec::new();
            let validated = build_manifest_task_cards(
                vec![task_card_entry("surface-bad", bad, "rows")],
                &names,
                &BTreeSet::new(),
                &read_only,
                &mut errors,
            );
            assert!(validated.is_empty(), "`{bad:?}` 不应通过");
            assert_eq!(errors.len(), 1, "实际 {errors:?}");
            assert!(
                matches!(errors[0], PluginManifestValidationError::InvalidTaskCardSurface { .. }),
                "应报 InvalidTaskCardSurface，实际 {:?}",
                errors[0]
            );
            let text = errors[0].to_string();
            assert!(text.contains("surface-bad"), "文案要点出是哪张卡：{text}");
            assert!(text.contains("chat") && text.contains("task_card"), "文案要列出词表：{text}");
        }
    }

    /// 同插件重复卡 id ⇒ **fail-closed**（leader 裁定 3）：报错让整份清单装不上，
    /// 不是「先到先得静默丢一张」——后者在界面上表现为「少了一张卡」且无处可查。
    #[test]
    fn task_card_duplicate_id_is_rejected_not_first_wins() {
        let names: BTreeSet<&str> = ["rows"].into_iter().collect();
        let mut errors = Vec::new();

        let validated = build_manifest_task_cards(
            vec![
                task_card_entry("dup-card", Some("task_card"), "rows"),
                task_card_entry("dup-card", Some("chat"), "rows"),
                task_card_entry("Bad-Card", Some("task_card"), "rows"),
            ],
            &names,
            &BTreeSet::new(),
            &[PluginPermission::Read],
            &mut errors,
        );

        // 第一份留下，第二份（重名）与大写 id 都被拒；errors 非空 ⇒ 调用方整份清单失败。
        assert_eq!(validated.len(), 1, "实际 {validated:?}");
        assert_eq!(validated[0].id, "dup-card");
        assert_eq!(errors.len(), 2, "实际 {errors:?}");
        assert!(
            matches!(
                errors[0],
                PluginManifestValidationError::DuplicateEntry { kind: "task card", .. }
            ),
            "应报 DuplicateEntry(kind=task card)，实际 {:?}",
            errors[0]
        );
        assert!(
            matches!(errors[1], PluginManifestValidationError::InvalidTaskCardId { .. }),
            "应报 InvalidTaskCardId，实际 {:?}",
            errors[1]
        );
    }

    /// 三项 membership：`summaryCommand` ∈ commands[]、`detailSchema` ∈ 本插件 ui.json key 集、
    /// 动作 `command` ∈ commands[]（⇒ 宿主命令名在声明面上写不出来）。
    #[test]
    fn task_card_references_must_stay_inside_own_declarations() {
        let names: BTreeSet<&str> = ["rows", "act"].into_iter().collect();
        let ui_keys: BTreeSet<String> = ["order-form".to_string()].into_iter().collect();
        let mut errors = Vec::new();

        let ok = RawPluginTaskCard {
            id: "ok-card".to_string(),
            title: "合规卡".to_string(),
            surface: Some("task_card".to_string()),
            summary_command: "rows".to_string(),
            detail_schema: Some("order-form".to_string()),
            tone: Some("  ".to_string()),
            order: 7,
            actions: vec![task_card_action_entry("approve", "act", false)],
        };
        // 三处各错一条：summaryCommand 指错、detailSchema 不在 ui.json、动作指向宿主命令名。
        let bad_summary =
            task_card_entry("bad-summary", Some("task_card"), "approve_evolution_plan");
        let mut bad_schema = task_card_entry("bad-schema", Some("task_card"), "rows");
        bad_schema.detail_schema = Some("other-plugin-form".to_string());
        let mut bad_action = task_card_entry("bad-action", Some("task_card"), "rows");
        bad_action.actions =
            vec![task_card_action_entry("approve", "approve_evolution_plan", false)];

        let validated = build_manifest_task_cards(
            vec![ok, bad_summary, bad_schema, bad_action],
            &names,
            &ui_keys,
            &[PluginPermission::Read],
            &mut errors,
        );

        assert_eq!(validated.len(), 1, "实际 {validated:?}");
        assert_eq!(validated[0].tone, None, "空白 tone 按「未声明」处理");
        assert_eq!(validated[0].order, 7);
        assert_eq!(validated[0].actions.len(), 1);
        assert!(!validated[0].actions[0].mutating);

        assert_eq!(errors.len(), 3, "实际 {errors:?}");
        assert!(
            matches!(errors[0], PluginManifestValidationError::TaskCardCommandUnknown { .. }),
            "实际 {:?}",
            errors[0]
        );
        assert!(
            matches!(errors[1], PluginManifestValidationError::TaskCardSchemaKeyUnknown { .. }),
            "实际 {:?}",
            errors[1]
        );
        assert!(
            matches!(errors[2], PluginManifestValidationError::TaskCardActionCommandUnknown { .. }),
            "实际 {:?}",
            errors[2]
        );
        let text = errors[2].to_string();
        assert!(text.contains("bad-action"), "文案要点出是哪张卡：{text}");
        assert!(text.contains("approve_evolution_plan"), "文案要点出那条命令：{text}");
    }

    /// 写权限闸（leader 裁定 2）：**只对 `mutating: true` 生效**，只读动作不要求 `write`。
    /// 判定必须在后端（`PluginSummaryDto` 不带 permissions），且必须在装载期拦。
    #[test]
    fn task_card_mutating_action_requires_write_permission() {
        let names: BTreeSet<&str> = ["rows", "act"].into_iter().collect();
        let card_with = |mutating: bool| {
            let mut card = task_card_entry("gate-card", Some("task_card"), "rows");
            card.actions = vec![task_card_action_entry("do-it", "act", mutating)];
            card
        };

        // ① 只读动作 + 只读权限 ⇒ 通过（不许给只读动作加写门槛）。
        let mut errors = Vec::new();
        let validated = build_manifest_task_cards(
            vec![card_with(false)],
            &names,
            &BTreeSet::new(),
            &[PluginPermission::Read],
            &mut errors,
        );
        assert!(errors.is_empty(), "只读动作不应要求 write：{errors:?}");
        assert_eq!(validated.len(), 1);

        // ② mutating 动作 + 只读权限 ⇒ 拒，且文案点出卡与动作。
        let mut errors = Vec::new();
        let validated = build_manifest_task_cards(
            vec![card_with(true)],
            &names,
            &BTreeSet::new(),
            &[PluginPermission::Read],
            &mut errors,
        );
        assert!(validated.is_empty(), "越权动作不得进投影");
        assert!(
            matches!(
                errors[0],
                PluginManifestValidationError::TaskCardActionWritePermissionMissing { .. }
            ),
            "实际 {:?}",
            errors[0]
        );
        let text = errors[0].to_string();
        assert!(
            text.contains("gate-card") && text.contains("do-it"),
            "文案要定位到卡与动作：{text}"
        );
        assert!(text.contains("write"), "文案要点出缺的是 write：{text}");

        // ③ 三个「隐含写」的权限档位都必须放开闸（与 PluginPermission::implies_write 同源）。
        for permission in [
            PluginPermission::Write,
            PluginPermission::FileSystemWrite,
            PluginPermission::Execute,
            PluginPermission::SubprocessExecution,
        ] {
            let mut errors = Vec::new();
            let validated = build_manifest_task_cards(
                vec![card_with(true)],
                &names,
                &BTreeSet::new(),
                &[permission],
                &mut errors,
            );
            assert!(errors.is_empty(), "{permission:?} 应蕴含 write：{errors:?}");
            assert!(validated[0].actions[0].mutating);
        }
    }

    /// 端到端：装一个声明了 `taskCards` 的插件 ⇒ 投影出该卡；禁用后仍投影但带 `enabled = false`。
    ///
    /// 这条同时是 **serde 别名陷阱的回归钉**：清单里写的是 camelCase `taskCards`，
    /// `RawPluginManifest` 没有 `rename_all`，漏了 `rename` / `alias` 就会解出空表
    /// （表现是「清单里有、界面上没有、且零报错」）。
    #[test]
    fn list_plugin_task_cards_projects_installed_declarations() {
        let label = "task-card";
        let source = temp_dir(&format!("{label}-source"));
        let config_home = temp_dir(&format!("{label}-home"));
        std::fs::create_dir_all(&source).expect("创建源目录");
        let manifest_path = source.join(MANIFEST_RELATIVE_PATH);
        std::fs::create_dir_all(manifest_path.parent().expect("manifest 有父目录"))
            .expect("创建 manifest 目录");
        std::fs::write(
            &manifest_path,
            r#"{
  "name": "task-card-demo",
  "version": "1.0.0",
  "description": "task card projection plugin",
  "defaultEnabled": true,
  "permissions": ["read", "write"],
  "commands": [
    { "name": "rows", "description": "rows", "command": "echo" },
    { "name": "act", "description": "act", "command": "echo" }
  ],
  "taskCards": [{
    "id": "demo-card",
    "title": "Demo Card",
    "surface": "task_card",
    "summaryCommand": "rows",
    "tone": "purple",
    "order": 50,
    "actions": [{ "id": "do-it", "title": "执行", "command": "act", "mutating": true }]
  }]
}"#,
        )
        .expect("写入 manifest");

        // 隔离构造：本模块在 `mod tests` 之后，走全路径引用唯一的构造点（不新增第二份配置）。
        let mut manager =
            PluginManager::new(crate::manager::test_support::isolated_config(&config_home));
        manager.install(source.to_str().expect("utf8 path")).expect("装带卡片的插件应通过校验");

        let list = manager.list_plugin_task_cards().expect("卡片投影应成功");
        assert_eq!(list.len(), 1, "实际 {list:?}");
        assert!(list[0].enabled, "defaultEnabled = true ⇒ 投影应带启用位");
        assert_eq!(list[0].task_cards.len(), 1);
        let card = &list[0].task_cards[0];
        assert_eq!(card.id, "demo-card");
        assert_eq!(card.surface, PluginCardSurface::TaskCard);
        assert_eq!(card.summary_command, "rows");
        assert_eq!(card.tone.as_deref(), Some("purple"));
        assert_eq!(card.order, 50);
        assert_eq!(card.actions.len(), 1);
        assert_eq!(card.actions[0].command, "act");
        assert!(card.actions[0].mutating);

        manager.disable(&list[0].id).expect("禁用插件");
        let after = manager.list_plugin_task_cards().expect("禁用后仍可投影");
        assert_eq!(after.len(), 1, "启停过滤归消费侧，投影面不隐藏条目");
        assert!(!after[0].enabled, "禁用位必须透出，否则前端会把死卡挂上去");

        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);
    }

    /// 反向：`taskCards[].summaryCommand` 指到未声明的命令 ⇒ **装载期**就拒，插件装不上。
    #[test]
    fn install_rejects_task_card_referencing_unknown_command() {
        let label = "task-card-bad";
        let source = temp_dir(&format!("{label}-source"));
        let config_home = temp_dir(&format!("{label}-home"));
        std::fs::create_dir_all(&source).expect("创建源目录");
        let manifest_path = source.join(MANIFEST_RELATIVE_PATH);
        std::fs::create_dir_all(manifest_path.parent().expect("manifest 有父目录"))
            .expect("创建 manifest 目录");
        std::fs::write(
            &manifest_path,
            r#"{
  "name": "task-card-bad-demo",
  "version": "1.0.0",
  "description": "task card bad reference plugin",
  "defaultEnabled": true,
  "permissions": ["read"],
  "commands": [{ "name": "rows", "description": "rows", "command": "echo" }],
  "taskCards": [{ "id": "ghost-card", "title": "Ghost", "surface": "chat", "summaryCommand": "not_declared" }]
}"#,
        )
        .expect("写入 manifest");

        let mut manager =
            PluginManager::new(crate::manager::test_support::isolated_config(&config_home));
        let text = manager
            .install(source.to_str().expect("utf8 path"))
            .expect_err("卡片引用未声明的命令必须被拒")
            .to_string();
        assert!(text.contains("ghost-card"), "文案要指出是哪张卡：{text}");
        assert!(text.contains("not_declared"), "文案要指出那条命令：{text}");
        drop(manager);
        let _ = std::fs::remove_dir_all(&config_home);
        let _ = std::fs::remove_dir_all(&source);
    }
}
