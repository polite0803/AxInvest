//! 全市场股票清单（窗口涨幅达标漏检核查的枚举域）
//!
//! 权威来源是东财 `push2 clist` 分页（每页 100，实测 `total=5921`），但该接口连续
//! 爬取会触发连接级封禁（见 `AUDIT-mover-universe-feasibility-2026-09-30.md`），故采集侧
//! 必须**分 tick 摊薄 + 断点续传**。除 clist 之外的代码（自选股、已分析股、历史候选池）
//! 也会被增量补录进来，因此本表的语义是「已知代码的并集」，不是「交易所权威全表」——
//! 未做过一次完整 clist 刷新时，消费方必须显式声明枚举域不完整。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "market_stock_universe")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// 6 位股票代码（不带市场前缀），主键
    #[sea_orm(primary_key, auto_increment = false)]
    pub stock_code: String,
    pub stock_name: String,
    /// 市场归属，取值以 `harness::market_data::detect_market_type()` 为准（不另写一份前缀规则）。
    /// **只用于报告分组，不用于判事件**。⚠ 已知风险：北交所新代码段 `920xxx` 首字符是 `9`，
    /// 该函数把它归到 `b_share` 分支 ⇒ 分组数会偏，详见 PLAN §Phase 5 的登记项。
    pub market_type: String,
    /// 代码首次进入清单的时间（ms）
    pub first_seen_at: i64,
    /// 最近一次在 clist 全量刷新中被确认的时间（ms）；从未被确认时为 0
    pub last_confirmed_at: i64,
    /// 进入清单的来源：`clist` | `watchlist` | `analyses` | `seed_pool`
    pub origin: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
