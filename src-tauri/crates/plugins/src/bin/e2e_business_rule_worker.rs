// SPDX-License-Identifier: AGPL-3.0-only

//! 端到端 B 层 worker 插件夹具（`PLAN-plugin-gap-closure.md` §2 缺口 #1）。
//!
//! 这是仓内**第一个真实声明 `worker` 的插件可执行文件**：以
//! `axagent-plugin-proto` 的帧协议长驻 stdin/stdout，向宿主声明并提供
//! `workflow.business_rule` 接缝。集成测试
//! （`tests/e2e_worker_test.rs`）经 `LoadedPlugin::load` 走
//! 「spawn → describe 握手 → 注册门面 → 宿主消费 → 卸载回滚」全链。

use axagent_harness::{RuleAction, RuleEvaluationOutcome, workflow_types::NodeKind};
use axagent_plugin_proto::{
    AXAGENT_PLUGIN_PROTO_VERSION, CapabilityDecl, FrameRequest, FrameResponse, PluginDeclaration,
    error_codes, ops, serve,
};

fn declaration() -> PluginDeclaration {
    PluginDeclaration {
        proto_version: AXAGENT_PLUGIN_PROTO_VERSION,
        subscribe: Vec::new(),
        calls: Vec::new(),
        capabilities: vec![CapabilityDecl {
            seam: "workflow.business_rule".to_string(),
            op: "evaluate".to_string(),
            version: "1.0".to_string(),
        }],
    }
}

fn evaluate(node_type: NodeKind, node_input: serde_json::Value) -> RuleEvaluationOutcome {
    // 夹具判据：对 Tool 节点的数字输入一律拒绝，其余放行 —— 返回值会跨帧协议
    // 往返，宿主侧门面反序列化后照常消费，以此验证「平权」语义。
    let numeric = node_input.is_number();
    match node_type {
        NodeKind::Tool if numeric => RuleEvaluationOutcome::Violation {
            rule_name: "e2e-denies-numeric-tool-input".to_string(),
            rule_description: "端到端夹具规则：Tool 节点不接受纯数字输入".to_string(),
            action: RuleAction::Warn("夹具拒绝".to_string()),
            reason: format!("worker 看到数字输入：{node_input}"),
        },
        _ => RuleEvaluationOutcome::Pass,
    }
}

fn main() {
    let outcome = serve(|req: &FrameRequest, _peer| match req.op.as_str() {
        op if op == ops::DESCRIBE => match serde_json::to_value(declaration()) {
            Ok(value) => FrameResponse::success(value),
            Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
        },
        "evaluate" => {
            let node_type: NodeKind = match req.args.get("node_type").cloned() {
                Some(raw) => match serde_json::from_value(raw) {
                    Ok(node_type) => node_type,
                    Err(e) => {
                        return FrameResponse::error(
                            error_codes::SEAM_CALL_INVALID_ARGS,
                            format!("node_type 反序列化失败：{e}"),
                        );
                    },
                },
                None => {
                    return FrameResponse::error(
                        error_codes::SEAM_CALL_INVALID_ARGS,
                        "evaluate 需要 node_type 参数",
                    );
                },
            };
            let node_input = req.args.get("node_input").cloned().unwrap_or(serde_json::Value::Null);
            match serde_json::to_value(evaluate(node_type, node_input)) {
                Ok(value) => FrameResponse::success(value),
                Err(e) => FrameResponse::error(error_codes::PLUGIN_CALL_FAILED, e.to_string()),
            }
        },
        other => FrameResponse::error(
            error_codes::PLUGIN_UNKNOWN_OP,
            format!("夹具 worker 未实现 op `{other}`"),
        ),
    });
    if let Err(e) = outcome {
        eprintln!("e2e 夹具 worker I/O 异常退出：{e}");
        std::process::exit(1);
    }
}
