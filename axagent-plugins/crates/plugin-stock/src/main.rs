// SPDX-License-Identifier: AGPL-3.0-only
//! 股票域插件 worker 的骨架（B4-a）。
//!
//! 宿主以 `worker.program` 相对路径 spawn 本进程，双方在 stdin/stdout 上交换长度前缀
//! JSON 帧（`axagent-plugin-proto`）。诊断信息一律走 stderr —— stdout 是协议通道，
//! 混入任何非帧字节都会让宿主读帧失败。
//!
//! 本片只交付**通道**：握手（`describe`）与停机（`shutdown`）。域逻辑随 B4 后续批次搬入，
//! 搬入前**不得**在声明里写空能力 —— 「声明了但没实现」在宿主侧是一条调不通的接缝，
//! 属于本仓明令禁止的半条链。未实现的操作一律回 `PLUGIN_UNKNOWN_OP`，不静默成功。
//!
//! 为什么是纯可执行包（只有 `src/main.rs`、没有 `src/lib.rs`）：分层门禁规则 4 要求每个
//! 库 crate 至少被一份 `Cargo.toml` 引用，而 worker 由宿主 spawn、没有 Cargo 消费方；
//! 门禁对「纯可执行包」已有豁免先例（`crates/schema-gen`），本 crate 照同一形态。

use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::ExitCode;

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, FrameRequest, FrameResponse, PluginDeclaration, error_codes, ops,
    read_json_frame, write_json_frame,
};

/// 逐帧读请求、回响应，直到对端关闭输入或发来 `shutdown`。
///
/// 返回处理的帧数（含 `shutdown`）—— 测试据此区分「走完了握手」与「提前 EOF 空转」。
/// 正常的对端关闭（EOF）不是错误：宿主停插件时就靠关 stdin 收尾。
fn serve<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<usize> {
    let mut handled = 0usize;
    loop {
        let at_eof = reader.fill_buf()?.is_empty();
        if at_eof {
            return Ok(handled);
        }
        let request: FrameRequest = read_json_frame(reader)?;
        let shutdown = request.op == ops::SHUTDOWN;
        write_json_frame(writer, &handle(&request))?;
        handled += 1;
        if shutdown {
            return Ok(handled);
        }
    }
}

fn handle(request: &FrameRequest) -> FrameResponse {
    let response = match request.op.as_str() {
        ops::DESCRIBE => FrameResponse::success(
            serde_json::to_value(PluginDeclaration {
                proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
                subscribe: Vec::new(),
                calls: Vec::new(),
                capabilities: Vec::new(),
            })
            .expect("插件声明是可序列化的纯数据"),
        ),
        ops::SHUTDOWN => FrameResponse::success(serde_json::json!({ "ok": true })),
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("股票插件尚未实现该操作：{other}"),
        ),
    };
    match &request.request_id {
        Some(id) => response.with_id(id.clone()),
        None => response,
    }
}

fn main() -> ExitCode {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    match serve(&mut reader, &mut writer) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[axagent-plugin-stock] 帧通道失败：{e}");
            ExitCode::FAILURE
        },
    }
}
