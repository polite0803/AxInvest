// SPDX-License-Identifier: AGPL-3.0-only

//! `cron_jobs` —— 定时任务持久化（整行 JSON 存储）。
//!
//! 表只有两列：`id`（任务 ID）+ `data`（`CronJob` 的 JSON）。
//! 之所以不做列展开：`CronJob` 模型演进频繁（`priority` / `delivery` /
//! `epoch_cost_estimate` / `enabled_toolsets` 都是后加的），整行 JSON 使新增
//! 字段无需配 migration。
//!
//! ## 本文件即建表契约
//! 运行时建表由 `Schema::create_table_from_entity` 依据本实体生成。
//! **不允许在别处再手写 `CREATE TABLE cron_jobs`** —— 那会让实体与实表各自漂移，
//! 而漂移不会报错，只会表现为「写入成功但读出来的字段是 None」。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "cron_jobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 序列化后的 `runtime-core::CronJob` JSON 字符串。
    ///
    /// 显式声明 `Text`：交给默认推导时 PG 侧可能落成有长度上限的类型，
    /// 而任务 JSON 会随 `delivery` / `enabled_toolsets` 持续增长。
    #[sea_orm(column_type = "Text")]
    pub data: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
