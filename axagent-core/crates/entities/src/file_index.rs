// SPDX-License-Identifier: AGPL-3.0-only

//! `file_index` —— 代码文件元数据索引（**侧车库 `index.db`**）。
//!
//! ⚠ 本表不在主库，落在 `src/indexing_triggers.rs` 的 `INDEX_DB_FILENAME`
//! （`index.db`）所指的独立 SQLite 文件里。实体与连接所在库无关，故仍在此声明
//! schema 真相源；但**不要**拿它去主库找这张表。
//!
//! ⚠ 该库是**可重建的缓存**：删掉 `index.db` 后下次启动会重新扫描工作区。
//! 因此本表相关的 schema 变更（加列/加约束）不需要数据迁移，重建即可。
//!
//! ## 本文件即建表契约
//! 说明见 `cron_job.rs`。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "file_index")]
pub struct Model {
    /// 相对工作区根目录的路径
    #[sea_orm(primary_key, auto_increment = false)]
    pub path: String,
    /// 扩展名（不含点）
    pub extension: String,
    /// 字节数。用 `i64`：`INTEGER` 在 SQLite 下是 64 位，但若将来迁 PG，
    /// 大文件会超 int4。
    pub size_bytes: i64,
    /// 修改时间（epoch 秒）
    pub modified_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
