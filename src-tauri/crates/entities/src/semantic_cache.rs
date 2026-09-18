// SPDX-License-Identifier: AGPL-3.0-only

//! 语义缓存表（DDL 由 v100 建表）。
//!
//! 一行 = 一条 (归一化 prompt 哈希, model) → LLM 响应的缓存。主键 `id`
//! （即 `prompt_hash`，见 `src/semantic_cache/mod.rs::store_by_hash` 的
//! `VALUES ($1, $2, ...)` 两参同值）。
//!
//! ⚠ **本表的 DDL 目前存在两份不同定义**，需收敛：
//! - v100 迁移（**已删**，原 `v100_consolidated.rs` 第 493 行）：本实体按此声明，
//!   即 `token_count`/`ttl_secs`/`hit_count` 为 `INTEGER`（PG = int4）且带
//!   `NOT NULL DEFAULT`。
//! - 运行时自建（`src/semantic_cache/mod.rs::create_table`）：PG 侧写成
//!   `BIGINT` 且**无 NOT NULL**。
//!
//! 二者只在「首次建表者是谁」上分野：应用启动总是先跑迁移 ⇒ 生产库实际
//! 形状 = v100 那一份；只有内存 SQLite 占位库（未跑迁移）会用到运行时那份。
//! 收敛方向见文档 `PLAN-declarative-schema-sync.md`（该表已登记为
//! 「一表两定义」待修项）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "semantic_cache")]
pub struct Model {
    /// 缓存键 = prompt 哈希（同时用作 `id` 与 `prompt_hash`）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 归一化 prompt 的 SHA-256
    #[sea_orm(indexed)]
    pub prompt_hash: String,
    /// 缓存的 LLM 响应正文
    pub response: String,
    /// 模型标识；NULL = 不限模型
    pub model_id: Option<String>,
    /// 该响应的 token 数
    #[sea_orm(default_value = 0)]
    pub token_count: i32,
    /// 任务类型（决定 TTL 档位：fact/trivial/reasoning/code/complex）
    #[sea_orm(default_value = "moderate")]
    pub task_type: String,
    /// 存活秒数（与 `created_at` 相加后与当前时间比较判过期）
    pub ttl_secs: i32,
    /// 写入时间戳（秒）
    #[sea_orm(indexed)]
    pub created_at: i64,
    /// 命中次数（`lookup_by_hash` 命中后自增）
    #[sea_orm(default_value = 0)]
    pub hit_count: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
