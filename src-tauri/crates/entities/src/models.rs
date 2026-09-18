// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "models")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub provider_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub model_id: String,
    pub name: String,
    pub group_name: Option<String>,
    #[sea_orm(default_value = "chat")]
    pub model_type: String,
    #[sea_orm(default_value = "[]")]
    pub capabilities: String,
    pub max_tokens: Option<i64>,
    #[sea_orm(default_value = 1)]
    pub enabled: i32,
    pub param_overrides: Option<String>,
    // ⚠ 这里原先写 `#[sea_orm(default_value = "NULL")]`（2026-09-16 修正）。那是把
    // 「默认 NULL」误当成 SQL——`default_value` 收的是 `Into<Value>` 的**字面量**，
    // 字符串 `"NULL"` 渲染出来是 `DEFAULT 'NULL'`，而 PG 的 `double precision` 不接受
    // 这个输入（实测 `'NULL'::double precision` 报「无效的类型 double precision 输入
    // 语法」，`ALTER … SET DEFAULT 'NULL'` 直接 DDL 失败）。
    // `Option<f64>` 列的「默认 NULL」本来就是**不写 DEFAULT 子句**的语义 ⇒ 删掉属性即可。
    pub input_price_per_mtok: Option<f64>,
    pub output_price_per_mtok: Option<f64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::providers::Entity",
        from = "Column::ProviderId",
        to = "super::providers::Column::Id",
        on_delete = "Cascade"
    )]
    Provider,
}

impl Related<super::providers::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Provider.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
