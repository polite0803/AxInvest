use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 组合监控：每日刷新时写入一行组合快照。
///
/// 数据来源：刷新时（手动或 cron 触发）汇总 `trades` + 行情价，
/// 配合沪深 300 同期收益算 beta / max_dd / sharpe_30d。
///
/// ⚠ **单位口径**（2026-10-15 核对，别再按注释读）：`top_concentration_pct` 与
/// `total_pnl_pct`、`max_drawdown_pct` 存的都是**百分数**（60.0 = 60%），不是 0-1 比例；
/// 原注释写的 `(0-1)` 与代码不符，留着就是下一位读者按 0.6 去比 60 的起点。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "portfolio_metrics_daily")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 快照日期 (YYYY-MM-DD)
    #[sea_orm(indexed)]
    pub snapshot_date: String,
    /// 持仓总市值
    pub total_market_value: f64,
    /// 现金占比（百分数）。**当前无数据来源，写入侧恒 0** —— 见
    /// `PLAN-monitor-chain-closure.md` 裁定 8，拿到来源之前这一格不进面板。
    pub cash_pct: f64,
    /// 总未实现盈亏（元）
    pub total_pnl: f64,
    /// 总未实现盈亏百分比
    pub total_pnl_pct: f64,
    /// 自有持仓起的滚动 max drawdown（百分数，取最近 90 日净值序列）
    pub max_drawdown_pct: f64,
    /// 相对沪深 300 的 beta（None 表示样本不足）
    pub beta: Option<f64>,
    /// 30 日滚动 sharpe（年化）
    pub sharpe_30d: Option<f64>,
    /// 平均两两相关性 (0-1)
    pub correlation_avg: Option<f64>,
    /// 单股最大占比（百分数）
    pub top_concentration_pct: f64,
    /// 参与本次集中度/分散度计算的持仓只数（= **拿到现价** 的那几只）。
    ///
    /// 可空：本列之前落库的老行没有这个数，`None` 要读成「不知道」，不能读成「0 只」。
    /// 有了它，历史快照才可能被 `get_dashboard` 复原风险等级与分散度 —— 原先读取路径
    /// 把这两样硬编码成 `"—"` 与 `0`（PLAN 阶段 G 的 G-5）。
    pub position_count: Option<i32>,
    /// 该次快照里**拿不到现价**因而被排除在分母外的持仓只数（G-1）。
    ///
    /// 与 `position_count` 合起来才说明白「按几只算的」；老行同样为 `None`。
    pub unpriced_count: Option<i32>,
    /// 行业暴露 JSON: { "科技": 0.18, "消费": 0.12, ... }
    pub sector_exposure_json: String,
    /// 压测结果 JSON: { "m10": {...}, "m20": {...}, "blackSwan": {...} }
    pub stress_test_json: Option<String>,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
