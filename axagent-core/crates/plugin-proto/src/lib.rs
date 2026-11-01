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
}

/// 调用链长度上限（`PLAN-everything-is-plugin.md` §15.5）：不含本帧接收方。
///
/// 与环检测互补 —— 环检测挡「同一条链上重复出现」，深度上限挡「无环但极深」的长链
/// （每层跨插件调用都要占一份栈 + 一份子进程等待，深链会耗尽资源）。
pub const MAX_SEAM_CALL_DEPTH: usize = 8;

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
}
