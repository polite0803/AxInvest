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
    /// 当日成交额（元）。2026-10-20 新增：存档侧原先不落这一列 ⇒ 收盘快照路径
    /// （`pick_movers`）没法按流动性筛强势股，只能按涨幅取前 N。
    /// 存量行为 0（含义 = 当日未采集该字段，**不等于成交额为 0**）——读侧不得把 0
    /// 当成「没有流动性」。
    #[sea_orm(default_value = 0)]
    pub amount: f64,
    /// 成交量（**股**，解析层已把「手」×100 换算）。2026-11-01 阶段5「数据面补齐」新增：
    /// 腾讯批量行情同一行就带这一列（`StockQuote.volume`），采集侧此前只拿它推定成交额
    /// （`close × volume`），**从不单独落库** ⇒ 量能族（20 日量能变化）在库里没有历史。
    /// 0 = 当日未采集该字段（**不等于零成交**），判据与 [`Self::amount`] 一致。
    #[sea_orm(default_value = 0)]
    pub volume: f64,
    /// 换手率（%）。2026-11-01 新增。**只有当日采集通道有**：腾讯实时批量带
    /// （`vendors/tencent.rs:277` = `fields[ts_idx+8]`），而日K线回填通道恒 `None`
    /// （`vendors/tencent.rs:383`）⇒ **历史不可回填，只能从加列那天起攒纵深**。
    /// NULL = 未采集（不等于「换手为 0」），读侧按缺席处理。
    pub turnover_rate: Option<f64>,
    /// 总市值 / 流通市值（元）。同样只在当日 quote 有、历史不可回填 ⇒ NULL = 未采集。
    /// 用途：阶段5 重跑普查时的**规模分层/中性化**判据 —— 反转因子若只在小票里成立，
    /// 单因子全池打分就是拿一个「子层效应」冒充「全池效应」，必须能查出来。
    pub total_mv: Option<f64>,
    pub circulating_mv: Option<f64>,
    /// 市盈率（TTM）。2026-11-01 新增：腾讯批量行情同一行就带这一列（`StockQuote.pe`），
    /// 采集侧此前直接丢弃 ⇒ 全市场没有任何可按日落库的历史估值面（`financial_snapshots`
    /// 只在单票分析时写，现网最新一天 1 行、最好的一天 290 行，且**全是分析过的票**）。
    /// 荐股的价值桶要的是「全市场谁便宜」，用那张表等于「在我们看过的票里挑便宜的」= 自指。
    /// NULL = 当日未采集该字段（不等于「无估值」），读侧按缺席处理。
    pub pe_ttm: Option<f64>,
    /// 市净率。取值口径同 [`Self::pe_ttm`]，NULL 同义。
    pub pb: Option<f64>,
    /// 采集时刻（ms）—— 区分「哪天采的」与「哪天的数据」
    pub collected_at: i64,
    /// 来源：`tencent_batch` | `sina_batch` | `kline_backfill`
    pub source: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
