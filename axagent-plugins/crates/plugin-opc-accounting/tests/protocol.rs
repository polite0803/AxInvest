// SPDX-License-Identifier: AGPL-3.0-only
//! accounting 域 worker 的真进程帧测试。
//!
//! 三条命令的 owner 已从公共 `plugin-opc` 迁出：这里钉住身份白名单、出站 seam/op、
//! 键名改写与形状闸，防止迁移只剩清单改名而实现仍留在公共 worker。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFrame, FrameRequest, FrameResponse, PluginDeclaration,
    ResponseKind, error_codes, ops, read_inbound, write_json_frame,
};

const PLUGIN_ID: &str = "opc-accounting@bundled";

struct Worker {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn_as(plugin_id: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-opc-accounting"))
            .env("AXAGENT_PLUGIN_ID", plugin_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("accounting worker 应能启动");
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
        self.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
        assert_eq!(self.recv_response().kind, ResponseKind::Success);
        drop(self.stdin.take());
        assert!(self.child.wait().expect("等待 worker 退出").success());
    }
}

fn command(plugin_id: &str, name: &str, input: serde_json::Value) -> FrameRequest {
    CommandFrame::new(plugin_id, name, input, 5_000).to_frame_request().expect("命令帧应能编成请求")
}

#[test]
fn describe_and_identity_are_scoped_to_accounting() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);
    worker.send(
        &FrameRequest::new(
            ops::DESCRIBE,
            serde_json::json!({ "proto_version": AXAGENT_PLUGIN_PROTO_VERSION }),
        )
        .with_id("req-describe"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    let declaration: PluginDeclaration =
        serde_json::from_value(response.value.expect("成功帧带值")).expect("合法声明");
    declaration.validate().expect("声明应通过宿主协议校验");
    assert_eq!(declaration.calls, vec!["opc.store".to_string()]);

    // 外部安装形态也必须是本 worker 的合法身份。
    let mut external = Worker::spawn_as("opc-accounting@external");
    external.send(&command(
        "opc-accounting@external",
        "opc_not_implemented",
        serde_json::json!({}),
    ));
    let response = external.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_UNKNOWN));
    external.shutdown();

    // 不相关的逻辑名（公共 OPC worker）必须在本 worker 被拒绝，不能串包执行。
    let mut other = Worker::spawn_as("opc@bundled");
    other.send(&command("opc@bundled", "opc_list_invoices", serde_json::json!({})));
    let response = other.recv_response();
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH));
    other.shutdown();
    worker.shutdown();
}

#[test]
fn accounting_commands_go_through_host_seam() {
    let mut worker = Worker::spawn_as(PLUGIN_ID);

    // ① 发票列表：过滤键原样嵌 request，空表是正常成功。
    worker.send(&command(
        PLUGIN_ID,
        "opc_list_invoices",
        serde_json::json!({ "status": "paid", "customerId": "cust-1", "limit": 20 }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.op, ops::CALL_SEAM);
    assert_eq!(call.args["seam"], "opc.store");
    assert_eq!(call.args["op"], "list_invoices");
    assert_eq!(call.args["args"]["request"]["status"], "paid");
    assert_eq!(call.args["args"]["request"]["customerId"], "cust-1");
    assert_eq!(call.args["args"]["request"]["limit"], 20);
    worker.send_response(
        &FrameResponse::success(serde_json::json!([]))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.value.expect("回执")["value"], serde_json::json!([]));

    // ② 创建发票：camelCase 入参 + 嵌 request；行金额由宿主现算。
    worker.send(&command(
        PLUGIN_ID,
        "opc_create_invoice",
        serde_json::json!({
            "customerId": "cust-1",
            "lineItems": [{ "description": "咨询", "quantity": 2, "unitPrice": 100.0 }],
            "currency": "CNY",
        }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "create_invoice");
    let request = call.args["args"]["request"].as_object().expect("request 应是对象");
    let mut keys: Vec<String> = request.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["currency", "customerId", "lineItems"].map(str::to_string).to_vec());
    let invoice = serde_json::json!({ "id": "inv-1", "total": 200.0 });
    worker.send_response(
        &FrameResponse::success(invoice.clone())
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.value.expect("回执")["value"], invoice);

    // ③ 状态变更：命令面 status 在出站帧改名为 target，缺行态逐字透传。
    worker.send(&command(
        PLUGIN_ID,
        "opc_transition_invoice",
        serde_json::json!({ "id": "inv-1", "status": "paid" }),
    ));
    let call = worker.recv_request();
    assert_eq!(call.args["op"], "transition_invoice");
    let args = call.args["args"].as_object().expect("出站 args 是对象");
    let mut keys: Vec<String> = args.keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["id", "target"].map(str::to_string).to_vec());
    assert_eq!(args["target"], "paid");
    assert!(args.get("status").is_none());
    worker.send_response(
        &FrameResponse::success(serde_json::json!("not_found"))
            .with_id(call.request_id.expect("出站请求带 request_id")),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.value.expect("回执")["value"], "not_found");

    // ④ 形状闸：缺键 / 类型不符在命令层拒，不发接缝帧。
    for input in [
        serde_json::json!({ "lineItems": [] }),
        serde_json::json!({ "customerId": "c", "lineItems": "x" }),
    ] {
        worker.send(&command(PLUGIN_ID, "opc_create_invoice", input));
        let response = worker.recv_response();
        assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_COMMAND_INVALID_ARGS));
    }

    worker.shutdown();
}
