// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #49（T2 派发口）端到端夹具 worker：长驻 + 承接 `ops::COMMAND` 命令帧。
//!
//! 示证对象是一条**无状态**命令 `echo_input`：回显 `input.text` 与其字符数。
//! 它只证明「命名命令能以命令帧跑通全链路」，**不含任何业务逻辑** —— 真实股票命令的
//! 端到端在 `e2e_stock_tool_test.rs` / `e2e_stock_bundle_install_test.rs`（跑
//! `plugin-stock` 本体），本夹具不复制那份词表（此前那版子集副本已删，2026-11-04）。
//!
//! 回执行：处理每条命令帧前在安装目录（= spawn 时的 cwd）追加一行
//! `ran command=<命令名> plugin_id=<帧上绑定的插件ID>` —— 集成测试据此**客观证明**
//! 「形状校验未通过时确实没派发」与「宿主绑定的 pluginId 原样到达 worker」。

use std::fs::OpenOptions;
use std::io::Write;
use std::time::Duration;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFault, CommandFrame, CommandOutcome, CommandReply,
    FrameRequest, FrameResponse, PluginDeclaration, error_codes, ops, serve,
};
use serde_json::{Value, json};

/// 声明：本夹具不提供任何接缝能力，只承接命令帧（capabilities / subscribe / calls 全空）。
fn declaration() -> PluginDeclaration {
    PluginDeclaration {
        proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
        subscribe: Vec::new(),
        calls: Vec::new(),
        capabilities: Vec::new(),
    }
}

/// 在安装目录（spawn 时的 cwd）追加一行执行标记。
fn note_ran(command: &str, plugin_id: &str) {
    let marker = std::path::Path::new("command_frame.marker");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(marker) {
        let _ = writeln!(file, "ran command={command} plugin_id={plugin_id}");
    }
}

/// 处理一条命令帧：身份自检 → 落标记 → 按命令名分发。
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
    // 身份自检（第二道防线）：宿主绑定的 pluginId 必须等于 spawn 注入的自身身份。
    // 第一道在宿主 `WorkerInvoker::execute_command`（写帧前）；这里防的是宿主侧 bug。
    let own_id = std::env::var("AXAGENT_PLUGIN_ID").unwrap_or_default();
    if frame.plugin_id != own_id {
        let fault = CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_ID_MISMATCH,
            format!("帧绑定 `{}` 与本 worker 身份 `{own_id}` 不符，拒绝执行", frame.plugin_id),
        );
        return fault.into_frame_response();
    }
    note_ran(&frame.command, &frame.plugin_id);

    let outcome = match frame.command.as_str() {
        "echo_input" => {
            let Some(Value::String(text)) = frame.input.get("text") else {
                return CommandFault::for_frame(
                    &frame,
                    error_codes::PLUGIN_COMMAND_INVALID_ARGS,
                    "input.text 必须是字符串",
                )
                .into_frame_response();
            };
            let reply = json!({ "text": text, "length": text.chars().count() });
            CommandOutcome::Reply(CommandReply::for_frame(&frame, reply))
        },
        // 「装死」演示：睡过预算再回帧 —— 宿主必须先到点放弃（超时被拦的用例载体）。
        "sleep_too_long" => {
            let sleep_ms = frame.input.get("sleepMs").and_then(Value::as_u64).unwrap_or(0);
            std::thread::sleep(Duration::from_millis(sleep_ms));
            CommandOutcome::Reply(CommandReply::for_frame(&frame, json!({ "sleptMs": sleep_ms })))
        },
        // 「自报失败」演示：失败回执带协议码，宿主报错必须把它透传上来。
        "fail_self" => CommandOutcome::Fault(CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_FAILED,
            "worker 侧故意失败（错误码透传演示）",
        )),
        other => CommandOutcome::Fault(CommandFault::for_frame(
            &frame,
            error_codes::PLUGIN_COMMAND_UNKNOWN,
            format!("夹具没有命令 `{other}` 的实现"),
        )),
    };
    match outcome.into_frame_response() {
        Ok(response) => response,
        Err(e) => FrameResponse::error(error_codes::PLUGIN_COMMAND_FAILED, e.to_string()),
    }
}

fn main() {
    let outcome = serve(|request: &FrameRequest, _peer| match request.op.as_str() {
        op if op == ops::DESCRIBE => match serde_json::to_value(declaration()) {
            Ok(value) => FrameResponse::success(value),
            Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
        },
        op if op == ops::COMMAND => handle_command(request),
        op if op == ops::SHUTDOWN => FrameResponse::success(json!({ "stopping": true })),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("夹具 worker 未实现 op `{other}`"),
        ),
    });
    if let Err(e) = outcome {
        eprintln!("e2e 命令帧夹具 worker I/O 异常退出：{e}");
        std::process::exit(1);
    }
}
