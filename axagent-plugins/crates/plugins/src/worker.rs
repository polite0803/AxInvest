// SPDX-License-Identifier: AGPL-3.0-only

//! B 层动态插件载入器：长驻 worker 子进程 + L1 事件桥 + L2 跨插件调用。
//!
//! ## 形态
//!
//! 插件被编译为一个**独立可执行文件**，由宿主 spawn 后**长驻**，双方以 stdin/stdout 上的
//! **长度前缀 JSON 帧**通信（协议见 `axagent-plugin-proto`）。与既有 hook / tool 的
//! 「per-call spawn」不同：进程只创建一次，之后每次调用仅是一次帧往返
//! （实测长驻 18.9 µs/次 vs per-call 72–195 ms，见 `PLAN-everything-is-plugin.md` §11）。
//!
//! ## 与能力注册表的关系（平权）
//!
//! worker 声明的每条能力，由本模块构造一个**远程门面**（facade：自身实现该接缝的 trait，
//! 内部把调用编成 JSON 帧转发），再经**既有**的 `CapabilityRegistry::register_*` 入口入表。
//! ⇒ 内置实现与外部插件在注册表里**是同一个类型**（`Arc<dyn Xxx>`），消费方
//! （`get_sandbox()` / `get_business_rule()` / …）**零改动**即可取到并使用。
//!
//! ## 卸载 = 可逆回滚
//!
//! 载入时收集全部 [`EffectHandle`]（能力 + 事件订阅），卸载时**逆序** `undo()`，
//! 最后向 worker 发 `shutdown` 帧；超时未退出则强杀（防孤儿进程）。
//! **逆序是必须的**：`EffectHandle::undo` 按注册时的槽位下标移除元素，乱序撤销会误伤
//! 其它效果（这是 `reversible_effect.rs` 既有语义，LIFO 即安全）。
//!
//! ## 孤儿进程防线（PLAN §13.3）
//!
//! ① worker 侧：插件用 `axagent_plugin_proto::serve` 实现 main 时，**stdin 读到 EOF 即自退**
//!    ⇒ 宿主崩溃 / 被杀后 worker 自行终止（主防线）；
//! ② 宿主侧：[`LoadedPlugin::unload`] 的 shutdown 帧 + 超时强杀；
//! ③ [`Drop`] 兜底强杀 —— 即使调用方忘了 `unload`，也不会留下常驻子进程。
//!
//! ## 双向（P4）：读线程 + `request_id` 配对表
//!
//! P4 起插件也能**主动**向宿主发请求（`ops::EMIT` / `ops::CALL_SEAM`，见
//! `PLAN-everything-is-plugin.md` §15.2/§15.4）。于是同一条管道上同时挂着两个方向，
//! 「一帧进一帧出」的串行化模型不再成立，改为：
//!
//! - 一把**专用读线程**消费 stdout，按 `request_id` 把响应投递到 `pending` 表里的等待者；
//! - 收到**请求**帧（worker 主动发起）就交给 [`HostPeer`] 落地并回帧（回填 `request_id`）；
//! - 一把**专用写线程**独占 stdin（任务 #56），调用方只**投递帧 + 带预算等写回执**，
//!   等响应期间**不持任何锁** —— 否则会与读线程互等。
//!
//! ## L2 接缝调用与三条死锁缓解（PLAN §15.4 / §15.5）
//!
//! `call_seam` 让插件 A 同步调用另一条接缝的 provider（可能又是插件 B）。为此：
//!
//! ① **`call_chain` 环检测**：链元素是插件 ID。**由目标端自检**
//!    （[`WorkerInvoker::invoke_with_chain`] 在写帧前判断 `call_chain` 是否已含自己）——
//!    宿主经注册表只拿得到 `Arc<dyn Xxx>` 门面，认不出背后是哪个插件，故判定只能落在目标端。
//!    命中即**拒绝、不进入等待**，因此环不会真的死锁。
//! ② **深度上限** [`MAX_SEAM_CALL_DEPTH`]：挡住「无环但极深」的长链（每层都要占一份栈 +
//!    一次子进程等待）。宿主侧对**进程内** provider 也要判（它们的 `invoke_with_chain`
//!    走默认实现，不做自检）。
//! ③ **跨插件优先用 L1 事件**：`emit` 不等待派发结果，见 [`HostPeer::handle_emit`] 的注释
//!    （同步等待会把「事件回投给发起者」变成自锁）。
//! ④ **接缝等待预算**（任务 #53 修 D1）：`invoke` / `invoke_with_chain` / UI action
//!    回流共用同一传输层入口，写帧后到点收不到回帧 ⇒ 放弃等待并报超时，连接随即标脏
//!    （拒后续接缝调用、停机走急停）；预算默认 [`DEFAULT_SEAM_TIMEOUT_MS`]，长跑型接缝
//!    （`run_turn`）按 plugin-proto 的 `SEAM_OP_TIMEOUT_OVERRIDES` 逐接缝覆盖，连接级可用
//!    [`PluginWorkerConfig::with_seam_wait_timeout`] 显式改默认。
//! ⑤ **握手等待预算**（任务 #63 补 D1 边角）：`ops::DESCRIBE` 不经接缝公共入口，
//!    故④对它不生效；[`WorkerInvoker::handshake`] 用独立的
//!    [`DEFAULT_HANDSHAKE_TIMEOUT_MS`]（[`PluginWorkerConfig::with_handshake_timeout`] 可覆盖），
//!    超时同样置「连接可疑」闸 —— 不是为了拒后续接缝调用（这条连接根本不会交付出去），
//!    而是为了让 [`LoadedPlugin::load`] 失败路径上的 `shutdown()` 跳过「等 shutdown 回执」，
//!    否则「不回 describe 的 worker」会把装载从「挂握手」改成「挂停机」。
//! ⑥ **嵌套调用的计时挂起**（任务 #73 修 M-3，D1 的另一半）：入站请求（`emit` /
//!    `call_seam`）改由**独立任务**处理，读线程不再被宿主侧那一跳占住；同时出向等待预算
//!    **只在对端不被宿主占用时消耗**。两者合起来才让「④ 到点 = 对端装死」这条推论成立 ——
//!    修复前只要插件 handler 里嵌套一次耗时 ≥ 外层预算的宿主侧调用（SDK 文档亲自示范的
//!    `peer.call_seam` 形态），外层就**必然**被误杀并把健康连接打到终局闸（审查稿 A4.3）。
//!    ⚠ 代价如实声明：外层 15 s 这道闸在嵌套期间不生效，若宿主替插件跑的那一跳自己卡死，
//!    外层跟着没有上界 —— 那是入站服务侧**既存**的无界（审查稿 A4.2：宿主服务 `call_seam`
//!    时没有任何等待上限），本批没有给它加闸，登记在交回清单。
//! ⑦ **写侧解阻**（任务 #56，D1 的另一半）：stdin 由**专用写线程**独占，调用方只投递帧并
//!    **带预算**等写回执 ⇒ 对端僵死不读 stdin 且管道满时，阻塞落在写线程上，调用方在自己的
//!    预算内拿到三态回执之一（`Written` / `Failed` / `NoAck`）。只有 `Failed` 且确认
//!    **一个字节都没写出** 才可报 `NotDelivered`（零副作用 ⇒ 允许上层回退）；
//!    `NoAck` 与「已写出部分字节」一律判连接不可用（毒化 + 杀进程关闭管道以解阻写者）
//!    并报 `Transport`。`shutdown()` / `Drop` 的关管道与回收也全部有界。

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axagent_harness::platform_config::PlatformConfig;
use axagent_harness::types::{
    ChatRequest, ChatResponse, ChatStreamChunk, EmbedRequest, EmbedResponse, Model,
};
use axagent_harness::workflow_types::{NodeKind, Variable, WorkflowTemplateData};
use axagent_harness::{
    AgentTurnRequest, AgentTurnResult, AgentTurnRunner, AxAgentError, BusinessRuleEvaluator,
    CapabilityRegistry, ContextContributor, ContextRequest, DispatchMode, DispatchResult,
    DomainEvent, EffectHandle, EventCategory, EventMatcher, EventSubscriber, EvolutionPopulation,
    EvolutionStats, InvariantViolation, MessagePlatformAdapter, ModelVisibleContent,
    NodeExecutionSnapshot, PlatformMessageCallback, ProviderAdapter, ProviderRequestContext,
    Reflection, RuleEvaluationOutcome, SandboxValidationResult, SeamInvoker, SessionLogInvariant,
    SubscriberVerdict, Tool, ToolCategory, ToolContext, ToolError, ToolResult, ToolSetProvider,
    WebhookDispatch, WebhookEvent, WorkflowEvolver, WorkflowExecutionRecord, WorkflowGenome,
    WorkflowGenomeLoader, WorkflowLlmMutator, WorkflowModification, WorkflowOptimizer,
    WorkflowPattern, WorkflowReflector, WorkflowRunStatus, WorkflowSandbox, WorkflowSuggestion,
    get_capability_registry,
};
use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFrame, CommandOutcome, CommandReply,
    DEFAULT_HANDSHAKE_TIMEOUT_MS, DEFAULT_SEAM_TIMEOUT_MS, FrameRequest, FrameResponse, Inbound,
    MAX_SEAM_CALL_DEPTH, PluginDeclaration, ResponseKind, error_codes, ops, read_inbound,
    seam_timeout_ms_for_op, write_json_frame,
};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::sandbox::{SandboxConfig, apply_env_to_command, check_subprocess_permission};
use crate::types::PluginPermission;

/// 已支持远程化的接缝 ID 清单（与 PLAN §5.4 的 op 映射表一致）。
///
/// 三条**动态 ID** 接缝（多实例，后缀由插件声明）：`platform.adapter.{平台名}`
/// （见 `CapabilityRegistry::register_platform_adapter`）、`model.provider.{类型名}`
/// （见 `register_model_provider`）、`system.prompt.{段名}`（见
/// `register_system_prompt_section`）。此处登记前缀，判定请用 [`is_supported_remote_seam`]，
/// 后缀取回用各自的 `*_from_seam` 助手。
///
/// `event.dispatch` **不在列**且不应加入：它本身是事件总线，插件对它只能是
/// **订阅方**（声明里的 `subscribe`），不是提供方（PLAN §5.4 / §15.3）。
///
/// `session.store` 亦不在列：会话数据主权在宿主，远程化判据未定（`PLAN-plugin-gap-closure.md` §2）。
pub const SUPPORTED_REMOTE_SEAMS: &[&str] = &[
    "agent.loop",
    "workflow.sandbox",
    "workflow.reflector",
    "workflow.evolver",
    "workflow.optimizer",
    "workflow.business_rule",
    "message.callback",
    "webhook.dispatch",
    "session.log.invariant",
    "platform.adapter",
    "tool.set",
    "model.provider",
    "system.prompt",
];

/// 宿主内部注入通道：注册表里**有 provider**，但不允许插件经 `call_seam` 调用。
///
/// 与 [`SUPPORTED_REMOTE_SEAMS`] 是两个方向的两张表：那张是「插件提供给宿主」的清单；
/// 这张是「宿主自己用、不对外」。`session.store` 是宿主工具取会话状态实例的注入通道
/// （`capability_load` / `save_as_workflow` 经 `CapabilityRegistry::get_session_state_store`
/// 直取），数据主权判据未定（PLAN §64.3）⇒ 保持注册，但插件调用必须在**取 provider 之前**
/// 拒绝 —— 否则 `list_by_prefix` 等于把全量会话状态读走。
pub const HOST_INTERNAL_SEAMS: &[&str] = &["session.store"];

/// UI action 回流的 op（宿主 → 插件方向，PLAN §10.5-2）。
///
/// **不在** [`SUPPORTED_REMOTE_SEAMS`] 里：那份清单是「插件**提供**的接缝」，
/// 本 op 是宿主**反向**回调插件自己声明的 UI 动作，方向相反、不参与接缝注册。
pub const OP_UI_ACTION: &str = axagent_plugin_proto::ops::UI_ACTION;

/// 接缝 ID 是否已支持远程化 —— 兼容 `platform.adapter.{平台名}` 这类**动态** ID。
///
/// 判定是「恰好相等」或「以 `{登记项}.` 为前缀」，故 `platform.adapternope` 不会被误判。
pub fn is_supported_remote_seam(seam: &str) -> bool {
    SUPPORTED_REMOTE_SEAMS.iter().any(|supported| {
        seam == *supported || seam.strip_prefix(supported).is_some_and(|rest| rest.starts_with('.'))
    })
}

/// `platform.adapter.{平台名}` 的动态接缝 ID 取回平台名。
///
/// 非该前缀返回 `None`。
fn platform_name_from_seam(seam: &str) -> Option<&str> {
    seam.strip_prefix("platform.adapter.").filter(|name| !name.is_empty())
}

/// `model.provider.{类型名}` 的动态接缝 ID 取回 provider 类型名。
///
/// 非该前缀返回 `None`。
fn provider_type_from_seam(seam: &str) -> Option<&str> {
    seam.strip_prefix("model.provider.").filter(|name| !name.is_empty())
}

/// `system.prompt.{段名}` 的动态接缝 ID 取回段名。
///
/// 非该前缀返回 `None`。
fn prompt_section_from_seam(seam: &str) -> Option<&str> {
    seam.strip_prefix("system.prompt.").filter(|name| !name.is_empty())
}

/// worker 载入 / 调用期的错误。
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// 插件未声明 `subprocess_execution` 权限（沙箱准入检查，见 PLAN §3.3）。
    #[error("插件 `{plugin_id}` 未声明 `subprocess_execution` 权限，禁止载入 worker 子进程")]
    SubprocessDenied { plugin_id: String },
    /// 无法启动 worker 进程。
    #[error("无法启动插件 `{plugin_id}` 的 worker 进程：{reason}")]
    Spawn { plugin_id: String, reason: String },
    /// worker 进程未提供 stdin/stdout 管道。
    #[error("插件 `{plugin_id}` 的 worker 进程未提供 stdin/stdout 管道")]
    MissingPipe { plugin_id: String },
    /// 握手（取声明 / 校验协议版本）失败。
    ///
    /// 三类原因都走本变体：进程没起来之后的管道错、worker 回了错信封、以及**握手帧超时**
    /// （任务 #63）。超时不另立错误码也不新增变体 —— [`Self::Timeout`] 的 Display 已带
    /// `op` 与预算毫秒数，被本变体的 `reason` 原样嵌入后即「握手失败 + describe + 上限」，
    /// 足以定位；新增码要同步 11 语言译文（AGENTS.md 禁区 2 / 后端错误码规范），成本不对。
    ///
    /// **处置指引与「连接可疑」闸同源**（任务 #73 M-2）：握手超时同样会置
    /// `WorkerInner::seam_suspect`，但那条连接**不会被交付**（`load` 返回 `Err`、随实例丢弃），
    /// 因此用户永远看不到闸的拒次文案，只能看到本条。若只给闸文案加「禁用再启用」，
    /// 握手这条置脏路径就少了同一句处置指引 —— 两条路径的**用户可见出口**都要说到，
    /// 否则「已经告诉你怎么办」只在半边成立。码位保持 #63 的选择（**不加码**），
    /// 与接缝/命令路径的 `SEAM_PROVIDER_UNAVAILABLE` 不是同一个可观测错误，不构成同路径双码。
    #[error("插件 `{plugin_id}` 握手失败：{reason}；请在设置中禁用再启用该插件以重试装载")]
    Handshake { plugin_id: String, reason: String },
    /// 插件声明的协议版本 / 能力项非法。
    #[error("插件 `{plugin_id}` 的声明无效：{reason}")]
    Declaration { plugin_id: String, reason: String },
    /// 插件声明了本轮尚不支持远程化的接缝。
    #[error("插件 `{plugin_id}` 声明了不支持的接缝 `{seam}`")]
    UnsupportedSeam { plugin_id: String, seam: String },
    /// 接缝注册被拒（如重复注册）。
    #[error("插件 `{plugin_id}` 的接缝 `{seam}` 注册失败：{reason}")]
    Register { plugin_id: String, seam: String, reason: String },
    /// 通道读写 / 对端返回错误。
    ///
    /// `code` 是**结构化错误码**（任务 #73 S-1）：值即对端错误信封里的 `code` 字段
    /// （[`error_codes::PLUGIN_UNKNOWN_OP`] 一类），由 [`Self::transport_error_with_code`]
    /// 在成形处填入；纯 I/O 断裂、形状非法这类没有协议码的失败为 `None`。
    /// ⚠ 消费方（`manager.rs` 的回退判据）**读这个字段，不读 `reason` 文本** ——
    /// 旧判据按 `reason` 首个 `:` 取段，任何人改动信封成形的文案都会**静默关掉回退**
    /// （审查稿 A2.3 / PLAN §26.2 第 3 条点名的正是这一行）。
    #[error("与插件 `{plugin_id}` 的通道错误：{reason}")]
    Transport { plugin_id: String, reason: String, code: Option<String> },
    /// 请求帧**从未送达** worker（任务 #73 M-1）：进程已退出、管道已断，或写帧本身失败。
    ///
    /// 它与 [`Self::Transport`] 的全部区别就是「**零副作用**」这一件事 —— 对端根本没收到
    /// 这一帧，因此不可能已经执行过任何东西，重发（命名命令回退到 spawn-on-call 通道）
    /// 不构成二次执行。反过来，帧一旦写出去就不再是本变体：此后连接才断、回执才坏，
    /// 命令**可能已执行完**，一律按 `Transport` 处理且**不回退**。
    #[error("插件 `{plugin_id}` 的 worker 未收到 `{op}` 帧：{reason}")]
    NotDelivered { plugin_id: String, op: String, reason: String },
    /// 调用超过宿主强制的等待上限，等待已被放弃（对端随后回来的响应帧按「无等待者」
    /// 由读线程丢弃并留警告）。三个使用者各带自己的预算：命令帧（任务 #49，
    /// 帧 `timeout_ms` + 收帧宽限）、接缝公共入口（任务 #53，`seam_wait_timeout` 与
    /// 逐接缝覆盖表）、握手 `describe`（任务 #63，`handshake_timeout`）。
    #[error("插件 `{plugin_id}` 的 `{op}` 超过宿主等待上限 {timeout_ms} ms，调用已放弃")]
    Timeout { plugin_id: String, op: String, timeout_ms: u64 },
}

impl WorkerError {
    /// 协议错误码（若该次失败由对端的错误信封 / Fault 成形而来）。
    ///
    /// 任务 #73 S-1：回退判据的唯一输入。文本匹配（旧写法取 `reason` 首段）已删除，
    /// 不留双判据 —— 文案怎么改都不会影响回退与否，码位由构造点负责。
    pub fn error_code(&self) -> Option<&str> {
        match self {
            Self::Transport { code, .. } => code.as_deref(),
            _ => None,
        }
    }
}

/// worker 载入配置。
pub struct PluginWorkerConfig {
    /// 插件 ID（写入 `AXAGENT_PLUGIN_ID` 环境变量，供插件自识别）。
    pub plugin_id: String,
    /// 插件可执行文件路径。
    pub program: PathBuf,
    /// 传给插件的命令行参数。
    pub args: Vec<String>,
    /// 进程工作目录（`None` = 继承宿主当前目录）。
    pub working_dir: Option<PathBuf>,
    /// 沙箱配置；其中 **ENV 白名单**是唯一在 OS 层生效的项（`env_clear` + 白名单回填）。
    pub sandbox: SandboxConfig,
    /// shutdown 帧后等待 worker 自行退出的上限，超时则强杀。
    pub shutdown_timeout: Duration,
    /// 接缝调用（宿主经 [`SeamInvoker`] 门面 / worker 发起的 `call_seam` 分派）等待
    /// worker 回帧的预算上限（任务 #53 修 D1）。
    ///
    /// 缺省取协议常量 [`DEFAULT_SEAM_TIMEOUT_MS`]；到点未收到配对响应 ⇒ 放弃等待、
    /// 报 [`WorkerError::Timeout`]，并把该连接标为「可疑」（`WorkerInner` 的 `seam_suspect`）。
    /// 进表 op（plugin-proto `SEAM_OP_TIMEOUT_OVERRIDES`，如 `run_turn`）用表值，本字段只管未进表 op。
    /// 命令通道与 spawn-on-call 路径**不读本字段**（#49 语义不变）。
    pub seam_wait_timeout: Duration,
    /// 握手帧 `describe` 等待 worker 回帧的预算上限（任务 #63 补 D1 边角）。
    ///
    /// 缺省取协议常量 [`DEFAULT_HANDSHAKE_TIMEOUT_MS`]。**与 [`Self::seam_wait_timeout`]
    /// 分开设**：握手是装载前置（spawn 之后的第一帧，不经接缝公共入口），接缝预算是用户
    /// 发起的调用；两者取值口径与失败代价都不同（详见 plugin-proto 该常量的文档）。
    /// 超时 ⇒ [`WorkerError::Handshake`]（reason 内嵌 [`WorkerError::Timeout`] 的文本），
    /// 并置「连接可疑」闸，使 [`LoadedPlugin::load`] 失败路径的 `shutdown()` 走急停。
    pub handshake_timeout: Duration,
    /// 插件清单声明的权限集（`PluginManifest::permissions` 的克隆）。
    ///
    /// 接缝分派只用它做**写 op 准入**（见 [`required_permission_for`]）；空表 = 未声明任何
    /// 权限，所有登记为需要 `write` 的 op 一律拒绝。本批只读：不随运行期变动。
    pub permissions: Vec<PluginPermission>,
}

impl PluginWorkerConfig {
    /// 用插件 ID + 可执行文件路径构造配置，其余字段取保守默认值。
    pub fn new(plugin_id: impl Into<String>, program: impl Into<PathBuf>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            program: program.into(),
            args: Vec::new(),
            working_dir: None,
            sandbox: SandboxConfig::default(),
            shutdown_timeout: Duration::from_secs(5),
            seam_wait_timeout: Duration::from_millis(DEFAULT_SEAM_TIMEOUT_MS),
            handshake_timeout: Duration::from_millis(DEFAULT_HANDSHAKE_TIMEOUT_MS),
            permissions: Vec::new(),
        }
    }

    /// 指定沙箱配置（决定 ENV 白名单与 `allow_subprocess` 准入）。
    pub fn with_sandbox(mut self, sandbox: SandboxConfig) -> Self {
        self.sandbox = sandbox;
        self
    }

    /// 指定 worker 进程工作目录。
    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// 指定接缝调用等待预算（缺省为 [`DEFAULT_SEAM_TIMEOUT_MS`]）。
    ///
    /// 面向「慢接缝显式覆盖」的接线口（任务 #53 交接报告待办）：本轮**调用方一律用缺省值**，
    /// 但预算有明确覆盖去处，就不需要散落的 `exchange(.., None)`。
    pub fn with_seam_wait_timeout(mut self, wait: Duration) -> Self {
        self.seam_wait_timeout = wait;
        self
    }

    /// 指定握手 `describe` 帧的等待预算（缺省为 [`DEFAULT_HANDSHAKE_TIMEOUT_MS`]）。
    ///
    /// 面向「慢启动插件显式放宽」的接线口；与 [`Self::with_seam_wait_timeout`] 是两个
    /// 独立旋钮（握手预算不会被接缝预算改写，反之亦然）。
    pub fn with_handshake_timeout(mut self, wait: Duration) -> Self {
        self.handshake_timeout = wait;
        self
    }

    /// 指定插件已声明的权限集（缺省空表 ⇒ 所有写 op 一律拒绝）。
    ///
    /// 通常由 [`crate::manager::PluginManager`] 从 `PluginManifest::permissions` 原样传入；
    /// 权限是**准入前提**，故只在装载时确定、运行期不变。
    pub fn with_permissions(mut self, permissions: Vec<PluginPermission>) -> Self {
        self.permissions = permissions;
        self
    }
}

// ───────────────────────── 调用链上下文（thread-local） ─────────────────────────

thread_local! {
    /// 当前线程正在处理的调用链（元素 = 插件 ID，按发起顺序）。
    ///
    /// 为什么还要 thread-local，明明链已经是帧上的参数？—— 因为链必须穿过注册表的
    /// **类型擦除门面**（`Arc<dyn AgentTurnRunner>` → `Arc<dyn SeamInvoker>`），
    /// 那里没有能挂参数的地方；而进程内 provider（如内置实现）转发时也读不到帧。
    /// 于是：门面在**跨越线程边界前**（`spawn_blocking`）取一份快照，再作为参数传给
    /// `invoke_with_chain`；thread-local 只负责在「同步链路内部」传递。
    ///
    /// 局限（如实声明）：门面被**非受控线程**直接调用时链为空，该次调用被视为新的顶层调用。
    static CALL_CHAIN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// 取当前线程的调用链快照。
///
/// **跨线程前必须调用**（`spawn_blocking` 之后 thread-local 就是另一条链了）。
fn current_call_chain() -> Vec<String> {
    CALL_CHAIN.with(|chain| chain.borrow().clone())
}

/// 在「调用链 = `chain`」的上下文里执行 `f`，返回后恢复原链。
fn with_call_chain<T>(chain: Vec<String>, f: impl FnOnce() -> T) -> T {
    let previous = CALL_CHAIN.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), chain));
    let out = f();
    CALL_CHAIN.with(|slot| *slot.borrow_mut() = previous);
    out
}

// ───────────────────────── 宿主侧入站处理器 ─────────────────────────

/// 宿主侧处理 worker **主动发起**的请求（`emit` / `call_seam`）。
///
/// 它把「插件能做什么」的判据集中在一处：声明校验（`calls`）、自调检查（自己的接缝）、
/// 深度上限、接缝分派。全部拒绝都在**写帧前**完成，故不存在「先挂起再判错」的路径。
struct HostPeer {
    /// 发起请求的插件 ID（用于错误信息与事件 `source` 兜底）。
    plugin_id: String,
    /// 本插件声明**提供**的接缝（自调检查：目标接缝若在此列 ⇒ 环）。
    declared_capabilities: Vec<String>,
    /// 本插件声明要**调用**的接缝（PLAN §5.2 `calls`）：未声明即拒。
    declared_calls: Vec<String>,
    /// 能力注册表（取接缝 provider）。持有 `Arc` 而非 `&'static`，便于测试注入局部表。
    registry: Arc<CapabilityRegistry>,
    /// 派发 **async** 接缝所需的 runtime handle（在载入时捕获）。
    ///
    /// 为 `None` 时：async 接缝的 `call_seam`/`emit` 返回明确错误，而不是 panic 或静默失败。
    runtime: Option<tokio::runtime::Handle>,
    /// 本插件清单声明的权限集（构造时从 [`WorkerInner::permissions`] 克隆，运行期不变）。
    ///
    /// 用途只有一个：[`Self::dispatch_seam`] 的写 op 准入闸。与 `declared_calls` 是两道
    /// 独立的闸 —— 前者管「能调哪些接缝」，本字段管「能写哪些接缝」。
    permissions: Vec<PluginPermission>,
}

/// 接缝写 op → 所需权限的登记表。
///
/// **本批登记 `jobs.store` 的四个写 op**（`add` / `remove` / `set_status` / `update`）；
/// 其余接缝或 op 一律 `None`（不设权限闸）。其中 `add` / `remove` / `set_status` 已有分派
/// 分支，`update` 仅登记、尚无实现（会落到「未知 op」的 `SEAM_CALL_UNSUPPORTED`）。
///
/// 同时登记 **`workflow.run` 的两个写 op**（`start` / `cancel`）：发起与取消执行都会改变
/// 宿主侧运行态（占用并发额度、触发引擎 `cancel`），故要求 `write`；`status` /
/// `list_active` / `list_recent` 只读，不设闸。
///
/// 再登记 **`trading.engine` 的唯一写 op**（`execute_trade`）：它写用户交易账本。其余五个
/// op（`get_positions` / `get_trades` / `validate_trade` / `compare_trade` /
/// `trading_enabled`）全为只读，不设闸。⚠ `execute_trade` 被放行后返回的
/// `accepted = false`（开关关闭 / 校验不过 / 重复提交）是**正常业务结果**（走 payload 的
/// `reason`，不借用错误码）——权限闸只管「能不能调」，不管「单子接没接」。
///
/// 又登记 **`stock.monitor` 的四个写 op**（`set_t0_config` / `upsert_alert` /
/// `remove_alert` / `set_aggregator_config`）：告警落库、T+0 全局开关、聚合器阈值都改宿主
/// 监控运行态，其中后两个**无 owner 隔离**、影响所有插件与前端；其余五个 op（`status` /
/// `list_configs` / `get_t0_config` / `aggregator_config` / `aggregator_buffer`）只读，不设闸。
///
/// 最后登记 **`quote.watcher` 的两个写 op**（`watch` / `unwatch`）：推送集合是所有插件与
/// 前端共享的一格，`watch` 直接放大对 vendor 的取数请求数；`list_watched` 只读，不设闸。
///
/// 本批只新增 **`knowledge.vector` 的唯一写 op**（`index_text`）一条：它会在宿主侧触发 chunk
/// 切分 + embedding 调用（按 token 计费，而 PLAN §56.2-3 明示「速率闸首批不做」⇒ `write` 一档
/// 权限是当前唯一防线）。其余三个 op（`search` / `keyword_search` / `get_collection_info`）
/// 只读，不设闸。
///
/// `trajectory.store` **一条条目都不加**：首批四个 op（`list` / `get` / `query` / `count`）
/// 全是读面（PLAN §56.1-Q4 裁定 `append` 不给 —— 写入口会污染进化门槛读的分母）⇒ 一律落
/// 下面的 `_ => None`。将来若开写 op，必须**同批**在这里补登记，不许靠分派分支自查权限。
///
/// 本批登记 **`opc.store` 的写 op**：blog 三件（`publish_blog_post` / `delete_blog_post` /
/// `create_blog_post`）+ #36 批 2 的四条（`transition_invoice` / `update_content_asset` /
/// `delete_content_asset` / `cancel_publish_schedule`）。发布把一行博客置为已发布、删除是
/// 不可逆的数据面动作（受保护 id 走三态 `Protected` 兜底）、创建把调用方给的正文写进库
/// （正文按契约的字节上限设闸）；批 2 的四条同样改库（发票状态 / 内容资产列 / 两处硬删）
/// —— 全都要 `write`。同接缝的九个读 op **必须仍是 `None`** —— 读缝加闸会把只读插件
/// （`opc_list_*` 那几条命令）无端拒掉，反向锁与正锁都在 `opc_store_*` 那几条用例里。
///
/// 纯函数（无 `self`、无全局状态），便于单测直接枚举判定。
fn required_permission_for(seam: &str, op: &str) -> Option<PluginPermission> {
    match (seam, op) {
        ("jobs.store", "add" | "remove" | "set_status" | "update") => Some(PluginPermission::Write),
        ("workflow.run", "start" | "cancel") => Some(PluginPermission::Write),
        ("trading.engine", "execute_trade") => Some(PluginPermission::Write),
        (
            "stock.monitor",
            "set_t0_config" | "upsert_alert" | "remove_alert" | "set_aggregator_config",
        ) => Some(PluginPermission::Write),
        ("quote.watcher", "watch" | "unwatch") => Some(PluginPermission::Write),
        ("knowledge.vector", "index_text") => Some(PluginPermission::Write),
        // `stock.analysis` 的唯一写 op：批量 upsert 改 `market_mainlines` 表（插入 / 更新 /
        // 可选归档当日未提及的主线，归档后要再 upsert 才回到 active ⇒ 不可逆）。
        // 另两个 op（`industry_constituents` / `stock_factor`）是纯读面 ⇒ **必须仍是
        // `None`**（给读缝加闸会把只读插件无端拒掉，正反向锁在 `stock_analysis_*` 用例）。
        ("stock.analysis", "batch_upsert_mainlines") => Some(PluginPermission::Write),
        // F1 余三条（#35）：`sweep_daily_snapshots` 是写 op —— 触发每日快照采集
        // （网络 + `astock_daily_snapshot` 落表）。另两条 `list_financial_snapshots` /
        // `refresh_trading_calendar` 是读面 ⇒ **必须仍是 `None`**（给读缝加闸会把只读
        // 插件无端拒掉）。
        ("stock.analysis", "sweep_daily_snapshots") => Some(PluginPermission::Write),
        // #323 T2 五命令批迁（子批②）：`save_neodata_token` 是写 op —— 写 Python 脚本
        // 缓存 + 改 `neodata_token` 共享内存 + 改 `workflow_template` 的 `stock-analysis`
        // 模板变量（三段都可观察的持久效应）。同批的 `query_decision_backtest`（子批③）
        // 纯读 ⇒ **必须仍是 `None`**（给读缝加闸会把只读插件无端拒掉）。
        ("stock.analysis", "save_neodata_token") => Some(PluginPermission::Write),
        // #324 T2 F7 回测验证三条迁：`run_decision_backtest` 是写 op —— 拉历史 K 线
        // 回放并写 `decision_validations` 表（执行体留宿主，cron 与 CLI 共用同一实现，
        // 接缝只做触发口）。同批两条读（`list_decision_validations` /
        // `compute_validation_report`）纯读 ⇒ **必须仍是 `None`**（读缝不加闸）。
        ("stock.analysis", "run_decision_backtest") => Some(PluginPermission::Write),
        // F6 别名族（#35，#322）：`evolve_analyst` 触发进化（Prompt/配置优化建议落库）
        // 、`save_analyst_feedback` 写反馈表 ⇒ `Write`。四条 get_*（两条进化状态 +
        // 两条反馈读面）是读 ⇒ **必须仍是 `None`**（同上：读缝不加闸）。
        (
            "stock.analysis",
            "evolve_analyst" | "save_analyst_feedback",
        ) => Some(PluginPermission::Write),
        // F2 组合与风控（#35，#322）：`update_portfolio_holding` 改持仓行 /
        // `import_portfolio_from_vlm` 批量落库（可选清空旧持仓）⇒ `Write`。
        // `get_position_limits` / `run_stress_test` 是读/纯计算面 ⇒ 不登记。
        (
            "trading.engine",
            "update_portfolio_holding" | "import_portfolio_from_vlm",
        ) => Some(PluginPermission::Write),
        // 批 1a（#35）：九个 cron 管理写 op 增删改宿主 `CronJobStore` ⇒ `Write`；
        // 三个 list 仍是 `None`（同上：读缝不加闸）。
        (
            "stock.analysis",
            "create_recommendation_cron"
            | "toggle_recommendation_cron"
            | "delete_recommendation_cron"
            | "create_trend_screening_cron"
            | "toggle_trend_screening_cron"
            | "delete_trend_screening_cron"
            | "create_pool_scan_cron"
            | "toggle_pool_scan_cron"
            | "delete_pool_scan_cron",
        ) => Some(PluginPermission::Write),
        // `opc.store` 的写 op 一律要 `write`：blog 三件（发布 / 删除 / 创建）+ #36 批 2 的
        // 四条 id·枚举写 op（发票状态变更 / 内容资产更新 / 内容资产删除 / 发布计划取消）+
        // #36 批 3 的八条创建 / 流程写 op + #32 批 B 的项目三件（更新 / 删除 / 里程碑完成）
        // —— 全都改库，删除类还**不可逆**。读 op（八个列表 + `get_financial_report` +
        // `get_project`）**必须仍是 `None`**（给读缝加闸会把只读插件无端拒掉）。
        (
            "opc.store",
            "publish_blog_post"
            | "delete_blog_post"
            | "create_blog_post"
            | "transition_invoice"
            | "update_content_asset"
            | "delete_content_asset"
            | "cancel_publish_schedule"
            | "create_invoice"
            | "delete_invoice"
            | "create_customer"
            | "update_customer"
            | "delete_customer"
            | "create_project"
            | "add_milestone"
            | "create_landing_page"
            | "create_content_asset"
            | "create_publish_schedule"
            | "process_due_schedules"
            | "update_project"
            | "delete_project"
            | "complete_milestone"
            | "publish_landing_page"
            | "delete_landing_page"
            | "mark_contact_read"
            // #264 批 1：交付域三条写 op（开票 / 状态机 / 删除）；两条读 op
            // （`list_delivery_invoices` / `delivery_summary`）仍 `None`。
            | "create_invoice_from_lead"
            | "update_delivery_invoice_status"
            | "delete_delivery_invoice"
            // #264 批 2：需求发现五条写 op（平台存/删、策略存、扫描落库、补录线索）；
            // 三条读 op（`list_platforms` / `list_leads` / `get_scan_policy`）仍 `None`。
            | "save_platform"
            | "delete_platform"
            | "save_scan_policy"
            | "discover_and_evaluate_leads"
            | "create_lead"
            // #264 批 3：需求实现三条写 op（状态机 / 转模板 / 启动执行）—— 都改库
            // （转模板还会回灌能力索引），无读 op 随迁。
            | "update_lead_status"
            | "convert_lead_to_workflow"
            | "run_lead_workflow"
            // #264 批 5：交付面四条写 op（落交付行 + 后台启工作流 / 状态更新 / 重试 /
            // 取消）—— 都改库；两条读 op（`list_deliveries` / `get_delivery`）仍 `None`。
            | "execute_demand_workflow"
            | "update_delivery"
            | "retry_delivery"
            | "cancel_delivery"
            // #264 批 6：发现面三条写 op（扫描落库 / 关闭缺口 / 分析自动建行）——
            // 都改库；两条读 op（`scan_capabilities` / `list_capability_gaps`）仍 `None`。
            | "proactive_evaluate_leads"
            | "close_capability_gap"
            | "analyze_capability_gaps"
            // #264 批 7 刀 A：订阅面两条写 op（新增/更新订阅、删除订阅 —— 后者不可逆）；
            // 同批另九条 op（`list_subscriptions` + 域包配置五 + 运行时域包三）全是只读
            // ⇒ 必须仍 `None`（给读缝加闸会把只读插件无端拒掉，锁在 READ_OPS 用例）。
            // #264 批 7 刀 B1：域包全读七 op（`get_workflow_config` / `validate_entity` /
            // `batch_validate_entities` / `compute_kpis` / `get_capability_pack_data` /
            // `get_capability_pack_precheck` / `get_capability_pack_health`）——
            // 同样全是只读，写分支**一条都不加**，锁在 READ_OPS 用例。
            | "save_subscription"
            | "delete_subscription"
            // #264 批 7 刀 B2：域包单据面**六条写 op**（更新发票 / 记录 KPI / 导入人才库 /
            // 同步舰队 / 更新发布计划 / 动态工作流执行）—— 全都改库，其中导入人才库与动态
            // 工作流执行还会触发批量的落库 / 引擎执行。同批另**六条读 op**（`get_invoice` /
            // `get_customer` / `find_customer_by_email` / `list_revenue` /
            // `get_investment_advice` / `get_publish_schedule`）**必须仍是 `None`**，
            // 锁在 READ_OPS 用例（给读缝加闸会把只读插件无端拒掉）。
            | "update_invoice"
            | "record_kpi"
            | "import_talent_library"
            | "sync_fleet"
            | "update_publish_schedule"
            | "execute_dynamic_workflow"
            // #264 批 7 刀 B3：域包规则触发（写 op —— 规则动作会改库）。同批另三条
            // （`capability_pack_dashboard` / `capability_pack_workflow_steps` /
            // `capability_pack_automation_rules`）全是只读，写分支一条都不加，
            // 锁在 READ_OPS 用例。
            | "run_automation_rules"
            // #264 批 7 刀 B3（后三条）：订阅扫描落库 + 幂等建/改调度任务 —— 都改库，
            // 扫描还会推进订阅到期时间。`match_lead_capabilities` 是纯读 ⇒ 必须仍
            // `None`（锁在 READ_OPS 用例）。
            | "run_subscription_scan"
            | "ensure_demand_scan_job"
            // 刀 B3 收尾：自动学习会写 RL 经验库并触发进化 —— 写 op。
            | "auto_learn_workflow"
            // #264 刀 C 批 3：学习面六条写 op（反思 / 进化 / 自我改进 / 学习闭环 /
            // 手动 RL 优化 / 手动记录经验）—— 都写经验库或改工作流 / 配置。同批
            // `get_rl_stats` 是纯读 ⇒ 必须仍 `None`（锁在 READ_OPS 用例）。
            | "reflect_on_workflow"
            | "evolve_workflow"
            | "run_self_improvement"
            | "trigger_capability_pack_learning"
            | "trigger_rl_optimization"
            | "record_rl_experience"
            // #264 批 4a：工作项六条写 op（建 / 认领 / 提交评审 / 升级阻塞 / 解除阻塞 /
            // 自改进循环）—— 都改 `opc_work_items` 或写经验库。同批 `kanban_board` 是纯读
            // ⇒ 必须仍 `None`（锁在 READ_OPS 用例）。
            | "create_work_item"
            | "work_item_start"
            | "work_item_review"
            | "escalate_work_item"
            | "work_item_unblock"
            | "run_self_improving_work_item"
            // #264 批 4b：域包与内容资产三条写 op（导出落盘 / 导入解包注册 / 启停写库）。
            // 同批 `get_content_asset` / `market_list` 是纯读 ⇒ 必须仍 `None`（锁在 READ_OPS 用例）。
            | "export_capability_pack"
            | "import_capability_pack"
            | "set_capability_pack_enabled",
        ) => Some(PluginPermission::Write),
        _ => None,
    }
}

impl HostPeer {
    /// 本插件的声明权限是否覆盖 `permission`。
    ///
    /// 只做**精确匹配**（不套 `PluginPermission::implies_*`）：登记表要求哪个权限，清单里就
    /// 必须出现哪个 —— 避免 `execute` / `subprocess_execution` 之类宽权限悄悄补齐 `write`
    /// 的准入面。要放宽口径必须显式改这里并补用例。
    #[must_use]
    fn has_permission(&self, permission: PluginPermission) -> bool {
        self.permissions.contains(&permission)
    }

    /// 处理一帧入站请求，返回**未回填 `request_id`** 的响应（由读线程统一回填）。
    fn handle_inbound(&self, request: &FrameRequest) -> FrameResponse {
        match request.op.as_str() {
            ops::EMIT => self.handle_emit(&request.args),
            ops::CALL_SEAM => self.handle_call_seam(&request.args, &request.call_chain),
            other => FrameResponse::error(
                error_codes::PLUGIN_UNKNOWN_OP,
                format!("宿主不支持插件主动发起的 op `{other}`"),
            ),
        }
    }

    /// `emit`：插件请求宿主代为派发一个领域事件（L1 事件桥的写方向）。
    ///
    /// **不等待派发完成**（返回 `{"queued": true}`）。理由不是偷懒，而是必须：
    /// B 层插件的订阅者用 `EventMatcher::any()` 挂在总线上，宿主派发时会**回调发起者自己**
    /// （[`WorkerEventSubscriber`] 再等该插件的 worker 回帧）。可该 worker 此刻正阻塞在
    /// 「等这条 `emit` 的响应」上，重入帧只能进它的 `deferred` 队列 ⇒ 同步等待必然自锁。
    fn handle_emit(&self, args: &Value) -> FrameResponse {
        let Some(bus) = self.registry.get_event_dispatcher() else {
            return self.unavailable("event.dispatch");
        };
        let category: EventCategory =
            match serde_json::from_value(args.get("category").cloned().unwrap_or(Value::Null)) {
                Ok(category) => category,
                Err(e) => {
                    return invalid_args(format!(
                        "`category` 必须是 agent / workflow / orchestration / system 之一：{e}"
                    ));
                },
            };
        let Some(kind) = args.get("kind").and_then(Value::as_str).filter(|k| !k.is_empty()) else {
            return invalid_args("`kind` 缺失或为空");
        };
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        // `source` 缺省取插件 ID：事件来源不该是空串，插件身份是最好的兜底。
        let source = args
            .get("source")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(self.plugin_id.as_str())
            .to_string();
        let mut event = DomainEvent::new(category, kind, payload, source);

        let Some(runtime) = self.runtime.clone() else {
            return FrameResponse::error(
                error_codes::PLUGIN_CALL_FAILED,
                "宿主缺少 tokio runtime，无法异步派发事件",
            );
        };
        let plugin_id = self.plugin_id.clone();
        runtime.spawn(async move {
            let outcome = bus.dispatch(&mut event, DispatchMode::Emit).await;
            tracing::debug!(
                plugin_id = %plugin_id,
                invoked = outcome.invoked,
                rejected = outcome.rejected,
                "插件 emit 的事件已完成派发"
            );
        });
        FrameResponse::success(json!({ "queued": true }))
    }

    /// `call_seam`：插件请求宿主代为调用**另一条接缝**（L2 跨插件调用）。
    ///
    /// `args` 形状：`{ "seam": String, "op": String, "args": Value }`，
    /// 返回值与同接缝的远程门面**完全对称**（`value` 即该接缝方法的返回值）。
    fn handle_call_seam(&self, args: &Value, inbound_chain: &[String]) -> FrameResponse {
        let Some(seam) = args.get("seam").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            return invalid_args("`seam` 缺失或为空");
        };
        let Some(op) = args.get("op").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            return invalid_args("`op` 缺失或为空");
        };
        let call_args = args.get("args").cloned().unwrap_or(Value::Null);

        // ① 声明校验（PLAN §5.2 `calls`）：声明是准入，运行时才校验 = 声明形同虚设。
        if !self.declared_calls.iter().any(|declared| declared == seam) {
            return FrameResponse::error(
                error_codes::SEAM_CALL_NOT_DECLARED,
                format!("插件 `{}` 未在声明 `calls` 中列出接缝 `{seam}`", self.plugin_id),
            );
        }
        // ② 自调检查：目标接缝是本插件自己提供的 ⇒ 环（本插件的 worker 正阻塞着等这条
        //    响应，再调回它必然是死锁，故这里必须**提前**拒掉，而不是靠等待超时）。
        if self.declared_capabilities.iter().any(|declared| declared == seam) {
            return FrameResponse::error(
                error_codes::SEAM_CALL_CYCLE_DETECTED,
                format!("插件 `{}` 调用了自己提供的接缝 `{seam}`", self.plugin_id),
            );
        }
        // ③ 深度上限：目标若是**进程内** provider，其 `invoke_with_chain` 走默认实现、
        //    不做自检，故宿主这一侧必须判（覆盖面差异见 AGENTS 的「门禁覆盖面」教训）。
        if inbound_chain.len() >= MAX_SEAM_CALL_DEPTH {
            return FrameResponse::error(
                error_codes::SEAM_CALL_DEPTH_EXCEEDED,
                format!(
                    "调用链长度 {} 已达上限 {MAX_SEAM_CALL_DEPTH}（链：{inbound_chain:?}）",
                    inbound_chain.len()
                ),
            );
        }
        // ④ 分派。把入站链设为新的 thread-local 基线：目标若是进程内 provider 并继续转发，
        //    链才能延续下去（门面在跨线程前会取快照）。
        let chain = inbound_chain.to_vec();
        with_call_chain(chain, || self.dispatch_seam(seam, op, call_args))
    }

    /// 按 PLAN §5.4 的 op 映射表把 `call_seam` 落到具体接缝上。
    ///
    /// 返回值形状与同接缝的门面**逐字对称**（门面把 `value` 反序列化为该方法的结果类型）。
    fn dispatch_seam(&self, seam: &str, op: &str, args: Value) -> FrameResponse {
        // 准入闸（**在 match 之前**）：写 op 要求清单已声明对应权限。
        // 拒绝发生在取 provider / 反序列化参数之前 ⇒ 不触达 provider、零副作用。
        if let Some(permission) = required_permission_for(seam, op)
            && !self.has_permission(permission)
        {
            return FrameResponse::error(
                error_codes::PLUGIN_PERMISSION_DENIED,
                format!(
                    "插件未声明 `{}` 权限，拒绝接缝 `{seam}` 的写操作 `{op}`",
                    permission.as_str()
                ),
            );
        }
        // 宿主内部接缝（`HOST_INTERNAL_SEAMS`）：注册表里有 provider，但**不向插件开放**。
        // 闸放在取 provider / 反序列化之前 ⇒ 拒绝路径零触达。
        if HOST_INTERNAL_SEAMS.contains(&seam) {
            return FrameResponse::error(
                error_codes::SEAM_CALL_UNSUPPORTED,
                format!(
                    "接缝 `{seam}` 是宿主内部注入通道，不对插件开放（会话状态数据主权在宿主，PLAN §64.3）"
                ),
            );
        }
        match seam {
            "agent.loop" if op == "run_turn" => {
                let Some(runner) = self.registry.get_agent_turn_runner() else {
                    return self.unavailable(seam);
                };
                let request: AgentTurnRequest =
                    match deserialize_arg(&args, "request", "agent.loop") {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(run) = self.block_on(runner.run_turn(request)) else {
                    return no_runtime(seam);
                };
                match run {
                    Ok(result) => to_value("AgentTurnResult", &result),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "workflow.sandbox" if op == "execute" => {
                let Some(sandbox) = self.registry.get_sandbox() else {
                    return self.unavailable(seam);
                };
                let genome: WorkflowGenome = match deserialize_arg(&args, "genome", seam) {
                    Ok(genome) => genome,
                    Err(response) => return response,
                };
                let test_input = args.get("test_input").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(sandbox.execute(&genome, &test_input)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(validation) => to_value("SandboxValidationResult", &validation),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e),
                }
            },
            // 唯一**同步**的接缝：不需要 runtime，也不存在跨线程丢链的问题。
            "workflow.business_rule" if op == "evaluate" => {
                let Some(evaluator) = self.registry.get_business_rule() else {
                    return self.unavailable(seam);
                };
                let node_type: NodeKind = match deserialize_arg(&args, "node_type", seam) {
                    Ok(node_type) => node_type,
                    Err(response) => return response,
                };
                let node_input = args.get("node_input").cloned().unwrap_or(Value::Null);
                let outcome = evaluator.evaluate(&node_type, &node_input);
                to_value("RuleEvaluationOutcome", &outcome)
            },
            "webhook.dispatch" if op == "dispatch" => {
                let Some(dispatcher) = self.registry.get_webhook_dispatch() else {
                    return self.unavailable(seam);
                };
                let event: WebhookEvent = match deserialize_arg(&args, "event", seam) {
                    Ok(event) => event,
                    Err(response) => return response,
                };
                let data: HashMap<String, Value> = match deserialize_arg(&args, "data", seam) {
                    Ok(data) => data,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(dispatcher.dispatch(event, data)) else {
                    return no_runtime(seam);
                };
                to_value("DispatchResult", &result)
            },
            // ═══════════ §5.4 其余 7 条接缝（P5 补齐；op 名沿用映射表，含别名） ═══════════
            "workflow.reflector" if op == "reflect" => {
                let Some(reflector) = self.registry.get_workflow_reflector() else {
                    return self.unavailable(seam);
                };
                let execution: WorkflowExecutionRecord =
                    match deserialize_arg(&args, "execution", seam) {
                        Ok(execution) => execution,
                        Err(response) => return response,
                    };
                let Some(result) = self.block_on(reflector.reflect(&execution)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(reflection) => to_value("Reflection", &reflection),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e),
                }
            },
            // 映射表的 op 名是 `evolve`（别名），对应 trait 的 `evolve_generation`。
            "workflow.evolver" if op == "evolve" => {
                let Some(evolver) = self.registry.get_workflow_evolver() else {
                    return self.unavailable(seam);
                };
                let mut population: EvolutionPopulation =
                    match deserialize_arg(&args, "population", seam) {
                        Ok(population) => population,
                        Err(response) => return response,
                    };
                let reflections: Vec<Reflection> = match deserialize_arg(&args, "reflections", seam)
                {
                    Ok(reflections) => reflections,
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(evolver.evolve_generation(&mut population, &reflections))
                else {
                    return no_runtime(seam);
                };
                match result {
                    // `&mut EvolutionPopulation` 的改写必须**回传调用方**（否则调用方白传），
                    // 故响应是 `{genome, population}` 双字段，而不是只有 genome。
                    Ok(genome) => {
                        let reply = json!({ "genome": genome, "population": population });
                        to_value("EvolveReply", &reply)
                    },
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e),
                }
            },
            "workflow.optimizer" if op == "suggest" => {
                let Some(optimizer) = self.registry.get_workflow_optimizer() else {
                    return self.unavailable(seam);
                };
                let template: WorkflowTemplateData = match deserialize_arg(&args, "template", seam)
                {
                    Ok(template) => template,
                    Err(response) => return response,
                };
                let reflection: Reflection = match deserialize_arg(&args, "reflection", seam) {
                    Ok(reflection) => reflection,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(optimizer.suggest(&template, &reflection)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(suggestions) => to_value("Vec<WorkflowSuggestion>", &suggestions),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e),
                }
            },
            "message.callback" if op == "on_message" => {
                let Some(callback) = self.registry.get_message_callback() else {
                    return self.unavailable(seam);
                };
                let platform: String = match deserialize_arg(&args, "platform", seam) {
                    Ok(platform) => platform,
                    Err(response) => return response,
                };
                let user_id: String = match deserialize_arg(&args, "user_id", seam) {
                    Ok(user_id) => user_id,
                    Err(response) => return response,
                };
                // 缺省 / `null` 一律落为 `None`（trait 里 `username` 本就可选）。
                let username: Option<String> =
                    match deserialize_optional_arg(&args, "username", seam) {
                        Ok(username) => username,
                        Err(response) => return response,
                    };
                let chat_id: String = match deserialize_arg(&args, "chat_id", seam) {
                    Ok(chat_id) => chat_id,
                    Err(response) => return response,
                };
                let text: String = match deserialize_arg(&args, "text", seam) {
                    Ok(text) => text,
                    Err(response) => return response,
                };
                let Some(reply) = self.block_on(callback.on_message(
                    &platform,
                    &user_id,
                    username.as_deref(),
                    &chat_id,
                    &text,
                )) else {
                    return no_runtime(seam);
                };
                to_value("OnMessageReply", &json!({ "reply": reply }))
            },
            // `SessionLogInvariant` 是**同步**接缝（无 async 方法），故此处不需要 runtime。
            "session.log.invariant" if op == "record" => {
                let Some(log) = self.registry.get_session_log_invariant() else {
                    return self.unavailable(seam);
                };
                let session_id: String = match deserialize_arg(&args, "session_id", seam) {
                    Ok(session_id) => session_id,
                    Err(response) => return response,
                };
                let content: ModelVisibleContent = match deserialize_arg(&args, "content", seam) {
                    Ok(content) => content,
                    Err(response) => return response,
                };
                match log.record_model_visible(&session_id, content) {
                    // 违反也已在宿主侧落盘（证据保留），错误如实回传远程调用方
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(violation) => {
                        FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, violation.to_string())
                    },
                }
            },
            "session.log.invariant" if op == "assert" => {
                let Some(log) = self.registry.get_session_log_invariant() else {
                    return self.unavailable(seam);
                };
                let session_id: String = match deserialize_arg(&args, "session_id", seam) {
                    Ok(session_id) => session_id,
                    Err(response) => return response,
                };
                match log.assert_replayable(&session_id) {
                    Ok(()) => FrameResponse::success(json!({ "ok": true })),
                    Err(violation) => {
                        FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, violation.to_string())
                    },
                }
            },
            // `tool.set` 的**执行面**：接缝的提供方是 `ToolSetProvider`（贡献工具集），
            // 而 `exec` 是「执行其中某个工具」，故这里走 `tools()` 取工具 + `Tool::call`。
            "tool.set" if op == "exec" => {
                let Some(provider) = self.registry.get_tool_set() else {
                    return self.unavailable(seam);
                };
                let name: String = match deserialize_arg(&args, "name", seam) {
                    Ok(name) => name,
                    Err(response) => return response,
                };
                let tool_input = args.get("args").cloned().unwrap_or(Value::Null);
                let Some(tool) =
                    provider.tools().into_iter().find(|tool| tool.name() == name.as_str())
                else {
                    return invalid_args(format!(
                        "接缝 `{seam}` 的工具集中没有名为 `{name}` 的工具"
                    ));
                };
                // `ToolContext` 含 `Arc<dyn …>` / `Arc<Notify>`，**不可跨进程传输**，
                // 故 §5.4 的 args 只约定 `{name, args}`；此处构造一个最小上下文。
                let ctx = ToolContext::new(".");
                let Some(result) = self.block_on(tool.call(tool_input, &ctx)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(result) => to_value("ToolResult", &result),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // ── market.quote 接缝：宿主把**已有**行情能力暴露给插件（方向与前几条相反） ──
            // `market.quote` 不在 `SUPPORTED_REMOTE_SEAMS`：那份清单是「插件提供给宿主」
            // 的接缝，本接缝是「宿主提供给插件」，只走入站分派、不参与远程门面注册。
            "market.quote" if op == "get_quote" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_quote(&stock_code)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(quote) => to_value("StockQuote", &quote),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_klines" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let period: String = match deserialize_arg(&args, "period", seam) {
                    Ok(period) => period,
                    Err(response) => return response,
                };
                // `limit` 缺省 100 根；`adj_type` 缺省 / `null` 都等于「不复权」。
                let limit: u32 = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(Some(limit)) => limit,
                    Ok(None) => 100,
                    Err(response) => return response,
                };
                let adj_type: Option<axagent_harness::AdjType> =
                    match deserialize_optional_arg(&args, "adj_type", seam) {
                        Ok(adj_type) => adj_type,
                        Err(response) => return response,
                    };
                let Some(result) =
                    self.block_on(provider.get_klines(&stock_code, &period, limit, adj_type))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(klines) => to_value("Vec<KLine>", &klines),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "search_stock" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let keyword: String = match deserialize_arg(&args, "keyword", seam) {
                    Ok(keyword) => keyword,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.search_stock(&keyword)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(results) => to_value("Vec<StockSearchResult>", &results),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // ── 个股数据面（P6.1 B 批）：A 批扩的 12 个 `MarketDataProvider` 方法 ──
            // 全部方法末参 `as_of` 由 args 显式透传（跨进程读不到宿主 task_local）；
            // 参数键保持 snake_case（与 #80 一致），DTO 字段才是 camelCase。
            "market.quote" if op == "get_financials" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_financials(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(reports) => to_value("FinancialReport", &reports),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // F1 余三条（#35）：交易日历刷新（宿主命令 `refresh_trading_calendar` 迁 T2）。
            // 无入参；读性质（拉取并返回节假日序列），不加权限闸。缺省实现报「不支持」
            // 与宿主命令保持等价失败行为（东财报表已下线的既存状态，见 harness 注释）。
            "market.quote" if op == "refresh_trading_calendar" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.refresh_trading_calendar()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(holidays) => to_value("TradingCalendarHolidays", &holidays),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_money_flow" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_money_flow(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(flow) => to_value("MoneyFlow", &flow),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_north_bound_holding" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(provider.get_north_bound_holding(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(holding) => to_value("NorthBoundHolding", &holding),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_dragon_tiger" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_dragon_tiger(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(entries) => to_value("DragonTigerEntry", &entries),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_margin_balance_trend" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let days: u32 = match deserialize_arg(&args, "days", seam) {
                    Ok(days) => days,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(provider.get_margin_balance_trend(&stock_code, days, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(trend) => to_value("MarginData", &trend),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_margin_data" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_margin_data(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(data) => to_value("MarginData", &data),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_holder_count" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_holder_count(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(count) => to_value("HolderCount", &count),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_block_trades" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_block_trades(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(trades) => to_value("BlockTrade", &trades),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_pledge_data" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_pledge_data(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(pledge) => to_value("PledgeData", &pledge),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_sector_info" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_sector_info(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(sector) => to_value("SectorInfo", &sector),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_news" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let limit: u32 = match deserialize_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_news(&stock_code, limit, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(news) => to_value("NewsItem", &news),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "market_snapshot_batch" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let codes: Vec<String> = match deserialize_arg(&args, "codes", seam) {
                    Ok(codes) => codes,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.market_snapshot_batch(&codes, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(quotes) => to_value("StockQuote", &quotes),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // ── P6.1 迁移：逐只并发批量面（原宿主 batch_get_* 命令的接缝形态）──
            // 与 market_snapshot_batch 的区别是契约的一部分：本面有单只超时/失败归因/
            // 提前终止，返回 BatchResult 双 map；`total_timeout_ms` 保持「接受但不强制」。
            "market.quote" if op == "batch_get_quotes" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let codes: Vec<String> = match deserialize_arg(&args, "codes", seam) {
                    Ok(codes) => codes,
                    Err(response) => return response,
                };
                let per_stock_timeout_ms: Option<u64> =
                    match deserialize_optional_arg(&args, "per_stock_timeout_ms", seam) {
                        Ok(value) => value,
                        Err(response) => return response,
                    };
                let total_timeout_ms: Option<u64> =
                    match deserialize_optional_arg(&args, "total_timeout_ms", seam) {
                        Ok(value) => value,
                        Err(response) => return response,
                    };
                let max_failures: Option<usize> =
                    match deserialize_optional_arg(&args, "max_failures", seam) {
                        Ok(value) => value,
                        Err(response) => return response,
                    };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let request = axagent_harness::market_data::MarketBatchQuotesRequest {
                    codes,
                    per_stock_timeout_ms,
                    total_timeout_ms,
                    max_failures,
                };
                let Some(result) = self.block_on(provider.batch_get_quotes(&request, as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(batch) => to_value("BatchResult", &batch),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "batch_get_financials" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let codes: Vec<String> = match deserialize_arg(&args, "codes", seam) {
                    Ok(codes) => codes,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.batch_get_financials(&codes, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(batch) => to_value("BatchResult", &batch),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // ── 剩余数据面（P6.1 D 批）：C 批补齐的 25 个 `MarketDataProvider` 方法 ──
            // 与 B 批同口径：参数键 snake_case、DTO 走 camelCase 序列化；
            // 无参 op 不读任何键，但**仍须**解析并透传 `as_of`（跨进程读不到宿主 task_local）。
            "market.quote" if op == "search_news" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let keyword: String = match deserialize_arg(&args, "keyword", seam) {
                    Ok(keyword) => keyword,
                    Err(response) => return response,
                };
                let limit: u32 = match deserialize_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.search_news(&keyword, limit, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(news) => to_value("NewsItem", &news),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "search_concept_boards" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let keyword: String = match deserialize_arg(&args, "keyword", seam) {
                    Ok(keyword) => keyword,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.search_concept_boards(&keyword, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(boards) => to_value("ConceptBoard", &boards),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_social_sentiment" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_social_sentiment(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(sentiment) => to_value("SocialSentiment", &sentiment),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_research_reports" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_research_reports(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(reports) => to_value("ResearchReport", &reports),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_peers" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_peers(&stock_code, as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(peers) => to_value("PeerComparison", &peers),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_option_pcr" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_option_pcr(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(pcr) => to_value("OptionPCR", &pcr),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_north_bound_flow" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_north_bound_flow(as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(flow) => to_value("NorthBoundFlow", &flow),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_market_dragon_tiger" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_market_dragon_tiger(as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(entries) => to_value("MarketDragonTiger", &entries),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_lockup_schedule" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_lockup_schedule(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(schedule) => to_value("LockupSchedule", &schedule),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_limit_up_pool" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                // `requested_date` 缺省 / `null` 都表示「取当下」（回放时按截止日）。
                let requested_date: Option<String> =
                    match deserialize_optional_arg(&args, "requested_date", seam) {
                        Ok(requested_date) => requested_date,
                        Err(response) => return response,
                    };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(provider.get_limit_up_pool(requested_date.as_deref(), as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(pool) => to_value("LimitUpPool", &pool),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_international_quote" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(provider.get_international_quote(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(quote) => to_value("StockQuote", &quote),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_international_klines" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let period: String = match deserialize_arg(&args, "period", seam) {
                    Ok(period) => period,
                    Err(response) => return response,
                };
                let limit: u32 = match deserialize_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let adj_type: Option<axagent_harness::AdjType> =
                    match deserialize_optional_arg(&args, "adj_type", seam) {
                        Ok(adj_type) => adj_type,
                        Err(response) => return response,
                    };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_international_klines(
                    &stock_code,
                    &period,
                    limit,
                    adj_type,
                    as_of,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(klines) => to_value("KLine", &klines),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_industry_ranking" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_industry_ranking(as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(ranking) => to_value("IndustryRank", &ranking),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_index_quotes_outcome" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_index_quotes_outcome(as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => to_value("DataOutcome", &outcome),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_hot_stocks" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_hot_stocks(as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(stocks) => to_value("HotStock", &stocks),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_forex_klines" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let pair: String = match deserialize_arg(&args, "pair", seam) {
                    Ok(pair) => pair,
                    Err(response) => return response,
                };
                let period: String = match deserialize_arg(&args, "period", seam) {
                    Ok(period) => period,
                    Err(response) => return response,
                };
                let limit: u32 = match deserialize_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(provider.get_forex_klines(&pair, &period, limit, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(klines) => to_value("KLine", &klines),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_earnings_calendar" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(provider.get_earnings_calendar(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(events) => to_value("EarningsEvent", &events),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_dividend_records" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_dividend_records(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(records) => to_value("DividendRecord", &records),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_consensus_eps" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_consensus_eps(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(eps) => to_value("ConsensusEPS", &eps),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_concept_board_members" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let board_code: String = match deserialize_arg(&args, "board_code", seam) {
                    Ok(board_code) => board_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(provider.get_concept_board_members(&board_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(members) => to_value("BoardMember", &members),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_concept_blocks" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_concept_blocks(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(blocks) => to_value("ConceptBlocks", &blocks),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_cls_flash" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_cls_flash(as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(items) => to_value("ClsFlashItem", &items),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_benchmark_klines" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let benchmark_code: String = match deserialize_arg(&args, "benchmark_code", seam) {
                    Ok(benchmark_code) => benchmark_code,
                    Err(response) => return response,
                };
                let period: String = match deserialize_arg(&args, "period", seam) {
                    Ok(period) => period,
                    Err(response) => return response,
                };
                let limit: u32 = match deserialize_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_benchmark_klines(
                    &benchmark_code,
                    &period,
                    limit,
                    as_of,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(klines) => to_value("KLine", &klines),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "get_announcements" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let stock_code: String = match deserialize_arg(&args, "stock_code", seam) {
                    Ok(stock_code) => stock_code,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_announcements(&stock_code, as_of))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(announcements) => to_value("Announcement", &announcements),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "market.quote" if op == "market_list_page" => {
                let Some(provider) = self.registry.get_market_data() else {
                    return self.unavailable(seam);
                };
                let page: u32 = match deserialize_arg(&args, "page", seam) {
                    Ok(page) => page,
                    Err(response) => return response,
                };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.market_list_page(page, as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(list_page) => to_value("MarketListPage", &list_page),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // ── jobs.store 接缝：宿主把 cron 任务读写能力暴露给插件 ──
            // 方向与 `market.quote` 相同（宿主提供给插件），故只走入站分派、不进远程门面
            // 清单。只读 op：`list` / `get` / `count`；写 op：`add` / `remove` / `set_status`
            // （`update` 不在本批）。写 op 的 `write` 权限闸已由 `dispatch_seam` 开头统一
            // 把关，分支内不重复判断。
            "jobs.store" if op == "list" => {
                let Some(provider) = self.registry.get_jobs_store() else {
                    return self.unavailable(seam);
                };
                let task_type: Option<String> =
                    match deserialize_optional_arg(&args, "task_type", seam) {
                        Ok(task_type) => task_type,
                        Err(response) => return response,
                    };
                // `active_only` 缺省 `false`：省略即「不按状态过滤」。
                let active_only: bool = match deserialize_optional_arg(&args, "active_only", seam) {
                    Ok(active_only) => active_only.unwrap_or(false),
                    Err(response) => return response,
                };
                let Some(result) =
                    self.block_on(provider.list_jobs(task_type.as_deref(), active_only))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(jobs) => to_value("CronJobView", &jobs),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "jobs.store" if op == "get" => {
                let Some(provider) = self.registry.get_jobs_store() else {
                    return self.unavailable(seam);
                };
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get_job(&id)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 不存在 ⇒ `null`，**不是错误**：调用方按「查无此任务」处理。
                    Ok(job) => to_value("CronJobView", &job),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "jobs.store" if op == "count" => {
                let Some(provider) = self.registry.get_jobs_store() else {
                    return self.unavailable(seam);
                };
                let task_type: Option<String> =
                    match deserialize_optional_arg(&args, "task_type", seam) {
                        Ok(task_type) => task_type,
                        Err(response) => return response,
                    };
                let Some(result) = self.block_on(provider.count_jobs(task_type.as_deref())) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(count) => FrameResponse::success(json!({ "count": count })),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // ── jobs.store 写面：`add` / `remove` / `set_status` ──
            // `add` 的 `job` 直接按 harness 的 [`axagent_harness::NewCronJob`] 反序列化：
            // 可选字段与 `recurring = true` / `priority = "medium"` 的缺省值全部由该 DTO
            // 的 serde 声明提供，分派层不再另填一套默认值（两处各写一份必然漂移）。
            "jobs.store" if op == "add" => {
                let job: axagent_harness::NewCronJob = match deserialize_arg(&args, "job", seam) {
                    Ok(job) => job,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_jobs_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.add_job(job)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(id) => FrameResponse::success(json!({ "id": id })),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "jobs.store" if op == "remove" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_jobs_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.remove_job(&id)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 三态结果**原样**透出：`"removed"` / `"not_found"` / `"protected"`。
                    // 「受保护」「不存在」都是正常结果，不借用错误码表达。
                    Ok(outcome) => to_value("RemoveJobOutcome", &outcome),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "jobs.store" if op == "set_status" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let status: String = match deserialize_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                // 先校验字面量、后取 provider：非法取值是调用方的参数错误，不该被
                // 「provider 暂不可用」掩盖，更不该送进 provider。
                if !matches!(status.as_str(), "active" | "paused" | "disabled") {
                    return invalid_args(format!(
                        "接缝 `{seam}` 的 `status` 只接受 `active` / `paused` / `disabled`，\
                         实际收到：`{status}`"
                    ));
                }
                let Some(provider) = self.registry.get_jobs_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.set_job_status(&id, &status)) else {
                    return no_runtime(seam);
                };
                match result {
                    // `false` = 任务不存在（正常结果，不是错误）。
                    Ok(found) => FrameResponse::success(json!({ "found": found })),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // ── workflow.run 接缝：异步 job（start 受理 + status / cancel / list_active / list_recent） ──
            // op 参数一律 **snake_case**（与 jobs.store / market.quote 的入站约定一致），而
            // `WorkflowRunRequest` 的线上 DTO 是 **camelCase** —— 故这里逐字段解析后在本层
            // 用结构体字面量构造，**不能**整体把 `args` 反序列化成 `WorkflowRunRequest`
            // （否则 `template_id` 被静默丢成空串，宿主只会报「模板不存在」）。
            // `owner` 不接收插件参数：一律用连接身份（[`Self::plugin_id`]）注入。
            "workflow.run" if op == "start" => {
                let template_id: String = match deserialize_arg(&args, "template_id", seam) {
                    Ok(template_id) => template_id,
                    Err(response) => return response,
                };
                // 空白模板 id 等于没给：在取 provider 之前判掉，免得送进宿主白名单查询。
                let template_id = template_id.trim().to_string();
                if template_id.is_empty() {
                    return invalid_args(format!(
                        "接缝 `{seam}` 的 `template_id` 不能为空白字符串"
                    ));
                }
                let input: Option<Value> = match deserialize_optional_arg(&args, "input", seam) {
                    Ok(input) => input,
                    Err(response) => return response,
                };
                let variables: Vec<Variable> =
                    match deserialize_optional_arg(&args, "variables", seam) {
                        Ok(variables) => variables.unwrap_or_default(),
                        Err(response) => return response,
                    };
                let model_id: Option<String> =
                    match deserialize_optional_arg(&args, "model_id", seam) {
                        Ok(model_id) => model_id,
                        Err(response) => return response,
                    };
                let provider_id: Option<String> =
                    match deserialize_optional_arg(&args, "provider_id", seam) {
                        Ok(provider_id) => provider_id,
                        Err(response) => return response,
                    };
                let max_concurrent: Option<u32> =
                    match deserialize_optional_arg(&args, "max_concurrent", seam) {
                        Ok(max_concurrent) => max_concurrent,
                        Err(response) => return response,
                    };
                let step_timeout_secs: Option<u64> =
                    match deserialize_optional_arg(&args, "step_timeout_secs", seam) {
                        Ok(step_timeout_secs) => step_timeout_secs,
                        Err(response) => return response,
                    };
                let tool_timeout_secs: Option<u64> =
                    match deserialize_optional_arg(&args, "tool_timeout_secs", seam) {
                        Ok(tool_timeout_secs) => tool_timeout_secs,
                        Err(response) => return response,
                    };
                let total_timeout_secs: Option<u64> =
                    match deserialize_optional_arg(&args, "total_timeout_secs", seam) {
                        Ok(total_timeout_secs) => total_timeout_secs,
                        Err(response) => return response,
                    };
                let dry_run: bool = match deserialize_optional_arg(&args, "dry_run", seam) {
                    Ok(dry_run) => dry_run.unwrap_or(false),
                    Err(response) => return response,
                };
                let as_of_date: Option<String> =
                    match deserialize_optional_arg(&args, "as_of_date", seam) {
                        Ok(as_of_date) => as_of_date,
                        Err(response) => return response,
                    };
                let request = axagent_harness::WorkflowRunRequest {
                    template_id,
                    input,
                    variables,
                    model_id,
                    provider_id,
                    max_concurrent,
                    step_timeout_secs,
                    tool_timeout_secs,
                    total_timeout_secs,
                    dry_run,
                    as_of_date,
                };
                let Some(provider) = self.registry.get_workflow_run() else {
                    return self.unavailable(seam);
                };
                let owner = self.plugin_id.clone();
                let Some(result) = self.block_on(provider.start(&owner, request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(handle) => to_value("WorkflowRunHandle", &handle),
                    Err(e) => workflow_run_error(e),
                }
            },
            "workflow.run" if op == "status" => {
                let execution_id: String = match deserialize_arg(&args, "execution_id", seam) {
                    Ok(execution_id) => execution_id,
                    Err(response) => return response,
                };
                let include_output: bool =
                    match deserialize_optional_arg(&args, "include_output", seam) {
                        Ok(include_output) => include_output.unwrap_or(false),
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_workflow_run() else {
                    return self.unavailable(seam);
                };
                let owner = self.plugin_id.clone();
                let Some(result) =
                    self.block_on(provider.status(&owner, &execution_id, include_output))
                else {
                    return no_runtime(seam);
                };
                match result {
                    // 查无（含跨 owner、宿主已重启）⇒ JSON `null`，**不是错误**。
                    Ok(view) => to_value("ExecutionView", &view),
                    Err(e) => workflow_run_error(e),
                }
            },
            "workflow.run" if op == "cancel" => {
                let execution_id: String = match deserialize_arg(&args, "execution_id", seam) {
                    Ok(execution_id) => execution_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_workflow_run() else {
                    return self.unavailable(seam);
                };
                let owner = self.plugin_id.clone();
                let Some(result) = self.block_on(provider.cancel(&owner, &execution_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 三态结果**原样**透出：真取消 / 终态幂等 / 查无都正常，不借用错误码。
                    Ok(outcome) => to_value("WorkflowRunCancelOutcome", &outcome),
                    Err(e) => workflow_run_error(e),
                }
            },
            "workflow.run" if op == "list_active" => {
                // `limit` 缺省 50，并夹在 1..=200：插件传超大值不得让宿主一次吐全表。
                let limit: u32 = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(limit) => limit.unwrap_or(50).clamp(1, 200),
                    Err(response) => return response,
                };
                let workflow_id: Option<String> =
                    match deserialize_optional_arg(&args, "workflow_id", seam) {
                        Ok(workflow_id) => workflow_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_workflow_run() else {
                    return self.unavailable(seam);
                };
                let owner = self.plugin_id.clone();
                let page_limit = limit as usize;
                let Some(result) =
                    self.block_on(provider.list_active(&owner, workflow_id.as_deref(), page_limit))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(page) => to_value("WorkflowRunActivePage", &page),
                    Err(e) => workflow_run_error(e),
                }
            },
            // `list_recent` = `list_active` 的同形只读 op，只是页内可含**已结束**的执行
            // （任务中心要答「今天这个插件跑过什么」，只给在跑的会看不见跑完的批次）。
            // 参数、夹档、错误映射都与 `list_active` 逐字一致 —— 两处不一致会让插件
            // 换个 op 就换个口径。终态行的 `finishedAt` 由宿主登记，本层不加工。
            "workflow.run" if op == "list_recent" => {
                let limit: u32 = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(limit) => limit.unwrap_or(50).clamp(1, 200),
                    Err(response) => return response,
                };
                let workflow_id: Option<String> =
                    match deserialize_optional_arg(&args, "workflow_id", seam) {
                        Ok(workflow_id) => workflow_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_workflow_run() else {
                    return self.unavailable(seam);
                };
                let owner = self.plugin_id.clone();
                let page_limit = limit as usize;
                let Some(result) =
                    self.block_on(provider.list_recent(&owner, workflow_id.as_deref(), page_limit))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(page) => to_value("WorkflowRunActivePage", &page),
                    Err(e) => workflow_run_error(e),
                }
            },
            // ── trading.engine 接缝：宿主把独持的交易引擎暴露给插件（ORM 视图不出线） ──
            // op 参数一律 **snake_case**；嵌套 DTO（`TradeValidationRequest` /
            // `TradeExecutionRequest`）的线上键是 **camelCase**（全站约定），在本层
            // **对象原样反序列化**，分派层不做任何键名映射。
            // `execute_trade` 是唯一写 op，`write` 权限闸由 `dispatch_seam` 开头统一把关；
            // 其 `accepted = false`（开关关闭 / 校验不过 / 重复提交）是**正常业务结果**，
            // 原样透出 `reason`，不借用错误码（契约见 harness `trading_engine.rs`）。
            // `as_of` 只出现在会触发行情取数的三个 op（get_positions / validate_trade /
            // execute_trade）上，全部复用 `deserialize_as_of_arg`（含未来日期校验）。
            "trading.engine" if op == "get_positions" => {
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_positions(as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(positions) => to_value("PositionSummary", &positions),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "trading.engine" if op == "get_trades" => {
                let stock_code: Option<String> =
                    match deserialize_optional_arg(&args, "stock_code", seam) {
                        Ok(stock_code) => stock_code,
                        Err(response) => return response,
                    };
                // `limit` 缺省 50；本层**不夹上限** —— 宿主 provider 统一 clamp 到
                // 1..=500（两处各夹一份必然漂移），插件传超大值由宿主收口。
                let limit: u32 = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(limit) => limit.unwrap_or(50),
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_trades(stock_code.as_deref(), limit))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(trades) => to_value("TradeView", &trades),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "trading.engine" if op == "validate_trade" => {
                let request: axagent_harness::TradeValidationRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.validate_trade(request, as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 校验不过（`valid = false`）是正常结果，不是错误。
                    Ok(validation) => to_value("TradeValidation", &validation),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "trading.engine" if op == "execute_trade" => {
                let request: axagent_harness::TradeExecutionRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let as_of = match deserialize_as_of_arg(&args, seam) {
                    Ok(as_of) => as_of,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.execute_trade(request, as_of)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 业务拒绝（`accepted = false` + `reason`）原样透出，不借用错误码；
                    // 宿主已把 DB 写失败收敛为 `Err` ⇒ 这里的 `reason` 不再有 `"write_failed"`。
                    Ok(outcome) => to_value("TradeExecutionOutcome", &outcome),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "trading.engine" if op == "compare_trade" => {
                let trade_id: String = match deserialize_arg(&args, "trade_id", seam) {
                    Ok(trade_id) => trade_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.compare_trade(&trade_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 查无（`Ok(None)`：交易不存在 / 交易前无已完成分析）⇒ JSON `null`，
                    // **不是错误**（与 `jobs.store get` 同口径）。
                    Ok(comparison) => to_value("TradePredictionComparison", &comparison),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            "trading.engine" if op == "trading_enabled" => {
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.trading_enabled()) else {
                    return no_runtime(seam);
                };
                match result {
                    // 具名对象 `{ "enabled": bool }`（照 `jobs.store count` 的 `{"count":n}`
                    // 风格），不返回裸数字/裸布尔。
                    Ok(enabled) => FrameResponse::success(json!({ "enabled": enabled })),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                }
            },
            // ── trading.engine 接缝 · F2 组合与风控四条（#35，#322）──────────────
            // 两条写 op（`update_portfolio_holding` 改持仓表 / `import_portfolio_from_vlm`
            // 批量落库）已在 `required_permission_for` 登记 `Write`；两条读/纯计算面
            // （`get_position_limits` / `run_stress_test`）不登记——读缝不加闸。
            // `run_stress_test` 的执行体（N+1 网络取数）整体留宿主，长任务不搬进帧；
            // `get_position_limits` 读全局面板变量，只在宿主进程内有效。四条签名与
            // `stock.analysis` 同款（Value 透传），入参嵌 `{"request": {...}}` 层。
            "trading.engine" if op == "get_position_limits" => {
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.get_position_limits(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "trading.engine" if op == "update_portfolio_holding" => {
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.update_portfolio_holding(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "trading.engine" if op == "import_portfolio_from_vlm" => {
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.import_portfolio_from_vlm(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "trading.engine" if op == "run_stress_test" => {
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.run_stress_test(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── trading.engine 接缝 · #323 T2 五命令批迁（子批④）─────────────────
            // 组合回测配置校验（宿主 `portfolio_backtest_run` 退役后的取数面）。读 op
            // （纯计算，不落库、不取行情）⇒ `required_permission_for` 不登记（与
            // `get_position_limits` / `run_stress_test` 同口径）。op 名与命令名不同形：
            // op 叫 `validate_portfolio_config`（trait 方法名）。入参嵌
            // `{"request": {"config_json": str}}` 透传 —— `config_json` 字符串是反序列化
            // 形状的唯一权威（`quant::PortfolioConfig`），本层不做任何键名映射。
            "trading.engine" if op == "validate_portfolio_config" => {
                let Some(provider) = self.registry.get_trading_engine() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.validate_portfolio_config(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── stock.monitor 接缝：宿主把独持的 RealtimeMonitor + CrossStockSignalAggregator
            // 暴露给插件（方向与 market.quote 同类：宿主 → 插件，见上方先例注释） ──
            // op 参数一律 **snake_case**；嵌套 DTO（`AlertUpsertRequest` / `TZeroConfig` /
            // `AggregatorConfig`）的线上键是 **camelCase**（全站约定），在本层**对象原样
            // 反序列化**，分派层不做任何键名映射。
            // 全部 op **不收 `as_of`**：读写的是内存 / DB / 宿主轮询循环已有的读数（契约见
            // harness `stock_monitor.rs` 模块文档）。
            // ⚠ **op 名 ≠ 方法名**：发布的 op 叫 `get_t0_config`，trait 方法叫 `t0_config`；
            // 其余 8 个一一对应。四个写 op（set_t0_config / upsert_alert / remove_alert /
            // set_aggregator_config）的 `write` 权限闸由 `dispatch_seam` 开头统一把关。
            "stock.monitor" if op == "status" => {
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.status()) else {
                    return no_runtime(seam);
                };
                match result {
                    // 「引擎未装配」是**正常响应**（`assembled = false`，计数全 0），不报
                    // `SEAM_PROVIDER_UNAVAILABLE` —— 缺席本身是要如实暴露的状态。
                    Ok(view) => to_value("MonitorStatusView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.monitor" if op == "list_configs" => {
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_configs()) else {
                    return no_runtime(seam);
                };
                match result {
                    // 「引擎实际在盯哪些条」的真相；`Option` 阈值按 serde 声明透出 `null`。
                    Ok(configs) => to_value("MonitorConfig", &configs),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.monitor" if op == "get_t0_config" => {
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                // op 叫 `get_t0_config`，方法叫 `t0_config`（见本段开头 ⚠ 注释）。
                let Some(result) = self.block_on(provider.t0_config()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(config) => to_value("TZeroConfig", &config),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 写 op 的取参顺序照 trading.engine `validate_trade` 先例：先反序列化、后取
            // provider —— 非法入参是调用方的错误，不该被「provider 暂不可用」掩盖。
            "stock.monitor" if op == "set_t0_config" => {
                // 全量结构体（照命令层 `set_t0_config`），不是 patch；缺任一字段 ⇒ serde
                // reject ⇒ `SEAM_CALL_INVALID_ARGS`（`TZeroConfig` 刻意不带 `serde(default)`）。
                let config: axagent_harness::TZeroConfig =
                    match deserialize_arg(&args, "config", seam) {
                        Ok(config) => config,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.set_t0_config(config)) else {
                    return no_runtime(seam);
                };
                match result {
                    // clamp（阈值 > 0、`minIntervalMinutes ≥ 1`）在宿主 provider 侧（裁定 J4/N4），
                    // 分派层不重复实现一套数字。
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.monitor" if op == "upsert_alert" => {
                // ⚠ 裁定 N2：args 是**嵌一层**的 `{ "request": {stockCode, stockName, alertType,
                // threshold} }` 形态（照 trading.engine `deserialize_arg(&args, "request", seam)`
                // 先例），**不支持平铺四个字段** —— 平铺时缺 `request` 键即 invalid_args。
                let request: axagent_harness::AlertUpsertRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.upsert_alert(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 三判据原样透出：`wired = false`（库里有行但不会被盯）是**正常结果**，
                    // 不借用错误码；`alert_type` 白名单拒绝在宿主 provider 侧（先拒后归一）。
                    Ok(outcome) => to_value("AlertUpsertOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.monitor" if op == "remove_alert" => {
                let alert_id: String = match deserialize_arg(&args, "alert_id", seam) {
                    Ok(alert_id) => alert_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.remove_alert(&alert_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 行不存在 ⇒ `{dbDeleted:false, memoryRemoved:false}`，**不是错误**
                    // （照 jobs.store get 的「查无即 null」口径）。
                    Ok(outcome) => to_value("RemoveAlertOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.monitor" if op == "aggregator_config" => {
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.aggregator_config()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(config) => to_value("AggregatorConfig", &config),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.monitor" if op == "set_aggregator_config" => {
                let config: axagent_harness::AggregatorConfig =
                    match deserialize_arg(&args, "config", seam) {
                        Ok(config) => config,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.set_aggregator_config(config)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 全量结构体；越界拒收、不静默夹取，clamp 区间以 harness
                    // `AGGREGATOR_*` 常量为唯一权威（宿主 provider 侧执行）。
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.monitor" if op == "aggregator_buffer" => {
                let Some(provider) = self.registry.get_stock_monitor() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.aggregator_buffer()) else {
                    return no_runtime(seam);
                };
                match result {
                    // 「窗口内几只同向」由消费方用 `SignalType::direction()` 自数（裁定 J7，
                    // 首批不发 aggregator_stats）。
                    Ok(signals) => to_value("StockSignal", &signals),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── quote.watcher 接缝：宿主把 RealTimeQuoteWatcher 的推送集合管理面暴露给插件 ──
            // 轮询本体（start / stop）不过线（跨插件 DoS 面 + 句柄跨不过帧协议，契约见
            // harness `quote_watcher.rs` 模块文档）；`watch` / `unwatch` 的 `write` 权限闸
            // 由 `dispatch_seam` 开头统一把关。同样不收 `as_of`。
            "quote.watcher" if op == "list_watched" => {
                let Some(provider) = self.registry.get_quote_watcher() else {
                    return self.unavailable(seam);
                };
                // op 叫 `list_watched`，方法叫 `watched_codes`（provider 侧须 sort，裁定 J6）。
                let Some(result) = self.block_on(provider.watched_codes()) else {
                    return no_runtime(seam);
                };
                match result {
                    // 具名对象 `{ "codes": [...] }`（照 jobs.store count 的 `{"count":n}`
                    // 风格），不返回裸数组。
                    Ok(codes) => FrameResponse::success(json!({ "codes": codes })),
                    Err(e) => seam_provider_error(e),
                }
            },
            "quote.watcher" if op == "watch" => {
                let codes: Vec<String> = match deserialize_arg(&args, "codes", seam) {
                    Ok(codes) => codes,
                    Err(response) => return response,
                };
                // ⚠ `priority` **严格解析**（必填）：serde 只认 `active` / `background`。
                // 命令层历史上还认 `bg` 别名并把未知串静默回落高频档 —— 接缝**不继承**这个
                // 宽容（插件侧没有「人看着面板选」兜底），`bg` 一律落 `SEAM_CALL_INVALID_ARGS`。
                let priority: axagent_harness::WatchPriority =
                    match deserialize_arg(&args, "priority", seam) {
                        Ok(priority) => priority,
                        Err(response) => return response,
                    };
                let replace: bool = match deserialize_optional_arg(&args, "replace", seam) {
                    Ok(replace) => replace.unwrap_or(false),
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_quote_watcher() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.watch(&codes, priority, replace)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 数量闸（单次 ≤ WATCH_BATCH_LIMIT、总量 ≤ WATCH_SET_LIMIT）超限**截断
                    // 不报错**：`applied < requested` 是合法结果，两个数原样暴露（裁定 J5）。
                    Ok(outcome) => to_value("WatchOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            "quote.watcher" if op == "unwatch" => {
                let codes: Vec<String> = match deserialize_arg(&args, "codes", seam) {
                    Ok(codes) => codes,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_quote_watcher() else {
                    return self.unavailable(seam);
                };
                // `unwatch` 不受单次闸约束（退订只让集合变小），`applied` 只数真摘掉的。
                let Some(result) = self.block_on(provider.unwatch(&codes)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => to_value("WatchOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── knowledge.vector 接缝：宿主把独持的 VectorStore + embedding 链暴露给插件 ──
            // 方向与 market.quote / stock.monitor 同类（宿主 → 插件，见上方先例注释）：只走入站
            // 分派，**不进** `SUPPORTED_REMOTE_SEAMS`（那是「插件提供给宿主」的清单）。
            // 边界总则「只传文本，embedding 归宿主」：`VectorIndexRequest` 只有三字段，
            // `embeddingProvider` / `dimensions` 都不是入参（PLAN §56.2-1，契约见 harness
            // `knowledge_vector.rs` 模块头）；四个 op **一律不收 `as_of`**。
            // ⚠ 硬闸数字（`KNOWLEDGE_CONTENT_MAX_BYTES` / `KNOWLEDGE_TOP_K_RANGE` /
            // `KNOWLEDGE_SNIPPET_MAX_CHARS`）与 snippet 裁剪、`score` 方向（L2 距离，越小越
            // 相似）全在契约层 + 宿主 provider 侧，**分派层一个数字都不写**（裁定 N4：两处各夹
            // 一份必然漂移）；provider 回的 `Err` 走 `seam_provider_error` 单点映射：
            // `Validation ⇒ SEAM_CALL_INVALID_ARGS`（契约按符号承诺；跨批行号会漂，故不引号），
            // 其余变体（含 trait 默认的 `Provider("…未实现…")`）仍 `PLUGIN_CALL_FAILED`。
            // ⚠ **op 名 ≠ 方法名**：发布的 op 叫 `get_collection_info`，trait 方法叫
            // `collection_info()`（PLAN §58.3-6 定案，先例 `get_t0_config` → `t0_config`、
            // `list_watched` → `watched_codes`）；其余三个 op 与方法名一一对应。
            "knowledge.vector" if op == "search" => {
                let Some(provider) = self.registry.get_knowledge_vector() else {
                    return self.unavailable(seam);
                };
                // ⚠ 裁定 N2 / §58.3-3：args 是**嵌一层**的 `{ "request": {knowledgeBaseId,
                // query, topK, minSimilarity?, docIds?} }`，**不支持平铺** —— 平铺时缺
                // `request` 键即 `SEAM_CALL_INVALID_ARGS`。
                let request: axagent_harness::VectorSearchRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(result) = self.block_on(provider.search(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 具名 Outcome 直接序列化（`{"results": [...]}`）；命中 0 条 ⇒ 空数组，
                    // **不是错误**（协议层无通用 NOT_FOUND，先例 `jobs.store`）。
                    Ok(outcome) => to_value("VectorSearchOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            "knowledge.vector" if op == "keyword_search" => {
                let Some(provider) = self.registry.get_knowledge_vector() else {
                    return self.unavailable(seam);
                };
                // 同样嵌一层（键名 `request`）；`KeywordSearchRequest` 是「宿主唯一的原生
                // 收文本的检索口」，零模型成本 ⇒ 插件想省钱得**显式**选这个 op，本层不替它猜。
                let request: axagent_harness::KeywordSearchRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(result) = self.block_on(provider.keyword_search(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 与 `search` 共用 `VectorSearchOutcome`；`score` 口径同 `fts_search`，
                    // 消费方不得按 op 分别假设方向。
                    Ok(outcome) => to_value("VectorSearchOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 本接缝**唯一**的写 op：`write` 权限闸由 `dispatch_seam` 开头统一把关（§58.3-4）。
            "knowledge.vector" if op == "index_text" => {
                // 取参顺序照 stock.monitor `upsert_alert` / trading.engine `validate_trade`
                // 先例：先反序列化、后取 provider —— 非法入参是调用方的错误，不该被
                // 「provider 暂不可用」掩盖。
                let request: axagent_harness::VectorIndexRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_knowledge_vector() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.index_text(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    // `chunk_count = 0`（正文切不出块）是**合法结果**，不是错误；正文超限、
                    // 集合不存在（§56.2-2「不得隐式建集合」）都在宿主 provider 侧拒收。
                    Ok(outcome) => to_value("VectorIndexOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            "knowledge.vector" if op == "get_collection_info" => {
                let Some(provider) = self.registry.get_knowledge_vector() else {
                    return self.unavailable(seam);
                };
                // 标量键（不是嵌一层）：JSON 侧 `collection_id`，trait 形参同名 `collection_id`。
                let collection_id: String = match deserialize_arg(&args, "collection_id", seam) {
                    Ok(collection_id) => collection_id,
                    Err(response) => return response,
                };
                // op 叫 `get_collection_info`，方法叫 `collection_info`（见本段开头 ⚠ 注释）。
                let Some(result) = self.block_on(provider.collection_info(&collection_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 集合不存在 ⇒ `Ok` + `{exists:false, dimensions:null}`，**不是错误**
                    // （照 `jobs.store get` 的「查无即 null」口径；插件据此先探再写）。
                    Ok(view) => to_value("CollectionInfoView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── trajectory.store 接缝：宿主把独持的 TrajectoryStorage 的**读面**暴露给插件 ──
            // 首批四个 op 全是读（PLAN §56.1-Q4：`append` 不给）⇒ `required_permission_for`
            // 里一条条目都没有；四个 op 同样**不收 `as_of`**。
            // 视图裁剪（`steps` / `rewards` / `patterns` / 会话原文整族不过线）与
            // `TRAJECTORY_LIST_LIMIT_RANGE` 钳位、`summary_snippet` 截断都在宿主 provider 侧，
            // `Trajectory` → `TrajectorySummaryView` 的映射点**唯一**（禁区 12），本层只透传
            // 契约已经给好的具名 Outcome / View。
            "trajectory.store" if op == "list" => {
                let Some(provider) = self.registry.get_trajectory_store() else {
                    return self.unavailable(seam);
                };
                // `limit` 是**可选标量**：缺失 / `null` 都合法（=「由宿主取缺省并钳位」），
                // 故走 `deserialize_optional_arg`；给了非数字才落 `SEAM_CALL_INVALID_ARGS`。
                let limit: Option<usize> = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.list(limit)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 具名对象 `{"trajectories": [...]}`，不返回裸数组（契约已给 Outcome，
                    // 本层不再自己包一层 json）。
                    Ok(outcome) => to_value("TrajectoryListOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            "trajectory.store" if op == "get" => {
                let Some(provider) = self.registry.get_trajectory_store() else {
                    return self.unavailable(seam);
                };
                // JSON 侧键名 `trajectory_id`；trait 形参名是 `id`（Rust 侧标识符，不是线上键）。
                let trajectory_id: String = match deserialize_arg(&args, "trajectory_id", seam) {
                    Ok(trajectory_id) => trajectory_id,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.get(&trajectory_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 查无 ⇒ `Ok(None)` ⇒ 帧里是 `null`，**不是错误**、不借错误码（协议层无
                    // 通用 NOT_FOUND，先例 `jobs.store` 的 `get`）。
                    Ok(view) => to_value("TrajectorySummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "trajectory.store" if op == "query" => {
                let Some(provider) = self.registry.get_trajectory_store() else {
                    return self.unavailable(seam);
                };
                // 嵌一层（键名 `request`）：DTO 是 `TrajectoryQueryView`（camelCase +
                // `sinceMs` / `untilMs` epoch 毫秒），**不是** `trajectory_types::TrajectoryQuery`
                // 原形（§56.1-Q6：原形无 `rename_all`、`time_range` 是元组，一字不动）。
                let filter: axagent_harness::TrajectoryQueryView =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(filter) => filter,
                        Err(response) => return response,
                    };
                let Some(result) = self.block_on(provider.query(filter)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 零命中 ⇒ `trajectories` 空数组，**不是错误**。
                    Ok(outcome) => to_value("TrajectoryQueryOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            "trajectory.store" if op == "count" => {
                let Some(provider) = self.registry.get_trajectory_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.count()) else {
                    return no_runtime(seam);
                };
                match result {
                    // 具名对象 `{"count": n}`，不返裸值（空库的 `0` 是合法值）；区间校验、
                    // `limit` 钳位一律由 provider 侧用 `TRAJECTORY_LIST_LIMIT_RANGE` 执行。
                    Ok(outcome) => to_value("TrajectoryCountOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── analysis.record 接缝：宿主把 `stock_analyses` 的**列表读面**暴露给插件 ──
            // 为什么不用 `workflow.run::list_recent`：那条按 owner 隔离、只看插件自己发起的
            // **执行**；宿主工作台 / 定时任务发起的单股分析记录在记录表里（契约模块头有判据）。
            // 唯一 op 是读面 ⇒ `required_permission_for` 里**一条都不许加**（多登记一条会让
            // 只读插件被无端拒掉，同 `trajectory.store` 的处置）。视图裁剪与 `limit` 区间闸
            // （`ANALYSIS_RECORD_LIST_LIMIT_RANGE`）都在宿主 provider 侧，本层只透传契约 DTO。
            "analysis.record" if op == "list" => {
                let Some(provider) = self.registry.get_analysis_record() else {
                    return self.unavailable(seam);
                };
                // `limit` / `offset` 都是**可选标量**：缺失 / `null` 都合法（=「宿主取缺省」），
                // 故走 `deserialize_optional_arg`；给了非数字才落 `SEAM_CALL_INVALID_ARGS`。
                let limit: Option<usize> = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let offset: Option<usize> = match deserialize_optional_arg(&args, "offset", seam) {
                    Ok(offset) => offset,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.list(limit, offset)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 具名对象 `{"records": [...], "total": n}`：空表是空数组 + total 0，
                    // **不是错误**（先例 `jobs.store` 的 `count` 口径）。
                    Ok(outcome) => to_value("AnalysisRecordListOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── stock.analysis 接缝：宿主把股票域三条确定性数据面暴露给插件 ──
            // 方向同 `analysis.record` / `opc.store`（宿主 → 插件），只走入站分派，
            // **不进** `SUPPORTED_REMOTE_SEAMS`（那是「插件提供给宿主」的清单）。
            // 三个 op 的 payload 与出参**整块 `Value` 透传**（形状权威在 `analysis-engine`，
            // harness 是 foundation 不上收 DTO，判据见 `stock_analysis` 模块头）⇒
            // 本层不反序列化、不做字段级改名，避免出现第二套形状定义（禁区 12）。
            // 权限闸**只加写 op**：`batch_upsert_mainlines => Write`，两个读 op 仍 `None`。
            "stock.analysis" if op == "industry_constituents" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                // 入参是嵌一层 `{"request": {...}}`（与 `opc.store` 的 `*ListQuery` 同形），
                // 缺键时按 `Null` 透传 ⇒ 由域包侧的 `serde` 报缺料（那才是唯一的形状权威）。
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.industry_constituents(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "stock_factor" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.stock_factor(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 写 op：`write` 权限闸由 `dispatch_seam` 开头统一把关（见
            // `required_permission_for(("stock.analysis", "batch_upsert_mainlines"))`）。
            // ⚠「唯一」自批 1a 起不成立：另九个 cron 管理写 op 同批登记 `Write`。
            "stock.analysis" if op == "batch_upsert_mainlines" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.batch_upsert_mainlines(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 读 op（#304 C3）：最近 N 天市场主线 —— 宿主 `market_mainline_list_recent`
            // 退役后的取数面；`required_permission_for` 不登记（读缝不加闸）。
            "stock.analysis" if op == "list_recent_mainlines" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.list_recent_mainlines(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── stock.analysis 接缝 · F1 余三条（#35）：快照读面 + 采集触发面 ──
            // `list_financial_snapshots` 读宿主本地 `financial_snapshots` 表原始行（与
            // `market.quote::get_financials` 的 vendor 财报**不等价**），读 op 无闸；
            // `sweep_daily_snapshots` 是写 op（见 `required_permission_for`），触发口
            // 语义 —— 执行体留宿主（后台定时服务同源复用），长任务不搬进 worker 帧。
            // 入参 snake_case 接缝风格（同 `opc.store`）：`{stock_code, start?, end?}` /
            // `{}`；出参 = 实体行数组 / 采集总结串。
            "stock.analysis" if op == "list_financial_snapshots" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.list_financial_snapshots(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "sweep_daily_snapshots" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.sweep_daily_snapshots(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── stock.analysis 接缝 · F6 分析师别名族六条（#35，#322）─────────────
            // `evolve_analyst` / `save_analyst_feedback` 是写 op（进化和反馈都落库），
            // 已在 `required_permission_for` 登记 `Write`；四条 get_* 是读 ⇒ 不登记。
            // 实现权威（evolve_node / get_node_evolution_status / 反馈读写）在宿主 +
            // analysis-engine，入参嵌 `{"request": {...}}` 透传；别名入参形状由插件
            // 命名命令层逐字保留（op 帧收 snake_case 归一后的形状）。
            "stock.analysis" if op == "evolve_analyst" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.evolve_analyst(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "get_analyst_evolution_status" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.get_analyst_evolution_status(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "get_node_evolution_status" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.get_node_evolution_status(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "save_analyst_feedback" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.save_analyst_feedback(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "get_analyst_feedbacks" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.get_analyst_feedbacks(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "get_analyst_feedback_stats" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.get_analyst_feedback_stats(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── stock.analysis 接缝 · T2 试点批（#248）：四条宿主命令的取数面 ──
            // 方向与上面三条同族（宿主 → 插件），全部只读/纯计算 ⇒ `required_permission_for`
            // **一条都不登记**（给读缝加闸会让只读插件被无端拒掉）。入参嵌一层
            // `{"request": {...}}` 透传，形状权威在主壳 commands 层（pipeline 行是手拼
            // `json!`、进化计划是 dao 模型、DES 仿真是 `commands/wf_des.rs` 的 DTO）。
            "stock.analysis" if op == "pipeline_history" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.pipeline_history(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "pipeline_run_detail" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.pipeline_run_detail(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "pending_evolution_plans" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.pending_evolution_plans(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "des_integration" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.des_integration(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── stock.analysis 接缝 · 批 1a（#35）：十二股票 cron 管理命令的管理面 ──
            // 方向与上面四条同族（宿主 → 插件）。与试点批的唯一差别是**有写 op**：
            // 3 create + 3 toggle + 3 delete 改宿主 `CronJobStore` ⇒ 权限表登记为 `Write`；
            // 3 个 list 仍 `None`。入参同样嵌一层 `{"request": {...}}` 透传，形状权威
            // 在主壳 `commands/recommendation_cron.rs` / `pool_scan.rs` 的无 State 核心。
            "stock.analysis" if op == "create_recommendation_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.create_recommendation_cron(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "list_recommendation_crons" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.list_recommendation_crons(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "toggle_recommendation_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.toggle_recommendation_cron(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "delete_recommendation_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.delete_recommendation_cron(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "create_trend_screening_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.create_trend_screening_cron(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "list_trend_screening_crons" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.list_trend_screening_crons(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "toggle_trend_screening_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.toggle_trend_screening_cron(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "delete_trend_screening_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.delete_trend_screening_cron(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "create_pool_scan_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.create_pool_scan_cron(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "list_pool_scan_crons" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.list_pool_scan_crons(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "toggle_pool_scan_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.toggle_pool_scan_cron(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "delete_pool_scan_cron" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.delete_pool_scan_cron(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── stock.analysis 接缝 · #323 T2 五命令批迁（子批②③）───────────────
            // 宿主 `save_neodata_token` / `query_decision_backtest` 退役后的接缝面。
            // 前者是写 op（脚本缓存 + 共享内存 + 模板变量三段持久效应），已在
            // `required_permission_for` 登记 `Write`；后者纯读 ⇒ 不登记。两条实现体
            // 都只在宿主进程内（token 缓存文件 / `neodata_token` 共享内存 /
            // `stock_analyses` 表查询），入参嵌 `{"request": {...}}` 透传。
            "stock.analysis" if op == "save_neodata_token" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.save_neodata_token(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "query_decision_backtest" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.query_decision_backtest(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── stock.analysis 接缝 · #324 T2 F7 回测验证三条迁 ──────────────────
            // 宿主 `run_decision_backtest` / `list_decision_validations` /
            // `compute_validation_report`（backtest_validation.rs）退役后的接缝面。
            // 写 op `run_decision_backtest` 已在 `required_permission_for` 登记
            // `Write`（执行体留宿主：cron 与 CLI 共用同一实现）；两条读不登记。
            // 三条出参 = 宿主 DTO 序列化整块透传，本层不做键名映射。
            "stock.analysis" if op == "run_decision_backtest" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.run_decision_backtest(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "list_decision_validations" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.list_decision_validations(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "stock.analysis" if op == "compute_validation_report" => {
                let Some(provider) = self.registry.get_stock_analysis() else {
                    return self.unavailable(seam);
                };
                let request = args.get("request").cloned().unwrap_or(Value::Null);
                let Some(result) = self.block_on(provider.compute_validation_report(request))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => FrameResponse::success(view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── opc.store 接缝：宿主把 `opc_blog_posts` 的列表读面 + 三个写面暴露给插件 ──
            // 方向与 analysis.record 同类（宿主 → 插件，见上方先例注释）：只走入站分派，
            // **不进** `SUPPORTED_REMOTE_SEAMS`（那是「插件提供给宿主」的清单）。
            // 权限闸**只加写 op**：`("opc.store", publish/delete/create) => Write`，读 op 仍 `None`
            // （给读缝加闸会让只读插件被无端拒掉 —— 两侧都有用例钉住）。
            // 列裁剪（`content` 连 SELECT 都没有）、`tags_json` 解码、`limit` 区间闸
            // （`OPC_BLOG_POST_LIST_LIMIT_RANGE`）与 `id` 非空闸全在契约层 + 宿主 provider / dao 侧，
            // 本层只透传契约 DTO、**一个数字都不写**。
            "opc.store" if op == "list_blog_posts" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                // `limit` / `offset` 都是**可选标量**：缺失 / `null` 都合法（=「宿主取缺省」），
                // 故走 `deserialize_optional_arg`；给了非数字才落 `SEAM_CALL_INVALID_ARGS`。
                let limit: Option<usize> = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let offset: Option<usize> = match deserialize_optional_arg(&args, "offset", seam) {
                    Ok(offset) => offset,
                    Err(response) => return response,
                };
                let Some(result) = self.block_on(provider.list_blog_posts(limit, offset)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 具名对象 `{"posts": [...], "total": n}`：空表是空数组 + total 0，
                    // **不是错误**（先例 `jobs.store` 的 `count` 口径）。
                    Ok(outcome) => to_value("BlogPostListOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // `opc.store` 的八个只读查询 op（#36 批 1）：入参是**嵌一层**的 `{"request": {...}}`
            // （契约 DTO `*ListQuery`，camelCase + `default` —— 缺键即 `None`），出参是
            // **裸数组**（逐字透传 `*SummaryView`，出口不做任何包装）。
            // 空表 = 空数组，**不是错误**（同 `list_blog_posts` 的空页口径）。
            "opc.store" if op == "list_invoices" => {
                let query: axagent_harness::InvoiceListQuery =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(query) => query,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_invoices(query)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<InvoiceSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_customers" => {
                let query: axagent_harness::CustomerListQuery =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(query) => query,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_customers(query)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<CustomerSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_projects" => {
                let query: axagent_harness::ProjectListQuery =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(query) => query,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_projects(query)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<ProjectSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_landing_pages" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_landing_pages()) else {
                    return no_runtime(seam);
                };
                match result {
                    // 空表 = 空数组，不是错误。
                    Ok(rows) => to_value("Vec<LandingPageSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_contacts" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_contacts()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<ContactSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_kpis" => {
                let query: axagent_harness::KpiListQuery =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(query) => query,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_kpis(query)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<KpiSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_content_assets" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_content_assets()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<ContentAssetSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_publish_schedules" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_publish_schedules()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<PublishScheduleSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 看板总览：零入参。宿主按整表聚合（不是列表页加总），见 provider 侧注释。
            "opc.store" if op == "dashboard_summary" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.dashboard_summary()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(summary) => to_value("DashboardSummaryView", &summary),
                    Err(e) => seam_provider_error(e),
                }
            },
            // `opc.store` 的写 op：入参只有 `id`（**不带正文**，也不接受正文），出参是契约的
            // 两态枚举。三处判据都跟着 `jobs.store` 的 `remove`：
            // ① `deserialize_arg` 只认 `id` 这一个键，缺键落 `SEAM_CALL_INVALID_ARGS`；
            // ② 空 / 全空白 id 交给 **provider 侧**的闸（分派层不另判一套 —— 读 op 的 limit 闸
            //    同样在 provider，两处各判一半迟早漂移）；
            // ③ `NotFound` 走 payload（`"not_found"`），**不借错误码**：权限闸只管「能不能调」，
            //    不管「这一行在不在」。
            "opc.store" if op == "publish_blog_post" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.publish_blog_post(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => to_value("PublishBlogPostOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // `opc.store` 的删除写 op：入参同样只有 `id`，出参是**三态**枚举
            // （`RemoveBlogPostOutcome`：`"removed"` / `"not_found"` / `"protected"`）。
            //
            // 三档里只有「真故障」走错误码：`NotFound`（本来就没有这篇）与 `Protected`
            // （受保护 id，未删除）都是**业务读数**，逐字透出，分派层不加包装、不换取值
            // （禁区 13 那条判据同样管枚举；判据与 `jobs.store` 的 `remove` 完全同族）。
            "opc.store" if op == "delete_blog_post" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_blog_post(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => to_value("RemoveBlogPostOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // `opc.store` 的创建写 op：入参是**嵌一层**的 `{ "request": {title, slug, excerpt,
            // content, tags} }`（先例 `knowledge.vector` 的 `request` 形态 —— 这是一个 DTO 而非
            // 散标量；平铺时会因缺 `request` 键落 `SEAM_CALL_INVALID_ARGS`，不做兼容翻译）。
            //
            // 正文的**字节预算**与 `slug` 非空都在 provider 侧判（数字与语义都不在这一层），
            // 越限 ⇒ `AxAgentError::Validation` ⇒ `seam_provider_error` 落
            // `SEAM_CALL_INVALID_ARGS`（与分派层自己的参数错误**同码**，插件侧只认一个码）。
            "opc.store" if op == "create_blog_post" => {
                let input: axagent_harness::CreateBlogPostInput =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(input) => input,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_blog_post(input)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 出参是列裁剪视图（与列表行同一套 10 个 camelCase 键，正文不过线）。
                    Ok(view) => to_value("BlogPostView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #36 批 2：四条 id / 枚举写 op ──────────────────────────────────
            //
            // 出参一律是契约的**外部标记枚举**：带数据的写成 `{"transitioned": {…}}` /
            // `{"updated": {…}}`，两态写 op 是 `"removed"` / `"not_found"`。「缺行」是正常
            // 业务读数，**不借错误码**（判据同 blog 三件）。
            //
            // 发票状态变更：入参是**散标量** `{"id", "target"}`（`target` 是枚举串，语义
            // 合法性由 provider 判 —— 非法串落 `SEAM_CALL_INVALID_ARGS`，非法转换落
            // `PLUGIN_CALL_FAILED`，两层不混）。
            "opc.store" if op == "transition_invoice" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let target: String = match deserialize_arg(&args, "target", seam) {
                    Ok(target) => target,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.transition_invoice(id, target)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => to_value("TransitionInvoiceOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 内容资产更新：`id` 散标量 + **嵌一层**的 `request` DTO（五个可选字段，
            // `None` = 该列不动）。DTO 必须嵌层 —— 与 `create_blog_post` 同一条判据。
            "opc.store" if op == "update_content_asset" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let input: axagent_harness::UpdateContentAssetInput =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(input) => input,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.update_content_asset(id, input)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => to_value("UpdateContentAssetOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 内容资产删除：入参只有 `id`，出参两态（没有「受保护 id」这一档）。
            "opc.store" if op == "delete_content_asset" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_content_asset(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => to_value("RemoveContentAssetOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 发布计划取消：**硬删**（不是状态位改写），出参两态。
            "opc.store" if op == "cancel_publish_schedule" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.cancel_publish_schedule(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => to_value("RemovePublishScheduleOutcome", &outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #36 批 3：创建 / 流程动作（八个写 op + 一条读 op）──
            //
            // 入参一律是**嵌一层**的 `{"request": {…}}`（契约写 DTO，camelCase +
            // `default`）；出参是创建后的列裁剪视图 / 视图数组，逐字透传。
            // 缺省值与语义闸都在 provider（本层只反序列化契约 DTO）。
            "opc.store" if op == "create_invoice" => {
                let request: axagent_harness::CreateInvoiceRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_invoice(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("InvoiceSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "create_customer" => {
                let request: axagent_harness::CreateCustomerRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_customer(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("CustomerSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "create_project" => {
                let request: axagent_harness::CreateProjectRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_project(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("ProjectSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "add_milestone" => {
                let request: axagent_harness::AddMilestoneRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.add_milestone(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("ProjectSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "create_landing_page" => {
                let request: axagent_harness::CreateLandingPageRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_landing_page(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("LandingPageSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // `update_customer` 是**散标量 + 嵌一层**混合：`id` 单独收（同
            // `transition_invoice`），正文收 `{"request": {…}}`。契约
            // `UpdateCustomerRequest` 的可空列是双层 `Option<Option<T>>`
            // （外层 None=不动、Some(None)=清空、Some(Some(v))=写入），与域包
            // `UpdateCustomerInput` 逐字段同构；`source`/`tags` 域包无更新位，
            // 契约刻意不摆。
            "opc.store" if op == "update_customer" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let request: axagent_harness::UpdateCustomerRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.update_customer(id, request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("CustomerSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 删除 op：纯 `id` 入参（同 `delete_blog_post`）；成功无载荷 ⇒
            // `Value::Null`（先例 platform.adapter 的 start/stop）。⚠
            // `delete_invoice` 删的是**域包发票**（opc_invoices 表）——宿主同名
            // 命令 `opc_delete_invoice` 删的是交付域发票，两套体系同名不同义。
            "opc.store" if op == "delete_customer" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_customer(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "delete_invoice" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_invoice(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #32 批 B：项目详情 / 更新 / 删除 / 里程碑完成 ─────────────────
            // `get_project` 是**读** op（`required_permission_for` 里刻意不登记它）：
            // 入参是散标量 `id`（不套 `request` 壳，同 `publish_blog_post`）；出参是详情视图
            // （摘要 + 描述 / 备注 / 里程碑明细）。
            "opc.store" if op == "get_project" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_project(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("ProjectDetailView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 更新是「散标量 + 嵌一层」混合（同 `update_customer`）：`id` 单收，可选字段进
            // `request`（接缝 DTO 是双层 `Option`，缺席 = 不动由 provider 判）。
            "opc.store" if op == "update_project" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let request: axagent_harness::UpdateProjectRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.update_project(id, request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("ProjectDetailView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 删除项目：纯 `id` 入参（同 `delete_customer`）；成功无载荷 ⇒ `Value::Null`。
            "opc.store" if op == "delete_project" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_project(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 完成里程碑：两个散标量（`projectId` / `milestoneId`，同 `transition_invoice`
            // 的 `id` + `target` 形态）；出参是完成后的详情视图，前端据此原地刷新时间线。
            "opc.store" if op == "complete_milestone" => {
                let project_id: String = match deserialize_arg(&args, "projectId", seam) {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                let milestone_id: String = match deserialize_arg(&args, "milestoneId", seam) {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.complete_milestone(project_id, milestone_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("ProjectDetailView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 发布落地页：纯 `id` 入参（同 `delete_project`）；出参是发布后的摘要视图
            // （`published` 已翻转）。缺行是域服务 `NotFound` 透传 ⇒ `Err(Provider)`。
            "opc.store" if op == "publish_landing_page" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.publish_landing_page(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("LandingPageSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 删除落地页：纯 `id` 入参（同 `delete_project`）；成功无载荷 ⇒ `Value::Null`。
            "opc.store" if op == "delete_landing_page" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_landing_page(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 标记联系提交已读：纯 `id` 入参（同 `delete_project`）；成功无载荷 ⇒ `Value::Null`。
            "opc.store" if op == "mark_contact_read" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.mark_contact_read(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 报表是**读** op：标量入参 `period`（不套 `request` 壳，同 `list_blog_posts`
            // 的 `limit` / `offset`），不设权限闸。
            "opc.store" if op == "get_financial_report" => {
                let period: String = match deserialize_arg(&args, "period", seam) {
                    Ok(period) => period,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_financial_report(period)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("FinancialReportView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "create_content_asset" => {
                let request: axagent_harness::CreateContentAssetRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_content_asset(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("ContentAssetSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "create_publish_schedule" => {
                let request: axagent_harness::CreatePublishScheduleRequest =
                    match deserialize_arg(&args, "request", seam) {
                        Ok(request) => request,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_publish_schedule(request)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("PublishScheduleSummaryView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "process_due_schedules" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.process_due_schedules()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<PublishScheduleSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 1：交付域五 op（三写两读）──────────────────────────
            //
            // 与域包发票 op（`list_invoices` / `transition_invoice` / `delete_invoice`）
            // **同名不同义**：这里只碰 `lead_id IS NOT NULL` 的交付来源行。
            // 入参一律散标量（`status` / `lead_id` / `invoice_id` / `id`），`input` 用契约 DTO
            // 原样透传 —— 三个写 op 已在 `required_permission_for` 登记 `Write`。
            "opc.store" if op == "list_delivery_invoices" => {
                let status: Option<String> = match deserialize_optional_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_delivery_invoices(status)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<DeliveryInvoiceDto>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "create_invoice_from_lead" => {
                let lead_id: String = match deserialize_arg(&args, "lead_id", seam) {
                    Ok(lead_id) => lead_id,
                    Err(response) => return response,
                };
                let input: Option<axagent_harness::types::opc_demand::CreateInvoiceFromLeadInput> =
                    match deserialize_optional_arg(&args, "input", seam) {
                        Ok(input) => input,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(
                    provider.create_invoice_from_lead(lead_id, input.unwrap_or_default()),
                ) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(invoice) => to_value("DeliveryInvoiceDto", &invoice),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "update_delivery_invoice_status" => {
                let invoice_id: String = match deserialize_arg(&args, "invoice_id", seam) {
                    Ok(invoice_id) => invoice_id,
                    Err(response) => return response,
                };
                let status: String = match deserialize_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.update_delivery_invoice_status(invoice_id, status))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(invoice) => to_value("DeliveryInvoiceDto", &invoice),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "delete_delivery_invoice" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_delivery_invoice(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "delivery_summary" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delivery_summary()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(summary) => to_value("DeliverySummary", &summary),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── 需求发现八 op（#264 批 2）──
            "opc.store" if op == "list_platforms" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_platforms()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<DemandPlatform>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "save_platform" => {
                let input: axagent_harness::types::opc_demand::SaveDemandPlatformInput =
                    match deserialize_arg(&args, "input", seam) {
                        Ok(input) => input,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.save_platform(input)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(platform) => to_value("DemandPlatform", &platform),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "delete_platform" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_platform(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_leads" => {
                let limit: Option<u64> = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let min_score: Option<f64> =
                    match deserialize_optional_arg(&args, "min_score", seam) {
                        Ok(min_score) => min_score,
                        Err(response) => return response,
                    };
                let status: Option<String> = match deserialize_optional_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_leads(limit, min_score, status))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<DemandLeadDto>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_scan_policy" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_scan_policy()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(policy) => to_value("ScanPolicy", &policy),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "save_scan_policy" => {
                let policy: axagent_harness::types::opc_demand::ScanPolicy =
                    match deserialize_arg(&args, "policy", seam) {
                        Ok(policy) => policy,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.save_scan_policy(policy)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(policy) => to_value("ScanPolicy", &policy),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "discover_and_evaluate_leads" => {
                let query: String = match deserialize_arg(&args, "query", seam) {
                    Ok(query) => query,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.discover_and_evaluate_leads(query))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(summary) => to_value("DiscoverLeadsSummary", &summary),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "create_lead" => {
                let input: axagent_harness::types::opc_demand::SaveDemandLeadInput =
                    match deserialize_arg(&args, "input", seam) {
                        Ok(input) => input,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_lead(input)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(lead) => to_value("DemandLeadDto", &lead),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 3：需求实现三 op（状态机 / 转模板 / 启动执行）──
            // 出参分别为线索视图、新模板、执行 id 字符串；转模板还会回灌能力索引。
            "opc.store" if op == "update_lead_status" => {
                let lead_id: String = match deserialize_arg(&args, "lead_id", seam) {
                    Ok(lead_id) => lead_id,
                    Err(response) => return response,
                };
                let status: String = match deserialize_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.update_lead_status(lead_id, status))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(lead) => to_value("DemandLeadDto", &lead),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "convert_lead_to_workflow" => {
                let lead_id: String = match deserialize_arg(&args, "lead_id", seam) {
                    Ok(lead_id) => lead_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.convert_lead_to_workflow(lead_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(template) => to_value("WorkflowTemplateData", &template),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "run_lead_workflow" => {
                let lead_id: String = match deserialize_arg(&args, "lead_id", seam) {
                    Ok(lead_id) => lead_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.run_lead_workflow(lead_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(execution_id) => to_value("String", &execution_id),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_domain_workflows" => {
                let domain_id: Option<String> =
                    match deserialize_optional_arg(&args, "domain_id", seam) {
                        Ok(domain_id) => domain_id,
                        Err(response) => return response,
                    };
                let tag: Option<String> = match deserialize_optional_arg(&args, "tag", seam) {
                    Ok(tag) => tag,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_domain_workflows(domain_id, tag))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<DomainWorkflowSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_domain_workflow" => {
                let workflow_id: String = match deserialize_arg(&args, "workflow_id", seam) {
                    Ok(workflow_id) => workflow_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_domain_workflow(workflow_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(template) => to_value("Option<WorkflowTemplateResponse>", &template),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_domains" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_domains()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<DomainEntryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 5：交付面六 op（execute 写 + list/get 读 + update/retry/cancel 写）──
            "opc.store" if op == "execute_demand_workflow" => {
                let lead_id: String = match deserialize_arg(&args, "lead_id", seam) {
                    Ok(lead_id) => lead_id,
                    Err(response) => return response,
                };
                let workflow_template_id: Option<String> =
                    match deserialize_optional_arg(&args, "workflow_template_id", seam) {
                        Ok(workflow_template_id) => workflow_template_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.execute_demand_workflow(lead_id, workflow_template_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(row) => to_value("DeliveryRecordView", &row),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_deliveries" => {
                let status: Option<String> = match deserialize_optional_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                let lead_id: Option<String> = match deserialize_optional_arg(&args, "lead_id", seam)
                {
                    Ok(lead_id) => lead_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_deliveries(status, lead_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<DeliveryRecordView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_delivery" => {
                let delivery_id: String = match deserialize_arg(&args, "delivery_id", seam) {
                    Ok(delivery_id) => delivery_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_delivery(delivery_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(row) => to_value("DeliveryRecordView", &row),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "update_delivery" => {
                let delivery_id: String = match deserialize_arg(&args, "delivery_id", seam) {
                    Ok(delivery_id) => delivery_id,
                    Err(response) => return response,
                };
                let status: String = match deserialize_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                let progress: Option<f64> = match deserialize_optional_arg(&args, "progress", seam)
                {
                    Ok(progress) => progress,
                    Err(response) => return response,
                };
                let result_summary: Option<String> =
                    match deserialize_optional_arg(&args, "result_summary", seam) {
                        Ok(result_summary) => result_summary,
                        Err(response) => return response,
                    };
                let deliverables: Option<serde_json::Value> =
                    match deserialize_optional_arg(&args, "deliverables", seam) {
                        Ok(deliverables) => deliverables,
                        Err(response) => return response,
                    };
                let errors: Option<serde_json::Value> =
                    match deserialize_optional_arg(&args, "errors", seam) {
                        Ok(errors) => errors,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.update_delivery(
                    delivery_id,
                    status,
                    progress,
                    result_summary,
                    deliverables,
                    errors,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(row) => to_value("DeliveryRecordView", &row),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "retry_delivery" => {
                let delivery_id: String = match deserialize_arg(&args, "delivery_id", seam) {
                    Ok(delivery_id) => delivery_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.retry_delivery(delivery_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(row) => to_value("DeliveryRecordView", &row),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "cancel_delivery" => {
                let delivery_id: String = match deserialize_arg(&args, "delivery_id", seam) {
                    Ok(delivery_id) => delivery_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.cancel_delivery(delivery_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(row) => to_value("DeliveryRecordView", &row),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 6：能力发现面五 op ──
            // 能力清单扫描：零入参读 op（权限仍 `None`）。
            "opc.store" if op == "scan_capabilities" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.scan_capabilities()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(inventory) => to_value("CapabilityInventory", &inventory),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 主动评估入库：零入参写 op（逐领域关键词扫描落库）。
            "opc.store" if op == "proactive_evaluate_leads" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.proactive_evaluate_leads()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(summary) => to_value("ProactiveScanSummary", &summary),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 能力缺口列表：`status` 可选过滤。
            "opc.store" if op == "list_capability_gaps" => {
                let status: Option<String> = match deserialize_optional_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_capability_gaps(status)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<CapabilityGapView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 关闭能力缺口：入参只有 `gap_id`。
            "opc.store" if op == "close_capability_gap" => {
                let gap_id: String = match deserialize_arg(&args, "gap_id", seam) {
                    Ok(gap_id) => gap_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.close_capability_gap(gap_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(row) => to_value("CapabilityGapView", &row),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 主动分析缺口：零入参写 op（Top 5 高频缺失自动建行）。
            "opc.store" if op == "analyze_capability_gaps" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.analyze_capability_gaps()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(summary) => to_value("AnalyzeGapsSummary", &summary),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 7 刀 A：需求订阅面三 op ──────────────────────────────
            //
            // 三条都是 dao 的直投影：关键词必填、`interval_hours` / `min_score` 钳位、
            // 查无行 `NotFound` 全在宿主 impl 委托的 dao 里，接缝不新增校验闸（「迁≠改」）。
            "opc.store" if op == "list_subscriptions" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_subscriptions()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<DemandSubscription>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "save_subscription" => {
                let input: axagent_harness::types::opc_demand::SaveDemandSubscriptionInput =
                    match deserialize_arg(&args, "input", seam) {
                        Ok(input) => input,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.save_subscription(input)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(row) => to_value("DemandSubscription", &row),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "delete_subscription" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.delete_subscription(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(()) => FrameResponse::success(Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 7 刀 A：域包配置注册表五 op（代码内 14 份注册表）──────
            //
            // ⚠ `list_industries`（本节，注册表）与下一节的 `list_runtime_industries`
            //（磁盘域包目录）同名不同源，故两条 op 分别保留、不可合并。
            "opc.store" if op == "list_industries" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_industries()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<PackSummaryEntryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_capability_pack_config" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.get_capability_pack_config(domain_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(config) => to_value("CapabilityPackConfig", &config),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_action_config" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let action_key: String = match deserialize_arg(&args, "action_key", seam) {
                    Ok(action_key) => action_key,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.get_action_config(domain_pack_id, action_key))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(action) => to_value("CapabilityPackActionConfig", &action),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_capability_pack_actions" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.list_capability_pack_actions(domain_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<PackActionSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_capability_pack_workflows" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.list_capability_pack_workflows(domain_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<PackWorkflowSummaryView>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 7 刀 A：运行时域包三 op（磁盘域包配置）────────────────
            "opc.store" if op == "list_runtime_industries" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_runtime_industries()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("RuntimeIndustryListView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_kpi_definitions" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_kpi_definitions(domain_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("PackKpiDefinitionsView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "has_capability_pack" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.has_capability_pack(domain_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(view) => to_value("PackExistenceView", &view),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 7 刀 B1：域包全读七 op（薄层透传）────────────────────
            //
            // 七条全是读 ⇒ 权限表写分支一条闸都不加（`READ_OPS` 测试集合钉住）。
            // `get_workflow_config` 走刀 A 上收的 `CapabilityPackWorkflowConfig`；
            // 其余六条的 provider 出参已是 `serde_json::Value`（形状钉在宿主实现体），
            // 本层直接透传、不做二次映射。
            "opc.store" if op == "get_workflow_config" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let workflow_id: String = match deserialize_arg(&args, "workflow_id", seam) {
                    Ok(workflow_id) => workflow_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.get_workflow_config(domain_pack_id, workflow_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(config) => to_value("CapabilityPackWorkflowConfig", &config),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "validate_entity" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let entity_type: String = match deserialize_arg(&args, "entity_type", seam) {
                    Ok(entity_type) => entity_type,
                    Err(response) => return response,
                };
                let entity_data: Value = match deserialize_arg(&args, "entity_data", seam) {
                    Ok(entity_data) => entity_data,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.validate_entity(
                    domain_pack_id,
                    entity_type,
                    entity_data,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "batch_validate_entities" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let entities: Vec<Value> = match deserialize_arg(&args, "entities", seam) {
                    Ok(entities) => entities,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.batch_validate_entities(domain_pack_id, entities))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "compute_kpis" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let days: Option<i64> = match deserialize_arg(&args, "days", seam) {
                    Ok(days) => days,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.compute_kpis(domain_pack_id, days))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_capability_pack_data" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let source_id: String = match deserialize_arg(&args, "source_id", seam) {
                    Ok(source_id) => source_id,
                    Err(response) => return response,
                };
                let data_domain: String = match deserialize_arg(&args, "data_domain", seam) {
                    Ok(data_domain) => data_domain,
                    Err(response) => return response,
                };
                let query: Option<Value> = match deserialize_arg(&args, "query", seam) {
                    Ok(query) => query,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_capability_pack_data(
                    domain_pack_id,
                    source_id,
                    data_domain,
                    query,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_capability_pack_precheck" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.get_capability_pack_precheck(domain_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_capability_pack_health" => {
                let domain_pack_id: String = match deserialize_arg(&args, "domain_pack_id", seam) {
                    Ok(domain_pack_id) => domain_pack_id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.get_capability_pack_health(domain_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            // ── #264 批 7 刀 B2：域包单据面十二 op（六读六写，薄层透传）─────────────
            //
            // 六条写 op（`update_invoice` / `record_kpi` / `import_talent_library` /
            // `sync_fleet` / `update_publish_schedule` / `execute_dynamic_workflow`）已在
            // `required_permission_for` 登记 `Write`；六条读 op 仍 `None`（`READ_OPS` 钉住）。
            // 入参键一律接缝 snake_case（`id` / `email` / `period` / `path` / `input`）；
            // `input` 是域模型 DTO，内部键沿用迁移前的 snake_case —— 本层只反序列化、不映射。
            "opc.store" if op == "get_invoice" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_invoice(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(invoice) => to_value("Invoice", &invoice),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "update_invoice" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                // 入参 DTO（`UpdateInvoiceInput`）权威定义在 company-runtime（implementor），
                // harness 是 foundation 不得引用 ⇒ 本层按 `Value` 透传，字段校验在宿主 provider。
                let input: Value = match deserialize_arg(&args, "input", seam) {
                    Ok(input) => input,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.update_invoice(id, input)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(invoice) => to_value("Invoice", &invoice),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_customer" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_customer(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(customer) => to_value("Customer", &customer),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "find_customer_by_email" => {
                let email: String = match deserialize_arg(&args, "email", seam) {
                    Ok(email) => email,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.find_customer_by_email(email)) else {
                    return no_runtime(seam);
                };
                match result {
                    // 「查无此邮箱」是**正常结果**（`None` → `null`），不许翻成错误码。
                    Ok(customer) => to_value("Option<Customer>", &customer),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "record_kpi" => {
                let input: Value = match deserialize_arg(&args, "input", seam) {
                    Ok(input) => input,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.record_kpi(input)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(record) => to_value("KpiRecord", &record),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_revenue" => {
                let category: Option<String> =
                    match deserialize_optional_arg(&args, "category", seam) {
                        Ok(category) => category,
                        Err(response) => return response,
                    };
                let limit: Option<u32> = match deserialize_optional_arg(&args, "limit", seam) {
                    Ok(limit) => limit,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_revenue(category, limit)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(rows) => to_value("Vec<RevenueRecord>", &rows),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_investment_advice" => {
                let period: String = match deserialize_arg(&args, "period", seam) {
                    Ok(period) => period,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_investment_advice(period)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(advice) => to_value("InvestmentAdvice", &advice),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "import_talent_library" => {
                let path: String = match deserialize_arg(&args, "path", seam) {
                    Ok(path) => path,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.import_talent_library(path)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "sync_fleet" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.sync_fleet()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_publish_schedule" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_publish_schedule(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(schedule) => to_value("PublishSchedule", &schedule),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "update_publish_schedule" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let input: Value = match deserialize_arg(&args, "input", seam) {
                    Ok(input) => input,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.update_publish_schedule(id, input))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(schedule) => to_value("PublishSchedule", &schedule),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "execute_dynamic_workflow" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let workflow_id: Option<String> =
                    match deserialize_optional_arg(&args, "workflow_id", seam) {
                        Ok(workflow_id) => workflow_id,
                        Err(response) => return response,
                    };
                let days: Option<u32> = match deserialize_optional_arg(&args, "days", seam) {
                    Ok(days) => days,
                    Err(response) => return response,
                };
                let user_input: Option<Value> =
                    match deserialize_optional_arg(&args, "user_input", seam) {
                        Ok(user_input) => user_input,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.execute_dynamic_workflow(
                    capability_pack_id,
                    workflow_id,
                    days,
                    user_input,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            // #264 批 7 刀 B3：域包自动规则 / 仪表盘四 op（三读一写，出参都是宿主组好的
            // JSON 信封，本层逐字透传；`run_automation_rules` 的 `fields` 是任意对象）。
            "opc.store" if op == "capability_pack_dashboard" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let days: Option<i64> = match deserialize_optional_arg(&args, "days", seam) {
                    Ok(days) => days,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.capability_pack_dashboard(capability_pack_id, days))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "capability_pack_workflow_steps" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.capability_pack_workflow_steps(capability_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "capability_pack_automation_rules" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.capability_pack_automation_rules(capability_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "run_automation_rules" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let entity_type: String = match deserialize_arg(&args, "entity_type", seam) {
                    Ok(entity_type) => entity_type,
                    Err(response) => return response,
                };
                let entity_id: String = match deserialize_arg(&args, "entity_id", seam) {
                    Ok(entity_id) => entity_id,
                    Err(response) => return response,
                };
                let status: Option<String> = match deserialize_optional_arg(&args, "status", seam) {
                    Ok(status) => status,
                    Err(response) => return response,
                };
                let overdue_days: Option<u32> =
                    match deserialize_optional_arg(&args, "overdue_days", seam) {
                        Ok(overdue_days) => overdue_days,
                        Err(response) => return response,
                    };
                let created_days: Option<u32> =
                    match deserialize_optional_arg(&args, "created_days", seam) {
                        Ok(created_days) => created_days,
                        Err(response) => return response,
                    };
                let fields: Option<Value> = match deserialize_optional_arg(&args, "fields", seam) {
                    Ok(fields) => fields,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.run_automation_rules(
                    capability_pack_id,
                    entity_type,
                    entity_id,
                    status,
                    overdue_days,
                    created_days,
                    fields,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            // #264 批 7 刀 B3：需求订阅扫描 / 扫描任务装配 / 能力匹配三条。
            // 前两条是写 op（扫描入库 + 建/改调度任务），`match_lead_capabilities` 是读 op。
            "opc.store" if op == "run_subscription_scan" => {
                let only_due: Option<bool> = match deserialize_optional_arg(&args, "only_due", seam)
                {
                    Ok(only_due) => only_due,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.run_subscription_scan(only_due)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(summary) => to_value("SubscriptionScanSummary", &summary),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "ensure_demand_scan_job" => {
                let cron_expression: Option<String> =
                    match deserialize_optional_arg(&args, "cron_expression", seam) {
                        Ok(cron_expression) => cron_expression,
                        Err(response) => return response,
                    };
                let webhook_url: Option<String> =
                    match deserialize_optional_arg(&args, "webhook_url", seam) {
                        Ok(webhook_url) => webhook_url,
                        Err(response) => return response,
                    };
                let file_path: Option<String> =
                    match deserialize_optional_arg(&args, "file_path", seam) {
                        Ok(file_path) => file_path,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.ensure_demand_scan_job(
                    cron_expression,
                    webhook_url,
                    file_path,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "match_lead_capabilities" => {
                let lead_id: String = match deserialize_arg(&args, "lead_id", seam) {
                    Ok(lead_id) => lead_id,
                    Err(response) => return response,
                };
                let top_k: Option<usize> = match deserialize_optional_arg(&args, "top_k", seam) {
                    Ok(top_k) => top_k,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.match_lead_capabilities(lead_id, top_k))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(matched) => to_value("LeadCapabilityMatch", &matched),
                    Err(e) => seam_provider_error(e),
                }
            },
            // 刀 B3 收尾：自动学习（学习引擎只在宿主装配，本层只做入参解包与错误映射）。
            "opc.store" if op == "auto_learn_workflow" => {
                let template_id: String = match deserialize_arg(&args, "template_id", seam) {
                    Ok(template_id) => template_id,
                    Err(response) => return response,
                };
                let workflow_result: Value = match deserialize_arg(&args, "workflow_result", seam) {
                    Ok(workflow_result) => workflow_result,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.auto_learn_workflow(template_id, workflow_result))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(outcome) => FrameResponse::success(outcome),
                    Err(e) => seam_provider_error(e),
                }
            },
            // #264 刀 C 批 1：域包 manifest/prompt 装配 + 学习面三条，全读 op、全信封透传
            // （出参形状钉在宿主 provider，本层只做入参解包与错误映射，同刀 B3 四条读 op）。
            "opc.store" if op == "capability_pack_manifest" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.capability_pack_manifest(capability_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "build_capability_pack_prompt" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let action_key: String = match deserialize_arg(&args, "action_key", seam) {
                    Ok(action_key) => action_key,
                    Err(response) => return response,
                };
                let user_input: Option<String> =
                    match deserialize_optional_arg(&args, "user_input", seam) {
                        Ok(user_input) => user_input,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.build_capability_pack_prompt(
                    capability_pack_id,
                    action_key,
                    user_input,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_learning_metrics" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_learning_metrics(capability_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_learning_config" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_learning_config(capability_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "list_learning_configs" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.list_learning_configs()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            // #264 刀 C 批 2：域包分析执行（读 op）。风控阈值与数据服务装配在宿主 provider，
            // 本层只做入参解包（`days` 可选；缺省 30 天由宿主补）。
            "opc.store" if op == "execute_analysis" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let days: Option<u32> = match deserialize_optional_arg(&args, "days", seam) {
                    Ok(days) => days,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.execute_analysis(capability_pack_id, days))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            // #264 刀 C 批 3：学习面七 op（六写一读）。学习引擎与适配器注册表都在宿主
            // provider，本层只做入参解包与权限闸判定。
            "opc.store" if op == "reflect_on_workflow" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let workflow_id: String = match deserialize_arg(&args, "workflow_id", seam) {
                    Ok(workflow_id) => workflow_id,
                    Err(response) => return response,
                };
                let workflow_result: Value = match deserialize_arg(&args, "workflow_result", seam) {
                    Ok(workflow_result) => workflow_result,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.reflect_on_workflow(
                    capability_pack_id,
                    workflow_id,
                    workflow_result,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "evolve_workflow" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let workflow_id: String = match deserialize_arg(&args, "workflow_id", seam) {
                    Ok(workflow_id) => workflow_id,
                    Err(response) => return response,
                };
                let reason: String = match deserialize_arg(&args, "reason", seam) {
                    Ok(reason) => reason,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.evolve_workflow(
                    capability_pack_id,
                    workflow_id,
                    reason,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "run_self_improvement" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let target: String = match deserialize_arg(&args, "target", seam) {
                    Ok(target) => target,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.run_self_improvement(capability_pack_id, target))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "trigger_capability_pack_learning" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let workflow_id: String = match deserialize_arg(&args, "workflow_id", seam) {
                    Ok(workflow_id) => workflow_id,
                    Err(response) => return response,
                };
                let workflow_result: Value = match deserialize_arg(&args, "workflow_result", seam) {
                    Ok(workflow_result) => workflow_result,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.trigger_capability_pack_learning(
                    capability_pack_id,
                    workflow_id,
                    workflow_result,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_rl_stats" => {
                let capability_pack_id: Option<String> =
                    match deserialize_optional_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_rl_stats(capability_pack_id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "trigger_rl_optimization" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.trigger_rl_optimization(capability_pack_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "record_rl_experience" => {
                let capability_pack_id: String =
                    match deserialize_arg(&args, "capability_pack_id", seam) {
                        Ok(capability_pack_id) => capability_pack_id,
                        Err(response) => return response,
                    };
                let workflow_id: String = match deserialize_arg(&args, "workflow_id", seam) {
                    Ok(workflow_id) => workflow_id,
                    Err(response) => return response,
                };
                let quality_score: f64 = match deserialize_arg(&args, "quality_score", seam) {
                    Ok(quality_score) => quality_score,
                    Err(response) => return response,
                };
                let workflow_result: Value = match deserialize_arg(&args, "workflow_result", seam) {
                    Ok(workflow_result) => workflow_result,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.record_rl_experience(
                    capability_pack_id,
                    workflow_id,
                    quality_score,
                    workflow_result,
                )) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            // #264 批 4a：工作项六写一读（六条写 op 在权限表登记 `Write`；`kanban_board` 纯读）。
            "opc.store" if op == "create_work_item" => {
                let title: String = match deserialize_arg(&args, "title", seam) {
                    Ok(title) => title,
                    Err(response) => return response,
                };
                let owner_role_id: Option<String> =
                    match deserialize_optional_arg(&args, "owner_role_id", seam) {
                        Ok(owner_role_id) => owner_role_id,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.create_work_item(title, owner_role_id))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "kanban_board" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.kanban_board()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "work_item_start" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.start_work_item(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "work_item_review" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.review_work_item(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "escalate_work_item" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let reason: String = match deserialize_arg(&args, "reason", seam) {
                    Ok(reason) => reason,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.escalate_work_item(id, reason)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "work_item_unblock" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.unblock_work_item(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "run_self_improving_work_item" => {
                let task: String = match deserialize_arg(&args, "task", seam) {
                    Ok(task) => task,
                    Err(response) => return response,
                };
                let max_rounds: Option<u32> =
                    match deserialize_optional_arg(&args, "max_rounds", seam) {
                        Ok(max_rounds) => max_rounds,
                        Err(response) => return response,
                    };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.run_self_improving_work_item(task, max_rounds))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            // #264 批 4b：域包与内容资产五 op（三条写 op 在权限表登记 `Write`；
            // `get_content_asset` / `market_list` 纯读）。
            "opc.store" if op == "export_capability_pack" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let out_dir: String = match deserialize_arg(&args, "out_dir", seam) {
                    Ok(out_dir) => out_dir,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.export_capability_pack(id, out_dir))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(path) => FrameResponse::success(serde_json::Value::String(path)),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "import_capability_pack" => {
                let archive_path: String = match deserialize_arg(&args, "archive_path", seam) {
                    Ok(archive_path) => archive_path,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.import_capability_pack(archive_path))
                else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(message) => FrameResponse::success(serde_json::Value::String(message)),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "set_capability_pack_enabled" => {
                let pack_id: String = match deserialize_arg(&args, "pack_id", seam) {
                    Ok(pack_id) => pack_id,
                    Err(response) => return response,
                };
                let enabled: bool = match deserialize_arg(&args, "enabled", seam) {
                    Ok(enabled) => enabled,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) =
                    self.block_on(provider.set_capability_pack_enabled(pack_id, enabled))
                else {
                    return no_runtime(seam);
                };
                match result {
                    // 出参 `()`：帧里与迁移前 Tauri 的 `Result<(), String>` 同形（`null`）。
                    Ok(()) => FrameResponse::success(serde_json::Value::Null),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "get_content_asset" => {
                let id: String = match deserialize_arg(&args, "id", seam) {
                    Ok(id) => id,
                    Err(response) => return response,
                };
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.get_content_asset(id)) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            "opc.store" if op == "market_list" => {
                let Some(provider) = self.registry.get_opc_store() else {
                    return self.unavailable(seam);
                };
                let Some(result) = self.block_on(provider.market_list()) else {
                    return no_runtime(seam);
                };
                match result {
                    Ok(value) => FrameResponse::success(value),
                    Err(e) => seam_provider_error(e),
                }
            },
            other if platform_name_from_seam(other).is_some() => {
                let platform = platform_name_from_seam(other).unwrap_or_default();
                let Some(adapter) = self.registry.get_platform_adapter(platform) else {
                    return self.unavailable(seam);
                };
                // `PlatformConfig` 是纯 DTO，可跨进程；故 start / send 要求调用方把它放进 args。
                match op {
                    "start" => {
                        let config: PlatformConfig = match deserialize_arg(&args, "config", seam) {
                            Ok(config) => config,
                            Err(response) => return response,
                        };
                        let Some(result) = self.block_on(adapter.start(&config)) else {
                            return no_runtime(seam);
                        };
                        match result {
                            Ok(()) => FrameResponse::success(Value::Null),
                            Err(e) => {
                                FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string())
                            },
                        }
                    },
                    "stop" => {
                        let Some(result) = self.block_on(adapter.stop()) else {
                            return no_runtime(seam);
                        };
                        match result {
                            Ok(()) => FrameResponse::success(Value::Null),
                            Err(e) => {
                                FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string())
                            },
                        }
                    },
                    "is_connected" => {
                        let Some(connected) = self.block_on(adapter.is_connected()) else {
                            return no_runtime(seam);
                        };
                        FrameResponse::success(json!({ "connected": connected }))
                    },
                    "send" => {
                        let config: PlatformConfig = match deserialize_arg(&args, "config", seam) {
                            Ok(config) => config,
                            Err(response) => return response,
                        };
                        let chat_id: String = match deserialize_arg(&args, "chat_id", seam) {
                            Ok(chat_id) => chat_id,
                            Err(response) => return response,
                        };
                        let text: String = match deserialize_arg(&args, "text", seam) {
                            Ok(text) => text,
                            Err(response) => return response,
                        };
                        let parse_mode: Option<String> =
                            match deserialize_optional_arg(&args, "parse_mode", seam) {
                                Ok(parse_mode) => parse_mode,
                                Err(response) => return response,
                            };
                        let Some(result) = self.block_on(adapter.send_message(
                            &config,
                            &chat_id,
                            &text,
                            parse_mode.as_deref(),
                        )) else {
                            return no_runtime(seam);
                        };
                        match result {
                            Ok(()) => FrameResponse::success(Value::Null),
                            Err(e) => {
                                FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string())
                            },
                        }
                    },
                    other_op => FrameResponse::error(
                        error_codes::SEAM_CALL_UNSUPPORTED,
                        format!("不支持 `call_seam` 到接缝 `{seam}` 的 op `{other_op}`"),
                    ),
                }
            },
            // `model.provider.{类型名}` 与 `system.prompt.{段名}` 同为**动态**接缝 ID，
            // 只能用前缀守卫；返回值形状与对应门面**逐字对称**。
            other if provider_type_from_seam(other).is_some() => {
                let provider_type = provider_type_from_seam(other).unwrap_or_default();
                let Some(adapter) = self.registry.get_model_provider(provider_type) else {
                    return self.unavailable(seam);
                };
                // chat / list_models / embed 都必须带调用上下文（API key / base URL 等，
                // 由发起方插件自备 —— 宿主不替它保管凭据）。
                let context: ProviderRequestContext = match deserialize_arg(&args, "context", seam)
                {
                    Ok(context) => context,
                    Err(response) => return response,
                };
                match op {
                    "chat" => {
                        let request: ChatRequest = match deserialize_arg(&args, "request", seam) {
                            Ok(request) => request,
                            Err(response) => return response,
                        };
                        let Some(result) = self.block_on(adapter.chat(&context, Arc::new(request)))
                        else {
                            return no_runtime(seam);
                        };
                        match result {
                            Ok(response) => to_value("ChatResponse", &response),
                            Err(e) => {
                                FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string())
                            },
                        }
                    },
                    "list_models" => {
                        let Some(result) = self.block_on(adapter.list_models(&context)) else {
                            return no_runtime(seam);
                        };
                        match result {
                            Ok(models) => to_value("models", &models),
                            Err(e) => {
                                FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string())
                            },
                        }
                    },
                    "embed" => {
                        let request: EmbedRequest = match deserialize_arg(&args, "request", seam) {
                            Ok(request) => request,
                            Err(response) => return response,
                        };
                        let Some(result) = self.block_on(adapter.embed(&context, request)) else {
                            return no_runtime(seam);
                        };
                        match result {
                            Ok(response) => to_value("EmbedResponse", &response),
                            Err(e) => {
                                FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string())
                            },
                        }
                    },
                    other_op => FrameResponse::error(
                        error_codes::SEAM_CALL_UNSUPPORTED,
                        format!("不支持 `call_seam` 到接缝 `{seam}` 的 op `{other_op}`"),
                    ),
                }
            },
            other if prompt_section_from_seam(other).is_some() => {
                let section = prompt_section_from_seam(other).unwrap_or_default();
                let Some(contributor) = self
                    .registry
                    .list_system_prompt_sections()
                    .into_iter()
                    .find(|contributor| contributor.name() == section)
                else {
                    return self.unavailable(seam);
                };
                if op != "contribute" {
                    return FrameResponse::error(
                        error_codes::SEAM_CALL_UNSUPPORTED,
                        format!("不支持 `call_seam` 到接缝 `{seam}` 的 op `{op}`"),
                    );
                }
                let system_prompt: Vec<String> = match deserialize_arg(&args, "system_prompt", seam)
                {
                    Ok(prompt) => prompt,
                    Err(response) => return response,
                };
                let extras: HashMap<String, String> =
                    match deserialize_optional_arg(&args, "extras", seam) {
                        Ok(Some(extras)) => extras,
                        Ok(None) => HashMap::new(),
                        Err(response) => return response,
                    };
                let ctx = ContextRequest {
                    session_id: args.get("session_id").and_then(Value::as_str).unwrap_or_default(),
                    conversation_id: args.get("conversation_id").and_then(Value::as_str),
                    agent_id: args.get("agent_id").and_then(Value::as_str),
                    system_prompt: &system_prompt,
                    extras: &extras,
                };
                let Some(content) = self.block_on(contributor.contribute(&ctx)) else {
                    return no_runtime(seam);
                };
                // `contribute` 本身就是 `Option<String>`：`null` = 该段跳过注入。
                to_value("system.prompt 段内容", &content)
            },
            // 接缝不在 §5.4 表内，或 op 与映射表不符 —— 都算「不支持」，不猜、不兜底。
            other => FrameResponse::error(
                error_codes::SEAM_CALL_UNSUPPORTED,
                format!("不支持 `call_seam` 到接缝 `{other}` 的 op `{op}`"),
            ),
        }
    }

    /// 该接缝当前没有 provider（典型场景：提供方插件已卸载）——**明确失败，不挂起**。
    fn unavailable(&self, seam: &str) -> FrameResponse {
        FrameResponse::error(
            error_codes::SEAM_PROVIDER_UNAVAILABLE,
            format!("接缝 `{seam}` 当前没有 provider（提供方可能已卸载）"),
        )
    }

    /// 在载入时捕获的 runtime 上同步跑一个 future；无 runtime 时返回 `None`。
    ///
    /// 为什么用 `Handle::block_on` 而不是 `tokio::task::block_in_place`：本方法只在
    /// **宿主读线程**被调用，而那是纯 `std::thread`（不在任何 async 上下文中），
    /// `block_on` 在此合法；`block_in_place` 反而要求调用方位于 multi-thread runtime 的
    /// worker 线程上。若在 async 上下文里调 `block_on`，tokio 会 panic —— 故本方法的
    /// 唯一调用点（[`run_reader`]）必须保持为普通线程。
    fn block_on<F: Future>(&self, future: F) -> Option<F::Output> {
        self.runtime.as_ref().map(|handle| handle.block_on(future))
    }
}

/// 构造「参数缺失 / 类型不符」的失败响应（统一走 `SEAM_CALL_INVALID_ARGS`）。
fn invalid_args(message: impl Into<String>) -> FrameResponse {
    FrameResponse::error(error_codes::SEAM_CALL_INVALID_ARGS, message.into())
}

/// 从 `args` 的指定键反序列化出目标类型，失败时给出**可直接定位**的错误响应。
fn deserialize_arg<T: serde::de::DeserializeOwned>(
    args: &Value,
    key: &str,
    seam: &str,
) -> Result<T, FrameResponse> {
    let raw = args
        .get(key)
        .cloned()
        .ok_or_else(|| invalid_args(format!("接缝 `{seam}` 需要 `{key}` 参数")))?;
    serde_json::from_value(raw)
        .map_err(|e| invalid_args(format!("接缝 `{seam}` 的 `{key}` 反序列化失败：{e}")))
}

/// 可选参数：键**缺失**或为 `null` 时返回 `None`，而不是像 `deserialize_arg` 那样报错。
///
/// 用于 trait 里本就可选的字段（`message.callback` 的 `username`、`platform.adapter` 的
/// `parse_mode`）—— 调用方省略它们是合法用法，不该被判成参数非法。
fn deserialize_optional_arg<T: serde::de::DeserializeOwned>(
    args: &Value,
    key: &str,
    seam: &str,
) -> Result<Option<T>, FrameResponse> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => serde_json::from_value(raw.clone())
            .map(Some)
            .map_err(|e| invalid_args(format!("接缝 `{seam}` 的 `{key}` 反序列化失败：{e}"))),
    }
}

/// 解析 `as_of` 参数：跨进程**显式透传** as-of，不依赖宿主 `task_local`。
///
/// 支持两种形态：`"YYYY-MM-DD"` 字符串（走 `AsOfContext::parse`，内部拒绝未来日期）
/// 与 `{ as_of_date, source, data_scope }` 对象（回放调度方原样透传）。
/// 键缺失或 `null` 表示实盘，返回 `Ok(None)`。
fn deserialize_as_of_arg(
    args: &Value,
    seam: &str,
) -> Result<Option<axagent_harness::as_of::AsOfContext>, FrameResponse> {
    match args.get("as_of") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => axagent_harness::as_of::AsOfContext::parse(raw)
            .map(Some)
            .map_err(|e| invalid_args(format!("接缝 `{seam}` 的 `as_of` 解析失败：{e}"))),
        Some(raw @ Value::Object(_)) => {
            let ctx: axagent_harness::as_of::AsOfContext = serde_json::from_value(raw.clone())
                .map_err(|e| invalid_args(format!("接缝 `{seam}` 的 `as_of` 反序列化失败：{e}")))?;
            // 对象形态绕过了 AsOfContext::new 的未来日期校验，这里补回同一契约。
            axagent_harness::as_of::AsOfContext::new(ctx.as_of_date, ctx.source)
                .map(|validated| validated.with_data_scope(ctx.data_scope))
                .map(Some)
                .map_err(|e| invalid_args(format!("接缝 `{seam}` 的 `as_of` 解析失败：{e}")))
        },
        Some(other) => Err(invalid_args(format!(
            "接缝 `{seam}` 的 `as_of` 只接受 `YYYY-MM-DD` 字符串或对象，实际收到：{other}"
        ))),
    }
}

/// 把接缝方法的返回值序列化为 `value`；序列化失败（理论不可达）也走明确错误。
fn to_value<T: serde::Serialize>(what: &str, value: &T) -> FrameResponse {
    match serde_json::to_value(value) {
        Ok(value) => FrameResponse::success(value),
        Err(e) => {
            FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, format!("{what} 序列化失败：{e}"))
        },
    }
}

/// async 接缝在没有 runtime 时的失败响应。
fn no_runtime(seam: &str) -> FrameResponse {
    FrameResponse::error(
        error_codes::PLUGIN_CALL_FAILED,
        format!("宿主缺少 tokio runtime，无法调用 async 接缝 `{seam}`"),
    )
}

/// `workflow.run` 提供者的错误 → 帧错误码。
///
/// [`AxAgentError::Validation`] 是契约里对「宿主拒绝受理」（模板不在白名单 / 准入并发超限）
/// 的表达，映射为 [`error_codes::WORKFLOW_RUN_REJECTED`]，让插件能与宿主内部故障区分开；
/// 其余错误一律 [`error_codes::PLUGIN_CALL_FAILED`]。
fn workflow_run_error(error: AxAgentError) -> FrameResponse {
    match error {
        AxAgentError::Validation(message) => {
            FrameResponse::error(error_codes::WORKFLOW_RUN_REJECTED, message)
        },
        other => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, other.to_string()),
    }
}

/// 四条「宿主 → 插件」查询接缝（`stock.monitor` / `quote.watcher` / `knowledge.vector` /
/// `trajectory.store`）提供者的错误 → 帧错误码。
///
/// [`AxAgentError::Validation`] 是契约里对「宿主拒收入参」的表达（阈值 / 引擎节拍 / 聚合区间
/// 越界或 NaN、`alert_type` 不在白名单、集合不存在、正文超限、`top_k` 与时间窗非法），映射为
/// [`error_codes::SEAM_CALL_INVALID_ARGS`] —— 与分派层在反序列化阶段自己给的 [`invalid_args`]
/// **同码**，插件侧只需认一个参数错误码；其余错误一律 [`error_codes::PLUGIN_CALL_FAILED`]，
/// 含 trait 默认实现的 `Err(Provider("…未实现…"))`（那条链路不许被本函数升码）。
///
/// 形态照 [`workflow_run_error`]：那条接缝的 `Validation` 已定形为
/// [`error_codes::WORKFLOW_RUN_REJECTED`]（属回归面），不并入本函数、也不改它。
fn seam_provider_error(error: AxAgentError) -> FrameResponse {
    match error {
        AxAgentError::Validation(message) => {
            FrameResponse::error(error_codes::SEAM_CALL_INVALID_ARGS, message)
        },
        other => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, other.to_string()),
    }
}

// ───────────────────────── 传输层（读线程 + 配对表） ─────────────────────────

/// 命令帧宿主侧「收帧宽限」（毫秒）：等待上限 = 帧预算 `timeout_ms` + 本宽限。
///
/// 为什么要宽限：`timeout_ms` 是给 **worker 的执行预算**；worker 恰在预算边界上算完、
/// 再序列化回帧到达宿主，总要多花一点点。没有宽限就会把「压线完成」误杀成超时，
/// 而误杀后 worker 其实已经把命令跑完了（副作用已发生）。宽限只保护宿主调用线程
/// 的等待上界，不改变「预算归 worker」的分层语义。
const COMMAND_FRAME_GRACE_MS: u64 = 1_000;

/// 「宿主正占用对端」期间的出向等待复查粒度（任务 #73 M-3）。
///
/// 这**不是**一个预算，也不是宽限：它只决定「对端什么时候不再欠着宿主的答案」这件事
/// 多快被发现（`recv_timeout` 没有「条件等待」接口，只能按粒度醒来复查计数）。
/// 预算本身在这段墙钟里一格不扣，所以粒度取值不影响超时判定的语义，只影响嵌套结束后
/// 最多多等一个粒度的墙钟；20 ms 相对毫秒~秒级的帧往返可以忽略。
const INBOUND_PAUSE_POLL: Duration = Duration::from_millis(20);

/// 写线程在「子进程已被杀」之后留给自己的退出宽限（任务 #56）。
///
/// 杀进程会关掉管道读端 ⇒ 阻塞中的 `write` 立刻返回错误 ⇒ 写线程处理完关闭命令即退出。
/// 这个宽限**只兜异常**（OS 迟迟不解除阻塞 / 测试用的无进程夹具），不是等待窗口：
/// 超时后调用方放弃回收（detach）并留 warn，绝不为了 join 一个线程把停机拖成无界。
const WRITER_EXIT_GRACE: Duration = Duration::from_millis(200);

// ───────────────────────── 出向写线程（任务 #56） ─────────────────────────

/// 一帧出向数据：宿主发起的请求，或宿主对入站请求的回写响应。
///
/// 两者共用同一条 stdin，且都必须「整帧写出或不写」；写线程按投递顺序串行写，
/// 帧完整性不再依赖调用方持锁。
enum OutboundFrame {
    Request(FrameRequest),
    Response(FrameResponse),
}

impl OutboundFrame {
    /// `W` 保持泛型而不是 `dyn Write`：`write_json_frame` 的泛型参数隐含 `Sized`，
    /// 传 `&mut dyn Write` 无法满足（会撞 `the size for values of type dyn Write
    /// cannot be known`）。实际调用方是 [`CountingWriter`]。
    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Self::Request(frame) => write_json_frame(writer, frame),
            Self::Response(frame) => write_json_frame(writer, frame),
        }
    }
}

/// 写回执三态（任务 #56 的验收判据）。
///
/// **`NotDelivered` 只允许由 [`Self::Failed`] 且 `wrote_any == false` 推出**：
/// 那一刻一个字节都没进管道，帧不可能被对端读到 ⇒ 零副作用 ⇒ 命名命令可回退。
/// 其余两种状态都**不能**报 `NotDelivered`：
///
/// - [`Self::NoAck`]：写线程仍阻塞在 `write` 里，帧可能已写出部分字节；此时即使立刻杀进程，
///   也无法追溯「截止那一刻到底写出了多少」—— 「未送达」是不可证明的陈述。
/// - `Failed { wrote_any: true }`：已经有字节被管道接受，对端可能已读到半帧，
///   流就此损坏（后续帧的边界不再可信）。
/// - `Failed { wrote_any: false }` + [`Self::Written`] 才是两个确定的端点。
#[derive(Debug)]
enum WriteAck {
    /// 整帧已写出并 flush（对端至少能读到完整帧）。
    Written,
    /// 写没完成。`wrote_any = false` ⇒ 确定整帧未写出（零副作用）；
    /// `true` ⇒ 已有字节进入管道（流可能损坏，禁止判「未送达」）。
    Failed { reason: String, wrote_any: bool },
    /// 预算耗尽：写线程仍阻塞，**无法确定**是否已写出（可能已写出部分字节）。
    NoAck,
}

/// 写线程命令：写一帧（带回执口）或关闭管道退出。
enum WriterCommand {
    Frame { frame: OutboundFrame, ack: mpsc::Sender<WriteAck> },
    Close,
}

/// 写线程句柄：投递口 + 退出信号 + join 句柄。
///
/// `done` 是「线程已退出」的有界等待口 —— 标准库的 `JoinHandle` 没有带超时的 join，
/// 而停机不允许为了回收一个可能卡死的线程把上层拖挂。
struct WriterThread {
    tx: mpsc::Sender<WriterCommand>,
    done: mpsc::Receiver<()>,
    handle: JoinHandle<()>,
}

/// 计数写层：包住真实管道，记录**已被管道接受**的字节数（含 flush 之前的全部写入）。
///
/// 复用 plugin-proto 的 [`write_json_frame`]（长度前缀分帧 + flush），只在外层数数：
/// 写失败时据此区分「一个字节都没出去」（可判未送达）与「已写出部分字节」。
struct CountingWriter<'a> {
    inner: &'a mut (dyn Write + Send),
    written: usize,
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.written += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 写线程主循环：**独占** stdin，串行写帧（帧完整性的唯一保证点）。
///
/// 它永不调用宿主逻辑、永不持任何共享锁，因此「对端僵死不读管道」只会卡住它自己；
/// 调用方各自带预算等回执，互不牵连。收到 [`WriterCommand::Close`] 或投递口断开即退出，
/// 退出时 drop 掉管道写端 —— 对端读到 EOF，`serve` 形态的 worker 自行退出（防线①）。
fn run_writer(
    mut stdin: Option<Box<dyn Write + Send>>,
    commands: mpsc::Receiver<WriterCommand>,
    done: mpsc::Sender<()>,
) {
    while let Ok(command) = commands.recv() {
        match command {
            WriterCommand::Frame { frame, ack } => {
                let ack_state = match stdin.as_mut() {
                    Some(writer) => {
                        let mut counting = CountingWriter { inner: writer.as_mut(), written: 0 };
                        match frame.write_to(&mut counting) {
                            Ok(()) => WriteAck::Written,
                            Err(e) => WriteAck::Failed {
                                reason: format!("写帧失败：{e}"),
                                wrote_any: counting.written > 0,
                            },
                        }
                    },
                    // 与停机竞争：管道已被本线程自己关掉（只可能在 Close 之后，理论不可达）。
                    None => WriteAck::Failed {
                        reason: "通道已关闭（stdin 已释放）".to_string(),
                        wrote_any: false,
                    },
                };
                let _ = ack.send(ack_state);
            },
            WriterCommand::Close => break,
        }
    }
    // 关管道：`stdin` 在此 drop ⇒ 对端读到 EOF。
    drop(stdin);
    // 接收端可能已随连接的 drop 消失：那是正常收尾，不报错。
    let _ = done.send(());
}

/// 宿主侧 worker 连接的共享状态：读线程与调用方共用。
struct WorkerInner {
    plugin_id: String,
    /// 出向写线程（任务 #56）：stdin 由它独占，调用方**永不**持锁做阻塞写。
    ///
    /// `None` = 写线程已关停（停机 `take()` 掉 / 启动失败）。此时投递一律得到
    /// [`WriteAck::Failed`]（零字节写出 ⇒ 可判未送达）。
    writer: Mutex<Option<WriterThread>>,
    /// 等待中的请求：`request_id` → 响应投递口。读线程按 ID 投递，调用方按 ID 取。
    pending: Mutex<HashMap<String, mpsc::Sender<FrameResponse>>>,
    /// worker 子进程；`None` 仅出现在测试用的流式构造里（无进程可管）。
    child: Mutex<Option<Child>>,
    shutdown_timeout: Duration,
    /// 入站请求处理器；握手之前为 `None`（此时任何插件主动请求都回 `PLUGIN_UNKNOWN_OP`）。
    host_peer: Mutex<Option<Arc<HostPeer>>>,
    /// 自增配对 ID 的序号（ID 形如 `h1`、`h2`…，`h` 表示 host 侧发起）。
    next_request_id: AtomicU64,
    /// drain 标志（PLAN §13.3 优雅停机）：置位后 [`SeamInvoker::invoke_with_chain`]
    /// 拒绝**新**调用，等在途帧自然收尾 —— 停机不再掐死进行中的请求。
    draining: AtomicBool,
    /// 接缝调用的宿主等待预算（来自 [`PluginWorkerConfig::seam_wait_timeout`]，
    /// 缺省即协议常量 [`DEFAULT_SEAM_TIMEOUT_MS`]）。
    seam_wait_timeout: Duration,
    /// 握手 `describe` 帧的宿主等待预算（来自 [`PluginWorkerConfig::handshake_timeout`]，
    /// 缺省即协议常量 [`DEFAULT_HANDSHAKE_TIMEOUT_MS`]）。
    ///
    /// 单独一份而非复用 `seam_wait_timeout`：握手不经接缝公共入口，取值口径也不同
    /// —— 见 plugin-proto 两个常量的文档。
    handshake_timeout: Duration,
    /// 「连接可疑」闸（任务 #53 修 D1 的处置半边）：接缝调用**写帧后**到点仍收不到
    /// 配对响应 ⇒ 置位。此后接缝公共入口拒绝新调用（不再排队等一个新预算），停机
    /// 跳过等待 shutdown 回执（装死的 worker 回不了帧，礼貌等待会把卸载一起拖挂）。
    /// 任务 #63 起**握手超时同样置位**：那条连接本就不会交付出去，置位的唯一作用是让
    /// [`LoadedPlugin::load`] 失败路径的 `shutdown()` 走急停（否则「不回 describe」会变成
    /// 「卡在等 shutdown 回执」—— 同一病灶换了个位置）。
    ///
    /// **复位口**（任务 #73 M-2 补，原注释「无复位口（有意）」作废）：闸的生命周期 =
    /// **一个 [`LoadedPlugin`] 的生命周期**，显式 `unload` / `disable` 即清（见
    /// [`LoadedPlugin::unload`]）。刻意**不是**「下次调用自动清」：自动清等于把
    /// 「对端已实证不回帧」这条信息每笔重新付一遍预算，正是 D1 要止的血。
    /// 复位之所以必要：`Arc<WorkerInvoker>` 会被能力门面与
    /// [`crate::manager::PluginManager::plugin_invoker`] 持有并可能活过本实例；不清的话
    /// 「停用再启用」之后仍可能被上一轮的脏标记拒绝。
    ///
    /// 装载失败那条路**不需要**复位：握手超时置脏的连接随实例整体丢弃（`load` 返回 `Err`
    /// ⇒ `Arc<WorkerInvoker>` 被 drop ⇒ 进程被强杀），下一次 `load` 是新 `WorkerInner`、
    /// 新实例（闸=false），故 #63 新增的握手置位点不构成需要复位的存量。
    ///
    /// 为什么「拒后续」而不是「自动重启」：重启要重建进程、重放握手、重新注册全部
    /// 能力句柄，并让插件的进程内状态（游标/缓存）凭空消失——那是错误恢复策略，
    /// 超出「超时处置」的本分；标脏后由**调用方决定**停机重启，处置权留在上层。
    /// 为什么不是只「标脏但继续接受调用」：对端已实证不回帧，后续每笔调用都要白等
    /// 一个完整预算才失败，僵死会沿着回合级联变慢。
    seam_suspect: AtomicBool,
    /// 宿主**正在替本插件处理**的入站请求数（任务 #73 M-3）。
    ///
    /// worker 主动发来的 `emit` / `call_seam` 交独立任务落地（不再占读线程），处理期间
    /// 本计数 > 0。它是出向等待预算的**暂停条件**：对端此刻在等宿主的答案，这段墙钟
    /// 不是「worker 装死」的证据 —— 按墙钟扣预算会把 SDK 示范的嵌套形态确定性误杀，
    /// 并把健康连接判死（审查稿 A4.3 的六步链）。
    inbound_inflight: AtomicUsize,
    /// 读线程已退出（stdout EOF 或读错误）⇒ 对端进程已不存在（任务 #73 M-1）。
    ///
    /// ⚠ 与 [`Self::seam_suspect`] 是**两件事**，判据必须分开写：本闸 = 「进程不在」
    /// ⇒ 命名命令**回退**原通道（`manager.rs` 契约里「worker 中途异常退出也退回原通道」
    /// 那半句的落地）；`seam_suspect` = 「进程在、但曾超时未回帧」⇒ **拒绝**并等显式卸载
    /// 复位。两者混在一个 bool 里就是 M-2 / M-3 的根（PLAN §26.3 对 S-3 的裁定）。
    connection_closed: AtomicBool,
    /// 插件清单声明的权限集（来自 [`PluginWorkerConfig::permissions`]）。
    ///
    /// 连接级不可变快照：装载时克隆一次，运行期不再改。入站接缝分派的门面
    /// （[`HostPeer`]）在注入声明时从这里再克隆一份，二者同源。
    permissions: Vec<PluginPermission>,
}

/// 长驻 worker 子进程的传输实现（[`SeamInvoker`] 的主路径实现）。
pub struct WorkerInvoker {
    inner: Arc<WorkerInner>,
    /// 读线程句柄；`None` = 已 join。
    reader: Mutex<Option<JoinHandle<()>>>,
}

impl WorkerInvoker {
    /// 启动 worker 并接管其管道。
    fn spawn(config: &PluginWorkerConfig) -> Result<Self, WorkerError> {
        let plugin_id = config.plugin_id.clone();

        // 沙箱准入：未声明 subprocess_execution 即拒绝 spawn（PLAN §3.3 —— 检查在前）。
        check_subprocess_permission(&config.sandbox)
            .map_err(|_| WorkerError::SubprocessDenied { plugin_id: plugin_id.clone() })?;

        let mut command = Command::new(&config.program);
        command.args(&config.args);
        if let Some(dir) = &config.working_dir {
            command.current_dir(dir);
        }
        // ENV 白名单：env_clear + 按白名单回填（本仓唯一在 OS 层强制生效的沙箱项）。
        apply_env_to_command(&mut command, &config.sandbox);
        // 显式设置不受白名单约束（apply_env_to_command 之后设置）。
        command.env("AXAGENT_PLUGIN_ID", &plugin_id);
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());

        // CREATE_NO_WINDOW：避免每次 spawn 弹出控制台窗口。
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }

        let mut child = command.spawn().map_err(|e| WorkerError::Spawn {
            plugin_id: plugin_id.clone(),
            reason: e.to_string(),
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorkerError::MissingPipe { plugin_id: plugin_id.clone() })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::MissingPipe { plugin_id: plugin_id.clone() })?;

        let inner = Arc::new(WorkerInner {
            plugin_id: plugin_id.clone(),
            writer: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(Some(child)),
            shutdown_timeout: config.shutdown_timeout,
            host_peer: Mutex::new(None),
            next_request_id: AtomicU64::new(0),
            draining: AtomicBool::new(false),
            seam_wait_timeout: config.seam_wait_timeout,
            handshake_timeout: config.handshake_timeout,
            seam_suspect: AtomicBool::new(false),
            inbound_inflight: AtomicUsize::new(0),
            connection_closed: AtomicBool::new(false),
            permissions: config.permissions.clone(),
        });
        // 写线程先起：它接管 stdin，之后的所有写帧都只经这条串行写者。
        // 起不来 ⇒ 进程已 spawn，必须杀掉，不能留孤儿（与读线程失败的处置一致）。
        let writer = match Self::spawn_writer(Box::new(stdin), &plugin_id) {
            Ok(writer) => writer,
            Err(e) => {
                inner.kill_child();
                return Err(e);
            },
        };
        // 立刻入槽：读线程一起来就可能收到入站请求并回写，不能出现「写者还没挂上」的空窗。
        *inner.writer.lock() = Some(writer);
        let reader = match Self::spawn_reader(inner.clone(), Box::new(stdout), &plugin_id) {
            Ok(reader) => reader,
            Err(e) => {
                inner.close_writer(Instant::now() + WRITER_EXIT_GRACE);
                inner.kill_child();
                return Err(e);
            },
        };
        Ok(Self { inner, reader: Mutex::new(Some(reader)) })
    }

    /// 用一对现成的收发通道构造（**测试专用**：免启动真实子进程，
    /// 以便用回环 socket 驱动一个假 worker，验证 `request_id` 配对与重入）。
    #[cfg(test)]
    fn from_streams(
        plugin_id: impl Into<String>,
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
    ) -> Self {
        // 缺省预算取协议常量：与生产 spawn 完全同源（测试不注入第二份默认值）。
        Self::from_streams_with_timeouts(
            plugin_id,
            reader,
            writer,
            Duration::from_millis(DEFAULT_SEAM_TIMEOUT_MS),
            Duration::from_millis(DEFAULT_HANDSHAKE_TIMEOUT_MS),
        )
    }

    /// [`Self::from_streams`] 的可注入预算变体（仅测试用：装死用例不能真等 15s / 10s）。
    #[cfg(test)]
    fn from_streams_with_timeouts(
        plugin_id: impl Into<String>,
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        seam_wait_timeout: Duration,
        handshake_timeout: Duration,
    ) -> Self {
        let plugin_id = plugin_id.into();
        let inner = Arc::new(WorkerInner {
            plugin_id: plugin_id.clone(),
            writer: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(None),
            shutdown_timeout: Duration::from_millis(200),
            host_peer: Mutex::new(None),
            next_request_id: AtomicU64::new(0),
            draining: AtomicBool::new(false),
            seam_wait_timeout,
            handshake_timeout,
            seam_suspect: AtomicBool::new(false),
            inbound_inflight: AtomicUsize::new(0),
            connection_closed: AtomicBool::new(false),
            // 流式构造（仅测试）不带清单权限：写 op 一律被拒，与「未声明」同口径。
            permissions: Vec::new(),
        });
        // 与生产同一条写路径：测试也经写线程，避免「测的是另一套写逻辑」。
        let writer = Self::spawn_writer(writer, &plugin_id).expect("测试用写线程应能启动");
        *inner.writer.lock() = Some(writer);
        let thread =
            Self::spawn_reader(inner.clone(), reader, &plugin_id).expect("测试用读线程应能启动");
        Self { inner, reader: Mutex::new(Some(thread)) }
    }

    /// 启动专用写线程（stdin → 帧流；串行写，帧完整性由「单写者」保证）。
    fn spawn_writer(
        stdin: Box<dyn Write + Send>,
        plugin_id: &str,
    ) -> Result<WriterThread, WorkerError> {
        let (tx, commands) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name(format!("plugin-worker-writer-{plugin_id}"))
            .spawn(move || run_writer(Some(stdin), commands, done_tx))
            .map_err(|e| WorkerError::Spawn {
                plugin_id: plugin_id.to_string(),
                reason: format!("无法启动 worker 写线程：{e}"),
            })?;
        Ok(WriterThread { tx, done, handle })
    }

    /// 启动专用读线程（stdout → 配对表 / 入站处理器）。
    fn spawn_reader(
        inner: Arc<WorkerInner>,
        stdout: Box<dyn Read + Send>,
        plugin_id: &str,
    ) -> Result<JoinHandle<()>, WorkerError> {
        std::thread::Builder::new()
            .name(format!("plugin-worker-{plugin_id}"))
            .spawn(move || run_reader(inner, stdout))
            .map_err(|e| WorkerError::Spawn {
                plugin_id: plugin_id.to_string(),
                reason: format!("无法启动 worker 读线程：{e}"),
            })
    }

    /// 注入握手所得的声明：入站处理器（`emit` / `call_seam`）靠它做声明与自调校验。
    fn set_declaration(&self, declaration: &PluginDeclaration, registry: Arc<CapabilityRegistry>) {
        let host_peer = HostPeer {
            plugin_id: self.inner.plugin_id.clone(),
            declared_capabilities: declaration
                .capabilities
                .iter()
                .map(|capability| capability.seam.clone())
                .collect(),
            declared_calls: declaration.calls.clone(),
            registry,
            // 载入点若不在 async 上下文中，这里为 None：async 接缝的 call_seam 会明确报错。
            runtime: tokio::runtime::Handle::try_current().ok(),
            permissions: self.inner.permissions.clone(),
        };
        *self.inner.host_peer.lock() = Some(Arc::new(host_peer));
    }

    /// 握手：取插件声明并校验协议版本与能力项。
    ///
    /// 等待预算 = [`WorkerInner::handshake_timeout`]（任务 #63）：**不经**接缝公共入口
    /// [`SeamInvoker::invoke_with_chain`]，因此不读逐接缝覆盖表、也不受连接级
    /// `seam_wait_timeout` 支配 —— 握手是装载前置，不是用户发起的调用。
    fn handshake(&self) -> Result<PluginDeclaration, WorkerError> {
        // 取声明失败 → Handshake（进程起没起来 / 是否实现了 describe / 握手帧超时）；
        // 声明拿到了但不合法 → Declaration（版本不符、字段为空）。两类失败分开报，便于定位。
        let declaration: PluginDeclaration = self
            .call_typed(
                ops::DESCRIBE,
                json!({ "proto_version": AXAGENT_PLUGIN_PROTO_VERSION }),
                self.inner.handshake_timeout,
            )
            .map_err(|e| {
                // 超时 ⇒ 置「连接可疑」闸，让调用方兜底的 `shutdown()` 跳过等待 shutdown
                // 回执（否则装死的 worker 会把装载从「挂握手」改成「挂停机」）。
                // 只认 Timeout，其余错误不置闸 —— 与 [`Self::invoke_with_chain`] 同一判据：
                // 「对端明确回了错」与「对端装死」是两种可观测性，不能混为一谈。
                if matches!(e, WorkerError::Timeout { .. }) {
                    self.inner.seam_suspect.store(true, Ordering::Release);
                }
                WorkerError::Handshake {
                    plugin_id: self.inner.plugin_id.clone(),
                    reason: e.to_string(),
                }
            })?;
        declaration.validate().map_err(|e| WorkerError::Declaration {
            plugin_id: self.inner.plugin_id.clone(),
            reason: e.to_string(),
        })?;
        Ok(declaration)
    }

    /// 发一帧请求并等回**配对**的响应，写侧与收侧**各自**带预算（任务 #56 起）。
    ///
    /// 任务 #53/#63 之后接缝与握手都不再有无限预算：接缝经
    /// [`Self::invoke_with_chain`]（`seam_wait_timeout` 与逐接缝表）、握手经
    /// [`Self::handshake`]（`handshake_timeout`）、停机帧经 [`Self::shutdown`]（总预算的一半）。
    /// `wait` 是**这个调用**的预算：写回执最多等 `wait`，写成功后收配对响应再等 `wait`
    /// （两者不会同时消耗，正常路径下写回执是毫秒级）。
    ///
    /// 并发模型的三条硬约束：
    /// - 写帧**由写线程串行完成**，调用方只投递帧并等回执：对端可能正等我们处理它的请求，
    ///   在调用线程上阻塞写会让整条连接（含入站回写）一起卡死（任务 #56）；
    /// - 等响应期间**不持任何锁**：读线程要能并发把响应投进配对表；
    /// - 读线程退出时清空配对表：等待者拿到 `Err` 而不是永久挂起。
    ///
    /// 语义边界（如实声明）：
    /// - 超时**不等价于取消** —— 对端 worker 可能仍在执行，其迟到的响应帧会被读线程
    ///   按「无等待者」丢弃并留警告日志；本批不做跨帧取消协议。
    /// - **预算按「对端不被宿主占用」的时长消耗**（任务 #73 M-3）：本连接有入站请求正在
    ///   宿主手里处理时，等待只按 [`INBOUND_PAUSE_POLL`] 的粒度醒来复查、计时挂起。
    ///   代价如实写明：若宿主替插件跑的那一跳**自己**卡死，外层这一等也跟着没有上界 ——
    ///   那是入站服务侧**既存**的无界（审查稿 A4.2：宿主服务 `call_seam` 时没有等待上限），
    ///   本批没有给它加闸，登记进交回清单。
    /// - **写帧结果的判据**（任务 #56）见 [`WriteAck`]：只有「整帧确定未写出」
    ///   （`Failed { wrote_any: false }`）才报 [`WorkerError::NotDelivered`]（零副作用 ⇒
    ///   命令通道可回退）；写回执超时（`NoAck`）与「已写出部分字节」一律判连接不可用并报
    ///   `Transport` —— 那时帧**可能已被对端读到半截**，「未送达」不可证明。
    fn exchange_with_timeout(
        &self,
        op: &str,
        args: Value,
        call_chain: Vec<String>,
        wait: Duration,
    ) -> Result<FrameResponse, WorkerError> {
        let sequence = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed) + 1;
        let request_id = format!("h{sequence}");
        let (sender, receiver) = mpsc::channel();
        self.inner.pending.lock().insert(request_id.clone(), sender);
        let request =
            FrameRequest::new(op, args).with_id(request_id.clone()).with_chain(call_chain);

        // ─── 写侧（任务 #56）：投递 + 带预算等回执，调用方**不持锁**做阻塞写 ───
        let write_result = self.inner.submit_frame(OutboundFrame::Request(request), wait);
        match write_result {
            Err(reason) => {
                // 未尝试写、且连接已被判不可用（可疑闸）⇒ 拒绝口径，**不得**报未送达
                // （可疑连接的回退口径是「拒绝」，与进程不在时的「回退」方向相反）。
                self.inner.pending.lock().remove(&request_id);
                return Err(self.transport_error(format!("连接已判不可用，拒绝写帧：{reason}")));
            },
            Ok(WriteAck::Written) => {},
            Ok(WriteAck::Failed { reason, wrote_any: false }) => {
                // 帧一个字节都没出去 ⇒ 不会有响应 ⇒ 必须自己清掉槽位，否则配对表残留。
                // 报 [`WorkerError::NotDelivered`]（任务 #73 M-1）：
                // 「对端从未收到这一帧」是**零副作用**的机械证据，命名命令的回退判据要的正是它。
                self.inner.pending.lock().remove(&request_id);
                return Err(WorkerError::NotDelivered {
                    plugin_id: self.inner.plugin_id.clone(),
                    op: op.to_string(),
                    reason,
                });
            },
            Ok(WriteAck::Failed { reason, wrote_any: true }) => {
                // 已有字节进入管道：对端可能读到半帧，流边界不再可信 ⇒ 连接判不可用。
                self.inner.pending.lock().remove(&request_id);
                self.inner.poison();
                return Err(self.transport_error(format!(
                    "写帧中途失败（已写出部分字节，帧可能被对端部分接收，连接判不可用）：{reason}"
                )));
            },
            Ok(WriteAck::NoAck) => {
                // 写者仍阻塞 ⇒ 可能已写出部分字节，且无法追溯 ⇒ 同样判不可用。
                // `submit_frame` 已置可疑闸并杀进程（关管道读端解阻写者）。
                self.inner.pending.lock().remove(&request_id);
                return Err(self.transport_error(format!(
                    "写帧回执在预算 {} ms 内未到达（写线程仍阻塞，无法确认帧是否写出）；\
                     连接已判不可用",
                    wait.as_millis()
                )));
            },
        }
        let response = {
            // 预算 `wait` **只在对端不被宿主占用时消耗**（任务 #73 M-3 的第二半）。
            //
            // 为什么必须暂停：worker 的 handler 里嵌套一次 `peer.call_seam(...)`（SDK
            // 文档示范的形态）时，它回自己那一帧的前提是**宿主先答完那一跳**。这段
            // 时间对端既没装死也没崩，是宿主欠着它的答案 —— 按墙钟扣预算就会把外层
            // 调用确定性误杀，并顺带把健康连接标脏（审查稿 A4.3 的六步链）。
            // 暂停之后「到点收不到回帧」重新成为**装了死**的有效证据，
            // 置可疑闸的推论才成立（见 [`Self::invoke_with_chain`]）。
            let mut remaining = wait;
            loop {
                // 醒来的粒度：对端被宿主占用时按 [`INBOUND_PAUSE_POLL`] 复查计数，
                // 不被占用时一口气等到预算耗尽（不多花一次唤醒）。
                let host_owes_answer = self.inner.inbound_inflight.load(Ordering::Acquire) > 0;
                let slice = if host_owes_answer {
                    remaining.min(INBOUND_PAUSE_POLL)
                } else {
                    remaining
                };
                match receiver.recv_timeout(slice) {
                    Ok(response) => break response,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        self.inner.pending.lock().remove(&request_id);
                        return Err(self.transport_error("worker 连接已关闭，未收到配对响应"));
                    },
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // **到点重新读一次**闸：判定「是不是装死」看的是此刻还有没有入站
                        // 请求压在宿主手里 —— 进 `recv_timeout` 之前读的那一次只决定醒来
                        // 粒度，拿它当扣不扣预算的依据会把嵌套形态照旧误杀（实测：外层
                        // 150ms 那一格正是在对端开始 `call_seam` 之后才走完的）。
                        if self.inner.inbound_inflight.load(Ordering::Acquire) > 0 {
                            // 这一格整格不计（保守方向 = 多给对端时间，不是误杀它）。
                            continue;
                        }
                        remaining = remaining.saturating_sub(slice);
                        if remaining.is_zero() {
                            // 撤槽位：本次调用不再有人等。迟到的响应由读线程按无等待者丢弃。
                            self.inner.pending.lock().remove(&request_id);
                            return Err(WorkerError::Timeout {
                                plugin_id: self.inner.plugin_id.clone(),
                                op: op.to_string(),
                                timeout_ms: wait.as_millis() as u64,
                            });
                        }
                    },
                }
            }
        };

        match response.kind {
            ResponseKind::Success => Ok(response),
            ResponseKind::Error => {
                let code = response.code.unwrap_or_else(|| "SEAM_EXECUTE_FAILED".to_string());
                let message = response.message.unwrap_or_else(|| "插件未提供错误信息".to_string());
                // 码与文本**同时**成形（任务 #73 S-1）：`reason` 保持 `{code}: {message}`
                // 的既有 Display 形态不破坏可读性，判据改读 `code` 字段本体。
                Err(self.transport_error_with_code(code, message))
            },
        }
    }

    /// 发一帧并从响应的 `value` 反序列化出强类型结果（`wait` 语义同
    /// [`Self::exchange_with_timeout`]：写侧与收侧各自以它为上限）。
    fn call_typed<T: serde::de::DeserializeOwned>(
        &self,
        op: &str,
        args: Value,
        wait: Duration,
    ) -> Result<T, WorkerError> {
        let response = self.exchange_with_timeout(op, args, current_call_chain(), wait)?;
        let value = response.value.unwrap_or(Value::Null);
        serde_json::from_value(value)
            .map_err(|e| self.transport_error(format!("响应无法反序列化为目标类型：{e}")))
    }

    /// 向长驻 worker 派发一条**命令帧**（任务 #49，T2 命令通道；由
    /// [`crate::manager::PluginManager::execute_plugin_command_with_timeout`] 调用）。
    ///
    /// 五道宿主侧防线，全部落在**写帧之前**（除等待上限外没有任何「先挂起再判错」路径）：
    /// ① drain 闸门：停机窗口内拒新命令，语义与 [`SeamInvoker::invoke_with_chain`] 一致；
    /// ② pluginId 绑定校验：帧上的 `plugin_id` 必须等于本连接身份（注册表权威键），
    ///    不符 ⇒ [`error_codes::PLUGIN_COMMAND_ID_MISMATCH`]，帧不发出去 ——
    ///    「派发口发错插件」在这里就被掐死，不靠 worker 侧兜底；
    /// ③ **进程存活预检**（任务 #73 M-1）：worker 进程已退出、或读线程已因 EOF/读错误退出
    ///    ⇒ 报 [`WorkerError::NotDelivered`]（帧不可能送达 ⇒ 零副作用 ⇒ 上层据此把命令
    ///    退回 spawn-on-call 通道，兑现 `manager.rs` 契约里「worker 中途异常退出也退回
    ///    原通道」那半句）。这一条排在脏闸**之前**：「进程不在」= 回退，
    ///    「进程在但曾超时」= 拒绝，两件事的判据不许合用一个 bool（PLAN §26.3）；
    /// ④ **接缝脏闸**（任务 #73 S-3）：连接已被实证「写帧后到点不回帧」时，命令帧**立即**
    ///    被拒，不再每条各自等满 `timeout_ms + 1 s`（默认 31 s）—— 此前 #53 的止血只覆盖
    ///    接缝入口，命令通道在同一具死连接上可以无限重复地白等；
    /// ⑤ 等待上限 = 帧预算 `timeout_ms` + [`COMMAND_FRAME_GRACE_MS`] 收帧宽限：
    ///    预算边界上算完并回帧的 worker 不该被误杀，超过上限则放弃在途调用并报
    ///    [`WorkerError::Timeout`]（边界语义见 [`Self::exchange_with_timeout`]）。
    ///    任务 #56 起同一预算也约束**写回执**：写线程卡在 `write` 里时本命令在预算内拿到
    ///    `Transport`（`NotDelivered` 只留给「整帧确定未写出」，见 [`WriteAck`]）。
    ///
    /// 调用链传空：命令是宿主发起的**顶层**调用，按协议定义 `call_chain` 不含接收方，
    /// 且此时没有任何插件参与发起（worker 在命令内再发 `call_seam` 时自行携带该链）。
    pub(crate) fn execute_command(
        &self,
        frame: &CommandFrame,
    ) -> Result<CommandReply, WorkerError> {
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(self.transport_error("插件正在停机（drain），拒绝新命令帧"));
        }
        if frame.plugin_id != self.inner.plugin_id {
            return Err(self.transport_error_with_code(
                error_codes::PLUGIN_COMMAND_ID_MISMATCH,
                format!(
                    "命令帧绑定插件 `{}`，但本连接身份为 `{}`（宿主绑定不符，未发出帧）",
                    frame.plugin_id, self.inner.plugin_id
                ),
            ));
        }
        // ③ 先判「进程不在」再判「进程在但脏」：两者去向相反（回退 vs 拒绝）。
        if let Some(reason) = self.worker_gone_reason() {
            return Err(WorkerError::NotDelivered {
                plugin_id: self.inner.plugin_id.clone(),
                op: ops::COMMAND.to_string(),
                reason,
            });
        }
        // ④ 脏闸：只**读**不写（命令帧自己超时不置闸，#49 语义不变）。
        if self.inner.seam_suspect.load(Ordering::Acquire) {
            return Err(self.transport_error_with_code(
                error_codes::SEAM_PROVIDER_UNAVAILABLE,
                format!(
                    "插件 `{}` 的 worker 曾超时未回帧，连接已视为不可用（可疑），拒绝新命令帧；\
                     请在设置中禁用再启用该插件以重建连接",
                    self.inner.plugin_id
                ),
            ));
        }
        let args = serde_json::to_value(frame)
            .map_err(|e| self.transport_error(format!("命令帧序列化失败：{e}")))?;
        let wait = Duration::from_millis(frame.timeout_ms.saturating_add(COMMAND_FRAME_GRACE_MS));
        let response = self.exchange_with_timeout(ops::COMMAND, args, Vec::new(), wait)?;
        match CommandOutcome::from_frame_response(&response)
            .map_err(|e| self.transport_error(format!("命令回执形状不合法：{e}")))?
        {
            CommandOutcome::Reply(reply) => Ok(reply),
            // exchange 已把 kind=Error 的错误信封转成 Transport；能解出 Fault 只可能是
            // 「成功信封里装着 Fault」的对端违例，保守按命令失败报，绝不当成功返回。
            CommandOutcome::Fault(fault) => {
                Err(self.transport_error_with_code(fault.code.clone(), fault.message.clone()))
            },
        }
    }

    fn transport_error(&self, reason: impl Into<String>) -> WorkerError {
        WorkerError::Transport {
            plugin_id: self.inner.plugin_id.clone(),
            reason: reason.into(),
            code: None,
        }
    }

    /// 带**协议错误码**的通道错误（任务 #73 S-1）。
    ///
    /// `reason` 维持既有的 `{code}: {message}` 形态（Display 逐字不变，可读性与历史日志
    /// 口径不破坏），同时把码放进结构化字段 —— 判据读字段，从此与文案的拼装方式解耦：
    /// 任何人改 `message`、改分隔符、加前缀都不会再**静默关掉回退**（旧写法的失效形态，
    /// 审查稿 A2.3）。
    fn transport_error_with_code(
        &self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> WorkerError {
        let code = code.into();
        let message = message.into();
        WorkerError::Transport {
            plugin_id: self.inner.plugin_id.clone(),
            reason: format!("{code}: {message}"),
            code: Some(code),
        }
    }

    /// 「worker 进程已不在」的判据（任务 #73 M-1）：给出死因，`None` = 活着。
    ///
    /// 两条独立证据，任一成立即判死，且**都在写帧之前**取，因此结论永远是
    /// 「这一帧不可能被执行过」：
    /// ① `try_wait` 拿到退出码（顺带回收，不给系统留僵尸）；
    /// ② 读线程已退出 —— 子进程一死，它的 stdout 管道必关，读侧的 EOF 比父进程的
    ///    回收动作更早可见，且这是**无子进程可管**的测试夹具（`from_streams`）唯一的
    ///    死讯来源。
    ///
    /// ⚠ 与 `seam_suspect`（「进程在、但曾超时未回帧」）是两件事：本判据 ⇒ 命令**回退**，
    /// 脏闸 ⇒ 命令**拒绝**。刻意不合并成一个标记 —— 合并就是 M-1/M-2/M-3 的共同根。
    fn worker_gone_reason(&self) -> Option<String> {
        // let 链（Edition 2024）：`lock()` 的临时 guard 在整个 if 体内有效，
        // 拆成嵌套 `if` 会被 clippy 的 `collapsible_if` 判红。
        if let Some(child) = self.inner.child.lock().as_mut()
            && let Ok(Some(status)) = child.try_wait()
        {
            return Some(format!("worker 进程已退出（{status}）"));
        }
        if self.inner.connection_closed.load(Ordering::Acquire) {
            return Some("与 worker 的管道已断开（读线程已收到 EOF 或读错误）".to_string());
        }
        None
    }

    /// 进入 drain：置位拒新单，限时等在途帧自然收尾。
    ///
    /// 返回 `true` = 配对表已清空**且**入站处理任务已排空；`false` = 超时仍有悬挂
    /// （调用方可继续停机，悬挂者会经读线程退出路径拿到 `Err`，不会永久等待）。
    /// 入站侧也要等（任务 #73 M-3）：独立入站任务收尾时会经写线程回写响应帧（带预算），
    /// 不等它就进急停路径，回写会撞上已关停的写线程 —— 结果只是少一条日志，
    /// 但「停机把对端的请求吞了」这件事必须可见，故排空条件写进本函数而不是留给 warn。
    /// 只由 [`LoadedPlugin::unload`] 调用；`shutdown` / `Drop` 路径不 drain（急停语义）。
    pub fn drain(&self, timeout: Duration) -> bool {
        self.inner.draining.store(true, Ordering::Release);
        let deadline = Instant::now() + timeout;
        loop {
            if self.inner.pending.lock().is_empty()
                && self.inner.inbound_inflight.load(Ordering::Acquire) == 0
            {
                return true;
            }
            if Instant::now() >= deadline {
                tracing::warn!(
                    plugin_id = %self.inner.plugin_id,
                    "drain 超时仍有在途调用，继续停机（悬挂调用将以 Transport 错误收尾）"
                );
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 显式复位「连接可疑」闸（任务 #73 M-2）。
    ///
    /// 唯一调用点是 [`LoadedPlugin::unload`] —— 即口径「闸随 `LoadedPlugin` 生命周期，
    /// 显式 `unload`/`disable` 即清」。刻意**不提供**任何自动复位路径（下一次调用不清、
    /// 超时不清、错误不清）：自动清等于每笔重新付一遍预算，正是 D1 要止的血。
    /// 调用时机在 `shutdown()` **之后**：停机流程自己要用脏闸走急停（见
    /// [`Self::shutdown`]），先清会把急停变成礼貌等待。
    pub(crate) fn reset_seam_suspect(&self) {
        self.inner.seam_suspect.store(false, Ordering::Release);
    }

    /// worker 侧是否还有一个活着的进程/管道（任务 #73 M-1 的判据，供上层诊断）。
    pub(crate) fn worker_is_running(&self) -> bool {
        self.worker_gone_reason().is_none()
    }

    /// 优雅停机：发 shutdown 帧 → 关 stdin → 限时等退出 → 超时强杀 → 收读线程。
    ///
    /// 两条**跳过礼貌帧**的例外，去向相同（直接进急停）但理由不同，判据也分开写：
    /// - 连接已被标脏（任务 #53）：对端被实证「收到帧也不回」，等 shutdown 回执只会
    ///   把卸载一起拖挂（D1 的卸载侧变体）。任务 #56 后这条等待本身已有界，
    ///   但「已实证不回帧」的连接不值得再付一次预算，故仍跳过。
    /// - **worker 进程已不在**（任务 #73 M-1）：读线程已退出 ⇒ 配对表被 `clear()`，
    ///   此后再 `insert` 的槽位没有任何人投递或清空 ⇒ 只会在预算到点时以超时收场。
    ///   写帧本身通常先失败（管道已关），但那是 OS 的时序、不是不变式，不能当护栏。
    ///
    /// **整体有界**（任务 #56）：礼貌帧只拿总预算的一半，剩下的给「关管道 + 等退出」；
    /// 写线程若卡在某个写调用上，先杀子进程断开管道把它解阻，再短暂等它退出 ——
    /// 绝不为了 join 一个线程把停机拖成无界。
    fn shutdown(&self) {
        let deadline = Instant::now() + self.inner.shutdown_timeout;

        // ① 先发 shutdown 帧（此刻读线程还活着，能收下这次响应）。
        //    预算 = 剩余的一半：礼貌帧失败（含写侧回执超时）只是提前进急停，不是错误。
        if !self.inner.seam_suspect.load(Ordering::Acquire) && self.worker_is_running() {
            let budget = deadline.saturating_duration_since(Instant::now()) / 2;
            if !budget.is_zero() {
                let _ = self.exchange_with_timeout(ops::SHUTDOWN, Value::Null, Vec::new(), budget);
            }
        }

        // ② **真正关闭 stdin**：写线程收到 Close 即 drop 管道写端 ⇒ 对端读到 EOF，
        //    `serve` 形态的 worker 自行退出（防线①）。写线程卡住时先杀进程解阻。
        self.inner.close_writer(deadline);
        self.wait_or_kill_until(deadline);
        // ③ 进程已死 ⇒ 管道 EOF ⇒ 读线程自然退出，这里只是回收它。
        self.join_reader();
    }

    /// 限时等待子进程退出（截止 `deadline`）；超时或已僵死则强杀。
    fn wait_or_kill_until(&self, deadline: Instant) {
        loop {
            let exited = {
                let mut guard = self.inner.child.lock();
                match guard.as_mut() {
                    Some(child) => matches!(child.try_wait(), Ok(Some(_))),
                    // 无子进程（测试用的流式构造）：视作已退出。
                    None => true,
                }
            };
            if exited {
                return;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.inner.kill_child();
    }

    /// 回收读线程（`join`。读线程只在读线程自己的栈上工作，不会持 `inner` 的锁等待，
    /// 故不会与 join 互等）。
    fn join_reader(&self) {
        if let Some(handle) = self.reader.lock().take() {
            let _ = handle.join();
        }
    }
}

impl WorkerInner {
    /// 投递一帧给写线程并**带预算**等回执（任务 #56 的核心入口）。
    ///
    /// 返回 `Err` 表示**未尝试写**且连接已被判不可用（可疑闸）—— 调用方必须报
    /// [`WorkerError::Transport`]，不得报 `NotDelivered`：可疑连接的口径是「拒绝」，
    /// 不是「回退」（两者去向相反，见 [`Self::seam_suspect`] 与 `worker_gone_reason`）。
    ///
    /// 返回 `Ok(ack)` 表示帧已进入写流程，结果由三态回执给出。**持锁范围只有「克隆
    /// 投递口」这一步**：等回执期间不持任何锁 —— 否则一个卡住的写会让其它调用在锁上
    /// 无限排队，预算形同虚设。
    fn submit_frame(&self, frame: OutboundFrame, budget: Duration) -> Result<WriteAck, String> {
        if self.seam_suspect.load(Ordering::Acquire) {
            return Err("连接已视为不可用（可疑），写侧拒绝新帧".to_string());
        }
        // 写线程已关停 ⇒ 等价于「stdin 已释放」：这一帧确定一个字节都没写出去，
        // 交给调用方按 `Failed { wrote_any: false }` 判 NotDelivered（允许回退）。
        let Some(tx) = self.writer.lock().as_ref().map(|writer| writer.tx.clone()) else {
            return Ok(WriteAck::Failed {
                reason: "通道已关闭（stdin 已释放）".to_string(),
                wrote_any: false,
            });
        };
        let (ack_tx, ack_rx) = mpsc::channel();
        if tx.send(WriterCommand::Frame { frame, ack: ack_tx }).is_err() {
            // 投递口断开 = 写线程已退出且**帧未入队** ⇒ 确定未写出。
            return Ok(WriteAck::Failed {
                reason: "写线程已退出（帧未入队）".to_string(),
                wrote_any: false,
            });
        }
        match ack_rx.recv_timeout(budget) {
            Ok(ack) => Ok(ack),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // 写者仍阻塞在某个写调用上：帧可能已写出部分字节，**不能**判「未送达」。
                // 连接就此判不可用：毒化（拒绝后续） + 杀进程（关管道读端，解阻写者）。
                self.poison();
                Ok(WriteAck::NoAck)
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // 写线程在给出回执前就没了（panic / 被放弃回收）：写出进度不可知。
                self.poison();
                Ok(WriteAck::NoAck)
            },
        }
    }

    /// 把连接判为不可用：置可疑闸 + 杀子进程（关管道读端是**解阻写者**的唯一手段）。
    ///
    /// 幂等。杀进程之后 [`WorkerInvoker::worker_gone_reason`] 的 `try_wait` 分支会给出
    /// 「进程已退出」，命名命令的下一次调用因此重新回到「未送达 ⇒ 可回退」的口径。
    fn poison(&self) {
        self.seam_suspect.store(true, Ordering::Release);
        self.kill_child();
    }

    /// 关停写线程并**有界**回收它（任务 #56）。
    ///
    /// 顺序（卡死时）：先杀子进程让管道断开 → 写者的阻塞写返回错误 → 写者处理 `Close`
    /// 并 drop 管道写端 → 投递退出信号。只有在杀掉进程之后写者才可能解阻，
    /// 所以被杀路径的等待放在后面；绝不先无限 join。
    ///
    /// `deadline` 只约束**正常**收尾（写者空闲时 Close 是即时的）；被杀后的宽限固定为
    /// [`WRITER_EXIT_GRACE`]，故整体上界 = `deadline` + 一个很小常量。
    fn close_writer(&self, deadline: Instant) {
        let Some(writer) = self.writer.lock().take() else {
            return;
        };
        let _ = writer.tx.send(WriterCommand::Close);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if writer.done.recv_timeout(remaining).is_ok() {
            let _ = writer.handle.join();
            return;
        }
        // 写者仍卡在某个 write 调用上：唯一解阻手段是断开管道那一头。
        tracing::warn!(
            plugin_id = %self.plugin_id,
            "worker 写线程未在停机预算内退出（对端可能僵死且管道已满），强制杀进程以解阻"
        );
        self.kill_child();
        if writer.done.recv_timeout(WRITER_EXIT_GRACE).is_ok() {
            let _ = writer.handle.join();
        } else {
            tracing::warn!(
                plugin_id = %self.plugin_id,
                "worker 写线程在杀进程后仍未退出，放弃回收（detach；不得因此拖挂停机）"
            );
        }
    }

    /// 强杀子进程（幂等：无子进程时什么也不做）。
    fn kill_child(&self) {
        let mut guard = self.child.lock();
        if let Some(child) = guard.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 目标端自检：调用链里是否已含本插件（环）、是否已达深度上限。
///
/// **命中即 `Err`，不进入任何等待** —— 这是「环不可能死锁」的全部依据（PLAN §15.5）。
fn check_chain(plugin_id: &str, call_chain: &[String]) -> Result<(), String> {
    if call_chain.iter().any(|id| id == plugin_id) {
        return Err(format!(
            "{}：调用链 `{call_chain:?}` 中已含本插件 `{plugin_id}`（拒绝调用，未进入等待）",
            error_codes::SEAM_CALL_CYCLE_DETECTED
        ));
    }
    if call_chain.len() >= MAX_SEAM_CALL_DEPTH {
        return Err(format!(
            "{}：调用链长度 {} 已达上限 {MAX_SEAM_CALL_DEPTH}（链：{call_chain:?}）",
            error_codes::SEAM_CALL_DEPTH_EXCEEDED,
            call_chain.len()
        ));
    }
    Ok(())
}

/// 读线程主循环：消费 stdout 上的每一帧。
///
/// - **响应** → 按 `request_id` 投递给等待者（无配对者 ⇒ 警告 + 丢弃，不打断循环）；
/// - **请求** → **交独立入站任务**落地并回帧（回填 `request_id`），读线程自己**不等处理结果**
///   （任务 #73 M-3，理由见 [`spawn_inbound_handler`]）；
/// - EOF / 读错误 → 退出，置 `connection_closed`（M-1 的死讯来源之一），并**清空配对表**
///   （让所有等待者拿到 `Err`，而不是永久挂起）。
fn run_reader(inner: Arc<WorkerInner>, mut stdout: Box<dyn Read + Send>) {
    let plugin_id = inner.plugin_id.clone();
    loop {
        match read_inbound(&mut stdout) {
            Ok(Inbound::Response(response)) => {
                let request_id = response.request_id.clone();
                let waiter = request_id.as_ref().and_then(|id| inner.pending.lock().remove(id));
                match waiter {
                    Some(sender) => {
                        let _ = sender.send(response);
                    },
                    None => tracing::warn!(
                        plugin_id = %plugin_id,
                        request_id = ?request_id,
                        "收到无等待者的响应帧，已丢弃（可能是超时后被放弃的调用）"
                    ),
                }
            },
            Ok(Inbound::Request(request)) => {
                // 入站请求**不在读线程上处理**（任务 #73 M-3 的修复本体）。
                //
                // 原形态是在这里 `handle_inbound` + 就地回写：宿主替插件跑一跳（`block_on`
                // 一个 async 接缝，宿主自己的 LLM 单请求上限 300 s）期间，这条连接的读线程
                // 被整段占住 —— 于是①任何出向响应都投不出去、任何后续入站都排队，
                // ②外层出向调用按墙钟到点 ⇒ 误杀 + 置**终局**闸，③内层答案回来后插件的
                // 迟到回帧按「无等待者」丢弃 ⇒ 该插件此后所有新接缝调用被永久拒绝。
                // SDK 文档亲自示范的 `peer.call_seam(...)` 形态必踩（审查稿 A4.3）。
                //
                // 现在的分工：读线程**只**做「分流 + 投递 + 派生活任务」，永不进入宿主逻辑；
                // 入站任务的回写与出向写帧共用**同一条写线程**（串行写，帧完整性由
                // 「单写者」保证；各自的等待都带预算，见任务 #56）。
                spawn_inbound_handler(inner.clone(), request);
            },
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                tracing::warn!(plugin_id = %plugin_id, error = %e, "读帧失败，读线程退出");
                break;
            },
        }
    }
    // 死讯先登记（M-1 的判据②），再让所有等待者拿到 `Err`
    // （`recv` 因发送端被 drop 而失败），而不是永久挂起。
    inner.connection_closed.store(true, Ordering::Release);
    inner.pending.lock().clear();
}

/// 把一条入站请求交给**独立任务**处理并回写响应（任务 #73 M-3）。
///
/// ## 保序口径（正面交代，不留给读者猜）
///
/// - **不跨请求全局串行**，也**不在本连接内串行**：同一连接的多条入站请求各起一个任务并发
///   处理。串行正是 M-3 的病根（一条慢的宿主侧调用把整条连接的读侧堵死）。
///   派单要求的「只在同插件 + 同 peer 内串行」这里**刻意不采纳**，理由是判据而不是偏好：
///   一条连接的两个端点就是「本插件 + 该 worker」，按它串 = 按连接串 = 保留 head-of-line
///   阻塞 ⇒ 嵌套链照样误杀。协议侧不需要跨请求顺序 —— 见下。
/// - **帧序保证**：worker→host 的请求帧与 host→worker 的响应帧共用同一条管道，但每个响应
///   都带**回填的 `request_id`**（`plugin-proto::Peer` 按 ID 配对，ID 不匹配立即 `Err`，
///   不猜不兜底）。因此并发回写的**先后**不改变语义：唯一硬要求是「一帧要么完整写出去、
///   要么不写」，这由**写线程串行写**（[`WorkerInner::submit_frame`] → [`run_writer`]）保证。
///   写侧阻塞（对端僵死且管道满）不再落到调用方线程上，见任务 #56 的回执三态。
/// - **重入安全**：入站任务里宿主可能反过来调本插件的接缝门面 ⇒ 那是**出向**帧 +
///   出向等待，与入站任务不在一条线程上，不再有「自己等自己」的环；跨插件的
///   `A→宿主→B` 链照旧由 [`check_chain`] 的环/深度检测在写帧前拦下。
/// - **资源上界**：并发度 = 该 worker 自己发在途请求的数量（插件侧 `Peer` 是单线程同步的，
///   真并发只来自插件自己的多线程实现），没有宿主侧的队列放大。
/// - **停机**：任务收尾时经写线程回写（带预算）；若写线程已关停（停机进行中），投递得到
///   [`WriteAck::Failed`]，只留一条 warn（原形态是 `break` 掉读线程 —— 那是把「一次回写
///   失败」升级成「整条连接判死」，在并发形态下没有依据）。[`WorkerInvoker::drain`] 会等
///   在途入站任务排空。
fn spawn_inbound_handler(inner: Arc<WorkerInner>, request: FrameRequest) {
    // 计数在**派生之前**自增：出向等待者复查时，宁可早看见「宿主欠着答案」，
    // 也不能出现「任务已排队但计数还是 0」那种会被误杀的空窗。
    inner.inbound_inflight.fetch_add(1, Ordering::AcqRel);
    let plugin_id = inner.plugin_id.clone();
    let task_inner = Arc::clone(&inner);
    let thread_result = std::thread::Builder::new()
        .name(format!("plugin-worker-inbound-{plugin_id}"))
        .spawn(move || {
            // `move` 之后任务自己持有这份 `Arc<WorkerInner>`：连接被 drop 也得让这条请求
            // 有始有终（计数归还、响应回写或明确留 warn），不能悬在半路。
            let inner = task_inner;
            // 计数归还写在 guard 的 Drop 里：`handle_inbound` panic 时也必须把这一格还回去，
            // 否则出向预算会被一个已不存在的入站请求永久挂起（计数泄漏比误杀更难查）。
            let _guard = InboundGuard { inner: Arc::clone(&inner) };
            let host_peer = inner.host_peer.lock().clone();
            let mut response = match host_peer {
                Some(host_peer) => host_peer.handle_inbound(&request),
                None => FrameResponse::error(
                    error_codes::PLUGIN_UNKNOWN_OP,
                    "宿主尚未完成握手，暂不接受插件主动发起的请求",
                ),
            };
            // 回填配对 ID：否则对端无法把这一帧与它发出的请求对上。
            response.request_id = request.request_id.clone();
            // 回写同样经写线程、同样**带预算**（任务 #56）：出向写卡住时这里不会跟着
            // 无限阻塞 —— 到点按 [`WriteAck::NoAck`] 收场（连接同时被判不可用），
            // 只留一条 warn，不把「一次回写失败」升级成「整条连接判死」之外的连锁等待。
            match inner.submit_frame(OutboundFrame::Response(response), inner.seam_wait_timeout) {
                Ok(WriteAck::Written) => {},
                Ok(ack) => tracing::warn!(
                    plugin_id = %inner.plugin_id,
                    ack = ?ack,
                    "回写插件请求的响应未确认写出（出向写可能正卡在对端僵死上）"
                ),
                Err(reason) => tracing::warn!(
                    plugin_id = %inner.plugin_id,
                    reason = %reason,
                    "连接已判不可用，丢弃插件主动发起的 `{}` 请求的响应",
                    request.op
                ),
            }
        });
    if thread_result.is_err() {
        // 起不了线程（系统资源）⇒ 计数必须自己还，且给对端一个明确失败而不是沉默。
        inner.inbound_inflight.fetch_sub(1, Ordering::AcqRel);
        tracing::warn!(plugin_id = %plugin_id, "无法启动入站处理任务，该请求被丢弃");
    }
}

/// 入站任务的计数守卫：Drop 时递减 [`WorkerInner::inbound_inflight`]。
struct InboundGuard {
    inner: Arc<WorkerInner>,
}

impl Drop for InboundGuard {
    fn drop(&mut self) {
        self.inner.inbound_inflight.fetch_sub(1, Ordering::AcqRel);
    }
}

impl SeamInvoker for WorkerInvoker {
    fn invoke(&self, op: &str, args: Value) -> Result<Value, String> {
        self.invoke_with_chain(op, args, current_call_chain())
    }

    fn invoke_with_chain(
        &self,
        op: &str,
        args: Value,
        call_chain: Vec<String>,
    ) -> Result<Value, String> {
        // drain 闸门（先于环检测之后的新调用准入）：停机窗口内拒绝新帧，
        // 在途帧不受影响（它们早已过了这一行，在配对表里等响应）。
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(format!("插件 `{}` 正在停机（drain），拒绝新调用", self.inner.plugin_id));
        }
        // 「连接可疑」闸（任务 #53 修 D1）：此前已有接缝调用写帧后到点收不到回帧。
        // 对端被实证不回帧 ⇒ 不再把新调用排进同一死通道（否则每笔都要白等一个预算，
        // 僵死沿调用链级联放大）。处置=拒后续 + 停机走急停，不自动重启（理由见
        // `WorkerInner::seam_suspect` 注释）。
        //
        // 文案两件事（任务 #73 M-2）：① 带**既有协议码**
        // [`error_codes::SEAM_PROVIDER_UNAVAILABLE`] —— 「提供方当前不可用」正是本闸的语义，
        // 复用现成码 ⇒ 免 11 语言译文成本（AGENTS.md 禁区 2）；② 给**处置指引**：本闸只在
        // 显式 `unload`/`disable` 时复位，用户能做的就一件事，必须写在文案里。
        if self.inner.seam_suspect.load(Ordering::Acquire) {
            return Err(format!(
                "{}: 插件 `{}` 的 worker 曾超时未回帧，连接已视为不可用（可疑），拒绝新接缝调用；\
                 请在设置中禁用再启用该插件以重建连接",
                error_codes::SEAM_PROVIDER_UNAVAILABLE,
                self.inner.plugin_id
            ));
        }
        // 目标端自检（环 + 深度）：在**写帧之前**判定 —— 命中即拒绝、不进入等待。
        let plugin_id = self.inner.plugin_id.clone();
        check_chain(&plugin_id, &call_chain)?;
        // 通过自检 ⇒ 把自身追加到链尾，随帧一起传下去（接收方据此继续判环）。
        let mut chain = call_chain;
        chain.push(plugin_id);
        // 等待预算（D1 修复本体）：**逐接缝预算表优先**（plugin-proto
        // `SEAM_OP_TIMEOUT_OVERRIDES`，按证据定，如 `run_turn` 的 180s LLM 回合），
        // 未进表的 op 回落实 `seam_wait_timeout`（缺省即协议常量
        // [`DEFAULT_SEAM_TIMEOUT_MS`]）。写帧后收不到配对响应 ⇒ `WorkerError::Timeout`
        // 并置可疑闸；其余错误（Transport 等）不置闸 ——
        // 「对端明确回了错」与「对端装死」是两种可观测性，不能混为一谈。
        //
        // ⚠ 任务 #73 M-3 之后这条推论才成立：预算按「对端不被宿主占用」的时长消耗，
        // 嵌套 `call_seam` 期间计时挂起（见 `exchange_with_timeout`）⇒ 到点意味着
        // **真的装死**，而不是「宿主自己正占着它」。
        let wait = self.seam_wait_for_op(op);
        let response = self.exchange_with_timeout(op, args, chain, wait).map_err(|e| {
            if matches!(e, WorkerError::Timeout { .. }) {
                self.inner.seam_suspect.store(true, Ordering::Release);
            }
            e.to_string()
        })?;
        Ok(response.value.unwrap_or(Value::Null))
    }
}

impl WorkerInvoker {
    /// 该 op 的接缝等待预算：覆盖表命中用表值，否则用连接缺省预算。
    ///
    /// 任务 #56 起这个预算同时约束写侧（写回执）与收侧（等配对响应）：两者都不会
    /// 无限等待，也不再互相牵连（写线程卡住时调用方在自己的预算处收口）。
    fn seam_wait_for_op(&self, op: &str) -> Duration {
        seam_timeout_ms_for_op(op).map_or(self.inner.seam_wait_timeout, Duration::from_millis)
    }
}

impl Drop for WorkerInvoker {
    fn drop(&mut self) {
        // 兜底：调用方忘了 `unload` 也不留常驻子进程。
        // 顺序（任务 #56 定死）：先杀进程（管道那头一关，写线程的阻塞写立刻返回错误、
        // 读线程的阻塞读立刻返回 EOF）→ 再有界回收写线程（关 stdin，防线①）→ 最后
        // 回收读线程。三步都不得因为「写者卡死」而变成无界等待。
        self.inner.kill_child();
        self.inner.close_writer(Instant::now() + self.inner.shutdown_timeout);
        self.join_reader();
    }
}

// ───────────────────────── 远程门面（facade） ─────────────────────────

/// `agent.loop` 接缝的远程门面。
struct RemoteAgentTurnRunner {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
}

#[async_trait]
impl AgentTurnRunner for RemoteAgentTurnRunner {
    async fn run_turn(
        &self,
        request: AgentTurnRequest,
    ) -> axagent_harness::Result<AgentTurnResult> {
        let invoker = self.invoker.clone();
        let op = self.op.clone();
        // **跨线程前取链快照**：`spawn_blocking` 之后是另一条线程，thread-local 就丢了。
        let chain = current_call_chain();
        // 线格式按 PLAN §5.4：`args = {request}`，`value = {result}`。
        let args = json!({ "request": request });
        let value =
            tokio::task::spawn_blocking(move || invoker.invoke_with_chain(&op, args, chain))
                .await
                .map_err(|e| AxAgentError::internal(format!("等待插件响应失败：{e}")))?
                .map_err(AxAgentError::internal)?;
        serde_json::from_value(value)
            .map_err(|e| AxAgentError::internal(format!("插件响应无法解析为 AgentTurnResult：{e}")))
    }
}

/// `workflow.sandbox` 接缝的远程门面。
struct RemoteSandbox {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
}

#[async_trait]
impl WorkflowSandbox for RemoteSandbox {
    async fn execute(
        &self,
        genome: &WorkflowGenome,
        test_input: &Value,
    ) -> Result<SandboxValidationResult, String> {
        let invoker = self.invoker.clone();
        let op = self.op.clone();
        let chain = current_call_chain();
        let args = json!({ "genome": genome, "test_input": test_input });
        let value =
            tokio::task::spawn_blocking(move || invoker.invoke_with_chain(&op, args, chain))
                .await
                .map_err(|e| format!("等待插件响应失败：{e}"))??;
        serde_json::from_value(value).map_err(|e| format!("插件响应无法解析为沙箱结果：{e}"))
    }
}

/// `workflow.business_rule` 接缝的远程门面。
///
/// 本门面是**同步**的，故无需快照调用链：`invoke` 自身会读当前线程的 thread-local。
struct RemoteBusinessRule {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
}

impl BusinessRuleEvaluator for RemoteBusinessRule {
    fn evaluate(&self, node_type: &NodeKind, node_input: &Value) -> RuleEvaluationOutcome {
        let args = json!({ "node_type": node_type, "node_input": node_input });
        match self.invoker.invoke(&self.op, args) {
            Ok(value) => match serde_json::from_value(value) {
                Ok(outcome) => outcome,
                Err(e) => {
                    tracing::warn!(
                        plugin_op = %self.op,
                        error = %e,
                        "远程业务规则响应无法解析，按放行处理（不阻断工作流）"
                    );
                    RuleEvaluationOutcome::Pass
                },
            },
            Err(e) => {
                tracing::warn!(
                    plugin_op = %self.op,
                    error = %e,
                    "远程业务规则调用失败，按放行处理（不阻断工作流）"
                );
                RuleEvaluationOutcome::Pass
            },
        }
    }
}

// 说明：`BusinessRuleEvaluator` 要求 `std::fmt::Debug`，故手工实现（避免暴露 invoker 细节）。
impl std::fmt::Debug for RemoteBusinessRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteBusinessRule").field("op", &self.op).finish()
    }
}

/// `webhook.dispatch` 接缝的远程门面。
struct RemoteWebhookDispatch {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
}

#[async_trait]
impl WebhookDispatch for RemoteWebhookDispatch {
    async fn dispatch(&self, event: WebhookEvent, data: HashMap<String, Value>) -> DispatchResult {
        let invoker = self.invoker.clone();
        let op = self.op.clone();
        let chain = current_call_chain();
        let args = json!({ "event": event, "data": data });
        let result =
            tokio::task::spawn_blocking(move || invoker.invoke_with_chain(&op, args, chain)).await;
        match result {
            Ok(Ok(value)) => match serde_json::from_value(value) {
                Ok(parsed) => parsed,
                Err(e) => {
                    tracing::warn!(error = %e, "远程 webhook 派发响应无法解析，按失败记账");
                    dispatch_failed(format!("插件响应无法解析：{e}"))
                },
            },
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "远程 webhook 派发调用失败，按失败记账");
                dispatch_failed(e)
            },
            Err(e) => {
                tracing::warn!(error = %e, "等待远程 webhook 派发响应失败，按失败记账");
                dispatch_failed(format!("等待插件响应失败：{e}"))
            },
        }
    }
}

/// 远程派发失败时的降级结果：**如实记一次失败**，而不是返回「零成功零失败」的假结果。
fn dispatch_failed(reason: impl Into<String>) -> DispatchResult {
    DispatchResult { success_count: 0, failure_count: 1, errors: vec![reason.into()] }
}

// ═══════════════ §5.4 其余 7 条接缝的远程门面（P5 补齐） ═══════════════
//
// 与 P3 的 4 条**同构**：门面自身实现该接缝的 trait，内部把调用编成 JSON 帧转发，
// 经既有 `register_*` 入口入表 ⇒ 消费方拿到的仍是同一个 `Arc<dyn Xxx>`。
//
// **op 路由规则**（照抄 PLAN §5.4 映射表）：
// - 表中 op 列**唯一**的接缝（`workflow.reflector` 等）：按插件声明的 `op` 转发；
// - 表中 op 列**多个**的接缝（`session.log.invariant` / `platform.adapter`）：op 词汇由
//   接缝固定（表内逐个列出），门面直接按表转发，声明的 `op` 只作声明标记、不参与路由；
// - **表外方法明确报错**，不静默降级 —— 静默返回 `Ok(())` 会让调用方以为已生效，
//   那是最坏的失败模式（见 [`remote_method_not_supported`]）。

/// 把一次远程调用编成帧并解出目标类型（async 门面的公共入口）。
///
/// `spawn_blocking` 之前**必须先取调用链快照**：换线程后 thread-local 就是另一条链了。
async fn forward_remote<T: serde::de::DeserializeOwned>(
    invoker: Arc<dyn SeamInvoker>,
    op: &str,
    args: Value,
    what: &str,
) -> Result<T, String> {
    let op = op.to_string();
    let chain = current_call_chain();
    let value = tokio::task::spawn_blocking(move || invoker.invoke_with_chain(&op, args, chain))
        .await
        .map_err(|e| format!("等待插件响应失败：{e}"))??;
    serde_json::from_value(value).map_err(|e| format!("插件响应无法解析为 {what}：{e}"))
}

/// 转发一个**无返回值**的调用（`start` / `stop` / `send`）：成功即 `Ok(())`。
async fn forward_remote_ack(
    invoker: Arc<dyn SeamInvoker>,
    op: &str,
    args: Value,
) -> Result<(), String> {
    let op = op.to_string();
    let chain = current_call_chain();
    tokio::task::spawn_blocking(move || invoker.invoke_with_chain(&op, args, chain))
        .await
        .map_err(|e| format!("等待插件响应失败：{e}"))?
        .map(|_| ())
}

/// 转发一个**契约明定不报错**的调用（`record_reflection` / `save_cursor`）。
///
/// 这两个方法在 harness 契约里**没有 `Result`**（反思记录 / 游标是辅助数据，失败不应影响
/// 主流程），故此处 best-effort 转发并**记日志** —— 既不假装成功，也不把失败升级成错误。
async fn forward_remote_best_effort(
    invoker: Arc<dyn SeamInvoker>,
    op: &str,
    args: Value,
    seam: &str,
) {
    let op_owned = op.to_string();
    let chain = current_call_chain();
    let result =
        tokio::task::spawn_blocking(move || invoker.invoke_with_chain(&op_owned, args, chain))
            .await;
    match result {
        Ok(Ok(_)) => {},
        Ok(Err(e)) => {
            tracing::warn!(seam = %seam, op = %op, error = %e, "远程 best-effort 调用失败");
        },
        Err(e) => {
            tracing::warn!(seam = %seam, op = %op, error = %e, "等待远程 best-effort 响应失败");
        },
    }
}

/// 表外方法的统一错误：**明确报错**，不静默降级。
///
/// §5.4 只约定部分签名的线格式；未约定者若悄悄返回 `Ok(())` / `Ok(false)`，调用方会
/// 以为「远程实现已生效」。故一律拒绝，并指向计划文档（要让该方法可用，先在 §5.4 补一行）。
fn remote_method_not_supported(seam: &str, method: &str) -> String {
    format!(
        "接缝 `{seam}` 的远程门面只转发 PLAN §5.4 映射表列出的方法，`{method}` 未在表内（线格式未约定）"
    )
}

/// `workflow.reflector` 接缝的远程门面。
struct RemoteReflector {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
}

#[async_trait]
impl WorkflowReflector for RemoteReflector {
    async fn reflect(&self, record: &WorkflowExecutionRecord) -> Result<Reflection, String> {
        forward_remote(self.invoker.clone(), &self.op, json!({ "execution": record }), "Reflection")
            .await
    }

    async fn reflect_node(
        &self,
        _record: &WorkflowExecutionRecord,
        _failed_node: &NodeExecutionSnapshot,
    ) -> Result<Reflection, String> {
        Err(remote_method_not_supported("workflow.reflector", "reflect_node"))
    }

    async fn aggregate_patterns(
        &self,
        _records: &[WorkflowExecutionRecord],
    ) -> Result<Vec<WorkflowPattern>, String> {
        Err(remote_method_not_supported("workflow.reflector", "aggregate_patterns"))
    }

    async fn get_history(
        &self,
        _workflow_id: &str,
        _limit: usize,
    ) -> Result<Vec<Reflection>, String> {
        Err(remote_method_not_supported("workflow.reflector", "get_history"))
    }
}

/// `workflow.evolver` 接缝的远程门面。
struct RemoteEvolver {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
}

/// `evolve` 的响应形状：`{genome, population}`。
///
/// `population` 必须回传：trait 的 `evolve_generation` 收 `&mut EvolutionPopulation`，
/// 只回 genome 的话，插件对种群的改写会在跨进程边界处**静默丢失**（§5.4 表里那一格写的是
/// `{population}`，即「进出都含 population」的简写）。
#[derive(serde::Deserialize)]
struct EvolveReply {
    genome: WorkflowGenome,
    population: EvolutionPopulation,
}

#[async_trait]
impl WorkflowEvolver for RemoteEvolver {
    async fn initialize(&self, template_id: &str) -> Result<EvolutionPopulation, String> {
        // §5.4 表**未列** `initialize` —— 属**必要补充**：不初始化则拿不到初始种群，
        // 表内的 `evolve` 入参 `{population}` 就没有来源（它自己不会凭空造种群）。
        forward_remote(
            self.invoker.clone(),
            "initialize",
            json!({ "template_id": template_id }),
            "EvolutionPopulation",
        )
        .await
    }

    async fn evolve_generation(
        &self,
        population: &mut EvolutionPopulation,
        reflections: &[Reflection],
    ) -> Result<WorkflowGenome, String> {
        let args = json!({ "population": &*population, "reflections": reflections });
        let reply: EvolveReply =
            forward_remote(self.invoker.clone(), &self.op, args, "EvolveReply").await?;
        // 回写：调用方传的是 `&mut`，不回写就等于「插件进化了一代，宿主毫不知情」。
        *population = reply.population;
        Ok(reply.genome)
    }

    async fn run(
        &self,
        _template_id: &str,
        _reflections: &[Reflection],
    ) -> Result<WorkflowModification, String> {
        Err(remote_method_not_supported("workflow.evolver", "run"))
    }

    async fn should_auto_evolve(&self, _template_id: &str) -> Result<bool, String> {
        Err(remote_method_not_supported("workflow.evolver", "should_auto_evolve"))
    }

    async fn record_reflection(
        &self,
        template_id: &str,
        quality_score: u8,
        status: WorkflowRunStatus,
    ) {
        // 契约**无 `Result`**（辅助数据，失败不影响主流程）⇒ best-effort，不升级为错误。
        forward_remote_best_effort(
            self.invoker.clone(),
            "record_reflection",
            json!({
                "template_id": template_id,
                "quality_score": quality_score,
                "status": status,
            }),
            "workflow.evolver",
        )
        .await;
    }

    async fn set_llm_provider(&self, _provider: Arc<dyn WorkflowLlmMutator>) -> Result<(), String> {
        // 注入类方法**不可远程化**：参数是 `Arc<dyn …>`（不可跨进程），§5.4 表也未列。
        Err(remote_method_not_supported("workflow.evolver", "set_llm_provider"))
    }

    async fn set_sandbox(&self, _sandbox: Arc<dyn WorkflowSandbox>) -> Result<(), String> {
        Err(remote_method_not_supported("workflow.evolver", "set_sandbox"))
    }

    async fn set_genome_loader(
        &self,
        _loader: Arc<dyn WorkflowGenomeLoader>,
    ) -> Result<(), String> {
        Err(remote_method_not_supported("workflow.evolver", "set_genome_loader"))
    }

    async fn get_stats(&self) -> Result<EvolutionStats, String> {
        Err(remote_method_not_supported("workflow.evolver", "get_stats"))
    }

    async fn is_running(&self) -> Result<bool, String> {
        Err(remote_method_not_supported("workflow.evolver", "is_running"))
    }
}

/// `workflow.optimizer` 接缝的远程门面。
struct RemoteOptimizer {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
}

#[async_trait]
impl WorkflowOptimizer for RemoteOptimizer {
    async fn suggest(
        &self,
        template: &WorkflowTemplateData,
        reflection: &Reflection,
    ) -> Result<Vec<WorkflowSuggestion>, String> {
        forward_remote(
            self.invoker.clone(),
            &self.op,
            json!({ "template": template, "reflection": reflection }),
            "Vec<WorkflowSuggestion>",
        )
        .await
    }

    async fn suggest_batch(
        &self,
        _template: &WorkflowTemplateData,
        _reflections: &[Reflection],
    ) -> Result<Vec<WorkflowSuggestion>, String> {
        Err(remote_method_not_supported("workflow.optimizer", "suggest_batch"))
    }

    async fn apply_suggestions(
        &self,
        _template: &WorkflowTemplateData,
        _suggestions: &[WorkflowSuggestion],
    ) -> Result<WorkflowTemplateData, String> {
        Err(remote_method_not_supported("workflow.optimizer", "apply_suggestions"))
    }

    async fn estimate_impact(
        &self,
        _template: &WorkflowTemplateData,
        _suggestion: &WorkflowSuggestion,
    ) -> Result<f32, String> {
        Err(remote_method_not_supported("workflow.optimizer", "estimate_impact"))
    }
}

/// `message.callback` 接缝的远程门面。
struct RemoteMessageCallback {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
}

/// `on_message` 的响应形状：`{reply}`（`null` = 不回复）。
#[derive(serde::Deserialize)]
struct OnMessageReply {
    reply: Option<String>,
}

#[async_trait]
impl PlatformMessageCallback for RemoteMessageCallback {
    async fn on_message(
        &self,
        platform: &str,
        user_id: &str,
        username: Option<&str>,
        chat_id: &str,
        text: &str,
    ) -> Option<String> {
        let args = json!({
            "platform": platform,
            "user_id": user_id,
            "username": username,
            "chat_id": chat_id,
            "text": text,
        });
        // 本方法签名里**没有错误位**（trait 契约如此），失败只能是「不回复」，
        // 故必须留日志，否则远程失败会表现为「静默不回消息」。
        match forward_remote::<OnMessageReply>(
            self.invoker.clone(),
            &self.op,
            args,
            "OnMessageReply",
        )
        .await
        {
            Ok(reply) => reply.reply,
            Err(e) => {
                tracing::warn!(plugin_op = %self.op, error = %e, "远程消息回调失败，按「不回复」处理");
                None
            },
        }
    }

    async fn save_cursor(&self, platform: &str, cursor: i64) {
        forward_remote_best_effort(
            self.invoker.clone(),
            "save_cursor",
            json!({ "platform": platform, "cursor": cursor }),
            "message.callback",
        )
        .await;
    }
}

/// `session.log.invariant` 接缝的远程门面。
///
/// 本接缝是**同步**的（trait 无 async 方法），故直接用 `invoke`，不需要 runtime，
/// 也不存在「跨线程丢链」的问题。
///
/// **op 路由**：§5.4 该行 op 列有两个（`record` / `assert`），词汇由接缝固定，
/// 故此处不取声明里的 `op`，直接按表内字面量转发（与 `platform.adapter` 同规则）。
struct RemoteSessionLogInvariant {
    invoker: Arc<dyn SeamInvoker>,
}

impl SessionLogInvariant for RemoteSessionLogInvariant {
    fn record_model_visible(
        &self,
        session_id: &str,
        content: ModelVisibleContent,
    ) -> Result<(), InvariantViolation> {
        let args = json!({ "session_id": session_id, "content": content });
        match self.invoker.invoke("record", args) {
            Ok(_) => Ok(()),
            // 宿主侧单条即时校验的违反必须如实报出（与 assert 同口径），不得吞掉。
            Err(e) => Err(InvariantViolation { session_id: session_id.to_string(), detail: e }),
        }
    }

    fn assert_replayable(&self, session_id: &str) -> Result<(), InvariantViolation> {
        let args = json!({ "session_id": session_id });
        match self.invoker.invoke("assert", args) {
            Ok(_) => Ok(()),
            // 有不变量违反时**必须如实报出来**（本方法签名里正有错误位），不得吞掉。
            Err(e) => Err(InvariantViolation { session_id: session_id.to_string(), detail: e }),
        }
    }
}

// 说明：`SessionLogInvariant` 要求 `std::fmt::Debug`，故手工实现（避免暴露 invoker 细节）。
impl std::fmt::Debug for RemoteSessionLogInvariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSessionLogInvariant").finish()
    }
}

/// 插件回报的单个工具描述（`tool.set` 的 `list` 发现面）。
///
/// `list` 是 §5.4 表**未列**的**必要补充**：`ToolSetProvider::tools()` 必须在宿主侧返回
/// 工具对象，而 `Tool::name()` / `description()` / `input_schema()` / `category()` 是**同步**
/// 且返回借用 —— 不先把元数据取回来，宿主根本构造不出这些工具。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteToolDescriptor {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    input_schema: Value,
    /// 工具类别（serde camelCase，如 `"fileRead"`）；缺省按 `Integration` 处理
    /// （插件贡献的正是「外部集成」类工具）。
    #[serde(default)]
    category: Option<ToolCategory>,
}

/// `exec` 的响应形状（§5.4 表里写的是 `{content}`）。
///
/// 只有 `content` 必然存在；`truncated` / `metadata` / `durationMs` 是可选补充 ——
/// 插件不报这些时按「未截断、无元数据」处理。失败走帧的 `kind: error`（转成 `Err`），
/// 故此处不设 `isError`。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolExecReply {
    content: String,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    duration_ms: Option<u64>,
}

/// 插件贡献的单个工具：元数据来自注册时的 `list`，执行走 `exec`。
struct RemoteTool {
    invoker: Arc<dyn SeamInvoker>,
    name: String,
    description: String,
    input_schema: Value,
    category: ToolCategory,
}

#[async_trait]
impl Tool for RemoteTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn category(&self) -> ToolCategory {
        self.category
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // `ToolContext` 含 `Arc<dyn …>` / `Arc<Notify>`，**不可跨进程传输**，
        // 故 §5.4 的 args 只约定 `{name, args}`；此处忽略 ctx（插件在自己的进程里
        // 按自己的沙箱配置执行，宿主的 ctx 传不过去）。
        let args = json!({ "name": self.name.as_str(), "args": input });
        match forward_remote::<ToolExecReply>(self.invoker.clone(), "exec", args, "ToolExecReply")
            .await
        {
            Ok(reply) => Ok(ToolResult {
                content: reply.content,
                truncated: reply.truncated,
                is_error: false,
                metadata: reply.metadata,
                duration_ms: reply.duration_ms,
                progress: Vec::new(),
            }),
            Err(e) => Err(ToolError::execution_failed_for(&self.name, e)),
        }
    }
}

/// `tool.set` 接缝的远程门面：把插件贡献的工具集包成本地 `ToolSetProvider`。
struct RemoteToolSet {
    /// 注册时一次取回的工具（各自持有调用面，故此处不再重复存 invoker）。
    tools: Vec<Arc<dyn Tool>>,
}

impl RemoteToolSet {
    /// 向插件要一次工具清单（`list`），构造远程工具集。
    ///
    /// **这里是同步阻塞调用**：`Tool::name()` 返回借用 `&str`，元数据必须缓存，而
    /// `register_remote_seam` 是同步函数。阻塞在此可接受 —— 同一条路径上的
    /// `WorkerInvoker::handshake()` 本来就是阻塞的（`LoadedPlugin::load` 即同步流程）。
    fn load(invoker: Arc<dyn SeamInvoker>) -> Result<Self, String> {
        let value = invoker.invoke("list", json!({}))?;
        let descriptors: Vec<RemoteToolDescriptor> = serde_json::from_value(value)
            .map_err(|e| format!("`tool.set` 的 `list` 响应无法解析为工具描述数组：{e}"))?;
        let tools = descriptors
            .into_iter()
            .map(|descriptor| {
                Arc::new(RemoteTool {
                    invoker: invoker.clone(),
                    name: descriptor.name,
                    description: descriptor.description,
                    input_schema: descriptor.input_schema,
                    category: descriptor.category.unwrap_or(ToolCategory::Integration),
                }) as Arc<dyn Tool>
            })
            .collect();
        Ok(Self { tools })
    }
}

impl ToolSetProvider for RemoteToolSet {
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }
}

/// `platform.adapter.{平台名}` 接缝的远程门面。
///
/// **op 路由**：§5.4 该行 op 列有 `start` / `stop` / `send`，词汇由接缝固定，故不取声明里的
/// `op`；另有 `is_connected` / `is_enabled` 两个**查询类**补充 op（trait 要求实现，且都只需
/// 一个布尔回值，属必要补充）。
struct RemotePlatformAdapter {
    invoker: Arc<dyn SeamInvoker>,
    /// 平台名 —— 接缝 ID 是动态的，但 trait 的 `name()` 要求 `&'static str`。
    ///
    /// 名字来自运行期帧，故在注册处 `Box::leak` 一次。泄漏量被「已载入插件数 × 每插件一条
    /// platform.adapter 能力」封顶（几十字节级），是**契约限制**的代价，不是偷懒。
    platform_name: &'static str,
}

/// `is_enabled` 的响应形状：`{enabled}`。
#[derive(serde::Deserialize)]
struct EnabledReply {
    enabled: bool,
}

/// `is_connected` 的响应形状：`{connected}`。
#[derive(serde::Deserialize)]
struct ConnectedReply {
    connected: bool,
}

#[async_trait]
impl MessagePlatformAdapter for RemotePlatformAdapter {
    fn name(&self) -> &'static str {
        self.platform_name
    }

    fn is_enabled(&self, config: &PlatformConfig) -> bool {
        // 同步方法 ⇒ 走 `invoke`（与 `RemoteBusinessRule::evaluate` 同形），无需 runtime。
        let args = json!({ "config": config });
        match self.invoker.invoke("is_enabled", args) {
            Ok(value) => match serde_json::from_value::<EnabledReply>(value) {
                Ok(reply) => reply.enabled,
                Err(e) => {
                    tracing::warn!(error = %e, "远程平台适配器 is_enabled 响应无法解析，按「未启用」处理");
                    false
                },
            },
            Err(e) => {
                tracing::warn!(error = %e, "远程平台适配器 is_enabled 调用失败，按「未启用」处理");
                false
            },
        }
    }

    async fn start(&self, config: &PlatformConfig) -> anyhow::Result<()> {
        // `PlatformConfig` 是纯 DTO，可跨进程；故 start / send 把它放进 args。
        forward_remote_ack(self.invoker.clone(), "start", json!({ "config": config }))
            .await
            .map_err(anyhow::Error::msg)
    }

    async fn stop(&self) -> anyhow::Result<()> {
        forward_remote_ack(self.invoker.clone(), "stop", Value::Null)
            .await
            .map_err(anyhow::Error::msg)
    }

    async fn is_connected(&self) -> bool {
        match forward_remote::<ConnectedReply>(
            self.invoker.clone(),
            "is_connected",
            json!({}),
            "ConnectedReply",
        )
        .await
        {
            Ok(reply) => reply.connected,
            Err(e) => {
                tracing::warn!(error = %e, "远程平台适配器 is_connected 失败，按「未连接」处理");
                false
            },
        }
    }

    async fn send_message(
        &self,
        config: &PlatformConfig,
        chat_id: &str,
        text: &str,
        parse_mode: Option<&str>,
    ) -> anyhow::Result<()> {
        let args = json!({
            "config": config,
            "chat_id": chat_id,
            "text": text,
            "parse_mode": parse_mode,
        });
        forward_remote_ack(self.invoker.clone(), "send", args).await.map_err(anyhow::Error::msg)
    }
}

/// `model.provider.{provider_type}` 接缝的远程门面。
///
/// **op 路由**：词汇由接缝固定（`chat` / `list_models` / `embed`），声明里的 `op`
/// 仅作标记（与 `platform.adapter` 同规则）。`validate_key` 不占独立 op —— trait
/// 默认实现即转 `list_models`，天然落在转发路径上。
///
/// **不支持面**：`chat_stream` 明确报错（帧协议一请求一响应，无流式分片复用，不假装有）；
/// Realtime / 语音 / Batch job 族保持 trait 默认「不支持」实现，覆写与否由后续版本决定。
struct RemoteProviderAdapter {
    invoker: Arc<dyn SeamInvoker>,
    /// provider 类型名 —— 接缝 ID 是动态的，错误信息必须能指到具体 provider。
    provider_type: &'static str,
}

impl RemoteProviderAdapter {
    /// 转发失败统一映射为 `Provider` 类错误，带接缝 ID 便于定位。
    fn provider_err(&self, op: &str, reason: String) -> AxAgentError {
        AxAgentError::Provider(format!(
            "远程 provider `model.provider.{}` 的 `{op}` 调用失败：{reason}",
            self.provider_type
        ))
    }
}

#[async_trait]
impl ProviderAdapter for RemoteProviderAdapter {
    async fn chat(
        &self,
        ctx: &ProviderRequestContext,
        request: Arc<ChatRequest>,
    ) -> axagent_harness::Result<ChatResponse> {
        let args = json!({ "context": ctx, "request": &*request });
        forward_remote(self.invoker.clone(), "chat", args, "ChatResponse")
            .await
            .map_err(|e| self.provider_err("chat", e))
    }

    fn chat_stream(
        &self,
        _ctx: &ProviderRequestContext,
        _request: ChatRequest,
        _cancel_token: Option<Arc<AtomicBool>>,
    ) -> Pin<Box<dyn futures::Stream<Item = axagent_harness::Result<ChatStreamChunk>> + Send>> {
        let provider_type = self.provider_type;
        // 契约没有「不支持」哨兵值 ⇒ 唯一的诚实出口是第一帧即错误（与 harness 的
        // `unsupported_speech_stream` 同形态），不得静默产出空流假装成功。
        Box::pin(futures::stream::once(async move {
            Err(AxAgentError::Provider(format!(
                "`chat_stream` 暂不支持远程 provider `model.provider.{provider_type}`（帧协议为一请求一响应，无流式分片复用）"
            )))
        }))
    }

    async fn list_models(
        &self,
        ctx: &ProviderRequestContext,
    ) -> axagent_harness::Result<Vec<Model>> {
        let args = json!({ "context": ctx });
        forward_remote(self.invoker.clone(), "list_models", args, "Vec<Model>")
            .await
            .map_err(|e| self.provider_err("list_models", e))
    }

    async fn embed(
        &self,
        ctx: &ProviderRequestContext,
        request: EmbedRequest,
    ) -> axagent_harness::Result<EmbedResponse> {
        let args = json!({ "context": ctx, "request": request });
        forward_remote(self.invoker.clone(), "embed", args, "EmbedResponse")
            .await
            .map_err(|e| self.provider_err("embed", e))
    }
}

/// `system.prompt.{section}` 接缝的远程门面。
///
/// 单 op 接缝：按声明里的 `op`（约定为 `contribute`）转发。
/// `contribute` 契约里**没有错误位**（`None` = 跳过注入），故远程失败只能按
/// 「跳过」处理并**留日志** —— 否则表现为「提示词段神秘消失」。
struct RemoteContextContributor {
    invoker: Arc<dyn SeamInvoker>,
    op: String,
    /// 段名 —— trait `name()` 要求 `&'static str`，名字来自运行期帧 ⇒ 注册处 leak 一次
    /// （与 `RemotePlatformAdapter::platform_name` 同一契约代价）。
    section: &'static str,
}

#[async_trait]
impl ContextContributor for RemoteContextContributor {
    async fn contribute(&self, ctx: &ContextRequest<'_>) -> Option<String> {
        let args = json!({
            "session_id": ctx.session_id,
            "conversation_id": ctx.conversation_id,
            "agent_id": ctx.agent_id,
            "system_prompt": ctx.system_prompt,
            "extras": ctx.extras,
        });
        // 线格式：value 即段内容（字符串），`null` = 跳过。
        match forward_remote::<Option<String>>(
            self.invoker.clone(),
            &self.op,
            args,
            "system.prompt 段内容",
        )
        .await
        {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    section = %self.section,
                    error = %e,
                    "远程系统提示词段获取失败，按「跳过注入」处理"
                );
                None
            },
        }
    }

    fn name(&self) -> &str {
        self.section
    }
}

// ───────────────────────── L1 事件桥 ─────────────────────────

/// 一个插件的**事件投递计数**（内部可变体，B3「丢弃计数」的插件侧落点）。
///
/// 为什么计在桥这一侧而不是只靠总线：总线看到的「订阅者正常返回 `Continue`」在这里
/// 可能是「帧没发出去 / 对端回了错 / 内层预算到点」——桥一律降级成 `Continue`
/// （「不因插件异常阻断宿主」的既有口径），于是**总线的 `completed` 会 +1，而事件其实没送达插件**。
/// 两级计数各管一段，缺一个就会互相指错：
/// - 总线 `timed_out`：订阅者连降级返回都没给（真装死）；
/// - 本结构 `dropped_*`：给了返回，但返回是假的 `Continue`（投递失败）。
#[derive(Debug, Default)]
struct EventDeliveryCounters {
    /// 事件过帧成功且拿到裁决。
    delivered: AtomicU64,
    /// payload 序列化失败 ⇒ 没发帧。
    dropped_serialize: AtomicU64,
    /// 调用面返回 `Err`（对端回错、transport、drain 拒单等）。
    dropped_invoke: AtomicU64,
    /// 其中「连接已被脏闸拒」的那些（`SEAM_PROVIDER_UNAVAILABLE` 前缀）。
    ///
    /// 单列一个数而不是并进 `dropped_invoke`：脏闸一旦落下，**后续每一条事件都会
    /// 走到这里**，读数会一路疯涨。看到 `dropped_unavailable` 在涨就知道处置动作是
    /// 「禁用再启用该插件」，而不是「插件最近老出错」。
    dropped_unavailable: AtomicU64,
    /// 等待裁决本身失败（`spawn_blocking` 的 join 错误，如任务被拒）。
    dropped_wait: AtomicU64,
}

impl EventDeliveryCounters {
    fn snapshot(&self) -> EventDeliveryStats {
        EventDeliveryStats {
            delivered: self.delivered.load(Ordering::Acquire),
            dropped_serialize: self.dropped_serialize.load(Ordering::Acquire),
            dropped_invoke: self.dropped_invoke.load(Ordering::Acquire),
            dropped_unavailable: self.dropped_unavailable.load(Ordering::Acquire),
            dropped_wait: self.dropped_wait.load(Ordering::Acquire),
        }
    }
}

/// 事件投递计数快照（对外可读形态，供诊断面 / 测试断言用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EventDeliveryStats {
    /// 成功投递并拿到裁决的条数。
    pub delivered: u64,
    /// 因 payload 序列化失败丢弃的条数（没发帧）。
    pub dropped_serialize: u64,
    /// 因调用面返回错误丢弃的条数（含 [`Self::dropped_unavailable`]）。
    pub dropped_invoke: u64,
    /// 其中因连接脏闸被拒的条数。
    pub dropped_unavailable: u64,
    /// 因等待任务本身失败丢弃的条数。
    pub dropped_wait: u64,
}

impl EventDeliveryStats {
    /// 丢弃总数 —— 判据面上「这个插件吞了多少事件」的那一个数。
    #[must_use]
    pub fn dropped_total(&self) -> u64 {
        self.dropped_serialize + self.dropped_invoke + self.dropped_wait
    }
}

/// 把 worker 接上 `EventDispatchBus`：宿主收到事件时转发给插件，并把插件裁决送回总线。
///
/// 插件因此可以**订阅并改写 / 拒绝**事件（`DispatchMode::Waterfall`），
/// 而无需与事件产生者互相认识 —— 这是 L1 解耦的全部来源（PLAN §15.3）。
struct WorkerEventSubscriber {
    plugin_id: String,
    invoker: Arc<dyn SeamInvoker>,
    counters: Arc<EventDeliveryCounters>,
}

impl WorkerEventSubscriber {
    fn new(plugin_id: impl Into<String>, invoker: Arc<dyn SeamInvoker>) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            invoker,
            counters: Arc::new(EventDeliveryCounters::default()),
        }
    }

    /// 计数句柄（与 [`Self::counters`] 字段同名，但本方法给的是可克隆的 `Arc`）。
    fn delivery_counters(&self) -> Arc<EventDeliveryCounters> {
        self.counters.clone()
    }
}

#[async_trait]
impl EventSubscriber for WorkerEventSubscriber {
    async fn handle(&self, event: &DomainEvent) -> SubscriberVerdict {
        let args = match serde_json::to_value(event) {
            Ok(value) => value,
            Err(e) => {
                self.counters.dropped_serialize.fetch_add(1, Ordering::AcqRel);
                tracing::warn!(plugin_id = %self.plugin_id, error = %e, "事件序列化失败，跳过该订阅者");
                return SubscriberVerdict::Continue;
            },
        };
        let invoker = self.invoker.clone();
        let plugin_id = self.plugin_id.clone();
        let op = ops::EVENT.to_string();
        // 跨线程前取链快照：本回调可能正处在某次插件调用的链上。
        let chain = current_call_chain();
        match tokio::task::spawn_blocking(move || invoker.invoke_with_chain(&op, args, chain)).await
        {
            Ok(Ok(value)) => {
                self.counters.delivered.fetch_add(1, Ordering::AcqRel);
                verdict_from_value(&value)
            },
            Ok(Err(e)) => {
                // 脏闸拒单（前缀即协议码）与「插件真回了错」是两种处置，分开记。
                if e.starts_with(error_codes::SEAM_PROVIDER_UNAVAILABLE) {
                    self.counters.dropped_unavailable.fetch_add(1, Ordering::AcqRel);
                }
                self.counters.dropped_invoke.fetch_add(1, Ordering::AcqRel);
                tracing::warn!(%plugin_id, error = %e, "插件事件处理失败，按继续处理");
                SubscriberVerdict::Continue
            },
            Err(e) => {
                self.counters.dropped_wait.fetch_add(1, Ordering::AcqRel);
                tracing::warn!(%plugin_id, error = %e, "等待插件事件裁决失败，按继续处理");
                SubscriberVerdict::Continue
            },
        }
    }
}

/// 把插件返回的裁决 JSON 映射为 [`SubscriberVerdict`]。
///
/// 约定形态：`{"verdict":"continue"}` / `{"verdict":"rewrite","payload":{…}}` /
/// `{"verdict":"reject"}`；缺失或非法一律降级为 `Continue`（**不因插件异常阻断宿主**）。
fn verdict_from_value(value: &Value) -> SubscriberVerdict {
    match value.get("verdict").and_then(Value::as_str) {
        Some("reject") => SubscriberVerdict::Reject,
        Some("rewrite") => match value.get("payload").cloned() {
            Some(payload) => SubscriberVerdict::Rewrite(payload),
            None => SubscriberVerdict::Continue,
        },
        _ => SubscriberVerdict::Continue,
    }
}

// ───────────────────────── 载入 / 卸载 ─────────────────────────

/// 已载入的 B 层插件。持有全部可逆句柄，[`Self::unload`] 即整体回滚。
pub struct LoadedPlugin {
    plugin_id: String,
    declaration: PluginDeclaration,
    invoker: Arc<WorkerInvoker>,
    handles: Vec<EffectHandle>,
    /// 事件投递计数（仅当事件订阅真的挂上了才非 `None`）。
    ///
    /// 为什么由 `LoadedPlugin` 持有而不是只留在订阅者里：总线快照随撤销句柄一起消失，
    /// 卸载后「这个插件生前吞了多少事件」就读不到了。计数活口归持有方，
    /// 由 [`Self::event_stats`] 在卸载前取走。
    event_counters: Option<Arc<EventDeliveryCounters>>,
}

impl LoadedPlugin {
    /// 载入一个 B 层插件：spawn worker → 握手取声明 → 逐条注册能力 → 挂事件订阅。
    ///
    /// **任一步失败即整体回滚**：已注册的接缝全部撤销、worker 进程停止。
    pub fn load(config: PluginWorkerConfig) -> Result<Self, WorkerError> {
        let plugin_id = config.plugin_id.clone();
        let registry = get_capability_registry();
        let invoker = Arc::new(WorkerInvoker::spawn(&config)?);

        // 握手失败也要停掉已 spawn 的进程 —— 不能只返回错误把子进程留下。
        // 握手**超时**时 invoker 已被标脏，故下面的 `shutdown()` 跳过等待 shutdown 回执，
        // 直接进「关管道 + 限时等退出 + 强杀」急停（任务 #63：否则挂握手会变成挂停机）。
        let declaration = match invoker.handshake() {
            Ok(declaration) => declaration,
            Err(e) => {
                invoker.shutdown();
                return Err(e);
            },
        };
        // 注入声明：此后插件主动发起的 `emit` / `call_seam` 才能被受理与校验。
        invoker.set_declaration(&declaration, Arc::new(registry.clone()));
        tracing::info!(
            plugin_id = %plugin_id,
            capabilities = declaration.capabilities.len(),
            calls = declaration.calls.len(),
            subscribe = declaration.subscribe.len(),
            "B 层插件握手完成"
        );

        // 逐条注册能力；任一条失败即**逆序回滚已注册项** + 停进程（整体失败不留半挂状态）。
        let mut handles: Vec<EffectHandle> = Vec::new();
        for capability in &declaration.capabilities {
            match register_remote_seam(
                registry,
                &plugin_id,
                &capability.seam,
                &capability.op,
                invoker.clone(),
            ) {
                Ok(handle) => handles.push(handle),
                Err(e) => {
                    while let Some(handle) = handles.pop() {
                        handle.undo();
                    }
                    invoker.shutdown();
                    return Err(e);
                },
            }
        }

        let mut loaded = Self { plugin_id, declaration, invoker, handles, event_counters: None };
        if let Some((handle, counters)) = loaded.event_subscription(registry) {
            loaded.handles.push(handle);
            loaded.event_counters = Some(counters);
        }
        Ok(loaded)
    }

    /// 该插件的事件投递计数快照（B3「丢弃计数」的对外读口）。
    ///
    /// `None` 的两种形态与 [`Self::event_subscription`] 的返回条件逐字一致：
    /// 没声明 `event.dispatch`，或声明了但装载时总线还没注册。
    /// **不是**「计数为零」—— 零投递是 `Some(EventDeliveryStats::default())`，
    /// 读侧要能把「没挂订阅」与「挂了但一条没收到」分开说，这两个数的处置动作不同。
    #[must_use]
    pub fn event_stats(&self) -> Option<EventDeliveryStats> {
        self.event_counters.as_ref().map(|c| c.snapshot())
    }

    /// 插件声明（握手所得）。
    pub fn declaration(&self) -> &PluginDeclaration {
        &self.declaration
    }

    /// 插件 ID。
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// 底层调用面（供宿主侧进一步转发，如 `call_seam`）。
    pub fn invoker(&self) -> Arc<dyn SeamInvoker> {
        self.invoker.clone()
    }

    /// 经长驻 worker 执行一条命名命令（任务 #49，T2 命令通道）。
    ///
    /// 宿主侧防线（drain / pluginId 绑定 / 进程存活预检 / 接缝脏闸 / 等待上限）全部在
    /// [`WorkerInvoker::execute_command`]，此处只是转发；授权门槛与 `inputSchema` 形状校验在
    /// 调用方 [`crate::manager::PluginManager::execute_plugin_command_with_timeout`] 里、
    /// 发帧之前完成。
    pub fn execute_command(&self, frame: &CommandFrame) -> Result<CommandReply, WorkerError> {
        self.invoker.execute_command(frame)
    }

    /// worker 侧是否还活着（进程在跑且管道未断）—— 任务 #73 M-1 判据的对外投影，
    /// 供 [`crate::manager::PluginManager::plugin_worker_is_running`] 做诊断与测试探针。
    pub(crate) fn worker_is_running(&self) -> bool {
        self.invoker.worker_is_running()
    }

    /// 构造 L1 事件订阅句柄（声明里含 `event.dispatch` 时才返回 `Some`）。
    ///
    /// 一并返回计数句柄：订阅者本体随 `Arc<dyn EventSubscriber>` 交给总线，
    /// 宿主这边若不留 `Arc<EventDeliveryCounters>`，卸载后就再没人能读出「吞了多少」。
    fn event_subscription(
        &self,
        registry: &CapabilityRegistry,
    ) -> Option<(EffectHandle, Arc<EventDeliveryCounters>)> {
        if !self.declaration.subscribe.iter().any(|s| s == "event.dispatch") {
            return None;
        }
        let Some(bus) = registry.get_event_dispatcher() else {
            // 装配顺序问题必须可见（PLAN §4.3）：总线未注册时插件订阅静默失效是坑。
            tracing::warn!(
                plugin_id = %self.plugin_id,
                "插件声明订阅 event.dispatch，但事件派发总线尚未注册 —— 订阅未挂载"
            );
            return None;
        };
        let subscriber = WorkerEventSubscriber::new(self.plugin_id.clone(), self.invoker.clone());
        let counters = subscriber.delivery_counters();
        let handle = bus.subscribe_named(
            &self.plugin_id,
            EventMatcher::any(),
            Arc::new(subscriber) as Arc<dyn EventSubscriber>,
        );
        Some((handle, counters))
    }

    /// 卸载：drain 优雅收尾（PLAN §13.3）→ **逆序**撤销全部可逆句柄（LIFO，见模块头注释）
    /// → 停 worker → **清接缝脏闸**。
    ///
    /// 顺序即语义：先关闸拒新单并等在途帧回完，再摘注册（门面摘除后新消费者取不到），
    /// 最后停进程 —— 原「直接掐死在途调用」的停机窗口不复存在。
    ///
    /// 清闸排在停机**之后**（任务 #73 M-2 的复位口）：停机自己要用脏闸走急停（见
    /// [`WorkerInvoker::shutdown`]），先清会把急停变成对死连接的礼貌等待。清的动作本身
    /// 是「显式 `unload`/`disable` 即清」的落地点 —— 闸的存续范围 = 一个 `LoadedPlugin`
    /// 的生命周期，`Arc<WorkerInvoker>` 若被门面或
    /// [`crate::manager::PluginManager::plugin_invoker`] 持有
    /// 得比本实例久，停用之后也不该再被上一轮的脏标记拒绝。
    pub fn unload(mut self) {
        let drained = self.invoker.drain(self.invoker.inner.shutdown_timeout);
        if !drained {
            tracing::warn!(plugin_id = %self.plugin_id, "worker drain 未完成，按急停继续");
        }
        self.rollback();
        self.invoker.shutdown();
        self.invoker.reset_seam_suspect();
    }

    /// 仅回滚注册（不涉及进程），用于载入中途失败的清理。
    fn rollback(&mut self) {
        while let Some(handle) = self.handles.pop() {
            handle.undo();
        }
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        // 兜底回滚注册项（进程强杀由 WorkerInvoker::drop 负责）。
        self.rollback();
    }
}

impl std::fmt::Debug for LoadedPlugin {
    /// 只打标识与计数：`WorkerInvoker` 内含子进程与读线程句柄，无可读表示，
    /// 且 `PluginManager` 派生了 `Debug`，需要本实现才编得过。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("plugin_id", &self.plugin_id)
            .field("capabilities", &self.declaration.capabilities.len())
            .field("handles", &self.handles.len())
            .field("event_stats", &self.event_stats())
            .finish()
    }
}

/// 把一条能力声明注册为远程接缝实现。
///
/// **平权**：走的是与内置实现完全相同的 `register_*` 入口，入表后是同一种类型。
///
/// **op 路由**：单 op 接缝（`agent.loop` / `workflow.sandbox` / `workflow.business_rule` /
/// `workflow.reflector` / `workflow.evolver` / `workflow.optimizer` / `message.callback` /
/// `webhook.dispatch` / `tool.set` / `system.prompt.{段名}`）取声明里的 `op`；多 op 接缝
/// （`session.log.invariant` / `platform.adapter` / `model.provider`）的 op 词汇由接缝固定，
/// 声明里的 `op` 仅作标记、不参与路由（对应门面直接用字面量转发）。
fn register_remote_seam(
    registry: &CapabilityRegistry,
    plugin_id: &str,
    seam: &str,
    op: &str,
    invoker: Arc<dyn SeamInvoker>,
) -> Result<EffectHandle, WorkerError> {
    // 早守卫：不支持的接缝一次拦下（含 `platform.adapter.{平台名}` 这类动态前缀接缝）。
    if !is_supported_remote_seam(seam) {
        return Err(WorkerError::UnsupportedSeam {
            plugin_id: plugin_id.to_string(),
            seam: seam.to_string(),
        });
    }
    let op = op.to_string();
    // 动态接缝 `platform.adapter.{平台名}`：接缝 ID 由插件声明，多实例共存。
    let handle = if let Some(platform_name) = platform_name_from_seam(seam) {
        // trait 的 `name()` 要求 `&'static str`，而名字来自运行期帧 ⇒ 在注册处 leak 一次。
        let leaked: &'static str = Box::leak(platform_name.to_string().into_boxed_str());
        registry.register_platform_adapter(
            platform_name,
            Arc::new(RemotePlatformAdapter { invoker, platform_name: leaked }),
        )
    } else if let Some(provider_type) = provider_type_from_seam(seam) {
        let leaked: &'static str = Box::leak(provider_type.to_string().into_boxed_str());
        registry.register_model_provider(
            provider_type,
            Arc::new(RemoteProviderAdapter { invoker, provider_type: leaked }),
        )
    } else if let Some(section) = prompt_section_from_seam(seam) {
        let leaked: &'static str = Box::leak(section.to_string().into_boxed_str());
        registry.register_system_prompt_section(
            section,
            Arc::new(RemoteContextContributor { invoker, op, section: leaked }),
        )
    } else {
        match seam {
            "agent.loop" => {
                registry.register_agent_loop(Arc::new(RemoteAgentTurnRunner { invoker, op }))
            },
            "workflow.sandbox" => {
                registry.register_sandbox(Arc::new(RemoteSandbox { invoker, op }))
            },
            "workflow.business_rule" => {
                registry.register_business_rule(Arc::new(RemoteBusinessRule { invoker, op }))
            },
            "workflow.reflector" => {
                registry.register_workflow_reflector(Arc::new(RemoteReflector { invoker, op }))
            },
            "workflow.evolver" => {
                registry.register_workflow_evolver(Arc::new(RemoteEvolver { invoker, op }))
            },
            "workflow.optimizer" => {
                registry.register_workflow_optimizer(Arc::new(RemoteOptimizer { invoker, op }))
            },
            "message.callback" => {
                registry.register_message_callback(Arc::new(RemoteMessageCallback { invoker, op }))
            },
            "webhook.dispatch" => {
                registry.register_webhook_dispatch(Arc::new(RemoteWebhookDispatch { invoker, op }))
            },
            "session.log.invariant" => registry
                .register_session_log_invariant(Arc::new(RemoteSessionLogInvariant { invoker })),
            "tool.set" => {
                // 元数据必须在注册时一次性取回：`Tool::name()` 返回借用 `&str`。
                let tool_set =
                    RemoteToolSet::load(invoker).map_err(|reason| WorkerError::Register {
                        plugin_id: plugin_id.to_string(),
                        seam: seam.to_string(),
                        reason,
                    })?;
                registry.register_tool_set(Arc::new(tool_set))
            },
            other => {
                return Err(WorkerError::UnsupportedSeam {
                    plugin_id: plugin_id.to_string(),
                    seam: other.to_string(),
                });
            },
        }
    };
    handle.map_err(|e| WorkerError::Register {
        plugin_id: plugin_id.to_string(),
        seam: seam.to_string(),
        reason: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::{
        AsOfDataScope, AsOfSource, DispatchMode, EventCategory, EventDispatchBus, ExecutionView,
        WorkflowRunActivePage, WorkflowRunActiveView, WorkflowRunCancelOutcome, WorkflowRunHandle,
        WorkflowRunProvider, WorkflowRunRequest,
    };
    use axagent_plugin_proto::{CommandFault, Peer};
    use std::net::{Shutdown, TcpListener, TcpStream};

    #[test]
    fn verdict_maps_continue_rewrite_reject() {
        assert_eq!(
            verdict_from_value(&json!({ "verdict": "continue" })),
            SubscriberVerdict::Continue
        );
        assert_eq!(verdict_from_value(&json!({ "verdict": "reject" })), SubscriberVerdict::Reject);
        assert_eq!(
            verdict_from_value(&json!({ "verdict": "rewrite", "payload": { "q": 7 } })),
            SubscriberVerdict::Rewrite(json!({ "q": 7 }))
        );
    }

    #[test]
    fn verdict_falls_back_to_continue_on_unknown_shape() {
        // 缺 verdict / 未知取值 / rewrite 缺 payload / 非对象 —— 一律降级为继续，不阻断宿主
        assert_eq!(verdict_from_value(&Value::Null), SubscriberVerdict::Continue);
        assert_eq!(verdict_from_value(&json!({ "verdict": "??" })), SubscriberVerdict::Continue);
        assert_eq!(
            verdict_from_value(&json!({ "verdict": "rewrite" })),
            SubscriberVerdict::Continue
        );
    }

    #[test]
    fn unsupported_seam_is_rejected_with_seam_id() {
        let registry = get_capability_registry();
        // `event.dispatch` 是**订阅**（L1 事件桥）而非「可远程实现的接缝」，不在 §5.4 表内。
        let err = register_remote_seam(
            registry,
            "demo@external",
            "event.dispatch",
            "dispatch",
            Arc::new(NoopInvoker),
        )
        .expect_err("未支持远程化的接缝必须被拒绝，而不是静默放过");
        match err {
            WorkerError::UnsupportedSeam { plugin_id, seam } => {
                assert_eq!(plugin_id, "demo@external");
                assert_eq!(seam, "event.dispatch");
            },
            other => panic!("期望 UnsupportedSeam，实际 {other}"),
        }
    }

    /// §5.4 映射表**全部**可作为远程实现的接缝，一条不漏。
    #[test]
    fn supported_seam_list_covers_p3_and_p5_facades() {
        let expected = [
            "agent.loop",
            "workflow.sandbox",
            "workflow.reflector",
            "workflow.evolver",
            "workflow.optimizer",
            "workflow.business_rule",
            "message.callback",
            "webhook.dispatch",
            "session.log.invariant",
            "platform.adapter",
            "tool.set",
            "model.provider",
            "system.prompt",
        ];
        for seam in expected {
            assert!(SUPPORTED_REMOTE_SEAMS.contains(&seam), "门面清单应含 {seam}");
            assert!(is_supported_remote_seam(seam), "is_supported_remote_seam 应认 {seam}");
        }
        // 动态前缀接缝：`platform.adapter.{平台名}` 走前缀匹配。
        assert!(is_supported_remote_seam("platform.adapter.telegram"));
        // 前缀必须落在 `.` 边界上 —— `platform.adapternope` 不是合法接缝。
        assert!(!is_supported_remote_seam("platform.adapternope"));
        // 本期新增的两条动态接缝（PLAN-plugin-gap-closure §3）同样按 `.` 边界判定。
        assert!(is_supported_remote_seam("model.provider.deepseek"));
        assert!(is_supported_remote_seam("system.prompt.memory"));
        assert!(!is_supported_remote_seam("model.providernope"));
        assert!(!is_supported_remote_seam("system.promptx"));
        // 空后缀（只有前缀点）不是合法接缝 ID。
        assert!(provider_type_from_seam("model.provider.").is_none());
        assert!(prompt_section_from_seam("system.prompt.").is_none());
    }

    /// 单 op 接缝按**声明里的 `op`** 转发（此处以 `workflow.reflector` 为例）。
    #[tokio::test]
    async fn remote_reflector_forwards_declared_op() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "workflow.reflector",
            "reflect",
            Arc::new(ScriptedInvoker::new(&[(
                "reflect",
                serde_json::to_value(Reflection::new("exec-1".to_string())).expect("serialize"),
            )])),
        )
        .expect("远程接缝应注册成功");
        let reflector = registry.get_workflow_reflector().expect("远程实现应可取回");
        let record = sample_execution_record();
        let out = reflector.reflect(&record).await.expect("转发应成功");
        assert_eq!(out.task_id, "exec-1");
        // 表外方法明确报错，不静默降级。
        let err = reflector.get_history("wf-1", 10).await.expect_err("表外方法必须报错");
        assert!(err.contains("workflow.reflector"), "错误应指向接缝：{err}");
    }

    /// `workflow.evolver` 的 `evolve` 必须**双向**带种群：插件对 `&mut population` 的改写
    /// 要回传到宿主，否则跨进程边界处静默丢失。
    #[tokio::test]
    async fn remote_evolver_round_trips_population() {
        let genome = sample_genome();
        let mut population = EvolutionPopulation {
            generation: 0,
            individuals: vec![genome.clone()],
            best_fitness: 0.0,
            avg_fitness: 0.0,
            fitness_history: Vec::new(),
        };
        let evolved = EvolutionPopulation {
            generation: 1,
            individuals: vec![genome.clone()],
            best_fitness: 1.0,
            avg_fitness: 1.0,
            fitness_history: vec![1.0],
        };
        let reply = json!({
            "genome": serde_json::to_value(&genome).expect("serialize"),
            "population": serde_json::to_value(&evolved).expect("serialize"),
        });
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "workflow.evolver",
            "evolve",
            Arc::new(ScriptedInvoker::new(&[("evolve", reply)])),
        )
        .expect("远程接缝应注册成功");
        let evolver = registry.get_workflow_evolver().expect("远程实现应可取回");

        let returned = evolver.evolve_generation(&mut population, &[]).await.expect("转发应成功");
        assert_eq!(returned.template_id, genome.template_id);
        assert_eq!(population.generation, 1, "插件回传的种群必须写回宿主侧的 `&mut`");
        assert_eq!(population.best_fitness, 1.0);
    }

    /// `webhook.dispatch` 之外，`message.callback` / `workflow.optimizer` 也走同一条转发路径。
    #[tokio::test]
    async fn remote_message_callback_returns_reply() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "message.callback",
            "on_message",
            Arc::new(ScriptedInvoker::new(&[("on_message", json!({ "reply": "pong" }))])),
        )
        .expect("远程接缝应注册成功");
        let callback = registry.get_message_callback().expect("远程实现应可取回");
        let reply = callback.on_message("telegram", "u1", Some("Alice"), "c1", "ping").await;
        assert_eq!(reply.as_deref(), Some("pong"));
    }

    /// `session.log.invariant` 是同步接缝：`record` / `assert` 的违反必须**如实报出**，
    /// 不得吞掉。
    #[test]
    fn remote_session_log_invariant_reports_violation() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "session.log.invariant",
            "",
            Arc::new(ScriptedInvoker::new(&[
                ("record", json!(null)),
                ("assert", json!({ "ok": true })),
            ])),
        )
        .expect("远程接缝应注册成功");
        let log = registry.get_session_log_invariant().expect("远程实现应可取回");
        log.record_model_visible(
            "s1",
            ModelVisibleContent {
                role: "user".to_string(),
                text: "hi".to_string(),
                thinking: String::new(),
                tool_names: Vec::new(),
                tool_call_id: None,
                non_text_parts: Vec::new(),
                content_hash: String::new(),
            },
        )
        .expect("宿主侧即时校验通过时应为 Ok");
        assert!(log.assert_replayable("s1").is_ok(), "无违反时应为 Ok");

        // record 的宿主侧违反 / IPC 失败必须如实透传（未脚本化 ⇒ invoke 返回 Err）
        let registry2 = CapabilityRegistry::new();
        register_remote_seam(
            &registry2,
            "demo2@external",
            "session.log.invariant",
            "",
            Arc::new(ScriptedInvoker::new(&[])),
        )
        .expect("远程接缝应注册成功");
        let log2 = registry2.get_session_log_invariant().expect("远程实现应可取回");
        let err = log2
            .record_model_visible(
                "s1",
                ModelVisibleContent {
                    role: "user".to_string(),
                    text: "hi".to_string(),
                    thinking: String::new(),
                    tool_names: Vec::new(),
                    tool_call_id: None,
                    non_text_parts: Vec::new(),
                    content_hash: String::new(),
                },
            )
            .unwrap_err();
        assert_eq!(err.session_id, "s1");
        assert!(err.detail.contains("record"), "违反细节应原样透传: {}", err.detail);
    }

    /// `tool.set`：注册时先 `list` 取回工具元数据，再按名 `exec`。
    #[tokio::test]
    async fn remote_tool_set_discovers_and_executes_tools() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "tool.set",
            "exec",
            Arc::new(ScriptedInvoker::new(&[
                (
                    "list",
                    json!([{
                        "name": "remote_echo",
                        "description": "Echo from plugin",
                        "inputSchema": { "type": "object" },
                    }]),
                ),
                ("exec", json!({ "content": "ok" })),
            ])),
        )
        .expect("远程接缝应注册成功");
        let tool_set = registry.get_tool_set().expect("远程实现应可取回");
        let tools = tool_set.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "remote_echo");

        let result = tools[0]
            .call(json!({ "message": "hi" }), &ToolContext::new("."))
            .await
            .expect("转发应成功");
        assert!(!result.content.is_empty(), "工具应返回内容");
    }

    /// `platform.adapter` 是**动态**接缝：接缝 ID 决定平台名，且能按名取回。
    #[test]
    fn remote_platform_adapter_uses_dynamic_seam_id() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "platform.adapter.telegram",
            "start",
            Arc::new(ScriptedInvoker::new(&[("is_enabled", json!({ "enabled": true }))])),
        )
        .expect("远程接缝应注册成功");
        let adapter = registry.get_platform_adapter("telegram").expect("应按平台名取回远程实现");
        assert_eq!(adapter.name(), "telegram");
        assert!(adapter.is_enabled(&PlatformConfig::default()));
        assert!(registry.get_platform_adapter("discord").is_none(), "未注册的平台名不应被命中");
    }

    /// `model.provider.{类型名}` 是**动态**接缝：按类型名注册与取回（多实例共存）。
    #[test]
    fn remote_model_provider_uses_dynamic_seam_id() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "model.provider.demo",
            "chat",
            Arc::new(ScriptedInvoker::new(&[])),
        )
        .expect("model.provider 远程接缝应注册成功");
        assert!(registry.get_model_provider("demo").is_some(), "应按类型名取回远程 provider");
        assert!(registry.list_model_providers().contains(&"demo".to_string()));
        assert!(registry.get_model_provider("other").is_none(), "未注册的类型名不应被命中");
    }

    /// `chat` / `list_models` 的线格式：args 带 `context` + `request`（chat），
    /// value 即结果本体；`validate_key` 经 trait 默认实现落到 `list_models`。
    #[tokio::test]
    async fn remote_provider_chat_forwards_context_and_request() {
        let registry = CapabilityRegistry::new();
        let recorder = Arc::new(RecordingInvoker::new(
            serde_json::to_value(ChatResponse::default()).expect("serialize"),
        ));
        register_remote_seam(
            &registry,
            "demo@external",
            "model.provider.demo",
            "chat",
            recorder.clone(),
        )
        .expect("远程接缝应注册成功");
        let adapter = registry.get_model_provider("demo").expect("远程 provider 应可取回");

        let response = adapter.chat(&sample_provider_ctx(), Arc::new(ChatRequest::default())).await;
        assert!(response.is_ok(), "chat 转发应成功：{response:?}");
        let (op, args) = recorder.last_call().expect("应有一次转发");
        assert_eq!(op, "chat");
        // `ProviderRequestContext` 以 camelCase 出线（禁区 13），插件侧才能对上 TS 惯例。
        assert_eq!(
            args.get("context").and_then(|c| c.get("apiKey")).and_then(Value::as_str),
            Some("sk-demo"),
            "args.context 应为序列化后的调用上下文：{args}"
        );
        assert!(args.get("request").is_some(), "chat 必须把 ChatRequest 带给插件");

        // `list_models` 只带 context；默认实现的 `validate_key` 复用它。
        let lister = Arc::new(RecordingInvoker::new(json!([])));
        let list_adapter = RemoteProviderAdapter { invoker: lister.clone(), provider_type: "demo" };
        assert!(list_adapter.list_models(&sample_provider_ctx()).await.expect("应成功").is_empty());
        let (op, _) = lister.last_call().expect("应有一次转发");
        assert_eq!(op, "list_models");
        assert!(
            list_adapter
                .validate_key(&sample_provider_ctx())
                .await
                .expect("默认实现应复用 list_models")
        );
    }

    /// `chat_stream` 在远程门面是**明确不支持**面：第一帧即错误，绝不静默产出空流。
    #[tokio::test]
    async fn remote_provider_stream_is_explicitly_unsupported() {
        let adapter = RemoteProviderAdapter {
            invoker: Arc::new(ScriptedInvoker::new(&[])),
            provider_type: "demo",
        };
        use futures::StreamExt;
        let first = adapter
            .chat_stream(&sample_provider_ctx(), ChatRequest::default(), None)
            .next()
            .await
            .expect("应立即产出一帧");
        let err = first.expect_err("chat_stream 必须明确失败");
        assert!(err.to_string().contains("chat_stream"), "错误应指向方法名：{err}");
    }

    /// `system.prompt.{段名}` 是**动态**接缝：按段名注册，`contribute` 返回文本或跳过。
    #[tokio::test]
    async fn remote_system_prompt_section_contributes_or_skips() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "system.prompt.stock_reflection",
            "contribute",
            Arc::new(ScriptedInvoker::new(&[("contribute", json!("插件注入的段落"))])),
        )
        .expect("system.prompt 远程接缝应注册成功");
        // 裸前缀（无段名后缀）必须被拒 —— 动态接缝的 ID 必须带后缀。
        register_remote_seam(
            &registry,
            "demo@external",
            "system.prompt",
            "contribute",
            Arc::new(ScriptedInvoker::new(&[])),
        )
        .expect_err("裸前缀不是合法接缝 ID，应被拒绝");

        let extras = HashMap::new();
        let system_prompt = vec!["base".to_string()];
        let ctx = ContextRequest {
            session_id: "s1",
            conversation_id: Some("c1"),
            agent_id: None,
            system_prompt: &system_prompt,
            extras: &extras,
        };
        let contributor = registry
            .list_system_prompt_sections()
            .into_iter()
            .find(|contributor| contributor.name() == "stock_reflection")
            .expect("应按段名取回远程贡献者");
        assert_eq!(contributor.contribute(&ctx).await.as_deref(), Some("插件注入的段落"));

        // `null` = 该段本轮跳过（契约无错误位，不得升级成 panic）。
        let skipper = RemoteContextContributor {
            invoker: Arc::new(ScriptedInvoker::new(&[("contribute", Value::Null)])),
            op: "contribute".to_string(),
            section: "skipped",
        };
        assert_eq!(skipper.contribute(&ctx).await, None);
        // 远程失败同样只能按「跳过」处理。
        let broken = RemoteContextContributor {
            invoker: Arc::new(ScriptedInvoker::new(&[])),
            op: "contribute".to_string(),
            section: "broken",
        };
        assert_eq!(broken.contribute(&ctx).await, None);
    }

    // ── 远程门面 / 事件桥：纯内存验证，不启动任何进程 ──
    // 真实子进程的生命周期用例（worker 崩溃 / 僵死强杀）属集成测试，需 `AXAGENT_TEST_PLUGIN_SUBPROCESS=1`
    // 显式门控（参照 `lib.rs` 的 `require_plugin_subprocess` 模式）。

    /// 门面透明性：同一接缝分别以**内置**与**远程**注册，消费方消费行为一致。
    ///
    /// 这是「平权」的验收点 —— 消费方（这里用 `get_webhook_dispatch()` 代表）
    /// 对实现来自内置还是插件**一无所知**：拿到的是同一个 `Arc<dyn WebhookDispatch>`。
    #[tokio::test]
    async fn remote_facade_is_consumed_like_native() {
        let native_registry = CapabilityRegistry::new();
        native_registry
            .register_webhook_dispatch(Arc::new(NativeWebhookDispatch))
            .expect("内置实现应注册成功");
        let native = native_registry.get_webhook_dispatch().expect("内置实现应可取回");

        let remote_registry = CapabilityRegistry::new();
        register_remote_seam(
            &remote_registry,
            "demo@external",
            "webhook.dispatch",
            "dispatch",
            Arc::new(ScriptedInvoker::new(&[(
                "dispatch",
                json!({ "successCount": 1, "failureCount": 0, "errors": [] }),
            )])),
        )
        .expect("远程接缝应注册成功");
        let remote = remote_registry.get_webhook_dispatch().expect("远程实现应可取回");

        let native_out = native.dispatch(WebhookEvent::ToolComplete, HashMap::new()).await;
        let remote_out = remote.dispatch(WebhookEvent::ToolComplete, HashMap::new()).await;
        assert_eq!(native_out.success_count, remote_out.success_count);
        assert_eq!(native_out.failure_count, remote_out.failure_count);
        assert_eq!(native_out.errors, remote_out.errors);
    }

    /// 远程调用失败必须**如实记一次失败**，不得返回「零成功零失败」的假结果
    ///（那会让上层以为「派发成功但无人接收」）。
    #[tokio::test]
    async fn remote_webhook_failure_is_recorded_not_silently_ok() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "webhook.dispatch",
            "dispatch",
            Arc::new(ScriptedInvoker::new(&[])),
        )
        .expect("远程接缝应注册成功");
        let remote = registry.get_webhook_dispatch().expect("远程实现应可取回");

        let out = remote.dispatch(WebhookEvent::AgentEnd, HashMap::new()).await;
        assert_eq!(out.success_count, 0);
        assert_eq!(out.failure_count, 1, "远程失败必须记一次失败");
        assert_eq!(out.errors.len(), 1);
    }

    /// L1 事件桥：插件（此处以脚本化调用面代表 worker）的 `rewrite` 裁决能被总线采纳。
    #[tokio::test]
    async fn worker_event_bridge_can_rewrite_in_waterfall() {
        let bus = EventDispatchBus::new();
        let subscriber: Arc<dyn EventSubscriber> = Arc::new(WorkerEventSubscriber::new(
            "demo@external",
            Arc::new(ScriptedInvoker::new(&[(
                ops::EVENT,
                json!({ "verdict": "rewrite", "payload": { "kind": "rewritten" } }),
            )])),
        ));
        let _handle = bus.subscribe(EventMatcher::any(), subscriber);

        let mut event = DomainEvent::new(
            EventCategory::Workflow,
            "NodeStarted",
            json!({ "kind": "original" }),
            "rt-workflow",
        );
        let outcome = bus.dispatch(&mut event, DispatchMode::Waterfall).await;

        assert_eq!(outcome.invoked, 1, "worker 订阅者应被调用");
        assert_eq!(
            outcome.rewritten,
            Some(json!({ "kind": "rewritten" })),
            "插件裁决的 payload 应被总线采纳"
        );
        assert!(!outcome.rejected);
    }

    /// `#140`：`ops::EVENT` 走**真实帧往返**（回环 socket + 与生产同源的 `WorkerInvoker`），
    /// 实测「订阅回调 → 写帧 → 假 worker 回帧 → 裁决」的宿主侧耗时。
    ///
    /// 判据只钉「没有走错通道」级别：正常应远小于 1 ms，这里给 2 s 上界（≈2000× 余量）——
    /// 既能在 CI 抖动下稳定，又能抓住「帧没发出去、一路等到默认预算」这类回归。
    /// 实测值由 `--nocapture` 打印，供 `SEAM_OP_TIMEOUT_OVERRIDES` 的证据栏引用。
    #[tokio::test]
    async fn event_op_round_trip_is_measured_and_stays_under_default_budget() {
        let (host_side, mut worker_side) = tcp_pair();
        // 留下一个对端控制句柄：`host_side.try_clone()` 让写端/读端各持一个 handle，
        // 只 drop 写端**不会**让对端读到 EOF ⇒ 收尾必须由本句柄显式 shutdown（见测试末尾）。
        let worker_ctl = worker_side.try_clone().expect("克隆回环连接（对端控制）");
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let _worker = std::thread::spawn(move || {
            loop {
                let mut len_buf = [0u8; 4];
                if worker_side.read_exact(&mut len_buf).is_err() {
                    break;
                }
                let len = u32::from_be_bytes(len_buf) as usize;
                let mut payload = vec![0u8; len];
                if worker_side.read_exact(&mut payload).is_err() {
                    break;
                }
                let Ok(request) = serde_json::from_slice::<FrameRequest>(&payload) else {
                    break;
                };
                let response = FrameResponse::success(json!({ "verdict": "continue" }))
                    .with_id(request.request_id.clone().unwrap_or_default());
                if write_json_frame(&mut worker_side, &response).is_err() {
                    break;
                }
            }
        });

        let invoker = Arc::new(WorkerInvoker::from_streams(
            "event-measure@external",
            Box::new(host_reader),
            Box::new(host_side),
        ));
        let subscriber = WorkerEventSubscriber::new("event-measure@external", invoker.clone());
        let event = bridge_event();

        // 预热一次（首次会带着 spawn_blocking 线程池的建立成本）。
        let _ = subscriber.handle(&event).await;
        const N: usize = 200;
        let mut samples = Vec::with_capacity(N);
        for _ in 0..N {
            let started = Instant::now();
            let verdict = subscriber.handle(&event).await;
            samples.push(started.elapsed());
            assert!(matches!(verdict, SubscriberVerdict::Continue), "裁决应原样透传");
        }
        samples.sort_unstable();
        let p50 = samples[N / 2];
        let p99 = samples[N * 99 / 100];
        let max = *samples.last().expect("N > 0");
        eprintln!("#140 event 往返：n={N} p50={p50:?} p99={p99:?} max={max:?}");
        assert_eq!(
            subscriber.delivery_counters().snapshot().delivered,
            (N + 1) as u64,
            "每个事件都应记为一次成功投递（含预热）"
        );
        assert!(
            max < Duration::from_secs(2),
            "event 往返最大 {max:?} 超过 2s —— 疑似没走帧通道/一路等到默认预算"
        );
        assert!(
            max < Duration::from_millis(DEFAULT_SEAM_TIMEOUT_MS),
            "event 往返必须远低于默认接缝预算，实际 {max:?}"
        );

        // 收尾（否则读线程阻塞在关闭的连接上，测试会挂到超时）：
        // 断对端 ⇒ 假 worker 读到 EOF 退出 ⇒ 读线程登记断连；随后才做 shutdown。
        let _ = worker_ctl.shutdown(Shutdown::Both);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !invoker.inner.connection_closed.load(Ordering::Acquire) && Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(subscriber);
        invoker.shutdown();
    }

    /// 固定回错的测试替身：`message` 原样作为 `Err` 返回（用来分reason计数）。
    struct FailingInvoker {
        message: String,
    }

    impl SeamInvoker for FailingInvoker {
        fn invoke(&self, _op: &str, _args: Value) -> Result<Value, String> {
            Err(self.message.clone())
        }
    }

    fn bridge_event() -> DomainEvent {
        DomainEvent::new(EventCategory::Workflow, "NodeStarted", json!({ "n": 1 }), "rt-workflow")
    }

    /// 桥按**原因**分桶计数：成功投递 / 调用回错 / 脏闸拒单，三者不能混成一个数。
    #[tokio::test]
    async fn event_bridge_counts_drops_by_reason() {
        // ① 正常投递。
        let ok = WorkerEventSubscriber::new(
            "demo@external",
            Arc::new(ScriptedInvoker::new(&[(ops::EVENT, json!({ "verdict": "continue" }))])),
        );
        ok.handle(&bridge_event()).await;
        assert_eq!(
            ok.delivery_counters().snapshot(),
            EventDeliveryStats { delivered: 1, ..Default::default() },
            "过帧成功只该动 delivered"
        );

        // ② 调用面回错（未脚本化）。
        let failed =
            WorkerEventSubscriber::new("demo@external", Arc::new(ScriptedInvoker::new(&[])));
        failed.handle(&bridge_event()).await;
        let s = failed.delivery_counters().snapshot();
        assert_eq!((s.delivered, s.dropped_invoke, s.dropped_unavailable), (0, 1, 0));
        assert_eq!(s.dropped_total(), 1, "丢弃总数按 reason 桶相加");

        // ③ 脏闸拒单：既进 `dropped_invoke`（它确实是一次 invoke 失败），
        //    也进 `dropped_unavailable`（子集，用于指认处置动作）—— 口径要在测试里钉住，
        //    否则读侧会把两个桶相加算出「两倍丢弃」。
        let gated = WorkerEventSubscriber::new(
            "demo@external",
            Arc::new(FailingInvoker {
                message: format!("{}: 连接已视为不可用", error_codes::SEAM_PROVIDER_UNAVAILABLE),
            }),
        );
        gated.handle(&bridge_event()).await;
        let s = gated.delivery_counters().snapshot();
        assert_eq!(
            (s.dropped_invoke, s.dropped_unavailable, s.dropped_total()),
            (1, 1, 1),
            "dropped_unavailable 是 dropped_invoke 的子集，不是第四路丢弃"
        );
    }

    /// 两级计数的分工（本批改动的核心判据）：
    /// 插件侧投递失败时，桥**仍要返回 `Continue`**（不因插件异常阻断宿主），
    /// 于是总线那侧看到的是「正常完成」，`timed_out` 为 0。
    ///
    /// 反过来说明为什么两级都要有：**只看总线会把投递成功高估**（这里 completed=1 而
    /// delivered=0），**只看桥则不知道订阅者是否真装死**。
    #[tokio::test]
    async fn bus_sees_completed_while_bridge_records_a_drop() {
        let bus = EventDispatchBus::new();
        let subscriber =
            WorkerEventSubscriber::new("demo@external", Arc::new(ScriptedInvoker::new(&[])));
        let counters = subscriber.delivery_counters();
        let _h = bus.subscribe_named("demo@external", EventMatcher::any(), Arc::new(subscriber));

        let mut event = bridge_event();
        let outcome = bus.dispatch(&mut event, DispatchMode::Emit).await;

        assert_eq!((outcome.invoked, outcome.timed_out), (1, 0), "总线视角：订阅者正常返回了");
        let stats = bus.subscriber_stats();
        assert_eq!(
            (stats[0].completed, stats[0].timed_out, stats[0].label.as_str()),
            (1, 0, "demo@external"),
            "归因标签取 plugin_id，计数按条累加"
        );
        assert_eq!(counters.snapshot().delivered, 0, "桥视角：一条也没真送到插件");
        assert_eq!(counters.snapshot().dropped_invoke, 1);
    }

    // ─────────────────── P4：环检测 / 深度 / 声明 / 配对 ───────────────────

    /// 看门狗：`f` 必须在 `limit` 内返回，否则**判失败**。
    ///
    /// PLAN §7 对环检测用例的要求是「测试自带超时，超时即判失败」——
    /// 否则真出现死锁时用例会永远挂着，CI 表现为「卡住」而不是「失败」。
    fn with_watchdog<T: Send + 'static>(
        limit: Duration,
        what: &str,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(f());
        });
        match receiver.recv_timeout(limit) {
            Ok(value) => value,
            Err(_) => panic!("{what} 超过 {limit:?} 未返回 —— 判定为挂死（不得死锁）"),
        }
    }

    /// 造一条长度为 `len` 的调用链（元素是插件 ID，不含接收方）。
    fn chain_of(len: usize) -> Vec<String> {
        (0..len).map(|index| format!("plugin.{index}")).collect()
    }

    fn test_host_peer(
        plugin_id: &str,
        capabilities: &[&str],
        calls: &[&str],
        registry: Arc<CapabilityRegistry>,
    ) -> HostPeer {
        HostPeer {
            plugin_id: plugin_id.to_string(),
            declared_capabilities: capabilities.iter().map(|s| (*s).to_string()).collect(),
            declared_calls: calls.iter().map(|s| (*s).to_string()).collect(),
            registry,
            runtime: None,
            permissions: Vec::new(),
        }
    }

    /// §7②：`A→B→C→B` 这类环必须被拒，且**不得死锁**（看门狗超时即判失败）。
    ///
    /// 环是在**目标端**发现的：链 `[A,B,C]` 到达 B 时，B 自检发现链里已有自己。
    #[test]
    fn cycle_a_b_c_b_is_rejected_without_deadlock() {
        let rejection = with_watchdog(Duration::from_secs(2), "环检测", || {
            check_chain(
                "plugin.b",
                &["plugin.a".to_owned(), "plugin.b".to_owned(), "plugin.c".to_owned()],
            )
        })
        .expect_err("B 已在链中 ⇒ 必须拒绝");
        assert!(
            rejection.starts_with(error_codes::SEAM_CALL_CYCLE_DETECTED),
            "应带环检测错误码：{rejection}"
        );

        // 自调（A→A）同样是环。
        let self_call =
            check_chain("plugin.a", &["plugin.a".to_owned()]).expect_err("自调必须拒绝");
        assert!(self_call.starts_with(error_codes::SEAM_CALL_CYCLE_DETECTED));

        // 无环的正常链必须放行（否则上面的拒绝没有说服力）。
        check_chain(
            "plugin.d",
            &["plugin.a".to_owned(), "plugin.b".to_owned(), "plugin.c".to_owned()],
        )
        .expect("无环链应放行");
        check_chain("plugin.a", &[]).expect("顶层调用（空链）应放行");
    }

    /// §7③：**非环**但超过深度上限的长链必须被拒（环检测挡不住它）。
    #[test]
    fn long_acyclic_chain_is_rejected_at_depth_limit() {
        let rejection = with_watchdog(Duration::from_secs(2), "深度上限", || {
            check_chain("plugin.target", &chain_of(MAX_SEAM_CALL_DEPTH))
        })
        .expect_err("链长达到上限 ⇒ 必须拒绝");
        assert!(
            rejection.starts_with(error_codes::SEAM_CALL_DEPTH_EXCEEDED),
            "应带深度上限错误码：{rejection}"
        );

        // 上限之下必须放行（边界不能提前一格）。
        check_chain("plugin.target", &chain_of(MAX_SEAM_CALL_DEPTH - 1)).expect("未达上限应放行");
    }

    /// §7④：插件调用**自己未声明**的接缝 ⇒ 拒绝（声明是准入，不是装饰）。
    #[test]
    fn undeclared_seam_call_is_rejected() {
        let registry = Arc::new(CapabilityRegistry::new());
        let peer = test_host_peer("plugin.a", &[], &["workflow.business_rule"], registry);
        let response = with_watchdog(Duration::from_secs(2), "未声明校验", move || {
            peer.handle_call_seam(
                &json!({ "seam": "workflow.sandbox", "op": "execute", "args": {} }),
                &[],
            )
        });
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_NOT_DECLARED));
    }

    /// 自调自身接缝 ⇒ 宿主**提前**判定为环（本插件 worker 正阻塞着等响应，调回即死锁）。
    #[test]
    fn calling_own_capability_is_rejected_as_cycle() {
        let registry = Arc::new(CapabilityRegistry::new());
        let peer = test_host_peer(
            "plugin.a",
            &["workflow.business_rule"],
            &["workflow.business_rule"],
            registry,
        );
        let response = with_watchdog(Duration::from_secs(2), "自调校验", move || {
            peer.handle_call_seam(
                &json!({ "seam": "workflow.business_rule", "op": "evaluate", "args": {} }),
                &[],
            )
        });
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_CYCLE_DETECTED));
    }

    /// §7⑥：目标接缝的 provider 已不在注册表（提供方已卸载）⇒ **明确失败，不挂起**。
    #[test]
    fn missing_provider_fails_fast_instead_of_hanging() {
        // 空注册表 = 提供方已卸载。
        let registry = Arc::new(CapabilityRegistry::new());
        let peer = test_host_peer("plugin.a", &[], &["workflow.business_rule"], registry);
        let response = with_watchdog(Duration::from_secs(2), "provider 缺失", move || {
            peer.handle_call_seam(
                &json!({ "seam": "workflow.business_rule", "op": "evaluate", "args": {} }),
                &[],
            )
        });
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_PROVIDER_UNAVAILABLE));
    }

    /// op 与 §5.4 映射表不符 ⇒ `SEAM_CALL_UNSUPPORTED`（不猜、不兜底到别的方法）。
    #[test]
    fn mismatched_op_is_unsupported() {
        let registry = Arc::new(CapabilityRegistry::new());
        let peer = test_host_peer("plugin.a", &[], &["workflow.business_rule"], registry);
        let response = peer.handle_call_seam(
            &json!({ "seam": "workflow.business_rule", "op": "not_an_op", "args": {} }),
            &[],
        );
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_UNSUPPORTED));

        // 未知 op（既不是 emit 也不是 call_seam）同样明确报错。
        let response = peer.handle_inbound(&FrameRequest::new("nope", Value::Null));
        assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_UNKNOWN_OP));
    }

    /// drain（PLAN §13.3 优雅停机）三段语义：置位后**新调用被拒**、
    /// **在途调用照常回完**、pending 清空后 `drain` 返回 `true`。
    #[test]
    fn drain_refuses_new_calls_and_waits_inflight() {
        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环（读端）");
        // 收尾句柄：`shutdown` 必须在 `drop(invoker)` **之前** —— 同一 socket 的全部句柄
        // 共享它，先制造 EOF/读错误，两端阻塞中的读才会醒来退出；顺序反过来，
        // `Drop for WorkerInvoker` 的 `join_reader()` 会与假 worker 互等到永远
        // （2026-09-27 实测：全量测试因此卡住 5940s+ 被取消）。
        let host_teardown = host_side.try_clone().expect("克隆回环（收尾）");
        let worker_reader = worker_side.try_clone().expect("克隆回环（worker 读端）");

        // 假 worker：`slow` 延迟 300ms 再答 —— 给 drain 制造确定性的在途窗口。
        // 不 join：本线程阻塞在等下一帧上，直到收尾 `shutdown` 掉 socket 才读到 EOF 退出。
        let _worker = std::thread::spawn(move || {
            let mut peer = Peer::new(worker_reader, worker_side);
            let _ = peer.run(|request, _peer| match request.op.as_str() {
                "slow" => {
                    std::thread::sleep(Duration::from_millis(300));
                    FrameResponse::success(json!({ "done": true }))
                },
                other => FrameResponse::error("PLUGIN_UNKNOWN_OP", format!("未知 op：{other}")),
            });
        });

        let invoker = Arc::new(WorkerInvoker::from_streams(
            "drain-test@external",
            Box::new(host_reader),
            Box::new(host_side),
        ));

        // ① 在途 + ② drain 在看门狗内跑：超时即判失败，不得挂死 CI（本模块判据）。
        // `join` 刻意不用 `expect`：线程 panic 也转成普通返回值在外层断言，
        // 否则看门狗线程先 panic，真正的断言信息会被超时文案掩盖。
        let watch_invoker = invoker.clone();
        let (inflight_result, drained): (Result<Value, String>, bool) =
            with_watchdog(Duration::from_secs(15), "drain 等待在途调用", move || {
                // ① 在途调用挂到后台线程（它必须**不受 drain 影响**照常回完）。
                let inflight_invoker = watch_invoker.clone();
                let inflight =
                    std::thread::spawn(move || inflight_invoker.invoke("slow", json!({})));
                // 留足时间让 `slow` 帧写出去并进入 pending。
                std::thread::sleep(Duration::from_millis(80));

                // ② drain：置位 + 等在途收尾（5s 上限，到点返回 false，不永久等）。
                let drain_invoker = watch_invoker.clone();
                let drainer =
                    std::thread::spawn(move || drain_invoker.drain(Duration::from_secs(5)));

                let inflight_result: Result<Value, String> = match inflight.join() {
                    Ok(result) => result,
                    Err(_) => Err("在途调用线程 panic".to_string()),
                };
                let drained = drainer.join().unwrap_or_default();
                // 收尾 shutdown 在这里做：`invoker` 原柄此刻还活着，但 socket 已关；
                // 外层最后的 `drop(invoker)` 只会回收已退出的读线程，不会互等
                // （同并发配对用例的收尾顺序，见该用例注释）。
                let _ = host_teardown.shutdown(Shutdown::Both);
                (inflight_result, drained)
            });
        assert!(inflight_result.is_ok(), "在途调用必须照常完成：{inflight_result:?}");
        assert!(drained, "pending 清空后 drain 应返回 true");

        // ③ drain 线程已 join 到 `true`，置位无竞态：新调用必须被拒。
        let refused = invoker.invoke("slow", json!({}));
        let err = refused.expect_err("drain 之后新调用必须被拒");
        assert!(err.contains("drain"), "错误应指向 drain 闸门：{err}");

        // 显式析构：读线程已随 shutdown 退出，`join_reader` 直接返回。
        drop(invoker);
    }

    /// §7①：并发 N 个请求 + worker 内回调宿主 —— 响应必须按 `request_id` 正确配对。
    ///
    /// 假 worker 用**真实的插件侧实现**（`axagent_plugin_proto::Peer`）跑在回环 TCP 上，
    /// 于是这条用例同时验证了协议两侧：宿主读线程的配对表、以及 worker 的
    /// 「等响应期间把对端帧塞进 deferred」的重入语义。
    #[test]
    fn concurrent_requests_pair_by_request_id_with_worker_reentry() {
        const N: usize = 8;

        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        // 收尾句柄：调用全部完成后 `shutdown` 这个 socket —— 同一 socket 的全部句柄共享它，
        // 故两端阻塞中的读都会醒来退出。缺了它，`Drop for WorkerInvoker` 的 `join_reader()`
        // 会永远等假 worker（它不是子进程，`kill_child()` 无进程可杀，制造不出 EOF）。
        let host_teardown = host_side.try_clone().expect("克隆回环连接（收尾）");
        let worker_reader = worker_side.try_clone().expect("克隆回环连接（worker 读端）");

        // 假 worker：收到 `slow` 就先回调宿主一次 `call_seam`，再回自己的响应。
        // 不 join：本线程阻塞在等下一帧上，直到用例收尾 `shutdown` 掉 socket 才读到 EOF 退出。
        let _worker = std::thread::spawn(move || {
            let mut peer = Peer::new(worker_reader, worker_side);
            let _ = peer.run(|request, peer| match request.op.as_str() {
                "slow" => {
                    let echoed = match peer.call_seam(
                        "workflow.business_rule",
                        "evaluate",
                        // `node_type` 用 `NodeKind` 的 serde 形态（PascalCase，该枚举无 rename_all）。
                        json!({ "node_type": "Tool", "node_input": null }),
                        request.call_chain.clone(),
                    ) {
                        Ok(response) => response.value.unwrap_or(Value::Null),
                        Err(e) => return FrameResponse::error("PLUGIN_CALL_FAILED", e.to_string()),
                    };
                    FrameResponse::success(json!({ "n": request.args["n"], "seam": echoed }))
                },
                other => FrameResponse::error("PLUGIN_UNKNOWN_OP", format!("未知 op：{other}")),
            });
        });

        let invoker = WorkerInvoker::from_streams(
            "fake@external",
            Box::new(host_reader),
            Box::new(host_side),
        );
        // 假 worker 不提供任何接缝，但声明要调用 business_rule。
        invoker.set_declaration(
            &PluginDeclaration {
                proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
                subscribe: Vec::new(),
                calls: vec!["workflow.business_rule".to_owned()],
                capabilities: Vec::new(),
            },
            Arc::new(native_business_rule_registry()),
        );

        let results = with_watchdog(Duration::from_secs(10), "并发配对", move || {
            let results = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..N)
                    .map(|index| {
                        let invoker = &invoker;
                        scope.spawn(move || invoker.invoke("slow", json!({ "n": index })))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("调用线程不应 panic"))
                    .collect::<Vec<_>>()
            });
            // 全部调用结束后才关 socket。必须在本闭包内做：`invoker` 随本闭包析构，
            // 而 `FnOnce` 是**先消费闭包再 `send`** ⇒ 不先制造 EOF，`join_reader()` 会与
            // 假 worker 互等到看门狗超时（表现为「配对失败」，其实配对早就对了）。
            let _ = host_teardown.shutdown(Shutdown::Both);
            results
        });

        // 每个调用者都必须拿到**自己那个 n** —— 这就是「按 request_id 配对」的判据：
        // 一旦帧错配，某个调用者会拿到别人的 n（或解析失败）。
        for (index, result) in results.iter().enumerate() {
            let value = result.as_ref().expect("重入调用应当成功").clone();
            assert_eq!(value["n"], json!(index), "第 {index} 个调用拿到了别人的响应 ⇒ 帧错配");
            assert!(
                value["seam"].is_object(),
                "worker 回调宿主 take 到的接缝返回值应被带回：{value}"
            );
        }
    }

    /// 读线程退出后，等待中的调用必须**尽快拿到 `Err`**，而不是永久阻塞。
    #[test]
    fn pending_call_fails_fast_when_reader_exits() {
        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let invoker = WorkerInvoker::from_streams(
            "fake@external",
            Box::new(host_reader),
            Box::new(host_side),
        );

        // 假 worker 收下请求后**不回任何帧**就退出：它的两个句柄随之关闭 ⇒ 宿主读线程读到 EOF
        // ⇒ 清空配对表 ⇒ 等待中的调用拿到 `Err`。
        //
        // 这里不能图省事用 `Peer::run`：它会回帧，而且会继续阻塞在「读下一帧」上不退出 ——
        // 用例既测不到「读线程退出」，收尾的 `join_reader()` 也会永远等下去
        // （假 worker 不是子进程，`kill_child()` 无进程可杀，制造不出 EOF）。
        let worker = std::thread::spawn(move || {
            let mut reader = worker_side.try_clone().expect("克隆回环连接（worker 读端）");
            let _ = read_inbound(&mut reader);
        });

        let result = with_watchdog(Duration::from_secs(5), "无响应退出", move || {
            invoker.invoke("slow", Value::Null)
        });
        assert!(result.is_err(), "读线程退出后必须返回 Err，而不是永久挂起");
        let _ = worker.join();
    }

    /// 回环 TCP 连接对：`try_clone()` 出的两个句柄各自可读可写，正好当双向帧通道。
    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定回环端口");
        let address = listener.local_addr().expect("取本地地址");
        let client = TcpStream::connect(address).expect("连接回环端口");
        let (server, _) = listener.accept().expect("接受回环连接");
        (client, server)
    }

    /// 最小工作流执行记录（反思门面用例的输入；`reflect` 只做序列化转发）。
    fn sample_execution_record() -> WorkflowExecutionRecord {
        WorkflowExecutionRecord {
            workflow_id: "wf-1".to_string(),
            execution_id: "exec-1".to_string(),
            template_id: None,
            template_version: None,
            status: WorkflowRunStatus::Completed,
            started_at: 0,
            completed_at: Some(1),
            duration_ms: 1,
            nodes: Vec::new(),
            edges: Vec::new(),
            template_nodes: Vec::new(),
            input: None,
            output: None,
            error_context: None,
        }
    }

    /// 最小工作流基因组（进化门面用例的输入）。
    fn sample_genome() -> WorkflowGenome {
        WorkflowGenome {
            template_id: "tpl-1".to_string(),
            name: "demo".to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
            variables: Vec::new(),
            fitness: 0.0,
            generation: 0,
            changed_node_ids: Vec::new(),
        }
    }

    /// 只装了内置 `workflow.business_rule` 的局部注册表（供重入用例的宿主侧接缝落地）。
    fn native_business_rule_registry() -> CapabilityRegistry {
        let registry = CapabilityRegistry::new();
        registry
            .register_business_rule(Arc::new(NativeBusinessRule))
            .expect("内置业务规则应注册成功");
        registry
    }

    /// 不接触任何进程的调用面替身（供「未支持接缝」用例使用）。
    struct NoopInvoker;

    impl SeamInvoker for NoopInvoker {
        fn invoke(&self, _op: &str, _args: Value) -> Result<Value, String> {
            Ok(Value::Null)
        }
    }

    /// 按 op 返回脚本化 JSON 的调用面替身 —— 门面与事件桥的内存测试基础。
    struct ScriptedInvoker {
        responses: HashMap<String, Value>,
    }

    impl ScriptedInvoker {
        fn new(entries: &[(&str, Value)]) -> Self {
            Self {
                responses: entries.iter().map(|(op, v)| ((*op).to_string(), v.clone())).collect(),
            }
        }
    }

    impl SeamInvoker for ScriptedInvoker {
        fn invoke(&self, op: &str, _args: Value) -> Result<Value, String> {
            self.responses.get(op).cloned().ok_or_else(|| format!("未脚本化的 op：{op}"))
        }
    }

    /// 记录最近一次转发的 `(op, args)` 并固定回放的测试替身 —— 用于断言**线格式**。
    struct RecordingInvoker {
        response: Value,
        calls: parking_lot::Mutex<Vec<(String, Value)>>,
    }

    impl RecordingInvoker {
        fn new(response: Value) -> Self {
            Self { response, calls: parking_lot::Mutex::new(Vec::new()) }
        }

        fn last_call(&self) -> Option<(String, Value)> {
            self.calls.lock().last().cloned()
        }
    }

    impl SeamInvoker for RecordingInvoker {
        fn invoke(&self, op: &str, args: Value) -> Result<Value, String> {
            self.calls.lock().push((op.to_string(), args));
            Ok(self.response.clone())
        }
    }

    /// 构造一份最小可用的 provider 调用上下文（字段全填以便断言 camelCase 出线）。
    fn sample_provider_ctx() -> ProviderRequestContext {
        ProviderRequestContext {
            header_timeout_secs: None,
            api_key: "sk-demo".to_string(),
            key_id: "k1".to_string(),
            provider_id: "p1".to_string(),
            base_url: Some("https://example.invalid/v1".to_string()),
            api_path: None,
            proxy_config: None,
            custom_headers: None,
            api_mode: None,
            conversation: None,
            previous_response_id: None,
            store_response: None,
        }
    }

    /// 静态内置 webhook 派发实现（门面透明性用例的内置侧）。
    struct NativeWebhookDispatch;

    #[async_trait]
    impl WebhookDispatch for NativeWebhookDispatch {
        async fn dispatch(
            &self,
            _event: WebhookEvent,
            _data: HashMap<String, Value>,
        ) -> DispatchResult {
            DispatchResult { success_count: 1, failure_count: 0, errors: Vec::new() }
        }
    }

    /// 静态内置业务规则实现（重入用例的宿主侧接缝落地）。
    #[derive(Debug)]
    struct NativeBusinessRule;

    impl BusinessRuleEvaluator for NativeBusinessRule {
        fn evaluate(&self, _node_type: &NodeKind, node_input: &Value) -> RuleEvaluationOutcome {
            // 选 `Violation` 而非 `Pass`：`RuleEvaluationOutcome` 是外部标签序列化，
            // 单元变体 `Pass` 落地为 JSON 字符串，而 `Violation { .. }` 落地为对象 ——
            // 用例据此断言「宿主接缝的返回值确实被带回 worker 侧」。
            RuleEvaluationOutcome::Violation {
                rule_name: "native".to_owned(),
                rule_description: "本地内置规则（测试替身）".to_owned(),
                action: axagent_harness::RuleAction::Warn(format!("看到输入：{node_input}")),
                reason: "测试替身固定返回违规".to_owned(),
            }
        }
    }

    // ─────────────────── 任务 #49：命令帧派发（传输层单测） ───────────────────

    /// 命令帧用例的统一插件 ID。
    const CMD_PLUGIN_ID: &str = "cmd-demo@external";

    /// 用回环 socket 造一个「宿主侧 invoker + 假 worker 端」连接对。
    ///
    /// 假 worker 端**不会自发回帧**：回什么、何时回，完全由用例经 `worker_side` 操纵，
    /// 这样才能分别构造「回成功回执」「回错误帧」「装死不回」三种世界。
    fn command_transport() -> (WorkerInvoker, TcpStream) {
        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let invoker =
            WorkerInvoker::from_streams(CMD_PLUGIN_ID, Box::new(host_reader), Box::new(host_side));
        (invoker, worker_side)
    }

    /// 命令帧往返全链：宿主发出的帧必须带注册表绑定的 `plugin_id` / 命令名 / 预算，
    /// 对端回的成功回执必须被解回强类型 [`CommandReply`]。
    #[test]
    fn execute_command_sends_host_bound_frame_and_returns_reply() {
        let (invoker, mut worker_side) = command_transport();
        let (sender, receiver) = mpsc::channel();
        // 假 worker：收下命令帧 ⇒ 转写给主线程 ⇒ 按请求 ID 回成功回执。
        std::thread::spawn(move || {
            let inbound = read_inbound(&mut worker_side).expect("假 worker 应读到命令帧");
            let request = inbound.as_request().expect("入站必须是请求帧").clone();
            let frame = CommandFrame::from_frame_request(&request).expect("args 必须是命令帧");
            let reply = CommandReply::for_frame(&frame, json!({ "pong": true }));
            let response = reply
                .into_frame_response()
                .expect("回执编码应成功")
                .with_id(request.request_id.clone().unwrap_or_default());
            let _ = sender.send(frame);
            write_json_frame(&mut worker_side, &response).expect("假 worker 应能回帧");
        });

        let frame = CommandFrame::new(CMD_PLUGIN_ID, "ping", json!({ "q": 1 }), 5_000);
        let reply = invoker.execute_command(&frame).expect("命令帧往返应成功");
        assert_eq!(reply.plugin_id, CMD_PLUGIN_ID, "回执绑定必须原样带回");
        assert_eq!(reply.command, "ping");
        assert_eq!(reply.value, json!({ "pong": true }));

        let sent = receiver.recv().expect("假 worker 应观察到宿主发出的命令帧");
        assert_eq!(sent.plugin_id, CMD_PLUGIN_ID, "宿主必须以注册表权威键绑定 pluginId");
        assert_eq!(sent.command, "ping");
        assert_eq!(sent.input, json!({ "q": 1 }), "输入必须原样承载");
        assert_eq!(sent.timeout_ms, 5_000, "执行预算必须随帧下发");
    }

    /// pluginId 绑定不符 ⇒ **写帧之前**就拒绝（不靠 worker 兜底），错误信息带协议码。
    #[test]
    fn execute_command_rejects_plugin_id_mismatch_before_writing() {
        // 无进程、无网络：守卫在 exchange 之前，任何通道形态都只应报错不回挂。
        let invoker =
            WorkerInvoker::from_streams(CMD_PLUGIN_ID, Box::new(&b""[..]), Box::new(Vec::new()));
        let frame = CommandFrame::new("evil@external", "ping", Value::Null, 1_000);
        let err = invoker.execute_command(&frame).expect_err("绑定不符必须拒绝");
        let text = err.to_string();
        assert!(
            text.contains(error_codes::PLUGIN_COMMAND_ID_MISMATCH),
            "错误需点明协议码，实际：{text}"
        );
        assert!(text.contains("未发出帧"), "语义必须写明帧未发出，实际：{text}");
    }

    /// 对端装死不回 ⇒ 宿主在「预算 + 收帧宽限」处拿 [`WorkerError::Timeout`]，
    /// **不得**永久挂起（D1 修复的判据）。
    #[test]
    fn execute_command_times_out_when_worker_hangs() {
        let (invoker, worker_side) = command_transport();
        let started = Instant::now();
        let outcome = with_watchdog(Duration::from_secs(10), "命令帧超时收口", move || {
            let frame = CommandFrame::new(CMD_PLUGIN_ID, "hang", Value::Null, 100);
            let outcome = invoker.execute_command(&frame);
            // 先制造 EOF 再随闭包析构 invoker：否则 join_reader 会等一个仍活着的 socket。
            let _ = worker_side.shutdown(Shutdown::Both);
            outcome
        });
        let elapsed = started.elapsed();
        let err = match outcome {
            Ok(reply) => panic!("装死 worker 不应有回执，实际 {:?}", reply),
            Err(err) => err,
        };
        assert!(
            matches!(err, WorkerError::Timeout { timeout_ms, .. } if timeout_ms == 1_100),
            "期望 Timeout(预算 100ms + 宽限 1000ms)，实际 {err:?}"
        );
        assert!(elapsed < Duration::from_secs(5), "超时收口不得迟于预算数量级，实际 {elapsed:?}");
    }

    /// worker 回错误帧（自带协议码）⇒ 宿主报错必须携带该码（映射 ErrorResponse 的输入）。
    #[test]
    fn execute_command_surfaces_worker_fault_code() {
        let (invoker, mut worker_side) = command_transport();
        std::thread::spawn(move || {
            let inbound = read_inbound(&mut worker_side).expect("假 worker 应读到命令帧");
            let request = inbound.as_request().expect("入站必须是请求帧").clone();
            let fault = CommandFrame::from_frame_request(&request)
                .map(|frame| {
                    CommandFault::for_frame(
                        &frame,
                        error_codes::PLUGIN_COMMAND_UNKNOWN,
                        "worker 没有该命令实现",
                    )
                })
                .expect("解码应成功")
                .into_frame_response()
                .with_id(request.request_id.clone().unwrap_or_default());
            write_json_frame(&mut worker_side, &fault).expect("假 worker 应能回错误帧");
        });

        let frame = CommandFrame::new(CMD_PLUGIN_ID, "nope", Value::Null, 5_000);
        let err = invoker.execute_command(&frame).expect_err("错误帧必须报 Err");
        let text = err.to_string();
        assert!(
            text.contains(error_codes::PLUGIN_COMMAND_UNKNOWN),
            "协议码必须透传到宿主侧错误，实际：{text}"
        );
    }

    /// drain 停机窗口内，新命令帧与接缝调用同口径被拒。
    #[test]
    fn execute_command_refused_while_draining() {
        let (invoker, worker_side) = command_transport();
        assert!(invoker.drain(Duration::ZERO), "无在途调用时 drain 应立即放行");
        let frame = CommandFrame::new(CMD_PLUGIN_ID, "ping", Value::Null, 1_000);
        let err = invoker.execute_command(&frame).expect_err("drain 后应拒绝新命令");
        assert!(err.to_string().contains("drain"), "错误应点明停机窗口，实际：{err}");
        // 让读线程尽快收尾（invoker 随本测试析构时 join_reader 依赖 EOF）。
        let _ = worker_side.shutdown(Shutdown::Both);
    }

    // ─────────────────── 任务 #53：D1（接缝调用预算 + 超时标脏） ───────────────────

    /// 接缝超时用例的统一插件 ID。
    const SEAM_HANG_PLUGIN_ID: &str = "seam-hang@external";

    /// 回环 socket 上的「装死假 worker」：读走**第一帧**后睡 `hang`，**永不回帧**。
    ///
    /// 它不看 op ⇒ 同一夹具两用：发接缝帧（op=`hang`）演 D1 的接缝装死（任务 #53），
    /// 直接调 [`WorkerInvoker::handshake`]（op=`describe`）演握手装死（任务 #63）。
    ///
    /// 返回宿主侧 invoker（两类预算分别注入 `wait` / `handshake_wait`）与 worker 侧
    /// socket 句柄：用例收尾时对后者 `shutdown(Both)` 制造 EOF，读线程才收得掉
    /// （假 worker 不是子进程，`kill_child()` 无进程可杀 —— 与 [`command_transport`]
    /// 同一套收尾约定）。
    fn seam_hang_transport(
        wait: Duration,
        handshake_wait: Duration,
        hang: Duration,
    ) -> (WorkerInvoker, TcpStream) {
        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let mut worker_reader = worker_side.try_clone().expect("克隆回环连接（worker 读端）");
        std::thread::spawn(move || {
            let _ = read_inbound(&mut worker_reader).expect("假 worker 应读到请求帧");
            std::thread::sleep(hang);
            // 句柄随线程结束 drop；测试主线程另持 worker_side 负责收尾。
        });
        let invoker = WorkerInvoker::from_streams_with_timeouts(
            SEAM_HANG_PLUGIN_ID,
            Box::new(host_reader),
            Box::new(host_side),
            wait,
            handshake_wait,
        );
        (invoker, worker_side)
    }

    /// D1 修复本体：worker 装死 ⇒ 接缝调用在预算内拿超时错误（而非永久挂起），
    /// 并且**连接被标脏**：第二次接缝调用立即被拒（不再排队等一个新预算），
    /// 标脏后的 `shutdown()` 也必须在看门狗内收口（卸载不被拖挂）。
    #[test]
    fn seam_call_times_out_within_budget_and_poisons_the_connection() {
        let wait = Duration::from_millis(150);
        let (invoker, worker_side) =
            seam_hang_transport(wait, Duration::from_millis(150), Duration::from_secs(2));

        // ① 首次调用：预算内报超时（文本来自 WorkerError::Timeout 的 Display）。
        let started = Instant::now();
        let err = invoker
            .invoke_with_chain("hang", json!({}), Vec::new())
            .expect_err("worker 装死必须被接缝预算拦下（D1 判据）");
        let elapsed = started.elapsed();
        assert!(err.contains("超过宿主等待上限"), "错误应点明超时，实际：{err}");
        assert!(err.contains("150 ms"), "预算应随错误带回，实际：{err}");
        assert!(
            elapsed >= wait && elapsed < Duration::from_secs(2),
            "收口应落在预算附近，实际 {elapsed:?}"
        );

        // ② 二次调用：被「可疑」闸拒，耗时必须显著小于一个预算（证明是拒、不是再等）。
        let started = Instant::now();
        let err2 = invoker
            .invoke_with_chain("hang", json!({}), Vec::new())
            .expect_err("标脏后新接缝调用必须被拒");
        let refused_in = started.elapsed();
        assert!(err2.contains("可疑"), "错误应点明连接可疑，实际：{err2}");
        assert!(refused_in < wait, "闸拒不得再等一个预算，实际 {refused_in:?}");

        // ③ 停机：装死的连接上「等 shutdown 回执」就是 D1 的卸载侧变体 ——
        //    标脏路径必须跳过礼貌帧并在看门狗内收口（真子进程形态另有 e2e 用例）。
        let _ = worker_side.shutdown(Shutdown::Both);
        with_watchdog(Duration::from_secs(5), "标脏后的 shutdown", move || {
            invoker.shutdown();
        });
    }

    /// 处置边界（任务 #73 S-3 改口径，原用例名
    /// `seam_poison_does_not_change_command_channel_behavior`）：接缝脏闸**也拦命令通道**。
    ///
    /// 修复前命令通道完全不读脏闸 ⇒ 已被实证装死的连接上，每条命名命令仍各自等满
    /// `timeout_ms + 1 s`（默认 31 s）且可无限重复，#53 的止血少覆盖一条通道。
    /// 现在必须**立即**被拒（耗时显著小于一个预算），且：
    /// - 错误带既有协议码 [`error_codes::SEAM_PROVIDER_UNAVAILABLE`] + 处置指引（M-2）；
    /// - 命令帧自己超时**不**置脏（#49 语义保留）—— 这条由
    ///   [`WorkerError::Timeout`] 只在 `invoke_with_chain` 的映射里置闸的结构保证。
    #[test]
    fn seam_poison_refuses_command_frames_immediately() {
        let (invoker, worker_side) = seam_hang_transport(
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_secs(3),
        );
        assert!(
            invoker.invoke_with_chain("hang", json!({}), Vec::new()).is_err(),
            "先经接缝超时置脏"
        );

        let started = Instant::now();
        let frame = CommandFrame::new(SEAM_HANG_PLUGIN_ID, "ping", Value::Null, 100);
        let err = invoker.execute_command(&frame).expect_err("脏连接上的命令帧必须被拒");
        let refused_in = started.elapsed();
        assert!(
            !matches!(err, WorkerError::Timeout { .. }),
            "不得再走「等满预算」的老路，实际 {err:?}"
        );
        assert!(
            refused_in < Duration::from_millis(1_100),
            "脏闸拒命令不得等一个帧预算 + 宽限（100ms+1s），实际 {refused_in:?}"
        );
        let text = err.to_string();
        assert!(
            text.contains(error_codes::SEAM_PROVIDER_UNAVAILABLE),
            "命令侧的拒绝也要带可映射的错误码，实际：{text}"
        );
        assert!(text.contains("禁用再启用"), "拒绝文案必须给处置指引，实际：{text}");
        // 结构化码位（S-1）：判据读的字段必须真的有值，不是只写在文本里。
        assert_eq!(
            err.error_code(),
            Some(error_codes::SEAM_PROVIDER_UNAVAILABLE),
            "脏闸拒绝必须带结构化码位 ⇒ 回退判据据此判定「不回退」"
        );
        let _ = worker_side.shutdown(Shutdown::Both);
    }

    /// M-1（worker 进程已死 ⇒ 命令帧判「未送达」）之**管道已断**这一支，无门控可跑。
    ///
    /// 复现的正是审查稿 A1.5 的形态：表项还在、对端已没了。判定来自读线程登记的死讯
    /// （`connection_closed`），因此它**不需要**子进程也能观测 —— 这也是
    /// [`WorkerInvoker::worker_gone_reason`] 判据②存在的理由（真子进程那一支见
    /// 门控 e2e `e2e_dead_worker_command_falls_back_to_spawn_once`）。
    #[test]
    fn execute_command_reports_not_delivered_when_the_pipe_is_gone() {
        let (invoker, worker_side) = command_transport();
        // 关掉对端 ⇒ 读线程读到 EOF ⇒ 置 `connection_closed` 并清空配对表。
        let _ = worker_side.shutdown(Shutdown::Both);
        // 自带截止的轮询（不用看门狗：不移动 `invoker`，也不会让用例永挂）。
        let deadline = Instant::now() + Duration::from_secs(5);
        while invoker.worker_gone_reason().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let gone_reason =
            invoker.worker_gone_reason().expect("读线程应在 5s 内登记死讯（EOF 即刻可得）");
        assert!(gone_reason.contains("管道已断开"), "死因应指向读线程已退出，实际：{gone_reason}");

        let started = Instant::now();
        let frame = CommandFrame::new(CMD_PLUGIN_ID, "ping", Value::Null, 1_000);
        let err = invoker.execute_command(&frame).expect_err("对端已死必须报错而不是静默成功");
        assert!(
            matches!(err, WorkerError::NotDelivered { .. }),
            "期望 NotDelivered（帧未送达 ⇒ 零副作用 ⇒ 上层可回退），实际 {err:?}"
        );
        // 关键区别：不是「等满预算」，预检在写帧**之前**就判死。
        assert!(started.elapsed() < Duration::from_millis(1_000), "预检不得等一个帧预算");
        let _ = worker_side.shutdown(Shutdown::Both);
    }

    /// M-1 的另一支：进程看着还活着（读线程未退出），但**写帧本身失败** ⇒ 同样报
    /// [`WorkerError::NotDelivered`]。这一支是回退判据「帧从未送达」的字面依据。
    #[test]
    fn execute_command_reports_not_delivered_when_the_frame_write_fails() {
        /// 恒失败的写侧替身：等价于对进程已消失的管道写帧（EPIPE / ERROR_BAD_DEVICE）。
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "模拟写帧失败"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "模拟写帧失败"))
            }
        }

        // 读侧用一条**保持活跃**的回环（对端不关 ⇒ `connection_closed` 不会被置位），
        // 写侧换成恒失败替身 ⇒ 预检放行、写帧失败，落进本支。
        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let invoker = WorkerInvoker::from_streams(
            CMD_PLUGIN_ID,
            Box::new(host_reader),
            Box::new(FailingWriter),
        );

        let frame = CommandFrame::new(CMD_PLUGIN_ID, "ping", Value::Null, 1_000);
        let err = invoker.execute_command(&frame).expect_err("写帧失败必须报错");
        assert!(
            matches!(err, WorkerError::NotDelivered { ref op, .. } if op == ops::COMMAND),
            "帧没出去 ⇒ 必须是 NotDelivered（回退判据的输入），实际 {err:?}"
        );
        assert!(err.to_string().contains("写帧失败"), "死因要能定位，实际：{err}");
        // 配对表不得残留（帧没出去就没有等待者）。
        assert!(invoker.inner.pending.lock().is_empty(), "写失败后必须自己清掉槽位");
        let _ = worker_side.shutdown(Shutdown::Both);
    }

    /// M-2 复位口：闸**只有显式复位才清**（`LoadedPlugin::unload` / disable 的落地点），
    /// 连续调用不会自己清；复位之后同一连接重新接受接缝调用，且拒绝文案带处置指引。
    #[test]
    fn seam_suspect_reset_is_explicit_only() {
        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let host_teardown = host_side.try_clone().expect("克隆回环连接（收尾）");
        let worker_reader = worker_side.try_clone().expect("克隆回环连接（worker 读端）");

        // 一个**会正常应答**的假 worker：闸只由测试手工置位，测的是闸本身而非对端行为。
        let _worker = std::thread::spawn(move || {
            let mut peer = Peer::new(worker_reader, worker_side);
            let _ = peer.run(|request, _peer| match request.op.as_str() {
                "ping" => FrameResponse::success(json!({ "pong": true })),
                other => FrameResponse::error("PLUGIN_UNKNOWN_OP", format!("未知 op：{other}")),
            });
        });

        let invoker = WorkerInvoker::from_streams_with_timeouts(
            "reset@external",
            Box::new(host_reader),
            Box::new(host_side),
            Duration::from_millis(100),
            Duration::from_millis(100),
        );
        invoker.set_declaration(
            &PluginDeclaration {
                proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
                subscribe: Vec::new(),
                calls: Vec::new(),
                capabilities: Vec::new(),
            },
            Arc::new(CapabilityRegistry::new()),
        );

        let outcome = with_watchdog(Duration::from_secs(5), "脏闸复位", move || {
            // 白盒置脏：等价于「曾有一次接缝超时」（写帧后到点没回帧）。
            invoker.inner.seam_suspect.store(true, Ordering::Release);
            let first = invoker.invoke_with_chain("ping", json!({}), Vec::new());
            // 第二次仍被拒 ⇒ 证明**没有**「下次调用自动清」这种复位路径（派单禁止项）。
            let second = invoker.invoke_with_chain("ping", json!({}), Vec::new());
            invoker.reset_seam_suspect();
            let after_reset = invoker.invoke_with_chain("ping", json!({}), Vec::new());
            let _ = host_teardown.shutdown(Shutdown::Both);
            (first, second, after_reset)
        });
        let (first, second, after_reset) = outcome;

        let err = first.expect_err("脏闸必须拒绝新接缝调用");
        assert!(err.contains("可疑"), "错误应点明连接可疑，实际：{err}");
        assert!(
            err.contains(error_codes::SEAM_PROVIDER_UNAVAILABLE),
            "拒绝要带可映射的错误码（M-2 复用既有码，不新增 ⇒ 免 11 语言成本），实际：{err}"
        );
        assert!(err.contains("禁用再启用"), "拒绝要给用户处置指引，实际：{err}");
        let second_err = second.expect_err("第二次不得自动复位");
        assert!(second_err.contains("可疑"), "闸必须是显式复位型，实际：{second_err}");
        assert_eq!(
            after_reset.expect("显式复位后同一连接应重新接受接缝调用"),
            json!({ "pong": true }),
            "复位口必须真的放行后续调用"
        );
    }

    /// 「默认值能从常量读到、不是散落的魔数」：配置缺省逐字来自协议常量，
    /// 且接缝预算严格小于命令帧预算（对话回合口径）；逐接缝预算表的选取规则
    /// （表值优先、未进表用连接值）也在纯函数层面钉死。
    #[test]
    fn seam_wait_default_is_the_protocol_constant_not_a_local_number() {
        // 编译期钉「接缝默认 < 命令预算」（assertions_on_constants 不许运行期比较两常量）。
        const _: () = assert!(
            axagent_plugin_proto::DEFAULT_SEAM_TIMEOUT_MS
                < axagent_plugin_proto::DEFAULT_COMMAND_TIMEOUT_MS
        );
        let config = PluginWorkerConfig::new("default-budget@external", "unused-program");
        assert_eq!(
            config.seam_wait_timeout,
            Duration::from_millis(axagent_plugin_proto::DEFAULT_SEAM_TIMEOUT_MS),
            "缺省预算必须直接来自 plugin-proto 常量"
        );
        assert_eq!(axagent_plugin_proto::DEFAULT_SEAM_TIMEOUT_MS, 15_000);
        // 握手预算（任务 #63）同为协议常量、且与接缝预算**分开**：两条通道口径不同。
        assert_eq!(
            config.handshake_timeout,
            Duration::from_millis(axagent_plugin_proto::DEFAULT_HANDSHAKE_TIMEOUT_MS),
            "缺省握手预算必须直接来自 plugin-proto 常量"
        );
        assert_eq!(axagent_plugin_proto::DEFAULT_HANDSHAKE_TIMEOUT_MS, 10_000);
        let tuned_config = PluginWorkerConfig::new("tuned-budget@external", "unused-program")
            .with_seam_wait_timeout(Duration::from_millis(300))
            .with_handshake_timeout(Duration::from_millis(700));
        assert_eq!(
            (tuned_config.seam_wait_timeout, tuned_config.handshake_timeout),
            (Duration::from_millis(300), Duration::from_millis(700)),
            "两个预算旋钮必须互不串写（接缝覆盖不得改到握手，反之亦然）"
        );

        // 逐接缝预算表（任务 #53 裁定）：长跑型 op 用表值，其余用连接缺省值。
        let invoker = WorkerInvoker::from_streams_with_timeouts(
            "budget-table@external",
            Box::new(&b""[..]),
            Box::new(Vec::new()),
            Duration::from_millis(axagent_plugin_proto::DEFAULT_SEAM_TIMEOUT_MS),
            Duration::from_millis(axagent_plugin_proto::DEFAULT_HANDSHAKE_TIMEOUT_MS),
        );
        assert_eq!(
            invoker.seam_wait_for_op("run_turn"),
            Duration::from_millis(axagent_plugin_proto::AGENT_LOOP_RUN_TURN_TIMEOUT_MS),
            "分钟级 LLM 回合必须走显式大额预算，不得被默认 15s 误杀"
        );
        assert_eq!(
            invoker.seam_wait_for_op("evaluate"),
            Duration::from_millis(axagent_plugin_proto::DEFAULT_SEAM_TIMEOUT_MS),
            "未进表 op 必须回落连接预算"
        );
        // 连接级覆盖只改「未进表」的回落值；UI action 回流与 seam 同入口，同样受预算。
        let tuned = WorkerInvoker::from_streams_with_timeouts(
            "budget-tuned@external",
            Box::new(&b""[..]),
            Box::new(Vec::new()),
            Duration::from_millis(300),
            Duration::from_millis(axagent_plugin_proto::DEFAULT_HANDSHAKE_TIMEOUT_MS),
        );
        assert_eq!(tuned.seam_wait_for_op("evaluate"), Duration::from_millis(300));
        assert_eq!(tuned.seam_wait_for_op(crate::worker::OP_UI_ACTION), Duration::from_millis(300));
        assert_eq!(
            tuned.seam_wait_for_op("run_turn"),
            Duration::from_millis(axagent_plugin_proto::AGENT_LOOP_RUN_TURN_TIMEOUT_MS),
            "进表 op 不受连接级回落值影响（表优先规则）"
        );
        // 握手**不**查这张表：`describe` 未进表，若哪天有人拿表凑数（假修复），
        // 本行会随 seam_timeout_ms_for_op 的语义变化一起红。
        assert_eq!(
            axagent_plugin_proto::seam_timeout_ms_for_op(axagent_plugin_proto::ops::DESCRIBE),
            None,
            "握手预算独立于逐接缝覆盖表"
        );
    }

    /// 任务 #63（D1 的握手边角）：worker 起来了但**不回 `describe`** ⇒ 握手必须在
    /// 预算内失败（宁可误杀不可永挂），错误可定位到「握手 + op + 预算」，且这条连接
    /// 被标脏 —— 标脏在这里的价值不是「拒后续接缝调用」（连接根本不交付出去），
    /// 而是让 [`LoadedPlugin::load`] 失败路径上的 `shutdown()` 跳过等待 shutdown 回执，
    /// 否则病灶只是从「挂握手」挪成「挂停机」。
    #[test]
    fn handshake_times_out_within_budget_and_poisons_the_connection() {
        let wait = Duration::from_millis(150);
        // 接缝预算给 5s：用于反证「闸拒」不是「又等了一个接缝预算」。
        let (invoker, worker_side) =
            seam_hang_transport(Duration::from_secs(5), wait, Duration::from_secs(2));

        // ① 握手在预算内失败（真挂死时本行永不返回，故另有用例③的看门狗兜住停机）。
        let started = Instant::now();
        let err =
            invoker.handshake().expect_err("worker 不回 describe 必须被握手预算拦下（#63 判据）");
        let elapsed = started.elapsed();
        let text = err.to_string();
        assert!(text.contains("握手失败"), "错误应点明失败发生在握手阶段，实际：{text}");
        assert!(text.contains("describe"), "错误应点名未回帧的 op，实际：{text}");
        assert!(
            text.contains("超过宿主等待上限 150 ms"),
            "错误应带握手预算毫秒数（不得退化成笼统 transport error），实际：{text}"
        );
        // 任务 #73 M-2 下半：握手是**第二条置脏路径**，它的用户可见出口（本条）也必须带
        // 与闸文案同一句处置指引 —— 否则「已经告诉用户怎么办」只在接缝那半边成立。
        assert!(
            text.contains("禁用再启用"),
            "握手失败同样要给出处置指引（该连接随实例丢弃，用户看不到闸文案），实际：{text}"
        );
        assert!(
            elapsed >= wait && elapsed < Duration::from_secs(2),
            "收口应落在预算附近（夹具装死 2 s），实际 {elapsed:?}"
        );

        // ② 连接已标脏：后续接缝调用被**立即**拒绝，不再等一个 5s 接缝预算。
        let started = Instant::now();
        let err2 =
            invoker.invoke_with_chain("hang", json!({}), Vec::new()).expect_err("握手超时应置脏");
        let refused_in = started.elapsed();
        assert!(err2.contains("可疑"), "错误应点明连接可疑，实际：{err2}");
        assert!(refused_in < wait, "闸拒不得再等一个预算，实际 {refused_in:?}");

        // ③ 停机：装死的连接上「等 shutdown 回执」就是 D1 的停机侧变体 ——
        //    标脏路径必须跳过礼貌帧并在看门狗内收口（真子进程形态另有 e2e 用例）。
        let _ = worker_side.shutdown(Shutdown::Both);
        with_watchdog(Duration::from_secs(5), "握手超时后的 shutdown", move || {
            invoker.shutdown();
        });
    }

    // ─────────────── 任务 #73：M-3（嵌套 call_seam 的确定性误杀 + 永久标脏） ───────────────

    /// 宿主侧「慢」的 `webhook.dispatch` 实现 —— 嵌套用例里内层那一跳的耗时来源。
    ///
    /// 刻意选 `webhook.dispatch`：① 它是 **async** 接缝，宿主服务入站 `call_seam` 时
    /// 走 [`HostPeer::block_on`]（正是 M-3 占住读线程的那一步）；② 该 trait 只有一个
    /// 必需方法，夹具面最小（与既有的 `NativeWebhookDispatch` 同形）。
    struct SlowWebhookDispatch {
        delay: Duration,
    }

    #[async_trait]
    impl WebhookDispatch for SlowWebhookDispatch {
        async fn dispatch(
            &self,
            _event: WebhookEvent,
            _data: HashMap<String, Value>,
        ) -> DispatchResult {
            tokio::time::sleep(self.delay).await;
            DispatchResult { success_count: 1, failure_count: 0, errors: Vec::new() }
        }
    }

    /// 只装了慢 `webhook.dispatch` 的局部注册表（嵌套用例的宿主侧落地）。
    fn slow_webhook_registry(delay: Duration) -> CapabilityRegistry {
        let registry = CapabilityRegistry::new();
        registry
            .register_webhook_dispatch(Arc::new(SlowWebhookDispatch { delay }))
            .expect("慢 webhook 实现应注册成功");
        registry
    }

    /// M-3 主判据（审查稿 A4.3 的六步链）：插件 handler 里嵌套一次 `peer.call_seam(...)`，
    /// 而**宿主替它跑的那一跳耗时 > 外层出向预算** ⇒ 修复前必然：
    /// 外层先超时 → 撤配对槽位 + 置终局闸 → 内层答案回来后插件的迟到回帧被丢弃 →
    /// **此后该插件全部新接缝调用永久被拒**（§26.1 表 M-3、§26.5 `N-3` 的补证）。
    ///
    /// 夹具形态逐字对齐 plugin-proto 模块文档示范的 `peer.call_seam` 用法，走回环
    /// socket + 真实 `Peer`（插件侧 SDK），**不起子进程** ⇒ 不受
    /// `AXAGENT_TEST_PLUGIN_SUBPROCESS` 门控，是无门控恒跑的端到端形态锁。
    #[test]
    fn nested_call_seam_beyond_outer_budget_does_not_poison_the_connection() {
        // 外层预算 150ms、内层宿主侧真实耗时 600ms —— 内层 > 外层，正是 M-3 的触发条件。
        const OUTER_BUDGET: Duration = Duration::from_millis(150);
        const INNER_WORK: Duration = Duration::from_millis(600);

        // 先进入 runtime 再 `set_declaration`：句柄由 `Handle::try_current()` 捕获，
        // 捕获不到时 async 接缝只能返回 `no_runtime`，这条嵌套链就碰不到 `block_on`。
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .expect("嵌套用例需要多线程 runtime");
        let _runtime_guard = runtime.enter();

        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let host_teardown = host_side.try_clone().expect("克隆回环连接（收尾）");
        let worker_reader = worker_side.try_clone().expect("克隆回环连接（worker 读端）");

        // 假 worker：收到 `nest` 先向宿主发一条 `call_seam`（内层），**拿到宿主结果之后**
        // 才回 `nest` 那一帧 —— 单线程 `Peer` 的语义决定了「内层耗时必然计入外层等待」。
        let _worker = std::thread::spawn(move || {
            let mut peer = Peer::new(worker_reader, worker_side);
            let _ = peer.run(|request, peer| match request.op.as_str() {
                "nest" => match peer.call_seam(
                    "webhook.dispatch",
                    "dispatch",
                    json!({ "event": "ToolComplete", "data": {} }),
                    request.call_chain.clone(),
                ) {
                    Ok(response) => FrameResponse::success(json!({ "nested": response.value })),
                    Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
                },
                // 不嵌套的快调用：用来验「外层误杀之后，新接缝调用是否被终局闸永久拒绝」。
                "ping" => FrameResponse::success(json!({ "pong": true })),
                other => FrameResponse::error("PLUGIN_UNKNOWN_OP", format!("未知 op：{other}")),
            });
        });

        let invoker = Arc::new(WorkerInvoker::from_streams_with_timeouts(
            "nested@external",
            Box::new(host_reader),
            Box::new(host_side),
            OUTER_BUDGET,
            Duration::from_millis(100),
        ));
        // 声明：不提供任何接缝（避免自调判环），只声明要调用 `webhook.dispatch`。
        invoker.set_declaration(
            &PluginDeclaration {
                proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
                subscribe: Vec::new(),
                calls: vec!["webhook.dispatch".to_owned()],
                capabilities: Vec::new(),
            },
            Arc::new(slow_webhook_registry(INNER_WORK)),
        );

        let (outer, follow_up, invoker) =
            with_watchdog(Duration::from_secs(10), "嵌套链收口", move || {
                let outer = invoker.invoke_with_chain("nest", json!({}), Vec::new());
                let follow_up = invoker.invoke_with_chain("ping", json!({}), Vec::new());
                // 先制造 EOF 再随闭包析构 invoker（同既有配对用例的收尾约定，否则 join_reader 互等）。
                let _ = host_teardown.shutdown(Shutdown::Both);
                (outer, follow_up, invoker)
            });
        drop(invoker);

        // ① 终局闸不得因「一次嵌套」落下：健康连接上的后续调用必须照常工作。
        assert!(
            follow_up.is_ok(),
            "嵌套超时把整条连接永久标脏，后续新接缝调用被拒（M-3 的「永久被拒」本体）；\
             后续调用 = {follow_up:?}，外层嵌套调用 = {outer:?}"
        );
        // ② 外层嵌套调用本身不得被误杀（§18 订正后的「只要嵌套一次就必然误杀」）。
        let value = outer.expect("内层宿主侧调用期间，外层出向等待不得按预算误杀（M-3）");
        assert!(
            value["nested"].is_object(),
            "内层宿主侧接缝的返回值必须被带回，否则嵌套链等于白跑：{value}"
        );
    }

    // ─────────────────── 任务 #56：写侧解阻（写线程 + 三态写回执） ───────────────────

    /// 写到一半就阻塞的管道替身：首个 `write` 只接受一半字节，其后阻塞到 gate 放行。
    ///
    /// 造的正是「`write_all` 中途阻塞」的形态（长度前缀只写出一半）—— 真实世界里
    /// 「对端僵死不读 stdin 且管道满」时写者的状态。`writes` 供用例断言「毒化之后的
    /// 调用根本没有进入写线程」。
    struct HalfThenBlockWriter {
        gate: Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
        writes: Arc<AtomicUsize>,
    }

    impl Write for HalfThenBlockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let call = self.writes.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                // 只接受一半：剩余字节留在 `write_all` 的下一轮，那一轮会永久阻塞。
                return Ok(buf.len() / 2);
            }
            let (released, condvar) = &*self.gate;
            let mut released = released.lock();
            while !*released {
                condvar.wait(&mut released);
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// 放行 [`HalfThenBlockWriter`]（用例收尾必须调用，否则测试自己会留下一个阻塞线程）。
    fn release_gate(gate: &Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>) {
        let (released, condvar) = &**gate;
        *released.lock() = true;
        condvar.notify_all();
    }

    /// 写回执超时（任务 #56 主判据）：写线程卡在 `write_all` 中途时，调用方必须在**自己
    /// 的预算**内拿到 `Transport`，**不得**报 `NotDelivered`（那时可能已写出部分字节，
    /// 「未送达」不可证明）；连接同时被毒化，后续调用毫秒级拒绝、不进入写。
    #[test]
    fn write_ack_timeout_is_bounded_and_never_reports_not_delivered() {
        const BUDGET: Duration = Duration::from_millis(120);
        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let gate = Arc::new((parking_lot::Mutex::new(false), parking_lot::Condvar::new()));
        let writes = Arc::new(AtomicUsize::new(0));
        let writer = HalfThenBlockWriter { gate: Arc::clone(&gate), writes: Arc::clone(&writes) };
        let invoker = WorkerInvoker::from_streams_with_timeouts(
            "write-hang@external",
            Box::new(host_reader),
            Box::new(writer),
            BUDGET,
            Duration::from_millis(100),
        );

        // ① 写线程卡死 ⇒ 调用方在预算内收口（外层 recv_timeout 看门狗 + Instant 上下界）。
        let (err, elapsed, invoker) =
            with_watchdog(Duration::from_secs(5), "写回执超时收口", move || {
                let started = Instant::now();
                let err = invoker
                    .exchange_with_timeout(ops::COMMAND, json!({ "q": 1 }), Vec::new(), BUDGET)
                    .expect_err("写线程阻塞时必须返回错误，而不是永久等待");
                (err, started.elapsed(), invoker)
            });
        assert!(
            !matches!(err, WorkerError::NotDelivered { .. }),
            "写可能已写出部分字节 ⇒ 不得报 NotDelivered（那会允许上层回退、造成二次执行），\
             实际 {err:?}"
        );
        assert!(
            matches!(err, WorkerError::Transport { .. }),
            "写回执超时必须是 Transport（连接判不可用），实际 {err:?}"
        );
        assert!(err.to_string().contains("写帧回执"), "错误应点明失败在写侧回执，实际：{err}");
        assert!(
            elapsed >= BUDGET && elapsed < Duration::from_secs(2),
            "收口应落在写回执预算附近，实际 {elapsed:?}"
        );
        assert!(
            invoker.inner.seam_suspect.load(Ordering::Acquire),
            "写回执超时必须毒化连接（否则后续每笔调用都会再等一个预算）"
        );
        // 写者被排到后，确认夹具确实卡在 write_all 中途（首个 write 只接受一半）。
        let deadline = Instant::now() + Duration::from_secs(2);
        while writes.load(Ordering::Relaxed) < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            writes.load(Ordering::Relaxed) >= 2,
            "夹具应确实卡在 write_all 中途（首个 write 只接受一半）"
        );

        // ② 毒化后第二次调用：毫秒级拒绝，且**不进入写**（写计数不增）。
        let writes_before = writes.load(Ordering::Relaxed);
        let started = Instant::now();
        let err2 = invoker
            .exchange_with_timeout(ops::COMMAND, json!({}), Vec::new(), BUDGET)
            .expect_err("毒化后新调用必须快速失败");
        let refused_in = started.elapsed();
        assert!(
            matches!(err2, WorkerError::Transport { .. }),
            "毒化后的口径是「拒绝」（Transport），不是「回退」，实际 {err2:?}"
        );
        assert!(
            !matches!(err2, WorkerError::NotDelivered { .. }),
            "毒化拒绝不得退化成 NotDelivered，实际 {err2:?}"
        );
        assert!(refused_in < Duration::from_millis(50), "毒化拒绝应毫秒级，实际 {refused_in:?}");
        assert_eq!(
            writes.load(Ordering::Relaxed),
            writes_before,
            "毒化后的调用不得再把帧投进写线程"
        );

        // ③ 写者卡死时的 `shutdown()` 也必须**有界**返回（不得为了 join 写线程拖挂卸载）。
        let _ = worker_side.shutdown(Shutdown::Both);
        let shutdown_elapsed =
            with_watchdog(Duration::from_secs(5), "写者卡死时的 shutdown", move || {
                let started = Instant::now();
                invoker.shutdown();
                started.elapsed()
            });
        assert!(
            shutdown_elapsed < Duration::from_secs(1),
            "shutdown 必须在 shutdown_timeout 量级内返回（夹具 200ms），实际 {shutdown_elapsed:?}"
        );
        // 收尾：放行夹具，避免测试自身留下永久阻塞的写线程。
        release_gate(&gate);
    }

    /// 健康路径不回归（任务 #56）：写线程逐字节原样写出帧；写线程关停后（stdin 已释放）
    /// 投递仍报 `NotDelivered` —— 那是「确定整帧未写出」的唯一合法来源之一。
    #[test]
    fn healthy_writer_delivers_the_exact_frame_and_closed_stdin_reports_not_delivered() {
        let (host_side, worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let mut worker_reader = worker_side.try_clone().expect("克隆回环连接（worker 读端）");
        let (frame_tx, frame_rx) = mpsc::channel();
        let _worker = std::thread::spawn(move || {
            // 直接读原始字节：既验证分帧（4 字节大端长度），也验证载荷与预期帧逐字节一致。
            let mut len_buf = [0u8; 4];
            worker_reader.read_exact(&mut len_buf).expect("假 worker 应读到长度前缀");
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut payload = vec![0u8; len];
            worker_reader.read_exact(&mut payload).expect("假 worker 应读到完整载荷");
            let request: FrameRequest =
                serde_json::from_slice(&payload).expect("载荷必须是合法请求帧");
            let response = FrameResponse::success(json!({ "pong": true }))
                .with_id(request.request_id.clone().unwrap_or_default());
            let _ = frame_tx.send((request, payload));
            write_json_frame(&mut worker_reader, &response).expect("假 worker 应能回帧");
        });

        let invoker = WorkerInvoker::from_streams(
            "healthy-writer@external",
            Box::new(host_reader),
            // 宿主写端必须是**本端** socket（与 `command_transport` 同形）：写 `worker_side`
            // 会把帧发回本端自己，假 worker 永远收不到。
            Box::new(host_side),
        );
        // 看门狗：本用例末段要验「停机后 NotDelivered」。若实现回归成永久等待，失败形态会是
        // `Drop` 里的 `join_reader` 挂住（进程不退出、也没有 FAILED 行）—— 看门狗把它变成明确失败。
        let (response, request, payload, err, invoker) =
            with_watchdog(Duration::from_secs(10), "健康写者往返与停机", move || {
                let response = invoker
                    .exchange_with_timeout(
                        "ping",
                        json!({ "q": 7 }),
                        Vec::new(),
                        Duration::from_secs(5),
                    )
                    .expect("健康写者必须正常往返");
                let (request, payload) =
                    frame_rx.recv_timeout(Duration::from_secs(2)).expect("应收到宿主写出的帧");

                // 对端关闭 ⇒ 读线程登记死讯；停机 ⇒ 写线程关停（stdin 已释放）。
                let _ = worker_side.shutdown(Shutdown::Both);
                let deadline = Instant::now() + Duration::from_secs(5);
                while !invoker.inner.connection_closed.load(Ordering::Acquire)
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                invoker.shutdown();
                let err = invoker
                    .exchange_with_timeout(
                        "ping",
                        Value::Null,
                        Vec::new(),
                        Duration::from_millis(200),
                    )
                    .expect_err("写线程关停后投递必须报错");
                (response, request, payload, err, invoker)
            });
        assert_eq!(response.value, Some(json!({ "pong": true })), "响应值必须原样带回");
        assert_eq!(request.op, "ping");
        assert_eq!(request.args, json!({ "q": 7 }), "参数必须原样承载");
        assert!(request.request_id.is_some(), "出向帧必须带配对 ID");
        assert!(request.call_chain.is_empty(), "顶层调用的链不带接收方");
        assert_eq!(
            payload,
            serde_json::to_vec(&request).expect("请求帧应可序列化"),
            "写线程必须原样写出帧字节（长度前缀 + JSON 载荷，无额外改动）"
        );
        assert!(
            matches!(err, WorkerError::NotDelivered { .. }),
            "stdin 已释放 ⇒ 确定整帧未写出 ⇒ NotDelivered（回退判据的输入），实际 {err:?}"
        );
        assert!(err.to_string().contains("stdin 已释放"), "死因应指向写通道已关闭，实际：{err}");
        assert!(invoker.inner.pending.lock().is_empty(), "未写出的帧不得在配对表里留槽位");
    }

    /// 入站总闸**未加**（PLAN §32.3-1 裁定）—— 但入站回写与出向写现在共用一条写线程，
    /// 必须锁住「一个卡住的出向写不会让入站回写无限等」：两边各自按预算收口。
    #[test]
    fn inbound_writeback_is_bounded_while_an_outbound_write_is_stuck() {
        let (host_side, mut worker_side) = tcp_pair();
        let host_reader = host_side.try_clone().expect("克隆回环连接（读端）");
        let gate = Arc::new((parking_lot::Mutex::new(false), parking_lot::Condvar::new()));
        let writes = Arc::new(AtomicUsize::new(0));
        let writer = HalfThenBlockWriter { gate: Arc::clone(&gate), writes: Arc::clone(&writes) };
        let inbound_budget = Duration::from_millis(120);
        let invoker = Arc::new(WorkerInvoker::from_streams_with_timeouts(
            "write-stuck@external",
            Box::new(host_reader),
            Box::new(writer),
            inbound_budget,
            Duration::from_millis(100),
        ));

        // 出向写先卡住（它自己也在预算内失败）。
        let outbound = {
            let invoker = Arc::clone(&invoker);
            std::thread::spawn(move || {
                let started = Instant::now();
                let err = invoker
                    .exchange_with_timeout(
                        ops::COMMAND,
                        json!({}),
                        Vec::new(),
                        Duration::from_millis(150),
                    )
                    .expect_err("写者阻塞必须报错");
                (err, started.elapsed())
            })
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while writes.load(Ordering::Relaxed) < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(writes.load(Ordering::Relaxed) >= 2, "写线程应已卡在 write_all 中途");

        // 对端此时主动发一条请求：宿主的回写排在同一条写者后面，必须按自己的预算收口。
        write_json_frame(&mut worker_side, &FrameRequest::new(ops::EMIT, json!({})))
            .expect("假 worker 应能向宿主发请求");
        let deadline = Instant::now() + Duration::from_secs(2);
        while invoker.inner.inbound_inflight.load(Ordering::Acquire) == 0
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            invoker.inner.inbound_inflight.load(Ordering::Acquire) > 0,
            "入站任务应已被派发（读线程不被写侧阻塞）"
        );
        let inbound_started = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(5);
        while invoker.inner.inbound_inflight.load(Ordering::Acquire) > 0
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let inbound_elapsed = inbound_started.elapsed();
        assert_eq!(
            invoker.inner.inbound_inflight.load(Ordering::Acquire),
            0,
            "入站回写不得无限等卡住的出向写（必须有界收场）"
        );
        assert!(
            inbound_elapsed < Duration::from_secs(1),
            "入站回写应在自己的预算量级内收口，实际 {inbound_elapsed:?}"
        );

        let (outbound_err, outbound_elapsed) = outbound.join().expect("出向调用线程应正常结束");
        assert!(
            !matches!(outbound_err, WorkerError::NotDelivered { .. }),
            "两个方向的写都没确认写出时不得报 NotDelivered，实际 {outbound_err:?}"
        );
        assert!(
            outbound_elapsed < Duration::from_secs(2),
            "出向调用应落在自己的预算附近，实际 {outbound_elapsed:?}"
        );

        // 收尾：放行写者 → 关对端 → 有界停机（顺序与生产 Drop 一致）。
        release_gate(&gate);
        let _ = worker_side.shutdown(Shutdown::Both);
        with_watchdog(Duration::from_secs(5), "写者解阻后的停机", move || {
            invoker.shutdown();
        });
    }

    // ─────────────────── 任务 #80：market.quote 入站分派 ───────────────────

    /// `get_klines` 实际收到的参数（用来锁「缺省值」而不是只锁「没报错」）。
    #[derive(Debug, Clone, Default)]
    struct CapturedKlinesArgs {
        stock_code: String,
        period: String,
        limit: u32,
        adj_type: Option<axagent_harness::AdjType>,
    }

    /// 假行情提供者：三方法返回固定 DTO，并记下 `get_klines` 收到的参数。
    struct FixedMarketDataProvider {
        klines_args: Mutex<Option<CapturedKlinesArgs>>,
        /// 记录 `get_financials` 收到的 `as_of`（外层 `None` = 尚未调用）。
        financials_as_of: Mutex<Option<Option<axagent_harness::AsOfContext>>>,
        /// 记录 `get_hot_stocks` 收到的 `as_of`（证明无参 op 也透传 as_of）。
        hot_stocks_as_of: Mutex<Option<Option<axagent_harness::AsOfContext>>>,
        /// 记录 `get_limit_up_pool` 收到的 `requested_date`。
        limit_up_requested: Mutex<Option<Option<String>>>,
        /// 记录 `get_international_klines` 收到的 `adj_type`。
        intl_klines_adj: Mutex<Option<Option<axagent_harness::AdjType>>>,
    }

    impl FixedMarketDataProvider {
        fn new() -> Self {
            Self {
                klines_args: Mutex::new(None),
                financials_as_of: Mutex::new(None),
                hot_stocks_as_of: Mutex::new(None),
                limit_up_requested: Mutex::new(None),
                intl_klines_adj: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl axagent_harness::MarketDataProvider for FixedMarketDataProvider {
        async fn get_quote(
            &self,
            stock_code: &str,
        ) -> axagent_harness::core_error::Result<axagent_harness::StockQuote> {
            Ok(axagent_harness::StockQuote {
                code: stock_code.to_string(),
                name: "浦发银行".to_string(),
                price: 10.5,
                pre_close: 10.4,
                open: 10.45,
                high: 10.6,
                low: 10.3,
                volume: 1_000_000.0,
                amount: 10_500_000.0,
                change_pct: 0.96,
                turnover_rate: 0.5,
                pe: Some(6.1),
                pb: Some(0.6),
                total_mv: Some(300_000_000_000.0),
                circulating_mv: Some(300_000_000_000.0),
                limit_up: Some(11.44),
                limit_down: Some(9.36),
                is_st: false,
                timestamp: "2026-11-02 15:00:00".to_string(),
            })
        }

        async fn get_klines(
            &self,
            stock_code: &str,
            period: &str,
            limit: u32,
            adj_type: Option<axagent_harness::AdjType>,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::KLine>> {
            *self.klines_args.lock() = Some(CapturedKlinesArgs {
                stock_code: stock_code.to_string(),
                period: period.to_string(),
                limit,
                adj_type,
            });
            Ok(vec![axagent_harness::KLine {
                date: "2026-11-02".to_string(),
                open: 10.45,
                high: 10.6,
                low: 10.3,
                close: 10.5,
                volume: 1_000_000.0,
                amount: 10_500_000.0,
                turnover_rate: Some(0.5),
                adj_factor: None,
            }])
        }

        async fn search_stock(
            &self,
            keyword: &str,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::StockSearchResult>> {
            Ok(vec![axagent_harness::StockSearchResult {
                code: "600000".to_string(),
                name: keyword.to_string(),
                market: "main_sh".to_string(),
            }])
        }

        async fn get_financials(
            &self,
            _stock_code: &str,
            as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::FinancialReport>> {
            *self.financials_as_of.lock() = Some(as_of);
            Ok(Vec::new())
        }

        async fn market_snapshot_batch(
            &self,
            codes: &[String],
            _as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::StockQuote>> {
            Ok(codes.iter().map(|code| fixed_quote(code)).collect())
        }

        async fn batch_get_quotes(
            &self,
            request: &axagent_harness::market_data::MarketBatchQuotesRequest,
            _as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<
            axagent_harness::BatchResult<axagent_harness::StockQuote>,
        > {
            let mut batch = axagent_harness::BatchResult::new();
            for code in &request.codes {
                batch.success.insert(code.clone(), fixed_quote(code));
            }
            Ok(batch)
        }

        async fn batch_get_financials(
            &self,
            codes: &[String],
            _as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<
            axagent_harness::BatchResult<Vec<axagent_harness::FinancialReport>>,
        > {
            let mut batch = axagent_harness::BatchResult::new();
            for code in codes {
                batch.success.insert(code.clone(), Vec::new());
            }
            Ok(batch)
        }

        async fn get_hot_stocks(
            &self,
            as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::stock_market_types::HotStock>>
        {
            *self.hot_stocks_as_of.lock() = Some(as_of);
            Ok(Vec::new())
        }

        async fn get_limit_up_pool(
            &self,
            requested_date: Option<&str>,
            _as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<
            Option<axagent_harness::stock_market_types::LimitUpPool>,
        > {
            *self.limit_up_requested.lock() = Some(requested_date.map(str::to_string));
            Ok(None)
        }

        async fn get_international_klines(
            &self,
            _stock_code: &str,
            _period: &str,
            _limit: u32,
            adj_type: Option<axagent_harness::AdjType>,
            _as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::KLine>> {
            *self.intl_klines_adj.lock() = Some(adj_type);
            Ok(Vec::new())
        }

        async fn market_list_page(
            &self,
            _page: u32,
            _as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<axagent_harness::stock_market_types::MarketListPage>
        {
            use axagent_harness::stock_market_types::{MarketListItem, MarketListPage};
            Ok(MarketListPage {
                items: vec![
                    MarketListItem {
                        code: "600000".to_string(),
                        name: "浦发银行".to_string(),
                        industry: Some("银行".to_string()),
                    },
                    MarketListItem {
                        code: "000001".to_string(),
                        name: "平安银行".to_string(),
                        industry: Some("银行".to_string()),
                    },
                ],
                total: 123,
            })
        }

        async fn get_index_quotes_outcome(
            &self,
            _as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<
            axagent_harness::data_outcome::DataOutcome<
                Vec<axagent_harness::stock_market_types::IndexQuote>,
            >,
        > {
            Ok(axagent_harness::data_outcome::DataOutcome::schema_changed(vec![], "field renamed"))
        }
    }

    /// 固定行情快照：`code` 回填入参，其余字段与 `get_quote` 一致。
    fn fixed_quote(code: &str) -> axagent_harness::StockQuote {
        axagent_harness::StockQuote {
            code: code.to_string(),
            name: "浦发银行".to_string(),
            price: 10.5,
            pre_close: 10.4,
            open: 10.45,
            high: 10.6,
            low: 10.3,
            volume: 1_000_000.0,
            amount: 10_500_000.0,
            change_pct: 0.96,
            turnover_rate: 0.5,
            pe: Some(6.1),
            pb: Some(0.6),
            total_mv: Some(300_000_000_000.0),
            circulating_mv: Some(300_000_000_000.0),
            limit_up: Some(11.44),
            limit_down: Some(9.36),
            is_st: false,
            timestamp: "2026-11-02 15:00:00".to_string(),
        }
    }

    /// 装了假行情提供者的局部注册表 + 可在 watchdog 线程里跑 async 接缝的 peer。
    fn market_peer(
        provider: Arc<FixedMarketDataProvider>,
        registry: Arc<CapabilityRegistry>,
        runtime: Option<tokio::runtime::Handle>,
    ) -> HostPeer {
        let provider: Arc<dyn axagent_harness::MarketDataProvider> = provider;
        registry.register_market_data(provider).expect("market.quote 接缝应注册成功");
        HostPeer {
            plugin_id: "plugin.market".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["market.quote".to_string()],
            registry,
            runtime,
            // market.quote 的 op 全部只读：不需要任何写权限。
            permissions: Vec::new(),
        }
    }

    /// 已注册 ⇒ `get_quote` 成功，返回值就是 provider 的 DTO（`code` 即入参）。
    #[test]
    fn market_quote_get_quote_returns_provider_dto() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let peer = market_peer(
            Arc::new(FixedMarketDataProvider::new()),
            Arc::new(CapabilityRegistry::new()),
            Some(runtime.handle().clone()),
        );
        let response = with_watchdog(Duration::from_secs(5), "market.quote get_quote", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "market.quote",
                    "op": "get_quote",
                    "args": { "stock_code": "600000" },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        assert_eq!(value["code"], json!("600000"), "返回值应是 provider 的 StockQuote：{value}");
        assert_eq!(value["price"], json!(10.5));
    }

    /// 未注册 ⇒ 与 `unavailable(seam)` 同码（不猜字符串，直接比同一函数的产出）。
    #[test]
    fn market_quote_without_provider_matches_unavailable_code() {
        let peer = HostPeer {
            plugin_id: "plugin.market".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["market.quote".to_string()],
            registry: Arc::new(CapabilityRegistry::new()),
            runtime: None,
            permissions: Vec::new(),
        };
        let expected = peer.unavailable("market.quote");
        let response =
            with_watchdog(Duration::from_secs(2), "market.quote 缺 provider", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_quote",
                        "args": { "stock_code": "600000" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, expected.code, "缺 provider 必须走 unavailable 的码");
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_PROVIDER_UNAVAILABLE));
    }

    /// 参数缺失 ⇒ `invalid_args` 同码，且不得 panic（provider 不会被调用）。
    #[test]
    fn market_quote_missing_args_is_invalid_without_panic() {
        let peer = market_peer(
            Arc::new(FixedMarketDataProvider::new()),
            Arc::new(CapabilityRegistry::new()),
            None,
        );
        assert_eq!(invalid_args("x").code, Some(error_codes::SEAM_CALL_INVALID_ARGS.to_string()));
        let response = with_watchdog(Duration::from_secs(2), "market.quote 缺参", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "market.quote",
                    "op": "get_klines",
                    "args": { "period": "daily" },
                }),
                &[],
            )
        });
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
    }

    /// `get_klines` 的缺省值：`limit` 缺省 100、`adj_type` 缺省 `None`，其余照传。
    #[test]
    fn market_quote_get_klines_applies_default_limit_and_adj() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let peer = market_peer(
            provider,
            Arc::new(CapabilityRegistry::new()),
            Some(runtime.handle().clone()),
        );
        let response =
            with_watchdog(Duration::from_secs(5), "market.quote get_klines", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_klines",
                        "args": { "stock_code": "600000", "period": "daily" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let seen = captured.klines_args.lock().clone().expect("provider 应被调用");
        assert_eq!(seen.stock_code, "600000");
        assert_eq!(seen.period, "daily");
        assert_eq!(seen.limit, 100, "`limit` 缺省应为 100");
        assert_eq!(seen.adj_type, None, "`adj_type` 缺省应为不复权");
    }

    // ─────────────── P6.1-B：12 个个股数据面的 `as_of` 显式透传 ───────────────

    /// 装假 provider 的 peer + 保活到用例结束的 runtime。
    fn market_peer_with_runtime(
        provider: Arc<FixedMarketDataProvider>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let peer = market_peer(
            provider,
            Arc::new(CapabilityRegistry::new()),
            Some(runtime.handle().clone()),
        );
        (peer, runtime)
    }

    /// `as_of` 字符串形态：`AsOfContext::parse` ⇒ `UserReplay` + `All`，返回值是空报告列表。
    #[test]
    fn market_quote_get_financials_passes_as_of_string() {
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = market_peer_with_runtime(provider);
        let response =
            with_watchdog(Duration::from_secs(5), "market.quote get_financials", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_financials",
                        "args": { "stock_code": "600519", "as_of": "2024-01-02" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        assert_eq!(response.value, Some(json!([])), "替身返回空报告列表");
        let seen = (*captured.financials_as_of.lock()).expect("provider 应被调用");
        let ctx = seen.expect("字符串形态必须解析为 Some(ctx)");
        assert_eq!(ctx.as_of_date.to_string(), "2024-01-02");
        assert_eq!(ctx.source, AsOfSource::UserReplay);
        assert_eq!(ctx.data_scope, AsOfDataScope::All);
    }

    /// `as_of` 对象形态：`source` / `data_scope` 原样透传，不得被重置成缺省。
    #[test]
    fn market_quote_get_financials_passes_as_of_object_verbatim() {
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = market_peer_with_runtime(provider);
        let response = with_watchdog(
            Duration::from_secs(5),
            "market.quote get_financials 对象",
            move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_financials",
                        "args": {
                            "stock_code": "600519",
                            "as_of": {
                                "as_of_date": "2024-01-02",
                                "source": "scheduled_replay",
                                "data_scope": "structured",
                            },
                        },
                    }),
                    &[],
                )
            },
        );
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let seen = (*captured.financials_as_of.lock()).expect("provider 应被调用");
        let ctx = seen.expect("对象形态必须解析为 Some(ctx)");
        assert_eq!(ctx.as_of_date.to_string(), "2024-01-02");
        assert_eq!(ctx.source, AsOfSource::ScheduledReplay, "source 不得丢失");
        assert_eq!(ctx.data_scope, AsOfDataScope::Structured, "data_scope 不得丢失");
    }

    /// 缺 `stock_code` ⇒ 既有 invalid-args 错误码，provider 不被调用。
    #[test]
    fn market_quote_get_financials_missing_stock_code_is_invalid() {
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = market_peer_with_runtime(provider);
        let response =
            with_watchdog(Duration::from_secs(2), "market.quote 缺 stock_code", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_financials",
                        "args": { "as_of": "2024-01-02" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
        assert!(captured.financials_as_of.lock().is_none(), "参数非法时不应调用 provider");
    }

    /// 未覆写的方法走 trait 默认实现 ⇒ `PLUGIN_CALL_FAILED`，错误文本含「未实现」。
    #[test]
    fn market_quote_unsupported_data_face_reports_not_implemented() {
        let (peer, _runtime) = market_peer_with_runtime(Arc::new(FixedMarketDataProvider::new()));
        let response =
            with_watchdog(Duration::from_secs(5), "market.quote get_money_flow", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_money_flow",
                        "args": { "stock_code": "600519" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_CALL_FAILED));
        let detail = response.message.unwrap_or_default();
        assert!(detail.contains("未实现"), "默认实现应报未实现，实际：{detail}");
    }

    /// `market_snapshot_batch` 的 `codes` 原样下传，返回条数与 `code` 与入参一致。
    #[test]
    fn market_quote_snapshot_batch_returns_one_quote_per_code() {
        let (peer, _runtime) = market_peer_with_runtime(Arc::new(FixedMarketDataProvider::new()));
        let response =
            with_watchdog(Duration::from_secs(5), "market.quote snapshot_batch", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "market_snapshot_batch",
                        "args": { "codes": ["600519", "000001"] },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        let quotes = value.as_array().expect("返回值应是行情数组");
        assert_eq!(quotes.len(), 2, "每个入参 code 一条：{value}");
        assert_eq!(quotes[0]["code"], json!("600519"));
        assert_eq!(quotes[1]["code"], json!("000001"));
    }

    /// `batch_get_quotes`：扁平 snake_case 参数 ⇒ 强类型 DTO；返回 BatchResult 双 map
    /// camelCase 线上形态（success/failures/elapsedMs/earlyTerminated）。
    #[test]
    fn market_quote_batch_get_quotes_returns_batch_result_wire_shape() {
        let (peer, _runtime) = market_peer_with_runtime(Arc::new(FixedMarketDataProvider::new()));
        let response =
            with_watchdog(Duration::from_secs(5), "market.quote batch_get_quotes", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "batch_get_quotes",
                        "args": {
                            "codes": ["600519", "000001"],
                            "per_stock_timeout_ms": 5000u64,
                            "total_timeout_ms": 30000u64,
                            "max_failures": 2usize,
                        },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        let success = value["success"].as_object().expect("success 应是逐 code map");
        assert_eq!(success.len(), 2, "每个入参 code 一条：{value}");
        assert_eq!(success["600519"]["code"], json!("600519"));
        assert!(value["failures"].is_object(), "failures 必须存在（可空）：{value}");
        assert!(value["elapsedMs"].is_number(), "elapsedMs 应是数字：{value}");
        assert_eq!(value["earlyTerminated"], json!(false));
    }

    /// `batch_get_financials`：codes 原样下传，逐 code 归因进 success map。
    #[test]
    fn market_quote_batch_get_financials_returns_batch_result_per_code() {
        let (peer, _runtime) = market_peer_with_runtime(Arc::new(FixedMarketDataProvider::new()));
        let response =
            with_watchdog(Duration::from_secs(5), "market.quote batch_get_financials", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "batch_get_financials",
                        "args": { "codes": ["600519", "000001"] },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        let success = value["success"].as_object().expect("success 应是逐 code map");
        assert_eq!(success.len(), 2, "每个入参 code 一条：{value}");
        assert!(success["600519"].is_array() && success["600519"].as_array().unwrap().is_empty());
        assert!(success.contains_key("000001"));
    }

    /// `as_of` 非法字符串 ⇒ invalid args，且不得 panic / 不得触达 provider。
    #[test]
    fn market_quote_invalid_as_of_is_invalid_args_without_panic() {
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = market_peer_with_runtime(provider);
        let response =
            with_watchdog(Duration::from_secs(2), "market.quote 非法 as_of", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_financials",
                        "args": { "stock_code": "600519", "as_of": "not-a-date" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
        assert!(captured.financials_as_of.lock().is_none(), "非法 as_of 不应调用 provider");
    }

    #[test]
    fn market_quote_object_as_of_rejects_future_date() {
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = market_peer_with_runtime(provider);
        let response = with_watchdog(
            Duration::from_secs(2),
            "market.quote 对象 as_of 未来日期",
            move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_financials",
                        "args": {
                            "stock_code": "600519",
                            "as_of": {
                                "as_of_date": "2099-01-01",
                                "source": "user_replay",
                                "data_scope": "all",
                            },
                        },
                    }),
                    &[],
                )
            },
        );
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
        assert!(
            response.message.as_deref().is_some_and(|d| d.contains("as_of")),
            "错误文案应指明 as_of：{:?}",
            response.message
        );
        assert!(captured.financials_as_of.lock().is_none(), "未来日期不应调用 provider");
    }

    // ─────────────── P6.1-D：剩余 25 个数据面的接线 ───────────────

    /// 无参 op 也必须解析并透传 `as_of`（不读任何业务键）。
    #[test]
    fn market_quote_get_hot_stocks_passes_as_of_without_args() {
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = market_peer_with_runtime(provider);
        let response =
            with_watchdog(Duration::from_secs(5), "market.quote get_hot_stocks", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_hot_stocks",
                        "args": { "as_of": "2024-01-02" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        assert_eq!(response.value, Some(json!([])), "替身返回空热股列表");
        let seen = (*captured.hot_stocks_as_of.lock()).expect("provider 应被调用");
        let ctx = seen.expect("无参 op 也必须收到 Some(ctx)");
        assert_eq!(ctx.as_of_date.to_string(), "2024-01-02");
    }

    /// `requested_date` 是可选项：带键透传 `Some(&str)`，缺键传 `None`。
    #[test]
    fn market_quote_limit_up_pool_forwards_optional_requested_date() {
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = market_peer_with_runtime(provider);
        let (with_date, without_date, seen_with, seen_without) =
            with_watchdog(Duration::from_secs(5), "market.quote get_limit_up_pool", move || {
                let with_date = peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_limit_up_pool",
                        "args": { "requested_date": "2024-01-02" },
                    }),
                    &[],
                );
                let seen_with = (*captured.limit_up_requested.lock()).clone();
                let without_date = peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_limit_up_pool",
                        "args": {},
                    }),
                    &[],
                );
                let seen_without = (*captured.limit_up_requested.lock()).clone();
                (with_date, without_date, seen_with, seen_without)
            });
        assert_eq!(with_date.code, None, "带 requested_date 应成功：{with_date:?}");
        assert_eq!(with_date.value, Some(Value::Null), "替身返回 Ok(None) ⇒ null");
        assert_eq!(seen_with, Some(Some("2024-01-02".to_string())));
        assert_eq!(without_date.code, None, "缺 requested_date 应成功：{without_date:?}");
        assert_eq!(seen_without, Some(None), "缺键应传 None，而不是空串");
    }

    /// `market_list_page` 的 DTO 走 camelCase，`total` 原样保留。
    #[test]
    fn market_quote_market_list_page_returns_items_and_total() {
        let (peer, _runtime) = market_peer_with_runtime(Arc::new(FixedMarketDataProvider::new()));
        let response =
            with_watchdog(Duration::from_secs(5), "market.quote market_list_page", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "market_list_page",
                        "args": { "page": 3 },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        let items = value["items"].as_array().expect("items 应是数组");
        assert_eq!(items.len(), 2, "替身返回两条：{value}");
        assert_eq!(items[0]["code"], json!("600000"));
        assert_eq!(items[0]["name"], json!("浦发银行"));
        assert_eq!(items[0]["industry"], json!("银行"));
        assert_eq!(value["total"], json!(123));
    }

    /// `DataOutcome` 信封原样透出：`status` 与 `detail` 都不得丢失。
    #[test]
    fn market_quote_index_quotes_outcome_keeps_schema_changed_detail() {
        let (peer, _runtime) = market_peer_with_runtime(Arc::new(FixedMarketDataProvider::new()));
        let response = with_watchdog(
            Duration::from_secs(5),
            "market.quote get_index_quotes_outcome",
            move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_index_quotes_outcome",
                        "args": {},
                    }),
                    &[],
                )
            },
        );
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        assert_eq!(value["status"], json!("schema_changed"));
        assert_eq!(value["detail"], json!("field renamed"));
        assert_eq!(value["data"], json!([]));
    }

    /// `adj_type` 复用 `AdjType` 反序列化：`"forward"` ⇒ `Some(Forward)`。
    #[test]
    fn market_quote_international_klines_forwards_adj_type() {
        let provider = Arc::new(FixedMarketDataProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = market_peer_with_runtime(provider);
        let response = with_watchdog(
            Duration::from_secs(5),
            "market.quote get_international_klines",
            move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "get_international_klines",
                        "args": {
                            "stock_code": "AAPL",
                            "period": "daily",
                            "limit": 20,
                            "adj_type": "forward",
                        },
                    }),
                    &[],
                )
            },
        );
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        assert_eq!(response.value, Some(json!([])), "替身返回空 K 线");
        let seen = (*captured.intl_klines_adj.lock()).expect("provider 应被调用");
        assert_eq!(seen, Some(axagent_harness::AdjType::Forward));
    }

    /// 缺必填 `keyword` ⇒ invalid args；此时 provider 不会被触达
    /// （peer 无 runtime：若走到 provider 会先报 `no_runtime` 而非 invalid args）。
    #[test]
    fn market_quote_search_news_missing_keyword_is_invalid() {
        let peer = market_peer(
            Arc::new(FixedMarketDataProvider::new()),
            Arc::new(CapabilityRegistry::new()),
            None,
        );
        let response = with_watchdog(
            Duration::from_secs(2),
            "market.quote search_news 缺 keyword",
            move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "market.quote",
                        "op": "search_news",
                        "args": { "limit": 5 },
                    }),
                    &[],
                )
            },
        );
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
        assert!(
            response.message.as_deref().is_some_and(|d| d.contains("keyword")),
            "错误文案应指明 keyword：{:?}",
            response.message
        );
    }

    // ───── jobs.store 读写接缝（读：list / get / count；写：add / remove / set_status） ─────

    /// 假定时任务提供者：记录读 / 写调用收到的参数，返回固定结果。
    struct FixedJobStoreProvider {
        /// 外层 `None` = 尚未调用；内层是 `(task_type, active_only)`。
        list_args: Mutex<Option<(Option<String>, bool)>>,
        /// `add_job` 收到的任务（外层 `None` = 尚未调用）。
        added_job: Mutex<Option<axagent_harness::NewCronJob>>,
        /// `remove_job` 收到的 id 序列（保留次数与顺序）。
        removed_ids: Mutex<Vec<String>>,
        /// `set_job_status` 收到的 `(id, status)` 序列（保留次数与顺序）。
        status_calls: Mutex<Vec<(String, String)>>,
    }

    impl FixedJobStoreProvider {
        fn new() -> Self {
            Self {
                list_args: Mutex::new(None),
                added_job: Mutex::new(None),
                removed_ids: Mutex::new(Vec::new()),
                status_calls: Mutex::new(Vec::new()),
            }
        }

        /// 按契约为出一条字段齐全的视图（仅 `status` 随用例变化）。
        fn sample_job(id: &str, status: &str) -> axagent_harness::CronJobView {
            axagent_harness::CronJobView {
                id: id.to_string(),
                name: format!("任务 {id}"),
                description: "jobs.store 测试替身".to_string(),
                schedule: "0 9 * * *".to_string(),
                prompt: "复盘今日持仓".to_string(),
                workflow_id: None,
                task_type: Some("stock-analysis".to_string()),
                platform: None,
                enabled_toolsets: None,
                status: status.to_string(),
                recurring: true,
                run_count: 3,
                last_run_at: Some(1_730_000_000),
                last_result: None,
                last_trigger: Some("schedule".to_string()),
                next_run_at: Some(1_730_086_400),
                priority: "normal".to_string(),
                created_at: 1_729_000_000,
                updated_at: 1_729_900_000,
            }
        }
    }

    #[async_trait]
    impl axagent_harness::JobStoreProvider for FixedJobStoreProvider {
        async fn list_jobs(
            &self,
            task_type: Option<&str>,
            active_only: bool,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::CronJobView>> {
            *self.list_args.lock() = Some((task_type.map(str::to_string), active_only));
            Ok(vec![
                Self::sample_job("job-active", "active"),
                Self::sample_job("job-paused", "paused"),
            ])
        }

        async fn get_job(
            &self,
            id: &str,
        ) -> axagent_harness::core_error::Result<Option<axagent_harness::CronJobView>> {
            if id == "exists" {
                Ok(Some(Self::sample_job("exists", "active")))
            } else {
                Ok(None)
            }
        }

        async fn count_jobs(
            &self,
            _task_type: Option<&str>,
        ) -> axagent_harness::core_error::Result<u64> {
            Ok(7)
        }

        async fn add_job(
            &self,
            job: axagent_harness::NewCronJob,
        ) -> axagent_harness::core_error::Result<String> {
            let id = job.id.clone();
            *self.added_job.lock() = Some(job);
            Ok(id)
        }

        async fn remove_job(
            &self,
            id: &str,
        ) -> axagent_harness::core_error::Result<axagent_harness::RemoveJobOutcome> {
            self.removed_ids.lock().push(id.to_string());
            Ok(match id {
                "protected-id" => axagent_harness::RemoveJobOutcome::Protected,
                "gone" => axagent_harness::RemoveJobOutcome::NotFound,
                _ => axagent_harness::RemoveJobOutcome::Removed,
            })
        }

        async fn set_job_status(
            &self,
            id: &str,
            status: &str,
        ) -> axagent_harness::core_error::Result<bool> {
            self.status_calls.lock().push((id.to_string(), status.to_string()));
            Ok(id != "missing")
        }
    }

    /// 装假 jobs 提供者的 peer + 保活到用例结束的 runtime。
    ///
    /// 缺省**不带任何权限**：只读 op 不受权限闸影响；写 op 用例请用
    /// [`Self::jobs_peer_with_runtime_and_permissions`]。
    fn jobs_peer_with_runtime(
        provider: Arc<FixedJobStoreProvider>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        jobs_peer_with_runtime_and_permissions(provider, Vec::new())
    }

    /// 同 [`Self::jobs_peer_with_runtime`]，但显式注入清单权限（写 op 准入用例）。
    fn jobs_peer_with_runtime_and_permissions(
        provider: Arc<FixedJobStoreProvider>,
        permissions: Vec<PluginPermission>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        let provider: Arc<dyn axagent_harness::JobStoreProvider> = provider;
        registry.register_jobs_store(provider).expect("jobs.store 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.jobs".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["jobs.store".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions,
        };
        (peer, runtime)
    }

    /// `task_type` / `active_only` 必须原样透传给 provider，返回值是视图数组。
    #[test]
    fn jobs_store_list_forwards_filters() {
        let provider = Arc::new(FixedJobStoreProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = jobs_peer_with_runtime(provider);
        let response = with_watchdog(Duration::from_secs(5), "jobs.store list", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "jobs.store",
                    "op": "list",
                    "args": { "task_type": "stock-analysis", "active_only": true },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let seen = captured.list_args.lock().clone().expect("provider 应被调用");
        assert_eq!(seen, (Some("stock-analysis".to_string()), true), "过滤参数应原样透传");
        let value = response.value.expect("成功响应必须带 value");
        assert_eq!(value.as_array().map(Vec::len), Some(2), "应返回 2 条任务：{value}");
    }

    /// 省略可选参数：`task_type` ⇒ `None`，`active_only` ⇒ `false`（不得报缺参）。
    #[test]
    fn jobs_store_list_defaults_active_only_false() {
        let provider = Arc::new(FixedJobStoreProvider::new());
        let captured = provider.clone();
        let (peer, _runtime) = jobs_peer_with_runtime(provider);
        let response = with_watchdog(Duration::from_secs(5), "jobs.store list 缺省", move || {
            peer.handle_call_seam(&json!({ "seam": "jobs.store", "op": "list", "args": {} }), &[])
        });
        assert_eq!(response.code, None, "省略可选参数应成功，实际 {response:?}");
        let seen = captured.list_args.lock().clone().expect("provider 应被调用");
        assert_eq!(seen, (None, false), "`active_only` 缺省必须为 false");
    }

    /// 查无此任务 ⇒ `null`，**不是错误**（不得借用错误码表达「没有」）。
    #[test]
    fn jobs_store_get_missing_returns_null_not_error() {
        let (peer, _runtime) = jobs_peer_with_runtime(Arc::new(FixedJobStoreProvider::new()));
        let response = with_watchdog(Duration::from_secs(5), "jobs.store get 缺失", move || {
            peer.handle_call_seam(
                &json!({ "seam": "jobs.store", "op": "get", "args": { "id": "nope" } }),
                &[],
            )
        });
        assert_eq!(response.code, None, "不存在不是错误，实际 {response:?}");
        assert_eq!(response.value, Some(Value::Null), "返回值应为 JSON null");
    }

    /// `count` 返回具名对象 `{ "count": n }`（不是裸数字）——与返回值形状契约一致。
    #[test]
    fn jobs_store_count_returns_named_object() {
        let (peer, _runtime) = jobs_peer_with_runtime(Arc::new(FixedJobStoreProvider::new()));
        let response = with_watchdog(Duration::from_secs(5), "jobs.store count", move || {
            peer.handle_call_seam(&json!({ "seam": "jobs.store", "op": "count", "args": {} }), &[])
        });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        assert_eq!(response.value, Some(json!({ "count": 7 })));
    }

    /// 缺必填 `id` ⇒ invalid args，且不得触达 provider（peer 无 runtime 亦可证明）。
    #[test]
    fn jobs_store_get_missing_id_is_invalid() {
        let (peer, _runtime) = jobs_peer_with_runtime(Arc::new(FixedJobStoreProvider::new()));
        let response = with_watchdog(Duration::from_secs(2), "jobs.store get 缺 id", move || {
            peer.handle_call_seam(&json!({ "seam": "jobs.store", "op": "get", "args": {} }), &[])
        });
        assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
    }

    // ─────────────── 任务 #87：接缝写 op 的权限闸（本批只登记 `jobs.store` 四个写 op） ───────────────

    /// 登记表：只有 `jobs.store` 的四个写 op 需要 `write`；只读 op 与其它接缝一律不设闸。
    #[test]
    fn required_permission_for_jobs_writes_requires_write() {
        let verdict = with_watchdog(Duration::from_secs(2), "required_permission_for 表", || {
            let writes: Vec<Option<PluginPermission>> = ["add", "remove", "set_status", "update"]
                .into_iter()
                .map(|op| required_permission_for("jobs.store", op))
                .collect();
            let reads: Vec<Option<PluginPermission>> = ["list", "get", "count"]
                .into_iter()
                .map(|op| required_permission_for("jobs.store", op))
                .collect();
            // 接缝维度必须参与判定：同名 op 换到 market.quote 不得再要求 write。
            let market: Vec<Option<PluginPermission>> = ["add", "remove", "set_status", "update"]
                .into_iter()
                .map(|op| required_permission_for("market.quote", op))
                .collect();
            (writes, reads, market)
        });
        assert_eq!(verdict.0, [Some(PluginPermission::Write); 4], "四个写 op 都必须要求 write");
        assert_eq!(verdict.1, [None, None, None], "list / get / count 不得设权限闸");
        assert_eq!(verdict.2, [None; 4], "market.quote 任意 op 都不得设权限闸");
    }

    /// 未声明 `write` ⇒ 写 op 在分派前被拒（`PLUGIN_PERMISSION_DENIED`），文案点明权限与接缝。
    #[test]
    fn jobs_store_write_without_permission_is_denied() {
        // 权限来自「缺省配置」：`PluginWorkerConfig::new` 的 permissions 是空表。
        let denied_config_permissions =
            PluginWorkerConfig::new("plugin.jobs", "plugin.exe").permissions;
        assert!(denied_config_permissions.is_empty(), "缺省配置必须不带任何权限");

        let provider = Arc::new(FixedJobStoreProvider::new());
        let (peer, _runtime) =
            jobs_peer_with_runtime_and_permissions(provider, denied_config_permissions);
        let response =
            with_watchdog(Duration::from_secs(5), "jobs.store remove 无权限", move || {
                peer.handle_call_seam(
                    &json!({ "seam": "jobs.store", "op": "remove", "args": { "id": "job-x" } }),
                    &[],
                )
            });
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 权限必须被拒，实际 {response:?}"
        );
        let detail = response.message.unwrap_or_default();
        assert!(detail.contains("write"), "文案应点明缺失的权限，实际：{detail}");
        assert!(detail.contains("jobs.store"), "文案应点明接缝，实际：{detail}");

        // 不触达 provider 的取证：注册表**空**（无 jobs provider）时仍是权限拒绝，
        // 而不是 `SEAM_PROVIDER_UNAVAILABLE` —— 说明闸在取 provider 之前就返回了。
        let bare = HostPeer {
            plugin_id: "plugin.jobs".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["jobs.store".to_string()],
            registry: Arc::new(CapabilityRegistry::new()),
            runtime: None,
            permissions: Vec::new(),
        };
        let bare_response =
            with_watchdog(Duration::from_secs(2), "jobs.store remove 无 provider", move || {
                bare.handle_call_seam(
                    &json!({ "seam": "jobs.store", "op": "remove", "args": { "id": "job-x" } }),
                    &[],
                )
            });
        assert_eq!(
            bare_response.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "权限闸必须先于 provider 查找，实际 {bare_response:?}"
        );
    }

    /// 已声明 `write` ⇒ 不再被权限闸拦；`update` 本批未实现，故落到「未知 op」的既有错误码。
    ///
    /// ⚠ 盘上口径：**已知接缝的未知 op** 走 `dispatch_seam` 末尾的 `SEAM_CALL_UNSUPPORTED`
    /// （`PLUGIN_UNKNOWN_OP` 只用于未知**帧** op，见 `HostPeer::handle_inbound`）。本用例按
    /// 盘上断言，未为 `update` 新增分派分支。
    #[test]
    fn jobs_store_write_with_permission_falls_through_to_unknown_op() {
        let (peer, _runtime) = jobs_peer_with_runtime_and_permissions(
            Arc::new(FixedJobStoreProvider::new()),
            vec![PluginPermission::Write],
        );
        let response =
            with_watchdog(Duration::from_secs(5), "jobs.store update 有权限", move || {
                peer.handle_call_seam(
                    &json!({ "seam": "jobs.store", "op": "update", "args": {} }),
                    &[],
                )
            });
        assert_ne!(
            response.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "已声明 write 时不得再被权限闸拦，实际 {response:?}"
        );
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::SEAM_CALL_UNSUPPORTED),
            "`update` 尚未实现 ⇒ 落入未知 op 的既有错误码，实际 {response:?}"
        );
    }

    // ───────────────────── jobs.store 写面：add / remove / set_status ─────────────────────

    /// `add` 把任务原样交给 provider；可选字段缺省由 harness DTO 提供
    /// （`recurring = true` / `priority = "medium"`），分派层不得另填一套默认值。
    #[test]
    fn jobs_store_add_forwards_new_job_and_defaults() {
        let provider = Arc::new(FixedJobStoreProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) =
            jobs_peer_with_runtime_and_permissions(provider, vec![PluginPermission::Write]);
        let response = with_watchdog(Duration::from_secs(5), "jobs.store add", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "jobs.store",
                    "op": "add",
                    "args": {
                        "job": {
                            "id": "job-new",
                            "name": "晨间复盘",
                            "description": "开盘前检查持仓",
                            "schedule": "0 9 * * *",
                            "prompt": "复盘今日持仓",
                        },
                    },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "已注册且已授权时应成功，实际 {response:?}");
        let job = captured.added_job.lock().clone().expect("provider 应收到 add_job");
        assert_eq!(job.id, "job-new", "id 应原样透传");
        assert_eq!(job.schedule, "0 9 * * *", "schedule 应原样透传");
        assert!(job.recurring, "`recurring` 缺省必须为 true");
        assert_eq!(job.priority, "medium", "`priority` 缺省必须为 medium");
        assert_eq!(response.value, Some(json!({ "id": "job-new" })));
    }

    /// 内置种子任务受保护：`remove` 的三态结果原样透出，`protected` 是**字符串**。
    #[test]
    fn jobs_store_remove_reports_protected() {
        let provider = Arc::new(FixedJobStoreProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) =
            jobs_peer_with_runtime_and_permissions(provider, vec![PluginPermission::Write]);
        let response =
            with_watchdog(Duration::from_secs(5), "jobs.store remove 受保护", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "jobs.store",
                        "op": "remove",
                        "args": { "id": "protected-id" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "受保护未删除不是错误，实际 {response:?}");
        assert_eq!(response.value, Some(json!("protected")));
        assert_eq!(
            captured.removed_ids.lock().clone(),
            vec!["protected-id".to_string()],
            "id 应原样透传给 remove_job"
        );
    }

    /// `set_status` 原样转发 `(id, status)`，并用 `found` 区分「不存在」。
    #[test]
    fn jobs_store_set_status_forwards_and_reports_found() {
        let provider = Arc::new(FixedJobStoreProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = jobs_peer_with_runtime_and_permissions(
            Arc::clone(&provider),
            vec![PluginPermission::Write],
        );
        let response =
            with_watchdog(Duration::from_secs(5), "jobs.store set_status 暂停", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "jobs.store",
                        "op": "set_status",
                        "args": { "id": "x", "status": "paused" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已授权时应成功，实际 {response:?}");
        assert_eq!(response.value, Some(json!({ "found": true })));

        // 不存在 ⇒ `found: false`，仍不是错误（同一个替身继续记录调用）。
        let (peer_missing, _runtime_missing) =
            jobs_peer_with_runtime_and_permissions(provider, vec![PluginPermission::Write]);
        let missing =
            with_watchdog(Duration::from_secs(5), "jobs.store set_status 缺失", move || {
                peer_missing.handle_call_seam(
                    &json!({
                        "seam": "jobs.store",
                        "op": "set_status",
                        "args": { "id": "missing", "status": "paused" },
                    }),
                    &[],
                )
            });
        assert_eq!(missing.code, None, "任务不存在不是错误，实际 {missing:?}");
        assert_eq!(missing.value, Some(json!({ "found": false })));

        assert_eq!(
            captured.status_calls.lock().clone(),
            vec![
                ("x".to_string(), "paused".to_string()),
                ("missing".to_string(), "paused".to_string()),
            ],
            "两次调用都应原样透传 (id, status)"
        );
    }

    /// 非法状态字面量必须在分派层拒绝：`SEAM_CALL_INVALID_ARGS`，且不触达 provider。
    #[test]
    fn jobs_store_set_status_rejects_unknown_literal() {
        let provider = Arc::new(FixedJobStoreProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) =
            jobs_peer_with_runtime_and_permissions(provider, vec![PluginPermission::Write]);
        let response =
            with_watchdog(Duration::from_secs(2), "jobs.store set_status 非法", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "jobs.store",
                        "op": "set_status",
                        "args": { "id": "x", "status": "running" },
                    }),
                    &[],
                )
            });
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "非法字面量必须走参数错误，实际 {response:?}"
        );
        let message = response.message.unwrap_or_default();
        for allowed in ["active", "paused", "disabled"] {
            assert!(message.contains(allowed), "文案应列出允许值 `{allowed}`，实际：{message}");
        }
        assert!(captured.status_calls.lock().is_empty(), "非法字面量不得送进 provider");
    }

    /// 缺必填 `job` ⇒ 参数错误，且不触达 provider。
    #[test]
    fn jobs_store_add_missing_job_is_invalid() {
        let provider = Arc::new(FixedJobStoreProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) =
            jobs_peer_with_runtime_and_permissions(provider, vec![PluginPermission::Write]);
        let response = with_watchdog(Duration::from_secs(2), "jobs.store add 缺 job", move || {
            peer.handle_call_seam(&json!({ "seam": "jobs.store", "op": "add", "args": {} }), &[])
        });
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `job` 必须走参数错误，实际 {response:?}"
        );
        assert!(captured.added_job.lock().is_none(), "缺参不得触达 provider");
    }

    // ───── workflow.run 接缝（start / status / cancel / list_active / list_recent） ─────

    /// 假工作流执行提供者：记录每次调用的 `owner` 与参数，返回可控结果。
    struct FixedWorkflowRunProvider {
        /// `start` 收到的 `(owner, request)` 序列。
        start_calls: Mutex<Vec<(String, WorkflowRunRequest)>>,
        /// `status` 收到的 `(owner, execution_id, include_output)` 序列。
        status_calls: Mutex<Vec<(String, String, bool)>>,
        /// `cancel` 收到的 `(owner, execution_id)` 序列。
        cancel_calls: Mutex<Vec<(String, String)>>,
        /// `list_active` 收到的 `(owner, workflow_id, limit)` 序列。
        list_calls: Mutex<Vec<(String, Option<String>, usize)>>,
        /// `list_recent` 收到的 `(owner, workflow_id, limit)` 序列。
        recent_calls: Mutex<Vec<(String, Option<String>, usize)>>,
        /// `Some(msg)` ⇒ `start` 返回 `Err(AxAgentError::Validation(msg))`（模拟宿主拒绝受理）。
        start_rejection: Option<String>,
    }

    impl FixedWorkflowRunProvider {
        fn new() -> Self {
            Self {
                start_calls: Mutex::new(Vec::new()),
                status_calls: Mutex::new(Vec::new()),
                cancel_calls: Mutex::new(Vec::new()),
                list_calls: Mutex::new(Vec::new()),
                recent_calls: Mutex::new(Vec::new()),
                start_rejection: None,
            }
        }

        /// 让 `start` 按「宿主拒绝受理」（模板不在白名单 / 准入并发超限）失败。
        fn rejecting(reason: &str) -> Self {
            Self { start_rejection: Some(reason.to_string()), ..Self::new() }
        }

        /// 契约里 `start` 的受理回执（字段值固定，供断言整体比对）。
        fn handle() -> WorkflowRunHandle {
            WorkflowRunHandle {
                execution_id: "exec-1".to_string(),
                workflow_id: "wf-1".to_string(),
                accepted_at: 1_700_000_000_000,
            }
        }
    }

    #[async_trait]
    impl WorkflowRunProvider for FixedWorkflowRunProvider {
        async fn start(
            &self,
            owner: &str,
            request: WorkflowRunRequest,
        ) -> axagent_harness::core_error::Result<WorkflowRunHandle> {
            self.start_calls.lock().push((owner.to_string(), request));
            match &self.start_rejection {
                Some(reason) => Err(AxAgentError::Validation(reason.clone())),
                None => Ok(Self::handle()),
            }
        }

        async fn status(
            &self,
            owner: &str,
            execution_id: &str,
            include_output: bool,
        ) -> axagent_harness::core_error::Result<Option<ExecutionView>> {
            self.status_calls.lock().push((
                owner.to_string(),
                execution_id.to_string(),
                include_output,
            ));
            // 查无固定回 `None`：分派层必须把它翻成 JSON `null` 而非错误。
            Ok(None)
        }

        async fn cancel(
            &self,
            owner: &str,
            execution_id: &str,
        ) -> axagent_harness::core_error::Result<WorkflowRunCancelOutcome> {
            self.cancel_calls.lock().push((owner.to_string(), execution_id.to_string()));
            Ok(WorkflowRunCancelOutcome {
                cancelled: true,
                status: Some(axagent_harness::workflow_types::ExecutionStatus::Cancelled),
            })
        }

        async fn list_active(
            &self,
            owner: &str,
            workflow_id: Option<&str>,
            limit: usize,
        ) -> axagent_harness::core_error::Result<WorkflowRunActivePage> {
            self.list_calls.lock().push((
                owner.to_string(),
                workflow_id.map(str::to_string),
                limit,
            ));
            Ok(WorkflowRunActivePage {
                executions: vec![WorkflowRunActiveView {
                    execution_id: "exec-1".to_string(),
                    workflow_id: "wf-1".to_string(),
                    status: axagent_harness::workflow_types::ExecutionStatus::Running,
                    current_node_id: Some("node-2".to_string()),
                    owner_plugin_id: owner.to_string(),
                    created_at: 1_700_000_000_000,
                    updated_at: 1_700_000_001_000,
                    finished_at: None,
                }],
                total: 1,
            })
        }

        async fn list_recent(
            &self,
            owner: &str,
            workflow_id: Option<&str>,
            limit: usize,
        ) -> axagent_harness::core_error::Result<WorkflowRunActivePage> {
            self.recent_calls.lock().push((
                owner.to_string(),
                workflow_id.map(str::to_string),
                limit,
            ));
            // 一页里同时给「在跑」与「已结束」：这是 `list_recent` 与 `list_active` 唯一的
            // 形状差别，也是耗时列能不能算出来的分水岭 ⇒ 分派层的线格式要在这里钉住。
            Ok(WorkflowRunActivePage {
                executions: vec![
                    WorkflowRunActiveView {
                        execution_id: "exec-1".to_string(),
                        workflow_id: "wf-1".to_string(),
                        status: axagent_harness::workflow_types::ExecutionStatus::Running,
                        current_node_id: Some("node-2".to_string()),
                        owner_plugin_id: owner.to_string(),
                        created_at: 1_700_000_000_000,
                        updated_at: 1_700_000_001_000,
                        finished_at: None,
                    },
                    WorkflowRunActiveView {
                        execution_id: "exec-0".to_string(),
                        workflow_id: "wf-1".to_string(),
                        status: axagent_harness::workflow_types::ExecutionStatus::Completed,
                        current_node_id: Some("node-5".to_string()),
                        owner_plugin_id: owner.to_string(),
                        created_at: 1_699_999_990_000,
                        updated_at: 1_699_999_999_000,
                        finished_at: Some(1_699_999_999_000),
                    },
                ],
                total: 2,
            })
        }
    }

    /// 装假 workflow.run 提供者的 peer + 保活到用例结束的 runtime。
    ///
    /// 缺省带 `write` 权限（`start` / `cancel` 的准入需要）；只读 op 请用
    /// [`Self::workflow_run_peer_with_permissions`] 传空表，以证明它们不设闸。
    fn workflow_run_peer(
        provider: Arc<FixedWorkflowRunProvider>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        workflow_run_peer_with_permissions(provider, vec![PluginPermission::Write])
    }

    /// 同 [`Self::workflow_run_peer`]，但显式注入清单权限。
    fn workflow_run_peer_with_permissions(
        provider: Arc<FixedWorkflowRunProvider>,
        permissions: Vec<PluginPermission>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        let provider: Arc<dyn axagent_harness::WorkflowRunProvider> = provider;
        registry.register_workflow_run(provider).expect("workflow.run 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.workflow".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["workflow.run".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions,
        };
        (peer, runtime)
    }

    /// `start` 逐字段解析入站（snake_case）并构造 camelCase DTO；owner 取连接身份。
    #[test]
    fn workflow_run_start_forwards_request_and_owner() {
        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer(provider);
        let response = with_watchdog(Duration::from_secs(5), "workflow.run start", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "workflow.run",
                    "op": "start",
                    "args": {
                        "template_id": "stock-analysis",
                        "input": { "codes": ["600519"] },
                        "max_concurrent": 3,
                        "dry_run": true,
                        "as_of_date": "2024-06-28",
                    },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "已注册且已授权时应成功，实际 {response:?}");
        let calls = captured.start_calls.lock().clone();
        assert_eq!(calls.len(), 1, "start 应恰被调用一次");
        let (owner, request) = &calls[0];
        assert_eq!(owner, "plugin.workflow", "owner 必须由连接身份注入，不得取自参数");
        assert_eq!(request.template_id, "stock-analysis", "`template_id` 应原样透传");
        assert_eq!(request.input, Some(json!({ "codes": ["600519"] })));
        assert_eq!(request.max_concurrent, Some(3));
        assert!(request.dry_run, "`dry_run` 应原样透传");
        assert_eq!(request.as_of_date.as_deref(), Some("2024-06-28"));
        assert!(request.variables.is_empty(), "`variables` 缺省必须为空 vec");
        assert_eq!(
            response.value,
            Some(json!({
                "executionId": "exec-1",
                "workflowId": "wf-1",
                "acceptedAt": 1_700_000_000_000_i64,
            }))
        );
    }

    /// 缺 `template_id` / 空白 `template_id` ⇒ 参数错误，且不触达 provider。
    #[test]
    fn workflow_run_start_requires_template_id() {
        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer(provider);
        let missing =
            with_watchdog(Duration::from_secs(2), "workflow.run start 缺参", move || {
                peer.handle_call_seam(
                    &json!({ "seam": "workflow.run", "op": "start", "args": {} }),
                    &[],
                )
            });
        assert_eq!(
            missing.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `template_id` 必须走参数错误，实际 {missing:?}"
        );

        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured_blank = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer(provider);
        let blank = with_watchdog(Duration::from_secs(2), "workflow.run start 空白", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "workflow.run",
                    "op": "start",
                    "args": { "template_id": "   " },
                }),
                &[],
            )
        });
        assert_eq!(
            blank.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "空白 `template_id` 必须走参数错误，实际 {blank:?}"
        );

        assert!(captured.start_calls.lock().is_empty(), "缺参不得触达 provider");
        assert!(captured_blank.start_calls.lock().is_empty(), "空白参数不得触达 provider");
    }

    /// 查无（`Ok(None)`）⇒ JSON `null`，**不是错误**；`include_output` 缺省 false 透传。
    ///
    /// peer 不带任何权限：`status` 是只读 op，不得被权限闸拦。
    #[test]
    fn workflow_run_status_missing_returns_null() {
        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer_with_permissions(provider, Vec::new());
        let response = with_watchdog(Duration::from_secs(5), "workflow.run status", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "workflow.run",
                    "op": "status",
                    "args": { "execution_id": "exec-404" },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "查无可不是错误，实际 {response:?}");
        assert_eq!(response.value, Some(Value::Null), "返回值应为 JSON null");
        assert_eq!(
            captured.status_calls.lock().clone(),
            vec![("plugin.workflow".to_string(), "exec-404".to_string(), false)],
            "`include_output` 缺省必须为 false 并原样透传"
        );
    }

    /// `cancel` 的三态结果原样透出（真取消 + 现值），owner 取连接身份。
    #[test]
    fn workflow_run_cancel_reports_outcome() {
        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer(provider);
        let response = with_watchdog(Duration::from_secs(5), "workflow.run cancel", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "workflow.run",
                    "op": "cancel",
                    "args": { "execution_id": "exec-1" },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "已授权时应成功，实际 {response:?}");
        assert_eq!(response.value, Some(json!({ "cancelled": true, "status": "cancelled" })));
        assert_eq!(
            captured.cancel_calls.lock().clone(),
            vec![("plugin.workflow".to_string(), "exec-1".to_string())]
        );
    }

    /// `limit` 夹在 1..=200：9999 ⇒ 200，缺省 ⇒ 50；`workflow_id` 缺省 ⇒ `None`。
    ///
    /// peer 不带任何权限：`list_active` 是只读 op，不得被权限闸拦。
    #[test]
    fn workflow_run_list_active_clamps_limit() {
        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer_with_permissions(provider, Vec::new());
        let response =
            with_watchdog(Duration::from_secs(5), "workflow.run list_active", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "workflow.run",
                        "op": "list_active",
                        "args": { "limit": 9999, "workflow_id": "wf-1" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        assert_eq!(value["total"], json!(1));
        assert_eq!(value["executions"][0]["ownerPluginId"], json!("plugin.workflow"));

        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured_default = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer_with_permissions(provider, Vec::new());
        let response =
            with_watchdog(Duration::from_secs(5), "workflow.run list_active 缺省", move || {
                peer.handle_call_seam(
                    &json!({ "seam": "workflow.run", "op": "list_active", "args": {} }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "省略可选参数应成功，实际 {response:?}");

        assert_eq!(
            captured.list_calls.lock().clone(),
            vec![("plugin.workflow".to_string(), Some("wf-1".to_string()), 200)],
            "limit=9999 必须被夹到 200"
        );
        assert_eq!(
            captured_default.list_calls.lock().clone(),
            vec![("plugin.workflow".to_string(), None, 50)],
            "`limit` 缺省必须为 50，`workflow_id` 缺省必须为 None"
        );
    }

    /// `list_recent` 与 `list_active` 的**同形**必须逐字一致，只多一处：页内可含终态行。
    ///
    /// 钉三件插件侧看不到、只能从线上验的事：
    /// 1. 夹档与缺省与 `list_active` 同档（9999⇒200、缺省⇒50 / None）；
    /// 2. 只读 ⇒ 不带任何权限也能过闸；
    /// 3. `finishedAt` 在在跑行**整体不出现**、在终态行是 camelCase 的数字 ——
    ///    插件按「键在不在」决定耗时列，出现一个 `null` 会被读成「0 毫秒」。
    #[test]
    fn workflow_run_list_recent_passes_terminal_rows() {
        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer_with_permissions(provider, Vec::new());
        let response =
            with_watchdog(Duration::from_secs(5), "workflow.run list_recent", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "workflow.run",
                        "op": "list_recent",
                        "args": { "limit": 9999, "workflow_id": "wf-1" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "只读 op 不得被权限闸拦，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        assert_eq!(value["total"], json!(2));
        assert_eq!(
            captured.recent_calls.lock().clone(),
            vec![("plugin.workflow".to_string(), Some("wf-1".to_string()), 200)],
            "limit=9999 必须与 list_active 同档夹到 200"
        );

        // 页内按 updated_at 降序 ⇒ [0] 是在跑的 exec-1，[1] 是已结束的 exec-0
        assert_eq!(value["executions"][0]["executionId"], json!("exec-1"));
        assert!(
            value["executions"][0].get("finishedAt").is_none(),
            "在跑行不得带结束时刻键：{}",
            value["executions"][0]
        );
        assert_eq!(value["executions"][1]["status"], json!("completed"));
        assert_eq!(value["executions"][1]["finishedAt"], json!(1_699_999_999_000_i64));
        assert!(
            value["executions"][1].get("finished_at").is_none(),
            "不得残留 snake_case 键：{}",
            value["executions"][1]
        );
    }

    /// `list_recent` 的缺省参数与 `list_active` 同档（50 / None）—— 两条分支各夹一份必然漂移。
    #[test]
    fn workflow_run_list_recent_defaults_match_list_active() {
        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer_with_permissions(provider, Vec::new());
        let response =
            with_watchdog(Duration::from_secs(5), "workflow.run list_recent 缺省", move || {
                peer.handle_call_seam(
                    &json!({ "seam": "workflow.run", "op": "list_recent", "args": {} }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "省略可选参数应成功，实际 {response:?}");
        assert_eq!(
            captured.recent_calls.lock().clone(),
            vec![("plugin.workflow".to_string(), None, 50)],
            "`limit` 缺省必须为 50，`workflow_id` 缺省必须为 None"
        );
    }

    /// 无 `write` 权限 ⇒ `start` / `cancel` 在分派前被拒，且不触达 provider。
    #[test]
    fn workflow_run_start_requires_write_permission() {
        let provider = Arc::new(FixedWorkflowRunProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = workflow_run_peer_with_permissions(provider, Vec::new());
        let (start, cancel) =
            with_watchdog(Duration::from_secs(5), "workflow.run 无权限", move || {
                let start = peer.handle_call_seam(
                    &json!({
                        "seam": "workflow.run",
                        "op": "start",
                        "args": { "template_id": "stock-analysis" },
                    }),
                    &[],
                );
                let cancel = peer.handle_call_seam(
                    &json!({
                        "seam": "workflow.run",
                        "op": "cancel",
                        "args": { "execution_id": "exec-1" },
                    }),
                    &[],
                );
                (start, cancel)
            });
        for response in [&start, &cancel] {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::PLUGIN_PERMISSION_DENIED),
                "无 write 权限必须被拒，实际 {response:?}"
            );
            let detail = response.message.clone().unwrap_or_default();
            assert!(detail.contains("write"), "文案应点明缺失的权限，实际：{detail}");
            assert!(detail.contains("workflow.run"), "文案应点明接缝，实际：{detail}");
        }
        assert!(captured.start_calls.lock().is_empty(), "被拒的 start 不得触达 provider");
        assert!(captured.cancel_calls.lock().is_empty(), "被拒的 cancel 不得触达 provider");
    }

    /// 宿主拒绝受理（`AxAgentError::Validation`）⇒ `WORKFLOW_RUN_REJECTED`，并保留原因。
    #[test]
    fn workflow_run_start_maps_validation_to_rejected() {
        let (peer, _runtime) = workflow_run_peer(Arc::new(FixedWorkflowRunProvider::rejecting(
            "模板 `stock-analysis` 不在白名单",
        )));
        let response =
            with_watchdog(Duration::from_secs(5), "workflow.run start 被拒", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "workflow.run",
                        "op": "start",
                        "args": { "template_id": "stock-analysis" },
                    }),
                    &[],
                )
            });
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::WORKFLOW_RUN_REJECTED),
            "宿主拒绝受理必须映射为 WORKFLOW_RUN_REJECTED，实际 {response:?}"
        );
        assert_eq!(
            response.message.as_deref(),
            Some("模板 `stock-analysis` 不在白名单"),
            "拒绝原因应原样透出"
        );
    }

    // ───── trading.engine 接缝（读：get_positions / get_trades / validate_trade /
    //        compare_trade / trading_enabled；写：execute_trade） ─────

    /// 假交易引擎提供者：记录各方法收到的入参，返回固定 / 可控结果。
    ///
    /// 返回值的选取刻意覆盖契约的三种「非错误」形态：`validate` 回 `valid = true`、
    /// `execute` 回 `accepted = false`（业务拒绝）、`compare` 回 `Ok(None)`（查无）。
    struct FixedTradingEngineProvider {
        /// `get_positions` 收到的 `as_of` 序列（保留次数与顺序）。
        positions_as_of: Mutex<Vec<Option<axagent_harness::AsOfContext>>>,
        /// `get_trades` 收到的 `(stock_code, limit)` 序列。
        trades_calls: Mutex<Vec<(Option<String>, u32)>>,
        /// `validate_trade` 收到的 `(request, as_of)` 序列。
        validate_calls: Mutex<
            Vec<(axagent_harness::TradeValidationRequest, Option<axagent_harness::AsOfContext>)>,
        >,
        /// `execute_trade` 收到的 `(request, as_of)` 序列。
        execute_calls: Mutex<
            Vec<(axagent_harness::TradeExecutionRequest, Option<axagent_harness::AsOfContext>)>,
        >,
        /// `compare_trade` 收到的 `trade_id` 序列。
        compare_calls: Mutex<Vec<String>>,
        /// `trading_enabled` 的调用次数（该方法无参数，计数即取证）。
        enabled_calls: Mutex<usize>,
    }

    impl FixedTradingEngineProvider {
        fn new() -> Self {
            Self {
                positions_as_of: Mutex::new(Vec::new()),
                trades_calls: Mutex::new(Vec::new()),
                validate_calls: Mutex::new(Vec::new()),
                execute_calls: Mutex::new(Vec::new()),
                compare_calls: Mutex::new(Vec::new()),
                enabled_calls: Mutex::new(0),
            }
        }

        /// 契约里 `get_positions` 的固定持仓视图（字段值供断言整体比对）。
        fn sample_position() -> axagent_harness::PositionSummary {
            axagent_harness::PositionSummary {
                stock_code: "600519".to_string(),
                stock_name: "贵州茅台".to_string(),
                total_shares: 100,
                avg_cost: 1_480.0,
                current_price: Some(1_500.0),
                market_value: Some(150_000.0),
                unrealized_pnl: Some(2_000.0),
                unrealized_pnl_pct: Some(1.35),
                total_realized_pnl: 500.0,
                sector_name: Some("白酒".to_string()),
            }
        }

        /// 契约里 `get_trades` 的固定交易视图（与 harness 测试样本同字段值）。
        fn sample_trade_view() -> axagent_harness::TradeView {
            axagent_harness::TradeView {
                id: "trade-1".to_string(),
                stock_code: "600519".to_string(),
                stock_name: "贵州茅台".to_string(),
                direction: "buy".to_string(),
                price: 1_500.0,
                quantity: 100,
                trade_date: "2026-11-02".to_string(),
                trade_time: "14:30".to_string(),
                fee: None,
                realized_pnl: None,
                strategy: None,
                notes: Some("测试".to_string()),
                created_at: 1_700_000_000_000,
            }
        }
    }

    #[async_trait]
    impl axagent_harness::TradingEngineProvider for FixedTradingEngineProvider {
        async fn get_positions(
            &self,
            as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::PositionSummary>> {
            self.positions_as_of.lock().push(as_of);
            Ok(vec![Self::sample_position()])
        }

        async fn get_trades(
            &self,
            stock_code: Option<&str>,
            limit: u32,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::TradeView>> {
            self.trades_calls.lock().push((stock_code.map(str::to_string), limit));
            Ok(vec![Self::sample_trade_view()])
        }

        async fn validate_trade(
            &self,
            request: axagent_harness::TradeValidationRequest,
            as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<axagent_harness::TradeValidation> {
            self.validate_calls.lock().push((request, as_of));
            Ok(axagent_harness::TradeValidation {
                valid: true,
                errors: Vec::new(),
                warnings: Vec::new(),
            })
        }

        async fn execute_trade(
            &self,
            request: axagent_harness::TradeExecutionRequest,
            as_of: Option<axagent_harness::AsOfContext>,
        ) -> axagent_harness::core_error::Result<axagent_harness::TradeExecutionOutcome> {
            self.execute_calls.lock().push((request, as_of));
            // 固定回「校验不过被拒」：分派层必须把它当**正常结果**透出，不得转错误帧。
            Ok(axagent_harness::TradeExecutionOutcome {
                accepted: false,
                reason: Some("validation_failed".to_string()),
                trade: None,
                errors: vec!["入场价偏离分析目标价超过 5%".to_string()],
                warnings: vec!["当前非交易日".to_string()],
            })
        }

        async fn compare_trade(
            &self,
            trade_id: &str,
        ) -> axagent_harness::core_error::Result<Option<axagent_harness::TradePredictionComparison>>
        {
            self.compare_calls.lock().push(trade_id.to_string());
            // 固定回 `None`：分派层必须翻成 JSON `null` 而非错误。
            Ok(None)
        }

        async fn trading_enabled(&self) -> axagent_harness::core_error::Result<bool> {
            *self.enabled_calls.lock() += 1;
            Ok(true)
        }
    }

    /// 装假 trading.engine 提供者的 peer + 保活到用例结束的 runtime。
    ///
    /// 缺省带 `write` 权限（`execute_trade` 的准入需要）；只读 op 用例请传空权限表，
    /// 以证明它们不设闸。
    fn trading_peer_with_permissions(
        provider: Arc<FixedTradingEngineProvider>,
        permissions: Vec<PluginPermission>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        let provider: Arc<dyn axagent_harness::TradingEngineProvider> = provider;
        registry.register_trading_engine(provider).expect("trading.engine 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.trading".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["trading.engine".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions,
        };
        (peer, runtime)
    }

    fn trading_peer(
        provider: Arc<FixedTradingEngineProvider>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        trading_peer_with_permissions(provider, vec![PluginPermission::Write])
    }

    /// `as_of`（字符串形态）必须解析成 `AsOfContext` 透传；省略 ⇒ provider 收到 `None`。
    ///
    /// peer 不带任何权限：`get_positions` 是只读 op，不得被权限闸拦。
    #[test]
    fn trading_engine_get_positions_forwards_as_of() {
        let provider = Arc::new(FixedTradingEngineProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = trading_peer_with_permissions(provider, Vec::new());
        let (with_date, without_date) =
            with_watchdog(Duration::from_secs(5), "trading.engine get_positions", move || {
                let with_date = peer.handle_call_seam(
                    &json!({
                        "seam": "trading.engine",
                        "op": "get_positions",
                        "args": { "as_of": "2026-10-30" },
                    }),
                    &[],
                );
                let without_date = peer.handle_call_seam(
                    &json!({ "seam": "trading.engine", "op": "get_positions", "args": {} }),
                    &[],
                );
                (with_date, without_date)
            });
        assert_eq!(with_date.code, None, "带 as_of 应成功，实际 {with_date:?}");
        let value = with_date.value.expect("成功响应必须带 value");
        assert_eq!(
            value[0]["stockCode"],
            json!("600519"),
            "持仓视图必须按 DTO 的 camelCase 线上键透出"
        );
        assert_eq!(without_date.code, None, "省略 as_of 应成功，实际 {without_date:?}");

        let seen = captured.positions_as_of.lock().clone();
        assert_eq!(seen.len(), 2, "两次调用都应触达 provider");
        assert_eq!(
            seen[0].as_ref().map(axagent_harness::AsOfContext::as_string).as_deref(),
            Some("2026-10-30"),
            "`as_of` 字符串应解析成同日期 AsOfContext 透传"
        );
        assert_eq!(seen[1], None, "省略 `as_of` 时 provider 必须收到 None（实盘口径）");
    }

    /// `{}` ⇒ provider 收到 `(None, 50)`：`limit` 缺省 50 由分派层填充，且**不夹上限**。
    #[test]
    fn trading_engine_get_trades_defaults_limit() {
        let provider = Arc::new(FixedTradingEngineProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = trading_peer_with_permissions(provider, Vec::new());
        let response =
            with_watchdog(Duration::from_secs(5), "trading.engine get_trades 缺省", move || {
                peer.handle_call_seam(
                    &json!({ "seam": "trading.engine", "op": "get_trades", "args": {} }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "省略可选参数应成功，实际 {response:?}");
        assert_eq!(
            captured.trades_calls.lock().clone(),
            vec![(None, 50)],
            "`stock_code` 缺省必须为 None、`limit` 缺省必须为 50"
        );
        let value = response.value.expect("成功响应必须带 value");
        assert_eq!(value[0]["tradeDate"], json!("2026-11-02"), "视图应按 camelCase 透出");
    }

    /// 嵌套 `request` 是 camelCase 对象，分派层**原样反序列化**（不做键名映射）：
    /// `priceDeviationLimit` ⇒ `price_deviation_limit = Some(5.0)`。缺 `request` ⇒ 参数错误且零调用。
    #[test]
    fn trading_engine_validate_forwards_camel_request() {
        let provider = Arc::new(FixedTradingEngineProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = trading_peer(provider);
        let response =
            with_watchdog(Duration::from_secs(5), "trading.engine validate", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "trading.engine",
                        "op": "validate_trade",
                        "args": {
                            "request": {
                                "stockCode": "600519",
                                "direction": "buy",
                                "quantity": 100,
                                "price": 1500.0,
                                "priceDeviationLimit": 5.0,
                            },
                        },
                    }),
                    &[],
                )
            });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        assert_eq!(response.value, Some(json!({ "valid": true, "errors": [], "warnings": [] })));
        let calls = captured.validate_calls.lock().clone();
        assert_eq!(calls.len(), 1, "validate_trade 应恰被调用一次");
        let (request, as_of) = &calls[0];
        assert_eq!(request.stock_code, "600519", "camelCase `stockCode` 应落进 snake_case 字段");
        assert_eq!(request.price_deviation_limit, Some(5.0), "`priceDeviationLimit` 应原样透传");
        assert_eq!(*as_of, None, "省略 `as_of` 时 provider 必须收到 None");

        // 缺必填 `request` ⇒ 参数错误，且不触达 provider（另一个全新替身取证零调用）。
        let provider = Arc::new(FixedTradingEngineProvider::new());
        let captured_missing = Arc::clone(&provider);
        let (peer, _runtime) = trading_peer(provider);
        let missing =
            with_watchdog(Duration::from_secs(2), "trading.engine validate 缺参", move || {
                peer.handle_call_seam(
                    &json!({ "seam": "trading.engine", "op": "validate_trade", "args": {} }),
                    &[],
                )
            });
        assert_eq!(
            missing.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `request` 必须走参数错误，实际 {missing:?}"
        );
        assert!(captured_missing.validate_calls.lock().is_empty(), "缺参不得触达 provider");
    }

    /// `accepted = false` 的业务拒绝是**正常结果**：响应为成功帧（`code = None`），
    /// outcome 原样透出，不借用错误码。
    #[test]
    fn trading_engine_execute_reports_outcome_not_error() {
        let provider = Arc::new(FixedTradingEngineProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = trading_peer(provider);
        let response = with_watchdog(Duration::from_secs(5), "trading.engine execute", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "trading.engine",
                    "op": "execute_trade",
                    "args": {
                        "request": {
                            "stockCode": "600519",
                            "stockName": "贵州茅台",
                            "direction": "buy",
                            "price": 1500.0,
                            "quantity": 100,
                            "tradeDate": "2026-11-02",
                            "tradeTime": "14:30",
                        },
                    },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "业务拒绝不是错误，实际 {response:?}");
        let value = response.value.expect("成功响应必须带 value");
        assert_eq!(value["accepted"], json!(false));
        assert_eq!(value["reason"], json!("validation_failed"));
        assert_eq!(value["trade"], json!(null));
        assert_eq!(value["errors"].as_array().map(Vec::len), Some(1));
        assert_eq!(value["warnings"].as_array().map(Vec::len), Some(1));
        assert_eq!(captured.execute_calls.lock().len(), 1, "已授权应触达 provider 恰一次");
    }

    /// 查无（`Ok(None)`）⇒ JSON `null`，**不是错误**（与 `jobs.store get` 同口径）。
    #[test]
    fn trading_engine_compare_missing_is_null() {
        let provider = Arc::new(FixedTradingEngineProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = trading_peer_with_permissions(provider, Vec::new());
        let response = with_watchdog(Duration::from_secs(5), "trading.engine compare", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "trading.engine",
                    "op": "compare_trade",
                    "args": { "trade_id": "t-404" },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "查无不是错误，实际 {response:?}");
        assert_eq!(response.value, Some(Value::Null), "返回值应为 JSON null");
        assert_eq!(
            captured.compare_calls.lock().clone(),
            vec!["t-404".to_string()],
            "`trade_id` 应原样透传"
        );
    }

    /// `trading_enabled` 返回**具名对象** `{ "enabled": bool }`，不是裸布尔。
    #[test]
    fn trading_engine_enabled_is_named_object() {
        let provider = Arc::new(FixedTradingEngineProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = trading_peer_with_permissions(provider, Vec::new());
        let response = with_watchdog(Duration::from_secs(5), "trading.engine enabled", move || {
            peer.handle_call_seam(
                &json!({ "seam": "trading.engine", "op": "trading_enabled", "args": {} }),
                &[],
            )
        });
        assert_eq!(response.code, None, "已注册时应成功，实际 {response:?}");
        assert_eq!(response.value, Some(json!({ "enabled": true })));
        assert_eq!(*captured.enabled_calls.lock(), 1, "trading_enabled 应恰被调用一次");
    }

    /// 无 `write` 权限 ⇒ `execute_trade` 在分派前被拒（`PLUGIN_PERMISSION_DENIED`），
    /// 且不触达 provider；其余五个只读 op 不受影响（权限表只登记这一个写 op）。
    #[test]
    fn trading_engine_execute_requires_write() {
        let provider = Arc::new(FixedTradingEngineProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = trading_peer_with_permissions(provider, Vec::new());
        let response =
            with_watchdog(Duration::from_secs(5), "trading.engine execute 无权限", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "trading.engine",
                        "op": "execute_trade",
                        "args": {
                            "request": {
                                "stockCode": "600519",
                                "stockName": "贵州茅台",
                                "direction": "buy",
                                "price": 1500.0,
                                "quantity": 100,
                                "tradeDate": "2026-11-02",
                                "tradeTime": "14:30",
                            },
                        },
                    }),
                    &[],
                )
            });
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 权限必须被拒，实际 {response:?}"
        );
        let detail = response.message.clone().unwrap_or_default();
        assert!(detail.contains("write"), "文案应点明缺失的权限，实际：{detail}");
        assert!(detail.contains("trading.engine"), "文案应点明接缝，实际：{detail}");
        assert!(captured.execute_calls.lock().is_empty(), "被拒的 execute 不得触达 provider");

        // 权限表枚举锁：trading.engine 只有 `execute_trade` 一个写 op，其余五个不设闸。
        let writes = required_permission_for("trading.engine", "execute_trade");
        assert_eq!(writes, Some(PluginPermission::Write), "execute_trade 必须要求 write");
        for read_op in
            ["get_positions", "get_trades", "validate_trade", "compare_trade", "trading_enabled"]
        {
            assert_eq!(
                required_permission_for("trading.engine", read_op),
                None,
                "只读 op `{read_op}` 不得设权限闸"
            );
        }
    }

    /// 假 `stock.analysis` 提供者：三个 op 都记录收到的**原始 payload**（`Value` 原样），
    /// 让「整块透传 / 被拒时零触达」两类判据都能取证。
    ///
    /// 返回的是最小合法形状，形状细节归 `analysis-engine` 的 DTO —— 本层只验分派。
    struct FixedStockAnalysisProvider {
        upsert_calls: Mutex<Vec<Value>>,
        industry_calls: Mutex<Vec<Value>>,
        factor_calls: Mutex<Vec<Value>>,
        /// 批 1a：`list_pool_scan_crons`（读 op）收到的载荷序列 —— 读 op 必须能穿过权限闸。
        cron_list_calls: Mutex<Vec<Value>>,
    }

    impl FixedStockAnalysisProvider {
        fn new() -> Self {
            Self {
                upsert_calls: Mutex::new(Vec::new()),
                industry_calls: Mutex::new(Vec::new()),
                factor_calls: Mutex::new(Vec::new()),
                cron_list_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl axagent_harness::StockAnalysisProvider for FixedStockAnalysisProvider {
        async fn industry_constituents(
            &self,
            input: Value,
        ) -> axagent_harness::core_error::Result<Value> {
            self.industry_calls.lock().push(input);
            Ok(json!({ "groups": [], "universeTotal": 0, "universeWithIndustry": 0 }))
        }

        async fn batch_upsert_mainlines(
            &self,
            input: Value,
        ) -> axagent_harness::core_error::Result<Value> {
            self.upsert_calls.lock().push(input);
            Ok(json!({ "inserted": 0, "updated": 0, "archived": 0 }))
        }

        async fn stock_factor(&self, input: Value) -> axagent_harness::core_error::Result<Value> {
            self.factor_calls.lock().push(input);
            Ok(json!({
                "rows": [],
                "requested": 0,
                "scored": 0,
                "missing": [],
                "cutoffDate": "",
                "windowRows": 0,
            }))
        }

        /// 批 1a：只覆盖一个**读** op。其余 cron op 吃默认实现（显式报未实现），
        /// 正反向用例因此不需要为九条写 op 各写一个替身。
        async fn list_pool_scan_crons(
            &self,
            input: Value,
        ) -> axagent_harness::core_error::Result<Value> {
            self.cron_list_calls.lock().push(input);
            Ok(json!([]))
        }
    }

    // ───── stock.monitor 接缝（9 op）+ quote.watcher 接缝（3 op）─────

    /// 假股票监控提供者：**刻意只实现部分 op**，其余吃 trait 默认实现
    /// （`Err(Provider("…未实现…"))` ⇒ 分派落 `PLUGIN_CALL_FAILED`）。契约明说外部实现
    /// 「只覆盖关心的 op」，下面的用例顺带把这条链路也钉住。
    struct FixedMonitorProvider {
        /// `status` 调用次数（该方法无参数，计数即取证）。
        status_calls: Mutex<usize>,
        /// `set_t0_config` 收到的配置序列。
        t0_calls: Mutex<Vec<axagent_harness::TZeroConfig>>,
        /// `upsert_alert` 收到的请求序列。
        upsert_calls: Mutex<Vec<axagent_harness::AlertUpsertRequest>>,
        /// `remove_alert` 收到的 alert_id 序列。
        remove_calls: Mutex<Vec<String>>,
    }

    impl FixedMonitorProvider {
        fn new() -> Self {
            Self {
                status_calls: Mutex::new(0),
                t0_calls: Mutex::new(Vec::new()),
                upsert_calls: Mutex::new(Vec::new()),
                remove_calls: Mutex::new(Vec::new()),
            }
        }

        /// 测试用运行读数：计数取非零值，便于验证 camelCase 线上键原样透出。
        fn sample_status() -> axagent_harness::MonitorStatusView {
            axagent_harness::MonitorStatusView {
                assembled: true,
                session: "intraday".to_string(),
                trading_now: true,
                poll_interval_secs: 30,
                configs_active: 2,
                round_codes_total: 2,
                round_codes_failed: 1,
                consecutive_all_failed_rounds: 0,
                round_finished_at_ms: 1_700_000_000_000,
            }
        }

        /// `price_alerts` 行的 12 列 camelCase 投影（与 harness 测试样本同字段风格）。
        fn sample_alert_view() -> axagent_harness::PriceAlertView {
            axagent_harness::PriceAlertView {
                id: "alert-uuid-1".to_string(),
                stock_code: "600519".to_string(),
                stock_name: "贵州茅台".to_string(),
                condition: "below".to_string(),
                target_price: 1_400.0,
                alert_type: Some("stop_loss".to_string()),
                condition_type: Some("price".to_string()),
                threshold: Some(1_400.0),
                is_triggered: 0,
                triggered_at: None,
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_001,
            }
        }
    }

    #[async_trait]
    impl axagent_harness::StockMonitorProvider for FixedMonitorProvider {
        async fn status(
            &self,
        ) -> axagent_harness::core_error::Result<axagent_harness::MonitorStatusView> {
            *self.status_calls.lock() += 1;
            Ok(Self::sample_status())
        }

        async fn set_t0_config(
            &self,
            config: axagent_harness::TZeroConfig,
        ) -> axagent_harness::core_error::Result<()> {
            self.t0_calls.lock().push(config);
            Ok(())
        }

        async fn upsert_alert(
            &self,
            request: axagent_harness::AlertUpsertRequest,
        ) -> axagent_harness::core_error::Result<axagent_harness::AlertUpsertOutcome> {
            self.upsert_calls.lock().push(request);
            // `wired = false` 刻意回 false：分派层必须原样透出「库里有行但不会被盯」，
            // 不得当成功抹平、也不得转成错误帧。
            Ok(axagent_harness::AlertUpsertOutcome {
                alert: Self::sample_alert_view(),
                reused: true,
                wired: false,
            })
        }

        async fn remove_alert(
            &self,
            alert_id: &str,
        ) -> axagent_harness::core_error::Result<axagent_harness::RemoveAlertOutcome> {
            self.remove_calls.lock().push(alert_id.to_string());
            // 两个计数域并列（摘内存失败但删库成功，也是合法结果）。
            Ok(axagent_harness::RemoveAlertOutcome { db_deleted: true, memory_removed: false })
        }
        // `list_configs` / `t0_config` / `aggregator_config` / `set_aggregator_config` /
        // `aggregator_buffer` 不覆盖 ⇒ 吃 trait 默认 `Err(Provider)`。
    }

    /// 假推送集合提供者：只实现 `watched_codes` / `watch`，`unwatch` 吃 trait 默认实现
    /// （由 `watcher_unwatch_default_is_call_failed` 取证）。
    struct FixedQuoteWatcher {
        /// `watched_codes` 调用次数。
        list_calls: Mutex<usize>,
        /// `watch` 收到的 `(codes, priority, replace)` 序列。
        watch_calls: Mutex<Vec<(Vec<String>, axagent_harness::WatchPriority, bool)>>,
    }

    impl FixedQuoteWatcher {
        fn new() -> Self {
            Self { list_calls: Mutex::new(0), watch_calls: Mutex::new(Vec::new()) }
        }
    }

    #[async_trait]
    impl axagent_harness::QuoteWatcherProvider for FixedQuoteWatcher {
        async fn watched_codes(&self) -> axagent_harness::core_error::Result<Vec<String>> {
            *self.list_calls.lock() += 1;
            // 契约 J6：provider 侧 sort ⇒ 替身给升序结果。
            Ok(vec!["000001".to_string(), "600519".to_string()])
        }

        async fn watch(
            &self,
            codes: &[String],
            priority: axagent_harness::WatchPriority,
            replace: bool,
        ) -> axagent_harness::core_error::Result<axagent_harness::WatchOutcome> {
            self.watch_calls.lock().push((codes.to_vec(), priority, replace));
            // 模拟数量闸截断：固定只应用 2 只 ⇒ `applied < requested` 是合法结果，
            // 分派层必须原样透出两个数、不得报错。
            Ok(axagent_harness::WatchOutcome { requested: codes.len() as u32, applied: 2 })
        }
        // `unwatch` 不覆盖 ⇒ 吃 trait 默认 `Err(Provider)`。
    }

    /// 装假 stock.monitor + quote.watcher 提供者的 peer + 保活到用例结束的 runtime。
    ///
    /// 缺省带 `write` 权限（写 op 准入需要）；只读 op 用例传空权限表，以证明它们不设闸。
    fn monitor_peer_with_permissions(
        monitor: Arc<FixedMonitorProvider>,
        watcher: Arc<FixedQuoteWatcher>,
        permissions: Vec<PluginPermission>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        let provider: Arc<dyn axagent_harness::StockMonitorProvider> = monitor;
        registry.register_stock_monitor(provider).expect("stock.monitor 接缝应注册成功");
        let watcher: Arc<dyn axagent_harness::QuoteWatcherProvider> = watcher;
        registry.register_quote_watcher(watcher).expect("quote.watcher 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.monitor".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["stock.monitor".to_string(), "quote.watcher".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions,
        };
        (peer, runtime)
    }

    /// 无 `write` 权限 ⇒ 六条写 op 全部在取参 / 取 provider 之前被拒
    /// （`PLUGIN_PERMISSION_DENIED`、零副作用）；权限表枚举锁同时钉住「6 写 + 6 读」的口径。
    #[test]
    fn monitor_watcher_write_ops_require_write() {
        let monitor = Arc::new(FixedMonitorProvider::new());
        let watcher = Arc::new(FixedQuoteWatcher::new());
        let captured_monitor = Arc::clone(&monitor);
        let captured_watcher = Arc::clone(&watcher);
        let (peer, _runtime) = monitor_peer_with_permissions(monitor, watcher, Vec::new());

        let (denied, denied_watch) =
            with_watchdog(Duration::from_secs(5), "写 op 无权限", move || {
                let denied = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "upsert_alert",
                        "args": {
                            "request": {
                                "stockCode": "600519",
                                "stockName": "贵州茅台",
                                "alertType": "stop_loss",
                                "threshold": 1400.0,
                            },
                        },
                    }),
                    &[],
                );
                let denied_watch = peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "watch",
                        "args": { "codes": ["600519"], "priority": "active" },
                    }),
                    &[],
                );
                (denied, denied_watch)
            });
        assert_eq!(
            denied.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 权限的 upsert_alert 必须被拒，实际 {denied:?}"
        );
        let detail = denied.message.clone().unwrap_or_default();
        assert!(detail.contains("write"), "文案应点明缺失的权限，实际：{detail}");
        assert!(detail.contains("stock.monitor"), "文案应点明接缝，实际：{detail}");
        assert_eq!(
            denied_watch.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 权限的 watch 必须被拒，实际 {denied_watch:?}"
        );
        assert!(captured_monitor.upsert_calls.lock().is_empty(), "被拒的写 op 不得触达 provider");
        assert!(captured_watcher.watch_calls.lock().is_empty(), "被拒的 watch 不得触达 provider");

        // 权限表枚举锁：两条接缝共 6 个写 op 要求 write，6 个只读 op 一律不设闸。
        for (seam, write_op) in [
            ("stock.monitor", "set_t0_config"),
            ("stock.monitor", "upsert_alert"),
            ("stock.monitor", "remove_alert"),
            ("stock.monitor", "set_aggregator_config"),
            ("quote.watcher", "watch"),
            ("quote.watcher", "unwatch"),
        ] {
            assert_eq!(
                required_permission_for(seam, write_op),
                Some(PluginPermission::Write),
                "写 op `{seam}`/`{write_op}` 必须要求 write"
            );
        }
        for (seam, read_op) in [
            ("stock.monitor", "status"),
            ("stock.monitor", "list_configs"),
            ("stock.monitor", "get_t0_config"),
            ("stock.monitor", "aggregator_config"),
            ("stock.monitor", "aggregator_buffer"),
            ("quote.watcher", "list_watched"),
        ] {
            assert_eq!(
                required_permission_for(seam, read_op),
                None,
                "只读 op `{seam}`/`{read_op}` 不得设权限闸"
            );
        }
    }

    // ─────────────── #240 E 批：`stock.analysis` 三 op 的权限闸（正反向锁） ───────────────

    /// 装假 `stock.analysis` 提供者的 peer + 保活到用例结束的 runtime。
    fn stock_analysis_peer_with_permissions(
        provider: Arc<FixedStockAnalysisProvider>,
        permissions: Vec<PluginPermission>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        let provider: Arc<dyn axagent_harness::StockAnalysisProvider> = provider;
        registry.register_stock_analysis(provider).expect("stock.analysis 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.stock".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["stock.analysis".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions,
        };
        (peer, runtime)
    }

    /// 权限表枚举锁（**反向**）：三个 op 里只有 `batch_upsert_mainlines` 要 `write`，
    /// 两个读 op 一条闸都不许加 —— 给读缝加闸会把只读插件无端拒掉。
    ///
    /// 这条锁的存在理由是 `required_permission_for` 的注释（`worker.rs:474-480`）明文承诺
    /// 「正反向锁在 `stock_analysis_*` 用例」：注释承诺的锁必须真有，否则就是注释假话。
    #[test]
    fn stock_analysis_write_op_requires_write_and_read_ops_require_nothing() {
        let verdict = with_watchdog(Duration::from_secs(2), "stock.analysis 权限表", || {
            (
                required_permission_for("stock.analysis", "batch_upsert_mainlines"),
                required_permission_for("stock.analysis", "industry_constituents"),
                required_permission_for("stock.analysis", "stock_factor"),
            )
        });
        assert_eq!(verdict.0, Some(PluginPermission::Write), "唯一写 op 必须要求 write");
        assert_eq!(verdict.1, None, "读 op `industry_constituents` 不得设权限闸");
        assert_eq!(verdict.2, None, "读 op `stock_factor` 不得设权限闸");
        // 接缝维度参与判定：同名 op 换接缝不得再命中本条。
        assert_eq!(required_permission_for("opc.store", "batch_upsert_mainlines"), None);
    }

    /// 批 1a（#35）十二 cron op 的权限档：3 create + 3 toggle + 3 delete = `Write`，
    /// 3 个 list 一条闸都不许加（同上面「给读缝加闸会把只读插件无端拒掉」的判据）。
    ///
    /// 九条写 op 在权限表里写在同一个 `|` 多分支里 —— 漏一条不会有编译错误，
    /// 只会让那条命令对只读插件永久开放，因此逐条枚举而不是只抽查。
    #[test]
    fn stock_analysis_cron_ops_permissions_follow_read_write_split() {
        let writes = [
            "create_recommendation_cron",
            "toggle_recommendation_cron",
            "delete_recommendation_cron",
            "create_trend_screening_cron",
            "toggle_trend_screening_cron",
            "delete_trend_screening_cron",
            "create_pool_scan_cron",
            "toggle_pool_scan_cron",
            "delete_pool_scan_cron",
        ];
        let reads =
            ["list_recommendation_crons", "list_trend_screening_crons", "list_pool_scan_crons"];
        for op in writes {
            assert_eq!(
                required_permission_for("stock.analysis", op),
                Some(PluginPermission::Write),
                "写 op `{op}` 必须要求 write"
            );
            // 接缝维度同样参与判定：同名 op 换接缝不得命中。
            assert_eq!(
                required_permission_for("opc.store", op),
                None,
                "`{op}` 只属于 stock.analysis"
            );
        }
        for op in reads {
            assert_eq!(
                required_permission_for("stock.analysis", op),
                None,
                "读 op `{op}` 不得设权限闸"
            );
        }
    }

    /// 正向（批 1a）：同一个无 `write` 权限的 peer 上，写 cron op 被拒且**零触达**，
    /// 读 cron op 照常穿闸并拿到 provider 的返回值。
    #[test]
    fn stock_analysis_cron_write_denied_without_permission_while_list_passes() {
        let provider = Arc::new(FixedStockAnalysisProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = stock_analysis_peer_with_permissions(provider, Vec::new());

        let (denied, listed) =
            with_watchdog(Duration::from_secs(2), "stock.analysis cron 权限闸", move || {
                let denied = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.analysis",
                        "op": "delete_pool_scan_cron",
                        "args": { "request": { "id": "pool-1" } },
                    }),
                    &[],
                );
                let listed = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.analysis",
                        "op": "list_pool_scan_crons",
                        "args": { "request": {} },
                    }),
                    &[],
                );
                (denied, listed)
            });

        assert_eq!(
            denied.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 的删除命令必须被拒，实际 {denied:?}"
        );
        assert_eq!(
            listed.kind,
            axagent_plugin_proto::ResponseKind::Success,
            "list 读 op 必须放行（无 write 也要能读），实际 {listed:?}"
        );
        assert_eq!(listed.value, Some(json!([])), "读 op 的返回值应原样回给插件");
        assert_eq!(captured.cron_list_calls.lock().len(), 1, "读 op 应触达 provider 一次");
        assert_eq!(captured.cron_list_calls.lock()[0], json!({}), "载荷应原样透传");
    }

    /// 正向：无 `write` 权限 ⇒ 写 op 在取 provider 之前被拒（零触达）；
    /// 同一 peer 的两个读 op 必须**照常放行**（证明闸只挂在写 op 上）。
    #[test]
    fn stock_analysis_write_denied_without_permission_while_reads_pass() {
        let provider = Arc::new(FixedStockAnalysisProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) = stock_analysis_peer_with_permissions(provider, Vec::new());

        let (denied, industry, factor) =
            with_watchdog(Duration::from_secs(2), "stock.analysis 权限闸", move || {
                let denied = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.analysis",
                        "op": "batch_upsert_mainlines",
                        "args": { "request": { "mainline_date": "2026-11-09", "mainlines": [] } },
                    }),
                    &[],
                );
                let industry = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.analysis",
                        "op": "industry_constituents",
                        "args": { "request": { "industries": ["光模块"] } },
                    }),
                    &[],
                );
                let factor = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.analysis",
                        "op": "stock_factor",
                        "args": { "request": { "stock_codes": ["600519"] } },
                    }),
                    &[],
                );
                (denied, industry, factor)
            });
        assert_eq!(
            denied.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 的 batch_upsert_mainlines 必须被拒，实际 {denied:?}"
        );
        let detail = denied.message.clone().unwrap_or_default();
        assert!(detail.contains("write"), "文案应点明缺失的权限，实际：{detail}");
        assert!(detail.contains("stock.analysis"), "文案应点明接缝，实际：{detail}");
        assert!(captured.upsert_calls.lock().is_empty(), "被拒的写 op 不得触达 provider");
        assert_eq!(
            industry.kind,
            axagent_plugin_proto::ResponseKind::Success,
            "读 op 必须放行（无 write 权限也要能读），实际 {industry:?}"
        );
        assert_eq!(
            factor.kind,
            axagent_plugin_proto::ResponseKind::Success,
            "读 op 必须放行（无 write 权限也要能读），实际 {factor:?}"
        );
        assert_eq!(captured.industry_calls.lock().len(), 1, "读 op 应触达 provider 一次");
        assert_eq!(captured.factor_calls.lock().len(), 1, "读 op 应触达 provider 一次");
    }

    /// 分派形态：入参**整块**取 `args["request"]` 原样交给 provider（不改名、不裁剪）。
    ///
    /// 形状权威在 `analysis-engine` 的 DTO；分派层若自己「整理」一次就是第二套形状。
    #[test]
    fn stock_analysis_request_is_passed_through_verbatim() {
        let provider = Arc::new(FixedStockAnalysisProvider::new());
        let captured = Arc::clone(&provider);
        let (peer, _runtime) =
            stock_analysis_peer_with_permissions(provider, vec![PluginPermission::Write]);

        let payload = json!({
            "mainline_date": "2026-11-09",
            "mainlines": [{ "theme": "AI 算力", "narrative": "测试" }],
            "archive_missing": true,
        });
        let expected = payload.clone();
        let (response,) =
            with_watchdog(Duration::from_secs(2), "stock.analysis 透传", move || {
                let response = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.analysis",
                        "op": "batch_upsert_mainlines",
                        "args": { "request": payload },
                    }),
                    &[],
                );
                (response,)
            });
        assert_eq!(
            response.kind,
            axagent_plugin_proto::ResponseKind::Success,
            "有 write 权限的写 op 必须成功，实际 {response:?}"
        );
        assert_eq!(
            captured.upsert_calls.lock().clone(),
            vec![expected],
            "载荷必须逐字到 provider：不改名、不补默认值、不裁剪"
        );
    }

    /// 裁定 N2 的两种反面形态都必须落 `SEAM_CALL_INVALID_ARGS` 且零调用：
    /// ① 完全缺参；② **平铺四字段**（不嵌 `request` 层）—— 分派层只认嵌一层形态。
    #[test]
    fn monitor_upsert_alert_missing_request_is_invalid_args() {
        let monitor = Arc::new(FixedMonitorProvider::new());
        let watcher = Arc::new(FixedQuoteWatcher::new());
        let captured = Arc::clone(&monitor);
        let (peer, _runtime) =
            monitor_peer_with_permissions(monitor, watcher, vec![PluginPermission::Write]);

        let (missing, flat) =
            with_watchdog(Duration::from_secs(2), "upsert_alert 反例", move || {
                let missing = peer.handle_call_seam(
                    &json!({ "seam": "stock.monitor", "op": "upsert_alert", "args": {} }),
                    &[],
                );
                // 平铺形态：键都在、只是没套 `request` 外层 ⇒ 同样必须拒（禁止分派层兼容平铺）。
                let flat = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "upsert_alert",
                        "args": {
                            "stockCode": "600519",
                            "stockName": "贵州茅台",
                            "alertType": "stop_loss",
                            "threshold": 1400.0,
                        },
                    }),
                    &[],
                );
                (missing, flat)
            });
        assert_eq!(
            missing.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `request` 必须走参数错误，实际 {missing:?}"
        );
        assert!(
            missing.message.as_deref().unwrap_or_default().contains("request"),
            "文案应点名缺失的 `request`，实际：{missing:?}"
        );
        assert_eq!(
            flat.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "平铺四字段不被支持，必须落参数错误，实际 {flat:?}"
        );
        assert!(captured.upsert_calls.lock().is_empty(), "参数错误不得触达 provider");
    }

    /// 嵌一层 `request`（camelCase）⇒ 原样反序列化为 `AlertUpsertRequest` 透传；响应里
    /// `alert` 是 12 列 camelCase 投影（`isTriggered` 保持 `i32`），`reused` / `wired`
    /// 三判据并列原样透出（`wired = false` 不得被抹平成成功语义）。
    #[test]
    fn monitor_upsert_alert_forwards_nested_camel_request() {
        let monitor = Arc::new(FixedMonitorProvider::new());
        let watcher = Arc::new(FixedQuoteWatcher::new());
        let captured = Arc::clone(&monitor);
        let (peer, _runtime) =
            monitor_peer_with_permissions(monitor, watcher, vec![PluginPermission::Write]);
        let response = with_watchdog(Duration::from_secs(5), "upsert_alert 正例", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "stock.monitor",
                    "op": "upsert_alert",
                    "args": {
                        "request": {
                            "stockCode": "600519",
                            "stockName": "贵州茅台",
                            "alertType": "stop_loss",
                            "threshold": 1400.0,
                        },
                    },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "已授权且参数齐备应成功，实际 {response:?}");
        let calls = captured.upsert_calls.lock().clone();
        assert_eq!(calls.len(), 1, "upsert_alert 应恰被调用一次");
        assert_eq!(calls[0].stock_code, "600519", "camelCase `stockCode` 应落进 snake_case 字段");
        assert_eq!(calls[0].alert_type, "stop_loss");
        assert_eq!(calls[0].threshold, 1400.0);
        let value = response.value.expect("成功响应必须带 value");
        assert_eq!(value["reused"], json!(true));
        assert_eq!(value["wired"], json!(false), "`wired = false` 必须原样透出");
        assert_eq!(value["alert"]["alertType"], json!("stop_loss"));
        assert_eq!(
            value["alert"]["isTriggered"],
            json!(0),
            "`isTriggered` 是 i32，不得收敛成 bool"
        );
        assert!(value["alert"].get("is_triggered").is_none(), "不得残留 snake_case 键");
    }

    /// `status` 是只读 op（空权限表也应放行）且返回 camelCase 九键；未被替身覆盖的
    /// `list_configs` 吃 trait 默认实现 ⇒ `PLUGIN_CALL_FAILED`，文案点名该 op「未实现」。
    #[test]
    fn monitor_status_camel_and_default_unimplemented_op_is_call_failed() {
        let monitor = Arc::new(FixedMonitorProvider::new());
        let watcher = Arc::new(FixedQuoteWatcher::new());
        let captured = Arc::clone(&monitor);
        let (peer, _runtime) = monitor_peer_with_permissions(monitor, watcher, Vec::new());

        let (ok, unimplemented) =
            with_watchdog(Duration::from_secs(5), "stock.monitor 读面", move || {
                let ok = peer.handle_call_seam(
                    &json!({ "seam": "stock.monitor", "op": "status", "args": {} }),
                    &[],
                );
                let unimplemented = peer.handle_call_seam(
                    &json!({ "seam": "stock.monitor", "op": "list_configs", "args": {} }),
                    &[],
                );
                (ok, unimplemented)
            });
        assert_eq!(ok.code, None, "只读 op 不得被权限闸拦，实际 {ok:?}");
        let value = ok.value.expect("status 必须带 value");
        assert_eq!(value["assembled"], json!(true));
        assert_eq!(value["tradingNow"], json!(true));
        assert_eq!(value["pollIntervalSecs"], json!(30));
        assert_eq!(value["roundCodesFailed"], json!(1));
        assert_eq!(value["roundFinishedAtMs"], json!(1_700_000_000_000_i64));
        assert!(value.get("poll_interval_secs").is_none(), "不得残留 snake_case 键");
        assert_eq!(*captured.status_calls.lock(), 1, "status 应恰被调用一次");
        assert_eq!(
            unimplemented.code.as_deref(),
            Some(error_codes::PLUGIN_CALL_FAILED),
            "trait 默认 Err(Provider) 应落成 PLUGIN_CALL_FAILED，实际 {unimplemented:?}"
        );
        let detail = unimplemented.message.clone().unwrap_or_default();
        assert!(detail.contains("list_configs"), "文案应点名未实现的 op，实际：{detail}");
        assert!(detail.contains("未实现"), "文案应说明未实现，实际：{detail}");
    }

    /// `set_t0_config` 全量结构体（缺 `minIntervalMinutes` ⇒ 整条 reject —— 该 DTO 刻意
    /// 不带 `serde(default)`）；`remove_alert` 透出 `{dbDeleted, memoryRemoved}` camel 两数。
    #[test]
    fn monitor_write_ops_roundtrip_and_missing_field_rejected() {
        let monitor = Arc::new(FixedMonitorProvider::new());
        let watcher = Arc::new(FixedQuoteWatcher::new());
        let captured = Arc::clone(&monitor);
        let (peer, _runtime) =
            monitor_peer_with_permissions(monitor, watcher, vec![PluginPermission::Write]);

        let (ok, partial, no_config, removed) =
            with_watchdog(Duration::from_secs(5), "stock.monitor 写面", move || {
                let ok = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "set_t0_config",
                        "args": {
                            "config": {
                                "enabled": true,
                                "changePctThreshold": 3.0,
                                "minIntervalMinutes": 30,
                            },
                        },
                    }),
                    &[],
                );
                // 缺必填 `minIntervalMinutes` ⇒ 整条 reject（保留「缺字段即报错」防线；
                // 两个阈值是 Option，省略是合法的，不能拿来当缺参反例）。
                let partial = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "set_t0_config",
                        "args": { "config": { "enabled": false } },
                    }),
                    &[],
                );
                // `turnoverRateThreshold` 这类 Option 字段可以省略（正例已证 None 兜住）；
                // 这里验证真正「缺 config 外层键」同样是 invalid_args。
                let no_config = peer.handle_call_seam(
                    &json!({ "seam": "stock.monitor", "op": "set_t0_config", "args": {} }),
                    &[],
                );
                let removed = peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "remove_alert",
                        "args": { "alert_id": "alert-uuid-1" },
                    }),
                    &[],
                );
                (ok, partial, no_config, removed)
            });
        assert_eq!(ok.code, None, "全量 camelCase 配置应成功，实际 {ok:?}");
        assert_eq!(ok.value, Some(Value::Null), "`Result<()>` 成功应回 JSON null");
        let seen = captured.t0_calls.lock().clone();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].enabled);
        assert_eq!(seen[0].change_pct_threshold, Some(3.0));
        assert_eq!(seen[0].turnover_rate_threshold, None, "缺省 Option 字段应落 None");
        assert_eq!(seen[0].min_interval_minutes, 30);
        assert_eq!(
            partial.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `minIntervalMinutes` 必须整条拒绝，实际 {partial:?}"
        );
        assert_eq!(
            no_config.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `config` 必须走参数错误，实际 {no_config:?}"
        );
        assert_eq!(captured.t0_calls.lock().len(), 1, "两次参数错误都不得触达 provider");
        assert_eq!(removed.code, None, "删告警应成功，实际 {removed:?}");
        assert_eq!(
            removed.value,
            Some(json!({ "dbDeleted": true, "memoryRemoved": false })),
            "两个计数域必须并列 camelCase 透出"
        );
        assert_eq!(captured.remove_calls.lock().clone(), vec!["alert-uuid-1".to_string()]);
    }

    /// 已知接缝 + 未知 op ⇒ `SEAM_CALL_UNSUPPORTED`（不猜、不兜底）：
    /// `aggregator_stats`（裁定 J7 首批不发）与 `set_priority`（裁定 K-6 并入 watch）。
    #[test]
    fn monitor_watcher_unknown_ops_are_unsupported() {
        let monitor = Arc::new(FixedMonitorProvider::new());
        let watcher = Arc::new(FixedQuoteWatcher::new());
        let (peer, _runtime) =
            monitor_peer_with_permissions(monitor, watcher, vec![PluginPermission::Write]);
        let responses = with_watchdog(Duration::from_secs(2), "未知 op", move || {
            [("stock.monitor", "aggregator_stats"), ("quote.watcher", "set_priority")].map(
                |(seam, op)| {
                    peer.handle_call_seam(&json!({ "seam": seam, "op": op, "args": {} }), &[])
                },
            )
        });
        for response in responses {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::SEAM_CALL_UNSUPPORTED),
                "已知接缝的未知 op 必须落 SEAM_CALL_UNSUPPORTED，实际 {response:?}"
            );
        }
    }

    /// `watch` 三格：`priority` **严格解析**（`bg` / 缺失 ⇒ `SEAM_CALL_INVALID_ARGS`，
    /// 不继承命令层的静默回落）；正例透传 `(codes, priority, replace)` 且截断两数原样暴露；
    /// `list_watched` 返回**具名对象**；`unwatch` 吃替身默认实现 ⇒ `PLUGIN_CALL_FAILED`。
    #[test]
    fn watcher_watch_priority_is_strict_and_rejects_bg() {
        let monitor = Arc::new(FixedMonitorProvider::new());
        let watcher = Arc::new(FixedQuoteWatcher::new());
        let captured = Arc::clone(&watcher);
        let (peer, _runtime) =
            monitor_peer_with_permissions(monitor, watcher, vec![PluginPermission::Write]);

        let inside = Arc::clone(&captured);
        let (bg, no_priority, ok, defaulted, listed, unwatch, rejected_calls) =
            with_watchdog(Duration::from_secs(5), "quote.watcher 全 op", move || {
                // `bg` 是命令层的历史别名：接缝必须拒，而不是静默当 background、更不是回落 active。
                let bg = peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "watch",
                        "args": { "codes": ["600519"], "priority": "bg" },
                    }),
                    &[],
                );
                let no_priority = peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "watch",
                        "args": { "codes": ["600519"] },
                    }),
                    &[],
                );
                // 拒绝时刻的快照：两次参数错误都不得触达 provider（其后的合法调用会让
                // 计数上升，跨调用断言必须在闭包内取数才有意义）。
                let rejected_calls = inside.watch_calls.lock().len();
                let ok = peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "watch",
                        "args": {
                            "codes": ["600519", "000001", "300750"],
                            "priority": "background",
                            "replace": true,
                        },
                    }),
                    &[],
                );
                // `replace` 缺省 false（可选参数走 deserialize_optional_arg）。
                let defaulted = peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "watch",
                        "args": { "codes": ["600519"], "priority": "active" },
                    }),
                    &[],
                );
                let listed = peer.handle_call_seam(
                    &json!({ "seam": "quote.watcher", "op": "list_watched", "args": {} }),
                    &[],
                );
                let unwatch = peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "unwatch",
                        "args": { "codes": ["600519"] },
                    }),
                    &[],
                );
                (bg, no_priority, ok, defaulted, listed, unwatch, rejected_calls)
            });
        assert_eq!(
            bg.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "`bg` 别名不得继承，必须落参数错误，实际 {bg:?}"
        );
        assert_eq!(rejected_calls, 0, "被拒的 `bg` / 缺参调用不得触达 provider");
        assert_eq!(
            no_priority.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "`priority` 必填，缺失必须落参数错误，实际 {no_priority:?}"
        );
        assert_eq!(ok.code, None, "合法 watch 应成功，实际 {ok:?}");
        let calls = captured.watch_calls.lock().clone();
        assert_eq!(calls.len(), 2, "两次合法 watch 应触达 provider");
        assert_eq!(
            calls[0],
            (
                vec!["600519".to_string(), "000001".to_string(), "300750".to_string()],
                axagent_harness::WatchPriority::Background,
                true,
            ),
            "codes / priority / replace 三参必须原样透传"
        );
        // 替身固定 applied=2（模拟被闸截断）：requested=3 ⇒ 两数并列原样透出。
        assert_eq!(ok.value, Some(json!({ "requested": 3, "applied": 2 })));
        assert_eq!(defaulted.code, None, "省略 replace 应成功，实际 {defaulted:?}");
        assert!(!calls[1].2, "`replace` 缺省必须为 false");
        assert_eq!(listed.code, None, "只读 op 应成功，实际 {listed:?}");
        assert_eq!(
            listed.value,
            Some(json!({ "codes": ["000001", "600519"] })),
            "必须返回具名对象 `codes`，不返回裸数组"
        );
        assert_eq!(*captured.list_calls.lock(), 1);
        assert_eq!(
            unwatch.code.as_deref(),
            Some(error_codes::PLUGIN_CALL_FAILED),
            "替身未实现 unwatch ⇒ 吃 trait 默认 Err(Provider)，实际 {unwatch:?}"
        );
        assert!(
            unwatch
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("QuoteWatcherProvider::unwatch"),
            "文案应点名 trait 方法，实际：{unwatch:?}"
        );
    }

    /// 注册表取不到 provider（启动窗口内 / 提供方卸载后）⇒ `SEAM_PROVIDER_UNAVAILABLE`，
    /// 明确失败不挂起。`test_host_peer` 的 `runtime: None` 同时证明取 provider 失败发生在
    /// `block_on` 之前（无 runtime 也拿得到该码，而不是 `PLUGIN_CALL_FAILED`）。
    #[test]
    fn monitor_watcher_unregistered_provider_is_unavailable() {
        let empty_registry = Arc::new(CapabilityRegistry::new());
        let peer =
            test_host_peer("plugin.none", &[], &["stock.monitor", "quote.watcher"], empty_registry);
        let responses = with_watchdog(Duration::from_secs(2), "provider 缺位", move || {
            [("stock.monitor", "status"), ("quote.watcher", "list_watched")].map(|(seam, op)| {
                peer.handle_call_seam(&json!({ "seam": seam, "op": op, "args": {} }), &[])
            })
        });
        for (seam, response) in [("stock.monitor", &responses[0]), ("quote.watcher", &responses[1])]
        {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::SEAM_PROVIDER_UNAVAILABLE),
                "`{seam}` 未注册 provider 必须落 SEAM_PROVIDER_UNAVAILABLE，实际 {response:?}"
            );
        }
    }

    // ───── knowledge.vector 接缝（4 op）+ trajectory.store 接缝（4 op）─────

    /// 假知识库向量提供者：**刻意不覆盖 `keyword_search`**，让它吃 trait 默认实现
    /// （`Err(Provider("…未实现…"))` ⇒ 分派落 `PLUGIN_CALL_FAILED`），顺带钉住「外部实现只覆盖
    /// 关心的 op」这条契约口径（先例 `FixedMonitorProvider` / `FixedQuoteWatcher`）。
    ///
    /// 记账一律用 `parking_lot::Mutex`（本文件既有替身同源做法）：`clippy.toml` 的
    /// `disallowed-types` 禁的是 `std::sync` 锁，且这里的临界区内不跨任何 `await`。
    struct FixedKnowledgeVector {
        /// `search` 收到的请求序列（取证「camelCase 线上键 → snake_case Rust 字段」落点）。
        search_calls: Mutex<Vec<axagent_harness::VectorSearchRequest>>,
        /// `index_text` 收到的请求序列（唯一写 op 的触达取证）。
        index_calls: Mutex<Vec<axagent_harness::VectorIndexRequest>>,
        /// `collection_info` 收到的集合 id 序列（⚠ op 名 `get_collection_info` ≠ 方法名）。
        collection_info_calls: Mutex<Vec<String>>,
    }

    impl FixedKnowledgeVector {
        fn new() -> Self {
            Self {
                search_calls: Mutex::new(Vec::new()),
                index_calls: Mutex::new(Vec::new()),
                collection_info_calls: Mutex::new(Vec::new()),
            }
        }

        /// 命中块样本：`score` 是 **f32**（L2 距离，越小越相似，方向由宿主 provider 侧权威）。
        ///
        /// ⚠ 断言侧不许写 `assert_eq!(val["score"], json!(0.21))` —— `f32` 经 `serde_json::Value`
        /// 读出的是 f64，与 f64 字面量**逐位不等**；只能 `.as_f64()` 后按容差比。
        fn sample_chunk() -> axagent_harness::KnowledgeChunkView {
            axagent_harness::KnowledgeChunkView {
                id: "chunk-1".to_string(),
                document_id: "doc-1".to_string(),
                chunk_index: 3,
                snippet: "贵州茅台三季度营收……（截断预览）".to_string(),
                score: 0.21,
                has_embedding: true,
            }
        }
    }

    #[async_trait]
    impl axagent_harness::KnowledgeVectorProvider for FixedKnowledgeVector {
        async fn search(
            &self,
            request: axagent_harness::VectorSearchRequest,
        ) -> axagent_harness::core_error::Result<axagent_harness::VectorSearchOutcome> {
            self.search_calls.lock().push(request.clone());
            // 「查无结果」的形态由替身按查询文本决定：空数组而不是错误（分派层不预判）。
            if request.query == "无命中" {
                return Ok(axagent_harness::VectorSearchOutcome { results: Vec::new() });
            }
            Ok(axagent_harness::VectorSearchOutcome { results: vec![Self::sample_chunk()] })
        }

        async fn index_text(
            &self,
            request: axagent_harness::VectorIndexRequest,
        ) -> axagent_harness::core_error::Result<axagent_harness::VectorIndexOutcome> {
            self.index_calls.lock().push(request.clone());
            // 回射 id 便于取证透传无损；`chunk_count` 给非零样本，`0`（切不出块）同样合法。
            Ok(axagent_harness::VectorIndexOutcome {
                chunk_count: 7,
                document_id: request.document_id.clone(),
                collection_id: request.collection_id.clone(),
            })
        }

        async fn collection_info(
            &self,
            collection_id: &str,
        ) -> axagent_harness::core_error::Result<axagent_harness::CollectionInfoView> {
            self.collection_info_calls.lock().push(collection_id.to_string());
            // 集合不存在 ⇒ `Ok` + `exists = false` + `dimensions = null`，**不是错误**。
            Ok(axagent_harness::CollectionInfoView {
                collection_id: collection_id.to_string(),
                dimensions: None,
                exists: false,
            })
        }
        // `keyword_search` 不覆盖 ⇒ 吃 trait 默认 `Err(Provider)`。
    }

    /// 假轨迹库提供者：四个读方法全覆盖（首批无写 op ⇒ 没有需要 `write` 的替身方法）。
    struct FixedTrajectoryStore {
        /// `list` 收到的 `limit` 序列（`None` = 缺省，合法）。
        list_calls: Mutex<Vec<Option<usize>>>,
        /// `get` 收到的 id 序列。
        get_calls: Mutex<Vec<String>>,
        /// `query` 收到的视图序列。
        query_calls: Mutex<Vec<axagent_harness::TrajectoryQueryView>>,
        /// `count` 调用次数。
        count_calls: Mutex<usize>,
    }

    impl FixedTrajectoryStore {
        fn new() -> Self {
            Self {
                list_calls: Mutex::new(Vec::new()),
                get_calls: Mutex::new(Vec::new()),
                query_calls: Mutex::new(Vec::new()),
                count_calls: Mutex::new(0),
            }
        }

        /// 摘要视图样本：字段裁剪（无 `steps` / `rewards` / `summary` 原文）由契约层负责，
        /// 本替身只给出「已经裁好的」视图，分派层原样透出。
        fn sample_view() -> axagent_harness::TrajectorySummaryView {
            axagent_harness::TrajectorySummaryView {
                id: "traj-1".to_string(),
                session_id: "sess-1".to_string(),
                user_id: "user-1".to_string(),
                agent_name: Some("股票分析师".to_string()),
                topic: "贵州茅台建仓".to_string(),
                summary_snippet: "先观察再小仓试买……（截断预览）".to_string(),
                outcome: axagent_harness::trajectory_types::TrajectoryOutcome::Success,
                duration_ms: 4200,
                quality: axagent_harness::trajectory_types::TrajectoryQuality::default(),
                value_score: 0.62,
                created_at_ms: 1_700_000_000_000,
                replay_count: 3,
            }
        }
    }

    #[async_trait]
    impl axagent_harness::TrajectoryStoreProvider for FixedTrajectoryStore {
        async fn list(
            &self,
            limit: Option<usize>,
        ) -> axagent_harness::core_error::Result<axagent_harness::TrajectoryListOutcome> {
            self.list_calls.lock().push(limit);
            Ok(axagent_harness::TrajectoryListOutcome { trajectories: vec![Self::sample_view()] })
        }

        async fn get(
            &self,
            id: &str,
        ) -> axagent_harness::core_error::Result<Option<axagent_harness::TrajectorySummaryView>>
        {
            self.get_calls.lock().push(id.to_string());
            // 查无 ⇒ `Ok(None)`（帧里 `null`），不是错误、不借错误码。
            if id == "traj-1" {
                Ok(Some(Self::sample_view()))
            } else {
                Ok(None)
            }
        }

        async fn query(
            &self,
            filter: axagent_harness::TrajectoryQueryView,
        ) -> axagent_harness::core_error::Result<axagent_harness::TrajectoryQueryOutcome> {
            self.query_calls.lock().push(filter);
            Ok(axagent_harness::TrajectoryQueryOutcome { trajectories: vec![Self::sample_view()] })
        }

        async fn count(
            &self,
        ) -> axagent_harness::core_error::Result<axagent_harness::TrajectoryCountOutcome> {
            *self.count_calls.lock() += 1;
            // 空库的 `0` 是合法值，不是错误。
            Ok(axagent_harness::TrajectoryCountOutcome { count: 0 })
        }
    }

    /// 一个方法都不覆盖的轨迹提供者：四个 op 全吃 trait 默认 `Err(Provider("…未实现…"))`
    /// ⇒ 用来取证「默认未实现」在分派层落成 `PLUGIN_CALL_FAILED`（与 `FixedKnowledgeVector`
    /// 刻意留空的 `keyword_search` 同一条链路，两条接缝各钉一格）。
    struct BareTrajectoryStore;

    #[async_trait]
    impl axagent_harness::TrajectoryStoreProvider for BareTrajectoryStore {}

    /// 装 knowledge.vector + trajectory.store 提供者的 peer + 保活到用例结束的 runtime。
    ///
    /// 形参收 **trait 对象**，好让上面三种替身（全覆盖 / 留空 / 全空）都能塞进来。
    /// 权限表由调用方给：`index_text` 的 `write` 准入需要它，只读 op 用例传空表以证明不设闸。
    fn knowledge_trajectory_peer(
        knowledge: Arc<dyn axagent_harness::KnowledgeVectorProvider>,
        store: Arc<dyn axagent_harness::TrajectoryStoreProvider>,
        permissions: Vec<PluginPermission>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        registry.register_knowledge_vector(knowledge).expect("knowledge.vector 接缝应注册成功");
        registry.register_trajectory_store(store).expect("trajectory.store 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.knowledge".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["knowledge.vector".to_string(), "trajectory.store".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions,
        };
        (peer, runtime)
    }

    /// ①（+ 裁定 §58.3-4）：两条接缝共 8 个 op 的权限表逐条枚举锁 ——
    /// `knowledge.vector` 只有 `index_text` 一个写 op，`trajectory.store` 首批四个 op 全读
    /// ⇒ **一条条目都不许加**（多登记一条 = 无 write 权限的插件被无端拒掉）。
    #[test]
    fn knowledge_trajectory_permission_table_is_eight_ops_one_write() {
        assert_eq!(
            required_permission_for("knowledge.vector", "index_text"),
            Some(PluginPermission::Write),
            "index_text 是本接缝唯一写 op"
        );
        for read_op in ["search", "keyword_search", "get_collection_info"] {
            assert_eq!(
                required_permission_for("knowledge.vector", read_op),
                None,
                "只读 op `knowledge.vector`/`{read_op}` 不得设权限闸"
            );
        }
        for read_op in ["list", "get", "query", "count"] {
            assert_eq!(
                required_permission_for("trajectory.store", read_op),
                None,
                "首批只读：`trajectory.store`/`{read_op}` 不得设权限闸"
            );
        }
        // 平铺/别名的缺省形态：写权限只在 `(knowledge.vector, index_text)` 这一格出现。
        assert_eq!(required_permission_for("knowledge.vector", "index"), None);
        assert_eq!(required_permission_for("trajectory", "index_text"), None);
    }

    /// 裁定 N2 / §58.3-3 的反面两格（knowledge.vector）：① 缺 `request` 外层键；
    /// ② **平铺 DTO 字段**（键都在、只是没套一层）—— 分派层不留任何平铺兼容代码，
    /// 两者都必须 `SEAM_CALL_INVALID_ARGS`，且**拒绝时刻**的快照零触达 provider。
    #[test]
    fn knowledge_vector_nested_request_missing_and_flat_are_invalid_args() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let captured = Arc::clone(&knowledge);
        let store = Arc::new(FixedTrajectoryStore::new());
        // 带 write 权限：让 `index_text` 越过准入闸，拒因才是「入参形态」而不是权限。
        let (peer, _runtime) =
            knowledge_trajectory_peer(knowledge, store, vec![PluginPermission::Write]);
        let inside = Arc::clone(&captured);
        let (missing, flat, flat_index, after_rejections) =
            with_watchdog(Duration::from_secs(2), "knowledge.vector 反例", move || {
                let missing = peer.handle_call_seam(
                    &json!({ "seam": "knowledge.vector", "op": "search", "args": {} }),
                    &[],
                );
                // 平铺形态：`VectorSearchRequest` 的三字段直接挂在 args 上。
                let flat = peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "search",
                        "args": { "knowledgeBaseId": "kb-1", "query": "筹码集中度", "topK": 8 },
                    }),
                    &[],
                );
                let flat_index = peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "index_text",
                        "args": {
                            "collectionId": "kb-1",
                            "documentId": "doc-1",
                            "content": "正文",
                        },
                    }),
                    &[],
                );
                // 拒绝时刻取快照（后续合法调用会让计数上升）。
                let after_rejections =
                    (inside.search_calls.lock().len(), inside.index_calls.lock().len());
                (missing, flat, flat_index, after_rejections)
            });
        for (what, response) in [("缺 request", &missing), ("平铺 request", &flat)] {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::SEAM_CALL_INVALID_ARGS),
                "{what} 形态必须落参数错误，实际 {response:?}"
            );
            assert!(
                response.message.as_deref().unwrap_or_default().contains("request"),
                "{what} 的文案应点名缺失的 `request` 键，实际：{response:?}"
            );
        }
        assert_eq!(
            flat_index.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "写 op 的平铺形态同样拒，实际 {flat_index:?}"
        );
        assert_eq!(after_rejections, (0, 0), "三次参数错误都不得触达 provider");
    }

    /// 裁定 N2 的反面两格（trajectory.store 的 `query`）：缺 `request` 与平铺 `request`。
    #[test]
    fn trajectory_store_query_missing_and_flat_request_are_invalid_args() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let store = Arc::new(FixedTrajectoryStore::new());
        let captured = Arc::clone(&store);
        let (peer, _runtime) = knowledge_trajectory_peer(knowledge, store, Vec::new());
        let inside = Arc::clone(&captured);
        let (missing, flat, rejected_calls) =
            with_watchdog(Duration::from_secs(2), "trajectory.store 反例", move || {
                let missing = peer.handle_call_seam(
                    &json!({ "seam": "trajectory.store", "op": "query", "args": {} }),
                    &[],
                );
                let flat = peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "query",
                        "args": { "sessionId": "sess-1", "limit": 20 },
                    }),
                    &[],
                );
                let rejected_calls = inside.query_calls.lock().len();
                (missing, flat, rejected_calls)
            });
        assert_eq!(
            missing.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `request` 必须走参数错误，实际 {missing:?}"
        );
        assert_eq!(
            flat.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "平铺形态不被支持，必须落参数错误，实际 {flat:?}"
        );
        assert_eq!(rejected_calls, 0, "被拒的 query 不得触达 provider");
    }

    /// 只记录 `(limit, offset)` 的 `analysis.record` 提供者：取证分派层两个**可选标量**
    /// 的翻译（缺失 / `null` ⇒ `None`，数字原样进门；非数字 ⇒ 参数错误且零触达）。
    struct FixedAnalysisRecord {
        calls: Mutex<Vec<(Option<usize>, Option<usize>)>>,
    }

    impl FixedAnalysisRecord {
        fn new() -> Self {
            Self { calls: Mutex::new(Vec::new()) }
        }
    }

    #[async_trait]
    impl axagent_harness::AnalysisRecordProvider for FixedAnalysisRecord {
        async fn list(
            &self,
            limit: Option<usize>,
            offset: Option<usize>,
        ) -> axagent_harness::core_error::Result<axagent_harness::AnalysisRecordListOutcome>
        {
            self.calls.lock().push((limit, offset));
            Ok(axagent_harness::AnalysisRecordListOutcome { records: Vec::new(), total: 338 })
        }
    }

    /// 一个方法都不覆盖：`list` 吃 trait 默认 `Err(Provider("…未实现…"))`
    /// ⇒ 用来取证「默认未实现」在分派层落成 `PLUGIN_CALL_FAILED`。
    struct BareAnalysisRecord;

    #[async_trait]
    impl axagent_harness::AnalysisRecordProvider for BareAnalysisRecord {}

    fn analysis_record_peer(
        provider: Arc<dyn axagent_harness::AnalysisRecordProvider>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        registry.register_analysis_record(provider).expect("analysis.record 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.stock".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["analysis.record".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions: Vec::new(),
        };
        (peer, runtime)
    }

    /// ① `analysis.record` 的 `list`：两个可选标量的三种形态（给值 / 缺省 / 非数字），
    /// 以及出参是契约的具名对象（`records` + `total`）。
    ///
    /// 只读 op ⇒ **不设权限闸**（`permissions` 传空表）：这条同时是那一格的取证 ——
    /// 若哪天有人往 `required_permission_for` 里加一条 `analysis.record` 条目，本用例即红。
    #[test]
    fn analysis_record_list_forwards_optional_scalars_and_returns_named_outcome() {
        let store = Arc::new(FixedAnalysisRecord::new());
        let captured = Arc::clone(&store);
        let (peer, _runtime) = analysis_record_peer(store);
        let (first, second, bad, calls) =
            with_watchdog(Duration::from_secs(2), "analysis.record list", move || {
                let first = peer.handle_call_seam(
                    &json!({
                        "seam": "analysis.record",
                        "op": "list",
                        "args": { "limit": 20, "offset": 40 },
                    }),
                    &[],
                );
                // 缺省形态：两个键都不给（宿主按契约取缺省 limit 与 offset=0）。
                let second = peer.handle_call_seam(
                    &json!({ "seam": "analysis.record", "op": "list", "args": {} }),
                    &[],
                );
                // 非数字：必须是参数错误，且**不得**触达 provider。
                let bad = peer.handle_call_seam(
                    &json!({
                        "seam": "analysis.record",
                        "op": "list",
                        "args": { "limit": "二十" },
                    }),
                    &[],
                );
                let calls = captured.calls.lock().clone();
                (first, second, bad, calls)
            });

        assert_eq!(first.kind, ResponseKind::Success, "第一次调用应成功：{first:?}");
        let value = first.value.expect("成功帧必须带 value");
        let mut keys: Vec<String> =
            value.as_object().expect("出参应是具名对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["records", "total"], "出参是 {{records, total}}，不是裸数组：{value}");
        assert_eq!(value["total"], json!(338));
        assert_eq!(value["records"], json!([]), "空页是空数组，不是错误");

        assert_eq!(second.kind, ResponseKind::Success, "缺省形态应成功：{second:?}");
        assert_eq!(
            bad.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "非数字 limit 必须落参数错误：{bad:?}"
        );

        // 前两次原样进门；第三次被拒 ⇒ 零触达（`limit` 的 snake_case 不做兼容翻译）。
        assert_eq!(calls, vec![(Some(20), Some(40)), (None, None)], "两次合法调用应原样透传");
    }

    /// ② 未覆盖任何方法的 provider ⇒ 默认 `Err(Provider)` 落成 `PLUGIN_CALL_FAILED`
    /// （与 `trajectory.store` 的 `BareTrajectoryStore` 同一条链路）。
    #[test]
    fn analysis_record_unimplemented_provider_maps_to_call_failed() {
        let (peer, _runtime) = analysis_record_peer(Arc::new(BareAnalysisRecord));
        let response =
            with_watchdog(Duration::from_secs(2), "analysis.record 未实现", move || {
                peer.handle_call_seam(
                    &json!({ "seam": "analysis.record", "op": "list", "args": {} }),
                    &[],
                )
            });
        assert_eq!(response.kind, ResponseKind::Error);
        assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_CALL_FAILED));
    }

    /// 只记录 `(limit, offset)` 的 `opc.store` 提供者：取证分派层两个**可选标量**的翻译，
    /// 与出参是契约的具名对象（`posts` + `total`）。
    ///
    /// 写面同样在这份替身上取证：记 `publish_calls` / `delete_calls` / `create_calls`
    /// （入参逐字），并让结果**可配置** —— 「命中 / 缺行 / 受保护」在分派层必须走同一条
    /// payload 通道，只有真故障才走错误码。
    struct FixedOpcStore {
        calls: Mutex<Vec<(Option<usize>, Option<usize>)>>,
        publish_calls: Mutex<Vec<String>>,
        publish_outcome: axagent_harness::PublishBlogPostOutcome,
        /// 删除写面：记 id 逐字，三态可配置（默认 `Removed`）。
        delete_calls: Mutex<Vec<String>>,
        delete_outcome: axagent_harness::RemoveBlogPostOutcome,
        /// 创建写面：记整份 DTO（逐字进门），并可用 `create_rejects` 让 provider 回
        /// `Err(Validation)` —— 正文超预算在宿主侧就是这条链路（`SEAM_CALL_INVALID_ARGS`）。
        create_calls: Mutex<Vec<axagent_harness::CreateBlogPostInput>>,
        create_rejects: Option<String>,
        /// #36 批 2 四条写 op 的入参逐字：`transition` 的 `(id, target)`、update 的
        /// `(id, DTO)`、两条删除的 id。`batch2_not_found` ⇒ 四条都回缺行（payload）。
        transition_calls: Mutex<Vec<(String, String)>>,
        update_calls: Mutex<Vec<(String, axagent_harness::UpdateContentAssetInput)>>,
        delete_asset_calls: Mutex<Vec<String>>,
        cancel_schedule_calls: Mutex<Vec<String>>,
        batch2_not_found: bool,
        /// D-4 三条写 op 的入参逐字：发布/删除落地页与联系已读，都是纯 `id`。
        landing_publish_calls: Mutex<Vec<String>>,
        landing_delete_calls: Mutex<Vec<String>>,
        contact_read_calls: Mutex<Vec<String>>,
    }

    impl FixedOpcStore {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                publish_calls: Mutex::new(Vec::new()),
                publish_outcome: axagent_harness::PublishBlogPostOutcome::Published,
                delete_calls: Mutex::new(Vec::new()),
                delete_outcome: axagent_harness::RemoveBlogPostOutcome::Removed,
                create_calls: Mutex::new(Vec::new()),
                create_rejects: None,
                transition_calls: Mutex::new(Vec::new()),
                update_calls: Mutex::new(Vec::new()),
                delete_asset_calls: Mutex::new(Vec::new()),
                cancel_schedule_calls: Mutex::new(Vec::new()),
                batch2_not_found: false,
                landing_publish_calls: Mutex::new(Vec::new()),
                landing_delete_calls: Mutex::new(Vec::new()),
                contact_read_calls: Mutex::new(Vec::new()),
            }
        }

        /// 批 2 四条写 op 全部回缺行（取证「缺行是 payload，不占错误码」）。
        fn batch2_missing() -> Self {
            Self { batch2_not_found: true, ..Self::new() }
        }

        /// 让写面回 `NotFound` 的夹具形态（缺行是**正常结果**，用例要靠它区分 payload 与错误码）。
        fn missing() -> Self {
            Self {
                publish_outcome: axagent_harness::PublishBlogPostOutcome::NotFound,
                delete_outcome: axagent_harness::RemoveBlogPostOutcome::NotFound,
                ..Self::new()
            }
        }

        /// 让删除写面回 `Protected`（受保护 id：**未删除**，也是正常结果）。
        fn protected() -> Self {
            Self {
                delete_outcome: axagent_harness::RemoveBlogPostOutcome::Protected,
                ..Self::new()
            }
        }

        /// 让创建写面回 `Err(Validation)`（正文超预算 / 空 slug 在宿主侧就是这一档）。
        fn rejecting_create(message: &str) -> Self {
            Self { create_rejects: Some(message.to_string()), ..Self::new() }
        }
    }

    #[async_trait]
    impl axagent_harness::OpcStoreProvider for FixedOpcStore {
        async fn list_blog_posts(
            &self,
            limit: Option<usize>,
            offset: Option<usize>,
        ) -> axagent_harness::core_error::Result<axagent_harness::BlogPostListOutcome> {
            self.calls.lock().push((limit, offset));
            Ok(axagent_harness::BlogPostListOutcome { posts: Vec::new(), total: 137 })
        }

        async fn publish_blog_post(
            &self,
            id: String,
        ) -> axagent_harness::core_error::Result<axagent_harness::PublishBlogPostOutcome> {
            self.publish_calls.lock().push(id);
            Ok(self.publish_outcome)
        }

        async fn delete_blog_post(
            &self,
            id: String,
        ) -> axagent_harness::core_error::Result<axagent_harness::RemoveBlogPostOutcome> {
            self.delete_calls.lock().push(id);
            Ok(self.delete_outcome)
        }

        async fn create_blog_post(
            &self,
            input: axagent_harness::CreateBlogPostInput,
        ) -> axagent_harness::core_error::Result<axagent_harness::BlogPostView> {
            self.create_calls.lock().push(input.clone());
            if let Some(message) = &self.create_rejects {
                return Err(axagent_harness::core_error::AxAgentError::Validation(message.clone()));
            }
            Ok(axagent_harness::BlogPostView {
                id: "post-new".into(),
                title: input.title,
                slug: input.slug,
                excerpt: input.excerpt,
                tags: input.tags,
                published: false,
                published_at: None,
                view_count: 0,
                created_at: 1_700_000_000,
                updated_at: 1_700_000_000,
            })
        }

        // ── #36 批 2：四条写 op 的替身 ──

        async fn transition_invoice(
            &self,
            id: String,
            target: String,
        ) -> axagent_harness::core_error::Result<axagent_harness::TransitionInvoiceOutcome>
        {
            self.transition_calls.lock().push((id, target));
            if self.batch2_not_found {
                return Ok(axagent_harness::TransitionInvoiceOutcome::NotFound);
            }
            Ok(axagent_harness::TransitionInvoiceOutcome::Transitioned(
                axagent_harness::InvoiceSummaryView {
                    id: "inv-1".into(),
                    customer_id: "cust-1".into(),
                    invoice_number: "INV-001".into(),
                    status: "sent".into(),
                    total: 100.0,
                    currency: "CNY".into(),
                    due_at: None,
                    created_at: 1_700_000_000,
                },
            ))
        }

        async fn update_content_asset(
            &self,
            id: String,
            input: axagent_harness::UpdateContentAssetInput,
        ) -> axagent_harness::core_error::Result<axagent_harness::UpdateContentAssetOutcome>
        {
            self.update_calls.lock().push((id, input));
            if self.batch2_not_found {
                return Ok(axagent_harness::UpdateContentAssetOutcome::NotFound);
            }
            Ok(axagent_harness::UpdateContentAssetOutcome::Updated(
                axagent_harness::ContentAssetSummaryView {
                    id: "asset-1".into(),
                    title: "资产".into(),
                    content_type: "article".into(),
                    status: "draft".into(),
                    tags: vec!["opc".into()],
                    created_at: 1_700_000_000,
                },
            ))
        }

        async fn delete_content_asset(
            &self,
            id: String,
        ) -> axagent_harness::core_error::Result<axagent_harness::RemoveContentAssetOutcome>
        {
            self.delete_asset_calls.lock().push(id);
            Ok(if self.batch2_not_found {
                axagent_harness::RemoveContentAssetOutcome::NotFound
            } else {
                axagent_harness::RemoveContentAssetOutcome::Removed
            })
        }

        async fn cancel_publish_schedule(
            &self,
            id: String,
        ) -> axagent_harness::core_error::Result<axagent_harness::RemovePublishScheduleOutcome>
        {
            self.cancel_schedule_calls.lock().push(id);
            Ok(if self.batch2_not_found {
                axagent_harness::RemovePublishScheduleOutcome::NotFound
            } else {
                axagent_harness::RemovePublishScheduleOutcome::Removed
            })
        }

        // ── D-4：落地页与联系人三条写 op 的替身 ──

        async fn publish_landing_page(
            &self,
            id: String,
        ) -> axagent_harness::core_error::Result<axagent_harness::LandingPageSummaryView> {
            self.landing_publish_calls.lock().push(id);
            Ok(axagent_harness::LandingPageSummaryView {
                id: "lp-1".into(),
                title: "落地页".into(),
                slug: "lp-slug".into(),
                published: true,
                created_at: 1_700_000_000,
            })
        }

        async fn delete_landing_page(&self, id: String) -> axagent_harness::core_error::Result<()> {
            self.landing_delete_calls.lock().push(id);
            Ok(())
        }

        async fn mark_contact_read(&self, id: String) -> axagent_harness::core_error::Result<()> {
            self.contact_read_calls.lock().push(id);
            Ok(())
        }
    }

    /// 一个方法都不覆盖：`list_blog_posts` 吃 trait 默认 `Err(Provider("…未实现…"))`
    /// ⇒ 用来取证「默认未实现」在分派层落成 `PLUGIN_CALL_FAILED`。
    struct BareOpcStore;

    #[async_trait]
    impl axagent_harness::OpcStoreProvider for BareOpcStore {}

    fn opc_store_peer(
        provider: Arc<dyn axagent_harness::OpcStoreProvider>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        // 既有用例走的是「未声明任何权限」的形态 —— 读缝不受闸，那条路必须照样通。
        opc_store_peer_with_permissions(provider, Vec::new())
    }

    /// `permissions` 可注入的形态：写 op 的两条用例（无 `write` 被拒 / 有 `write` 放行）
    /// 靠它把「闸在取 provider 之前」钉成可观测的零触达。
    fn opc_store_peer_with_permissions(
        provider: Arc<dyn axagent_harness::OpcStoreProvider>,
        permissions: Vec<PluginPermission>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        registry.register_opc_store(provider).expect("opc.store 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.opc".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["opc.store".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions,
        };
        (peer, runtime)
    }

    /// ① `opc.store` 的 `list_blog_posts`：两个可选标量的三种形态（给值 / 缺省 / 非数字），
    /// 出参是 `{posts, total}`，**表外 op 明确拒**。
    ///
    /// 只读 op ⇒ **不设权限闸**（`permissions` 传空表）：这条同时是那一格的取证 ——
    /// 若哪天有人往 `required_permission_for` 里加一条 `opc.store` 条目，本用例即红。
    #[test]
    fn opc_store_list_blog_posts_forwards_optional_scalars_and_returns_named_outcome() {
        let store = Arc::new(FixedOpcStore::new());
        let captured = Arc::clone(&store);
        let (peer, _runtime) = opc_store_peer(store);
        let (first, second, bad, unknown_op, calls) =
            with_watchdog(Duration::from_secs(2), "opc.store list_blog_posts", move || {
                let first = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "list_blog_posts",
                        "args": { "limit": 20, "offset": 40 },
                    }),
                    &[],
                );
                // 缺省形态：两个键都不给（宿主按契约取缺省 limit 与 offset=0）。
                let second = peer.handle_call_seam(
                    &json!({ "seam": "opc.store", "op": "list_blog_posts", "args": {} }),
                    &[],
                );
                // 非数字：必须是参数错误，且**不得**触达 provider。
                let bad = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "list_blog_posts",
                        "args": { "limit": "二十" },
                    }),
                    &[],
                );
                // 表外 op 不许被兜底接住（`list_landing_pages` 自 #36 批 1 起已是支持 op，
                // 不能再当探针 —— 用永不可能注册的名字）。
                let unknown_op = peer.handle_call_seam(
                    &json!({ "seam": "opc.store", "op": "list_not_a_real_op", "args": {} }),
                    &[],
                );
                let calls = captured.calls.lock().clone();
                (first, second, bad, unknown_op, calls)
            });

        assert_eq!(first.kind, ResponseKind::Success, "第一次调用应成功：{first:?}");
        let value = first.value.expect("成功帧必须带 value");
        let mut keys: Vec<String> =
            value.as_object().expect("出参应是具名对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["posts", "total"], "出参是 {{posts, total}}，不是裸数组：{value}");
        assert_eq!(value["total"], json!(137));
        assert_eq!(value["posts"], json!([]), "空页是空数组，不是错误");

        assert_eq!(second.kind, ResponseKind::Success, "缺省形态应成功：{second:?}");
        assert_eq!(
            bad.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "非数字 limit 必须落参数错误：{bad:?}"
        );
        assert_eq!(
            unknown_op.code.as_deref(),
            Some(error_codes::SEAM_CALL_UNSUPPORTED),
            "表外 op 必须明确拒，不许落进别的分支：{unknown_op:?}"
        );

        // 前两次原样进门；第三次被拒 ⇒ 零触达；第四次被拒为「不支持」⇒ 同样零触达。
        assert_eq!(calls, vec![(Some(20), Some(40)), (None, None)], "两次合法调用应原样透传");
    }

    /// ② 未覆盖任何方法的 provider ⇒ 默认 `Err(Provider)` 落成 `PLUGIN_CALL_FAILED`
    /// （与 `analysis.record` / `trajectory.store` 同一条链路）。
    #[test]
    fn opc_store_unimplemented_provider_maps_to_call_failed() {
        let (peer, _runtime) = opc_store_peer(Arc::new(BareOpcStore));
        let response = with_watchdog(Duration::from_secs(2), "opc.store 未实现", move || {
            peer.handle_call_seam(
                &json!({ "seam": "opc.store", "op": "list_blog_posts", "args": {} }),
                &[],
            )
        });
        assert_eq!(response.kind, ResponseKind::Error);
        assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_CALL_FAILED));
    }

    /// `opc.store` 的权限表**逐条枚举锁**（全量写 op + 全量读 op + 两侧的反向锁）：
    /// blog 三件 + #36 批 2 四条 + #36 批 3 八条 + D-4 三条 + #32 批 A·B 六条 +
    /// #264 批 1 三条 + 批 2 五条 + 批 3 三条 + 批 6 三条 + 批 7 刀 A 两条（订阅存 / 删）
    /// ⇒ `Write`；读 op（列表 / 报表 / 详情 / 策略 / 摘要 / 能力清单 / 缺口列表 /
    /// 订阅表 / 域包配置与运行时域包）⇒ **必须仍是 `None`**；
    /// 表外 op 与近似接缝名 ⇒ `None`（不许被通配符或前缀匹配接住）。
    ///
    /// 为什么读缝那条要专门钉：`required_permission_for` 的函数头承诺「只登记写 op」，
    /// 这条承诺唯一的机械防线就是本用例 —— 多登记一条会把只读插件（今天
    /// `opc_list_*` 那几条命名命令）无端拒成 `PLUGIN_PERMISSION_DENIED`。
    #[test]
    fn opc_store_permission_table_gates_every_registered_write_op() {
        const WRITE_OPS: &[&str] = &[
            "publish_blog_post",
            "delete_blog_post",
            "create_blog_post",
            "transition_invoice",
            "update_content_asset",
            "delete_content_asset",
            "cancel_publish_schedule",
            "create_invoice",
            "delete_invoice",
            "create_customer",
            "update_customer",
            "delete_customer",
            "create_project",
            "add_milestone",
            "create_landing_page",
            "create_content_asset",
            "create_publish_schedule",
            "process_due_schedules",
            "update_project",
            "delete_project",
            "complete_milestone",
            "publish_landing_page",
            "delete_landing_page",
            "mark_contact_read",
            "create_invoice_from_lead",
            "update_delivery_invoice_status",
            "delete_delivery_invoice",
            "save_platform",
            "delete_platform",
            "save_scan_policy",
            "discover_and_evaluate_leads",
            "create_lead",
            "update_lead_status",
            "convert_lead_to_workflow",
            "run_lead_workflow",
            "execute_demand_workflow",
            "update_delivery",
            "retry_delivery",
            "cancel_delivery",
            "proactive_evaluate_leads",
            "close_capability_gap",
            "analyze_capability_gaps",
            // #264 批 7 刀 A：订阅面两条写 op
            "save_subscription",
            "delete_subscription",
            // #264 批 7 刀 B2：域包单据面六条写 op（更新发票 / 记录 KPI / 导入人才库 /
            // 同步舰队 / 更新发布计划 / 动态工作流执行）。
            "update_invoice",
            "record_kpi",
            "import_talent_library",
            "sync_fleet",
            "update_publish_schedule",
            "execute_dynamic_workflow",
            // #264 批 7 刀 B3：域包规则触发（写 op）；同批三条域包读 op 在 READ_OPS。
            "run_automation_rules",
            // #264 批 7 刀 B3（后三条）：订阅扫描落库 + 幂等建/改调度任务。
            "run_subscription_scan",
            "ensure_demand_scan_job",
            // 刀 B3 收尾：自动学习（写 RL 经验 + 反思/进化）。
            "auto_learn_workflow",
            // #264 刀 C 批 3：学习面六条写 op（反思 / 进化 / 自我改进 / 学习闭环 /
            // 手动 RL 优化 / 手动记录经验）。
            "reflect_on_workflow",
            "evolve_workflow",
            "run_self_improvement",
            "trigger_capability_pack_learning",
            "trigger_rl_optimization",
            "record_rl_experience",
            // #264 批 4a：工作项六条写 op（建 / 认领 / 提交评审 / 升级阻塞 / 解除阻塞 /
            // 自改进循环）。
            "create_work_item",
            "work_item_start",
            "work_item_review",
            "escalate_work_item",
            "work_item_unblock",
            "run_self_improving_work_item",
            // #264 批 4b：域包与内容资产三条写 op（导出落盘 / 导入解包注册 / 启停写库）。
            "export_capability_pack",
            "import_capability_pack",
            "set_capability_pack_enabled",
        ];
        const READ_OPS: &[&str] = &[
            "list_blog_posts",
            "list_invoices",
            "list_customers",
            "list_projects",
            "list_landing_pages",
            "list_contacts",
            "list_kpis",
            "list_content_assets",
            "list_publish_schedules",
            "get_financial_report",
            "get_project",
            "list_delivery_invoices",
            "delivery_summary",
            "list_platforms",
            "list_leads",
            "get_scan_policy",
            "list_domain_workflows",
            "get_domain_workflow",
            "list_domains",
            "list_deliveries",
            "get_delivery",
            "scan_capabilities",
            "list_capability_gaps",
            // #264 批 7 刀 A：订阅面一条 + 域包配置注册表五条 + 运行时域包三条，全只读。
            "list_subscriptions",
            "list_industries",
            "get_capability_pack_config",
            "get_action_config",
            "list_capability_pack_actions",
            "list_capability_pack_workflows",
            "list_runtime_industries",
            "get_kpi_definitions",
            "has_capability_pack",
            // #264 批 7 刀 B1：域包全读七条（薄层透传），全只读 ⇒ 写分支一条不加。
            "get_workflow_config",
            "validate_entity",
            "batch_validate_entities",
            "compute_kpis",
            "get_capability_pack_data",
            "get_capability_pack_precheck",
            "get_capability_pack_health",
            // #264 批 7 刀 B2：域包单据面六条读 op（取发票 / 取客户 / 按邮箱查客户 /
            // 列出收入 / 取投资建议 / 取发布计划详情）—— 全只读，写分支一条不加。
            "get_invoice",
            "get_customer",
            "find_customer_by_email",
            "list_revenue",
            "get_investment_advice",
            "get_publish_schedule",
            // #264 批 7 刀 B3：域包自动规则 / 仪表盘三条读 op —— 全只读，写分支一条不加。
            "capability_pack_dashboard",
            "capability_pack_workflow_steps",
            "capability_pack_automation_rules",
            // #264 刀 C 批 1：域包 manifest / prompt 装配 + 学习面五条读 op —— 同样零写闸。
            "capability_pack_manifest",
            "build_capability_pack_prompt",
            "get_learning_metrics",
            "get_learning_config",
            "list_learning_configs",
            // #264 刀 C 批 2：域包分析执行 —— 读 op（写 `execute_dynamic_workflow` 已在写集）。
            "execute_analysis",
            // #264 刀 C 批 3：经验池统计 —— 读 op（同批六条写 op 在写集）。
            "get_rl_stats",
            // #264 批 4a：看板投影 —— 读 op（同批六条写 op 在写集）。
            "kanban_board",
            // #264 批 4b：内容资产全文 + 市场域包列表 —— 读 op（同批三条写 op 在写集）。
            "get_content_asset",
            "market_list",
        ];
        for write_op in WRITE_OPS {
            assert_eq!(
                required_permission_for("opc.store", write_op),
                Some(PluginPermission::Write),
                "写 op `{write_op}` 必须要求 write"
            );
        }
        for read_op in READ_OPS {
            assert_eq!(
                required_permission_for("opc.store", read_op),
                None,
                "读缝一条都不许加闸：加了一条 = 只读插件被拒（{read_op}）"
            );
        }
        // 未登记的 op 名（含将来才有的 op）一律落 `_ => None`，不许被通配接住。
        for unregistered in ["publish", "blog_publish", "delete", "create", "remove_blog_post", ""]
        {
            assert_eq!(
                required_permission_for("opc.store", unregistered),
                None,
                "未登记的 op 必须无闸：{unregistered}"
            );
        }
        // 接缝名逐字：`opc` 少一段、`opc.stores` 多一个字符都不算命中。
        for write_op in WRITE_OPS {
            assert_eq!(required_permission_for("opc", write_op), None);
            assert_eq!(required_permission_for("opc.stores", write_op), None);
        }
    }

    /// D-4 三条写 op 的跨层口径（一条用例串起来）：
    /// ① 无 `write` ⇒ 三条全被拒且**零触达**；
    /// ② `publish_landing_page` 纯 `id` 进门，出参是发布后的摘要视图（camelCase 线格式）；
    /// ③ 两条无载荷写（删落地页 / 联系已读）成功出参是 `Value::Null`，`id` 逐字进门。
    #[test]
    fn opc_store_d4_writes_gate_and_return_views() {
        // ① 无 write：三条都被拒，替身一条都没收到。
        let denied = Arc::new(FixedOpcStore::new());
        let (peer, _runtime) = opc_store_peer_with_permissions(denied.clone(), Vec::new());
        let denied_responses =
            with_watchdog(Duration::from_secs(5), "opc.store D-4 无权限", move || {
                ["publish_landing_page", "delete_landing_page", "mark_contact_read"]
                    .into_iter()
                    .map(|op| {
                        peer.handle_call_seam(
                            &json!({ "seam": "opc.store", "op": op, "args": { "id": "lp-1" } }),
                            &[],
                        )
                    })
                    .collect::<Vec<_>>()
            });
        for (index, response) in denied_responses.iter().enumerate() {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::PLUGIN_PERMISSION_DENIED),
                "第 {} 条写 op 无 write 必须被拒：{response:?}",
                index
            );
        }
        assert!(
            denied.landing_publish_calls.lock().is_empty()
                && denied.landing_delete_calls.lock().is_empty()
                && denied.contact_read_calls.lock().is_empty(),
            "被拒时不得触达 provider"
        );

        // ②③ 有 write：`id` 逐字进门；发布出参是摘要视图，另两条成功无载荷。
        let store = Arc::new(FixedOpcStore::new());
        let (peer, _runtime) =
            opc_store_peer_with_permissions(store.clone(), vec![PluginPermission::Write]);
        let (publish, delete, read) =
            with_watchdog(Duration::from_secs(5), "opc.store D-4 有权限", move || {
                let publish = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "publish_landing_page",
                        "args": { "id": "lp-1" },
                    }),
                    &[],
                );
                let delete = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "delete_landing_page",
                        "args": { "id": "lp-1" },
                    }),
                    &[],
                );
                let read = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "mark_contact_read",
                        "args": { "id": "ct-1" },
                    }),
                    &[],
                );
                (publish, delete, read)
            });

        assert_eq!(publish.kind, ResponseKind::Success, "发布应成功：{publish:?}");
        let publish_value = publish.value.expect("成功帧必须带 value");
        assert_eq!(publish_value["id"], json!("lp-1"), "视图逐字回显 id：{publish_value}");
        assert_eq!(publish_value["published"], json!(true), "发布后 published 已翻转");
        assert_eq!(
            store.landing_publish_calls.lock().clone(),
            vec!["lp-1".to_string()],
            "id 必须逐字进门"
        );

        assert_eq!(delete.kind, ResponseKind::Success, "删除应成功：{delete:?}");
        assert_eq!(delete.value, Some(Value::Null), "无载荷写成功出参是 Value::Null");
        assert_eq!(store.landing_delete_calls.lock().clone(), vec!["lp-1".to_string()]);

        assert_eq!(read.kind, ResponseKind::Success, "标记已读应成功：{read:?}");
        assert_eq!(read.value, Some(Value::Null), "无载荷写成功出参是 Value::Null");
        assert_eq!(store.contact_read_calls.lock().clone(), vec!["ct-1".to_string()]);
    }

    /// #36 批 2 四条写 op 的跨层口径（一条用例串起来）：
    /// ① 无 `write` ⇒ 四条全被拒且**零触达**；
    /// ② `transition_invoice` 的两个散标量逐字进门，出参是 `{"transitioned": {…}}`；
    /// ③ `update_content_asset` 的 `request` 嵌层 + camelCase 线格式能反序列化；
    /// ④ 两条删除出参是 snake_case 枚举串 `"removed"`，缺行则 `"not_found"` 走 payload。
    #[test]
    fn opc_store_batch2_writes_gate_and_return_marked_outcomes() {
        let requests = [
            ("transition_invoice", json!({ "id": "inv-1", "target": "sent" })),
            ("update_content_asset", json!({ "id": "asset-1", "request": {} })),
            ("delete_content_asset", json!({ "id": "asset-1" })),
            ("cancel_publish_schedule", json!({ "id": "sched-1" })),
        ];

        // ① 无 write：四条都被拒，替身一条都没收到。
        let denied = Arc::new(FixedOpcStore::new());
        let (peer, _runtime) = opc_store_peer_with_permissions(denied.clone(), Vec::new());
        let denied_responses =
            with_watchdog(Duration::from_secs(5), "opc.store 批2 无权限", move || {
                requests
                    .into_iter()
                    .map(|(op, args)| {
                        peer.handle_call_seam(
                            &json!({ "seam": "opc.store", "op": op, "args": args }),
                            &[],
                        )
                    })
                    .collect::<Vec<_>>()
            });
        for (index, response) in denied_responses.iter().enumerate() {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::PLUGIN_PERMISSION_DENIED),
                "第 {} 条写 op 无 write 必须被拒：{response:?}",
                index
            );
        }
        assert!(
            denied.transition_calls.lock().is_empty()
                && denied.update_calls.lock().is_empty()
                && denied.delete_asset_calls.lock().is_empty()
                && denied.cancel_schedule_calls.lock().is_empty(),
            "被拒时不得触达 provider"
        );

        // ②③④ 有 write：入参逐字进门，出参是外部标记枚举。
        let store = Arc::new(FixedOpcStore::new());
        let (peer, _runtime) =
            opc_store_peer_with_permissions(store.clone(), vec![PluginPermission::Write]);
        let (transition, update, delete, cancel) =
            with_watchdog(Duration::from_secs(5), "opc.store 批2 有权限", move || {
                let transition = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "transition_invoice",
                        "args": { "id": "inv-1", "target": "sent" },
                    }),
                    &[],
                );
                let update = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "update_content_asset",
                        "args": {
                            "id": "asset-1",
                            "request": { "title": "新标题", "contentType": "video", "tags": ["a"] },
                        },
                    }),
                    &[],
                );
                let delete = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "delete_content_asset",
                        "args": { "id": "asset-1" },
                    }),
                    &[],
                );
                let cancel = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "cancel_publish_schedule",
                        "args": { "id": "sched-1" },
                    }),
                    &[],
                );
                (transition, update, delete, cancel)
            });

        assert_eq!(transition.kind, ResponseKind::Success, "状态变更应成功：{transition:?}");
        let transition_value = transition.value.expect("成功帧必须带 value");
        assert_eq!(
            transition_value["transitioned"]["invoiceNumber"],
            json!("INV-001"),
            "出参是带数据的外部标记枚举：{transition_value}"
        );
        assert_eq!(
            store.transition_calls.lock().clone(),
            vec![("inv-1".to_string(), "sent".to_string())],
            "id / target 必须逐字进门"
        );

        assert_eq!(update.kind, ResponseKind::Success, "内容资产更新应成功：{update:?}");
        let update_value = update.value.expect("成功帧必须带 value");
        assert_eq!(
            update_value["updated"]["contentType"],
            json!("article"),
            "出参是带数据的外部标记枚举：{update_value}"
        );
        let update_calls = store.update_calls.lock().clone();
        assert_eq!(update_calls.len(), 1, "更新只应触达一次");
        assert_eq!(update_calls[0].0, "asset-1", "id 必须逐字进门");
        assert_eq!(update_calls[0].1.title.as_deref(), Some("新标题"));
        // camelCase 线格式 → Rust 字段（禁区 13）：`contentType` 落进 `content_type`。
        assert_eq!(update_calls[0].1.content_type.as_deref(), Some("video"));
        assert_eq!(update_calls[0].1.tags.as_deref(), Some(&["a".to_string()][..]));

        assert_eq!(delete.kind, ResponseKind::Success, "删除应成功：{delete:?}");
        assert_eq!(delete.value, Some(json!("removed")), "两态出参是 snake_case 枚举串");
        assert_eq!(store.delete_asset_calls.lock().clone(), vec!["asset-1".to_string()]);

        assert_eq!(cancel.kind, ResponseKind::Success, "取消应成功：{cancel:?}");
        assert_eq!(cancel.value, Some(json!("removed")), "两态出参是 snake_case 枚举串");
        assert_eq!(store.cancel_schedule_calls.lock().clone(), vec!["sched-1".to_string()]);
    }

    /// 批 2 四条写 op 的缺行口径：`ResponseKind::Success` + payload `"not_found"`，
    /// 四条一致 —— 缺行是正常业务读数，**不占错误码**。
    #[test]
    fn opc_store_batch2_missing_rows_are_payload_not_errors() {
        let store = Arc::new(FixedOpcStore::batch2_missing());
        let (peer, _runtime) =
            opc_store_peer_with_permissions(store, vec![PluginPermission::Write]);
        let (transition, update, delete, cancel) =
            with_watchdog(Duration::from_secs(5), "opc.store 批2 缺行", move || {
                let transition = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "transition_invoice",
                        "args": { "id": "inv-x", "target": "sent" },
                    }),
                    &[],
                );
                let update = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "update_content_asset",
                        "args": { "id": "asset-x", "request": {} },
                    }),
                    &[],
                );
                let delete = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "delete_content_asset",
                        "args": { "id": "asset-x" },
                    }),
                    &[],
                );
                let cancel = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "cancel_publish_schedule",
                        "args": { "id": "sched-x" },
                    }),
                    &[],
                );
                (transition, update, delete, cancel)
            });

        for (label, response) in [
            ("transition_invoice", transition),
            ("update_content_asset", update),
            ("delete_content_asset", delete),
            ("cancel_publish_schedule", cancel),
        ] {
            assert_eq!(response.kind, ResponseKind::Success, "`{label}` 缺行应成功：{response:?}");
            assert_eq!(response.code, None, "`{label}` 缺行不得占错误码");
            assert_eq!(response.value, Some(json!("not_found")), "`{label}` 应回 not_found");
        }
    }

    /// 写 op 的四条跨层口径（一条用例串起来，因为它们互为反例）：
    /// ① 无 `write` ⇒ `PLUGIN_PERMISSION_DENIED`，且**零触达 provider**（闸在取 provider /
    ///    反序列化之前）；
    /// ② 有 `write` ⇒ `id` 逐字进门，出参是契约枚举的 snake_case 串 `"published"`；
    /// ③ 有 `write` 但缺 `id` ⇒ `SEAM_CALL_INVALID_ARGS`（权限过了不等于参数能缺），零触达；
    /// ④ 缺行 ⇒ `ResponseKind::Success` + payload `"not_found"`，**不占错误码**。
    #[test]
    fn opc_store_publish_gate_denies_before_provider_and_returns_two_state_payload() {
        // ① 无 write：被拒，且替身一条都没收到。
        let denied_store = Arc::new(FixedOpcStore::new());
        let captured_denied = Arc::clone(&denied_store);
        // `with_watchdog` 要 `'static` 闭包 ⇒ peer 移进闭包（闭包体只引用它），
        // 替身的快照留在外面，拒绝时刻之后才取得到。
        let (denied_peer, _denied_runtime) =
            opc_store_peer_with_permissions(denied_store, Vec::new());
        let denied =
            with_watchdog(Duration::from_secs(2), "opc.store publish 无权限", move || {
                denied_peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "publish_blog_post",
                        "args": { "id": "post-1" },
                    }),
                    &[],
                )
            });
        assert_eq!(
            denied.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 权限的 publish 必须被拒，实际 {denied:?}"
        );
        let detail = denied.message.clone().unwrap_or_default();
        assert!(detail.contains("write"), "文案应点明缺失的权限，实际：{detail}");
        assert!(detail.contains("opc.store"), "文案应点明接缝，实际：{detail}");
        assert!(
            captured_denied.publish_calls.lock().is_empty(),
            "被拒的写 op 不得触达 provider（闸在取 provider 之前）"
        );

        // ② + ③ 有 write：id 逐字进门、出参 `"published"`；缺 id 落参数错误且不再触达。
        let store = Arc::new(FixedOpcStore::new());
        let captured = Arc::clone(&store);
        let (peer, _runtime) =
            opc_store_peer_with_permissions(store, vec![PluginPermission::Write]);
        let (ok, missing_id) =
            with_watchdog(Duration::from_secs(2), "opc.store publish 有权限", move || {
                let ok = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "publish_blog_post",
                        "args": { "id": "post-1" },
                    }),
                    &[],
                );
                let missing_id = peer.handle_call_seam(
                    &json!({ "seam": "opc.store", "op": "publish_blog_post", "args": {} }),
                    &[],
                );
                (ok, missing_id)
            });
        assert_eq!(ok.kind, ResponseKind::Success, "有 write 的 publish 应成功：{ok:?}");
        assert_eq!(
            ok.value,
            Some(json!("published")),
            "出参是契约枚举的 snake_case 串，不是对象、不是布尔"
        );
        assert_eq!(captured.publish_calls.lock().clone(), vec!["post-1".to_string()]);
        assert_eq!(
            missing_id.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `id` 必须是参数错误：{missing_id:?}"
        );
        assert_eq!(
            captured.publish_calls.lock().len(),
            1,
            "缺参那次不得触达 provider（两次调用只进门一次）"
        );

        // ④ 缺行：Success + `"not_found"`，没有错误码。
        let (absent_peer, _absent_runtime) = opc_store_peer_with_permissions(
            Arc::new(FixedOpcStore::missing()),
            vec![PluginPermission::Write],
        );
        let absent = with_watchdog(Duration::from_secs(2), "opc.store publish 缺行", move || {
            absent_peer.handle_call_seam(
                &json!({
                    "seam": "opc.store",
                    "op": "publish_blog_post",
                    "args": { "id": "post-absent" },
                }),
                &[],
            )
        });
        assert_eq!(
            absent.kind,
            ResponseKind::Success,
            "缺行是正常结果，不许落成错误帧：{absent:?}"
        );
        assert_eq!(absent.code, None, "成功帧不该带错误码：{absent:?}");
        assert_eq!(absent.value, Some(json!("not_found")));
    }

    /// 未覆盖任何方法的 provider（`BareOpcStore`）走写 op ⇒ 默认 `Err(Provider)` 落成
    /// `PLUGIN_CALL_FAILED` —— 与读 op 同一条链路；「未实现」不许被升成参数错误，
    /// 也不许被伪装成 `"not_found"`（那会把「宿主没接这条 op」读成「这篇不存在」）。
    #[test]
    fn opc_store_unimplemented_publish_maps_to_call_failed() {
        let (peer, _runtime) =
            opc_store_peer_with_permissions(Arc::new(BareOpcStore), vec![PluginPermission::Write]);
        let response =
            with_watchdog(Duration::from_secs(2), "opc.store publish 未实现", move || {
                peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "publish_blog_post",
                        "args": { "id": "post-1" },
                    }),
                    &[],
                )
            });
        assert_eq!(response.kind, ResponseKind::Error);
        assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_CALL_FAILED));
    }

    /// `opc.store` 的**删除**写 op（#180-①）四条跨层口径：
    /// ① 无 `write` ⇒ `PLUGIN_PERMISSION_DENIED` 且**零触达** provider（闸在取 provider 之前）；
    /// ② 有 `write` ⇒ `id` 逐字进门，出参是契约枚举的 snake_case 串 `"removed"`；
    /// ③ 有 `write` 但缺 `id` ⇒ `SEAM_CALL_INVALID_ARGS`，零触达（权限过了不等于参数能缺）；
    /// ④ `"not_found"` / `"protected"` 都是 `ResponseKind::Success` + payload，
    ///    **不占错误码** —— 「本来就没有这篇」与「这篇不许删」是两种业务读数。
    #[test]
    fn opc_store_delete_gate_denies_before_provider_and_returns_three_state_payload() {
        // ① 无 write：被拒，替身一条都没收到。
        let denied_store = Arc::new(FixedOpcStore::new());
        let captured_denied = Arc::clone(&denied_store);
        let (denied_peer, _denied_runtime) =
            opc_store_peer_with_permissions(denied_store, Vec::new());
        let denied =
            with_watchdog(Duration::from_secs(2), "opc.store delete 无权限", move || {
                denied_peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "delete_blog_post",
                        "args": { "id": "post-1" },
                    }),
                    &[],
                )
            });
        assert_eq!(
            denied.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 权限的删除必须被拒，实际 {denied:?}"
        );
        assert!(
            captured_denied.delete_calls.lock().is_empty(),
            "被拒的写 op 不得触达 provider（闸在取 provider 之前）"
        );

        // ② + ③ 有 write：id 逐字进门、出参 `"removed"`；缺 id 落参数错误且不再触达。
        let store = Arc::new(FixedOpcStore::new());
        let captured = Arc::clone(&store);
        let (peer, _runtime) =
            opc_store_peer_with_permissions(store, vec![PluginPermission::Write]);
        let (removed, missing_id) =
            with_watchdog(Duration::from_secs(2), "opc.store delete 有权限", move || {
                let removed = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "delete_blog_post",
                        "args": { "id": "post-1" },
                    }),
                    &[],
                );
                let missing_id = peer.handle_call_seam(
                    &json!({ "seam": "opc.store", "op": "delete_blog_post", "args": {} }),
                    &[],
                );
                (removed, missing_id)
            });
        assert_eq!(removed.kind, ResponseKind::Success, "有 write 的删除应成功：{removed:?}");
        assert_eq!(
            removed.value,
            Some(json!("removed")),
            "出参是契约枚举的 snake_case 串，不建包装对象"
        );
        assert_eq!(captured.delete_calls.lock().clone(), vec!["post-1".to_string()]);
        assert_eq!(
            missing_id.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `id` 必须是参数错误：{missing_id:?}"
        );
        assert_eq!(
            captured.delete_calls.lock().len(),
            1,
            "缺参那次不得触达 provider（两次调用只进门一次）"
        );

        // ④ NotFound / Protected：都走 Success+payload，且取值逐字。
        for (store, want) in
            [(FixedOpcStore::missing(), "not_found"), (FixedOpcStore::protected(), "protected")]
        {
            let (absent_peer, _absent_runtime) =
                opc_store_peer_with_permissions(Arc::new(store), vec![PluginPermission::Write]);
            let response =
                with_watchdog(Duration::from_secs(2), "opc.store delete 三态", move || {
                    absent_peer.handle_call_seam(
                        &json!({
                            "seam": "opc.store",
                            "op": "delete_blog_post",
                            "args": { "id": "post-absent" },
                        }),
                        &[],
                    )
                });
            assert_eq!(
                response.kind,
                ResponseKind::Success,
                "`{want}` 是正常结果，不许落成错误帧：{response:?}"
            );
            assert_eq!(response.code, None, "成功帧不该带错误码：{response:?}");
            assert_eq!(response.value, Some(json!(want)), "三态取值必须逐字透出");
        }
    }

    /// `opc.store` 的**创建**写 op（#180-②）四条跨层口径：
    /// ① 无 `write` ⇒ 被拒且零触达；② 有 `write` ⇒ 入参**嵌一层 `request`**、五个字段逐字
    /// 进门，出参是列裁剪视图（10 个 camelCase 键，无 `content`）；③ 平铺（缺 `request` 键）
    /// ⇒ `SEAM_CALL_INVALID_ARGS` 且零触达；④ provider 回 `Validation`（正文超预算 / 空 slug）
    /// ⇒ `SEAM_CALL_INVALID_ARGS`（与分派层自己的参数错误同码，插件侧只认一个码）。
    #[test]
    fn opc_store_create_requires_write_and_forwards_nested_request() {
        // ① 无 write：被拒 + 零触达。
        let denied_store = Arc::new(FixedOpcStore::new());
        let captured_denied = Arc::clone(&denied_store);
        let (denied_peer, _denied_runtime) =
            opc_store_peer_with_permissions(denied_store, Vec::new());
        let denied =
            with_watchdog(Duration::from_secs(2), "opc.store create 无权限", move || {
                denied_peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "create_blog_post",
                        "args": { "request": { "title": "标题", "slug": "s", "excerpt": "摘要",
                                               "content": "正文", "tags": [] } },
                    }),
                    &[],
                )
            });
        assert_eq!(
            denied.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 权限的创建必须被拒，实际 {denied:?}"
        );
        assert!(
            captured_denied.create_calls.lock().is_empty(),
            "被拒的写 op 不得触达 provider（闸在取 provider 之前）"
        );

        // ② 有 write：DTO 逐字进门；出参是视图。③ 平铺形态必须被拒且零触达。
        let store = Arc::new(FixedOpcStore::new());
        let captured = Arc::clone(&store);
        let (peer, _runtime) =
            opc_store_peer_with_permissions(store, vec![PluginPermission::Write]);
        let (ok, flat) =
            with_watchdog(Duration::from_secs(2), "opc.store create 有权限", move || {
                let ok = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "create_blog_post",
                        "args": { "request": {
                            "title": "第一篇", "slug": "first-post", "excerpt": "摘要",
                            "content": "正文", "tags": ["opc", "blog"],
                        } },
                    }),
                    &[],
                );
                // 平铺形态（把 DTO 的字段直接摊在 args 上）：缺 `request` 键 ⇒ 参数错误。
                let flat = peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "create_blog_post",
                        "args": { "title": "第一篇", "slug": "first-post", "excerpt": "摘要",
                                  "content": "正文", "tags": [] },
                    }),
                    &[],
                );
                (ok, flat)
            });

        assert_eq!(ok.kind, ResponseKind::Success, "有 write 的创建应成功：{ok:?}");
        let view = ok.value.expect("成功帧必须带 value");
        let mut keys: Vec<String> =
            view.as_object().expect("出参应是行对象").keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "createdAt",
                "excerpt",
                "id",
                "published",
                "publishedAt",
                "slug",
                "tags",
                "title",
                "updatedAt",
                "viewCount",
            ],
            "出参是接缝的列裁剪视图（10 个 camelCase 键）：{view}"
        );
        assert_eq!(view["slug"], json!("first-post"), "视图逐字来自 provider");
        assert_eq!(view["id"], json!("post-new"), "创建必须回后端给的 id");
        assert!(view.get("content").is_none(), "正文不过线");

        let recorded = captured.create_calls.lock().clone();
        assert_eq!(recorded.len(), 1, "平铺那次不得触达 provider（缺 request 键）");
        assert_eq!(recorded[0].title, "第一篇");
        assert_eq!(recorded[0].slug, "first-post");
        assert_eq!(recorded[0].excerpt, "摘要");
        assert_eq!(recorded[0].content, "正文", "正文必须逐字进门（字节闸在 provider 侧）");
        assert_eq!(recorded[0].tags, vec!["opc".to_string(), "blog".to_string()]);

        assert_eq!(
            flat.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "平铺形态必须被拒（嵌一层 `request` 是唯一受理形态）：{flat:?}"
        );

        // ④ provider 回 Validation ⇒ 与分派层参数错误同码。
        let (reject_peer, _reject_runtime) = opc_store_peer_with_permissions(
            Arc::new(FixedOpcStore::rejecting_create("content 越出契约上限")),
            vec![PluginPermission::Write],
        );
        let rejected =
            with_watchdog(Duration::from_secs(2), "opc.store create 超预算", move || {
                reject_peer.handle_call_seam(
                    &json!({
                        "seam": "opc.store",
                        "op": "create_blog_post",
                        "args": { "request": { "title": "t", "slug": "s", "excerpt": "e",
                                               "content": "c", "tags": [] } },
                    }),
                    &[],
                )
            });
        assert_eq!(
            rejected.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "宿主拒收入参（正文超预算）必须落 SEAM_CALL_INVALID_ARGS：{rejected:?}"
        );
        assert!(
            rejected.message.as_deref().is_some_and(|m| m.contains("契约上限")),
            "错误文案应原样透出 provider 的说明：{rejected:?}"
        );
    }

    /// 两个新写 op 在**未实现** provider 上必须落 `PLUGIN_CALL_FAILED`（与 publish 同一条链路）：
    /// 「宿主没接这条 op」不许被升成参数错误，也不许被伪装成 `not_found` / 空视图。
    #[test]
    fn opc_store_unimplemented_delete_and_create_map_to_call_failed() {
        for (op, args) in [
            ("delete_blog_post", json!({ "id": "post-1" })),
            (
                "create_blog_post",
                json!({ "request": { "title": "t", "slug": "s", "excerpt": "e", "content": "c",
                                     "tags": [] } }),
            ),
        ] {
            let (peer, _runtime) = opc_store_peer_with_permissions(
                Arc::new(BareOpcStore),
                vec![PluginPermission::Write],
            );
            let response =
                with_watchdog(Duration::from_secs(2), "opc.store 新写 op 未实现", move || {
                    peer.handle_call_seam(
                        &json!({ "seam": "opc.store", "op": op, "args": args }),
                        &[],
                    )
                });
            assert_eq!(response.kind, ResponseKind::Error, "未实现必须失败：{op}");
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::PLUGIN_CALL_FAILED),
                "未实现必须是 PLUGIN_CALL_FAILED（op={op}）：{response:?}"
            );
        }
    }

    /// ③ + ④（写 op）：无 `write` 权限 ⇒ `PLUGIN_PERMISSION_DENIED` 且**零触达**；
    /// 有 `write` ⇒ 嵌一层的 camelCase `request` 原样落进 `VectorIndexRequest` 的
    /// snake_case 字段，响应是 `VectorIndexOutcome`（`chunkCount` / `documentId` /
    /// `collectionId` 三键，无 snake_case 残留）。
    #[test]
    fn knowledge_index_text_requires_write_and_forwards_nested_request() {
        // 反面：权限闸在取 provider / 反序列化之前拒绝（零副作用）。
        let denied_knowledge = Arc::new(FixedKnowledgeVector::new());
        let captured_denied = Arc::clone(&denied_knowledge);
        let (denied_peer, _denied_runtime) = knowledge_trajectory_peer(
            denied_knowledge,
            Arc::new(FixedTrajectoryStore::new()),
            Vec::new(),
        );
        let inside = Arc::clone(&captured_denied);
        let (denied, touched) =
            with_watchdog(Duration::from_secs(2), "index_text 无权限", move || {
                let denied = denied_peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "index_text",
                        "args": {
                            "request": {
                                "collectionId": "kb-1",
                                "documentId": "doc-1",
                                "content": "正文",
                            },
                        },
                    }),
                    &[],
                );
                let touched = denied_index_calls(&inside);
                (denied, touched)
            });
        assert_eq!(
            denied.code.as_deref(),
            Some(error_codes::PLUGIN_PERMISSION_DENIED),
            "无 write 权限的 index_text 必须被拒，实际 {denied:?}"
        );
        let detail = denied.message.clone().unwrap_or_default();
        assert!(detail.contains("write"), "文案应点明缺失的权限，实际：{detail}");
        assert!(detail.contains("knowledge.vector"), "文案应点明接缝，实际：{detail}");
        assert_eq!(touched, 0, "被拒的写 op 不得触达 provider");

        // 正面：带 write 权限 + 嵌一层 camelCase。
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let captured = Arc::clone(&knowledge);
        let (peer, _runtime) = knowledge_trajectory_peer(
            knowledge,
            Arc::new(FixedTrajectoryStore::new()),
            vec![PluginPermission::Write],
        );
        let response = with_watchdog(Duration::from_secs(5), "index_text 正例", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "knowledge.vector",
                    "op": "index_text",
                    "args": {
                        "request": {
                            "collectionId": "kb-1",
                            "documentId": "doc-1",
                            "content": "贵州茅台三季度营收……",
                        },
                    },
                }),
                &[],
            )
        });
        assert_eq!(response.code, None, "已授权且参数齐备应成功，实际 {response:?}");
        let calls = captured.index_calls.lock().clone();
        assert_eq!(calls.len(), 1, "index_text 应恰被调用一次");
        assert_eq!(calls[0].collection_id, "kb-1", "camelCase `collectionId` 应落 snake 字段");
        assert_eq!(calls[0].document_id, "doc-1");
        assert_eq!(calls[0].content, "贵州茅台三季度营收……");
        assert_eq!(
            response.value,
            Some(json!({
                "chunkCount": 7,
                "documentId": "doc-1",
                "collectionId": "kb-1",
            })),
            "必须直接透出 `VectorIndexOutcome`，不再自己包一层"
        );
        let value = response.value.clone().unwrap_or_default();
        assert!(value.get("chunk_count").is_none(), "不得残留 snake_case 键");
    }

    /// ④ + ⑤（search）：camelCase 入参落 `VectorSearchRequest` 的 snake_case 字段；
    /// 两个可选字段整体省略 ⇒ `None`（不是「阈值 0」/「空集合」）；响应 `VectorSearchOutcome`
    /// 的 `results` 是 camelCase 六键视图、`score` 按 f64 容差比；**命中 0 条 = 空数组不是错误**。
    ///
    /// ⚠ 浮点坑：`score` / `minSimilarity` 是 `f32`，不许与 `json!(0.35)` 逐位断言。
    #[test]
    fn knowledge_search_forwards_camel_request_and_empty_hit_is_not_error() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let captured = Arc::clone(&knowledge);
        let store = Arc::new(FixedTrajectoryStore::new());
        let (peer, _runtime) = knowledge_trajectory_peer(knowledge, store, Vec::new());
        let (ok, minimal, snake_input, empty) =
            with_watchdog(Duration::from_secs(5), "knowledge.search", move || {
                let ok = peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "search",
                        "args": {
                            "request": {
                                "knowledgeBaseId": "kb-1",
                                "query": "筹码集中度",
                                "topK": 8,
                                "minSimilarity": 0.35,
                                "docIds": ["doc-1", "doc-2"],
                            },
                        },
                    }),
                    &[],
                );
                // 两个可选字段整体省略是合法的（契约 `#[serde(default)]`），故此处只给三字段。
                let minimal = peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "search",
                        "args": {
                            "request": {
                                "knowledgeBaseId": "kb-1",
                                "query": "主力净流入",
                                "topK": 5,
                            },
                        },
                    }),
                    &[],
                );
                // snake_case 入参必须**不被接受**（DTO 是 camelCase，无别名兼容）。
                let snake_input = peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "search",
                        "args": {
                            "request": {
                                "knowledge_base_id": "kb-1",
                                "query": "筹码集中度",
                                "top_k": 8,
                            },
                        },
                    }),
                    &[],
                );
                let empty = peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "search",
                        "args": {
                            "request": {
                                "knowledgeBaseId": "kb-1",
                                "query": "无命中",
                                "topK": 8,
                            },
                        },
                    }),
                    &[],
                );
                (ok, minimal, snake_input, empty)
            });
        assert_eq!(ok.code, None, "只读 op 不得被权限闸拦，实际 {ok:?}");
        let calls = captured.search_calls.lock().clone();
        assert_eq!(calls.len(), 3, "三次合法调用应触达 provider（snake 反例不算）");
        assert_eq!(calls[0].knowledge_base_id, "kb-1");
        assert_eq!(calls[0].top_k, 8);
        let min_similarity = calls[0].min_similarity.expect("camelCase minSimilarity 应落字段");
        assert!(
            (min_similarity - 0.35f32).abs() < 1e-6,
            "minSimilarity 应≈ 0.35，实际 {min_similarity}"
        );
        assert_eq!(calls[0].doc_ids.clone().unwrap_or_default().len(), 2);
        assert!(calls[1].min_similarity.is_none(), "省略可选字段必须是 None（不是阈值 0）");
        assert!(calls[1].doc_ids.is_none(), "省略 docIds 必须是 None（不是空集合）");
        assert_eq!(minimal.code, None, "两个可选字段整体省略是合法入参，实际 {minimal:?}");
        assert_eq!(
            minimal.value.clone().expect("省略可选字段仍要带结果")["results"]
                .as_array()
                .expect("results 应为数组")
                .len(),
            1,
            "缺省可选字段不得改变结果形态"
        );

        let value = ok.value.clone().expect("成功响应必须带 value");
        assert!(value.get("results").is_some(), "必须直接透出 Outcome 的 `results`");
        let hits = value["results"].as_array().expect("results 应为数组").clone();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["documentId"], json!("doc-1"));
        assert_eq!(hits[0]["chunkIndex"], json!(3));
        assert_eq!(hits[0]["hasEmbedding"], json!(true));
        assert_eq!(hits[0]["id"], json!("chunk-1"));
        let score = hits[0]["score"].as_f64().expect("score 应为数值");
        assert!((score - 0.21).abs() < 1e-6, "score 应≈ 0.21，实际 {score}");
        assert!(hits[0].get("document_id").is_none(), "不得残留 snake_case 键");
        assert!(hits[0].get("chunk_index").is_none());
        assert!(hits[0].get("has_embedding").is_none());
        assert!(hits[0].get("content").is_none(), "整块原文不过线");
        assert!(hits[0].get("embedding").is_none(), "向量本体不过线");

        assert_eq!(
            snake_input.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "snake_case 入参键不被兼容，实际 {snake_input:?}"
        );
        assert_eq!(captured.search_calls.lock().len(), 3, "snake 反例不得触达 provider");

        assert_eq!(empty.code, None, "命中 0 条不是错误，实际 {empty:?}");
        assert_eq!(empty.value, Some(json!({ "results": [] })), "查无结果 = 空数组");
    }

    /// ④ + op 名 ≠ 方法名：`get_collection_info` 走标量键 `collection_id`，落到 trait 的
    /// `collection_info()`；集合不存在 ⇒ `{exists:false, dimensions:null}` **不是错误**。
    /// 反面：缺标量键 ⇒ `SEAM_CALL_INVALID_ARGS`。
    #[test]
    fn knowledge_collection_info_absent_collection_is_not_error() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let captured = Arc::clone(&knowledge);
        let store = Arc::new(FixedTrajectoryStore::new());
        let (peer, _runtime) = knowledge_trajectory_peer(knowledge, store, Vec::new());
        let inside = Arc::clone(&captured);
        let (ok, missing, rejected_calls) =
            with_watchdog(Duration::from_secs(5), "get_collection_info", move || {
                let ok = peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "get_collection_info",
                        "args": { "collection_id": "kb-x" },
                    }),
                    &[],
                );
                let missing = peer.handle_call_seam(
                    &json!({ "seam": "knowledge.vector", "op": "get_collection_info", "args": {} }),
                    &[],
                );
                let rejected_calls = inside.collection_info_calls.lock().len();
                (ok, missing, rejected_calls)
            });
        assert_eq!(ok.code, None, "集合不存在是正常响应，实际 {ok:?}");
        assert_eq!(
            ok.value,
            Some(json!({ "collectionId": "kb-x", "dimensions": null, "exists": false })),
            "三键 camelCase 透出 `CollectionInfoView`"
        );
        assert_eq!(
            captured.collection_info_calls.lock().clone(),
            vec!["kb-x".to_string()],
            "op `get_collection_info` 必须落到 trait 方法 `collection_info()`"
        );
        assert_eq!(
            missing.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `collection_id` 必须走参数错误，实际 {missing:?}"
        );
        assert_eq!(rejected_calls, 1, "参数错误不得新增 provider 触达");
    }

    /// ⑥：**反面锁**。替身未覆盖的 op 吃 trait 默认 `Err(Provider("…未实现…"))` ⇒ 分派层
    /// 一律 `PLUGIN_CALL_FAILED`（`seam_provider_error` 只动 `Validation` 那一格，其余变体
    /// 不升级）。文案点名 trait 方法名与「未实现」。同批钉住：`list_collections`（PLAN
    /// §56.1-Q3 已裁定首批不开）仍是 `SEAM_CALL_UNSUPPORTED`，两类「做不到」不许混成一个码。
    #[test]
    fn knowledge_keyword_search_default_unimplemented_is_call_failed() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let captured = Arc::clone(&knowledge);
        let store = Arc::new(FixedTrajectoryStore::new());
        let (peer, _runtime) = knowledge_trajectory_peer(knowledge, store, Vec::new());
        let (unimplemented, covered_ok) =
            with_watchdog(Duration::from_secs(5), "keyword_search 默认实现", move || {
                let unimplemented = peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "keyword_search",
                        "args": {
                            "request": { "collectionId": "kb-1", "query": "主力净流入", "topK": 8 },
                        },
                    }),
                    &[],
                );
                let covered_ok = peer.handle_call_seam(
                    &json!({ "seam": "knowledge.vector", "op": "list_collections", "args": {} }),
                    &[],
                );
                (unimplemented, covered_ok)
            });
        assert_eq!(
            unimplemented.code.as_deref(),
            Some(error_codes::PLUGIN_CALL_FAILED),
            "trait 默认 Err(Provider) 应落成 PLUGIN_CALL_FAILED，实际 {unimplemented:?}"
        );
        let detail = unimplemented.message.clone().unwrap_or_default();
        assert!(detail.contains("未实现"), "文案应说明未实现，实际：{detail}");
        assert!(
            detail.contains("KnowledgeVectorProvider::keyword_search"),
            "文案应点名 trait 方法，实际：{detail}"
        );
        // 合法的 keyword_search 入参也不该被「猜」到别的 op 上：默认实现被真调到了。
        assert!(captured.search_calls.lock().is_empty(), "未覆盖的 op 不得转投别的方法");
        assert_eq!(
            covered_ok.code.as_deref(),
            Some(error_codes::SEAM_CALL_UNSUPPORTED),
            "同接缝的未知 op 仍是 SEAM_CALL_UNSUPPORTED，实际 {covered_ok:?}"
        );
    }

    /// ⑤ + ④（trajectory 读面三格）：`list` 的 `limit` **可选**（缺失 = `None` 合法、
    /// 非数字 = 参数错误）；`get` 查无 ⇒ 帧里 `null` 且 `code` 为空；`count` 的 `0` 是合法值；
    /// 摘要视图 12 键 camelCase（含嵌套 `quality` 的五键），无 snake_case 残留、无原文字段。
    #[test]
    fn trajectory_store_list_get_count_shapes_are_conform() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let store = Arc::new(FixedTrajectoryStore::new());
        let captured = Arc::clone(&store);
        let (peer, _runtime) = knowledge_trajectory_peer(knowledge, store, Vec::new());
        let inside = Arc::clone(&captured);
        let (no_limit, with_limit, bad_limit, hit, absent, no_id, counted, list_calls, get_calls) =
            with_watchdog(Duration::from_secs(5), "trajectory.store 读面", move || {
                let no_limit = peer.handle_call_seam(
                    &json!({ "seam": "trajectory.store", "op": "list", "args": {} }),
                    &[],
                );
                let with_limit = peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "list",
                        "args": { "limit": 20 },
                    }),
                    &[],
                );
                let bad_limit = peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "list",
                        "args": { "limit": "20" },
                    }),
                    &[],
                );
                // 拒绝时刻取快照：两次 bad/missing 之后仍应只有上面两次合法调用。
                let list_calls = inside.list_calls.lock().len();
                let hit = peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "get",
                        "args": { "trajectory_id": "traj-1" },
                    }),
                    &[],
                );
                let absent = peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "get",
                        "args": { "trajectory_id": "traj-404" },
                    }),
                    &[],
                );
                let no_id = peer.handle_call_seam(
                    &json!({ "seam": "trajectory.store", "op": "get", "args": {} }),
                    &[],
                );
                let get_calls = inside.get_calls.lock().len();
                let counted = peer.handle_call_seam(
                    &json!({ "seam": "trajectory.store", "op": "count", "args": {} }),
                    &[],
                );
                (
                    no_limit, with_limit, bad_limit, hit, absent, no_id, counted, list_calls,
                    get_calls,
                )
            });
        assert_eq!(no_limit.code, None, "`limit` 缺失是合法的「由宿主取缺省」，实际 {no_limit:?}");
        assert_eq!(with_limit.code, None, "带 `limit` 应成功，实际 {with_limit:?}");
        assert_eq!(
            captured.list_calls.lock().clone(),
            vec![None, Some(20)],
            "`limit` 必须原样透传（本层不钳位、不写数字）"
        );
        assert_eq!(list_calls, 2, "非数字 `limit` 不得触达 provider");
        assert_eq!(
            bad_limit.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "`limit` 传字符串必须落参数错误，实际 {bad_limit:?}"
        );
        let value = no_limit.value.clone().expect("list 必须带具名对象");
        assert!(value.get("trajectories").is_some(), "必须透出 TrajectoryListOutcome，不返裸数组");
        assert_eq!(value["trajectories"].as_array().expect("应为数组").len(), 1);
        assert_eq!(value["trajectories"][0]["sessionId"], json!("sess-1"));
        assert_eq!(value["trajectories"][0]["durationMs"], json!(4200));
        assert_eq!(value["trajectories"][0]["createdAtMs"], json!(1_700_000_000_000_i64));
        assert_eq!(value["trajectories"][0]["replayCount"], json!(3));
        assert_eq!(value["trajectories"][0]["outcome"], json!("success"));
        let value_score = value["trajectories"][0]["valueScore"].as_f64().expect("应为数值");
        assert!((value_score - 0.62).abs() < 1e-6, "valueScore 应≈ 0.62，实际 {value_score}");
        for bad in ["session_id", "summary_snippet", "duration_ms", "value_score", "created_at_ms"]
        {
            assert!(value["trajectories"][0].get(bad).is_none(), "不得残留 snake_case 键 {bad}");
        }
        for forbidden in ["steps", "rewards", "patterns", "content", "reasoning", "summary"] {
            assert!(
                value["trajectories"][0].get(forbidden).is_none(),
                "{forbidden} 属大文本/会话原文，不过线"
            );
        }

        assert_eq!(hit.code, None, "命中应成功，实际 {hit:?}");
        assert_eq!(hit.value.clone().expect("命中带视图")["id"], json!("traj-1"));
        assert_eq!(absent.code, None, "查无此轨迹**不是错误**，实际 {absent:?}");
        assert_eq!(absent.value, Some(Value::Null), "`Ok(None)` ⇒ 帧里是 null");
        assert_eq!(get_calls, 2, "缺 `trajectory_id` 不得触达 provider");
        assert_eq!(
            no_id.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "缺 `trajectory_id` 必须走参数错误，实际 {no_id:?}"
        );
        assert_eq!(counted.code, None, "空库的 0 是合法值，实际 {counted:?}");
        assert_eq!(counted.value, Some(json!({ "count": 0 })), "具名对象，不返裸值");
        assert_eq!(*captured.count_calls.lock(), 1);
    }

    /// ④（query）：嵌一层的 camelCase `request` 落 `TrajectoryQueryView` 的 snake_case 字段
    /// （`sinceMs` / `untilMs` 是 epoch 毫秒，不是 `time_range` 元组）；响应透出
    /// `TrajectoryQueryOutcome`。反面：未知 `outcome` 字面量 ⇒ 参数错误、零触达。
    #[test]
    fn trajectory_store_query_forwards_nested_camel_request() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let store = Arc::new(FixedTrajectoryStore::new());
        let captured = Arc::clone(&store);
        let (peer, _runtime) = knowledge_trajectory_peer(knowledge, store, Vec::new());
        let inside = Arc::clone(&captured);
        let (ok, bad_outcome, rejected_calls) =
            with_watchdog(Duration::from_secs(5), "trajectory.query", move || {
                let ok = peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "query",
                        "args": {
                            "request": {
                                "sessionId": "sess-1",
                                "outcome": "partial",
                                "minQuality": 0.7,
                                "sinceMs": 1_600_000_000_000_i64,
                                "untilMs": 1_700_000_000_000_i64,
                                "limit": 20,
                            },
                        },
                    }),
                    &[],
                );
                let bad_outcome = peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "query",
                        "args": { "request": { "outcome": "weird" } },
                    }),
                    &[],
                );
                let rejected_calls = inside.query_calls.lock().len();
                (ok, bad_outcome, rejected_calls)
            });
        assert_eq!(ok.code, None, "合法 query 应成功，实际 {ok:?}");
        let calls = captured.query_calls.lock().clone();
        assert_eq!(calls.len(), 1, "query 应恰被调用一次");
        assert_eq!(calls[0].session_id.as_deref(), Some("sess-1"));
        assert_eq!(
            calls[0].outcome,
            Some(axagent_harness::trajectory_types::TrajectoryOutcome::Partial),
            "camelCase 小写字面量 `partial` 应落进既有 harness 枚举"
        );
        assert_eq!(calls[0].since_ms, Some(1_600_000_000_000));
        assert_eq!(calls[0].until_ms, Some(1_700_000_000_000));
        assert_eq!(calls[0].limit, Some(20));
        assert_eq!(calls[0].user_id, None, "省略的可选字段必须是 None");
        let min_quality = calls[0].min_quality.expect("camelCase minQuality 应落字段");
        assert!((min_quality - 0.7).abs() < 1e-6, "minQuality 应≈ 0.7，实际 {min_quality}");
        let value = ok.value.clone().expect("query 必须带具名对象");
        assert!(value.get("trajectories").is_some(), "透出 TrajectoryQueryOutcome");
        assert_eq!(value["trajectories"][0]["agentName"], json!("股票分析师"));
        assert!(value["trajectories"][0].get("agent_name").is_none(), "不得残留 snake_case 键");

        assert_eq!(
            bad_outcome.code.as_deref(),
            Some(error_codes::SEAM_CALL_INVALID_ARGS),
            "未知 `outcome` 字面量必须拒（不继承命令层的静默回落），实际 {bad_outcome:?}"
        );
        assert_eq!(rejected_calls, 1, "被拒的 query 不得触达 provider");
    }

    /// ⑥（trajectory 侧）：一个方法都不覆盖的实现 ⇒ 四个 op 全吃 trait 默认 `Err(Provider)`，
    /// 分派层一律 `PLUGIN_CALL_FAILED` + 「未实现」，**不**借用 NOT_FOUND / INVALID_ARGS。
    /// 正面同批钉：同接缝的未知 op（`append`，PLAN §56.1-Q4 明确不给）落 UNSUPPORTED。
    #[test]
    fn trajectory_bare_store_default_unimplemented_is_call_failed() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let (peer, _runtime) =
            knowledge_trajectory_peer(knowledge, Arc::new(BareTrajectoryStore), Vec::new());
        let responses = with_watchdog(Duration::from_secs(5), "轨迹库默认实现", move || {
            [
                ("list", json!({})),
                ("get", json!({ "trajectory_id": "traj-1" })),
                ("count", json!({})),
                ("append", json!({})),
            ]
            .map(|(op, args)| {
                peer.handle_call_seam(
                    &json!({ "seam": "trajectory.store", "op": op, "args": args }),
                    &[],
                )
            })
        });
        for (index, op) in ["list", "get", "count"].into_iter().enumerate() {
            let response = &responses[index];
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::PLUGIN_CALL_FAILED),
                "默认未实现的 `{op}` 应落 PLUGIN_CALL_FAILED，实际 {response:?}"
            );
            let detail = response.message.clone().unwrap_or_default();
            assert!(detail.contains("未实现"), "`{op}` 文案应说明未实现，实际：{detail}");
            assert!(
                detail.contains(&format!("TrajectoryStoreProvider::{op}")),
                "`{op}` 文案应点名 trait 方法，实际：{detail}"
            );
        }
        assert_eq!(
            responses[3].code.as_deref(),
            Some(error_codes::SEAM_CALL_UNSUPPORTED),
            "`append` 不在契约里，必须落 UNSUPPORTED 而不是「未实现」，实际 {:?}",
            responses[3]
        );
    }

    /// ⑦：已知接缝 + 契约里没有的 op ⇒ `SEAM_CALL_UNSUPPORTED`（不猜、不兜底到别的方法）。
    /// 两个反例都取自被明确砍掉的写面：`delete_document`（PLAN §56.1-Q2 不开）、
    /// `append`（§56.1-Q4 不开）。
    #[test]
    fn knowledge_trajectory_unknown_ops_are_unsupported() {
        let knowledge = Arc::new(FixedKnowledgeVector::new());
        let captured_knowledge = Arc::clone(&knowledge);
        let store = Arc::new(FixedTrajectoryStore::new());
        let captured_store = Arc::clone(&store);
        let (peer, _runtime) = knowledge_trajectory_peer(knowledge, store, Vec::new());
        let inside_knowledge = Arc::clone(&captured_knowledge);
        let inside_store = Arc::clone(&captured_store);
        let responses = with_watchdog(Duration::from_secs(2), "未知 op", move || {
            let responses =
                [("knowledge.vector", "delete_document"), ("trajectory.store", "remove")].map(
                    |(seam, op)| {
                        peer.handle_call_seam(&json!({ "seam": seam, "op": op, "args": {} }), &[])
                    },
                );
            (
                responses,
                inside_knowledge.search_calls.lock().len(),
                inside_store.get_calls.lock().len(),
            )
        });
        for response in &responses.0 {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::SEAM_CALL_UNSUPPORTED),
                "已知接缝的未知 op 必须落 SEAM_CALL_UNSUPPORTED，实际 {response:?}"
            );
        }
        assert_eq!((responses.1, responses.2), (0, 0), "未知 op 不得触达 provider");
    }

    /// ⑧：注册表取不到 provider ⇒ `SEAM_PROVIDER_UNAVAILABLE`（明确失败、不挂起）。
    /// `test_host_peer` 的 `runtime: None` 同时证明**取 provider 失败先于 `block_on`**
    /// （无 runtime 也拿不到 `PLUGIN_CALL_FAILED`），且带合法 args 也一样 —— 拒因是接缝没装配。
    #[test]
    fn knowledge_trajectory_unregistered_provider_is_unavailable() {
        let empty_registry = Arc::new(CapabilityRegistry::new());
        let peer = test_host_peer(
            "plugin.none",
            &[],
            &["knowledge.vector", "trajectory.store"],
            empty_registry,
        );
        let responses = with_watchdog(Duration::from_secs(2), "provider 缺位", move || {
            let knowledge = peer.handle_call_seam(
                &json!({
                    "seam": "knowledge.vector",
                    "op": "get_collection_info",
                    "args": { "collection_id": "kb-1" },
                }),
                &[],
            );
            let trajectory = peer.handle_call_seam(
                &json!({ "seam": "trajectory.store", "op": "count", "args": {} }),
                &[],
            );
            (knowledge, trajectory)
        });
        for (seam, response) in
            [("knowledge.vector", &responses.0), ("trajectory.store", &responses.1)]
        {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::SEAM_PROVIDER_UNAVAILABLE),
                "`{seam}` 未注册 provider 必须落 SEAM_PROVIDER_UNAVAILABLE，实际 {response:?}"
            );
        }
    }

    /// 写 op 被权限闸拒掉时的「零触达」取证助手（避免在闭包内散落锁读取）。
    fn denied_index_calls(provider: &Arc<FixedKnowledgeVector>) -> usize {
        provider.index_calls.lock().len()
    }

    // ─────────── 任务 #112：Validation ⇒ SEAM_CALL_INVALID_ARGS 单点映射 ───────────

    /// 替身回**哪一类**错误由这一格控制，好让「该升码的那一支」和「不许升码的其余变体」
    /// 在同一条接缝、同一个 op、同一句 reason 上做严格 A/B —— 唯一变量是错误变体。
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum StubReject {
        /// 契约里「宿主拒收入参」的表达 ⇒ 必须落 `SEAM_CALL_INVALID_ARGS`。
        Validation,
        /// 宿主内部故障 ⇒ **不许**被误升成参数错误，必须仍 `PLUGIN_CALL_FAILED`。
        Internal,
    }

    impl StubReject {
        /// 造错点唯一：两批用例共用同一句 reason，断言才能证明「变的只有码，不是文案」。
        fn err(self, reason: &str) -> AxAgentError {
            match self {
                Self::Validation => AxAgentError::Validation(reason.to_string()),
                Self::Internal => AxAgentError::Internal(reason.to_string()),
            }
        }
    }

    /// 会拒收入参的 `stock.monitor` 替身：三个越界写 op 一律回 [`StubReject`] 指定的错。
    ///
    /// **刻意不复用 `FixedMonitorProvider`**：那批用例钉的是「成功透传」，往同一替身塞条件
    /// 分支会改掉它们的语义。`status` / `list_configs` 等不覆盖 ⇒ 吃 trait 默认
    /// `Err(Provider("…未实现…"))`，顺带当 ④ 反面锁的素材。
    struct RejectingMonitorProvider {
        reject: Mutex<StubReject>,
        t0_calls: Mutex<Vec<axagent_harness::TZeroConfig>>,
        upsert_calls: Mutex<Vec<axagent_harness::AlertUpsertRequest>>,
        aggregator_calls: Mutex<Vec<axagent_harness::AggregatorConfig>>,
    }

    impl RejectingMonitorProvider {
        fn new(reject: StubReject) -> Self {
            Self {
                reject: Mutex::new(reject),
                t0_calls: Mutex::new(Vec::new()),
                upsert_calls: Mutex::new(Vec::new()),
                aggregator_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl axagent_harness::StockMonitorProvider for RejectingMonitorProvider {
        async fn set_t0_config(
            &self,
            config: axagent_harness::TZeroConfig,
        ) -> axagent_harness::core_error::Result<()> {
            self.t0_calls.lock().push(config);
            Err(self.reject.lock().err("阈值 / 引擎节拍越界：minIntervalMinutes=0"))
        }

        async fn upsert_alert(
            &self,
            request: axagent_harness::AlertUpsertRequest,
        ) -> axagent_harness::core_error::Result<axagent_harness::AlertUpsertOutcome> {
            self.upsert_calls.lock().push(request);
            Err(self.reject.lock().err("alert_type 不在白名单：`stop_lostt`"))
        }

        async fn set_aggregator_config(
            &self,
            config: axagent_harness::AggregatorConfig,
        ) -> axagent_harness::core_error::Result<()> {
            self.aggregator_calls.lock().push(config);
            Err(self.reject.lock().err("聚合四区间越界：minSignalCount=0"))
        }
    }

    /// 会拒收入参的 `quote.watcher` 替身：`watch` / `unwatch` 回指定错；
    /// `watched_codes` 不覆盖 ⇒ 吃 trait 默认 `Err(Provider)`。
    struct RejectingWatcherProvider {
        reject: Mutex<StubReject>,
        watch_calls: Mutex<usize>,
        unwatch_calls: Mutex<usize>,
    }

    impl RejectingWatcherProvider {
        fn new(reject: StubReject) -> Self {
            Self {
                reject: Mutex::new(reject),
                watch_calls: Mutex::new(0),
                unwatch_calls: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl axagent_harness::QuoteWatcherProvider for RejectingWatcherProvider {
        async fn watch(
            &self,
            _codes: &[String],
            _priority: axagent_harness::WatchPriority,
            _replace: bool,
        ) -> axagent_harness::core_error::Result<axagent_harness::WatchOutcome> {
            // 计数记**调用次数**而不是 codes 长度：空集合那一格也要能证明真触达了 provider。
            *self.watch_calls.lock() += 1;
            Err(self.reject.lock().err("订阅集合越界：codes 为空"))
        }

        async fn unwatch(
            &self,
            _codes: &[String],
        ) -> axagent_harness::core_error::Result<axagent_harness::WatchOutcome> {
            *self.unwatch_calls.lock() += 1;
            Err(self.reject.lock().err("退订集合越界：codes 为空"))
        }
    }

    /// 会拒收入参的 `knowledge.vector` 替身：`search`（`top_k` 越界）与 `index_text`
    /// （集合不存在 / 正文超 `KNOWLEDGE_CONTENT_MAX_BYTES`）回指定错；
    /// `keyword_search` / `collection_info` 不覆盖 ⇒ 吃 trait 默认 `Err(Provider)`。
    struct RejectingKnowledgeVector {
        reject: Mutex<StubReject>,
        search_calls: Mutex<Vec<axagent_harness::VectorSearchRequest>>,
        index_calls: Mutex<Vec<axagent_harness::VectorIndexRequest>>,
    }

    impl RejectingKnowledgeVector {
        fn new(reject: StubReject) -> Self {
            Self {
                reject: Mutex::new(reject),
                search_calls: Mutex::new(Vec::new()),
                index_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl axagent_harness::KnowledgeVectorProvider for RejectingKnowledgeVector {
        async fn search(
            &self,
            request: axagent_harness::VectorSearchRequest,
        ) -> axagent_harness::core_error::Result<axagent_harness::VectorSearchOutcome> {
            self.search_calls.lock().push(request.clone());
            Err(self.reject.lock().err("top_k 越界：0 不在 KNOWLEDGE_TOP_K_RANGE"))
        }

        async fn index_text(
            &self,
            request: axagent_harness::VectorIndexRequest,
        ) -> axagent_harness::core_error::Result<axagent_harness::VectorIndexOutcome> {
            self.index_calls.lock().push(request.clone());
            Err(self.reject.lock().err("目标集合不存在：`kb-missing`（禁止隐式建集合）"))
        }
    }

    /// 会拒收入参的 `trajectory.store` 替身：`query`（半边时间窗 / `untilMs < sinceMs`）与
    /// `list`（`limit` 越界）回指定错；`get` / `count` 不覆盖 ⇒ 吃 trait 默认 `Err(Provider)`。
    struct RejectingTrajectoryStore {
        reject: Mutex<StubReject>,
        query_calls: Mutex<Vec<axagent_harness::TrajectoryQueryView>>,
        list_calls: Mutex<Vec<Option<usize>>>,
    }

    impl RejectingTrajectoryStore {
        fn new(reject: StubReject) -> Self {
            Self {
                reject: Mutex::new(reject),
                query_calls: Mutex::new(Vec::new()),
                list_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl axagent_harness::TrajectoryStoreProvider for RejectingTrajectoryStore {
        async fn query(
            &self,
            filter: axagent_harness::TrajectoryQueryView,
        ) -> axagent_harness::core_error::Result<axagent_harness::TrajectoryQueryOutcome> {
            self.query_calls.lock().push(filter);
            Err(self.reject.lock().err("半边时间窗：给了 untilMs 却没给 sinceMs"))
        }

        async fn list(
            &self,
            limit: Option<usize>,
        ) -> axagent_harness::core_error::Result<axagent_harness::TrajectoryListOutcome> {
            self.list_calls.lock().push(limit);
            Err(self.reject.lock().err("limit 越界：9999999 超 TRAJECTORY_LIST_LIMIT_RANGE"))
        }
    }

    /// 四条接缝共用一个 peer：四个 provider 一次装齐，`declared_calls` 同批给全
    /// （少声明一条就先吃 `SEAM_CALL_NOT_DECLARED`，根本到不了分派分支）。
    /// 形态照 `monitor_peer_with_permissions` / `knowledge_trajectory_peer`。
    fn seam_error_peer(
        monitor: Arc<RejectingMonitorProvider>,
        watcher: Arc<RejectingWatcherProvider>,
        knowledge: Arc<RejectingKnowledgeVector>,
        store: Arc<RejectingTrajectoryStore>,
        permissions: Vec<PluginPermission>,
    ) -> (HostPeer, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("async 接缝用例需要 runtime");
        let registry = Arc::new(CapabilityRegistry::new());
        let provider: Arc<dyn axagent_harness::StockMonitorProvider> = monitor;
        registry.register_stock_monitor(provider).expect("stock.monitor 接缝应注册成功");
        let provider: Arc<dyn axagent_harness::QuoteWatcherProvider> = watcher;
        registry.register_quote_watcher(provider).expect("quote.watcher 接缝应注册成功");
        let provider: Arc<dyn axagent_harness::KnowledgeVectorProvider> = knowledge;
        registry.register_knowledge_vector(provider).expect("knowledge.vector 接缝应注册成功");
        let provider: Arc<dyn axagent_harness::TrajectoryStoreProvider> = store;
        registry.register_trajectory_store(provider).expect("trajectory.store 接缝应注册成功");
        let peer = HostPeer {
            plugin_id: "plugin.seam_error".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec![
                "stock.monitor".to_string(),
                "quote.watcher".to_string(),
                "knowledge.vector".to_string(),
                "trajectory.store".to_string(),
            ],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions,
        };
        (peer, runtime)
    }

    /// ①②③：四条接缝的越界入参由 provider 回 `Err(Validation)`，分派层必须落成
    /// `SEAM_CALL_INVALID_ARGS`（改前一律 `PLUGIN_CALL_FAILED` ⇒ 契约那几句是假话）。
    ///
    /// 断言两件事：帧码升对了 **且** 文案是 `Validation` 的**内层** reason（不带 `Display`
    /// 前缀 `Validation error:`）—— 后者才证明走的是新映射、不是 `other.to_string()` 兜底。
    #[test]
    fn seam_validation_maps_to_invalid_args_across_four_seams() {
        let monitor = Arc::new(RejectingMonitorProvider::new(StubReject::Validation));
        let watcher = Arc::new(RejectingWatcherProvider::new(StubReject::Validation));
        let knowledge = Arc::new(RejectingKnowledgeVector::new(StubReject::Validation));
        let store = Arc::new(RejectingTrajectoryStore::new(StubReject::Validation));
        let (peer, _runtime) = seam_error_peer(
            Arc::clone(&monitor),
            Arc::clone(&watcher),
            Arc::clone(&knowledge),
            Arc::clone(&store),
            vec![PluginPermission::Write],
        );
        let responses =
            with_watchdog(Duration::from_secs(5), "Validation 升码九格", move || {
                vec![
                peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "set_t0_config",
                        "args": {
                            "config": {
                                "enabled": true,
                                "changePctThreshold": 3.0,
                                "minIntervalMinutes": 0,
                            },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "upsert_alert",
                        "args": {
                            "request": {
                                "stockCode": "600519",
                                "stockName": "贵州茅台",
                                "alertType": "stop_lostt",
                                "threshold": 1400.0,
                            },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "set_aggregator_config",
                        "args": {
                            "config": {
                                "windowSecs": 300,
                                "minSignalCount": 0,
                                "cooldownSecs": 600,
                                "minStrength": 0.3,
                            },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "watch",
                        "args": { "codes": [], "priority": "active" },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "unwatch",
                        "args": { "codes": [] },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "search",
                        "args": {
                            "request": { "knowledgeBaseId": "kb-1", "query": "筹码", "topK": 0 },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "index_text",
                        "args": {
                            "request": {
                                "collectionId": "kb-missing",
                                "documentId": "doc-1",
                                "content": "正文",
                            },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "query",
                        "args": { "request": { "untilMs": 1_700_000_000_000_i64 } },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "list",
                        "args": { "limit": 9_999_999 },
                    }),
                    &[],
                ),
            ]
            });
        let expected = [
            ("stock.monitor", "set_t0_config"),
            ("stock.monitor", "upsert_alert"),
            ("stock.monitor", "set_aggregator_config"),
            ("quote.watcher", "watch"),
            ("quote.watcher", "unwatch"),
            ("knowledge.vector", "search"),
            ("knowledge.vector", "index_text"),
            ("trajectory.store", "query"),
            ("trajectory.store", "list"),
        ];
        for ((seam, op), response) in expected.iter().zip(responses.iter()) {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::SEAM_CALL_INVALID_ARGS),
                "`{seam}`/`{op}` 的 Validation 必须升码为 SEAM_CALL_INVALID_ARGS，实际 {response:?}"
            );
            let detail = response.message.clone().unwrap_or_default();
            assert!(
                !detail.starts_with("Validation error:"),
                "`{seam}`/`{op}` 应透出内层 reason，不该带 Display 前缀，实际：{detail}"
            );
            assert!(!detail.is_empty(), "`{seam}`/`{op}` 的拒因文案不得被抹平");
        }
        // 九格都真触达了 provider（否则「升码」可能来自参数解析阶段的同码，取证会失真）。
        assert_eq!(responses.len(), expected.len(), "取证格数与断言表不符");
        assert_eq!(monitor.t0_calls.lock().len(), 1, "set_t0_config 应被转发");
        assert_eq!(monitor.upsert_calls.lock().len(), 1, "upsert_alert 应被转发");
        assert_eq!(monitor.aggregator_calls.lock().len(), 1, "set_aggregator_config 应被转发");
        assert_eq!(*watcher.watch_calls.lock(), 1, "空 codes 也应转发到 provider");
        assert_eq!(*watcher.unwatch_calls.lock(), 1, "unwatch 应被转发");
        assert_eq!(knowledge.search_calls.lock().len(), 1, "search 应被转发");
        assert_eq!(knowledge.index_calls.lock().len(), 1, "index_text 应被转发");
        assert_eq!(store.query_calls.lock().len(), 1, "query 应被转发");
        assert_eq!(store.list_calls.lock().len(), 1, "list 应被转发");
        // 越界数字的分寸归 provider，本层一个都不写（裁定 N4）：替身收到的就是原样入参。
        assert_eq!(monitor.t0_calls.lock()[0].min_interval_minutes, 0, "越界值不得被夹取");
        assert_eq!(store.list_calls.lock()[0], Some(9_999_999), "limit 越界值不得被夹取");
    }

    /// 映射规则的**另一半**：同一批 op、同一句造错入口，只要变体不是 `Validation`
    /// （这里用 `Internal`）就**不许**升码，必须仍 `PLUGIN_CALL_FAILED`，且走
    /// `other.to_string()` 兜底（文案带 `Display` 前缀）。
    #[test]
    fn seam_non_validation_error_stays_call_failed() {
        let (peer, _runtime) = seam_error_peer(
            Arc::new(RejectingMonitorProvider::new(StubReject::Internal)),
            Arc::new(RejectingWatcherProvider::new(StubReject::Internal)),
            Arc::new(RejectingKnowledgeVector::new(StubReject::Internal)),
            Arc::new(RejectingTrajectoryStore::new(StubReject::Internal)),
            vec![PluginPermission::Write],
        );
        let responses =
            with_watchdog(Duration::from_secs(5), "非 Validation 不升码", move || {
                vec![
                    peer.handle_call_seam(
                        &json!({
                            "seam": "stock.monitor",
                            "op": "upsert_alert",
                            "args": {
                                "request": {
                                    "stockCode": "600519",
                                    "stockName": "贵州茅台",
                                    "alertType": "stop_loss",
                                    "threshold": 1400.0,
                                },
                            },
                        }),
                        &[],
                    ),
                    peer.handle_call_seam(
                        &json!({
                            "seam": "knowledge.vector",
                            "op": "index_text",
                            "args": {
                                "request": {
                                    "collectionId": "kb-1",
                                    "documentId": "doc-1",
                                    "content": "正文",
                                },
                            },
                        }),
                        &[],
                    ),
                    peer.handle_call_seam(
                        &json!({
                            "seam": "trajectory.store",
                            "op": "query",
                            "args": {
                                "request": {
                                    "sinceMs": 1_700_000_000_000_i64,
                                    "untilMs": 1_600_000_000_000_i64,
                                },
                            },
                        }),
                        &[],
                    ),
                ]
            });
        assert_eq!(responses.len(), 3);
        for response in &responses {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::PLUGIN_CALL_FAILED),
                "非 Validation 变体不得升码，实际 {response:?}"
            );
            let detail = response.message.clone().unwrap_or_default();
            assert!(
                detail.starts_with("Internal error:"),
                "应走 `other.to_string()` 兜底（带 Display 前缀），实际：{detail}"
            );
        }
    }

    /// ④ **反面锁**：本批改的是错码映射，不许把 trait 默认实现的
    /// `Err(Provider("…未实现…"))` 一起升码 —— 必须仍 `PLUGIN_CALL_FAILED` 且文案含「未实现」。
    ///
    /// 选的四格都是**本批改过的那 20 行**里的分支（四条接缝各一格），直接证明
    /// 「只动 `Validation` 那一格」这条规则落在改过的面上。
    #[test]
    fn seam_trait_default_unimplemented_stays_call_failed() {
        let (peer, _runtime) = seam_error_peer(
            Arc::new(RejectingMonitorProvider::new(StubReject::Validation)),
            Arc::new(RejectingWatcherProvider::new(StubReject::Validation)),
            Arc::new(RejectingKnowledgeVector::new(StubReject::Validation)),
            Arc::new(RejectingTrajectoryStore::new(StubReject::Validation)),
            vec![PluginPermission::Write],
        );
        let responses =
            with_watchdog(Duration::from_secs(5), "默认未实现不升码", move || {
                vec![
                peer.handle_call_seam(
                    &json!({ "seam": "stock.monitor", "op": "list_configs", "args": {} }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({ "seam": "quote.watcher", "op": "list_watched", "args": {} }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "keyword_search",
                        "args": {
                            "request": { "collectionId": "kb-1", "query": "主力净流入", "topK": 8 },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "trajectory.store",
                        "op": "get",
                        "args": { "trajectory_id": "traj-1" },
                    }),
                    &[],
                ),
            ]
            });
        let expected = [
            ("stock.monitor", "list_configs"),
            ("quote.watcher", "list_watched"),
            ("knowledge.vector", "keyword_search"),
            ("trajectory.store", "get"),
        ];
        assert_eq!(responses.len(), expected.len(), "反面锁格数与断言表不符");
        for ((seam, op), response) in expected.iter().zip(responses.iter()) {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::PLUGIN_CALL_FAILED),
                "`{seam}`/`{op}` 的 trait 默认 Err(Provider) 必须仍 PLUGIN_CALL_FAILED，实际 {response:?}"
            );
            let detail = response.message.clone().unwrap_or_default();
            assert!(detail.contains("未实现"), "`{seam}`/`{op}` 文案应说明未实现，实际：{detail}");
        }
    }

    /// ⑥ **闸位锁**：权限闸必须在错码映射**之前**。无 `write` 权限时即使 provider 会回
    /// `Validation`，帧码也必须仍是 `PLUGIN_PERMISSION_DENIED` 且零触达 provider ——
    /// 不许因为本批改了错码，就把无权限调用放进 provider。
    #[test]
    fn seam_validation_mapping_still_behind_permission_gate() {
        let monitor = Arc::new(RejectingMonitorProvider::new(StubReject::Validation));
        let watcher = Arc::new(RejectingWatcherProvider::new(StubReject::Validation));
        let knowledge = Arc::new(RejectingKnowledgeVector::new(StubReject::Validation));
        let store = Arc::new(RejectingTrajectoryStore::new(StubReject::Validation));
        let (peer, _runtime) = seam_error_peer(
            Arc::clone(&monitor),
            Arc::clone(&watcher),
            Arc::clone(&knowledge),
            store,
            Vec::new(),
        );
        let responses = with_watchdog(Duration::from_secs(5), "闸在映射之前", move || {
            vec![
                peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "set_t0_config",
                        "args": {
                            "config": {
                                "enabled": true,
                                "changePctThreshold": 3.0,
                                "minIntervalMinutes": 0,
                            },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "upsert_alert",
                        "args": {
                            "request": {
                                "stockCode": "600519",
                                "stockName": "贵州茅台",
                                "alertType": "stop_lostt",
                                "threshold": 1400.0,
                            },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "stock.monitor",
                        "op": "set_aggregator_config",
                        "args": {
                            "config": {
                                "windowSecs": 300,
                                "minSignalCount": 0,
                                "cooldownSecs": 600,
                                "minStrength": 0.3,
                            },
                        },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "quote.watcher",
                        "op": "watch",
                        "args": { "codes": ["600519"], "priority": "active" },
                    }),
                    &[],
                ),
                peer.handle_call_seam(
                    &json!({
                        "seam": "knowledge.vector",
                        "op": "index_text",
                        "args": {
                            "request": {
                                "collectionId": "kb-missing",
                                "documentId": "doc-1",
                                "content": "正文",
                            },
                        },
                    }),
                    &[],
                ),
            ]
        });
        assert_eq!(responses.len(), 5);
        for response in &responses {
            assert_eq!(
                response.code.as_deref(),
                Some(error_codes::PLUGIN_PERMISSION_DENIED),
                "闸必须早于映射：无 write 权限不得落到任何参数码 / 故障码，实际 {response:?}"
            );
            assert!(
                response.message.clone().unwrap_or_default().contains("write"),
                "文案应点明缺失的权限，实际：{response:?}"
            );
        }
        // 闸在映射之前 ⇒ 五格都该零触达；一旦有触达，就说明升码路径绕过了准入。
        assert!(monitor.t0_calls.lock().is_empty(), "set_t0_config 不得触达 provider");
        assert!(monitor.upsert_calls.lock().is_empty(), "upsert_alert 不得触达 provider");
        assert!(
            monitor.aggregator_calls.lock().is_empty(),
            "set_aggregator_config 不得触达 provider"
        );
        assert_eq!(*watcher.watch_calls.lock(), 0, "watch 不得触达 provider");
        assert!(knowledge.index_calls.lock().is_empty(), "index_text 不得触达 provider");
    }

    /// `session.store` 是**宿主内部注入通道**：注册在案，但插件调用必须在取 provider 前被拒。
    ///
    /// 判据两点：① 错误码是 `SEAM_CALL_UNSUPPORTED`（不是「没有 provider」）；
    /// ② 拒绝路径零触达 —— 即使插件有 `write` 权限，被拒的 `set` 也不得落库。
    #[test]
    fn session_store_is_host_internal_and_rejects_plugin_calls() {
        use axagent_harness::SessionStateStore as _;

        let memory = Arc::new(axagent_harness::test_support::MemorySessionStateStore::new());
        let registry = Arc::new(CapabilityRegistry::new());
        registry
            .register_session_state_store(memory.clone())
            .expect("session.store 接缝应注册成功");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("session.store 用例需要 runtime");
        runtime.block_on(memory.set("k", "before", None)).expect("预置应成功");
        let peer = HostPeer {
            plugin_id: "plugin.session".to_string(),
            declared_capabilities: Vec::new(),
            declared_calls: vec!["session.store".to_string()],
            registry,
            runtime: Some(runtime.handle().clone()),
            permissions: vec![PluginPermission::Write],
        };
        let response = with_watchdog(Duration::from_secs(5), "session.store 拒绝", move || {
            peer.handle_call_seam(
                &json!({
                    "seam": "session.store",
                    "op": "set",
                    "args": { "key": "k", "value": "after" },
                }),
                &[],
            )
        });
        assert_eq!(
            response.code.as_deref(),
            Some(error_codes::SEAM_CALL_UNSUPPORTED),
            "宿主内部接缝必须显式拒绝，实际 {response:?}"
        );
        assert!(
            response.message.clone().unwrap_or_default().contains("宿主内部"),
            "文案要点明边界，实际：{response:?}"
        );
        assert_eq!(
            runtime.block_on(memory.get("k")).expect("读回应成功"),
            Some("before".to_string()),
            "被拒的写不得落库"
        );
    }
}
