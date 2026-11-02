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
use axagent_harness::workflow_types::{NodeKind, WorkflowTemplateData};
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
}

impl HostPeer {
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
            // `platform.adapter.{平台名}` 的 seam ID 是**动态**的，只能用前缀守卫。
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

/// 把 worker 接上 `EventDispatchBus`：宿主收到事件时转发给插件，并把插件裁决送回总线。
///
/// 插件因此可以**订阅并改写 / 拒绝**事件（`DispatchMode::Waterfall`），
/// 而无需与事件产生者互相认识 —— 这是 L1 解耦的全部来源（PLAN §15.3）。
struct WorkerEventSubscriber {
    plugin_id: String,
    invoker: Arc<dyn SeamInvoker>,
}

impl WorkerEventSubscriber {
    fn new(plugin_id: impl Into<String>, invoker: Arc<dyn SeamInvoker>) -> Self {
        Self { plugin_id: plugin_id.into(), invoker }
    }
}

#[async_trait]
impl EventSubscriber for WorkerEventSubscriber {
    async fn handle(&self, event: &DomainEvent) -> SubscriberVerdict {
        let args = match serde_json::to_value(event) {
            Ok(value) => value,
            Err(e) => {
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
            Ok(Ok(value)) => verdict_from_value(&value),
            Ok(Err(e)) => {
                tracing::warn!(%plugin_id, error = %e, "插件事件处理失败，按继续处理");
                SubscriberVerdict::Continue
            },
            Err(e) => {
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

        let mut loaded = Self { plugin_id, declaration, invoker, handles };
        if let Some(handle) = loaded.event_subscription(registry) {
            loaded.handles.push(handle);
        }
        Ok(loaded)
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
    fn event_subscription(&self, registry: &CapabilityRegistry) -> Option<EffectHandle> {
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
        let subscriber: Arc<dyn EventSubscriber> =
            Arc::new(WorkerEventSubscriber::new(self.plugin_id.clone(), self.invoker.clone()));
        Some(bus.subscribe(EventMatcher::any(), subscriber))
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
    use axagent_harness::{DispatchMode, EventCategory, EventDispatchBus};
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
    }

    impl FixedMarketDataProvider {
        fn new() -> Self {
            Self { klines_args: Mutex::new(None) }
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
        };
        let expected = peer.unavailable("market.quote");
        let response = with_watchdog(Duration::from_secs(2), "market.quote 缺 provider", move || {
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
        assert_eq!(
            invalid_args("x").code,
            Some(error_codes::SEAM_CALL_INVALID_ARGS.to_string())
        );
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
        let response = with_watchdog(Duration::from_secs(5), "market.quote get_klines", move || {
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
}
