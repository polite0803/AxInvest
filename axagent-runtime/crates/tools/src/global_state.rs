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

/// **入口统一脱敏**（脱敏口径唯一真源：`axagent_dao::db::redact_db_url`）。
///
/// 2026-10-31 事故：本函数此前原样存入连接串，PG 模式下是**含明文口令**的
/// `postgres://user:pw@host/db`。消费方拿它做诊断标识（如租约 holder_id）时，
/// 明文口令就随日志 / 数据库行外泄。在入口脱敏而不是要求每个消费方自己脱敏 ——
/// 「拿到 `get_db_path()` 就能安全使用」必须是无条件承诺，靠约定守不住。
///
/// 对 SQLite 场景是恒等变换：`sqlite:<路径>`、裸文件路径、`:memory:` 都没有
/// userinfo，脱敏原样返回 ⇒ 消费方（如 knowledge 工具打开 SQLite 文件）不受影响。
/// PG 连接串变成 `postgres://user:***@host/db`，仍保留 scheme 判别能力
/// （`bin/axagent-batch-rerun.rs:802` 等只判 `postgres://` 前缀的用法不受影响），
/// 但**不能再用于连接** —— 这正是刻意的：`db_path()` 的契约（harness
/// `persistence_mod.rs:25`）是「仅用于诊断 / 日志」，需要连接请走连接池。
pub fn set_db_path(path: &str) {
    let mut db_path = GLOBAL_DB_PATH.write();
    *db_path = Some(axagent_dao::db::redact_db_url(path));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 守住「入口脱敏」这条接线：全局状态里**永远不可能**出现明文口令。
    ///
    /// 不能只测 `redact_db_url` 本身（dao 侧已有 9 组输入守住它）—— 那证明不了
    /// 本函数真的调用了它。历史事故形态正是「函数是对的、调用点没接」。
    ///
    /// ⚠ 本测试会写入进程级全局状态。cargo 多线程并行下若其他测试并发读
    /// `get_db_path()`，拿到的是脱敏后的 PG 串 —— 与生产行为一致，无回滚必要；
    /// 但顺序敏感的测试不应依赖具体值（当前 tools crate 无此类测试）。
    #[test]
    fn set_db_path_redacts_password_at_ingress() {
        // PG 串：口令必须消失，host 与 scheme 判别必须保留
        set_db_path("postgres://svc:s3cret@db.internal:5432/axagent");
        let stored = get_db_path().expect("set 之后必须能读到");
        assert!(!stored.contains("s3cret"), "全局状态不得存明文口令：{stored}");
        assert_eq!(stored, "postgres://svc:***@db.internal:5432/axagent");
        assert!(stored.starts_with("postgres://"), "scheme 判别能力必须保留");

        // SQLite 串：恒等 —— 文件路径语义必须原样保留
        set_db_path("sqlite:/home/u/.axagent/axagent.db");
        assert_eq!(
            get_db_path().as_deref(),
            Some("sqlite:/home/u/.axagent/axagent.db"),
            "SQLite 路径不得被脱敏改动（消费方拿它打开文件）"
        );

        // 裸文件路径：恒等
        set_db_path("D:/data/axagent.db");
        assert_eq!(get_db_path().as_deref(), Some("D:/data/axagent.db"));
    }
}
