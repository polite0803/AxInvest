// SPDX-License-Identifier: AGPL-3.0-only

//! 原 9 个声明式 OPC 域翻牌为 worker 后的通用真 bundle 端到端矩阵。
//!
//! 由 CI staging 逐域设置各自的 `<DOMAIN>_BUNDLE_DIR` 环境变量门控：未设置的域打印
//! SKIP 并跳过该行，其余行照跑；缺任一已门控域的 bundle 文件直接失败。
//! 每域验证安装 → 启用 → 代表性工具命令经 `tool.set` 逐字到达宿主替身 →
//! 自动化规则经 `opc.store` 固定注入本域 id（调用方故意传别的 capabilityPackId）→
//! 禁用后命令与域包投影一并隔离 → 恢复启用后命令可执行。
//! 命令实现只在 worker 做形状校验、固定域注入与接缝转发，工具逻辑留在宿主，
//! worker 不复制工具逻辑。game_dev / security 由各自专属 E2E 覆盖，本表不重复。

mod common;

use std::sync::Arc;

use axagent_harness::{
    OpcStoreProvider, Tool, ToolCategory, ToolContext, ToolError, ToolResult, ToolSetProvider,
    get_capability_registry,
};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use serde_json::{Value, json};

/// 每列依次是：plugin 名、domain pack id、门控环境变量、权威底册页面 id、
/// 代表性工具命令、宿主工具名、该命令在 `src-web/main.ts` SAMPLES 中的 JSON input 文本。
type Case = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const CASES: &[Case] = &[
    (
        "opc-ai-research",
        "ai_research",
        "AXAGENT_AI_RESEARCH_BUNDLE_DIR",
        "domain:data-ai-research",
        "opc_ai_research_file_read",
        "FileRead",
        r#"{"path":"README.md"}"#,
    ),
    (
        "opc-consulting",
        "consulting",
        "AXAGENT_CONSULTING_BUNDLE_DIR",
        "domain:automation-consulting",
        "opc_consulting_opc_get_dashboard",
        "OpcGetDashboard",
        "{}",
    ),
    (
        "opc-design",
        "design",
        "AXAGENT_DESIGN_BUNDLE_DIR",
        "domain:content-design",
        "opc_design_opc_create_landing_page",
        "OpcCreateLandingPage",
        "{}",
    ),
    (
        "opc-ecommerce",
        "ecommerce",
        "AXAGENT_ECOMMERCE_BUNDLE_DIR",
        "domain:automation-ecommerce",
        "opc_ecommerce_opc_list_customers",
        "OpcListCustomers",
        "{}",
    ),
    (
        "opc-education",
        "education",
        "AXAGENT_EDUCATION_BUNDLE_DIR",
        "domain:content-education",
        "opc_education_file_write",
        "FileWrite",
        r#"{"path":"note.md","content":"正文"}"#,
    ),
    (
        "opc-finance-invest",
        "finance_invest",
        "AXAGENT_FINANCE_INVEST_BUNDLE_DIR",
        "domain:finance-analysis",
        "opc_finance_invest_opc_list_kpis",
        "OpcListKpis",
        "{}",
    ),
    (
        "opc-geospatial",
        "geospatial",
        "AXAGENT_GEOSPATIAL_BUNDLE_DIR",
        "domain:data-geospatial",
        "opc_geospatial_opc_create_project",
        "OpcCreateProject",
        "{}",
    ),
    (
        "opc-project-management",
        "project_management",
        "AXAGENT_PROJECT_MANAGEMENT_BUNDLE_DIR",
        "domain:automation-projects",
        "opc_project_management_opc_add_milestone",
        "OpcAddMilestone",
        "{}",
    ),
    (
        "opc-software-dev",
        "software_dev",
        "AXAGENT_SOFTWARE_DEV_BUNDLE_DIR",
        "domain:devops-software",
        "opc_software_dev_web_search",
        "WebSearch",
        r#"{"query":"搜索关键词"}"#,
    ),
];

/// 工具替身把转发的工具名与入参序列化进 `ToolResult.content`；不落盘、不实现工具逻辑。
struct StubTool {
    name: &'static str,
}

#[async_trait::async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "OPC worker 域端到端测试替身"
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Automation
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success(json!({ "tool": self.name, "input": input }).to_string()))
    }
}

/// 一张表覆盖 9 行用到的全部工具名，避免逐域重复登记替身。
struct StubToolSet;

impl ToolSetProvider for StubToolSet {
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        CASES.iter().map(|case| Arc::new(StubTool { name: case.5 }) as Arc<dyn Tool>).collect()
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
fn worker_domains_commands_go_through_host_seams_and_respect_disable() {
    let runtime = tokio::runtime::Runtime::new().expect("端到端测试需要 tokio 运行时");
    let _entered = runtime.enter();
    let registry = get_capability_registry();
    // 接缝是单实例注册，整个矩阵共用同一对替身。
    let _tools = registry.register_tool_set(Arc::new(StubToolSet)).expect("工具集替身应能注册");
    let _store =
        registry.register_opc_store(Arc::new(StubOpcStore)).expect("opc.store 替身应能注册");

    for &(plugin_name, pack_id, env_name, page_id, command, tool_name, sample_json) in CASES {
        let Ok(dir) = std::env::var(env_name) else {
            eprintln!("SKIP: 未设置 {env_name}，跳过 {plugin_name} worker 端到端。");
            continue;
        };
        let dir = std::path::PathBuf::from(dir);
        assert!(dir.join("plugin.json").is_file(), "{plugin_name} 缺少 plugin.json");
        assert!(dir.join("bin").is_dir(), "{plugin_name} 缺少 worker 产物 bin/");
        let pack_file = format!("domain_packs/{pack_id}.json");
        assert!(dir.join(&pack_file).is_file(), "{plugin_name} 缺少 {pack_file}");
        assert!(dir.join("pages/bundle.js").is_file(), "{plugin_name} 缺少页面 bundle");
        let pack_manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(&pack_file)).expect("域包清单应可读"),
        )
        .expect("域包清单应是 JSON 文本");
        assert_eq!(pack_manifest["page"]["id"], page_id, "{plugin_name} 页面 id 与底册不一致");

        let home = common::TempDir::new(&format!("worker-{plugin_name}"));
        let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
        let outcome = manager
            .install_test_fixture(dir.to_str().expect("产物路径应是 UTF-8"))
            .unwrap_or_else(|error| panic!("{plugin_name} bundle 应能安装：{error}"));
        manager
            .enable(&outcome.plugin_id)
            .unwrap_or_else(|error| panic!("{plugin_name} enable 应成功：{error}"));

        // ① 代表性工具命令：工具名与入参必须逐字到达宿主替身。
        let sample: Value = serde_json::from_str(sample_json).expect("SAMPLES input 应是 JSON");
        let output = manager
            .execute_plugin_command(&outcome.plugin_id, command, &sample)
            .unwrap_or_else(|error| panic!("{plugin_name} 启用后 {command} 应可执行：{error}"));
        let parsed = tool_payload(&output);
        assert_eq!(parsed["tool"], tool_name, "{command} 转发到了错误的工具：{parsed:?}");
        assert_eq!(parsed["input"], sample, "{command} 的入参被改写：{parsed:?}");

        // ② 自动化规则：域 id 由 worker 固定注入，调用方无法越域。
        let rules_command = format!("opc_{pack_id}_run_automation_rules");
        let rules = manager
            .execute_plugin_command(
                &outcome.plugin_id,
                &rules_command,
                &json!({
                    "capabilityPackId": "domain-you-do-not-own",
                    "entityType": pack_id,
                    "entityId": "demo",
                    "overdueDays": 3,
                }),
            )
            .unwrap_or_else(|error| {
                panic!("{plugin_name} 启用后 {rules_command} 应可执行：{error}")
            });
        let parsed_rules: Value = serde_json::from_str(&rules).expect("规则输出应是 JSON 文本");
        assert_eq!(parsed_rules["domain"], pack_id, "不得让调用方越域：{parsed_rules:?}");
        assert_eq!(parsed_rules["entityId"], "demo");
        assert_eq!(parsed_rules["overdueDays"], 3);
        assert_eq!(parsed_rules["matchedRules"], 1);
        let packs = manager.list_plugin_domain_packs().expect("域包投影应可读");
        let projection = packs
            .iter()
            .find(|item| item.id == outcome.plugin_id)
            .unwrap_or_else(|| panic!("{plugin_name} 启用后域包投影应可见：{packs:?}"));
        assert!(projection.domain_packs.iter().any(|item| item.id == pack_id));

        // ③ 禁用后整条链路不可用，域包投影消失；恢复后仍可执行。
        manager
            .disable(&outcome.plugin_id)
            .unwrap_or_else(|error| panic!("{plugin_name} disable 应成功：{error}"));
        let err = manager.execute_plugin_command(&outcome.plugin_id, command, &sample).unwrap_err();
        assert!(
            err.to_string().contains("未启用"),
            "{plugin_name} 禁用后命令不得再执行，错误应说明禁用态：{err}"
        );
        let packs = manager.list_plugin_domain_packs().expect("禁用后投影应可读");
        assert!(
            !packs.iter().any(|item| item.id == outcome.plugin_id),
            "{plugin_name} 禁用后域包投影不得出现：{packs:?}"
        );

        manager
            .enable(&outcome.plugin_id)
            .unwrap_or_else(|error| panic!("{plugin_name} 恢复 enable 应成功：{error}"));
        let restored = manager
            .execute_plugin_command(&outcome.plugin_id, command, &sample)
            .unwrap_or_else(|error| panic!("{plugin_name} 恢复后 {command} 应可执行：{error}"));
        let parsed_restored = tool_payload(&restored);
        assert_eq!(parsed_restored["tool"], tool_name);
        assert_eq!(parsed_restored["input"], sample);
    }
}
