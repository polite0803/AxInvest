// SPDX-License-Identifier: AGPL-3.0-only

//! 面板变量表快照的 **wiring 装入器**（2026-10-08 A 批接线）。
//!
//! `axagent_harness::panel_variables` 是端口（进程内快照，零 `axagent-*` 依赖），
//! 本文件是它在 wiring 层的唯一装入口：这里同时看得见 `entities` 与 `sea-orm`，
//! 而下层的落点（`astock-data` 评分与取数、`analysis-engine` 仓位限制）都看不见 DB。
//! 同形先例：`init/daily_snapshot_store.rs`（`astock-data` 端口的 wiring 实现）。
//!
//! ## 三个装卸点，缺一个就有「面板改了没人读」的窗口
//!
//! 1. **启动**（`start_background_services`，且必须早于 `start_realtime_monitor`）
//!    —— 监控轮询间隔在启动装配时读，装晚了这一轮就用默认值；
//! 2. **模板保存后**（`commands::workflow_template::update_workflow_template`）
//!    —— 设置面板写的就是这一行；不刷就是「要重启才生效」；
//! 3. **种子化后**（`commands::stock_analysis_setup::ensure_stock_analysis_experts_seeded`）
//!    —— 版本门放行重建时，`merge_variable_values` 与 `force_variable_value` 会改这张表
//!    （典型如 `kline_limit` 的一次性覆写）。只在启动装一次 ⇒ 升版那一轮读的还是旧表。
//!
//! ⚠ 失败口径统一是 **warn + 保持原快照**，绝不 panic、绝不阻断启动/保存/种子：
//!   读侧对每个键都有回落默认值，那份默认值就是接线前的现值 ⇒ 装不上最多退化成
//!   「这次还是按默认跑」，而阻断会让模板卡在旧版本或让面板保存失败（代价更大）。

// 只 `EntityTrait`，不带 `QuerySelect`：本文件做的是主键等值查询（`find_by_id(..).one(db)`），
// 没有列选择，全量 `cargo clippy --workspace --all-targets --all-features -- -D warnings`
// 会把多 import 的那个 trait 报成 `unused_imports`（首个 error 即短路，所以这道门必须真跑完）。
// 同形先例：`commands/memory.rs` 的 `use sea_orm::EntityTrait;`。
use sea_orm::EntityTrait;

use axagent_entities::workflow_template;
use axagent_harness::panel_variables;

/// 变量表的权威模板 id —— 与设置面板的 `TEMPLATE_ID`（`StockAnalysisConfigPanel.tsx`）
/// 及种子侧 `seed_stock_analysis::SOURCE_TEMPLATE_ID` 同名，不是第四份抄写。
const STOCK_ANALYSIS_TEMPLATE_ID: &str = "stock-analysis";

/// 从 DB 读 `stock-analysis` 的 `variables` 并整表装入进程内快照，返回装入条数。
///
/// 整表**替换**而非增量合并（语义见 `panel_variables::install_panel_variables`）：
/// 退役一个键必须让快照里的它一起消失，否则读侧仍在读旧值。
pub async fn refresh_from_db(db: &sea_orm::DatabaseConnection) -> usize {
    match workflow_template::Entity::find_by_id(STOCK_ANALYSIS_TEMPLATE_ID).one(db).await {
        Ok(Some(row)) => {
            let raw = row.variables.unwrap_or_default();
            let len = panel_variables::install_panel_variables_from_str(&raw);
            tracing::info!(
                "[panel_variables] 变量表快照已装入：{len} 条（模板 {STOCK_ANALYSIS_TEMPLATE_ID}）"
            );
            len
        },
        // 模板还没播种（首启空库 / 刚被删）⇒ 空表，读侧全部回落默认值。
        Ok(None) => {
            tracing::warn!(
                "[panel_variables] 模板 {STOCK_ANALYSIS_TEMPLATE_ID} 尚不存在 ⇒ 参数按 Rust 默认值运行"
            );
            0
        },
        Err(e) => {
            tracing::warn!("[panel_variables] 变量表读取失败（{e}）⇒ 保持上一次快照 / 回落默认值");
            0
        },
    }
}

/// 快照里读一个整数参数（`None` ⇒ 调用方用自己的默认值）。
///
/// 装配点走这里而不是在 `init` 里再手写一遍 `serde_json` 取值路径：
/// 「怎么才算一个可用数值」的判据必须只有一份（在 `panel_variables` 里）。
pub fn integer(name: &str) -> Option<i64> {
    panel_variables::integer(name)
}

/// 当前快照条数（启动日志的自证读数：0 表示变量表没装进来，参数全按默认跑）。
pub fn len() -> usize {
    panel_variables::panel_variables_len()
}
