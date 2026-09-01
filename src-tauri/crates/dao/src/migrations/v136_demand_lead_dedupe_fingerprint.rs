// SPDX-License-Identifier: AGPL-3.0-only
//! v136: 需求线索去重键从 `(platform, source_url)` 迁移为内容指纹。
//!
//! ## Background
//!
//! 旧去重键 `(platform, source_url)` 唯一索引有两个致命缺陷（缺陷复审
//! `output/demand-discovery-defects-2026-09-02.md` P0-5）：
//! 1. 闲鱼等扫描器所有线索的 URL 指向**同一搜索页** → 一轮 100 条线索
//!    只入库 1 条，其余被判定为同轮重复跳过（去重语义从「同一条需求」
//!    退化成「同一个搜索页」）；
//! 2. 无内容指纹：同一需求换个 URL 重发、或跨平台重复出现无法合并，
//!    「需求热度」信号丢失。
//!
//! ## Key 语义
//!
//! - 新增 `content_fingerprint` 列：标题+描述归一化哈希（16 位 hex），
//!   由扫描器归一化层（`DemandLead::new_from_raw` → `scanner_common::
//!   content_fingerprint`）计算，内容为空时为 NULL。
//! - 删除旧唯一索引 `idx_opc_demand_leads_dedupe (platform, source_url)`，
//!   新建 `idx_opc_demand_leads_fingerprint (platform, content_fingerprint)`。
//!   PostgreSQL 唯一索引中 NULL 不参与约束，与旧语义一致（无指纹线索可重复插入）。
//! - 存量行指纹为 NULL，不参与新去重（可接受：老数据不回填，新数据自然生效）。
//! - `source_url` 退化为展示信息；DAO 层仍按指纹优先、URL 兜底做窗口刷新。

use sea_orm::ConnectionTrait;
use sea_orm::DbBackend;
use sea_orm::DbErr;

pub async fn up(db: sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    let is_pg = db.get_database_backend() == DbBackend::Postgres;

    let alter = |sql: &'static str| db.execute_unprepared(sql);

    if is_pg {
        alter("ALTER TABLE opc_demand_leads ADD COLUMN IF NOT EXISTS content_fingerprint TEXT")
            .await?;
    } else {
        // SQLite：重复执行时报 duplicate column name，视为已完成
        if let Err(e) =
            alter("ALTER TABLE opc_demand_leads ADD COLUMN content_fingerprint TEXT").await
        {
            let msg = format!("{e}");
            if !msg.contains("duplicate column name") {
                return Err(e);
            }
            tracing::debug!("[v136] opc_demand_leads.content_fingerprint 已存在，跳过");
        }
    }

    // 旧唯一索引（platform, source_url）会阻止同搜索页的多条线索入库，删除
    db.execute_unprepared("DROP INDEX IF EXISTS idx_opc_demand_leads_dedupe").await?;

    // 新唯一索引：内容指纹（NULL 不参与唯一约束）
    db.execute_unprepared(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_opc_demand_leads_fingerprint \
         ON opc_demand_leads (platform, content_fingerprint)",
    )
    .await?;

    tracing::info!(
        "[v136] opc_demand_leads: dedupe key (platform, source_url) -> (platform, content_fingerprint)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm::Statement;

    #[tokio::test]
    async fn v136_migrates_dedupe_index_sqlite() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        // 依赖 v132 建表 + v133 加列
        crate::migrations::v132_opc_demand_discovery::up(db.clone()).await.unwrap();
        crate::migrations::v133_lead_workflow_link::up(db.clone()).await.unwrap();
        up(db.clone()).await.unwrap();

        // 新指纹索引存在
        let row = db
            .query_one_raw(Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type='index' \
                 AND name='idx_opc_demand_leads_fingerprint'"
                    .to_string(),
            ))
            .await
            .unwrap();
        assert!(row.is_some(), "指纹唯一索引应存在");

        // 旧 URL 唯一索引已删除
        let row = db
            .query_one_raw(Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type='index' \
                 AND name='idx_opc_demand_leads_dedupe'"
                    .to_string(),
            ))
            .await
            .unwrap();
        assert!(row.is_none(), "旧 (platform, source_url) 唯一索引应已删除");

        // 同平台同搜索页 URL 的两条线索（不同指纹）应都能插入 —— 旧索引下第二条必撞约束
        let now = 1_700_000_000i64;
        for (title, fp) in [("需求A", Some("fp-a")), ("需求B", Some("fp-b"))] {
            let sql = format!(
                "INSERT INTO opc_demand_leads (id, platform, title, description, \
                 budget_currency, raw_snapshot, status, confidence, pain_score, \
                 market_gap_score, commercial_value_score, demand_type, \
                 source_url, content_fingerprint, created_at, updated_at) \
                 VALUES ('{title}', 'xianyu', '{title}', '', 'CNY', '{{}}', 'new', 0, 0, 0, 0, \
                 'unknown', 'https://s.goofish.com/search?q=x', '{}', {now}, {now})",
                fp.map(|f| f.to_string()).unwrap_or_default(),
            );
            db.execute_unprepared(&sql).await.unwrap();
        }
        let row = db
            .query_one_raw(Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM opc_demand_leads WHERE platform='xianyu'".to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by::<i64, _>("n").unwrap(), 2, "同 URL 不同指纹应各自成立");

        // 幂等
        up(db).await.unwrap();
    }
}
