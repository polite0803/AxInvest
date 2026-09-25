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
//! - 发起方写帧时**只持写锁**，等响应期间**不持任何锁** —— 否则会与读线程互等。
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

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    AXAGENT_PLUGIN_PROTO_VERSION, FrameRequest, FrameResponse, Inbound, MAX_SEAM_CALL_DEPTH,
    PluginDeclaration, ResponseKind, error_codes, ops, read_inbound, write_json_frame,
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
    #[error("插件 `{plugin_id}` 握手失败：{reason}")]
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
    #[error("与插件 `{plugin_id}` 的通道错误：{reason}")]
    Transport { plugin_id: String, reason: String },
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
                log.record_model_visible(&session_id, content);
                FrameResponse::success(Value::Null)
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

/// 宿主侧 worker 连接的共享状态：读线程与调用方共用。
struct WorkerInner {
    plugin_id: String,
    /// `None` = 已关闭（停机时 `take()` 掉，真正关管道 → 对端读到 EOF）。
    stdin: Mutex<Option<Box<dyn Write + Send>>>,
    /// 等待中的请求：`request_id` → 响应投递口。读线程按 ID 投递，调用方按 ID 取。
    pending: Mutex<HashMap<String, mpsc::Sender<FrameResponse>>>,
    /// worker 子进程；`None` 仅出现在测试用的流式构造里（无进程可管）。
    child: Mutex<Option<Child>>,
    shutdown_timeout: Duration,
    /// 入站请求处理器；握手之前为 `None`（此时任何插件主动请求都回 `PLUGIN_UNKNOWN_OP`）。
    host_peer: Mutex<Option<Arc<HostPeer>>>,
    /// 自增配对 ID 的序号（ID 形如 `h1`、`h2`…，`h` 表示 host 侧发起）。
    next_request_id: AtomicU64,
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
            stdin: Mutex::new(Some(Box::new(stdin))),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(Some(child)),
            shutdown_timeout: config.shutdown_timeout,
            host_peer: Mutex::new(None),
            next_request_id: AtomicU64::new(0),
        });
        let reader = Self::spawn_reader(inner.clone(), Box::new(stdout), &plugin_id)?;
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
        let plugin_id = plugin_id.into();
        let inner = Arc::new(WorkerInner {
            plugin_id: plugin_id.clone(),
            stdin: Mutex::new(Some(writer)),
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(None),
            shutdown_timeout: Duration::from_millis(200),
            host_peer: Mutex::new(None),
            next_request_id: AtomicU64::new(0),
        });
        let thread =
            Self::spawn_reader(inner.clone(), reader, &plugin_id).expect("测试用读线程应能启动");
        Self { inner, reader: Mutex::new(Some(thread)) }
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
    fn handshake(&self) -> Result<PluginDeclaration, WorkerError> {
        // 取声明失败 → Handshake（进程起没起来 / 是否实现了 describe）；
        // 声明拿到了但不合法 → Declaration（版本不符、字段为空）。两类失败分开报，便于定位。
        let declaration: PluginDeclaration = self
            .call_typed(ops::DESCRIBE, json!({ "proto_version": AXAGENT_PLUGIN_PROTO_VERSION }))
            .map_err(|e| WorkerError::Handshake {
                plugin_id: self.inner.plugin_id.clone(),
                reason: e.to_string(),
            })?;
        declaration.validate().map_err(|e| WorkerError::Declaration {
            plugin_id: self.inner.plugin_id.clone(),
            reason: e.to_string(),
        })?;
        Ok(declaration)
    }

    /// 发一帧请求并等回**配对**的响应。
    ///
    /// 三个「不」是这套并发模型的关键：
    /// - 写帧时**只持写锁**（写完即放）：对端可能正等我们处理它的请求，持锁等待会一起卡死；
    /// - 等响应期间**不持任何锁**：读线程要能并发把响应投进配对表；
    /// - 读线程退出时清空配对表：等待者拿到 `Err` 而不是永久挂起。
    fn exchange(
        &self,
        op: &str,
        args: Value,
        call_chain: Vec<String>,
    ) -> Result<FrameResponse, WorkerError> {
        let sequence = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed) + 1;
        let request_id = format!("h{sequence}");
        let (sender, receiver) = mpsc::channel();
        self.inner.pending.lock().insert(request_id.clone(), sender);
        let request =
            FrameRequest::new(op, args).with_id(request_id.clone()).with_chain(call_chain);

        let write_result = {
            let mut guard = self.inner.stdin.lock();
            match guard.as_mut() {
                Some(stdin) => {
                    write_json_frame(stdin, &request).map_err(|e| format!("写帧失败：{e}"))
                },
                None => Err("通道已关闭（stdin 已释放）".to_string()),
            }
        };
        if let Err(reason) = write_result {
            // 帧没出去 ⇒ 不会有响应 ⇒ 必须自己清掉槽位，否则配对表残留。
            self.inner.pending.lock().remove(&request_id);
            return Err(self.transport_error(reason));
        }
        let response = match receiver.recv() {
            Ok(response) => response,
            Err(_) => {
                // 投递口消失 = 读线程已退出（对端关闭 / 崩溃）。清槽位后明确报错。
                self.inner.pending.lock().remove(&request_id);
                return Err(self.transport_error("worker 连接已关闭，未收到配对响应"));
            },
        };

        match response.kind {
            ResponseKind::Success => Ok(response),
            ResponseKind::Error => {
                let code = response.code.unwrap_or_else(|| "SEAM_EXECUTE_FAILED".to_string());
                let message = response.message.unwrap_or_else(|| "插件未提供错误信息".to_string());
                Err(self.transport_error(format!("{code}: {message}")))
            },
        }
    }

    /// 发一帧并从响应的 `value` 反序列化出强类型结果。
    fn call_typed<T: serde::de::DeserializeOwned>(
        &self,
        op: &str,
        args: Value,
    ) -> Result<T, WorkerError> {
        let response = self.exchange(op, args, current_call_chain())?;
        let value = response.value.unwrap_or(Value::Null);
        serde_json::from_value(value)
            .map_err(|e| self.transport_error(format!("响应无法反序列化为目标类型：{e}")))
    }

    fn transport_error(&self, reason: impl Into<String>) -> WorkerError {
        WorkerError::Transport { plugin_id: self.inner.plugin_id.clone(), reason: reason.into() }
    }

    /// 优雅停机：发 shutdown 帧 → 关 stdin → 限时等退出 → 超时强杀 → 收读线程。
    fn shutdown(&self) {
        // ① 先发 shutdown 帧（此刻读线程还活着，能收下这次响应）。
        let _ = self.exchange(ops::SHUTDOWN, Value::Null, Vec::new());

        // ② **真正关闭 stdin**（`take()` + drop）：worker 侧若用 `serve` 实现，
        //    读到 EOF 即正常返回并退出 —— 这是孤儿进程自退的主防线①。
        self.inner.stdin.lock().take();
        self.wait_or_kill();
        // ③ 进程已死 ⇒ 管道 EOF ⇒ 读线程自然退出，这里只是回收它。
        self.join_reader();
    }

    /// 限时等待子进程退出；超时或已僵死则强杀。
    fn wait_or_kill(&self) {
        let deadline = Instant::now() + self.inner.shutdown_timeout;
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
        self.kill_child();
    }

    /// 强杀子进程（幂等：无子进程时什么也不做）。
    fn kill_child(&self) {
        let mut guard = self.inner.child.lock();
        if let Some(child) = guard.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// 回收读线程（`join`。读线程只在读线程自己的栈上工作，不会持 `inner` 的锁等待，
    /// 故不会与 join 互等）。
    fn join_reader(&self) {
        if let Some(handle) = self.reader.lock().take() {
            let _ = handle.join();
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
/// - **请求** → 交给 [`HostPeer`] 落地，回帧时**回填 `request_id`**；
/// - EOF / 读错误 → 退出，并**清空配对表**（让所有等待者拿到 `Err`，而不是永久挂起）。
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
                // 先把 handler 取出来（`clone` 出 `Arc`）再调：不要握着 `host_peer` 的锁
                // 执行分派，那样会在嵌套调用时自锁。
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
                let mut guard = inner.stdin.lock();
                match guard.as_mut() {
                    Some(stdin) => {
                        if let Err(e) = write_json_frame(stdin, &response) {
                            tracing::warn!(
                                plugin_id = %plugin_id,
                                error = %e,
                                "回写插件请求的响应失败，读线程退出"
                            );
                            break;
                        }
                    },
                    None => break,
                }
            },
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                tracing::warn!(plugin_id = %plugin_id, error = %e, "读帧失败，读线程退出");
                break;
            },
        }
    }
    // 让所有等待者拿到 `Err`（`recv` 因发送端被 drop 而失败），而不是永久挂起。
    inner.pending.lock().clear();
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
        // 目标端自检（环 + 深度）：在**写帧之前**判定 —— 命中即拒绝、不进入等待。
        let plugin_id = self.inner.plugin_id.clone();
        check_chain(&plugin_id, &call_chain)?;
        // 通过自检 ⇒ 把自身追加到链尾，随帧一起传下去（接收方据此继续判环）。
        let mut chain = call_chain;
        chain.push(plugin_id);
        let response = self.exchange(op, args, chain).map_err(|e| e.to_string())?;
        Ok(response.value.unwrap_or(Value::Null))
    }
}

impl Drop for WorkerInvoker {
    fn drop(&mut self) {
        // 兜底：调用方忘了 `unload` 也不留常驻子进程。
        // 顺序：先杀进程（管道那头一关，读线程的阻塞读立刻返回 EOF）→ 再关 stdin
        //（防线①）→ 最后回收读线程。
        self.kill_child();
        self.inner.stdin.lock().take();
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
    fn record_model_visible(&self, session_id: &str, content: ModelVisibleContent) {
        let args = json!({ "session_id": session_id, "content": content });
        if let Err(e) = self.invoker.invoke("record", args) {
            // 契约无 `Result` ⇒ best-effort，只记日志。
            tracing::warn!(error = %e, "远程 session.log.invariant::record 失败");
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

    /// 卸载：**逆序**撤销全部可逆句柄（LIFO，见模块头注释）+ 停 worker。
    pub fn unload(mut self) {
        self.rollback();
        self.invoker.shutdown();
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
    use axagent_plugin_proto::Peer;
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

    /// `session.log.invariant` 是同步接缝：`assert` 的违反必须**如实报出**，不得吞掉。
    #[test]
    fn remote_session_log_invariant_reports_violation() {
        let registry = CapabilityRegistry::new();
        register_remote_seam(
            &registry,
            "demo@external",
            "session.log.invariant",
            "",
            Arc::new(ScriptedInvoker::new(&[("assert", json!({ "ok": true }))])),
        )
        .expect("远程接缝应注册成功");
        let log = registry.get_session_log_invariant().expect("远程实现应可取回");
        log.record_model_visible(
            "s1",
            ModelVisibleContent {
                role: "user".to_string(),
                text: "hi".to_string(),
                tool_names: Vec::new(),
                content_hash: String::new(),
            },
        );
        assert!(log.assert_replayable("s1").is_ok(), "无违反时应为 Ok");
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
}
