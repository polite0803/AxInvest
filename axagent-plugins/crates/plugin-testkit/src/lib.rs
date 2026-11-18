// SPDX-License-Identifier: AGPL-3.0-only
//! 插件 SDK 的契约快照与测试夹具。
//!
//! 目的：让插件作者在自己的独立仓里用**同一份 SDK 夹具**验证帧形状、握手声明与
//! 命令回执，而不是各自手抄一遍协议常量。本 crate 只做纯函数级构造与断言，
//! 不启动进程、不依赖宿主运行时，也不依赖任何业务实现层。
//!
//! 使用方式（插件仓测试）：
//!
//! ```no_run
//! use axagent_plugin_testkit::{command_frame, describe_frame, declaration_of};
//!
//! # fn demo() {
//! let describe = describe_frame();
//! let command = command_frame("opc@bundled", "opc_list_invoices", serde_json::json!({}), 5_000);
//! assert_eq!(describe.op, "describe");
//! assert_eq!(command.op, "command");
//! # }
//! ```

use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CommandFrame, FrameRequest, FrameResponse, PluginDeclaration,
    ResponseKind, ops,
};
use serde_json::{Value, json};

/// 构造 `describe` 请求（协议版本随 SDK 走，插件仓不必手抄）。
#[must_use]
pub fn describe_frame() -> FrameRequest {
    FrameRequest::new(ops::DESCRIBE, json!({ "proto_version": AXAGENT_PLUGIN_PROTO_VERSION }))
}

/// 构造 `shutdown` 请求。
#[must_use]
pub fn shutdown_frame() -> FrameRequest {
    FrameRequest::new(ops::SHUTDOWN, json!({}))
}

/// 构造命令帧请求。
///
/// `plugin_id` 必须是宿主注册表键（`{name}@{marketplace}`）；`timeout_ms` 是宿主执行预算。
#[must_use]
pub fn command_frame(
    plugin_id: impl Into<String>,
    command: impl Into<String>,
    input: Value,
    timeout_ms: u64,
) -> FrameRequest {
    CommandFrame::new(plugin_id, command, input, timeout_ms)
        .to_frame_request()
        .expect("CommandFrame 是纯数据，序列化不会失败")
}

/// 断言一次成功回执里的声明与期望一致。
///
/// 返回解码后的 [`PluginDeclaration`]，调用方可继续做更细的断言。
pub fn declaration_of(response: &FrameResponse) -> PluginDeclaration {
    assert_eq!(response.kind, ResponseKind::Success, "describe 必须成功：{response:?}");
    let value = response.value.clone().expect("成功帧必须带 value");
    let declaration: PluginDeclaration =
        serde_json::from_value(value).expect("describe 回帧必须是合法 PluginDeclaration");
    declaration.validate().expect("声明必须通过宿主同一套校验");
    declaration
}

/// 断言命令成功回执里的 `value` 与期望相同。
///
/// 回执走 `CommandReply` 信封（`{plugin_id, command, value}`），本函数只比较 `value` 字段；
/// 这是 SDK 侧的契约快照，避免插件仓各自写一套解包逻辑。
pub fn assert_command_value(response: &FrameResponse, expected: &Value) {
    assert_eq!(response.kind, ResponseKind::Success, "命令必须成功：{response:?}");
    let value = response.value.clone().expect("成功帧必须带 value");
    assert_eq!(&value["value"], expected, "命令 value 与契约定稿不一致：{value:?}");
}

/// 断言命令失败回执的错误码。
pub fn assert_command_error_code(response: &FrameResponse, expected_code: &str) {
    assert_eq!(response.kind, ResponseKind::Error, "期望错误回执：{response:?}");
    assert_eq!(response.code.as_deref(), Some(expected_code), "错误码不匹配：{response:?}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_plugin_proto::{CommandFault, CommandReply};

    #[test]
    fn describe_frame_carries_the_sdk_protocol_version() {
        let frame = describe_frame();
        assert_eq!(frame.op, ops::DESCRIBE);
        assert_eq!(frame.args["proto_version"], AXAGENT_PLUGIN_PROTO_VERSION);
    }

    #[test]
    fn command_frame_produces_a_decodable_command_envelope() {
        let request =
            command_frame("opc@bundled", "opc_list_invoices", json!({ "limit": 10 }), 5_000);
        let frame = CommandFrame::from_frame_request(&request).expect("命令帧应可解码");
        assert_eq!(frame.plugin_id, "opc@bundled");
        assert_eq!(frame.command, "opc_list_invoices");
        assert_eq!(frame.input["limit"], 10);
        assert_eq!(frame.timeout_ms, 5_000);
    }

    #[test]
    fn declaration_helper_accepts_a_valid_declaration() {
        let declaration = PluginDeclaration {
            proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
            subscribe: Vec::new(),
            calls: vec!["opc.store".to_string()],
            capabilities: Vec::new(),
        };
        let response =
            FrameResponse::success(serde_json::to_value(declaration).expect("声明可序列化"));
        let decoded = declaration_of(&response);
        assert_eq!(decoded.proto_version, AXAGENT_PLUGIN_PROTO_VERSION);
    }

    #[test]
    fn command_value_helper_matches_the_reply_envelope() {
        let frame = CommandFrame::new("opc@bundled", "opc_list_invoices", json!({}), 5_000);
        let response = CommandReply::for_frame(&frame, json!({ "rows": 1 }))
            .into_frame_response()
            .expect("回执可序列化");
        assert_command_value(&response, &json!({ "rows": 1 }));
    }

    #[test]
    fn command_error_helper_reads_the_fault_code() {
        let frame = CommandFrame::new("opc@bundled", "opc_list_invoices", json!({}), 5_000);
        let response = CommandFault::for_frame(&frame, "PLUGIN_COMMAND_INVALID_ARGS", "bad")
            .into_frame_response();
        assert_command_error_code(&response, "PLUGIN_COMMAND_INVALID_ARGS");
    }
}
