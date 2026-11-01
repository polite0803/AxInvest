// SPDX-License-Identifier: AGPL-3.0-only

//! `ast_call_edges` —— 调用关系边（**侧车库 `index.db`，仅 SQLite**）。
//!
//! 背景与「一模块一实体」契约说明见 `ast_functions.rs`。
//!
//! ## ⚠ 主键是**新增**约束，与存量库不一致
//! 原手写 DDL 的 `ast_call_edges` **没有任何主键**，而 SeaORM 实体在语法上
//! 必须有主键。这里取四列复合主键（`caller_file, caller_function, callee_name, line`），
//! 语义上就是「一条调用边的身份」—— 插入侧应配 `OnConflict::do_nothing()`，
//! 因为完全重复的边本身没有意义。
//!
//! ⚠ 存量库的表由旧 DDL 建成、**没有这个约束**，而 `CREATE TABLE IF NOT EXISTS`
//! 不会补约束 ⇒ 实体的主键声明对旧库不生效。因该库是**可重建缓存**，
//! 处置方式是删库重扫（无损），而不是 ALTER（SQLite 不支持加主键）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "ast_call_edges")]
pub struct Model {
    /// 调用方所在文件
    #[sea_orm(primary_key, auto_increment = false)]
    pub caller_file: String,
    /// 调用方函数名
    #[sea_orm(primary_key, auto_increment = false)]
    pub caller_function: String,
    /// 被调用的函数名
    #[sea_orm(primary_key, auto_increment = false)]
    pub callee_name: String,
    /// 调用发生行号（1-based）
    #[sea_orm(primary_key, auto_increment = false)]
    pub line: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
