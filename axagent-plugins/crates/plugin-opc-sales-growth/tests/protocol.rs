// SPDX-License-Identifier: AGPL-3.0-only
//! sales_growth 域 worker 的真进程帧测试。
//!
//! 使用 SDK `plugin-testkit` 构造 describe / command / shutdown 帧，锁住身份白名单、
//! 三条命令的出站 seam / op 与回执形状。协议常量不在这里手抄。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    FrameRequest, FrameResponse, ResponseKind, ops, read_inbound, write_json_frame,
};
use axagent_plugin_testkit::{command_frame, declaration_of, describe_frame, shutdown_frame};

const PLUGIN_ID: &str = "opc-sales-growth@bundled";

struct Worker {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn_as(plugin_id: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-opc-sales-growth"))
            .env("AXAGENT_PLUGIN_ID", plugin_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sales_growth worker 应能启动");
        let stdin = BufWriter::new(child.stdin.take().expect("stdin 已设为 piped"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout 已设为 piped"));
        Self { child, stdin: Some(stdin), stdout }
    }

    fn send(&mut self, request: &FrameRequest) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), request).expect("写帧应成功");
    }

    fn send_response(&mut self, response: &FrameResponse) {
        write_json_frame(self.stdin.as_mut().expect("写端仍打开"), response).expect("写帧应成功");
    }

    fn recv_request(&mut self) -> FrameRequest {
        read_inbound(&mut self.stdout)
            .expect("读帧应成功")
            .as_request()
            .cloned()
            .expect("期望 worker → 宿主的出站请求")
    }

    fn recv_response(&mut self) -> FrameResponse {
        read_inbound(&mut self.stdout)
            .expect("读帧应成功")
            .as_response()
            .cloned()
            .expect("期望 worker 回执")
    }

    fn shutdown(&mut self) {
        self.send(&shutdown_frame());
        assert_eq!(self.recv_response().kind, ResponseKind::Success);
        drop(self.stdin.take());
        assert!(self.child.wait().expect("等待 worker 退出").success());
    }
}

/// 发一条命令 → 取出站帧 → 校验 seam / op → 回固定 payload → 校验逐字透传。
fn roundtrip(
    worker: &mut Worker,
    command: &str,
    input: serde_json::Value,
    expected_op: &str,
    reply: serde_json::Value,
) {
    worker.send(&command_frame(PLUGIN_ID, command, input, 5_000));
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM, "{command} 必须经 call_seam 出站");
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], expected_op, "{command} 的 op 名必须逐字");
    worker.send_response(
        &FrameResponse::success(reply.clone())
            .with_id(call.request_id.expect("出站请求必须带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success, "{command} 应回成功");
    let value = response.value.expect("命令回执必须带 value");
    assert_eq!(value["value"], reply, "{command} 出参必须逐字透传");
}

#[test]
fn describe_and_identity_are_scoped_to_sales_growth() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(&describe_frame().with_id("req-describe"));
    let response = worker.recv_response();
    let declaration = declaration_of(&response);
    assert_eq!(declaration.calls, vec!["opc.store".to_string()]);

    let mut external = Worker::spawn_as("opc-sales-growth@external");
    external.send(&command_frame(
        "opc-sales-growth@external",
        "opc_not_implemented",
        serde_json::json!({}),
        5_000,
    ));
    assert_eq!(
        external.recv_response().code.as_deref(),
        Some(axagent_plugin_proto::error_codes::PLUGIN_COMMAND_UNKNOWN)
    );
    external.shutdown();
    worker.shutdown();
}

#[test]
fn sales_growth_commands_go_through_host_seam() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);

    roundtrip(
        &mut worker,
        "opc_create_customer",
        serde_json::json!({ "name": "张三", "email": "z@example.com" }),
        "create_customer",
        serde_json::json!({ "id": "cust-new", "name": "张三" }),
    );
    roundtrip(
        &mut worker,
        "opc_list_landing_pages",
        serde_json::json!({}),
        "list_landing_pages",
        serde_json::json!([]),
    );
    roundtrip(
        &mut worker,
        "opc_list_contacts",
        serde_json::json!({}),
        "list_contacts",
        serde_json::json!([]),
    );

    worker.shutdown();
}
