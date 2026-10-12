use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 时间旅行批次复盘运行记录
///
/// 每次 `run_time_travel_batch` 调用对应一行。批次状态同时保留在进程内存
/// （`commands/time_travel_batch.rs` 的 BATCH_RUNS，供运行中轮询），落库行用于
/// 重启后查看历史批次；运行中进程异常退出遗留的 `running` 行会在启动时收敛为
/// `interrupted`（as_of 封闭世界栈是进程级的，不支持跨进程续跑）。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "time_travel_batch_runs")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// = batch_id
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 批次锚定历史交易日（YYYY-MM-DD）
    #[sea_orm(indexed)]
    pub as_date: String,
    /// 相对回看交易日数（fixed 模式为空）
    pub trading_days_ago: Option<i32>,
    /// running / completed / partial / failed / interrupted
    #[sea_orm(indexed)]
    pub status: String,
    /// analysis / reflection
    pub phase: String,
    /// 人话补充（如「当日荐股无候选」、interrupted 原因）
    pub message: Option<String>,
    pub total: i32,
    pub ok: i32,
    pub fail: i32,
    /// 智能荐股链候选行数（合并去重前）
    pub reco_picks: i32,
    /// 趋势智选链候选行数（合并去重前）
    pub serenity_picks: i32,
    /// 趋势智选链失败原因（best-effort 降级）
    pub serenity_error: Option<String>,
    /// 发起请求参数快照 JSON（TimeTravelBatchRequest）
    pub request_json: Option<String>,
    /// 逐只分析结果 JSON（Vec<PoolCandidateResult>）
    pub analyses_json: Option<String>,
    /// 四档汇总 JSON（Vec<HorizonSummary>）
    pub horizons_json: Option<String>,
    /// batch_reflection_inner 返回的原始 JSON 摘要
    pub reflection_summary: Option<String>,
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
    /// 创建时间戳（ms）
    pub created_at_ms: i64,
    /// 最近更新时间戳（ms）
    pub updated_at_ms: i64,
    /// 完成时间戳（ms，可空）
    pub finished_at_ms: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
