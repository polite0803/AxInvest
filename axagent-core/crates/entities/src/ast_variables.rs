// SPDX-License-Identifier: AGPL-3.0-only

//! `ast_variables` —— 变量声明（**侧车库 `index.db`，仅 SQLite**）。
//!
//! 背景与「一模块一实体」契约说明见 `ast_functions.rs`。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "ast_variables")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub file_path: String,
    pub name: String,
    /// 类型标注；无标注时为 NULL
    pub type_annotation: Option<String>,
    /// 声明所在行号（1-based）
    pub line: i32,
    pub language: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
