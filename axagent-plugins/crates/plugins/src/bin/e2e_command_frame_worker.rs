// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #49（T2 派发口）端到端夹具 worker：长驻 + 承接 `ops::COMMAND` 命令帧。
//!
//! 示证对象是一条**无状态**股票命名命令 `parse_analysis_intent`：真实实现在
//! `axagent-quant/crates/analysis-engine/src/intent_parser.rs`，判据是它的命令入口
//! （`stock_analysis.rs:7713`）签名不带 `State<'_, AppState>`，函数体只做
//! 「字符串 → 结构化意图」的纯计算 —— 不查数据库、不读文件、不取时钟、不碰任何
//! 宿主共享状态，所有输入由调用方传入。这正是 P6.1 迁出口要的形态。
//!
//! ⚠ 本夹具只移植了词表**子集**（6 位代码、贵州茅台/茅台/宁德时代、三档周期、
//! 两类动作），目的是证明「可迁移的股票命令能以命令帧跑通全链路」，
//! **不是**业务逻辑的最终落位；真迁移时 worker 直接依赖 `intent_parser` 本体，
//! 不维持这份副本（迁移模板见任务 #49 交接报告）。
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

/// 热门词表子集（与 `intent_parser.rs` 的 `HOT_STOCK_MAP` 同名同码）。
const HOT_STOCKS: &[(&str, &str)] =
    &[("贵州茅台", "600519"), ("茅台", "600519"), ("宁德时代", "300750")];

/// 周期词表子集（原表更长，见 `HORIZON_KEYWORDS`）。
const HORIZONS: &[(&str, &str)] = &[("短线", "short"), ("中线", "mid"), ("长线", "long")];

/// 与 `extract_stock_code` 同判据：纯 6 位数字或分词中的 6 位数字。
fn extract_code(input: &str) -> Option<String> {
    if input.len() == 6 && input.chars().all(|c| c.is_ascii_digit()) {
        return Some(input.to_string());
    }
    input
        .split_whitespace()
        .find(|word| word.len() == 6 && word.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

/// 与 `extract_horizon` 同判据（子集词表）。
fn extract_horizon(input: &str) -> Option<&'static str> {
    HORIZONS.iter().find(|(kw, _)| input.contains(kw)).map(|(_, h)| *h)
}

/// 与 `extract_action` 同判据：先「查看类」后「分析类」，兜底 `view`。
fn extract_action(input: &str) -> &'static str {
    if ["看看", "查看", "查", "关注"].iter().any(|kw| input.contains(kw)) {
        return "view";
    }
    if ["调研", "分析", "研究", "评估"].iter().any(|kw| input.contains(kw)) {
        return "research";
    }
    "view"
}

/// 移植版 `parse_analysis_intent`：输出与 `ParsedIntent` 的 serde 形态同构（camelCase）。
fn parse_intent(input: &str) -> Value {
    let trimmed = input.trim();
    let horizon = extract_horizon(trimmed);
    let action = extract_action(trimmed);
    let horizon_json = horizon.map(Value::from).unwrap_or(Value::Null);
    if let Some(code) = extract_code(trimmed) {
        return json!({
            "rawInput": trimmed,
            "stockQuery": code.clone(),
            "stockCode": code.clone(),
            "timeHorizon": horizon_json,
            "actionType": action,
            "success": true,
            "description": format!("代码识别: {code}"),
        });
    }
    if let Some((name, code)) = HOT_STOCKS.iter().find(|(name, _)| trimmed.contains(name)) {
        return json!({
            "rawInput": trimmed,
            "stockQuery": name,
            "stockCode": code,
            "timeHorizon": horizon_json,
            "actionType": action,
            "success": true,
            "description": format!("热门词命中: {name} ({code})"),
        });
    }
    json!({
        "rawInput": trimmed,
        "stockQuery": trimmed,
        "stockCode": Value::Null,
        "timeHorizon": Value::Null,
        "actionType": action,
        "success": false,
        "description": "夹具词表未覆盖，无法解析",
    })
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
        "parse_analysis_intent" => {
            let Some(Value::String(text)) = frame.input.get("input") else {
                return CommandFault::for_frame(
                    &frame,
                    error_codes::PLUGIN_COMMAND_INVALID_ARGS,
                    "input.input 必须是字符串",
                )
                .into_frame_response();
            };
            CommandOutcome::Reply(CommandReply::for_frame(&frame, parse_intent(text.as_str())))
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
