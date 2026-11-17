// SPDX-License-Identifier: AGPL-3.0-only

//! `axagent-plugin-proto` —— 宿主与插件 worker 子进程之间的**长度前缀 JSON 帧协议**。
//!
//! 定位：**foundation** 层（零 `axagent-*` 依赖，仅 `serde` / `serde_json` / `thiserror`）。
//! 宿主侧（`axagent-plugins`）与插件 worker 侧共用同一份帧定义，避免两侧各自实现导致协议漂移。
//!
//! 帧格式：`[u32 大端 payload_len][payload 字节]`，payload 为 UTF-8 JSON 文本。
//!
//! 两个协议常量：[`AXAGENT_PLUGIN_PROTO_VERSION`]（握手版本）与 [`MAX_FRAME_LEN`]（单帧上限）。
//!
//! 畸形输入的处理原则：长度字段与实际字节数不符、长度超上限、截断帧、非法 JSON、
//! 合法 JSON 但缺必填字段 —— **一律返回 `Err`**，绝不 panic、绝不死锁。
//! 插件是第三方代码，宿主不能因一帧坏数据被拖垮。
//!
//! ## 双向与重入（P4）
//!
//! 早期版本是「一帧进、一帧出」的单向模型；插件间通信（`PLAN-everything-is-plugin.md` §15）
//! 要求双向：**worker 也能主动向宿主发请求**（`call_seam` / `emit`）。故帧上加了
//! `request_id`（配对）与 `call_chain`（环检测），并由 [`Peer`] 承载「等响应期间仍能收到
//! 对端帧」的重入语义。
//!
//! 插件作者用法（worker 侧 main）：
//!
//! ```no_run
//! use axagent_plugin_proto::{error_codes, serve, FrameRequest, FrameResponse};
//!
//! fn main() -> std::io::Result<()> {
//!     serve(|req: &FrameRequest, peer| match req.op.as_str() {
//!         "describe" => FrameResponse::success(serde_json::json!({ "proto_version": 1 })),
//!         "execute" => {
//!             // 需要同步拿到另一条接缝的返回值时（L2），**原样带上入站帧的 call_chain**。
//!             // 不带链 ⇒ 宿主侧的环检测收不到这条边，A→B→C→B 只能退化到深度上限才发现。
//!             let call = peer.call_seam(
//!                 "workflow.business_rule",
//!                 "evaluate",
//!                 // `node_type` 取 `NodeKind` 的 serde 形态（PascalCase，该枚举无 rename_all）。
//!                 serde_json::json!({ "node_type": "Tool", "node_input": null }),
//!                 req.call_chain.clone(),
//!             );
//!             match call {
//!                 Ok(response) => response,
//!                 Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
//!             }
//!         },
//!         other => FrameResponse::error(
//!             error_codes::PLUGIN_UNKNOWN_OP,
//!             format!("未知操作：{other}"),
//!         ),
//!     })
//! }
//! ```
//!
//! ## 命令帧（任务 #49，T2 派发口形态）
//!
//! 命名命令（manifest `commands[]`）的执行通道从「spawn-on-call」升级为**复用长驻 worker**
//! 后，命令通过一个新的框架 op [`ops::COMMAND`] 投递。它是**加法式**扩展，复用现有
//! [`FrameRequest`] / [`FrameResponse`] 信封与配对机制，**不改握手版本**
//! [`AXAGENT_PLUGIN_PROTO_VERSION`]：
//!
//! - 旧 worker 收到未实现的 `command` op ⇒ 按既有约定回 [`error_codes::PLUGIN_UNKNOWN_OP`]
//!   错误帧，宿主把它归一成命令失败响应，**不会**误当成功；
//! - 新 worker 与宿主共用本节的帧形（[`CommandFrame`] 请求 / [`CommandReply`] 成功回执 /
//!   [`CommandFault`] 失败回执），两侧不再各写一份 JSON 形状。
//!
//! 命令帧承载宿主绑定的 `plugin_id`（宿主按注册表权威键填写，worker 应校验它等于自己的
//! 身份），执行预算 `timeout_ms` 由宿主强制：宿主等待「预算 + 收帧宽限」后放弃在途调用并
//! 报超时；worker 若自觉预算也允许提前回 [`error_codes::PLUGIN_COMMAND_TIMEOUT`]。

use std::collections::VecDeque;
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// 协议版本：握手时宿主用它与插件声明的 `proto_version` 比对（见 [`PluginDeclaration`]）。
pub const AXAGENT_PLUGIN_PROTO_VERSION: u32 = 1;

/// 单帧最大长度（64 MiB）。读取前先校验，防止畸形长度字段触发超大内存分配。
pub const MAX_FRAME_LEN: usize = 64 * 1024 * 1024;

// ───────────────────────────── 长度前缀分帧 ─────────────────────────────

/// 构造「帧长度超上限」错误：写入与读取两侧共用，保证错误形态一致。
fn frame_too_long(len: usize) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("帧长度 {len} 超过上限 {MAX_FRAME_LEN}"))
}

/// 写入一帧：4 字节大端长度 + 负载，**随即 flush**（不 flush 对端会一直等）。
///
/// 负载超过 [`MAX_FRAME_LEN`] 时直接返回 [`io::ErrorKind::InvalidData`]：否则长度字段
/// 会被 `as u32` 静默截断，写出的帧与对端读到的长度对不上。
pub fn write_frame<W: Write>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME_LEN {
        return Err(frame_too_long(payload.len()));
    }
    w.write_all(&(payload.len() as u32).to_be_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// 读取一帧：先读 4 字节长度，再读足量负载。
///
/// **必须先校验长度上限再分配内存**：长度字段声称超过 [`MAX_FRAME_LEN`] 时立刻返回
/// [`io::ErrorKind::InvalidData`] 的 `Err`，绝不按该长度预分配（否则一帧 `0xFFFFFFFF`
/// 就能让宿主按近 4 GiB 申请内存）。
///
/// 截断帧（长度字段与实际字节数不符、中途 EOF）由 `read_exact` 的 `UnexpectedEof`
/// 自然返回 `Err`；畸形输入一律 `Err`，不 panic、不死锁。
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_LEN {
        return Err(frame_too_long(len));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// 序列化 `value` 后按帧写出；序列化失败映射为 [`io::ErrorKind::InvalidData`]。
pub fn write_json_frame<W: Write, T: Serialize>(w: &mut W, value: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("JSON 序列化失败：{e}")))?;
    write_frame(w, &payload)
}

/// 读出一帧并反序列化为 `T`；JSON 非法（或字段缺失、类型不符）映射为
/// [`io::ErrorKind::InvalidData`]。
pub fn read_json_frame<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<T> {
    let payload = read_frame(r)?;
    serde_json::from_slice(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("JSON 反序列化失败：{e}")))
}

// ───────────────────────────── 帧类型 ─────────────────────────────

/// 请求帧：**双向** —— 宿主 → worker（框架调用），以及 worker → 宿主（`call_seam` / `emit`）。
///
/// 字段名保持 snake_case —— 这是 Rust 与 Rust 之间的进程内协议，不是前后端 DTO，
/// 故**不加** `rename_all = "camelCase"`；字段名与设计文档的 schema 逐字对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameRequest {
    /// 操作名，取值见 [`ops`]。
    pub op: String,
    /// 操作参数；缺省为 `null`（`serde_json::Value` 的 `Default` 即 `Value::Null`）。
    #[serde(default)]
    pub args: serde_json::Value,
    /// 配对 ID：**仅由发起方填写**，回应方必须原样回填到 [`FrameResponse::request_id`]。
    ///
    /// 为什么必须有（`PLAN-everything-is-plugin.md` §15.4 ①）：双向化之后同一条管道上
    /// 会同时挂着「我发出去的请求」与「对端发来的请求」，没有配对 ID 就无法判断
    /// 一帧响应到底是谁的 —— 一旦错配，调用方会拿到别人的返回值而不自知。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// 调用链（L2 环检测）：元素为**插件 ID**，按「发起顺序」排列，不含本帧接收方。
    ///
    /// 接收方写帧前先自检 `call_chain.contains(自身 ID)` ⇒ 命中即环（拒绝、不进入等待）；
    /// 否则把自身 ID 追加到链尾再向下传递。详见 [`error_codes::SEAM_CALL_CYCLE_DETECTED`]
    /// 与 [`MAX_SEAM_CALL_DEPTH`]。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub call_chain: Vec<String>,
}

impl FrameRequest {
    /// 构造一个无配对 ID、空调用链的请求（最常见的形态）。
    pub fn new(op: impl Into<String>, args: serde_json::Value) -> Self {
        Self { op: op.into(), args, request_id: None, call_chain: Vec::new() }
    }

    /// 设置配对 ID。
    #[must_use]
    pub fn with_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    /// 设置调用链。
    #[must_use]
    pub fn with_chain(mut self, call_chain: Vec<String>) -> Self {
        self.call_chain = call_chain;
        self
    }
}

/// 响应帧的成败判别；缺 `kind` 字段的 JSON 会在此反序列化失败（`kind` 无 default），
/// 这正是期望行为：畸形输入走 `Err`，不走 panic。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    /// 成功。
    Success,
    /// 失败。
    Error,
}

/// 响应帧：**双向** —— 与 [`FrameRequest`] 方向相反。
///
/// `value` / `message` / `code` 均为可选字段：成功帧只带 `value`，失败帧带 `code` + `message`；
/// 空字段在序列化时被跳过，帧体更紧凑。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameResponse {
    /// 成败判别。
    pub kind: ResponseKind,
    /// 成功时的返回值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// 失败时的人类可读说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 失败时的错误码（宿主侧据此归类/国际化）。参见 [`error_codes`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 对应请求的 [`FrameRequest::request_id`]，**必须原样回填**（否则发起方无法配对）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl FrameResponse {
    /// 构造成功响应。
    pub fn success(value: serde_json::Value) -> Self {
        Self {
            kind: ResponseKind::Success,
            value: Some(value),
            message: None,
            code: None,
            request_id: None,
        }
    }

    /// 构造失败响应。
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ResponseKind::Error,
            value: None,
            message: Some(message.into()),
            code: Some(code.into()),
            request_id: None,
        }
    }

    /// 回填配对 ID。
    #[must_use]
    pub fn with_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}

/// 协议级 op 常量：这些是**框架**操作，插件作者不实现它们（由本 crate / 宿主侧承载）。
pub mod ops {
    /// 握手：宿主请求插件返回声明（[`super::PluginDeclaration`]）。
    pub const DESCRIBE: &str = "describe";
    /// 宿主向 worker 推送事件（L1 事件桥，与插件声明的 `subscribe` 对应）。
    pub const EVENT: &str = "event";
    /// worker 请求宿主代为派发事件。
    pub const EMIT: &str = "emit";
    /// worker 请求宿主代为调用**另一条接缝**（L2 跨插件调用，见 `PLAN-everything-is-plugin.md` §15.4 ②）。
    pub const CALL_SEAM: &str = "call_seam";
    /// 卸载时的优雅停机请求。
    pub const SHUTDOWN: &str = "shutdown";
    /// 宿主把一条**命名命令**派发给长驻 worker 执行（任务 #49，T2 命令通道）。
    ///
    /// 载荷形状见 [`super::CommandFrame`]；回执见 [`super::CommandOutcome`]。
    /// 与 `event` / `ui.action` 同属「宿主 → worker」方向的框架 op，插件不在声明里
    /// 提供它，但 worker 的 `serve` 处理器需要实现它才能承接命令。
    pub const COMMAND: &str = "command";
    /// 宿主把动态 UI 里触发的 action 回流给**该插件自己**（`PLAN-everything-is-plugin.md` §10.5-2）。
    ///
    /// 方向与 `EMIT` / `CALL_SEAM` 相反（那两者是插件 → 宿主），故也不是一条「接缝」：
    /// 插件不需要声明它，宿主按 `pluginId` 直接把帧发给对应 worker。
    pub const UI_ACTION: &str = "ui.action";
}

/// 插件协议的**内部**错误码（不是用户可见错误）。
///
/// 为什么不进 `axagent-harness` 的 UI 错误码表：本 crate 是 foundation，
/// **禁止依赖任何 `axagent-*` crate**（铁律 1），故不能引用 harness 的常量。
/// 且这些码是「宿主与插件之间的协议语义」，不面向终端用户。
pub mod error_codes {
    /// 调用链中已出现接收方自身 ⇒ 环，拒绝调用（不进入等待，故不会死锁）。
    pub const SEAM_CALL_CYCLE_DETECTED: &str = "SEAM_CALL_CYCLE_DETECTED";
    /// 调用链长度已达 [`super::MAX_SEAM_CALL_DEPTH`] ⇒ 拒绝调用，防无环深链耗尽栈/进程。
    pub const SEAM_CALL_DEPTH_EXCEEDED: &str = "SEAM_CALL_DEPTH_EXCEEDED";
    /// 插件调用了**自己未在声明 `calls` 中列出**的接缝 ⇒ 拒绝（P3 声明的运行时兜底）。
    pub const SEAM_CALL_NOT_DECLARED: &str = "SEAM_CALL_NOT_DECLARED";
    /// 目标接缝在能力注册表中没有 provider（典型场景：提供方插件已卸载）⇒ 明确失败，**不挂起**。
    pub const SEAM_PROVIDER_UNAVAILABLE: &str = "SEAM_PROVIDER_UNAVAILABLE";
    /// 宿主不支持对该接缝的 `call_seam` 分派（不在宿主的分派表内）。
    pub const SEAM_CALL_UNSUPPORTED: &str = "SEAM_CALL_UNSUPPORTED";
    /// `call_seam` 的参数形状不合法（缺 `seam` / `op`，或 `args` 不是对象）。
    pub const SEAM_CALL_INVALID_ARGS: &str = "SEAM_CALL_INVALID_ARGS";
    /// 收到未知 op（框架操作之外的、插件自己未实现的 op）。
    pub const PLUGIN_UNKNOWN_OP: &str = "PLUGIN_UNKNOWN_OP";
    /// 插件侧主动发起的调用失败（I/O、对端退出等）时的兜底码。
    pub const PLUGIN_CALL_FAILED: &str = "PLUGIN_CALL_FAILED";
    /// worker 未实现命令帧（老 worker 会把 `command` op 报成 `PLUGIN_UNKNOWN_OP`；
    /// 本码用于 worker **认识命令帧但没有该命令实现**的情形）。
    pub const PLUGIN_COMMAND_UNKNOWN: &str = "PLUGIN_COMMAND_UNKNOWN";
    /// 命令帧本身形状非法（worker 解码 [`super::CommandFrame`] 失败的兜底码）。
    pub const PLUGIN_COMMAND_INVALID_ARGS: &str = "PLUGIN_COMMAND_INVALID_ARGS";
    /// 命令帧携带的 `plugin_id` 与 worker 自身身份不符（宿主绑定被篡改/错配）。
    pub const PLUGIN_COMMAND_ID_MISMATCH: &str = "PLUGIN_COMMAND_ID_MISMATCH";
    /// worker 自觉超出执行预算（`CommandFrame::timeout_ms`）主动回错；
    /// 宿主侧强制超时不走本码，由宿主直接以 [`super::CommandFrame::timeout_ms`] 为界放弃等待。
    pub const PLUGIN_COMMAND_TIMEOUT: &str = "PLUGIN_COMMAND_TIMEOUT";
    /// 命令在 worker 内执行失败的兜底码（worker 未给出更具体分类时使用）。
    pub const PLUGIN_COMMAND_FAILED: &str = "PLUGIN_COMMAND_FAILED";
    /// 插件未在清单 `permissions` 中声明目标写操作所需的权限 ⇒ 宿主拒绝接缝写 op。
    ///
    /// 内部诊断码：只在宿主与插件之间的接缝分派里出现，**不进** 11 语言 UI 词表
    /// （与同模块其余码一致，见模块头注释）。
    pub const PLUGIN_PERMISSION_DENIED: &str = "PLUGIN_PERMISSION_DENIED";
    /// 宿主拒绝受理工作流执行（模板不在白名单 / 准入并发超限）。内部诊断码，不进 11 语言 UI 词表。
    pub const WORKFLOW_RUN_REJECTED: &str = "WORKFLOW_RUN_REJECTED";
}

/// 调用链长度上限（`PLAN-everything-is-plugin.md` §15.5）：不含本帧接收方。
///
/// 与环检测互补 —— 环检测挡「同一条链上重复出现」，深度上限挡「无环但极深」的长链
/// （每层跨插件调用都要占一份栈 + 一份子进程等待，深链会耗尽资源）。
pub const MAX_SEAM_CALL_DEPTH: usize = 8;

// ───────────────────────────── 命令帧（任务 #49，T2） ─────────────────────────────

/// 宿主派发单条命名命令的默认执行预算（毫秒）。
///
/// 命令走长驻 worker（[`ops::COMMAND`]）时，宿主以「预算 + 收帧宽限」为等待上限强制收尾，
/// 插件僵死不再级联卡死宿主调用线程（D1）。调用方可按命令耗时特征显式覆盖。
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 30_000;

/// 宿主等待 worker 回帧的**接缝调用**预算上限（毫秒，任务 #53 修 D1）。
///
/// 适用对象：宿主经接缝公共入口（[`ops::CALL_SEAM`] 分派、远程门面、UI action 回流）向
/// worker 发帧后等待配对响应的时间。与命令帧预算 [`DEFAULT_COMMAND_TIMEOUT_MS`]
/// 分开、且**必须更小**，理由是两条通道的等待位置不同：
///
/// - 命令帧是用户动作的**终执行**，预算可由调用方显式覆盖（[`CommandFrame::timeout_ms`]
///   必填），30s 是给「worker 真的在算」的宽预算；
/// - 接缝调用是**同步夹在对话回合中间**的依赖调用 —— 单条依赖僵死若按 30s 收口，
///   一个回合内多条接缝叠加最坏会按调用链深度（[`MAX_SEAM_CALL_DEPTH`]）成倍放大，
///   用户感知即「整个应用没反应」。
///
/// 取值判据：15s 对占绝大多数的毫秒~秒级接缝操作（规则评估 / 沙箱校验 / 工具清单 /
/// webhook 派发）是正常延迟的一个数量级余量，又把「回合被一条僵死依赖拖住」的上界
/// 压在半分钟以内。长跑型接缝不靠调大本常量解决，走 [`SEAM_OP_TIMEOUT_OVERRIDES`]
/// 的逐接缝预算表（均按证据定，不许拍脑袋）。
///
/// 两项如实声明（任务 #53 裁定）：
/// - **预算不进帧**（不给 [`FrameRequest`] 加字段）：预算是宿主策略，不是插件可
///   协商的东西；协议面零变化优先级高于「可配置」。
/// - **本批只覆盖收侧**（等回帧的 `recv_timeout`）：写帧侧的卡死（对端僵死不读
///   stdin 且管道满 ⇒ 写方持锁阻塞，`recv_timeout` 覆盖不到）另立案 #56，
///   修它需要 writer 线程 / 异步写的接缝栈重构。读侧超时后在途调用的处置沿用
///   既有语义：撤配对槽位、迟到帧由读线程按「无等待者」丢弃（宿主侧另有
///   「标脏拒后续」，见 `axagent-plugins` 的 `WorkerInvoker::invoke_with_chain`）。
pub const DEFAULT_SEAM_TIMEOUT_MS: u64 = 15_000;

/// `agent.loop` 长跑型接缝的专用预算（毫秒）。
///
/// 适用对象：`run_turn` 是一次**完整 LLM Agent 回合**（多轮模型调用 + 工具执行，
/// `harness::AgentTurnRunner::run_turn`，PLAN §5.4 行 `agent.loop` ⇒ op
/// `run_turn`），耗时特征为分钟级；15s 默认预算必误杀。
///
/// **取值规则 = `max(宿主 LLM 单请求上限, 生产工作流给 Agent 节点的预算)`**，两个来源都取
/// 盘上现存的宿主预算，本值不另立数字：
/// - 宿主 LLM 客户端单请求上限 **300 s** —— `axagent-ai/crates/providers/src/lib.rs:278`
///   的 `build_http_client`：`.timeout(std::time::Duration::from_secs(300))`；
/// - 生产工作流给一个 Agent 节点的预算 **900 s** ——
///   `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:4515` 对
///   `WorkflowNode::Agent`（debate-convergence）写死 `a.base.timeout = Some(900)`，取舍
///   理由的实证在同文件 `:4509`「大上下文 LLM 的 TTFB 偶发 >120s」；同族节点预算还有多智能体节点 900 s
///   （`axagent-core/crates/harness/src/capability_pack_orchestration/subgraph.rs:145`）、
///   OPC 子 Agent 节点 600 s（同文件 `:94`）、模板变量 `agent_timeout_secs` 的现网缺省
///   600 s（`src-tauri/src/commands/stock_analysis_setup/seed_variables.rs:202`，消费点
///   钳位 [10, 3600]）。⇒ 该族里的最大值即 900 s。
///
/// ⇒ `max(300, 900) = 900`。取 max 而非取小的理由：一个回合含**多轮**模型调用，300 s 只是
/// 单轮上限；而宿主已愿意为这个节点等 900 s，接缝侧若低于它就成了「宿主还在等、依赖侧先撤」。
///
/// 为什么这条必须有上界（而不只是「比 15s 大」）：`run_turn` 的真实消费方就在生产 ——
/// `axagent-runtime/crates/rt-workflow/src/work_engine/executors/agent_executor.rs:270`
/// 的 `runner.run_turn(request).await`（runner 取自同函数 `:433` 的能力注册表，注册表缺
/// 则回退字段注入）。该路径**不按 `max_tool_rounds` 收口回合长度**：它是
/// `Option<u32>`（`agent_executor.rs:266` 透传节点配置），多个生产种子点直接给 `None`
/// （`axagent-core/crates/harness/src/assembly_builder.rs:201`、
/// `axagent-core/crates/harness/src/plan_compiler.rs:142`、
/// `axagent-core/crates/harness/src/capability_pack_orchestration/subgraph.rs:118`），
/// 回合的兜底来自 `runtime-core` 的迭代数（缺省 50，
/// `axagent-runtime/crates/runtime-core/src/conversation.rs:262`）× 单请求 300 s ⇒
/// **没有秒级上界**。故本值按「与宿主既有预算同源」定档，不做「猜一个够用的数」。
///
/// ⚠ 如实口径：本值是**宿主侧两份既存预算的 max**（出处见上），不再是无出处的裁定值；
/// 但 `run_turn` 自身耗时的实测分布盘上仍然**没有**，真插件接入后应按回合耗时分布复核。
pub const AGENT_LOOP_RUN_TURN_TIMEOUT_MS: u64 = 900_000;

/// `tool.set` 的 op `exec` 专用预算（毫秒）。
///
/// 依据（§26.3 S-6 的对齐方向：**远程接缝对齐 spawn 通道**，不是反过来削 spawn）：同一个
/// 插件工具在宿主侧有**两条通道**，本批之前预算差 4 倍 ——
/// - per-call spawn 通道 **60 s**：`axagent-plugins/crates/plugins/src/types.rs:837` 的
///   `const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 60`，用于同文件 `:867` 与 `:955` 的两处轮询；
///   这是**既存产品行为**，本批不动它（也不许有人拿本常量去改它 —— 那是另一个任务）；
/// - 远程长驻 worker 通道：op `exec` 经 [`seam_timeout_ms_for_op`] 取预算（取预算位点 =
///   `axagent-plugins/crates/plugins/src/worker.rs` 的 `seam_wait_for_op`，转发点 = 同文件
///   `forward_remote::<ToolExecReply>` 的 `"exec"`；该文件正被改造，**只认符号不认行号**），
///   未进表时回落 [`DEFAULT_SEAM_TIMEOUT_MS`] = 15 s。
///
/// ⇒ 同一个工具「换个装载形态就要快 4 倍才判死」没有判据支撑，故对齐到 spawn 侧的 60 s。
/// 与 [`AGENT_LOOP_RUN_TURN_TIMEOUT_MS`] 的区别要说清：那条有**宿主预算出处**（两份取 max），
/// 本条的出处是**宿主另一条通道对自己设的超时**（`types.rs:837`）—— 二者都是「盘上已有的
/// 宿主数字」，不是 op 级耗时实测；`exec` 自身耗时的实测分布盘上仍**没有**。
pub const TOOL_SET_EXEC_TIMEOUT_MS: u64 = 60_000;

/// 逐接缝预算**覆盖表**（任务 #53）：键 = PLAN §5.4 约定的 op 名，值 = 等待预算毫秒。
///
/// 为什么键是 op 而不是 seam：宿主传输层（`WorkerInvoker::invoke_with_chain`）在写帧
/// 时只见得到 op 字符串，接缝 ID 已消失在门面路由之后；op 词汇由 §5.4 映射表固定
/// （单 op 接缝取声明 op 名，多 op 接缝由门面硬编码，全表见 `PLAN-everything-is-plugin.md`
/// §5.4 + worker.rs 门面）。若插件声明漂移出表 ⇒ 落回默认预算 —— 失败方向是「长跑
/// 调用被更早放弃」而非协议错配，可接受。
///
/// **收录判据（逐接缝过证据；证据 = 盘上实测或有出处的宿主预算，「看起来快」不算）**：
/// - `run_turn`：分钟级 LLM 回合，§5.4 + 两份盘上宿主预算取 max ⇒ **已进表**，理由与出处见
///   [`AGENT_LOOP_RUN_TURN_TIMEOUT_MS`]；
/// - `exec`（`tool.set`）：**已进表** = 60_000，与宿主 spawn 通道同源
///   （`axagent-plugins/crates/plugins/src/types.rs:837` 的 `DEFAULT_TOOL_TIMEOUT_SECS = 60`），
///   对齐理由、两通道取预算的位点与「这不是 op 级实测」的如实口径见
///   [`TOOL_SET_EXEC_TIMEOUT_MS`]（§26.3 S-6）；
/// - `chat` / `embed`（`model.provider`）：走的是宿主同一个 LLM 客户端，文档口径「0.5–30 s」
///   （`PLAN-everything-is-plugin.md:274`），单请求上限同 `run_turn` 那条 300 s ⇒ 15 s 默认
///   在高端会误杀，但**没有 op 级耗时实测** ⇒ 暂留默认，标「预算待实证」；
/// - `reflect`（reflector）、`suggest`（optimizer）、`evolve`（evolver，§5.4 原注「种群可能
///   极大，唯一工程项」）、`on_message`（message.callback）：都可能走到 LLM / 外网，**无实测
///   分布亦无盘上预算出处** ⇒ 暂留默认，标「预算待实证」；
/// - `evaluate`（business_rule）：**有实测** —— 长驻 `pipe` 往返 18.9 µs
///   （`PLAN-everything-is-plugin.md:274` / `:670`，§11 基准表）⇒ 15 s 余量充足，不进表；
/// - `event`（L1 事件桥，`ops::EVENT`）：**有实测（任务 #140）** —— 回环 socket + 与生产同源的
///   `WorkerInvoker` 走完整 `WorkerEventSubscriber::handle` 路径，每轮 200 次采样、两轮：
///   **p50 0.39–0.50 ms / p99 0.87–1.74 ms / max 1.5–2.1 ms**（`worker.rs` 的
///   `event_op_round_trip_is_measured_and_stays_under_default_budget`，2026-11-04）⇒
///   相对默认预算有四个数量级余量，**不进表**；
/// - `execute`（sandbox）、`record` / `assert`、`start` / `stop` / `send` / `is_*`、
///   `dispatch`、`list`：**只有形态推断（毫秒~秒级），盘上无实测、无出处** ⇒ 留默认，
///   其中 `send` / `dispatch` 触外网，属「无证据但存疑」，需要实测才能裁定。
pub const SEAM_OP_TIMEOUT_OVERRIDES: &[(&str, u64)] =
    &[("run_turn", AGENT_LOOP_RUN_TURN_TIMEOUT_MS), ("exec", TOOL_SET_EXEC_TIMEOUT_MS)];

/// 查 op 的接缝预算覆盖：命中 [`SEAM_OP_TIMEOUT_OVERRIDES`] 返回专用预算，否则 `None`
/// （调用方回落到宿主默认 —— 通常是 [`DEFAULT_SEAM_TIMEOUT_MS`]）。
#[must_use]
pub fn seam_timeout_ms_for_op(op: &str) -> Option<u64> {
    SEAM_OP_TIMEOUT_OVERRIDES
        .iter()
        .find(|(override_op, _)| *override_op == op)
        .map(|(_, timeout_ms)| *timeout_ms)
}

/// 宿主等待 worker 回 **`ops::DESCRIBE` 握手帧** 的预算上限（毫秒，任务 #63 补 D1 边角）。
///
/// 与 [`DEFAULT_SEAM_TIMEOUT_MS`] 是**两条不同通道的预算**，不许互相借用：
///
/// - **接缝预算**服务的是「用户发起的调用」—— 对话回合中间的一次依赖调用，会沿调用链
///   深度（[`MAX_SEAM_CALL_DEPTH`]）成倍放大，故取 15s 并把长跑型 op 逐条放进
///   [`SEAM_OP_TIMEOUT_OVERRIDES`]；
/// - **握手预算**服务的是「装载前置」—— `LoadedPlugin::load` 里 spawn 之后的第一帧，
///   不在接缝公共入口（`SeamInvoker::invoke_with_chain`）的控制流内，因此**不经**
///   [`seam_timeout_ms_for_op`]，也不进那张表（表按「op ⇒ 接缝预算」的语义组织，把
///   `describe` 塞进去会让「表只给 seam op」的既有不变量失效，见本模块测试
///   `seam_timeout_override_table_is_evidence_bounded`）。
///
/// 取值判据：握手**没有任何业务计算**，只需要 worker 把进程/解释器起来并回一帧声明，
/// 正常形态是毫秒级；但它发生在「装载」这条串行路径上（宿主逐个插件启用，一个装死的
/// worker 会把它后面的全部插件一起堵住），所以预算必须**有界**；同时不能收得过紧 ——
/// 解释器型插件（Python/Node）冷启动时的 import 与运行时初始化是秒级量级，误杀会让
/// 插件「看起来装不上」。10s = 秒级冷启动的一个数量级余量，且严格小于接缝预算
/// （握手的等待位置更靠前、更不该占着一大段）。⚠ 该数为**判断值、非实测分布**，
/// 与 [`AGENT_LOOP_RUN_TURN_TIMEOUT_MS`] 一样**缺实测分布**（那边已有宿主预算出处，本数仍是
/// 判断值）；真插件接入后应按握手耗时分布复核。
///
/// 线格式**零变化**：预算是宿主侧等待策略，不进帧，故 [`AXAGENT_PLUGIN_PROTO_VERSION`] 保持 1。
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 10_000;

/// 命令帧三型之一：**请求**（宿主 → worker）。
///
/// 信封复用 [`FrameRequest`]：`op` 固定 [`ops::COMMAND`]，本结构序列化后放 `args`。
/// `request_id` 配对、`call_chain` 等框架语义完全沿用，故**不需要**协议版本 bump ——
/// 旧 worker 读得懂信封，只是把没实现的 op 回成 [`error_codes::PLUGIN_UNKNOWN_OP`]。
///
/// 字段命名与 [`FrameRequest`] 同口径（Rust↔Rust 进程内协议，snake_case，不加 camelCase）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandFrame {
    /// 宿主绑定的插件 ID（**注册表权威键**，由宿主填写，不是调用方入参的透传）。
    ///
    /// worker 侧应校验它等于自身身份（spawn 时注入的 `AXAGENT_PLUGIN_ID` 环境变量），
    /// 不符必须回 [`error_codes::PLUGIN_COMMAND_ID_MISMATCH`] 并拒绝执行。
    pub plugin_id: String,
    /// 命令名（manifest `commands[].name`；声明真相在宿主侧，worker 只按名分发）。
    pub command: String,
    /// 命令输入 JSON（宿主已按 `inputSchema` 做过授权门槛 + 最小形状校验才发帧）。
    /// 缺省为 `null`。
    #[serde(default)]
    pub input: serde_json::Value,
    /// 执行预算（毫秒）。**必填、无缺省**：预算语义是「宿主对超时负责」的载体，
    /// 缺了它帧就不完整，宁可解码报错也不给「无限预算」的静默形态。
    pub timeout_ms: u64,
}

impl CommandFrame {
    /// 构造命令帧请求体（`plugin_id` 必须传宿主注册表键，见字段注释）。
    pub fn new(
        plugin_id: impl Into<String>,
        command: impl Into<String>,
        input: serde_json::Value,
        timeout_ms: u64,
    ) -> Self {
        Self { plugin_id: plugin_id.into(), command: command.into(), input, timeout_ms }
    }

    /// 编成 [`FrameRequest`] 信封（不带配对 ID —— 由发起方经传输层补填）。
    pub fn to_frame_request(&self) -> io::Result<FrameRequest> {
        let args = serde_json::to_value(self).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("命令帧序列化失败：{e}"))
        })?;
        Ok(FrameRequest::new(ops::COMMAND, args))
    }

    /// 从 [`FrameRequest`] 解出命令帧；非 `command` op 或形状非法都返回 `Err`。
    pub fn from_frame_request(request: &FrameRequest) -> Result<Self, CommandFrameError> {
        if request.op != ops::COMMAND {
            return Err(CommandFrameError::NotCommand { op: request.op.clone() });
        }
        serde_json::from_value(request.args.clone())
            .map_err(|e| CommandFrameError::InvalidShape { reason: e.to_string() })
    }
}

/// 命令帧三型之二：**成功回执**（worker → 宿主，或回执解码后的成功形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandReply {
    /// 回填请求帧的插件 ID 绑定（与 [`CommandFrame::plugin_id`] 一致，防错配）。
    pub plugin_id: String,
    /// 回填命令名（与 [`CommandFrame::command`] 一致）。
    pub command: String,
    /// 命令输出 JSON；缺省为 `null`。
    #[serde(default)]
    pub value: serde_json::Value,
}

impl CommandReply {
    /// 按请求帧回填绑定字段（worker 侧便捷构造）。
    pub fn for_frame(frame: &CommandFrame, value: serde_json::Value) -> Self {
        Self { plugin_id: frame.plugin_id.clone(), command: frame.command.clone(), value }
    }

    /// 编成 [`FrameResponse`] 成功帧（`value` 字段承载本结构的 JSON）。
    pub fn into_frame_response(self) -> io::Result<FrameResponse> {
        let value = serde_json::to_value(&self).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("回执序列化失败：{e}"))
        })?;
        Ok(FrameResponse::success(value))
    }
}

/// 命令帧三型之三：**失败回执**（worker → 宿主，或回执解码后的失败形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandFault {
    /// 请求帧绑定的插件 ID（对端未回填绑定时为空串 —— 解码不因此失败，但绑定信息缺失）。
    #[serde(default)]
    pub plugin_id: String,
    /// 命令名（同上，缺绑定为空串）。
    #[serde(default)]
    pub command: String,
    /// 协议错误码（见 [`error_codes`]；缺省兜底 [`error_codes::PLUGIN_COMMAND_FAILED`]）。
    pub code: String,
    /// 人类可读说明。
    #[serde(default)]
    pub message: String,
}

impl CommandFault {
    /// 按请求帧回填绑定字段（worker 侧便捷构造）。
    pub fn for_frame(
        frame: &CommandFrame,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            plugin_id: frame.plugin_id.clone(),
            command: frame.command.clone(),
            code: code.into(),
            message: message.into(),
        }
    }

    /// 编成 [`FrameResponse`] 错误帧（绑定字段随 `value` 透传，供发起方核对）。
    pub fn into_frame_response(self) -> FrameResponse {
        FrameResponse {
            kind: ResponseKind::Error,
            value: Some(serde_json::json!(
                { "plugin_id": self.plugin_id, "command": self.command }
            )),
            message: Some(self.message),
            code: Some(self.code),
            request_id: None,
        }
    }
}

/// 命令帧回执的两型判别（发起方按 [`FrameResponse::kind`] 解码后的强类型形态）。
#[derive(Debug, Clone, PartialEq)]
pub enum CommandOutcome {
    /// 成功回执。
    Reply(CommandReply),
    /// 失败回执（含错误码）。
    Fault(CommandFault),
}

impl CommandOutcome {
    /// 编成 [`FrameResponse`] 写回对端。
    pub fn into_frame_response(self) -> io::Result<FrameResponse> {
        match self {
            Self::Reply(reply) => reply.into_frame_response(),
            Self::Fault(fault) => Ok(fault.into_frame_response()),
        }
    }

    /// 从 [`FrameResponse`] 解码回执。
    ///
    /// 成功帧的 `value` 必须是 [`CommandReply`] 形状（裸输出 ⇒ [`CommandFrameError::InvalidShape`]，
    /// 不猜）；错误帧容忍缺失绑定字段（`plugin_id` / `command` 落空串），
    /// 缺 `code` 兜底 [`error_codes::PLUGIN_COMMAND_FAILED`]。
    /// 「对端是旧 worker」的降级路径也走这里：它把 `command` op 回成
    /// [`error_codes::PLUGIN_UNKNOWN_OP`] 错误帧 ⇒ 解码为 Fault，**无需版本 bump**。
    pub fn from_frame_response(response: &FrameResponse) -> Result<Self, CommandFrameError> {
        match response.kind {
            ResponseKind::Success => {
                serde_json::from_value(response.value.clone().unwrap_or(serde_json::Value::Null))
                    .map(Self::Reply)
                    .map_err(|e| CommandFrameError::InvalidShape { reason: e.to_string() })
            },
            ResponseKind::Error => {
                let binding = response.value.clone().unwrap_or(serde_json::Value::Null);
                let fault = CommandFault {
                    plugin_id: binding
                        .get("plugin_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    command: binding
                        .get("command")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    code: response
                        .code
                        .clone()
                        .unwrap_or_else(|| error_codes::PLUGIN_COMMAND_FAILED.to_string()),
                    message: response.message.clone().unwrap_or_default(),
                };
                Ok(Self::Fault(fault))
            },
        }
    }
}

/// 命令帧编解码错误（协议内部判别用，非用户可见错误）。
#[derive(Debug, thiserror::Error)]
pub enum CommandFrameError {
    /// 帧不是命令帧（op 不匹配）。
    #[error("不是命令帧：op 为 `{op}`，期望 `command`")]
    NotCommand {
        /// 实际收到的 op。
        op: String,
    },
    /// 形状不合法（缺必填字段 / 类型不符 / JSON 非法）。
    #[error("命令帧形状不合法：{reason}")]
    InvalidShape {
        /// 具体原因（serde 报错原文）。
        reason: String,
    },
}

// ───────────────────────────── 插件声明 ─────────────────────────────

/// 单条能力声明：插件**提供**的某个接缝实现。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDecl {
    /// 接缝 ID（如 `workflow.business_rule`）；空则声明非法。
    pub seam: String,
    /// 该接缝内的操作名。
    pub op: String,
    /// 接缝协议版本；空则声明非法。
    pub version: String,
}

/// 插件声明（设计文档 §5.2）：worker 收到 [`ops::DESCRIBE`] 时返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDeclaration {
    /// 插件实现的帧协议版本，必须等于 [`AXAGENT_PLUGIN_PROTO_VERSION`]。
    pub proto_version: u32,
    /// 插件**订阅**的事件（如 `"event.dispatch"`）：宿主派发这些事件时投递给本插件。
    #[serde(default)]
    pub subscribe: Vec<String>,
    /// 插件要**调用**的其他接缝：宿主在载入时校验（P4 用），避免运行时才发现缺依赖。
    #[serde(default)]
    pub calls: Vec<String>,
    /// 插件**提供**的接缝能力。
    #[serde(default)]
    pub capabilities: Vec<CapabilityDecl>,
}

/// 声明校验失败的原因。
#[derive(Debug, thiserror::Error)]
pub enum DeclarationError {
    /// 插件声明的协议版本与宿主支持的不一致。
    #[error("协议版本不匹配：插件声明 {declared}，宿主支持 {expected}")]
    VersionMismatch {
        /// 插件声明的版本。
        declared: u32,
        /// 宿主支持的版本。
        expected: u32,
    },
    /// 能力声明缺少接缝 ID。
    #[error("能力声明缺少接缝 ID")]
    EmptySeam,
    /// 能力声明缺少版本号。
    #[error("能力声明缺少版本号：{seam}")]
    EmptyVersion {
        /// 出问题的那条能力声明所属的接缝 ID。
        seam: String,
    },
}

impl PluginDeclaration {
    /// 校验声明的合法性：协议版本一致、每条能力声明的接缝 ID 与版本号非空。
    pub fn validate(&self) -> Result<(), DeclarationError> {
        if self.proto_version != AXAGENT_PLUGIN_PROTO_VERSION {
            return Err(DeclarationError::VersionMismatch {
                declared: self.proto_version,
                expected: AXAGENT_PLUGIN_PROTO_VERSION,
            });
        }
        for cap in &self.capabilities {
            if cap.seam.is_empty() {
                return Err(DeclarationError::EmptySeam);
            }
            if cap.version.is_empty() {
                return Err(DeclarationError::EmptyVersion { seam: cap.seam.clone() });
            }
        }
        Ok(())
    }
}

// ─────────────────── 任务中心数据面（T2 命令通道的消费面 · PLAN §98）───────────────────

/// 插件贡献的**任务源**一行运行记录（`manifest.taskSources[].command` 的返回元素）。
///
/// 为什么行契约定在本 crate，而不是宿主前端类型或 `axagent-harness`：
/// - **权威只此一份**（AGENTS.md 铁律 4）。宿主任务中心的 `UnifiedRun` 是**前端视图类型**
///   （含 `capabilities` / `badges` 等宿主才有的口径），插件 worker 既拿不到也不该拿；
///   插件唯一够得着的共享面就是这个协议 crate ⇒ 两侧对同一形状各自序列化，不写第二遍。
/// - **只给数据，不给渲染**（PLAN §93 裁定）：插件交回的是行，不是 React 组件；
///   `deep_link` 是宿主路由的**字符串**，宿主只跳转、不解释其内容。
///
/// 线上键为 **camelCase**（全站约定，禁区 13）；除四个必填项外全部可缺省。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTaskRunRow {
    /// 源内原生 id。宿主按 `"<sourceId>:<nativeId>"` 拼全局唯一 id，插件**不得**自带前缀。
    pub native_id: String,
    /// 列表展示名（插件按自己的口径拼好，含股票代码/名称等）；**不经宿主 i18n**。
    pub name: String,
    /// 运行状态。词汇表与宿主 `RunState` 逐字一致：
    /// `queued` / `running` / `waiting` / `paused` / `succeeded` / `failed` / `canceled`。
    /// ⚠ 表外值宿主一律按「未知」处理，**不猜**成某个具体状态（与 `mapRunTrigger` 同一条纪律）。
    pub state: String,
    /// 触发来源；表外/缺省 ⇒ 宿主落 `unknown`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// 开始时刻（epoch 毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    /// 结束时刻（epoch 毫秒）；未结束 ⇒ 缺省。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    /// 耗时（毫秒）；缺省 ⇒ 宿主按 `finished_at - started_at` 算，算不出就不显示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 「打开详情」跳的站内路由；缺省 ⇒ 该 run 不可打开（宿主 `openSource` 能力关）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
    /// 源原生记录，原样透出给源专属抽屉；中心不解读。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

/// [`PluginTaskRunRow::state`] 的合法词汇表（与宿主 `RunState` 同源，改动必须两处同批）。
pub const PLUGIN_TASK_RUN_STATES: [&str; 7] =
    ["queued", "running", "waiting", "paused", "succeeded", "failed", "canceled"];

/// 判断字符串是否为 [`PLUGIN_TASK_RUN_STATES`] 之一（精确小写，不做宽容）。
pub fn is_plugin_task_run_state(value: &str) -> bool {
    PLUGIN_TASK_RUN_STATES.contains(&value)
}

// ───────────────────────────── 入站帧判别 ─────────────────────────────

/// 入站帧的两种可能方向（双向管道上「读到的这一帧是谁发的」）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Inbound {
    /// 对端发来的**请求**（带 `op`）。
    Request(FrameRequest),
    /// 对端发来的**响应**（带 `kind`）。
    Response(FrameResponse),
}

impl Inbound {
    /// 若是请求，取出其引用。
    pub fn as_request(&self) -> Option<&FrameRequest> {
        match self {
            Self::Request(req) => Some(req),
            Self::Response(_) => None,
        }
    }

    /// 若是响应，取出其引用。
    pub fn as_response(&self) -> Option<&FrameResponse> {
        match self {
            Self::Response(resp) => Some(resp),
            Self::Request(_) => None,
        }
    }
}

/// 判别一帧 JSON 的方向：**请求带 `op`，响应带 `kind`**。
///
/// 两者皆无、或两者皆有 ⇒ [`io::ErrorKind::InvalidData`]：方向不明就不猜，
/// 猜错会让调用方拿到别人的返回值（比直接报错危险得多）。
pub fn classify_inbound(value: serde_json::Value) -> io::Result<Inbound> {
    let has_op = value.get("op").is_some();
    let has_kind = value.get("kind").is_some();
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    match (has_op, has_kind) {
        (true, false) => serde_json::from_value(value)
            .map(Inbound::Request)
            .map_err(|e| bad(&format!("请求帧反序列化失败：{e}"))),
        (false, true) => serde_json::from_value(value)
            .map(Inbound::Response)
            .map_err(|e| bad(&format!("响应帧反序列化失败：{e}"))),
        (true, true) => Err(bad("入站帧同时含 `op` 与 `kind`，方向不明")),
        (false, false) => Err(bad("入站帧既无 `op` 也无 `kind`，方向不明")),
    }
}

/// 读一帧并判别方向；EOF（`UnexpectedEof`）照常上抛，由调用方决定「正常退出还是错误」。
pub fn read_inbound<R: Read>(r: &mut R) -> io::Result<Inbound> {
    let payload = read_frame(r)?;
    let value: serde_json::Value = serde_json::from_slice(&payload).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("JSON 反序列化失败：{e}"))
    })?;
    classify_inbound(value)
}

// ───────────────────────────── 插件侧服务循环 ─────────────────────────────

/// worker 侧的对端句柄：拥有收发通道，承载「等响应期间仍能收到对端帧」的重入语义。
///
/// ## 为什么需要它（`PLAN-everything-is-plugin.md` §15.4 ①④）
///
/// 一旦插件能主动调宿主（`call_seam` / `emit`），同一条管道上就会**同时**挂着两个方向：
/// 我方请求的响应、对端发来的新请求。朴素的「写一帧读一帧」在此时必然帧错配
/// （读到的是对端请求，却被当成自己的响应）。故：
///
/// - 读到的帧按 [`Inbound`] 分流：自己等的**响应**才返回；
/// - 对端发来的**请求**进 `deferred` 队列（**不丢、不阻塞**），由 [`Peer::run`] 的主循环
///   按 FIFO 处理 —— 于是「重入」既不会死锁，也不会让对端的请求凭空消失。
///
/// 注意：本结构体是**单线程同步**的（reader/writer 按值拥有，无内部锁）。
/// 宿主侧需要并发多路复用，走的是「读者线程 + `request_id` 配对表」的另一套实现
/// （见 `axagent-plugins` 的 `worker.rs`），不共用本结构体。
pub struct Peer<R: Read, W: Write> {
    /// 收方向（对端 → 我方）。
    reader: R,
    /// 发方向（我方 → 对端）。
    writer: W,
    /// 自增配对 ID 的序号（ID 形如 `p1`、`p2`…，`p` 表示 plugin 侧发起）。
    next_request_id: u64,
    /// 等待响应期间收到的对端请求（FIFO，由主循环消费）。
    deferred: VecDeque<FrameRequest>,
}

impl<R: Read, W: Write> Peer<R, W> {
    /// 用一对收发通道构造。
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer, next_request_id: 0, deferred: VecDeque::new() }
    }

    /// 主循环：处理请求帧（含先前被推迟的），直到对端关闭。
    ///
    /// **读到 EOF（父进程关闭管道）即正常返回 `Ok(())`** —— 这是「孤儿进程自退」的主防线：
    /// 宿主异常退出后 worker 不会变成无人回收的僵尸进程，而是自然结束。
    /// 其余 I/O 错误与畸形帧照常向上返回 `Err`，交由调用方决定是否退出。
    ///
    /// 处理器返回的响应会**自动回填 `request_id`**（取自对应请求）—— 插件作者不必操心配对。
    pub fn run<F>(&mut self, mut handler: F) -> io::Result<()>
    where
        F: FnMut(&FrameRequest, &mut Self) -> FrameResponse,
    {
        loop {
            let req = match self.deferred.pop_front() {
                Some(deferred) => deferred,
                None => match read_inbound(&mut self.reader) {
                    Ok(Inbound::Request(req)) => req,
                    // 主循环里出现响应帧 ⇒ 没有等待者的响应，协议失序。明确报错，不静默丢弃。
                    Ok(Inbound::Response(resp)) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "主循环收到无等待者的响应帧（request_id={:?}）",
                                resp.request_id
                            ),
                        ));
                    },
                    // EOF：管道已关闭，正常退出（不是错误）。
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                },
            };
            let mut resp = handler(&req, self);
            // 回填配对 ID：否则对端无法把这一帧与它发出的请求对上。
            resp.request_id = req.request_id.clone();
            write_json_frame(&mut self.writer, &resp)?;
        }
    }

    /// 向对端发一个请求并**同步等待**配对响应。
    ///
    /// `call_chain` 为该次调用的调用链（见 [`FrameRequest::call_chain`]）；
    /// 等待期间收到的对端请求进 `deferred`，由 [`Peer::run`] 后续处理。
    ///
    /// 读到 `request_id` 不匹配的响应 ⇒ 立刻 `Err`（不错配、不猜）。
    pub fn request(
        &mut self,
        op: &str,
        args: serde_json::Value,
        call_chain: Vec<String>,
    ) -> io::Result<FrameResponse> {
        self.next_request_id += 1;
        let id = format!("p{}", self.next_request_id);
        let req = FrameRequest::new(op, args).with_id(id.clone()).with_chain(call_chain);
        write_json_frame(&mut self.writer, &req)?;
        loop {
            match read_inbound(&mut self.reader)? {
                Inbound::Response(resp) => {
                    if resp.request_id.as_deref() == Some(id.as_str()) {
                        return Ok(resp);
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("响应帧 request_id 不匹配：期望 {id}，收到 {:?}", resp.request_id),
                    ));
                },
                Inbound::Request(req) => self.deferred.push_back(req),
            }
        }
    }

    /// 请求宿主代为调用另一条接缝（L2 跨插件调用）。
    ///
    /// `call_chain` 必须**原样带上入站帧的链**（见模块文档示例），否则宿主侧的环检测
    /// 收不到这条边。宿主侧还会校验「本插件是否在声明 `calls` 里列了这条接缝」。
    pub fn call_seam(
        &mut self,
        seam: &str,
        op: &str,
        args: serde_json::Value,
        call_chain: Vec<String>,
    ) -> io::Result<FrameResponse> {
        self.request(
            ops::CALL_SEAM,
            serde_json::json!({ "seam": seam, "op": op, "args": args }),
            call_chain,
        )
    }

    /// 请求宿主代为派发一个领域事件（L1 事件桥的写方向）。
    pub fn emit(
        &mut self,
        category: &str,
        kind: &str,
        payload: serde_json::Value,
        source: &str,
    ) -> io::Result<FrameResponse> {
        self.request(
            ops::EMIT,
            serde_json::json!({
                "category": category,
                "kind": kind,
                "payload": payload,
                "source": source,
            }),
            Vec::new(),
        )
    }
}

/// worker 侧服务循环：读一帧 → 交给 `handler` → 回写一帧，直到对端关闭。
///
/// 收发通道**按值**交给 [`Peer`]（而非 `&mut` 借用）：后者会在 `FnMut` 的
/// `&mut Peer<..>` 形参上引入双重晚绑定（HRTB），极易触发
/// 「closure not general enough」。按值拥有之后泛型只有一层，闭包类型推断顺畅。
pub fn serve_with<R: Read, W: Write, F>(r: R, w: W, handler: F) -> io::Result<()>
where
    F: FnMut(&FrameRequest, &mut Peer<R, W>) -> FrameResponse,
{
    Peer::new(r, w).run(handler)
}

/// [`serve_with`] 的便捷封装：用加锁的 stdin / stdout 作为收发通道。
pub fn serve<F>(handler: F) -> io::Result<()>
where
    F: FnMut(
        &FrameRequest,
        &mut Peer<io::StdinLock<'static>, io::StdoutLock<'static>>,
    ) -> FrameResponse,
{
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve_with(stdin.lock(), stdout.lock(), handler)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造「长度字段 + 负载」的原始帧流（长度字段可刻意写错，用于畸形输入用例）。
    fn raw_frame(len: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(4 + payload.len());
        v.extend_from_slice(&len.to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    /// 构造长度字段正确的原始帧。
    fn frame_of(payload: &[u8]) -> Vec<u8> {
        raw_frame(payload.len() as u32, payload)
    }

    /// 构造带指定 op 的请求（避免测试里反复写结构体字面量）。
    fn req_of(op: &str) -> FrameRequest {
        FrameRequest::new(op, serde_json::Value::Null)
    }

    /// 构造一个能力声明（`seam` / `version` 可传空串以覆盖校验失败分支）。
    fn capability(seam: &str, version: &str) -> CapabilityDecl {
        CapabilityDecl {
            seam: seam.to_owned(),
            op: ops::EVENT.to_owned(),
            version: version.to_owned(),
        }
    }

    /// 构造一份版本合法的声明。
    fn decl_of(capabilities: Vec<CapabilityDecl>) -> PluginDeclaration {
        PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: vec!["event.dispatch".to_owned()],
            calls: vec!["storage.kv".to_owned()],
            capabilities,
        }
    }

    #[test]
    fn frame_roundtrip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"hello").expect("写帧应成功");
        let mut r: &[u8] = &buf;
        assert_eq!(read_frame(&mut r).expect("读帧应成功"), b"hello".to_vec());

        // 空负载同样要能往返（长度字段为 0）。
        let mut buf = Vec::new();
        write_frame(&mut buf, b"").expect("写空帧应成功");
        let mut r: &[u8] = &buf;
        assert_eq!(read_frame(&mut r).expect("读空帧应成功"), Vec::<u8>::new());
    }

    #[test]
    fn json_frame_roundtrip() {
        let req = req_of(ops::DESCRIBE);
        let mut buf = Vec::new();
        write_json_frame(&mut buf, &req).expect("写 JSON 帧应成功");

        let mut r: &[u8] = &buf;
        let back: FrameRequest = read_json_frame(&mut r).expect("读 JSON 帧应成功");
        assert_eq!(back.op, ops::DESCRIBE);
        assert_eq!(back.args, serde_json::Value::Null);

        // 缺 `args` 字段的 JSON 走 `#[serde(default)]`，应解析为 `null` 而非报错。
        let raw = frame_of(b"{\"op\":\"shutdown\"}");
        let mut r: &[u8] = &raw;
        let back: FrameRequest = read_json_frame(&mut r).expect("缺 args 应能解析");
        assert_eq!(back.op, ops::SHUTDOWN);
        assert_eq!(back.args, serde_json::Value::Null);
    }

    /// 畸形输入 ①：长度字段与实际字节数不符 ⇒ `Err`（不得 panic / 死锁）。
    #[test]
    fn frame_len_field_larger_than_body_is_err() {
        let raw = raw_frame(10, b"abcd");
        let mut r: &[u8] = &raw;
        let err = read_frame(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    /// 畸形输入 ②：长度超出上限 ⇒ 立刻 `Err`，且**必须**在分配前拒绝。
    #[test]
    fn frame_len_over_limit_is_err() {
        let raw = raw_frame(MAX_FRAME_LEN as u32 + 1, b"");
        let mut r: &[u8] = &raw;
        let err = read_frame(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // 逼近 u32 上限的长度字段同样被拒（若先分配会尝试申请近 4 GiB）。
        let raw = raw_frame(u32::MAX, b"");
        let mut r: &[u8] = &raw;
        let err = read_frame(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// 畸形输入 ③：截断帧（长度头不全 / 负载中途 EOF）⇒ `Err`。
    #[test]
    fn truncated_frame_is_err() {
        // 长度头只读到 3 字节即 EOF。
        let raw = [0u8, 0, 0];
        let mut r: &[u8] = &raw;
        assert!(read_frame(&mut r).is_err());

        let raw = raw_frame(8, b"abc");
        let mut r: &[u8] = &raw;
        assert!(read_frame(&mut r).is_err());
    }

    /// 畸形输入 ④：非法 JSON（非法 UTF-8 / 语法错误）⇒ `Err`。
    #[test]
    fn invalid_json_payload_is_err() {
        let raw = frame_of(&[0xff, 0xfe, 0xfd]);
        let mut r: &[u8] = &raw;
        let err = read_json_frame::<_, FrameRequest>(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        let raw = frame_of(b"{\"op\":");
        let mut r: &[u8] = &raw;
        let err = read_json_frame::<_, FrameRequest>(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// 畸形输入 ⑤：合法 JSON 但缺 `kind` 字段 ⇒ `Err`（`kind` 无 default，不得 panic）。
    #[test]
    fn json_missing_kind_field_is_err() {
        let raw = frame_of(b"{\"value\":1}");
        let mut r: &[u8] = &raw;
        let err = read_json_frame::<_, FrameResponse>(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn frame_response_constructors_and_shape() {
        let ok = FrameResponse::success(serde_json::json!({ "n": 1 }));
        assert_eq!(ok.kind, ResponseKind::Success);
        assert_eq!(ok.value, Some(serde_json::json!({ "n": 1 })));
        assert!(ok.message.is_none());
        assert!(ok.code.is_none());

        let text = serde_json::to_string(&ok).expect("序列化成功响应");
        assert!(text.contains("\"kind\":\"success\""));
        assert!(text.contains("\"value\":{\"n\":1}"));
        // 空字段被 skip_serializing_if 跳过。
        assert!(!text.contains("message"));
        assert!(!text.contains("code"));

        let err = FrameResponse::error("PLUGIN_UNKNOWN_OP", "未知操作");
        assert_eq!(err.kind, ResponseKind::Error);
        assert!(err.value.is_none());
        assert_eq!(err.code.as_deref(), Some("PLUGIN_UNKNOWN_OP"));
        assert_eq!(err.message.as_deref(), Some("未知操作"));

        let text = serde_json::to_string(&err).expect("序列化错误响应");
        assert!(text.contains("\"kind\":\"error\""));
        assert!(text.contains("\"code\":\"PLUGIN_UNKNOWN_OP\""));
        assert!(!text.contains("value"));
    }

    #[test]
    fn declaration_serde_roundtrip_uses_snake_case() {
        let cap = capability("seam.demo", "1.0.0");
        let decl = decl_of(vec![cap]);

        let text = serde_json::to_string(&decl).expect("序列化声明");
        // 进程内协议：字段名一律 snake_case，不得 camelCase。
        assert!(text.contains("\"proto_version\""));
        assert!(text.contains("\"subscribe\""));
        assert!(text.contains("\"calls\""));
        assert!(text.contains("\"capabilities\""));

        let back: PluginDeclaration = serde_json::from_str(&text).expect("反序列化声明");
        assert!(back.validate().is_ok());
        assert_eq!(back.capabilities.len(), 1);
        assert_eq!(back.capabilities[0].seam, "seam.demo");

        // 三个可选列表字段全部缺省时也应能解析为空 Vec。
        let minimal = frame_of(b"{\"proto_version\":1}");
        let mut r: &[u8] = &minimal;
        let back: PluginDeclaration = read_json_frame(&mut r).expect("最小声明应能解析");
        assert!(back.subscribe.is_empty());
        assert!(back.calls.is_empty());
        assert!(back.capabilities.is_empty());
        assert!(back.validate().is_ok());
    }

    #[test]
    fn declaration_validate_rejects_bad_input() {
        assert!(decl_of(Vec::new()).validate().is_ok());

        let mut bad_version = decl_of(Vec::new());
        bad_version.proto_version = AXAGENT_PLUGIN_PROTO_VERSION + 1;
        let text = bad_version.validate().unwrap_err().to_string();
        assert!(text.starts_with("协议版本不匹配"));

        let mut empty_seam = decl_of(Vec::new());
        empty_seam.capabilities = vec![capability("", "1.0.0")];
        let text = empty_seam.validate().unwrap_err().to_string();
        assert!(text.starts_with("能力声明缺少接缝 ID"));

        let mut empty_version = decl_of(Vec::new());
        empty_version.capabilities = vec![capability("seam.demo", "")];
        let text = empty_version.validate().unwrap_err().to_string();
        assert!(text.starts_with("能力声明缺少版本号"));
    }

    #[test]
    fn serve_with_processes_frames_in_order_then_exits_on_eof() {
        let mut input = Vec::new();
        write_json_frame(&mut input, &req_of(ops::DESCRIBE)).expect("写请求 1");
        write_json_frame(&mut input, &req_of(ops::SHUTDOWN)).expect("写请求 2");

        let mut output = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut r: &[u8] = &input;
        serve_with(&mut r, &mut output, |req: &FrameRequest, _peer| {
            seen.push(req.op.clone());
            FrameResponse::success(serde_json::json!({ "op": req.op }))
        })
        .expect("两帧处理完后读到 EOF 应正常返回 Ok(())");

        // 请求按序被处理。
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], ops::DESCRIBE);
        assert_eq!(seen[1], ops::SHUTDOWN);

        // 响应按序写回，且与请求一一对应。
        let mut r: &[u8] = &output;
        let first: FrameResponse = read_json_frame(&mut r).expect("读响应 1");
        assert_eq!(first.kind, ResponseKind::Success);
        assert_eq!(first.value, Some(serde_json::json!({ "op": ops::DESCRIBE })));
        let second: FrameResponse = read_json_frame(&mut r).expect("读响应 2");
        assert_eq!(second.kind, ResponseKind::Success);
        assert_eq!(second.value, Some(serde_json::json!({ "op": ops::SHUTDOWN })));
        // 没有多余的第三帧。
        assert!(read_frame(&mut r).is_err());
    }

    #[test]
    fn serve_with_returns_ok_on_immediate_eof() {
        let mut input: &[u8] = &[];
        let mut output = Vec::new();
        let mut calls = 0usize;
        serve_with(&mut input, &mut output, |_req: &FrameRequest, _peer| {
            calls += 1;
            FrameResponse::success(serde_json::Value::Null)
        })
        .expect("空输入应正常退出");
        assert_eq!(calls, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn serve_with_propagates_malformed_frame() {
        // 畸形帧（缺 op，无法反序列化为 FrameRequest）不应让服务循环 panic，而是把 Err 交给调用方。
        let input = frame_of(b"{\"value\":1}");
        let mut output = Vec::new();
        let mut r: &[u8] = &input;
        let err = serve_with(&mut r, &mut output, |_req: &FrameRequest, _peer| {
            FrameResponse::success(serde_json::Value::Null)
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(output.is_empty());
    }

    // ─────────────────── P4：配对 ID / 调用链 / 方向判别 / Peer ───────────────────

    /// P4：`request_id` / `call_chain` 缺省时**不得**出现在帧体里（保持帧紧凑），
    /// 显式设置时按 snake_case 序列化，且老帧（无这两个字段）仍必须能解析。
    #[test]
    fn frame_request_optional_fields_are_skipped_when_default() {
        let text = serde_json::to_string(&req_of(ops::DESCRIBE)).expect("序列化请求");
        assert!(!text.contains("request_id"), "缺省配对 ID 不应写入帧体：{text}");
        assert!(!text.contains("call_chain"), "空调用链不应写入帧体：{text}");

        let chain = vec!["plugin.a".to_owned(), "plugin.b".to_owned()];
        let req = FrameRequest::new(ops::CALL_SEAM, serde_json::json!({ "seam": "s" }))
            .with_id("p1")
            .with_chain(chain.clone());
        let text = serde_json::to_string(&req).expect("序列化带配对的请求");
        assert!(text.contains("\"request_id\":\"p1\""));
        assert!(text.contains("\"call_chain\":[\"plugin.a\",\"plugin.b\"]"));

        let back: FrameRequest = serde_json::from_str(&text).expect("反序列化带配对的请求");
        assert_eq!(back.request_id.as_deref(), Some("p1"));
        assert_eq!(back.call_chain, chain);

        // 协议向后兼容：早于 P4 的帧（无这两个字段）必须照常解析。
        let raw = frame_of(b"{\"op\":\"shutdown\"}");
        let mut r: &[u8] = &raw;
        let back: FrameRequest = read_json_frame(&mut r).expect("旧帧应能解析");
        assert!(back.request_id.is_none());
        assert!(back.call_chain.is_empty());
    }

    /// P4：响应帧的 `request_id` 同样「缺省省略、显式保留」。
    #[test]
    fn frame_response_request_id_is_optional_in_wire_format() {
        let text = serde_json::to_string(&FrameResponse::success(serde_json::json!(1)))
            .expect("序列化响应");
        assert!(!text.contains("request_id"));

        let text =
            serde_json::to_string(&FrameResponse::success(serde_json::json!(1)).with_id("h7"))
                .expect("序列化带配对的响应");
        assert!(text.contains("\"request_id\":\"h7\""));

        let back: FrameResponse = serde_json::from_str(&text).expect("反序列化");
        assert_eq!(back.request_id.as_deref(), Some("h7"));
    }

    /// P4：入站帧方向判别四态 —— 只有「恰好一个」判别字段才算合法；方向不明**不猜**。
    #[test]
    fn classify_inbound_four_states() {
        let req = classify_inbound(serde_json::json!({ "op": "event", "args": {} }))
            .expect("带 op 应判为请求");
        assert_eq!(req.as_request().map(|r| r.op.as_str()), Some(ops::EVENT));
        assert!(req.as_response().is_none());

        let resp = classify_inbound(serde_json::json!({ "kind": "success", "value": 1 }))
            .expect("带 kind 应判为响应");
        assert!(resp.as_request().is_none());
        assert_eq!(resp.as_response().map(|r| r.kind), Some(ResponseKind::Success));

        // 两者皆有 / 两者皆无 / 非对象 ⇒ 一律 InvalidData。
        for bad in [
            serde_json::json!({ "op": "event", "kind": "success" }),
            serde_json::json!({ "args": {} }),
            serde_json::json!(42),
        ] {
            let err = classify_inbound(bad).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        }
    }

    /// P4：`read_inbound` 从字节流读帧并判别方向，EOF 原样上抛（由主循环翻译成「正常退出」）。
    #[test]
    fn read_inbound_classifies_and_propagates_eof() {
        let mut input = Vec::new();
        write_json_frame(&mut input, &req_of(ops::EVENT)).expect("写请求帧");
        write_json_frame(
            &mut input,
            &FrameResponse::success(serde_json::Value::Null).with_id("p1"),
        )
        .expect("写响应帧");

        let mut r: &[u8] = &input;
        assert!(read_inbound(&mut r).expect("第一帧").as_request().is_some());
        let second = read_inbound(&mut r).expect("第二帧");
        assert_eq!(second.as_response().and_then(|resp| resp.request_id.as_deref()), Some("p1"));

        let err = read_inbound(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    /// P4 回归（最要紧）：等待响应期间读到**对端请求**时必须继续等自己的配对响应，
    /// 绝不能把它误当成响应（帧错配）；同时该请求要**排队而非丢弃**，由主循环补处理。
    #[test]
    fn peer_request_skips_interleaved_peer_request_then_run_drains_it() {
        let mut input = Vec::new();
        // ① 对端（宿主）在我们等待期间又发来一帧请求。
        write_json_frame(&mut input, &req_of(ops::EVENT).with_id("h1")).expect("写对端请求");
        // ② 我们等的那条响应排在对端请求之后。
        write_json_frame(
            &mut input,
            &FrameResponse::success(serde_json::json!({ "echo": true })).with_id("p1"),
        )
        .expect("写配对响应");

        let mut output = Vec::new();
        let mut handled: Vec<String> = Vec::new();
        {
            let mut peer = Peer::new(input.as_slice(), &mut output);
            let resp = peer
                .request(
                    ops::CALL_SEAM,
                    serde_json::json!({ "seam": "workflow.sandbox" }),
                    Vec::new(),
                )
                .expect("应跳过对端请求、取到配对响应");
            assert_eq!(resp.value, Some(serde_json::json!({ "echo": true })));

            // 主循环随后消费被推迟的对端请求（输入已耗尽 ⇒ 处理完即 EOF 正常返回）。
            peer.run(|req, _peer| {
                handled.push(req.op.clone());
                FrameResponse::success(serde_json::json!({ "ok": true }))
            })
            .expect("EOF 应正常返回 Ok(())");
        }

        assert_eq!(handled, vec![ops::EVENT.to_owned()], "被推迟的对端请求必须仍被处理，不得丢弃");

        // 我们写出的两帧：先是带自增 ID 的请求，再是对对端请求的响应（且回填了它的 ID）。
        let mut r: &[u8] = &output;
        let sent: FrameRequest = read_json_frame(&mut r).expect("读我方请求帧");
        assert_eq!(sent.op, ops::CALL_SEAM);
        assert_eq!(sent.request_id.as_deref(), Some("p1"), "插件侧 ID 形如 p<n>");
        let reply: FrameResponse = read_json_frame(&mut r).expect("读我方响应帧");
        assert_eq!(reply.request_id.as_deref(), Some("h1"), "响应必须回填请求的 request_id");
    }

    /// P4：`call_seam` / `emit` 的帧形状（宿主侧按同一形状解析，两边不得漂移）。
    #[test]
    fn peer_call_seam_and_emit_frame_shapes() {
        let mut output = Vec::new();
        {
            // 空输入 ⇒ 写完帧立刻 EOF ⇒ 等不到响应。必须是 `Err`，**不得挂起、不得 panic**。
            let mut peer = Peer::new(&b""[..], &mut output);
            assert!(
                peer.call_seam(
                    "workflow.business_rule",
                    "evaluate",
                    serde_json::json!({ "n": 1 }),
                    vec!["plugin.a".to_owned()],
                )
                .is_err(),
                "对端关闭后应返回 Err"
            );
            assert!(
                peer.emit("agent", "TurnStarted", serde_json::json!({ "i": 1 }), "plugin.demo")
                    .is_err()
            );
        }

        let mut r: &[u8] = &output;
        let call: FrameRequest = read_json_frame(&mut r).expect("读 call_seam 帧");
        assert_eq!(call.op, ops::CALL_SEAM);
        assert_eq!(call.args["seam"], serde_json::json!("workflow.business_rule"));
        assert_eq!(call.args["op"], serde_json::json!("evaluate"));
        assert_eq!(call.args["args"], serde_json::json!({ "n": 1 }));
        assert_eq!(call.request_id.as_deref(), Some("p1"));
        assert_eq!(call.call_chain, vec!["plugin.a".to_owned()]);

        let emit: FrameRequest = read_json_frame(&mut r).expect("读 emit 帧");
        assert_eq!(emit.op, ops::EMIT);
        assert_eq!(emit.args["category"], serde_json::json!("agent"));
        assert_eq!(emit.args["kind"], serde_json::json!("TurnStarted"));
        assert_eq!(emit.args["source"], serde_json::json!("plugin.demo"));
        assert_eq!(emit.request_id.as_deref(), Some("p2"), "配对 ID 逐次自增");
        assert!(emit.call_chain.is_empty(), "emit 不是跨插件调用，不带调用链");
    }

    /// P4：响应 ID 不匹配 ⇒ 立刻 `Err`（宁可失败，也不把别人的返回值当自己的）。
    #[test]
    fn peer_request_rejects_mismatched_response_id() {
        let mut input = Vec::new();
        write_json_frame(
            &mut input,
            &FrameResponse::success(serde_json::json!(null)).with_id("p9"),
        )
        .expect("写错配响应");

        let mut output = Vec::new();
        let mut peer = Peer::new(input.as_slice(), &mut output);
        let err = peer.request(ops::CALL_SEAM, serde_json::Value::Null, Vec::new()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("不匹配"), "错误信息需点明错配：{err}");
    }

    /// P4：主循环收到「无等待者的响应帧」⇒ 协议失序，明确 `Err`（不静默丢弃）。
    #[test]
    fn peer_run_rejects_stray_response_frame() {
        let mut input = Vec::new();
        write_json_frame(&mut input, &FrameResponse::success(serde_json::json!(1)).with_id("p1"))
            .expect("写孤儿响应");

        let mut output = Vec::new();
        let mut peer = Peer::new(input.as_slice(), &mut output);
        let err =
            peer.run(|_req, _peer| FrameResponse::success(serde_json::Value::Null)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// P4：调用链深度上限与错误码都是协议常量，宿主与 worker 共用同一份。
    #[test]
    fn protocol_constants_are_shared_shape() {
        assert_eq!(MAX_SEAM_CALL_DEPTH, 8, "§15.5 定的深度上限为 8");
        // 错误码即码串本身（便于宿主直接透传给插件作者看）。
        assert_eq!(error_codes::SEAM_CALL_CYCLE_DETECTED, "SEAM_CALL_CYCLE_DETECTED");
        assert_eq!(error_codes::SEAM_CALL_DEPTH_EXCEEDED, "SEAM_CALL_DEPTH_EXCEEDED");
        assert_eq!(error_codes::SEAM_PROVIDER_UNAVAILABLE, "SEAM_PROVIDER_UNAVAILABLE");
    }

    // ─────────────────── 任务 #49：命令帧（T2 派发口） ───────────────────

    /// 命令帧**请求型**经传输层往返：编码 → 帧字节 → 解码 → 逐字段一致；
    /// 并锁定线格式为 snake_case（进程内协议口径，与 [`FrameRequest`] 同族）。
    #[test]
    fn command_frame_request_roundtrip_over_wire() {
        let frame = CommandFrame::new(
            "demo@external",
            "parse_analysis_intent",
            serde_json::json!({ "input": "调研茅台短线" }),
            12_345,
        );
        let request = frame.to_frame_request().expect("命令帧编码应成功");
        assert_eq!(request.op, ops::COMMAND);
        let text = serde_json::to_string(&request.args).expect("args 应可序列化");
        assert!(text.contains("\"plugin_id\""), "线格式必须 snake_case：{text}");
        assert!(text.contains("\"timeout_ms\""), "超时字段必填：{text}");
        assert!(!text.contains("pluginId"), "不得出现 camelCase：{text}");

        let mut buf = Vec::new();
        write_json_frame(&mut buf, &request).expect("写帧应成功");
        let mut r: &[u8] = &buf;
        let back: FrameRequest = read_json_frame(&mut r).expect("读帧应成功");
        let decoded = CommandFrame::from_frame_request(&back).expect("命令帧解码应成功");
        assert_eq!(decoded, frame, "往返必须逐字段一致");
    }

    /// 命令帧请求体的畸形输入：非 command op、缺 `timeout_ms`、input 缺省三类形态。
    #[test]
    fn command_frame_request_rejects_malformed_or_foreign() {
        // ① 别的 op ⇒ NotCommand（绝不按命令帧猜）。
        let foreign = FrameRequest::new(ops::EVENT, serde_json::json!({ "plugin_id": "a" }));
        let err = CommandFrame::from_frame_request(&foreign).unwrap_err();
        assert!(matches!(err, CommandFrameError::NotCommand { op } if op == ops::EVENT));

        // ② 缺必填 timeout_ms ⇒ InvalidShape（不给「无限预算」的静默形态）。
        let no_budget = FrameRequest::new(
            ops::COMMAND,
            serde_json::json!({
                "plugin_id": "a@external", "command": "x"
            }),
        );
        let err = CommandFrame::from_frame_request(&no_budget).unwrap_err();
        assert!(matches!(err, CommandFrameError::InvalidShape { .. }), "实际 {err}");

        // ③ input 缺省合法（`#[serde(default)]` ⇒ null）。
        let no_input = FrameRequest::new(
            ops::COMMAND,
            serde_json::json!({
                "plugin_id": "a@external", "command": "x", "timeout_ms": 1000
            }),
        );
        let frame = CommandFrame::from_frame_request(&no_input).expect("input 可缺省");
        assert_eq!(frame.input, serde_json::Value::Null);
    }

    /// 命令帧**成功回执型**经传输层往返，且宿主绑定字段随回执透传。
    #[test]
    fn command_frame_reply_roundtrip_over_wire() {
        let frame = CommandFrame::new("demo@external", "ping", serde_json::json!(null), 1000);
        let reply = CommandReply::for_frame(&frame, serde_json::json!({ "pong": true }));
        let response = CommandOutcome::Reply(reply.clone())
            .into_frame_response()
            .expect("回执编码应成功")
            .with_id("h3");
        assert_eq!(response.kind, ResponseKind::Success);

        let mut buf = Vec::new();
        write_json_frame(&mut buf, &response).expect("写回执帧应成功");
        let mut r: &[u8] = &buf;
        let back: FrameResponse = read_json_frame(&mut r).expect("读回执帧应成功");
        assert_eq!(back.request_id.as_deref(), Some("h3"), "配对 ID 必须随信封往返");
        let outcome = CommandOutcome::from_frame_response(&back).expect("回执解码应成功");
        assert_eq!(outcome, CommandOutcome::Reply(reply));
        let CommandOutcome::Reply(parsed) = outcome else {
            panic!("成功帧必须解为 Reply");
        };
        assert_eq!(parsed.plugin_id, "demo@external", "回执必须携带宿主绑定");
        assert_eq!(parsed.command, "ping");
    }

    /// 命令帧**失败回执型**往返：错误码、说明与绑定字段都要存活。
    #[test]
    fn command_frame_fault_roundtrip_over_wire() {
        let frame = CommandFrame::new("demo@external", "boom", serde_json::json!(null), 1000);
        let fault =
            CommandFault::for_frame(&frame, error_codes::PLUGIN_COMMAND_FAILED, "worker 自报失败");
        let response =
            CommandOutcome::Fault(fault.clone()).into_frame_response().expect("失败回执编码应成功");
        assert_eq!(response.kind, ResponseKind::Error);

        let mut buf = Vec::new();
        write_json_frame(&mut buf, &response).expect("写失败帧应成功");
        let mut r: &[u8] = &buf;
        let back: FrameResponse = read_json_frame(&mut r).expect("读失败帧应成功");
        let outcome = CommandOutcome::from_frame_response(&back).expect("失败回执解码应成功");
        assert_eq!(outcome, CommandOutcome::Fault(fault));
    }

    /// 加法式兼容（**版本不 bump** 的依据）：
    /// ① 命令帧仍带 `op` ⇒ [`classify_inbound`] 判为请求，入站分流逻辑零改动；
    /// ② 旧 worker 对未实现 op 回 `PLUGIN_UNKNOWN_OP` 错误帧 ⇒ 新宿主解码为 Fault（降级为失败，
    ///    不会误成功）；缺 `code` 的裸错误帧也兜底成 `PLUGIN_COMMAND_FAILED`。
    #[test]
    fn command_frame_is_additive_no_proto_bump_required() {
        assert_eq!(
            AXAGENT_PLUGIN_PROTO_VERSION, 1,
            "命令帧是加法式 op：握手版本必须保持 1，bump 会拒载全部已装插件"
        );

        let frame = CommandFrame::new("demo@external", "x", serde_json::json!(null), 1000);
        let request = frame.to_frame_request().expect("编码应成功");
        let inbound = classify_inbound(serde_json::to_value(&request).expect("转 Value"))
            .expect("命令帧必须仍按 op 判为请求");
        assert!(inbound.as_request().is_some(), "入站分流零改动");

        // 旧 worker 的降级错误帧。
        let legacy =
            FrameResponse::error(error_codes::PLUGIN_UNKNOWN_OP, "未知操作：command").with_id("h1");
        let outcome = CommandOutcome::from_frame_response(&legacy).expect("错误帧解码应成功");
        let CommandOutcome::Fault(fault) = outcome else {
            panic!("错误帧必须解为 Fault");
        };
        assert_eq!(fault.code, error_codes::PLUGIN_UNKNOWN_OP);
        assert!(fault.plugin_id.is_empty(), "旧 worker 无绑定字段，允许为空");

        // 连 code 都没有的裸错误帧 ⇒ 兜底码，不 panic 也不当成成功。
        let bare = FrameResponse {
            kind: ResponseKind::Error,
            value: None,
            message: Some("对端异常".to_string()),
            code: None,
            request_id: None,
        };
        let outcome = CommandOutcome::from_frame_response(&bare).expect("兜底解码应成功");
        let CommandOutcome::Fault(fault) = outcome else {
            panic!("错误帧必须解为 Fault");
        };
        assert_eq!(fault.code, error_codes::PLUGIN_COMMAND_FAILED);
    }

    /// 成功帧 `value` 不是 CommandReply 形状（裸输出）⇒ InvalidShape，**不猜**。
    #[test]
    fn command_reply_shape_is_strict_on_success() {
        let bare = FrameResponse::success(serde_json::json!("直接一段文本"));
        let err = CommandOutcome::from_frame_response(&bare).unwrap_err();
        assert!(matches!(err, CommandFrameError::InvalidShape { .. }), "实际 {err}");
    }

    /// 命令帧相关协议常量与错误码逐字自洽（宿主与 worker 共用同一份）。
    #[test]
    fn command_frame_protocol_constants_are_shared_shape() {
        assert_eq!(ops::COMMAND, "command");
        assert_eq!(DEFAULT_COMMAND_TIMEOUT_MS, 30_000);
        assert_eq!(error_codes::PLUGIN_COMMAND_UNKNOWN, "PLUGIN_COMMAND_UNKNOWN");
        assert_eq!(error_codes::PLUGIN_COMMAND_INVALID_ARGS, "PLUGIN_COMMAND_INVALID_ARGS");
        assert_eq!(error_codes::PLUGIN_COMMAND_ID_MISMATCH, "PLUGIN_COMMAND_ID_MISMATCH");
        assert_eq!(error_codes::PLUGIN_COMMAND_TIMEOUT, "PLUGIN_COMMAND_TIMEOUT");
        assert_eq!(error_codes::PLUGIN_COMMAND_FAILED, "PLUGIN_COMMAND_FAILED");
    }

    /// 任务 #53（D1）：接缝调用预算是**协议常量**，宿主默认值从这里读、不散落魔数；
    /// 且必须严格小于命令帧预算（「同步夹在对话回合里」的口径），并**不**伴随版本 bump。
    #[test]
    fn seam_timeout_default_is_protocol_constant_below_command_budget() {
        // 编译期钉大小关系（`assertions_on_constants` 不许运行期比较两常量，
        // 而 const 断言更强：违反即在构建期红，不用等测试跑到）。
        const _: () = assert!(DEFAULT_SEAM_TIMEOUT_MS < DEFAULT_COMMAND_TIMEOUT_MS);
        assert_eq!(DEFAULT_SEAM_TIMEOUT_MS, 15_000);
        assert_eq!(
            AXAGENT_PLUGIN_PROTO_VERSION, 1,
            "接缝预算常量是宿主侧等待语义，不改线格式：握手版本必须保持 1"
        );
    }

    /// 任务 #53 立、任务 #76 升级：逐接缝预算表按证据收录，锁成**双向** ——
    /// - **(i) 表内每一项必须带盘上出处**（`文件:行` 原文），且查表必须命中该预算；
    /// - **(ii) 名单外的 op 才断言 `None`**（未进表即走默认，这是「不许拍脑袋」的机械锁）。
    ///
    /// ⚠ **这不是放宽，别读成「为了过测试改测试」**：旧形态是「23 个 op 恒断言 `None`，另把
    /// `run_turn` 单独断言 `Some`」；新形态是「22 个 op 断言 `None`，表内 2 项各断言 `Some`
    /// 且逐项带出处」⇒ **被断言的 op 名总数仍是 24（22 + 2），一个没少**。变的只是判据：
    /// 从「未进表者恒 `None`」改成「按有无盘上证据分两档，进表必须自证出处」。
    /// 于是「加一条**有**证据的表项」不再需要动本用例的口径（任务 #74 §7.1 抓到的「表被
    /// 冻死」形态），而「加一条**没**证据的表项」反而更早红 —— 见下面对
    /// [`SEAM_OP_TIMEOUT_OVERRIDES`] 的反向自检，旧形态缺的正是这一半。
    #[test]
    fn seam_timeout_override_table_is_evidence_bounded() {
        // ── (i) 表内每一项：出处 + 命中 ──
        // 出处一律写「文件:行」原文：改值必须连带改掉出处，抹掉或写成空话当场红
        // （判据只问「像不像一个源文件行」，真伪由注释里那串路径承担复核成本）。
        const EVIDENCED: &[(&str, u64, &str)] = &[
            (
                "run_turn",
                AGENT_LOOP_RUN_TURN_TIMEOUT_MS,
                "max(300s: axagent-ai/crates/providers/src/lib.rs:278, 900s: src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:4515)",
            ),
            (
                "exec",
                TOOL_SET_EXEC_TIMEOUT_MS,
                "axagent-plugins/crates/plugins/src/types.rs:1188 DEFAULT_TOOL_TIMEOUT_SECS = 60（用于同文件 :1218 / :1306）",
            ),
        ];
        for (op, timeout_ms, source) in EVIDENCED {
            assert!(source.contains(".rs:"), "表项 `{op}` 的出处必须指到源文件行：{source}");
            assert!(
                SEAM_OP_TIMEOUT_OVERRIDES.iter().any(|(key, _)| key == op),
                "表项 `{op}`（出处 {source}）应真的在覆盖表里，不许只在测试里自说自话"
            );
            assert_eq!(
                seam_timeout_ms_for_op(op),
                Some(*timeout_ms),
                "`{op}` 已获证据进表，必须命中表值"
            );
        }
        // 反向自检（本批新增的那一半）：表里不得出现未登记出处的 op ⇒ 进表即必须自证。
        for (table_op, _) in SEAM_OP_TIMEOUT_OVERRIDES {
            assert!(
                EVIDENCED.iter().any(|(op, _, _)| op == table_op),
                "覆盖表项 `{table_op}` 没有盘上出处登记 ⇒ 按判据不许进表"
            );
        }
        // 取值锁：run_turn = max(宿主 LLM 单请求上限 300 s, 生产 Agent 节点预算 900 s)；
        // exec = 宿主 spawn 通道的 60 s（§26.3 S-6 的对齐方向）。改这些数必须连带改掉上面的出处。
        const _: () = assert!(AGENT_LOOP_RUN_TURN_TIMEOUT_MS > DEFAULT_SEAM_TIMEOUT_MS);
        const _: () = assert!(TOOL_SET_EXEC_TIMEOUT_MS > DEFAULT_SEAM_TIMEOUT_MS);
        assert_eq!(AGENT_LOOP_RUN_TURN_TIMEOUT_MS, 900_000);
        assert_eq!(TOOL_SET_EXEC_TIMEOUT_MS, 60_000);

        // ── (ii) 名单外的 op 才断言 None ──
        // 这份名单逐条来自 §5.4 op 词汇 + worker.rs 门面硬编码词表；相对旧形态**只移出
        // `exec`**（它已按 (i) 带出处进表），其余 22 项逐字未动。
        for op in [
            "evaluate",
            "execute",
            "reflect",
            "evolve",
            "suggest",
            "on_message",
            "dispatch",
            "record",
            "assert",
            "start",
            "stop",
            "send",
            "is_enabled",
            "is_connected",
            "list",
            "chat",
            "embed",
            "ui.action",
            "call_seam",
            "event",
            "describe",
            "shutdown",
        ] {
            assert_eq!(seam_timeout_ms_for_op(op), None, "`{op}` 未获证据进表，必须走默认预算");
        }
        // 表自身一致性：键不得重复（重复即后键失效，静默漂移）。
        for (index, (left, _)) in SEAM_OP_TIMEOUT_OVERRIDES.iter().enumerate() {
            for (right, _) in SEAM_OP_TIMEOUT_OVERRIDES.iter().skip(index + 1) {
                assert_ne!(left, right, "覆盖表键重复：{left}");
            }
        }
    }

    /// 任务 #63（D1 边角）：握手预算是**独立的协议常量**，不复用接缝预算、
    /// 更不进逐接缝覆盖表 —— 本用例钉的就是「不许拿 seam 表凑数」这条口径。
    #[test]
    fn handshake_budget_is_a_separate_constant_and_not_a_seam_table_entry() {
        // 编译期钉关系：握手比接缝更该早收口（装载串行路径），且远小于命令预算。
        const _: () = assert!(DEFAULT_HANDSHAKE_TIMEOUT_MS < DEFAULT_SEAM_TIMEOUT_MS);
        const _: () = assert!(DEFAULT_HANDSHAKE_TIMEOUT_MS < DEFAULT_COMMAND_TIMEOUT_MS);
        assert_eq!(DEFAULT_HANDSHAKE_TIMEOUT_MS, 10_000);
        assert_eq!(
            AXAGENT_PLUGIN_PROTO_VERSION, 1,
            "握手预算是宿主侧等待语义，不改线格式：握手版本必须保持 1"
        );

        // `describe` **不得**进接缝覆盖表：握手不走接缝公共入口，进表既是假修复也会
        // 碎掉「表只给 seam op」的不变量（与 seam_timeout_override_table_is_evidence_bounded
        // 的名单互为佐证，这里单独点名是因为 #63 的诱惑正是「拿表包住 describe」）。
        assert_eq!(
            seam_timeout_ms_for_op(ops::DESCRIBE),
            None,
            "握手预算与接缝预算分通道：`describe` 不得出现在 seam 覆盖表里"
        );
    }

    /// 任务源行契约（PLAN §98）：线上键必须 camelCase，可缺省项缺省即不出现。
    ///
    /// 钉这条是因为宿主前端 `PluginTaskRunRowDto` 按 camelCase 逐字消费 —— 形状漂了
    /// 不会编译报错，只会让任务中心静默少一列。
    #[test]
    fn task_run_row_serializes_camel_case_with_optional_fields_omitted() {
        let row = PluginTaskRunRow {
            native_id: "exec-7".to_string(),
            name: "贵州茅台 600519 · 管线".to_string(),
            state: "running".to_string(),
            trigger: None,
            started_at: Some(1_700_000_000_000),
            finished_at: None,
            duration_ms: None,
            deep_link: Some("/invest/600519".to_string()),
            raw: None,
        };
        let value = serde_json::to_value(&row).expect("行记录应可序列化");
        assert_eq!(value["nativeId"], "exec-7");
        assert_eq!(value["state"], "running");
        assert_eq!(value["startedAt"], 1_700_000_000_000_i64);
        assert_eq!(value["deepLink"], "/invest/600519");
        for absent in ["trigger", "finishedAt", "durationMs", "raw", "native_id"] {
            assert!(value.get(absent).is_none(), "缺省字段 `{absent}` 不得进帧：{value}");
        }

        let back: PluginTaskRunRow =
            serde_json::from_value(value).expect("同一份 JSON 必须能解回原类型");
        assert_eq!(back.native_id, "exec-7");
        assert_eq!(back.trigger, None);
    }

    /// 状态词汇表是「表外值不猜」的判据来源，必须与宿主 `RunState` 七态逐字一致。
    #[test]
    fn task_run_state_vocabulary_is_exact_lowercase_seven_states() {
        assert_eq!(
            PLUGIN_TASK_RUN_STATES,
            ["queued", "running", "waiting", "paused", "succeeded", "failed", "canceled"]
        );
        assert!(is_plugin_task_run_state("canceled"));
        // 大小写宽容与美式拼法都不接受：静默接受就是把「不知道」伪装成可信读数。
        for wrong in ["Canceled", "cancelled", "", "completed"] {
            assert!(!is_plugin_task_run_state(wrong), "`{wrong}` 不得判成合法状态");
        }
    }
}
