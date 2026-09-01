// SPDX-License-Identifier: AGPL-3.0-only
//! v131: 创建 OPC 需求发现两张表 —— 平台配置 + 需求线索持久化。
//!
//! ## Background
//!
//! 需求发现此前只有扫描器实现（`axagent_tools` 的 18 个平台扫描器），
//! 没有任何持久化与命令层——扫描结果进程退出即丢（审查报告 P0-5）。
//!
//! ## Key 语义
//!
//! - `opc_demand_platforms.id` 与内置扫描器 `platform()` 返回值一致，
//!   是 `add_platform` → `builtin_scanner_for` 路由的键。
//! - `opc_demand_leads` 去重键 = `(platform, source_url)` 唯一索引；
//!   SQLite 与 PostgreSQL 中 NULL 均不参与唯一约束，手动补录的无 URL
//!   线索可重复插入。
//!
//! ## Strategy
//!
//! `CREATE TABLE IF NOT EXISTS` —— 幂等，SQLite 与 PostgreSQL 均支持。

use sea_orm::ConnectionTrait;
use sea_orm::DbErr;

pub async fn up(db: sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS opc_demand_platforms (\
         id TEXT NOT NULL PRIMARY KEY, \
         name TEXT NOT NULL, \
         platform_type TEXT NOT NULL, \
         enabled INTEGER NOT NULL DEFAULT 1, \
         base_url TEXT, \
         config_json TEXT NOT NULL DEFAULT '{}', \
         last_sync_at BIGINT, \
         status TEXT NOT NULL DEFAULT 'idle', \
         created_at BIGINT NOT NULL, \
         updated_at BIGINT NOT NULL)",
    )
    .await?;

    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS opc_demand_leads (\
         id TEXT NOT NULL PRIMARY KEY, \
         platform TEXT NOT NULL, \
         title TEXT NOT NULL, \
         description TEXT NOT NULL, \
         budget_min REAL, \
         budget_max REAL, \
         budget_currency TEXT NOT NULL DEFAULT 'CNY', \
         contact_name TEXT, \
         contact_email TEXT, \
         contact_phone TEXT, \
         source_url TEXT, \
         raw_snapshot TEXT NOT NULL DEFAULT '{}', \
         status TEXT NOT NULL DEFAULT 'new', \
         confidence REAL NOT NULL DEFAULT 0, \
         pain_score REAL NOT NULL DEFAULT 0, \
         market_gap_score REAL NOT NULL DEFAULT 0, \
         commercial_value_score REAL NOT NULL DEFAULT 0, \
         demand_type TEXT NOT NULL DEFAULT 'unknown', \
         created_at BIGINT NOT NULL, \
         updated_at BIGINT NOT NULL)",
    )
    .await?;

    db.execute_unprepared(
        "CREATE INDEX IF NOT EXISTS idx_opc_demand_leads_platform \
         ON opc_demand_leads (platform)",
    )
    .await?;

    db.execute_unprepared(
        "CREATE INDEX IF NOT EXISTS idx_opc_demand_leads_score \
         ON opc_demand_leads (commercial_value_score)",
    )
    .await?;

    db.execute_unprepared(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_opc_demand_leads_dedupe \
         ON opc_demand_leads (platform, source_url)",
    )
    .await?;

    tracing::info!("[v131] Created opc_demand_platforms + opc_demand_leads tables");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm::DbBackend;
    use sea_orm::Statement;

    #[tokio::test]
    async fn v131_creates_tables_sqlite() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        up(db.clone()).await.unwrap();

        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM opc_demand_platforms".to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by::<i64, _>("n").unwrap(), 0);

        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM opc_demand_leads".to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by::<i64, _>("n").unwrap(), 0);

        // 幂等
        up(db).await.unwrap();
    }
}
