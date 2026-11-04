// SPDX-License-Identifier: AGPL-3.0-only
//! 真进程 + 真管道的协议测试：宿主将来就是按这个形态 spawn 它的。
//!
//! 为什么不用 `serve()` 的进程内替代：`stdin`/`stdout` 的缓冲与 flush 语义只有在真管上
//! 才暴露（写端不 flush 会让对端等满预算）；进程内调用测不出这一层。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, FrameRequest, FrameResponse, PluginDeclaration, ResponseKind,
    error_codes, ops, read_json_frame, write_json_frame,
};

struct Worker {
    child: Child,
    /// `Option` 只为一件事：`closing_stdin_is_a_clean_exit` 要真的关掉写端。
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-stock"))
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

    fn recv(&mut self) -> FrameResponse {
        read_json_frame(&mut self.stdout).expect("读帧应成功")
    }

    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }
}

#[test]
fn describe_returns_a_valid_declaration_over_a_real_pipe() {
    let mut worker = Worker::spawn();
    worker.send(
        &FrameRequest::new(
            ops::DESCRIBE,
            serde_json::json!({ "proto_version": AXAGENT_PLUGIN_PROTO_VERSION }),
        )
        .with_id("req-describe"),
    );
    let response = worker.recv();
    assert_eq!(response.kind, ResponseKind::Success);
    assert_eq!(response.request_id.as_deref(), Some("req-describe"), "request_id 必须原样回填");

    let declaration: PluginDeclaration =
        serde_json::from_value(response.value.expect("成功帧必须带 value"))
            .expect("声明应是合法 JSON");
    assert_eq!(declaration.proto_version, AXAGENT_PLUGIN_PROTO_VERSION);
    declaration.validate().expect("握手声明必须通过宿主同一套校验");

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv().kind, ResponseKind::Success);
    let status = worker.child.wait().expect("等待 worker 退出");
    assert!(status.success(), "收到 shutdown 后应以 0 退出，实际 {status:?}");
}

#[test]
fn unknown_op_is_an_explicit_error_not_a_silent_success() {
    let mut worker = Worker::spawn();
    worker.send(&FrameRequest::new("stock.not_implemented_yet", serde_json::json!({})));
    let response = worker.recv();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.code.as_deref(), Some(error_codes::PLUGIN_UNKNOWN_OP));

    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv().kind, ResponseKind::Success);
    assert!(worker.child.wait().expect("等待 worker 退出").success());
}

#[test]
fn closing_stdin_is_a_clean_exit() {
    let mut worker = Worker::spawn();
    // 不发 shutdown，直接关 stdin：宿主停机时靠关管道收尾，worker 不得把它当作错误。
    worker.close_stdin();
    let status = worker.child.wait().expect("等待 worker 退出");
    assert!(status.success(), "EOF 应是干净退出，实际 {status:?}");
}
