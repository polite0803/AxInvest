// SPDX-License-Identifier: AGPL-3.0-only
//! 股票域插件 worker（B4 骨架 + 首个真实能力）。
//!
//! 宿主以 `worker.program` 相对路径 spawn 本进程，双方在 stdin/stdout 上交换长度前缀
//! JSON 帧（`axagent-plugin-proto`）。诊断信息一律走 stderr —— stdout 是协议通道，
//! 混入任何非帧字节都会让宿主读帧失败。
//!
//! **首个能力（B4-c）**：`tool.set` 接缝上的只读工具 `get_stock_quote`。
//! 它是「宿主只读、不持 AppState、不碰凭证」那一档的最小实证：
//! - 工具本身不查库、不读文件、不取时钟；
//! - 行情一律经宿主接缝 `market.quote` 的 `get_quote` op 取（凭证与 provider 都在宿主侧）；
//! - 宿主侧 `RemoteToolSet::load` 会在注册时先调 `list` 取元数据，`exec` 按名执行。
//!
//! 为什么用 `plugin-proto` 的 [`serve`] 而不是自写读循环：插件一旦能主动调宿主，
//! 同一条管道上就同时挂着「我方请求的响应」与「对端发来的新请求」，朴素读一帧写一帧
//! 必然错配。`Peer` 把对端请求进 `deferred` 队列，等响应的同时不丢帧、不死锁。
//!
//! 为什么是纯可执行包（只有 `src/main.rs`、没有 `src/lib.rs`）：分层门禁规则 4 要求每个
//! 库 crate 至少被一份 `Cargo.toml` 引用，而 worker 由宿主 spawn、没有 Cargo 消费方；
//! 门禁对「纯可执行包」已有豁免先例（`crates/schema-gen`），本 crate 照同一形态。

use std::io::{StdinLock, StdoutLock};
use std::process::ExitCode;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CapabilityDecl, CommandFault, CommandFrame, CommandOutcome,
    CommandReply, FrameRequest, FrameResponse, Peer, PluginDeclaration, ResponseKind, error_codes,
    ops, serve,
};
use serde_json::{Value, json};

/// 本插件提供的工具名（与宿主登记的工具名逐字一致）。
const TOOL_NAME: &str = "get_stock_quote";
/// 本插件承接的命名命令（manifest `commands[].name` 必须与它逐字一致）。
const COMMAND_GET_QUOTE: &str = "stock_get_quote";
/// `tool.set` 接缝的协议版本；随工具集形状变化时递增。
const TOOL_SET_VERSION: &str = "0.1.0";
/// 工具取数走的宿主接缝：宿主把已有行情能力以入站分派的方式暴露给插件。
const MARKET_SEAM: &str = "market.quote";

fn main() -> ExitCode {
    match serve(handle) {
        // 对端关管道 = 宿主停机，正常退出（见 `Peer::run`：EOF 不是错误）。
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[axagent-plugin-stock] 帧通道失败：{e}");
            ExitCode::FAILURE
        },
    }
}

fn handle(
    request: &FrameRequest,
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
    match request.op.as_str() {
        ops::DESCRIBE => declaration(),
        ops::SHUTDOWN => FrameResponse::success(json!({ "ok": true })),
        // `tool.set` 的发现面：宿主要在注册时一次性取回工具元数据（`Tool::name()` 是同步借用）。
        "list" => FrameResponse::success(tools()),
        // `tool.set` 的执行面：宿主发 `{name, args}`，执行结果放 `content`（字符串）。
        "exec" => exec_tool(&request.args, &request.call_chain, peer),
        // T2 命令面：宿主发 `CommandFrame`（`pluginId` 绑定 + 命令名 + 入参 + 预算）。
        ops::COMMAND => handle_command(request, peer),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("股票插件未实现该操作：{other}"),
        ),
    }
}

/// 处理一条命令帧：身份自检 → 按命令名分发。
///
/// 身份自检是**第二道防线**（第一道在宿主 `WorkerInvoker::execute_command` 写帧前）：
/// 帧上的 `plugin_id` 必须等于 spawn 时注入的 `AXAGENT_PLUGIN_ID`，防宿主侧错配。
fn handle_command(
    request: &FrameRequest,
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
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

    let outcome = match frame.command.as_str() {
        COMMAND_GET_QUOTE => match fetch_quote(&frame.input, &request.call_chain, peer) {
            Ok((_stock_code, quote)) => {
                CommandOutcome::Reply(CommandReply::for_frame(&frame, quote))
            },
            Err((code, message)) => {
                CommandOutcome::Fault(CommandFault::for_frame(&frame, code, message))
            },
        },
        other => CommandOutcome::Fault(CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_UNKNOWN,
            format!("股票插件没有命令 `{other}` 的实现"),
        )),
    };
    outcome
        .into_frame_response()
        .unwrap_or_else(|e| FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()))
}

/// 握手声明：只声明**真正实现**的东西 —— 空能力会让宿主注册一条调不通的接缝。
///
/// `calls` 是出站准入（宿主 `handle_call_seam` 会校验本插件是否声明过目标接缝），
/// 少写一条 = 运行时被拒。
fn declaration() -> FrameResponse {
    FrameResponse::success(
        serde_json::to_value(PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: vec![MARKET_SEAM.to_string()],
            capabilities: vec![CapabilityDecl {
                seam: "tool.set".to_string(),
                op: "exec".to_string(),
                version: TOOL_SET_VERSION.to_string(),
            }],
        })
        .expect("插件声明是可序列化的纯数据"),
    )
}

/// 工具清单（`tool.set` 的 `list` 回帧）。字段名是宿主 `RemoteToolDescriptor` 的 camelCase 形态。
fn tools() -> Value {
    json!([{
        "name": TOOL_NAME,
        "description": "获取 A 股个股实时行情（价格/涨跌/成交量等），行情由宿主行情接缝提供",
        "inputSchema": {
            "type": "object",
            "properties": {
                "stock_code": { "type": "string", "description": "6 位股票代码，如 600519" }
            },
            "required": ["stock_code"]
        },
        "category": "integration"
    }])
}

/// 执行一个命名工具。未知工具显式报错（不静默成功）；参数缺失按 `SEAM_CALL_INVALID_ARGS` 报。
fn exec_tool(
    args: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> FrameResponse {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    if name != TOOL_NAME {
        return FrameResponse::error(
            error_codes::PLUGIN_CALL_FAILED,
            format!("股票插件的工具集中没有名为 `{name}` 的工具"),
        );
    }

    let input = args.get("args").cloned().unwrap_or(Value::Null);
    match fetch_quote(&input, call_chain, peer) {
        Ok((stock_code, quote)) => {
            // 工具出参按宿主 `ToolExecReply` 的约定：`content` 是字符串。
            FrameResponse::success(json!({
                "content": serde_json::to_string(&quote).unwrap_or_else(|_| quote.to_string()),
                "metadata": { "stock_code": stock_code }
            }))
        },
        Err((code, message)) => FrameResponse::error(code, message),
    }
}

/// 取一条个股行情，返回 `(入参里的股票代码, 行情 JSON)`。
///
/// 工具执行面与命令面共用的唯一实现（不写第二遍取数逻辑）。出站走宿主接缝：
/// 凭证 / provider / 限流都在宿主侧，插件只传代码；`call_chain` 必须原样带上入站链，
/// 否则宿主侧的环检测收不到这条边。
fn fetch_quote(
    input: &Value,
    call_chain: &[String],
    peer: &mut Peer<StdinLock<'static>, StdoutLock<'static>>,
) -> Result<(String, Value), (String, String)> {
    let Some(stock_code) =
        input.get("stock_code").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty())
    else {
        return Err((
            error_codes::SEAM_CALL_INVALID_ARGS.to_string(),
            "`stock_code` 缺失或为空白".to_string(),
        ));
    };

    let reply = peer
        .call_seam(
            MARKET_SEAM,
            "get_quote",
            json!({ "stock_code": stock_code }),
            call_chain.to_vec(),
        )
        .map_err(|e| {
            (error_codes::PLUGIN_CALL_FAILED.to_string(), format!("行情接缝调用失败：{e}"))
        })?;

    match reply.kind {
        ResponseKind::Success => Ok((stock_code.to_string(), reply.value.unwrap_or(Value::Null))),
        ResponseKind::Error => Err((
            reply.code.unwrap_or_else(|| error_codes::PLUGIN_CALL_FAILED.to_string()),
            reply.message.unwrap_or_else(|| "行情接缝返回了错误但没有说明".to_string()),
        )),
    }
}
