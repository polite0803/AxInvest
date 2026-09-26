// SPDX-License-Identifier: AGPL-3.0-only

//! B 层端到端验证（`PLAN-plugin-gap-closure.md` §2 缺口 #1）：
//! 真实子进程 worker → describe 握手 → 接缝门面入表 → 宿主消费 → 卸载可逆回滚。
//!
//! 与单测层（ScriptedInvoker 内存替身）互补：本文件证明**帧协议 + 进程边界**
//! 本身可用，而非仅门面逻辑可用。沿用 `AXAGENT_TEST_PLUGIN_SUBPROCESS=1`
//! 门控口径（Windows 下测试 exe 锁问题见 AGENTS.md 门禁教训）。

use axagent_harness::{RuleEvaluationOutcome, get_capability_registry, workflow_types::NodeKind};
use axagent_plugins::{LoadedPlugin, PluginWorkerConfig};
use serde_json::json;

fn require_plugin_subprocess() -> bool {
    if std::env::var("AXAGENT_TEST_PLUGIN_SUBPROCESS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!(
        "SKIP: 未设置 AXAGENT_TEST_PLUGIN_SUBPROCESS=1，跳过端到端 worker 子进程测试。\
         \n      本测试**未通过，也未被验证** —— 跳过仅表示环境不允许跑子进程。"
    );
    false
}

#[test]
fn e2e_worker_business_rule_roundtrip_and_rollback() {
    if !require_plugin_subprocess() {
        return;
    }
    // 集成测试是独立进程，全局能力注册表单例在此进程内为空白，无跨用例污染。
    let registry = get_capability_registry();
    assert!(registry.get_business_rule().is_none(), "测试进程内全局表起点应为空");

    let program = std::path::PathBuf::from(env!("CARGO_BIN_EXE_e2e_business_rule_worker"));
    let loaded = LoadedPlugin::load(PluginWorkerConfig::new("e2e-worker@external", program))
        .expect("真实 worker 应完成 spawn + describe 握手 + 能力注册");

    // 宿主消费方零改动：经既有接缝 getter 取到的就是远程门面（「平权」端到端成立）。
    let rule = registry.get_business_rule().expect("worker 提供的 business_rule 应已入表");
    let outcome = rule.evaluate(&NodeKind::Tool, &json!(42));
    match outcome {
        RuleEvaluationOutcome::Violation { rule_name, .. } => {
            assert_eq!(rule_name, "e2e-denies-numeric-tool-input");
        },
        other => panic!("期望夹具规则命中 Violation，实际 {other:?}"),
    }
    assert!(
        matches!(rule.evaluate(&NodeKind::Agent, &json!(42)), RuleEvaluationOutcome::Pass),
        "夹具对非 Tool 节点应放行"
    );

    // 卸载 = 可逆回滚：句柄撤销后接缝不得残留 provider。
    drop(loaded);
    assert!(registry.get_business_rule().is_none(), "drop 后应 LIFO 回滚注册");
}
