use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 妖股漏检 → as-of 历史补分析的批次登记（P2）。
///
/// 与 `time_travel_batch_runs` 同族但**不共表**：时间旅行批次锚定**单个**历史交易日、
/// 含建池/反思两阶段；补分析批次的每个事件各有自己的 `window_start`（多 as-of 日）、
/// 不建池（候选是漏检事件本身）、P2 不做反思。逐票明细的权威是
/// `mover_recall_events.batch_id` 反查，本表只存批次级汇总。
///
/// 运行态同时保留在进程内存（`commands/mover_recall_analysis.rs` 的 BATCH_RUNS）；
/// 异常退出遗留的 `running` 行在新进程首次访问时收敛为 `interrupted`
/// （as_of 封闭世界栈是进程级的，不支持跨进程续跑）。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "mover_recall_analysis_batches")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// = batch_id（uuid）
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// running / completed / partial / failed / interrupted
    #[sea_orm(indexed)]
    pub status: String,
    /// 人话补充（如「窗口内无待补事件」）
    pub message: Option<String>,
    /// 本批认领事件数
    pub total: i32,
    /// 补分析成功数
    pub ok: i32,
    /// 重试一次后仍失败数
    pub fail: i32,
    /// 发起请求参数快照 JSON（RunMoverRecallAnalysisRequest）
    pub request_json: Option<String>,
    pub error_detail: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
