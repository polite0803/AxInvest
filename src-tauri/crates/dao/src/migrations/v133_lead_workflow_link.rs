// SPDX-License-Identifier: AGPL-3.0-only
//! v133: 为 `opc_demand_leads` 添加实现链路两列 —— 转化工作流 ID + 执行时间。
//!
//! ## Background
//!
//! 需求全链路审计（`output/full-demand-chain-audit-2026-09-01.md`）确认：
//! 「发现 → 响应」之间无转化路径，线索只进不出。方案是把线索一键转化为
//! 可执行的工作流模板（复用 workflow_templates + WorkEngine 底座），
//! 需要在线索表上记录转化产物与执行时间。
//!
//! ## Key 语义
//!
//! - `linked_workflow_id`：`opc_convert_lead_to_workflow` 写入，指向
//!   `workflow_templates.id`；NULL = 未转化。转化不改变 `status`
//!   （status 语义归状态机命令 `opc_update_lead_status`）。
//! - `implemented_at`：`opc_run_lead_workflow` 启动执行后写入；
//!   NULL = 未执行过。
//!
//! ## Strategy
//!
//! PostgreSQL 用 `ADD COLUMN IF NOT EXISTS`；SQLite 无此语法，
//! ALTER 失败且报 duplicate column 时视为已完成（幂等可重入）。

use sea_orm::ConnectionTrait;
use sea_orm::DbBackend;
use sea_orm::DbErr;

pub async fn up(db: sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    let is_pg = db.get_database_backend() == DbBackend::Postgres;

    let alter = |sql: &'static str| db.execute_unprepared(sql);

    if is_pg {
        alter("ALTER TABLE opc_demand_leads ADD COLUMN IF NOT EXISTS linked_workflow_id TEXT")
            .await?;
        alter("ALTER TABLE opc_demand_leads ADD COLUMN IF NOT EXISTS implemented_at BIGINT")
            .await?;
    } else {
        // SQLite：重复执行时报 duplicate column name，视为已完成
        if let Err(e) =
            alter("ALTER TABLE opc_demand_leads ADD COLUMN linked_workflow_id TEXT").await
        {
            let msg = format!("{e}");
            if !msg.contains("duplicate column name") {
                return Err(e);
            }
            tracing::debug!("[v133] opc_demand_leads.linked_workflow_id 已存在，跳过");
        }
        if let Err(e) = alter("ALTER TABLE opc_demand_leads ADD COLUMN implemented_at BIGINT").await
        {
            let msg = format!("{e}");
            if !msg.contains("duplicate column name") {
                return Err(e);
            }
            tracing::debug!("[v133] opc_demand_leads.implemented_at 已存在，跳过");
        }
    }

    // 转化反查索引：按工作流模板 ID 找线索
    db.execute_unprepared(
        "CREATE INDEX IF NOT EXISTS idx_opc_demand_leads_workflow \
         ON opc_demand_leads (linked_workflow_id)",
    )
    .await?;

    tracing::info!("[v133] Added linked_workflow_id + implemented_at to opc_demand_leads");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm::Statement;

    #[tokio::test]
    async fn v133_adds_columns_sqlite() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        // 依赖 v131 建表
        crate::migrations::v131_opc_demand_discovery::up(db.clone()).await.unwrap();
        up(db.clone()).await.unwrap();

        let row = db
            .query_one_raw(Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT linked_workflow_id, implemented_at FROM opc_demand_leads LIMIT 1"
                    .to_string(),
            ))
            .await
            .unwrap();
        // 空表查询无行也说明两列存在（否则 SQL 报错）
        assert!(row.is_none());

        // 幂等
        up(db).await.unwrap();
    }
}
