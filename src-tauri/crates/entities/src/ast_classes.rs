// SPDX-License-Identifier: AGPL-3.0-only

//! `ast_classes` —— 类 / 结构体定义（**侧车库 `index.db`，仅 SQLite**）。
//!
//! 背景与「一模块一实体」契约说明见 `ast_functions.rs`。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "ast_classes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub file_path: String,
    pub name: String,
    /// 起始行号（1-based）
    pub line_start: i32,
    /// 结束行号（1-based）
    pub line_end: i32,
    pub language: String,
    /// 父类名；无继承时为 NULL
    pub parent_class: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
