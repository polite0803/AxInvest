//! 全局状态访问（最小化）
//!
//! 仅保留数据库连接访问等跨模块必需的状态。
//! TODO: 后续通过 ToolContext.extra 传递，彻底消除全局状态。

use sea_orm::DatabaseConnection;
use std::sync::Arc;
use std::sync::LazyLock;
// SAFETY: These RwLock instances wrap global database state that is only
// accessed from synchronous set/get helpers. No lock is ever held across an
// .await boundary, and all access goes through the module-level functions
// which are themselves synchronous.
use std::sync::RwLock;

// ── 数据库路径 ────────────────────────────────────────────────────────────

static GLOBAL_DB_PATH: LazyLock<RwLock<Option<String>>> = LazyLock::new(|| RwLock::new(None));

pub fn set_db_path(path: &str) {
    let mut db_path = GLOBAL_DB_PATH.write().unwrap_or_else(|e| e.into_inner());
    *db_path = Some(path.to_string());
}

pub fn get_db_path() -> Option<String> {
    GLOBAL_DB_PATH
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

// ── SeaORM 数据库连接 ─────────────────────────────────────────────────────

static GLOBAL_SEA_DB: LazyLock<RwLock<Option<Arc<DatabaseConnection>>>> =
    LazyLock::new(|| RwLock::new(None));

pub fn set_sea_db(db: Arc<DatabaseConnection>) {
    let mut sea_db = GLOBAL_SEA_DB.write().unwrap_or_else(|e| e.into_inner());
    *sea_db = Some(db);
}

pub fn get_sea_db() -> Option<Arc<DatabaseConnection>> {
    GLOBAL_SEA_DB
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

// ── AStockClient ─────────────────────────────────────────────────────────

use axagent_astock_data::AStockClient;

static GLOBAL_ASTOCK_CLIENT: LazyLock<RwLock<Option<Arc<AStockClient>>>> =
    LazyLock::new(|| RwLock::new(None));

pub fn set_astock_client(client: Arc<AStockClient>) {
    let mut c = GLOBAL_ASTOCK_CLIENT
        .write()
        .unwrap_or_else(|e| e.into_inner());
    *c = Some(client);
}

pub fn get_astock_client() -> Option<Arc<AStockClient>> {
    GLOBAL_ASTOCK_CLIENT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
