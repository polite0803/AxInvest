// SPDX-License-Identifier: AGPL-3.0-only

//! game_dev 域 worker 的真 bundle 安装 / 执行 / 禁用 / 恢复端到端。
//!
//! 与声明式插件的「页面数据存在即通过」不同，这里真起 Rust worker 进程：
//! 四条域命令分别经 `tool.set/exec`（WebSearch / FileWrite / FileRead）与
//! `opc.store::run_automation_rules` 到达宿主替身，验证命令名、工具名、域 id
//! 和安装态链路全部贯通。门控：`AXAGENT_GAME_DEV_BUNDLE_DIR`。

mod common;

use std::sync::Arc;

use axagent_harness::{
    OpcStoreProvider, Tool, ToolCategory, ToolContext, ToolError, ToolResult, ToolSetProvider,
    get_capability_registry,
};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use serde_json::{Value, json};

/// 记录工具调用的工具替身；文件写工具本身不落盘，只证明转发到了正确的工具名。
struct StubTool {
    name: &'static str,
    category: ToolCategory,
}

#[async_trait::async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "game_dev 端到端测试替身"
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    fn category(&self) -> ToolCategory {
        self.category
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success(json!({ "tool": self.name, "input": input }).to_string()))
    }
}

struct StubToolSet;

impl ToolSetProvider for StubToolSet {
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(StubTool { name: "WebSearch", category: ToolCategory::Network }),
            Arc::new(StubTool { name: "FileRead", category: ToolCategory::FileRead }),
            Arc::new(StubTool { name: "FileWrite", category: ToolCategory::FileWrite }),
        ]
    }
}

/// 只实现自动化规则一条数据面；其余 op 用默认未实现分支。
struct StubOpcStore;

#[async_trait::async_trait]
impl OpcStoreProvider for StubOpcStore {
    async fn run_automation_rules(
        &self,
        capability_pack_id: String,
        entity_type: String,
        entity_id: String,
        status: Option<String>,
        overdue_days: Option<u32>,
        _created_days: Option<u32>,
        _fields: Option<Value>,
    ) -> axagent_harness::core_error::Result<Value> {
        Ok(json!({
            "domain": capability_pack_id,
            "entityType": entity_type,
            "entityId": entity_id,
            "status": status,
            "overdueDays": overdue_days,
            "matchedRules": 1,
        }))
    }
}

/// 工具替身把转发的工具名与入参序列化进 `ToolResult.content`；命令回执的外层是
/// camelCase 的 `ToolResult`，断言前先取出 content 再解一次。
fn tool_payload(output: &str) -> Value {
    let envelope: Value = serde_json::from_str(output).expect("工具输出应是 JSON 文本");
    let content = envelope["content"].as_str().expect("工具回执应有 content");
    serde_json::from_str(content).expect("content 应是工具替身写入的 JSON 文本")
}

#[test]
fn game_dev_bundle_commands_go_through_host_seams_and_respect_disable() {
    let Ok(dir) = std::env::var("AXAGENT_GAME_DEV_BUNDLE_DIR") else {
        eprintln!(
            "SKIP: 未设置 AXAGENT_GAME_DEV_BUNDLE_DIR，跳过 game_dev worker 端到端。\
             \n      跑法：node axagent-plugins/scripts/build-plugin-bundle.mjs --plugin opc-game-dev \
             后指向 axagent-plugins/dist/plugins/opc-game-dev。"
        );
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    assert!(dir.join("plugin.json").is_file());
    assert!(dir.join("bin").is_dir());
    assert!(dir.join("domain_packs/game_dev.json").is_file());
    assert!(dir.join("pages/bundle.js").is_file(), "worker 域必须带页面 bundle");

    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    let _tools = registry.register_tool_set(Arc::new(StubToolSet)).expect("工具集替身应能注册");
    let _store =
        registry.register_opc_store(Arc::new(StubOpcStore)).expect("opc.store 替身应能注册");

    let home = common::TempDir::new("game-dev-bundle-home");
    let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
    let outcome = manager
        .install_test_fixture(dir.to_str().expect("产物路径应是 UTF-8"))
        .expect("game_dev bundle 应能安装");
    manager.enable(&outcome.plugin_id).expect("enable 应成功");

    // ① 三条工具命令：工具名与入参必须逐字到达宿主替身。
    let cases = [
        ("opc_game_dev_web_search", "WebSearch", json!({ "query": "关卡设计" })),
        ("opc_game_dev_file_read", "FileRead", json!({ "path": "README.md" })),
        ("opc_game_dev_file_write", "FileWrite", json!({ "path": "note.md", "content": "正文" })),
    ];
    for (command, tool_name, input) in cases {
        let output = manager
            .execute_plugin_command(&outcome.plugin_id, command, &input)
            .unwrap_or_else(|e| panic!("启用后 {command} 应可执行：{e}"));
        let parsed = tool_payload(&output);
        assert_eq!(parsed["tool"], tool_name, "{command} 转发到了错误的工具：{parsed:?}");
        assert_eq!(parsed["input"], input, "{command} 的入参被改写：{parsed:?}");
    }

    // ② 自动化规则：域 id 由 worker 固定注入，调用方无法越域。
    let rules = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_game_dev_run_automation_rules",
            &json!({
                "capabilityPackId": "security",
                "entityType": "game",
                "entityId": "g-1",
                "overdueDays": 3,
            }),
        )
        .expect("启用后自动化规则命令应可执行");
    let parsed_rules: Value = serde_json::from_str(&rules).expect("规则输出应是 JSON 文本");
    assert_eq!(parsed_rules["domain"], "game_dev", "不得让调用方越域：{parsed_rules:?}");
    assert_eq!(parsed_rules["entityId"], "g-1");
    assert_eq!(parsed_rules["overdueDays"], 3);
    assert_eq!(parsed_rules["matchedRules"], 1);

    // ③ 禁用后整条链路不可用，域包投影消失；恢复后仍可执行。
    manager.disable(&outcome.plugin_id).expect("disable 应成功");
    let err = manager
        .execute_plugin_command(&outcome.plugin_id, "opc_game_dev_web_search", &json!({}))
        .expect_err("禁用后命令不得再执行");
    assert!(err.to_string().contains("未启用"), "错误应说明禁用态：{err}");
    let packs = manager.list_plugin_domain_packs().expect("域包投影应可读");
    assert!(
        !packs.iter().any(|p| p.id == outcome.plugin_id),
        "禁用后 game_dev 插件的域包投影不得出现：{packs:?}"
    );

    manager.enable(&outcome.plugin_id).expect("恢复启用应成功");
    let restored = manager
        .execute_plugin_command(
            &outcome.plugin_id,
            "opc_game_dev_file_read",
            &json!({ "path": "AGENTS.md" }),
        )
        .expect("恢复后命令应可执行");
    let parsed_restored = tool_payload(&restored);
    assert_eq!(parsed_restored["tool"], "FileRead");
}
