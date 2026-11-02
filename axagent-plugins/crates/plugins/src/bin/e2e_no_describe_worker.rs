// SPDX-License-Identifier: AGPL-3.0-only

//! 任务 #63（补 D1 的握手边角）端到端夹具 worker：**读到帧就装死，连 `describe` 都不回**。
//!
//! 复现的形态是「进程起来了、也读到了握手帧，但就是不回声明」。修复前宿主的握手
//! （`WorkerInvoker::handshake`）走**无限等待** —— 而它不在接缝公共入口里，任务 #53 的
//! 接缝预算对它完全不生效，于是一个僵死 worker 能把整条插件装载路径永久挂住。
//!
//! 与 `e2e_seam_timeout_worker.rs`（#53）刻意相反：那个夹具**先**正常回掉 describe、
//! 只对接缝帧装死；本夹具对**任何** op 一律睡过 [`HANG_MS`] 再回一条错误 ——
//! 测试注入的握手预算（300 ms）到点时宿主必须已经放弃等待。迟到帧由宿主读线程按
//! 「无等待者」丢弃（既有语义），夹具不需要感知自己已被放弃。
//!
//! 夹具把收到的每个 op 追加进 spawn 时 cwd 的 `handshake_op.marker`：标记里出现
//! `op=describe` 才证明失败原因是「**不回帧**」，而不是「进程根本没起来 / 管道没接上」——
//! 两者在宿主侧同样表现为握手失败，但只有前者是本用例要演的场景。

use std::fs::OpenOptions;
use std::io::Write;
use std::time::Duration;

use axagent_plugin_proto::{FrameRequest, FrameResponse, error_codes, serve};

/// 装死时长（毫秒）：必须**显著长于**测试注入的握手预算（300 ms）与停机上限，
/// 短到仍远小于用例的收口硬上界（10 s）⇒ 「夹具装死」与「缺陷复发导致真挂死」可区分。
const HANG_MS: u64 = 20_000;

/// 把收到的 op 追加进 cwd 的标记文件（宿主按安装目录设 cwd）。
fn note_op(op: &str) {
    let marker = std::path::Path::new("handshake_op.marker");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(marker) {
        let _ = writeln!(file, "op={op}");
    }
}

fn main() {
    // 刻意不按 plugin-proto 的 handler 模板回 describe：握手都拒不完成，才能演
    // 「装载被僵死 worker 卡住」这条病灶。
    let outcome = serve(|request: &FrameRequest, _peer| {
        note_op(&request.op);
        std::thread::sleep(Duration::from_millis(HANG_MS));
        FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, "夹具装死后才回的迟到帧")
    });
    if let Err(e) = outcome {
        eprintln!("e2e 握手装死夹具 worker I/O 异常退出：{e}");
        std::process::exit(1);
    }
}
