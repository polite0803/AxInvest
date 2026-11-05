// SPDX-License-Identifier: AGPL-3.0-only
//! 真进程 + 真管道的协议测试：宿主将来就是按这个形态 spawn 它的。
//!
//! 为什么不用进程内替代：`stdin`/`stdout` 的缓冲与 flush、以及「等响应期间对端又发来
//! 请求」的重入语义，只有在真管上才暴露。本文件只锁 #36 第 1 批的**骨架契约**：
//! 声明是空表、身份是 `plugin.opc`、未知命令显式失败、停机正常退出。

use std::io::{BufReader, BufWriter};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFrame, FrameRequest, FrameResponse, PluginDeclaration,
    ResponseKind, error_codes, ops, read_inbound, write_json_frame,
};

/// 与宿主 `PluginWorkerConfig::plugin_id` 对齐的身份（命令帧的身份自检拿它比对）。
///
/// 注意：协议声明里**没有** plugin id 字段（id 由宿主清单持有、经环境变量注入），
/// 所以「describe 返回 plugin id」在线上形态是「describe 的声明合法 + 命令回执回填
/// 这个 id」，本文件两件都断言。
const PLUGIN_ID: &str = "plugin.opc";

struct Worker {
    child: Child,
    /// `Option` 只为一件事：停机前要真的关掉写端（EOF 是 worker 的停机信号）。
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
}

impl Worker {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_axagent-plugin-opc"))
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
fn describe_is_empty_and_identity_is_plugin_opc() {
    let mut worker = Worker::spawn();

    // ① 握手：声明必须合法，且**如实**为空（本批零能力、零出站接缝）。
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
    assert!(declaration.subscribe.is_empty(), "本批不订阅任何事件：{:?}", declaration.subscribe);
    assert!(declaration.calls.is_empty(), "本批不调用任何宿主接缝：{:?}", declaration.calls);
    assert!(
        declaration.capabilities.is_empty(),
        "本批不提供任何接缝能力：{:?}",
        declaration.capabilities
    );

    // ② 命令表为空：任何命令名都必须显式报未知，且回执回填本插件的身份。
    worker.send(
        &CommandFrame::new(PLUGIN_ID, "opc_not_implemented", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求")
            .with_id("req-command"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(response.request_id.as_deref(), Some("req-command"));
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_UNKNOWN),
        "空命令表必须回未知命令，不得静默成功"
    );
    let binding = response.value.expect("错误帧必须透传绑定字段");
    assert_eq!(binding["plugin_id"], PLUGIN_ID, "回执必须回填宿主绑定的身份");
    assert_eq!(binding["command"], "opc_not_implemented");

    // ③ 身份不符：帧绑定别的插件 ⇒ 拒执行（与未知命令是两个错误码）。
    worker.send(
        &CommandFrame::new("plugin.other", "opc_not_implemented", serde_json::json!({}), 5_000)
            .to_frame_request()
            .expect("命令帧应能编成请求"),
    );
    let response = worker.recv_response();
    assert_eq!(response.kind, ResponseKind::Error);
    assert_eq!(
        response.code.as_deref(),
        Some(error_codes::PLUGIN_COMMAND_ID_MISMATCH),
        "帧绑定他人身份必须拒执行"
    );

    // ④ 停机：shutdown 回执后关写端，worker 以 0 退出（宿主停机不走 kill）。
    worker.send(&FrameRequest::new(ops::SHUTDOWN, serde_json::json!({})));
    assert_eq!(worker.recv_response().kind, ResponseKind::Success);
    worker.close_stdin();
    let status = worker.child.wait().expect("等待 worker 退出");
    assert!(status.success(), "收到 shutdown 并关管道后应以 0 退出，实际 {status:?}");
}
