// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #73（修 M-1）端到端夹具：**装载成功之后才死**的长驻 worker。
//!
//! 复现的是审查稿 A1.5 的那一格状态：`loaded_workers` 的表项还在，子进程却已经没了 ——
//! 它正常回完 `describe`（于是 `enable()` 成功、能力入表、表项建立），随后**立即退出**，
//! 不再读也不再写任何帧。此前宿主派发命名命令会拿到 transport 错误、不命中
//! `PLUGIN_UNKNOWN_OP` ⇒ 命令**凭空失败**（= M1 的失败形态换了触发条件），
//! 而契约文档写的是「worker 中途异常退出也退回原通道」。
//!
//! 与 `e2e_legacy_worker.rs` 互补，两条夹具合起来才覆盖契约承诺的两个回退理由：
//! - 旧 worker（活着、但对命令帧回 `PLUGIN_UNKNOWN_OP`）⇒ 能力拒绝 ⇒ 回退；
//! - 本夹具（进程已不在 ⇒ 命令帧**根本没送达**）⇒ 零副作用 ⇒ 回退。
//!
//! 夹具不落任何 op 标记文件：命令帧若真被投出去，对端必须活着 —— 它已经退了，
//! 因此「安装目录里出现本夹具写的标记」本身就是缺陷证据。存活判定由宿主的
//! [`axagent_plugins::PluginManager::plugin_worker_is_running`] 提供（集成测试可观测）。

use std::io;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, FrameResponse, Inbound, PluginDeclaration, error_codes, ops,
    read_inbound, write_json_frame,
};

/// 握手应答的声明：与 `e2e_legacy_worker.rs` 同形（不提供接缝能力，只为让 `enable()` 成功）。
fn declaration() -> PluginDeclaration {
    PluginDeclaration {
        proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
        subscribe: Vec::new(),
        calls: Vec::new(),
        capabilities: Vec::new(),
    }
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    // 只处理第一帧（必然是宿主的 `describe`）：回完就走，不留进程、不读下一帧。
    let inbound = match read_inbound(&mut reader) {
        Ok(inbound) => inbound,
        // 宿主在握手前就关了管道（停机 / 崩溃）⇒ 按 `serve` 的孤儿自退语义正常退出。
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return,
        Err(e) => {
            eprintln!("e2e 已死后退出夹具读帧失败：{e}");
            std::process::exit(1);
        },
    };
    let request = match inbound {
        Inbound::Request(request) => request,
        Inbound::Response(_) => {
            eprintln!("e2e 已死后退出夹具收到非请求帧");
            std::process::exit(1);
        },
    };

    let response = if request.op == ops::DESCRIBE {
        match serde_json::to_value(declaration()) {
            Ok(value) => FrameResponse::success(value),
            Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
        }
    } else {
        // 不是 describe 就说明宿主的握手形态变了：明确失败，别让用例拿到假绿。
        FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("夹具只应答 describe，收到 {}", request.op),
        )
    };
    let response = response.with_id(request.request_id.clone().unwrap_or_default());
    if let Err(e) = write_json_frame(&mut writer, &response) {
        eprintln!("e2e 已死后退出夹具回写 describe 失败：{e}");
        std::process::exit(1);
    }
    // 回完即退：此刻起宿主手里是「表项还在、进程已没了」的连接 —— M-1 的触发条件。
    std::process::exit(0);
}
