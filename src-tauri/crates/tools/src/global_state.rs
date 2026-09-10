// SPDX-License-Identifier: AGPL-3.0-only

//! 全局状态访问（最小化）
//!
//! 仅保留数据库连接访问等跨模块必需的状态。
//!
//! Future: Migrate to ToolContext.extra to eliminate global state.
//! This requires updating all tool implementations to accept db via context,
//! which is a significant refactor tracked separately.

use sea_orm::DatabaseConnection;
use std::sync::Arc;
use std::sync::LazyLock;
// SAFETY: These RwLock instances wrap global database state that is only
// accessed from synchronous set/get helpers. No lock is ever held across an
// .await boundary, and all access goes through the module-level functions
// which are themselves synchronous. Using tokio::sync::RwLock would require
// async everywhere and break ~10+ callers in the tools crate.
use parking_lot::RwLock;

// ── 数据库路径 ────────────────────────────────────────────────────────────

static GLOBAL_DB_PATH: LazyLock<RwLock<Option<String>>> = LazyLock::new(|| RwLock::new(None));

pub fn set_db_path(path: &str) {
    let mut db_path = GLOBAL_DB_PATH.write();
    *db_path = Some(path.to_string());
}

pub fn get_db_path() -> Option<String> {
    GLOBAL_DB_PATH.read().clone()
}

// ── SeaORM 数据库连接 ─────────────────────────────────────────────────────

static GLOBAL_SEA_DB: LazyLock<RwLock<Option<Arc<DatabaseConnection>>>> =
    LazyLock::new(|| RwLock::new(None));

pub fn set_sea_db(db: Arc<DatabaseConnection>) {
    let mut sea_db = GLOBAL_SEA_DB.write();
    *sea_db = Some(db);
}

pub fn get_sea_db() -> Option<Arc<DatabaseConnection>> {
    GLOBAL_SEA_DB.read().clone()
}

// ── A 股行情客户端 ────────────────────────────────────────────────────────

static GLOBAL_ASTOCK_CLIENT: LazyLock<RwLock<Option<Arc<axagent_astock_data::AStockClient>>>> =
    LazyLock::new(|| RwLock::new(None));

/// [2026-09-03 接线恢复] finance.rs 的 api_tool! 宏（研报/概念板块/北向资金/龙虎榜/财联社快讯）
/// 依赖此客户端；由 init/state.rs 构造 AppState 时注入。
pub fn set_astock_client(client: Arc<axagent_astock_data::AStockClient>) {
    let mut guard = GLOBAL_ASTOCK_CLIENT.write();
    *guard = Some(client);
}

pub fn get_astock_client() -> Option<Arc<axagent_astock_data::AStockClient>> {
    GLOBAL_ASTOCK_CLIENT.read().clone()
}

// ── 需求精评 LLM 桥 ──────────────────────────────────────────────────────

static GLOBAL_DEMAND_LLM: LazyLock<
    RwLock<Option<Arc<dyn crate::tools::demand_llm::DemandLlmBridge>>>,
> = LazyLock::new(|| RwLock::new(None));

/// 注册需求精评 LLM 桥（由 init 层在 provider 配置就绪后注入；未注册 = 精评跳过）
pub fn set_demand_llm(bridge: Arc<dyn crate::tools::demand_llm::DemandLlmBridge>) {
    let mut guard = GLOBAL_DEMAND_LLM.write();
    *guard = Some(bridge);
}

pub fn get_demand_llm() -> Option<Arc<dyn crate::tools::demand_llm::DemandLlmBridge>> {
    GLOBAL_DEMAND_LLM.read().clone()
}
