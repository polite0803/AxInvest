// SPDX-License-Identifier: AGPL-3.0-only

//! `gateway_message_queue` —— agent 间消息队列的持久化表。
//!
//! ## 为什么搬到这里（2026-09-16）
//! 本实体原定义在 `crates/runtime/src/persistent_queue.rs`。`dao/build.rs` 只扫
//! **本 crate** 的 `pub mod` 生成 `entity_modules!` ⇒ 定义放在 `runtime` 时，
//! **schema 引擎根本看不见它**（`entity_modules!` 是引擎 introspect / 孤儿判定的
//! 注册清单）。后果不是"报错"，而是引擎按"库里有表、注册表里没有"判为孤儿并 DROP。
//!
//! 全仓 185 个 `table_name` 注解中，仅此 1 处落在本 crate 之外 —— 已收敛。
//!
//! ## 真实消费面：零
//! 搬迁时全仓实测：`persistent_queue` / `PersistentQueue` 的引用**只有原模块声明本身**。
//! 原模块的文件注释自称「6 小时无人值守持久重试调度器，由 `init/` 的守护进程定时唤醒」
//! —— 与文件内容（仅本实体 + 一个 `Default` impl）**完全不符**，且 `init/` 下无任何引用。
//! 故搬入的是**表定义**，不是「接通了某个调度器」；调度器若需要，属新建功能。
//!
//! ## 建表契约
//! 表最初由已删的 `v200` 迁移建立（列清单与本实体逐字一致）。
//! 后续建表由 `Schema::create_table_from_entity` 依据本实体生成 ——
//! **不允许在别处再手写 `CREATE TABLE gateway_message_queue`**。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "gateway_message_queue")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub from_agent: String,
    /// 索引对齐迁移 `v200:377` 的 `idx_gateway_message_queue_to_agent`。
    /// 引擎按 `indexed` 生成的名字是 `idx-gateway_message_queue-to_agent`（连字符），
    /// 与迁移的下划线命名不同 —— 命名策略已裁决「采用引擎默认」，此处从之。
    #[sea_orm(indexed)]
    pub to_agent: String,
    pub payload_type: String,
    pub payload: String,
    /// 索引对齐迁移 `v200:375` 的 `idx_gateway_message_queue_status`。
    #[sea_orm(indexed)]
    pub status: String,
    #[sea_orm(default_value = "0")]
    pub retry_count: i32,
    pub max_retries: i32,
    pub created_at: i64,
    pub updated_at: i64,
    pub expires_at: Option<i64>,
    pub correlation_id: Option<String>,
    pub reply_to: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Default for Model {
    fn default() -> Self {
        Self {
            id: uuid_v4(),
            from_agent: String::new(),
            to_agent: String::new(),
            payload_type: "text".to_string(),
            payload: String::new(),
            status: "pending".to_string(),
            retry_count: 0,
            max_retries: 3,
            created_at: now_ts(),
            updated_at: now_ts(),
            expires_at: None,
            correlation_id: None,
            reply_to: None,
        }
    }
}

/// 当前时间（**秒**，非毫秒）。
///
/// ⚠ `created_at` / `updated_at` 列在该表存的是 **epoch 秒**，与本仓多数表的
/// 毫秒口径**不同**（同型混存已在别处登记）。改本函数前先核对迁移与既有数据。
fn now_ts() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
        as i64
}

/// 生成 32 位十六进制 id。
///
/// ⚠ 函数名沿用原模块的 `uuid_v4`，但产出**不是** RFC 4122 v4 格式
/// （无 `8-4-4-4-12` 连字符结构，而是「纳秒时间戳 << 64 | 随机 u64」的 `{:032x}`）。
/// 名不符实，此处如实标注；改名会改动 id 生成契约，故保留原名。
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let random: u128 = timestamp << 64 | (rand_u64() as u128);
    format!("{:032x}", random)
}

fn rand_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    RandomState::new().build_hasher().finish()
}
