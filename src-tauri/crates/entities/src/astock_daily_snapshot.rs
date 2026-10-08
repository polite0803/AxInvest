//! A 股「每日快照归档」历史表（#20② / B-3，2026-10-08 从 DiskCache 升格而来）
//!
//! 每行 = 「某方法 × 某作用域 × 某交易日」的一份当日快照原文（vendor 返回值的 JSON）。
//! 主键 `id = "{method}@{scope或'-'}@{date}"` —— 主键即天然去重，重复采集走 upsert 覆盖
//! （采集是幂等 tick，同一归属日可能被补跑多次）。`scope` 承载三种形态：
//! `NULL_MARKER`（`"-"`）= 全市场方法、股票代码 = 个股级方法、关键词 = `search_stock`
//! 这类带 keyword 维度的方法（三者共用一列，键空间与旧 DiskCache 的三种 key 逐一对齐）。
//!
//! ## 为什么不用 DiskCache 承载本表（这是本批的立表理由，不是性能优化）
//!
//! 1. **LRU 会吃掉历史**。旧归档是「独立 DiskCache 实例 + 单文件
//!    `astock_daily_snapshot.json`」，上限 10_000 条 + 64 MB 字节预算、超预算按
//!    `last_access` LRU 淘汰。而同一预算里 K 线条目单条约 70 KB，几条就能把最旧的快照
//!    挤掉 ⇒ 回放「哪天有、哪天没」，现场只看得到「没数据」，看不出是被淘汰吃掉的。
//!    归档要的是**只增不减**，缓存的淘汰语义与它是相反的。
//! 2. **结构上取不出跨日聚合**。DiskCache 只有点查 API，**没有 prefix-scan**
//!    ⇒「某方法跨多日的聚合」在旧存储里根本写不出来（`count_by_method_dates` 之类
//!    只能靠外部记日期）。本表给 `(method, snapshot_date)` 建了多列索引
//!    （声明在 `crates/dao/src/reconcile/extras.rs` 的 `MULTI_COL_INDEXES`），
//!    跨日窗口聚合就是一次 SQL `COUNT(DISTINCT snapshot_date)`。
//! 3. **全量重写单文件**。DiskCache 每次 flush 都是「全量序列化 → tmp → rename」，
//!    归档越大启动与落盘越慢，且崩溃窗口内的数据只能等下一次整写才生效。
//!
//! 同族的既有正门：`market_daily_close`（同一理由建的表，见其文件头注释）+
//! `analysis-engine/src/market_close_store.rs`。端口声明在 `astock-data`
//! （`daily_snapshot::DailySnapshotStore`），适配器在 wiring 层
//! （`src-tauri/src/init/daily_snapshot_store.rs`）—— `astock-data` 的依赖里
//! 没有 entities/dao/sea-orm，给它加数据库依赖会把 implementor 层拧成环（禁区 1），
//! 故照 `NewsArchiveSink` 的先例走「端口在下层声明、适配器在上层实现」。
//!
//! ⚠ `payload` 为**原文直存**，不做二次编码：读取侧按各方法的 DTO 反序列化，
//! 空快照（`"[]"` / `""` / `"null"`）在适配器出口由 `daily_snapshot::usable` 统一判掉，
//! **采集侧就写不进表**（旧 DiskCache 是「写进去、读的时候滤」，于是判重闸门
//! `has_daily_snapshot` 会被中毒条目锁死，该日永远修不好）。
//!
//! ⚠ `snapshot_date` 存的是**归一后的交易日**（读写两侧都先过
//! `daily_snapshot::key_date`，见该函数注释）⇒ 本表的日期键空间等于交易日，
//! 周末/假日锚点的回放能回填上一交易日那条。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 全市场快照的 `scope` 占位值（主键与列都用它，不用 NULL）。
///
/// 为什么用哨兵而不是 `Option<String>`：NULL 在 SQL 里既不等于 NULL 也不进
/// 多数索引统计，`contains` / 跨日聚合都得为它单独分支；哨兵让三种作用域
/// 共用同一条等值查询路径。
pub const NULL_SCOPE: &str = "-";

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "astock_daily_snapshot")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// `"{method}@{scope或'-'}@{date}"` —— 主键即天然去重，重复采集走 upsert 覆盖
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 方法名（与 `daily_snapshot::SNAPSHOT_METHODS` 逐字一致，是读取侧的白名单键）
    #[sea_orm(indexed)]
    pub method: String,
    /// 作用域：`NULL_SCOPE`（全市场）| 股票代码（个股级）| 关键词（`search_stock`）
    pub scope: String,
    /// 归属交易日（`YYYY-MM-DD`，已过 `key_date` 归一，与 `snapshot_target_date` 同口径）
    #[sea_orm(indexed)]
    pub snapshot_date: String,
    /// 快照原文（vendor 返回值序列化后的 JSON 串）
    pub payload: String,
    /// 采集时刻（ms）—— 区分「哪天采的」与「哪天的数据」，补跑时只有这里会变
    pub collected_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
