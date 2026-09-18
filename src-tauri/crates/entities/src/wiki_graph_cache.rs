// SPDX-License-Identifier: AGPL-3.0-only

//! Wiki 图谱缓存表（DDL 由 v103 建表）。
//!
//! 一行 = 某个 wiki/vault 的图谱快照 + Louvain 社区检测结果。主键 `vault_id`。
//!
//! 动机：10 万节点规模下，`get_wiki_graph` 每次要做三次 DB 扫描 + 内存聚合，
//! 社区检测还要跑 Louvain，单次数秒到数十秒；缓存到本表后前端读取可命中缓存。
//! 失效策略：`notes` 表有写操作时调用 `invalidate_cache(vault_id)` 清缓存。
//!
//! 持久化入口为 [`axagent_dao::repo::wiki_graph_cache`]，历史实现用原生 SQL
//! 手写双方言分支，已改为走本实体。
//!
//! **列宽注意**：DDL 中 `computed_at`/`updated_at` 为 `BIGINT`（i64），
//! `node_count`/`edge_count` 为 `INTEGER`（PG = int4，i32）。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "wiki_graph_cache")]
pub struct Model {
    /// wiki / vault ID（一 wiki 一行）
    #[sea_orm(primary_key, auto_increment = false)]
    pub vault_id: String,
    /// 图谱数据（`GraphData` 的 JSON 序列化）
    pub graph_data_json: String,
    /// Louvain 社区检测结果 JSON；NULL = 尚未计算
    pub communities_json: Option<String>,
    /// 节点数（用于展示与新鲜度判断）
    #[sea_orm(default_value = 0)]
    pub node_count: i32,
    /// 边数
    #[sea_orm(default_value = 0)]
    pub edge_count: i32,
    /// 图谱数据计算时间戳
    pub computed_at: i64,
    /// 行更新时间戳
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
