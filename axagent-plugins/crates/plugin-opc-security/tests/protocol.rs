// SPDX-License-Identifier: AGPL-3.0-only
//! security worker 的真进程 + 真管道协议测试。
//!
//! 锁三件事：① 声明的两条出站接缝（少一条宿主即拒）；② 每条域命令转发到正确的
//! seam / op，且入参形状不漂移；③ 自动化规则命令固定注入 `security` 域 id，调用方
//! 不能越域。宿主工具逻辑 / 数据主权都不在 worker 内，故这里只验帧面。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    CommandFrame, FrameRequest, FrameResponse, ResponseKind, error_codes, ops, read_inbound,
    write_json_frame,
};
use axagent_plugin_testkit::{command_frame, shutdown_frame};

const PLUGIN_ID: &str = "opc-security@bundled";

struct Worker {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn() -> Self {
        Self::spawn_as(PLUGIN_ID)
    }

    fn spawn_as(injected_id: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-opc-security"))
            .env("AXAGENT_PLUGIN_ID", injected_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("worker 二进制应能启动");
        let stdin = BufWriter::new(child.stdin.take().expect("stdin 已被设为 piped"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout 已被设为 piped"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
        }
    }

    fn send(&mut self, request: &FrameRequest) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), request).expect("写帧应成功");
    }

    fn send_response(&mut self, response: &FrameResponse) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), response).expect("写帧应成功");
    }

    fn recv_request(&mut self) -> FrameRequest {
        match read_inbound(&mut self.stdout)
            .expect("读帧应成功")
            .as_request()
            .cloned()
        {
            Some(request) => request,
            None => panic!("期望一帧请求（worker → 宿主），实际是响应"),
        }
    }

    fn recv_response(&mut self) -> FrameResponse {
        match read_inbound(&mut self.stdout)
            .expect("读帧应成功")
            .as_response()
            .cloned()
        {
            Some(response) => response,
            None => panic!("期望一帧响应，实际是请求"),
        }
    }

    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    fn expect_clean_exit(&mut self) {
        self.send(&shutdown_frame());
        assert_eq!(self.recv_response().kind, ResponseKind::Success);
        self.close_stdin();
        assert!(self.child.wait().expect("等待 worker 退出").success());
    }
}

fn command(name: &str, input: serde_json::Value) -> FrameRequest {
    CommandFrame::new(PLUGIN_ID, name, input, 5_000)
        .to_frame_request()
        .expect("命令帧应能编成请求")
}

#[test]
fn describe_declares_both_outbound_seams() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new(ops::DESCRIBE, serde_json::Value::Null).with_id("req-desc"));
    let response = worker.recv_response();
    assert_eq!(
        response.kind,
        ResponseKind::Success,
        "describe 应成功：{response:?}"
    );
    let declaration = response.value.expect("describe 必须带声明");
    let calls = declaration["calls"].as_array().expect("calls 应是数组");
    for seam in ["opc.store", "tool.set"] {
        assert!(
            calls.iter().any(|value| value.as_str() == Some(seam)),
            "声明缺少出站接缝 {seam}"
        );
    }
    worker.expect_clean_exit();
}

#[test]
fn tool_commands_forward_to_tool_set_exec_with_verbatim_args() {
    let cases = [
        (
            "opc_security_opc_search_wiki",
            "OpcSearchWiki",
            serde_json::json!({}),
        ),
        (
            "opc_security_web_search",
            "WebSearch",
            serde_json::json!({
              "query": "搜索关键词"
            }),
        ),
        (
            "opc_security_file_write",
            "FileWrite",
            serde_json::json!({
              "path": "note.md",
              "content": "正文"
            }),
        ),
    ];
    for (index, (command_name, tool_name, input)) in cases.into_iter().enumerate() {
        let mut worker = Worker::spawn();
        worker.send(&command(command_name, input.clone()).with_id(format!("req-{index}")));

        let call = worker.recv_request();
        assert_eq!(call.op, ops::CALL_SEAM);
        assert_eq!(call.args["seam"], "tool.set");
        assert_eq!(call.args["op"], "exec");
        assert_eq!(call.args["args"]["name"], tool_name, "工具名必须逐字转发");
        assert_eq!(call.args["args"]["args"], input, "工具入参必须原样透传");

        worker.send_response(
            &FrameResponse::success(serde_json::json!({ "ok": true, "tool": tool_name }))
                .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
        );
        let response = worker.recv_response();
        assert_eq!(
            response.kind,
            ResponseKind::Success,
            "{command_name} 应成功：{response:?}"
        );
        let reply = response.value.expect("命令回执必须带 value");
        assert_eq!(reply["plugin_id"], PLUGIN_ID);
        assert_eq!(reply["value"]["tool"], tool_name);
        worker.expect_clean_exit();
    }
}

#[test]
fn automation_rules_inject_fixed_domain_id() {
    let mut worker = Worker::spawn();
    // 调用方故意塞 `capabilityPackId` 想越域；worker 必须用自身域 id 覆盖。
    worker.send(
        &command(
            "opc_security_run_automation_rules",
            serde_json::json!({
                "capabilityPackId": "security",
                "entityType": "security",
                "entityId": "demo-1",
                "overdueDays": 3,
                "fields": { "stage": "示例" },
            }),
        )
        .with_id("req-rules"),
    );

    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], "run_automation_rules");
    assert_eq!(
        call.args["args"]["capability_pack_id"], "security",
        "不得让调用方越域"
    );
    assert_eq!(call.args["args"]["entity_type"], "security");
    assert_eq!(call.args["args"]["entity_id"], "demo-1");
    assert_eq!(
        call.args["args"]["overdue_days"], 3,
        "camelCase 必须转成接缝 snake_case"
    );
    assert_eq!(call.args["args"]["fields"]["stage"], "示例");
    assert!(
        call.args["args"].get("capabilityPackId").is_none(),
        "接缝不得混入命令面键名"
    );

    worker.send_response(
        &FrameResponse::success(serde_json::json!({ "matchedRules": 2 }))
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(
        response.value.expect("回执须带 value")["value"]["matchedRules"],
        2
    );
    worker.expect_clean_exit();
}

#[test]
fn malformed_inputs_and_wrong_identity_are_rejected() {
    let mut worker = Worker::spawn();
    worker.send(
        &command("opc_security_opc_search_wiki", serde_json::json!([1, 2])).with_id("req-shape"),
    );
    let response = worker.recv_response();
    assert_eq!(
        response.kind,
        ResponseKind::Error,
        "非对象入参必须在命令层被拒"
    );
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS)
    );
    worker.expect_clean_exit();

    let mut worker = Worker::spawn();
    worker.send(
        &command("opc-security_no_such_command", serde_json::json!({})).with_id("req-unknown"),
    );
    let response = worker.recv_response();
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_UNKNOWN)
    );
    worker.expect_clean_exit();

    // 错误身份必须是任何域都不使用的 id：早期模板写死 opc-security，
    // security 自身翻牌后该用例就不再是「错的身份」而会假绿。
    let mut worker = Worker::spawn_as("unrelated-plugin@bundled");
    worker.send(&command("opc_security_opc_search_wiki", serde_json::json!({})).with_id("req-id"));
    let response = worker.recv_response();
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH)
    );
    worker.expect_clean_exit();
}

#[test]
fn seam_error_is_propagated_without_fabricating_success() {
    let mut worker = Worker::spawn();
    worker.send(
        &command("opc_security_opc_search_wiki", serde_json::json!({})).with_id("req-seam-error"),
    );
    let call = worker.recv_request();
    worker.send_response(
        &FrameResponse::error("TEST_TOOL_SEAM_UNAVAILABLE", "宿主未注册工具集")
            .with_id(call.request_id.clone().expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some("TEST_TOOL_SEAM_UNAVAILABLE"));
    assert_eq!(response.request_id.as_deref(), Some("req-seam-error"));
    worker.expect_clean_exit();
}

// 显式使用 testkit 的 `command_frame` 别名，保证测试与宿主 spawn 使用的帧构造器同源。
#[test]
fn testkit_command_frame_is_compatible_with_worker() {
    let mut worker = Worker::spawn();
    let request = command_frame(
        PLUGIN_ID,
        "opc_security_opc_search_wiki",
        serde_json::json!({}),
        5_000,
    )
    .with_id("req-testkit");
    worker.send(&request);
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "exec");
    worker.send_response(
        &FrameResponse::success(serde_json::Value::Null)
            .with_id(call.request_id.expect("出站请求必须带 request_id")),
    );
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.expect_clean_exit();
}
