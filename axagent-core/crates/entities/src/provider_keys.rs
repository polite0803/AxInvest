// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "provider_keys")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub provider_id: String,
    pub key_encrypted: String,
    #[sea_orm(default_value = "")]
    pub key_prefix: String,
    #[sea_orm(default_value = 1)]
    pub enabled: i32,
    pub last_validated_at: Option<i64>,
    pub last_error: Option<String>,
    #[sea_orm(default_value = 0)]
    pub rotation_index: i32,
    pub created_at: i64,
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
