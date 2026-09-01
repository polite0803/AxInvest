// SPDX-License-Identifier: AGPL-3.0-only
//! v135: 创建 OPC 交付发票表 `opc_invoices`。
//!
//! ## Background
//!
//! 需求全链路审计（`output/full-demand-chain-audit-2026-09-01.md`）确认：
//! 「交付」环节零实现 —— browserMock 里的 `total_revenue` / `total_invoices`
//! 是无后端的幻影。won 线索没有任何后续承载，转化率无从统计。
//!
//! ## Key 语义
//!
//! - 一个 won 线索至多一张**有效**发票：`lead_id` 上建普通索引（不是唯一），
//!   因为作废后允许重开（P4 简化版不做 void 态，删除即允许重开，命令层幂等兜底）。
//! - `status` 状态机：`draft → sent → paid` 单向推进，`sent` 落 `issued_at`，
//!   `paid` 落 `paid_at`；同状态幂等。终态 `paid` 不可再变。
//! - `amount` REAL + `currency` TEXT：多币种并存，汇总按币种分组，不假装能换算。
//! - `linked_workflow_id`：P2 转化出的工作流，交付产物的溯源入口。
//!
//! ## Strategy
//!
//! `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS`，
//! SQLite 与 PostgreSQL 均支持，幂等可重入。

use sea_orm::ConnectionTrait;
use sea_orm::DbErr;

pub async fn up(db: sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS opc_invoices (\
         id TEXT NOT NULL PRIMARY KEY, \
         lead_id TEXT NOT NULL, \
         linked_workflow_id TEXT, \
         title TEXT NOT NULL, \
         amount REAL NOT NULL DEFAULT 0, \
         currency TEXT NOT NULL DEFAULT 'CNY', \
         status TEXT NOT NULL DEFAULT 'draft', \
         issued_at BIGINT, \
         paid_at BIGINT, \
         notes TEXT, \
         created_at BIGINT NOT NULL, \
         updated_at BIGINT NOT NULL)",
    )
    .await?;

    db.execute_unprepared(
        "CREATE INDEX IF NOT EXISTS idx_opc_invoices_lead ON opc_invoices (lead_id)",
    )
    .await?;

    db.execute_unprepared(
        "CREATE INDEX IF NOT EXISTS idx_opc_invoices_status ON opc_invoices (status)",
    )
    .await?;

    tracing::info!("[v135] Created opc_invoices table");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm::DbBackend;
    use sea_orm::Statement;

    #[tokio::test]
    async fn v135_creates_table_sqlite() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        up(db.clone()).await.unwrap();
        // 幂等：重复执行不报错
        up(db.clone()).await.unwrap();

        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM sqlite_master \
                 WHERE type='table' AND name='opc_invoices'"
                    .to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by::<i64, _>("n").unwrap(), 1);
    }

    #[tokio::test]
    async fn v135_indexes_exist() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        up(db.clone()).await.unwrap();

        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM sqlite_master WHERE type='index' \
                 AND name LIKE 'idx_opc_invoices%'"
                    .to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        let n: i64 = row.try_get_by("n").unwrap();
        assert_eq!(n, 2, "应有两个索引（lead_id / status）");
    }
}
