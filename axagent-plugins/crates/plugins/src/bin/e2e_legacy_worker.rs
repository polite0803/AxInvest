// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #59（修 M1 回归）端到端夹具：**旧形态**长驻 worker（未实现 `ops::COMMAND`）。
//!
//! 复现的对象是任务 #49 之前就已装机的插件形态：worker 二进制按 plugin-proto 的 handler
//! 模板只实现 `describe`（握手必需）＋兜底分支，兜底分支对**任何**未实现的 op（包括新
//! 宿主发来的 `command` 帧）回 [`error_codes::PLUGIN_UNKNOWN_OP`] 错误信封。
//! 它自己不认识命令帧，也**不会**落任何「命令跑过」的标记 —— 这正是 M1 的触发条件：
//! 宿主若只把这条错误归成 `CommandFailed`，存量插件的命名命令就凭空消失了。
//!
//! 与 `e2e_command_frame_worker.rs`（新 worker，承接 `ops::COMMAND`）互补，两个夹具
//! 同仓共存，用于同时钉住「新 worker 走帧通道」与「旧 worker 回退 spawn 通道」。
//!
//! 夹具额外把**收到的每个 op** 追加进安装目录（= spawn 时 cwd）的 `legacy_op.marker`，
//! 让集成测试能客观证明两件事：① 命令帧**确实派发过**（不是宿主偷偷跳过通道一）；
//! ② 派发次数恰好一次（回退没有变成「帧 ⇄ spawn」双跑循环）。
//! 本夹具**不执行任何命令逻辑**，因此它的标记与「命令真跑了」无关 ——
//! 「真跑了」的证据由同目录的 `ran.marker` 承担（由 spawn 通道的脚本落盘）。

use std::fs::OpenOptions;
use std::io::Write;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, FrameRequest, FrameResponse, PluginDeclaration, error_codes, ops,
    serve,
};

/// 握手用的声明：与 #49 夹具同形（不提供接缝能力，只为让 `enable()` 成功长驻）。
fn declaration() -> PluginDeclaration {
    PluginDeclaration {
        proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
        subscribe: Vec::new(),
        calls: Vec::new(),
        capabilities: Vec::new(),
    }
}

/// 把收到的 op 追加进 cwd 的标记文件（宿主按安装目录设 cwd）。
fn note_op(op: &str) {
    let marker = std::path::Path::new("legacy_op.marker");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(marker) {
        let _ = writeln!(file, "op={op}");
    }
}

fn main() {
    // 刻意只实现 `describe`：其余 op（含 `command` / `shutdown` / `ui.action`）一律走
    // 兜底分支，形态逐字对齐 plugin-proto 文档里的 handler 模板。
    let outcome = serve(|request: &FrameRequest, _peer| {
        note_op(&request.op);
        match request.op.as_str() {
            op if op == ops::DESCRIBE => match serde_json::to_value(declaration()) {
                Ok(value) => FrameResponse::success(value),
                Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
            },
            other => {
                FrameResponse::error(error_codes::PLUGIN_UNKNOWN_OP, format!("未知操作：{other}"))
            },
        }
    });
    if let Err(e) = outcome {
        eprintln!("e2e 旧形态夹具 worker I/O 异常退出：{e}");
        std::process::exit(1);
    }
}
