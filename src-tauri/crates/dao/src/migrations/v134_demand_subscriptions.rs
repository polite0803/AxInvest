// SPDX-License-Identifier: AGPL-3.0-only
//! v134: 创建 OPC 需求订阅词表 `opc_demand_subscriptions`。
//!
//! ## Background
//!
//! 需求全链路审计（`output/full-demand-chain-audit-2026-09-01.md`）确认：
//! 「发现」环节纯靠手动输入单关键词点按钮，scheduler 0 接入 —— 与
//! 「一人公司自动化」定位矛盾。本表把「扫描意图」持久化，让定时任务
//! 能按间隔自动挑出到期订阅执行扫描。
//!
//! ## Key 语义
//!
//! - `keyword` 唯一：同一词重复订阅无意义，且会让扫描器重复打同一批平台。
//! - `interval_hours` + `last_scanned_at` 共同决定到期；`last_scanned_at`
//!   为 NULL 表示从未扫描，立即到期。
//! - `min_score` 是**推送门槛**，不是入库门槛：线索无论分数都会入库，
//!   但只有 ≥ min_score 的才计入高价值命中并触发 delivery 推送。
//! - `platforms_json` 空数组 = 跟随全局启用的平台；非空则只扫这些平台。
//!
//! ## Strategy
//!
//! `CREATE TABLE IF NOT EXISTS` + `CREATE UNIQUE INDEX IF NOT EXISTS`，
//! SQLite 与 PostgreSQL 均支持，幂等可重入。

use sea_orm::ConnectionTrait;
use sea_orm::DbErr;

pub async fn up(db: sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS opc_demand_subscriptions (\
         id TEXT NOT NULL PRIMARY KEY, \
         keyword TEXT NOT NULL, \
         enabled INTEGER NOT NULL DEFAULT 1, \
         interval_hours INTEGER NOT NULL DEFAULT 6, \
         min_score REAL NOT NULL DEFAULT 60, \
         platforms_json TEXT NOT NULL DEFAULT '[]', \
         last_scanned_at BIGINT, \
         last_hit_count INTEGER NOT NULL DEFAULT 0, \
         created_at BIGINT NOT NULL, \
         updated_at BIGINT NOT NULL)",
    )
    .await?;

    db.execute_unprepared(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_opc_demand_subs_keyword \
         ON opc_demand_subscriptions (keyword)",
    )
    .await?;

    db.execute_unprepared(
        "CREATE INDEX IF NOT EXISTS idx_opc_demand_subs_due \
         ON opc_demand_subscriptions (enabled, last_scanned_at)",
    )
    .await?;

    tracing::info!("[v134] Created opc_demand_subscriptions table");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm::DbBackend;
    use sea_orm::Statement;

    #[tokio::test]
    async fn v134_creates_table_sqlite() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        up(db.clone()).await.unwrap();

        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM opc_demand_subscriptions".to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by::<i64, _>("n").unwrap(), 0);

        // 幂等
        up(db).await.unwrap();
    }

    #[tokio::test]
    async fn v134_keyword_unique() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        up(db.clone()).await.unwrap();

        db.execute_unprepared(
            "INSERT INTO opc_demand_subscriptions \
             (id, keyword, created_at, updated_at) VALUES ('a', 'wiki', 1, 1)",
        )
        .await
        .unwrap();

        let dup = db
            .execute_unprepared(
                "INSERT INTO opc_demand_subscriptions \
                 (id, keyword, created_at, updated_at) VALUES ('b', 'wiki', 1, 1)",
            )
            .await;
        assert!(dup.is_err(), "同一关键词重复订阅应被唯一索引拒绝");

        // 不同关键词可正常插入
        db.execute_unprepared(
            "INSERT INTO opc_demand_subscriptions \
             (id, keyword, created_at, updated_at) VALUES ('c', 'crm', 1, 1)",
        )
        .await
        .unwrap();
    }
}
