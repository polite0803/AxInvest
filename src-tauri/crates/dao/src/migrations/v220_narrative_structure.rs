// SPDX-License-Identifier: AGPL-3.0-only

//! v220 — 叙事结构表冗余保护（NO-OP，真实建表 = v126）
//!
//! ## ⚠️ 事实澄清（2026-09-03 死代码审计第 12 轮修正）
//!
//! 本 migration 自称「建表」，实为**冗余 no-op**：
//! - `narrative_structures` 表由 **v126** 首建，现行 schema 为 `structure`
//!   单列 JSON（消费方 `repo/narrative.rs` + `commands/narrative.rs`）。
//! - 本文件下方 DDL 是**旧三列 schema**（arcs/confluences/foreshadows）的
//!   残留草稿，依赖 v126 先建表 + `IF NOT EXISTS` 才不会生效。
//! - 正常执行顺序（v126 seq < v220）下本 DDL 永远 no-op。
//!
//! ## 保留原因
//!
//! 保留注册条目以维持 migration 序列连续（seq 220 不能留空洞），
//! 真正的 schema 权威是 v126。勿在此追加任何 DDL。
//!
//! ## 幂等性
//!
//! 使用 CREATE TABLE IF NOT EXISTS，重跑安全。

use sea_orm::{ConnectionTrait, DbErr};

pub async fn up(db: sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    let create_table = r#"
CREATE TABLE IF NOT EXISTS narrative_structures (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    genre TEXT NOT NULL DEFAULT 'novel',
    arcs TEXT NOT NULL DEFAULT '[]',
    confluences TEXT NOT NULL DEFAULT '[]',
    foreshadows TEXT NOT NULL DEFAULT '[]',
    is_template INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
)"#;

    let indices = [
        "CREATE INDEX IF NOT EXISTS idx_narrative_structures_is_template ON narrative_structures(is_template)",
        "CREATE INDEX IF NOT EXISTS idx_narrative_structures_genre ON narrative_structures(genre)",
    ];

    db.execute_unprepared(create_table).await?;
    for stmt in &indices {
        db.execute_unprepared(stmt).await?;
    }

    Ok(())
}
