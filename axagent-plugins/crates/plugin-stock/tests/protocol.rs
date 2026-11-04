// SPDX-License-Identifier: AGPL-3.0-only
//! 真进程 + 真管道的协议测试：宿主将来就是按这个形态 spawn 它的。
//!
//! 为什么不用进程内替代：`stdin`/`stdout` 的缓冲与 flush、以及「等响应期间对端又发来
//! 请求」的重入语义，只有在真管上才暴露。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFrame, FrameRequest, FrameResponse, PluginDeclaration,
    ResponseKind, error_codes, ops, read_inbound, write_json_frame,
};

struct Worker {
    child: Child,
    /// `Option` 只为一件事：`closing_stdin_is_a_clean_exit` 要真的关掉写端。
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

/// 与宿主 `PluginWorkerConfig::plugin_id` 对齐的身份（命令帧的身份自检拿它比对）。
const PLUGIN_ID: &str = "plugin.stock";

impl Worker {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-stock"))
            // 宿主 spawn 时会注入自身身份；命令帧的身份自检依赖它（缺省即空串 ⇒ 任何帧都拒）。
            .env("AXAGENT_PLUGIN_ID", PLUGIN_ID)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("worker 二进制应能启动");
        let stdin = BufWriter::new(child.stdin.take().expect("stdin 已被设为 piped"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout 已被设为 piped"));
        Self { child, stdin: Some(stdin), stdout }
    }

    fn send(&mut self, request: &FrameRequest) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), request).expect("写帧应成功");
    }

    fn send_response(&mut self, response: &FrameResponse) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), response).expect("写帧应成功");
    }

    /// 读一帧**请求**（worker → 宿主）。
    fn recv_request(&mut self) -> FrameRequest {
        match read_inbound(&mut self.stdout).expect("读帧应成功").as_request().cloned() {
            Some(request) => request,
            None => panic!("期望一帧请求（worker → 宿主），实际是响应"),
        }
    }

    /// 读一帧**响应**（宿主 → worker 的请求被回执）。
    fn recv_response(&mut self) -> FrameResponse {
        match read_inbound(&mut self.stdout).expect("读帧应成功").as_response().cloned() {
            Some(response) => response,
            None => panic!("期望一帧响应（宿主 → worker 的请求被回执），实际是请求"),
        }
    }

    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }
}

#[test]
fn describe_declares_the_tool_seam_and_its_outbound_call() {
    let mut worker = Worker::spawn();
    worker.send(
        &FrameRequest::new(
            ops::DESCRIBE,
            serde_json::json!({ "proto_version": AXAGENT_PLUGIN_PROTO_VERSION }),
        )
        .with_id("req-describe"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-describe"), "request_id 必须原样回填");

    let declaration: PluginDeclaration =
        serde_json::from_value(response.value.expect("成功帧必须带 value"))
            .expect("声明应是合法 JSON");
    assert_eq!(declaration.proto_version, AXAGENT_PLUGIN_PROTO_VERSION);
    declaration.validate().expect("握手声明必须通过宿主同一套校验");

    let caps: Vec<(&str, &str)> =
        declaration.capabilities.iter().map(|c| (c.seam.as_str(), c.op.as_str())).collect();
    assert!(caps.contains(&("tool.set", "exec")), "必须声明 tool.set 的执行面：{caps:?}");
    assert_eq!(
        declaration.calls,
        vec!["market.quote".to_string()],
        "出站接缝必须声明，否则宿主拒调"
    );

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    let status = worker.child.wait().expect("等待 worker 退出");
    assert!(status.success(), "收到 shutdown 并关管道后应以 0 退出，实际 {status:?}");
}

#[test]
fn tool_list_exposes_get_stock_quote() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new("list", serde_json::json!({})));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);

    let tools = response.value.expect("list 必须带 value");
    let tools = tools.as_array().expect("list 应返回数组");
    assert_eq!(tools.len(), 1, "当前只贡献一个工具：{tools:?}");
    assert_eq!(tools[0]["name"], "get_stock_quote");
    // 字段名是宿主 `RemoteToolDescriptor` 的 camelCase 形态；写错宿主会解析失败。
    assert!(tools[0]["inputSchema"]["required"][0] == "stock_code", "缺少 inputSchema.required");
}

#[test]
fn exec_calls_back_into_the_host_market_seam_and_returns_content() {
    let mut worker = Worker::spawn();
    worker.send(
        &FrameRequest::new(
            "exec",
            serde_json::json!({ "name": "get_stock_quote", "args": { "stock_code": "600519" } }),
        )
        .with_id("req-exec"),
    );

    // ① worker 必须先向宿主发 call_seam（不是自己去取数：凭证/provider 都在宿主侧）。
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "market.quote");
    assert_eq!(call.args["op"], "get_quote");
    assert_eq!(call.args["args"]["stock_code"], "600519");
    let call_id = call.request_id.clone().expect("出站请求必须带 request_id");

    // ② 宿主侧行情 provider 的回帧（固定值，证明插件把数据原样转成 content）。
    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "stockCode": "600519", "price": 1500.25 }))
            .with_id(call_id),
    );

    // ③ 工具执行结果回到宿主：`content` 是字符串，`request_id` 原样回填。
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-exec"));
    let value = response.value.expect("exec 成功必须带 value");
    let content = value["content"].as_str().expect("content 必须是字符串");
    let quote: serde_json::Value = serde_json::from_str(content).expect("content 应是 JSON 字符串");
    assert_eq!(quote["price"], 1500.25, "行情数据必须原样透出：{quote:?}");
    assert_eq!(value["metadata"]["stock_code"], "600519");

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

#[test]
fn market_seam_error_is_propagated_as_a_failed_tool_call() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new(
        "exec",
        serde_json::json!({ "name": "get_stock_quote", "args": { "stock_code": "000001" } }),
    ));
    let call = worker.recv_request();
    worker.send_response(
        &FrameResponse::error("MARKET_PROVIDER_UNAVAILABLE", "行情源不可用")
            .with_id(call.request_id.clone().expect("带 id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error, "提供方报错不得被包装成成功");
    assert_eq!(response.code.as_deref(), Some("MARKET_PROVIDER_UNAVAILABLE"));
}

#[test]
fn unknown_op_and_unknown_tool_are_explicit_errors() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new("stock.not_implemented_yet", serde_json::json!({})));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_UNKNOWN_OP));

    worker.send(&FrameRequest::new("exec", serde_json::json!({ "name": "nope", "args": {} })));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_CALL_FAILED));

    // 缺参数同样显式失败，不按默认值继续。
    worker.send(&FrameRequest::new(
        "exec",
        serde_json::json!({ "name": "get_stock_quote", "args": { "stock_code": "   " } }),
    ));
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::SEAM_CALL_INVALID_ARGS));
}

#[test]
fn closing_stdin_is_a_clean_exit() {
    let mut worker = Worker::spawn();
    // 不发 shutdown，直接关 stdin：宿主停机时靠关管道收尾，worker 不得把它当作错误。
    worker.close_stdin();
    let status = worker.child.wait().expect("等待 worker 退出");
    assert!(status.success(), "EOF 应是干净退出，实际 {status:?}");
}

#[test]
fn command_frame_runs_stock_get_quote_through_the_host_seam() {
    let mut worker = Worker::spawn();
    worker.send(
        &CommandFrame::new(
            PLUGIN_ID,
            "stock_get_quote",
            serde_json::json!({ "stock_code": "600519" }),
            5_000,
        )
        .to_frame_request()
        .expect("命令帧应能编成请求")
        .with_id("req-command"),
    );

    // 命令面与工具面共用同一条出站路径：worker 仍要回调宿主 market.quote 取数。
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "market.quote");
    assert_eq!(call.args["op"], "get_quote");
    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "code": "600519", "price": 1500.25 }))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );

    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-command"));
    let reply = response.value.expect("命令回执必须带 value");
    // `CommandReply` 没有 `rename_all` ⇒ 线格式是 snake_case，与 `CommandFrame` 同形。
    assert_eq!(reply["plugin_id"], PLUGIN_ID, "回执必须回填宿主绑定的身份");
    assert_eq!(reply["command"], "stock_get_quote");
    assert_eq!(reply["value"]["price"], 1500.25, "行情必须原样透出：{reply:?}");

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

#[test]
fn command_frame_identity_mismatch_and_unknown_command_are_explicit_errors() {
    let mut worker = Worker::spawn();

    // 身份不符：帧绑定别的插件 ⇒ 拒执行，且**不得**发起行情调用。
    worker.send(
        &CommandFrame::new("plugin.other", "stock_get_quote", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH));

    // 未声明的命令名：显式报未知，不静默成功。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "stock_not_implemented", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_UNKNOWN));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
}
