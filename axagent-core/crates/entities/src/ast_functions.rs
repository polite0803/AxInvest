// SPDX-License-Identifier: AGPL-3.0-only

//! `ast_functions` —— 函数/方法定义（**侧车库 `index.db`，仅 SQLite**）。
//!
//! AST 结构化代码索引的一部分，用于语义代码检索的 L2 阶段。
//!
//! ⚠ 不在主库：落在 `src/indexing_triggers.rs` 的 `INDEX_DB_FILENAME`（`index.db`）。
//! 该库是**可重建缓存**，删掉后下次启动重新扫描工作区 ⇒ schema 变更无需数据迁移。
//! 也正因如此，它不参与 `dao` 的主库 schema 自愈（`heal_all` 对「主库里没有的表」跳过）。
//!
//! ⚠ **一模块一实体**是 `entities` crate 的硬契约：`dao/build.rs` 扫描本 crate 的全部
//! `pub mod` 生成 `entity_modules!`，并展开为 `axagent_entities::$module::Entity`。
//! 因此**不允许**写聚合模块（如把 5 张 AST 表塞进一个 `ast_index` 模块）——
//! 那会让 `dao` 侧报 `cannot find type Entity in module ...`。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "ast_functions")]
pub struct Model {
    /// 内容派生的稳定 ID（非自增）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub file_path: String,
    pub name: String,
    pub signature: String,
    /// 起始行号（1-based）
    pub line_start: i32,
    /// 结束行号（1-based）
    pub line_end: i32,
    /// `pub` / `pub(crate)` / 空
    pub visibility: String,
    /// 语言标签（如 `rust` / `typescript`）
    pub language: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
