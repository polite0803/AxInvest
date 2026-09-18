// SPDX-License-Identifier: AGPL-3.0-only

//! `l2_search_results` —— L2 搜索结果缓存（**侧车库，`disk-cache` 自持 SQLite 文件**）。
//!
//! 与 L1 内存缓存（TTL + 关停落盘）构成冷热分离：按 query hash 命中，带 TTL 与容量淘汰。
//!
//! ⚠ 不在主库：由 `disk-cache` crate 自己打开一个 SQLite 文件持有。
//! ⚠ **一模块一实体**是 `entities` crate 的硬契约（`dao/build.rs` 扫描全部 `pub mod`
//! 生成 `entity_modules!`，展开为 `axagent_entities::$module::Entity`）⇒ 不允许聚合模块。
//! 也因此，本表不参与主库 schema 自愈（`heal_all` 对主库里没有的表直接跳过）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "l2_search_results")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// 查询串的稳定 hash（`DiskCache::query_hash`）
    ///
    /// ⚠ `unique` 是**功能必需**，不是索引优化：`store_search_results` 用
    /// `INSERT ... ON CONFLICT (query_hash) DO UPDATE` 做真 upsert，而 ON CONFLICT
    /// 的冲突目标**必须**有唯一约束/唯一索引 —— 否则 SQLite 直接报
    /// 「ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint」。
    ///
    /// ⚠ 改动本行前先读 `disk-cache/src/lib.rs` 的 `store_search_results`：
    /// 该字段**无**唯一约束时，`INSERT OR REPLACE` 不会触发替换（那是本字段
    /// 最初被漏标时踩过的坑，见该方法的「演进史」）。
    #[sea_orm(unique)]
    pub query_hash: String,
    /// 原始查询文本（便于排查缓存内容）
    pub query_text: String,
    /// 序列化后的结果集
    #[sea_orm(column_type = "Text")]
    pub results_json: String,
    /// 结果条数
    #[sea_orm(default_value = 0)]
    pub result_count: i32,
    /// 命中次数（每次 get 自增）
    #[sea_orm(default_value = 1)]
    pub hit_count: i32,
    /// 写入时间（epoch 秒）
    #[sea_orm(default_value = 0)]
    pub created_at: i64,
    /// 最近访问时间（epoch 秒），淘汰排序键
    #[sea_orm(default_value = 0)]
    pub last_accessed_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
