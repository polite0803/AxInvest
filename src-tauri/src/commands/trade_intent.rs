// SPDX-License-Identifier: AGPL-3.0-only

//! 交易意图的调度侧保留面。
//!
//! 命令面 8 条（list_pending_trade_intents / list_trade_intents_by_stock /
//! approve_trade_intent / reject_trade_intent / link_trade_intent_to_trade /
//! record_analysis_trade_intent / expire_old_trade_intents / get_trade_intent_stats）
//! 已于 2026-11-13 整批退役（PLAN-shell-plus-plugins.md §160 候选 B）：
//! 前端全零调用、智能体工具面零消费，业务权威本就在
//! `axagent-analysis-engine` 的 `TradeIntentService`，不受本次退役影响。
//!
//! 本文件只剩两条「必须保留」物（§156「被 init/调度器消费的自由函数与常量
//! 不许随命令批删除」红线）：
//! - [`DEFAULT_INTENT_MAX_AGE_HOURS`] —— 被 `init/stock_automation_cron_seed.rs` 引用
//! - [`expire_intents_for_scheduler`] —— 被 `init/services.rs` 调度器直调

use axagent_analysis_engine::trade_intent::TradeIntentService;

/// pending 意图的默认存活时长（小时）—— 命令面缺省与定时任务共用同一个数。
///
/// 裁定 14：意图自动过期原先只挂在下线的 `risk_inspection` 链上 ⇒ 没人触发，
/// 过期态在现网从不产生。这里把它接到独立任务上；口径仍是接线前的 72 小时，
/// **默认行为逐位不变**（改这个数要同时改两侧，写两份就会分叉）。
///
/// ⚠ 本任务的 `task_type` 常量**不在这里**，权威在 `commands/constants.rs`
/// （`INTENT_EXPIRY_TASK_TYPE`）—— 定时任务的 task_type 一律归那里，业务模块只引用，
/// 就地再定义一份就会长出第二权威（本文件 2026-10-15 曾短暂两处并存，值还不同）。
pub const DEFAULT_INTENT_MAX_AGE_HOURS: i64 = 72;

/// 定时任务侧的过期入口（裁定 14）。
///
/// 原命令面 `expire_old_trade_intents` 已退役，本函数是这条口径的唯一存活入口；
/// 刻意**不接受** `max_age_hours` 参数：cron 任务的 prompt 里没有这条口径，
/// 传参只会造出「面板一个值、定时另一个值」。
pub async fn expire_intents_for_scheduler(
    db: &axagent_dao::DatabaseConnection,
) -> Result<u64, String> {
    TradeIntentService::expire_old_intents(db, DEFAULT_INTENT_MAX_AGE_HOURS).await
}
