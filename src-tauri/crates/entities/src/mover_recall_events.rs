use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 妖股（窗口涨幅达标）漏检**事件**明细：一行 = 某票在某档窗口达标且被机械归因漏检。
///
/// 由 `mover-recall` 核查在落 `mover_recall_runs` 汇总时**全量替换**写入
/// （同 run_id 先删后插，cron 重跑幂等）；面板展示的截断 200 条只在 runs 表，
/// 本表存全集供补分析批次（P2）与漏检反思（P3）消费。
///
/// 状态机：
/// - `excluded`：`PoolDegraded` 层（数据可达性问题，不进补分析样本）；落库即判定
/// - `pending`：其余四层（not_in_pool / l3_scored_out / other_period / unexplained）
/// - `running`：被补分析批次认领
/// - `analyzed`：as-of 补分析成功，`analysis_id` 已回填
/// - `failed`：批次内重试一次后仍失败，`error_detail` 留因
/// - `reflected`：漏检反思完成（P3 回填 `reflection_id`）
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "mover_recall_events")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// = `mre-{uuid}`
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 所属核查轮次（mover_recall_runs.id）
    #[sea_orm(indexed)]
    pub run_id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 档位：ultra_short / short / mid / long
    pub period: String,
    /// 窗口右端（达标确认日，YYYY-MM-DD）
    pub anchor_date: String,
    /// 窗口起点（YYYY-MM-DD）：补分析的 as-of 锚点。
    /// 「窗口起步时系统该不该看出它」是漏检复盘的问题，故从起点而非右端重放。
    pub window_start: String,
    /// 窗口右端（= anchor_date，冗余落库便于读侧不回看 runs）
    pub window_end: String,
    /// 窗口长度（交易日）
    pub window_days: i32,
    /// 窗口累计涨幅（%）
    pub cum_gain_pct: f64,
    /// 窗口内最大单日涨幅（%）
    pub max_daily_pct: f64,
    /// 该档达标阈值（%）
    pub threshold_pct: f64,
    /// 板块归属（仅分组用）
    pub market_type: String,
    /// 本行归因所属**链**（reco | serenity），值域同 `mover_recall_runs.chain`；NULL = 旧行未分链。
    ///
    /// ⚠ 同一条达标事件在两条链上**各有一行**（两行的窗口数字相同、归因层可以不同）。
    /// 因此补分析的工作单元是「票 × 窗口右端日 × 档」三键而不是行 id：
    /// 认领 / 回写 / 盖章都必须按三键扩散到姊妹行，否则一只票被补分析两遍、另一行永远 pending。
    pub chain: Option<String>,
    /// 归因层：not_in_pool / pool_degraded / l3_scored_out / other_period / unexplained
    pub miss_layer: String,
    /// 窗口内是否有过任一档推荐（1=true 档位错配，0=false）
    pub picked_any_period: i32,
    /// 有截断留痕的风格键 JSON 数组（仅 l3_scored_out 非空）
    pub scored_out_styles_json: Option<String>,
    /// 补分析批次（mover_recall_analysis_batches.id），未被认领为 NULL
    #[sea_orm(indexed)]
    pub batch_id: Option<String>,
    /// 补分析产出的 stock_analyses 行 id
    #[sea_orm(indexed)]
    pub analysis_id: Option<String>,
    /// 漏检反思行 id（P3）
    pub reflection_id: Option<String>,
    /// pending / excluded / running / analyzed / failed / reflected
    #[sea_orm(indexed)]
    pub status: String,
    /// 失败原因（failed 时）
    pub error_detail: Option<String>,
    /// 回灌结论备注（`status='reflow_pending'` 时非空）：反思判定「本应入池」的理由。
    /// 与 `error_detail` 分开存：一个是失败原因，一个是对票的看法依据，混用两者都读不准。
    pub reflow_note: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
