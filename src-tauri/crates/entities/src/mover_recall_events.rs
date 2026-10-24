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
    /// 回灌结论备注（反思判定「本应入池」时非空）：反思判定「本应入池」的理由。
    /// 与 `error_detail` 分开存：一个是失败原因，一个是对票的看法依据，混用两者都读不准。
    pub reflow_note: Option<String>,
    /// 是否列入**回灌清单**（D-1，2026-10-24）。
    ///
    /// 为什么单开布尔列而不复用 `status='reflow_pending'`：`status` 是补分析状态机
    /// （pending/running/analyzed/failed/reflected）的一个取值，而回灌是**正交**属性 ——
    /// 旧实现把状态改成 `reflow_pending` 后，`stamp_reflected` 又把它改回 `reflected`
    /// ⇒ `list_reflow_candidates` 恒空（现网 0 行实证）。布尔列与状态机互不覆盖。
    /// `NULL` = 本列引入前的行（等价 false）。
    pub reflow_pending: Option<bool>,
    /// 本行 `in_pool` 判定所用的候选池批次时刻（ISO 8601）。
    ///
    /// F-2（2026-10-24）：池证据优先取「窗口起点及之前最近一批」，窗口内首批只作备用。
    /// 记下实际用的那一批，才能事后复核「当时拿哪一池判的」—— 不记就等于只留结论。
    /// `NULL` = 本列引入前的行，或该事件没有任何池证据（`in_pool=None`）。
    pub pool_evidence_at: Option<String>,
    /// 补分析完成后回写的**事件档决策摘要**（JSON：`{action, confidence, horizon}`）。
    ///
    /// F-4（2026-10-24）：面板要能分「池没覆盖 / 池覆盖但决策观望 / 决策买入却被截断」——
    /// 后两者只能靠这一列，八层归因枚举不变。`NULL` = 尚未补分析或本列引入前的行。
    pub analysis_signal_at_start: Option<String>,
    /// 本事件所属窗口起点的 **as-of 荐股回放批次键**（= 那次回放的 `reco_picks.generated_at`）。
    ///
    /// `NULL` = 还没跑过 as-of 回放（或该事件属于 serenity 链行 —— 智选由 LLM 输出，
    /// 不做同款回放）。差异查询必须按这一列取证据，不能拿窗口内 live 推荐冒充。
    pub asof_reco_generated_at: Option<String>,
    /// as-of 回放得到的归因层（`MissLayer::as_str`，八层语义与 `miss_layer` 相同）。
    ///
    /// 与 `miss_layer`（live 层）**分列**：两者分歧本身就是诊断信息。读侧不得把
    /// `asof_layer` 当成 `miss_layer` 的替代 —— 反思用 as-of 层，面板两列并示。
    pub asof_layer: Option<String>,
    /// as-of 差异明细 JSON：池来源计数、命中的 style/档、被截断的 style/档、
    /// 停用风格、`degradations`、池判据（`inPool` / `poolSnapshot`）、以及
    /// 「本轮未含分析定档来源」的显式声明。形状见 `mover_recall_analysis` 模块文档。
    pub asof_diff_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
