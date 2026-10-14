use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 复盘→进化：每条 strategy 在某只股票某段持仓后的实际表现。
///
/// 数据来源：复盘 cron 跑完后，对 stock_analyses 中 status=completed 且 outcome
/// 已经被 init/services.rs 写回 win/loss 的行，按 (strategy_id, period, stock_code) 拆分写入。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "strategy_performance")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// strategy 标识："trend" | "value" | "capital" | "reversion" | "watchlist"
    pub strategy_id: String,
    /// 持仓周期："short" | "mid" | "long"
    pub period: String,
    #[sea_orm(indexed)]
    pub stock_code: String,
    pub stock_name: String,
    /// 决策时间（ms）
    #[sea_orm(indexed)]
    pub decision_at: i64,
    /// 退出时间（ms）
    pub exit_at: i64,
    pub holding_days: i32,
    /// 实际收益百分比（已含手续费），如 -3.2 表示 -3.2%
    pub return_pct: f64,
    /// 决策是否正确：买入/增持类 → exit > entry；卖出/减持类 → exit < entry
    pub was_correct: i32,
    /// 决策时的 LLM 置信度（0-100）
    pub decision_confidence: i32,
    /// 各 horizon 的盈亏序列 JSON（1d/3d/5d/10d/20d）
    pub horizon_pnl_json: Option<String>,
    /// 公式 vs LLM 决策一致性分数（0-100，Phase 3 新增）
    pub agreement_score: Option<i32>,
    /// 本行所**复盘的那条分析**所属的算法代际（= `stock_analyses.template_version` 的建点副本，
    /// 与 `stock_reflections.template_version` 同源同值）。
    ///
    /// 为什么权重线也需要它：`compute_adjusted_weights` 的胜率就是「同一 (策略, 档) 的历史样本」，
    /// 跨代混池算出来的胜率是两套判据的加权平均（#31 / PLAN §五十一-②）。
    /// 读侧按 `HORIZON_BRANCH_GENERATION_FLOOR` **下限**筛（不是等号：等号每次换代都会清零窗口）。
    /// NULL = 代际未知（本列引入前的存量行、对话直执行通道）⇒ 不进分母。
    pub template_version: Option<i32>,
    /// 幂等来源键：标识本行由「哪条**被复盘决策**的哪个成熟档」物化而来，形如
    /// `reflection-horizon:{stock_analyses.id}:{horizon}`。
    ///
    /// 为什么键里是**决策 id 而不是反思行 id**：四周期模型「一行一档」——同一条决策会建
    /// 4 条独立 pending 反思行（ultra_short/short/mid/long 各一条），且每次反思都会当场
    /// 重算**全部四档**快照，已到期的更短档在后续反思行里同样判为 mature。若按「反思行 ×
    /// 档」去重，同一决策的 ultra_short 档会在 4 条反思行里各物化一行（胜率被一条决策反复
    /// 计票）。按「**决策 × 档**」upsert，无论由哪条反思行展开都收敛到同一行。
    /// 本列 + 部分唯一索引（`idx_strategy_performance_source_ref`，
    /// `WHERE source_ref IS NOT NULL`）兜底。
    ///
    /// NULL = 非反思物化通道写入的历史/其它来源行（如对话直执行回测），不参与唯一约束。
    pub source_ref: Option<String>,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
