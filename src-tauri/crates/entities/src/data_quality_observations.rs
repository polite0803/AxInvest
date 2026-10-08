//! #8 P5 数据质量熔断的**观测投影**（逐轮一行，不做唯一索引）
//!
//! 一行 = 一次分析结束时，`data-quality` 节点对该票**整体**证据面的判级快照。
//! 存在的唯一理由是把「连续多少轮 B 级以下」变成可读、可聚合、可归因的持久事实：
//! 熔断（路由 / 置信度上限 / 权重降权）不能只看当前这一轮，也不能去解析引擎自己的
//! `node_executions` 大 JSON 行（PG 里该列是 text、单行可达 2.6 MB，热路径上代价高且写法专有）。
//!
//! ## 它不是原始事实
//!
//! **派生自** `node_executions` / `stock_analyses.blackboard_snapshot` 里的 `result.data-quality`
//! 输出（写入口 `record_data_quality_observation` 从快照取数，不自己再判一次级）。
//! 因此它是**投影**：grade 的权威阈值只有一份，在 `data-quality.rhai` 的输出侧；
//! 本表若与快照不一致，以快照为准，说明写入口漂移 ⇒ 修写入口而不是改历史行。
//!
//! ## 为什么是追加式（照 `lesson_applications` 的形态）
//!
//! 「连续次数」是**读侧聚合**的结果，不是写侧状态：写侧只如实记每一轮，读侧按时间倒序
//! 数到第一个正常轮为止。好处有三：① 重跑/回补同一条分析不会把计数器「加错两次」以外
//! 的形状搞出来（幂等判据见 `analysis_id` 列）；② 历史可查（哪一轮开始变差的）；
//! ③ 不需要 (scope, horizon) 的唯一索引与 upsert 分支——那会把「一个 scope 一行」
//! 变成写侧状态，一旦并发跑两轮就互相覆盖。
//!
//! ## 档维度为什么先留 NULL
//!
//! 现网只有**全局** grade：`data-quality.rhai` 输出一个综合分档位，per-analyst 只有事实数
//! （其文档明文禁止派生 per-node 字母等级），而 v133 起逐档分析师实例在进本节点前就被
//! 折叠成「代表实例」（见 `seed_stock_analysis.rs` 的 data-quality 映射段）。
//! ⇒ 按档计数**当前没有数据源**。本列按 `NULL` 落，将来真要做按档质量线时补
//! 折叠展开 + 键名域扩档（牵动快速链 remap），届时只填值、不改表形状。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "data_quality_observations")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 计数归属的**作用域**。现阶段恒为 `"global"`（一轮分析一条），
    /// 预留它是为了让「按分析师维度计」这类扩展不改动历史行的解释方式。
    #[sea_orm(indexed)]
    pub scope: String,
    /// 档位（`ultra_short` / `short` / `mid` / `long`）。
    /// `NULL` = **本表引入时不存在按档 grade**（不是「四档同值」，也不是「第 0 档」）。
    pub horizon: Option<String>,
    /// `data-quality` 输出的字母等级原值（A–F）。`NULL` = 该轮该节点没跑成 ⇒ 无从判级。
    pub grade: Option<String>,
    /// 本轮是否算「异常」。**写侧一次定死**（`grade ∈ {B, C, D, F}` 或 grade 为 NULL），
    /// 读侧不再重算 —— 否则「异常」的定义一改，历史行的归属就跟着漂。
    pub abnormal: i32,
    /// 被观测的那次分析 id（→ `stock_analyses.id`）。同一条分析重跑会再落一行，
    /// 读侧按「同一 analysis_id 只算一次」去重，避免重跑把连续计数虚增。
    #[sea_orm(indexed)]
    pub analysis_id: String,
    /// 观测时刻（毫秒）。排序与窗口都按这一列，不用 `created_at` —— 补建历史时
    /// 真实观测时刻是 `observed_at`，写入时刻只是审计痕迹。
    #[sea_orm(indexed)]
    pub observed_at: i64,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
