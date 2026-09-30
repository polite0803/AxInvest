//! 全市场日收盘快照（窗口涨幅达标漏检核查的事件地基）
//!
//! 每行 = 「某票某交易日收盘」。窗口感知的累计涨幅**不在这里存**，由消费侧按
//! `default_holding_days` 复利连乘算出（`∏(1+r_i)−1`），故主路径不需要逐票 K 线。
//!
//! ⚠ 停牌/取不到值的票该日**不落行**，绝不写 `change_pct = 0.0` —— 写 0 等于把
//! 「拿不到」伪装成「没涨」，会直接污染事件表的分母与覆盖率。
//!
//! 数据源：腾讯批量行情 `qt.gtimg.cn/q=`（80 码/请求，实测全市场 74 请求零失败）。
//! 不用 DiskCache 承载本表：快照层是 LRU 10_000 条，历史上 K 线条目把快照挤掉过，
//! 造成「哪天有、哪天没」；窗口聚合必须是 SQL。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "market_daily_close")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// `"{stock_code}@{trade_date}"` —— 主键即天然去重，重复采集走 upsert 覆盖
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(indexed)]
    pub stock_code: String,
    /// 收盘时的股票名（报表直读，不再回查清单表；ST/更名随日变化，按当日值存）
    pub stock_name: String,
    /// 交易日 ISO 日期（`YYYY-MM-DD`），与 `daily_snapshot::snapshot_target_date` 同口径
    #[sea_orm(indexed)]
    pub trade_date: String,
    /// 收盘价
    pub close: f64,
    /// 昨收
    pub prev_close: f64,
    /// 当日涨跌幅（%），与东财 `clist` f3 互校过 |Δ|=0
    pub change_pct: f64,
    /// 采集时刻（ms）—— 区分「哪天采的」与「哪天的数据」
    pub collected_at: i64,
    /// 来源：`tencent_batch` | `sina_batch` | `kline_backfill`
    pub source: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
