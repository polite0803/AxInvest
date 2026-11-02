// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #53（D1 修复）端到端夹具 worker：长驻 + **接缝帧装死**。
//!
//! 示证对象是宿主侧的接缝等待预算（`PluginWorkerConfig::seam_wait_timeout`）：
//! 收到 op=`hang` 的接缝帧后睡过宿主预算再回帧。宿主的判据是「先到点放弃、把连接
//! 标脏」，而不是「永久挂住对话回合」—— 迟到的成功帧由宿主读线程按「无等待者」
//! 丢弃（既有语义），夹具不需要感知自己已被放弃。
//!
//! 与 `e2e_command_frame_worker.rs`（#49，命令帧通道）互补：本夹具**不**承接命令帧，
//! 也**不**提供任何接缝能力（`capabilities` 为空 ⇒ 载入只走 spawn + describe 握手），
//! 测试直接经 `LoadedPlugin::invoker()` 的接缝公共入口发原始帧，路径最短、
//! 变量最少 —— 证明的是传输层的等待预算，不是某条具体门面的业务语义。

use std::time::Duration;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, FrameRequest, FrameResponse, PluginDeclaration, error_codes, ops,
    serve,
};
use serde_json::json;

/// 装死时长（毫秒）：必须**显著长于**测试注入的接缝预算（300 ms），
/// 又必须短于缺省停机上限（5 s），卸载路径才能自然收尾。
const HANG_MS: u64 = 2_500;

/// 声明：本夹具不提供任何接缝能力，也不订阅事件（只为接缝超时演示存在）。
fn declaration() -> PluginDeclaration {
    PluginDeclaration {
        proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
        subscribe: Vec::new(),
        calls: Vec::new(),
        capabilities: Vec::new(),
    }
}

fn main() {
    let outcome = serve(|request: &FrameRequest, _peer| match request.op.as_str() {
        op if op == ops::DESCRIBE => match serde_json::to_value(declaration()) {
            Ok(value) => FrameResponse::success(value),
            Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
        },
        // 「装死」演示：睡过宿主接缝预算再回帧 —— 宿主必须先到点放弃并报超时。
        "hang" => {
            std::thread::sleep(Duration::from_millis(HANG_MS));
            FrameResponse::success(json!({ "late": true }))
        },
        op if op == ops::SHUTDOWN => FrameResponse::success(json!({ "stopping": true })),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("夹具 worker 未实现 op `{other}`"),
        ),
    });
    if let Err(e) = outcome {
        eprintln!("e2e 接缝超时夹具 worker I/O 异常退出：{e}");
        std::process::exit(1);
    }
}
