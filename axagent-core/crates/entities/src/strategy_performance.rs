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
    ///
    /// ⚠ **计数域 = 主档那一对的五维分**（`decision.rs::compute_decision_agreement`，内部满分 90
    /// 归一化回 100）。逐档分数**不得**写进本列 —— 那是三维 / 满分 65 的另一把尺子，混进同一列
    /// 后 `get_agreement_score_history` 的历史线就成了两种口径的平均，自证失效。
    /// 逐档那把尺子住在 [`horizon_agreement`]。
    pub agreement_score: Option<i32>,
    /// B3（拍板 5，`PLAN-llm-horizon-decision-and-parity` §18）：**该档**公式 vs LLM 三维
    /// 归一分（65 制 → 100），产端 = `decision_json.agreementByHorizon[].total`。
    ///
    /// 与 `agreement_score` 是**两个计数域**，故分列而不是复用：同一档行内两套分数的分母不同，
    /// 并排或混池都会让人拿 63 与 46 比大小。
    /// NULL = 该档「无从比对」（LLM 逐档契约上线前的记录 / 该档单侧缺席），**不是 0 分**。
    pub horizon_agreement: Option<i32>,
    /// B3：同一窗口、同一判定函数（`deterministic_horizon_was_correct`）跑在 **LLM 该档动作**
    /// 上的结果 1/0 —— 与 `was_correct`（公式侧该档）同分母，才配得出一张 2×2。
    ///
    /// 为什么单列而不是拿 `was_correct` 反推：`was_correct` 判的是公式动作，反推等于假设
    /// 「LLM 与公式相反 ⇒ LLM 错」，那是把两个视角的结论焊在一起，样本会自己证明自己。
    /// NULL = 未成熟 / 该档 LLM 缺席 ⇒ 不进 2×2 的任何一格。
    pub llm_was_correct: Option<i32>,
    /// B3：LLM 该档动作原文（`agreementByHorizon[].llmAction`）。
    ///
    /// 统计列必须能回查到具体记录，否则 2×2 的每一格不可复核；本列只作溯源与呈现，
    /// **不参与**任何判定（判定走 `llm_was_correct`）。
    pub llm_action: Option<String>,
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
    /// **显式外键**：产出本行的那条反思行 id（`stock_reflections.id`）。
    ///
    /// 为什么不用 `source_ref` 承担：那一列的键是「决策 id × 档」且带部分唯一索引，
    /// 语义是幂等去重；Gap1 旧格（`strategy_id='reflection_verdict'`）刻意**不去重、
    /// 每次重跑追加**，塞进同一列会把重跑变成互相覆盖（两种意思挤一列，见实体上方
    /// `source_ref` 的同款论证）。
    ///
    /// 为什么必须有它（2026-10-14，PLAN-reflection-optimization-closure §二）：
    /// `build_hitrate_stats` 原先按 `(stock_code, created_at, 复盘档)` 匹配反思行，
    /// 而本列的 `created_at` 写的是**反思完成时刻**、反思行的 `created_at` 是**建点时刻**
    /// （pending 行由决策落盘时建）⇒ 两个时点必然不等，现网实测 8 行全部配不上，
    /// 逐档展开（`horizon_results_json`）从未生效、统计恒落 legacy 回退。
    ///
    /// NULL = 本列引入前的存量行 ⇒ 读侧允许退回旧时间戳键，但会计数并告警。
    pub reflection_id: Option<String>,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
