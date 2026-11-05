// SPDX-License-Identifier: AGPL-3.0-only
//! OPC 域插件 worker —— 任务 #36 第 1 批**骨架**（零业务命令）。
//!
//! 宿主以 `worker.program` 相对路径 spawn 本进程，双方在 stdin/stdout 上交换长度前缀
//! JSON 帧（`axagent-plugin-proto`）。诊断信息一律走 stderr —— stdout 是协议通道，
//! 混入任何非帧字节都会让宿主读帧失败。
//!
//! 本批只做三件事，多一件都不做：
//! 1. **握手**：`describe` 回一份合法声明（协议版本 + 空 `subscribe` / 空 `calls` /
//!    空 `capabilities`）。空声明是**如实**描述零能力 —— 声明了却调不通，宿主会在
//!    启用期注册一条死接缝，比不声明更糟；
//! 2. **命令帧通道**：`command` op 只做「身份自检 + 显式未知命令」，让 T2 派发通道的
//!    错误面在骨架期就跑通（宿主发帧 → 得到带错误码的失败回执，而不是挂起或静默成功）；
//! 3. **停机**：`shutdown` 回执 + 等 stdin EOF 退出（与 `plugin-stock` 同一形态）。
//!
//! 为什么 `Peer` 而不是自写读循环：插件一旦能主动调宿主（下一批的首条只读接缝
//! `opc.store`），同一条管道上就同时挂着「我方请求的响应」与「对端发来的新请求」，
//! 朴素读一帧写一帧必然错配。`Peer` 把对端请求进 `deferred` 队列，等响应期间不丢帧。
//!
//! 为什么是纯可执行包（只有 `src/main.rs`、没有 `src/lib.rs`）：与 `plugin-stock`、
//! `schema-gen` 同一形态 —— worker 由宿主 spawn，没有 Cargo 消费方，不需要库入口。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, FrameRequest, FrameResponse, Peer,
    PluginDeclaration, error_codes, ops, serve,
};
use serde_json::json;

/// 本 worker 的插件身份：宿主 spawn 时经 `AXAGENT_PLUGIN_ID` 注入同名值，
/// 命令帧上的 `plugin_id` 必须与它逐字相等（第二道防线，第一道在宿主写帧前）。
const PLUGIN_ID: &str = "plugin.opc";

fn main() -> ExitCode {
    match serve(handle) {
        // 对端关管道 = 宿主停机，正常退出（`Peer::run` 里 EOF 不是错误）。
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[axagent-plugin-opc] 帧通道失败：{e}");
            ExitCode::FAILURE
        },
    }
}

fn handle(
    request: &FrameRequest,
    _peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
    match request.op.as_str() {
        ops::DESCRIBE => declaration(),
        ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
        // T2 命令面：宿主发 `CommandFrame`（pluginId 绑定 + 命令名 + 入参 + 预算）。
        ops::COMMAND => handle_command(request),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("OPC 插件未实现该操作：{other}"),
        ),
    }
}

/// 处理一条命令帧：身份自检 → （本批无命令可分发）显式报未知。
///
/// 本批**不注册任何业务命令**：命令表为空是事实，不是遗漏。因此这里不写
/// `match frame.command` —— 一个永远走 `other` 分支的空 match 只会让下一个人误以为
/// 「已经有分发表、只是漏了某条」。搬到第一条命令时再引入分发表。
fn handle_command(request: &FrameRequest) -> FrameResponse {
    let frame = match CommandFrame::from_frame_request(request) {
        Ok(frame) => frame,
        Err(e) => {
            return FrameResponse::error(
                error_codes::PLUGIN_COMMAND_INVALID_ARGS,
                format!("命令帧解码失败：{e}"),
            );
        },
    };
    let own_id = std::env::var("AXAGENT_PLUGIN_ID").unwrap_or_default();
    if frame.plugin_id != own_id {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧绑定 `{}` 与本 worker 身份 `{own_id}` 不符，拒绝执行", frame.plugin_id),
        )
        .into_frame_response();
    }
    // 注入身份与二进制自述身份也要一致：宿主把 OPC 的帧发给别的 worker 时，
    // 上面那条只在「宿主注入值 = 帧绑定值」时成立，这里再钉一次二进制归属。
    if own_id != PLUGIN_ID {
        return CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("worker 身份 `{own_id}` 与本二进制声明的 `{PLUGIN_ID}` 不符，拒绝执行"),
        )
        .into_frame_response();
    }
    CommandFault::for_frame(
        &frame,
        error_codes::PLUGIN_COMMAND_UNKNOWN,
        format!("OPC 插件本批未注册任何命令（收到 `{}`）", frame.command),
    )
    .into_frame_response()
}

/// 握手声明：如实描述「零能力」。
///
/// `calls` 是出站准入（宿主 `handle_call_seam` 会校验本插件是否声明过目标接缝）——
/// 下一批接入首条只读接缝 `opc.store` 时必须先补在这里，否则运行时被拒。
fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: Vec::new(),
            capabilities: Vec::new(),
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}
