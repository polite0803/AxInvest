use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 妖股（窗口涨幅达标）漏检核查的逐轮快照
///
/// 每次 `analyze_mover_recall_inner` 成功执行对应一行（手动面板调用不落库，
/// 仅 `mover-recall` 定时任务与任务中心「立即执行」落库）。P1 只存汇总与
/// 截断后的漏检明细；逐事件规范化（`mover_recall_events` 表、as-of 补分析、
/// 漏检反思关联）在 P2 落地。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "mover_recall_runs")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// = `mrr-{uuid}`
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 核查覆盖起点（YYYY-MM-DD，= 首个有收盘快照的交易日）
    pub date_from: String,
    /// 核查覆盖终点（YYYY-MM-DD）
    pub date_to: String,
    /// 数据起点声明：早于此日没有快照
    pub data_since: String,
    /// 区间内实际采集的交易日数（只数到 date_to）
    pub collected_days: i32,
    pub universe_size: i64,
    pub universe_confirmed: i64,
    /// 行情源申报的全市场总数（`reported_total`）—— 面板的全 A 股分母。
    /// NULL = 该列引入前的旧轮次（面板显示「未记录」，不拿 universe_size 冒充）。
    pub reported_total: Option<i64>,
    /// 四档判据（Vec<TierRule> JSON）
    pub rules_json: String,
    /// 召回率（RecallRates JSON）
    pub rates_json: String,
    /// 分层计数（Vec<LayerRow> JSON）
    pub layers_json: String,
    /// 达标事件总数
    pub events_total: i32,
    /// 漏检事件数（counts_as_miss 口径）
    pub miss_count: i32,
    /// 漏检明细 JSON（Vec<MissRow>，后端已按累计涨幅降序截断 200 条）
    pub misses_json: Option<String>,
    /// 逐档达标事件数 JSON（`{"ultra_short":3,...}`）。
    ///
    /// 为什么单存一列：只有它能把「本档已满但零达标」与「本档有达标」区分开 ——
    /// `layers_json` 是跨档聚合、`misses_json` 只有漏检行，两者都答不了这个问题。
    /// NULL = 本行早于该列（面板必须成句声明「本轮未记录逐档计数」，不得显示成 0）。
    pub events_by_tier_json: Option<String>,
    /// 有收盘行（至少一行）的票数 —— 事件实际可来自的票数。
    ///
    /// 与 `universe_size`（清单规模）是两个域：清单里没采到收盘的票不会产生任何事件。
    /// NULL = 本行早于该列（面板显示「未记录」，不得拿清单数冒充）。
    pub codes_with_close: Option<i32>,
    /// 逐档「该票历史长度 ≥ 该档窗口天数」的可判票数 JSON。
    /// NULL = 本行早于该列（面板显示「未记录」）。
    pub judgeable_codes_by_tier_json: Option<String>,
    /// 归因层口径版本（如 `mover-layers-v2`）。
    /// NULL = 本行早于该列；跨版本轮次的漏检数不可直接比较（面板需标注）。
    pub gate_version: Option<String>,
    /// 执行状态："success"（核查本身成功，不代表零漏检）
    pub status: String,
    /// 触发来源：调度器执行（含定时到点与任务中心「立即执行」，两者共用同一 dispatch）
    pub trigger_source: String,
    /// 本轮归因针对的**链**：`reco` = 智能荐股，`serenity` = 趋势智选。
    /// 值域唯一权威在 `analysis_engine::mover_recall::RecoChain`（面板与门都读它，不各抄一份）。
    ///
    /// 2026-10-20 A-2 起，一次核查**每链落一行**（同刻两行）：混池读数会把「智选推荐过」
    /// 算成荐股也命中，反之亦然 ⇒ 分不出该由哪条链去优化。
    /// NULL = 本行早于分链改造 ⇒ 那是**混池口径**，读侧必须单独成句，不得冒充某一链的读数。
    pub chain: Option<String>,
    /// 执行时间戳（ms）
    pub created_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
