// SPDX-License-Identifier: AGPL-3.0-only

//! 插件 `schema.json` → 库上的 **additive-only** 收敛（任务 `#134` 第二片）。
//!
//! 挂载点见 `PLAN-shell-plus-plugins.md` §69.3：安装 / 启用 / 启动各一次，
//! 卸载**零 DDL**。本模块只负责「找文件 → 解析 → 一次 cycle」，不持有任何表级白名单
//! —— 「卸载不删表」由 `bootstrap_options()` 的 additive-only 与零破坏配额保证。
//!
//! ## 为什么一次传全部插件，而不是逐个 cycle
//!
//! 两份插件声明同一张表必须**当场报错**（`build_with_external` 的判据 2）。
//! 逐个应用时，第二份会先命中「与期望集重名」，错误信息就退化成「和宿主打架」，
//! 把真正的「两份插件声明打架」掩盖掉 —— 与 §68 踩到的判据顺序坑同型。

use std::path::PathBuf;

use axagent_dao::reconcile::{
    apply::{bootstrap_options, cycle_with_external, dialect_of},
    external::{ExternalSchema, SCHEMA_FILE_NAME},
    model::TableModel,
};
use axagent_plugins::PluginManager;
use sea_orm::DatabaseConnection;
use tokio::sync::RwLock;

/// 把**已安装**插件的 `schema.json` 收敛进库；`only` 用于安装 / 启用后只跑该插件。
///
/// 返回实际带表声明的插件数（没有 `schema.json` 或声明为空的不计）。
/// 任何一步失败都返回 `Err(String)`：安装 / 启用路径据此把错误交给用户；
/// 启动路径只 `error!` 并继续（一个坏声明不能挡住 App 启动）。
pub async fn reconcile_installed(
    db: &DatabaseConnection,
    plugin_manager: &RwLock<PluginManager>,
    only: Option<&str>,
) -> Result<usize, String> {
    let (roots, failed_plugins) = {
        let manager = plugin_manager.read().await;
        let report =
            manager.plugin_registry_report().map_err(|e| format!("读取插件清单失败：{e}"))?;
        let failures = report.failures().len();
        let roots: Vec<(String, PathBuf)> = report
            .summaries()
            .into_iter()
            .filter(|s| only.is_none_or(|id| s.metadata.id == id))
            .filter_map(|s| s.metadata.root.map(|root| (s.metadata.id, root)))
            .collect();
        (roots, failures)
    };
    if failed_plugins > 0 {
        tracing::warn!(
            failed = failed_plugins,
            "插件 schema.json 收敛：有 {failed_plugins} 个插件装载失败，已跳过（其声明不参与本轮）"
        );
    }

    let dialect = dialect_of(db).map_err(|e| format!("判定数据库方言失败：{e}"))?;
    let mut tables: Vec<TableModel> = Vec::new();
    let mut declared_by = 0usize;
    for (plugin_id, root) in &roots {
        let path = root.join(SCHEMA_FILE_NAME);
        if !path.is_file() {
            continue;
        }
        let source = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("插件 `{plugin_id}` 的 {} 读取失败：{e}", path.display()))?;
        let parsed = ExternalSchema::parse(&source)
            .map_err(|e| format!("插件 `{plugin_id}` 的 {} 非法：{e}", path.display()))?;
        if parsed.tables.is_empty() {
            continue;
        }
        let converted = parsed
            .into_tables(dialect)
            .map_err(|e| format!("插件 `{plugin_id}` 的 {} 非法：{e}", path.display()))?;
        tables.extend(converted);
        declared_by += 1;
    }
    if tables.is_empty() {
        return Ok(0);
    }

    let out = cycle_with_external(db, &bootstrap_options(), &tables)
        .await
        .map_err(|e| format!("插件表收敛失败：{e}"))?;
    if let Some(refusal) = &out.refusal {
        return Err(format!("插件表收敛被安全闸拒绝：{refusal:?}"));
    }
    tracing::info!(
        plugins = declared_by,
        tables = tables.len(),
        executed = out.executed,
        "插件 schema.json 收敛完成"
    );
    Ok(declared_by)
}
