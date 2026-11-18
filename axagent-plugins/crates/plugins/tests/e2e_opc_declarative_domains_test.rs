// SPDX-License-Identifier: AGPL-3.0-only
//!
//! 11 个声明式 OPC 域包的真实 bundle 启停矩阵。
//!
//! 每域验证：安装 → 启用 → 页面资产可解析 → 域包投影可见 → 禁用后页面/投影消失 →
//! 恢复后两者回来。门控 `AXAGENT_DECLARATIVE_BUNDLE_ROOT`；CI 在 nextest 前串行 staging
//! 全部 11 个 bundle，缺失任一产物都直接失败，不把 SKIP 当绿。

mod common;

use axagent_plugins::{PluginManager, PluginManagerConfig};

const CASES: &[(&str, &str, &str, &str)] = &[
    ("opc-ai-research", "opc-ai-research", "ai_research", "plugin-opc-common"),
    ("opc-consulting", "opc-consulting", "consulting", "plugin-opc-common"),
    ("opc-design", "opc-design", "design", "plugin-opc-common"),
    ("opc-ecommerce", "opc-ecommerce", "ecommerce", "plugin-opc-common"),
    ("opc-education", "opc-education", "education", "plugin-opc-common"),
    ("opc-finance-invest", "opc-finance-invest", "finance_invest", "plugin-opc-common"),
    ("opc-game-dev", "opc-game-dev", "game_dev", "plugin-opc-common"),
    ("opc-geospatial", "opc-geospatial", "geospatial", "plugin-opc-common"),
    ("opc-project-management", "opc-project-management", "project_management", "plugin-opc-common"),
    ("opc-security", "opc-security", "security", "plugin-opc-common"),
    ("opc-software-dev", "opc-software-dev", "software_dev", "plugin-opc-common"),
];

#[test]
fn every_declarative_domain_bundle_installs_toggles_and_projects_its_page() {
    let Ok(root) = std::env::var("AXAGENT_DECLARATIVE_BUNDLE_ROOT") else {
        eprintln!("SKIP: 未设置 AXAGENT_DECLARATIVE_BUNDLE_ROOT，跳过声明式域包矩阵。");
        return;
    };
    let root = std::path::PathBuf::from(root);

    for (plugin_name, page_id, pack_id, owner) in CASES {
        let dir = root.join(plugin_name);
        assert!(dir.join("plugin.json").is_file(), "缺少 bundle：{}", dir.display());
        assert!(dir.join("pages/index.html").is_file(), "缺少页面：{}", dir.display());
        assert!(dir.join("pages/domain-data.js").is_file(), "缺少页面数据：{}", dir.display());
        assert!(
            dir.join("domain_packs").join(format!("{pack_id}.json")).is_file(),
            "缺少域包清单：{}",
            dir.display()
        );

        let home = common::TempDir::new(&format!("declarative-{plugin_name}"));
        let mut manager = PluginManager::new(PluginManagerConfig::new(&home.path));
        let outcome = manager
            .install(dir.to_str().expect("产物路径应是 UTF-8"))
            .unwrap_or_else(|error| panic!("{plugin_name} bundle 应能安装：{error}"));
        manager
            .enable(&outcome.plugin_id)
            .unwrap_or_else(|error| panic!("{plugin_name} enable 应成功：{error}"));

        let located = manager
            .resolve_page_asset(&outcome.plugin_id, page_id)
            .unwrap_or_else(|error| panic!("{plugin_name} 页面应可解析：{error}"));
        let page_path = std::path::PathBuf::from(&located.absolute_path);
        assert!(page_path.is_file());
        let html = std::fs::read_to_string(&page_path).expect("页面入口应可读");
        assert!(html.contains("./domain-data.js"), "{plugin_name} 页面未加载数据资产");

        let packs = manager.list_plugin_domain_packs().expect("域包投影应可读");
        let pack = packs
            .iter()
            .find(|item| item.id == outcome.plugin_id)
            .unwrap_or_else(|| panic!("{plugin_name} 启用后投影应可见：{packs:?}"));
        assert!(pack.enabled);
        assert_eq!(pack.domain_packs.len(), 1);
        assert_eq!(pack.domain_packs[0].id, *pack_id);
        assert_eq!(pack.domain_packs[0].transport_owner, *owner);
        assert!(pack.domain_packs[0].commands.is_empty(), "{plugin_name} 不得复制命令 owner");

        manager
            .disable(&outcome.plugin_id)
            .unwrap_or_else(|error| panic!("{plugin_name} disable 应成功：{error}"));
        assert!(
            manager.resolve_page_asset(&outcome.plugin_id, page_id).is_err(),
            "{plugin_name} 禁用后页面必须不可解析"
        );
        let packs = manager.list_plugin_domain_packs().expect("禁用后投影应可读");
        assert!(
            !packs.iter().any(|item| item.id == outcome.plugin_id),
            "{plugin_name} 禁用后投影必须消失：{packs:?}"
        );

        manager
            .enable(&outcome.plugin_id)
            .unwrap_or_else(|error| panic!("{plugin_name} 恢复 enable 应成功：{error}"));
        assert!(
            manager.resolve_page_asset(&outcome.plugin_id, page_id).is_ok(),
            "{plugin_name} 恢复后页面应可解析"
        );
        let packs = manager.list_plugin_domain_packs().expect("恢复后投影应可读");
        assert!(packs.iter().any(|item| item.id == outcome.plugin_id));
    }
}
