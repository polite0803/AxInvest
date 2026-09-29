use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// F1 借鉴：反思教训规则化表
///
/// 借鉴 TradingAgents 反思 → 规则提取机制。每次反思完成后，
/// 提取 lesson_summary 为可重用的规则，下次决策可以查询。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "reflection_lessons")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// ≤200 字符规则描述
    pub lesson_summary: String,
    /// 规则触发条件（如"分批建仓节奏 ≤3 天"）
    pub rule_pattern: Option<String>,
    /// 来源反思行 ID
    pub source_reflection_id: Option<String>,
    /// 适用 ticker（None=通用规则）
    #[sea_orm(indexed)]
    pub stock_code: Option<String>,
    /// 本教训**所属的持有周期档**：`ultra_short` / `short` / `mid` / `long`；
    /// `None` = 周期无关的通用规则（两侧都可注入）。
    ///
    /// 为什么必须由「本次复盘的那一档」盖章（〇-B v2 第 4 条 + PLAN 断点⑤）：
    /// 一次反思只复盘一个周期，而 lesson 此前**没有周期维度** ⇒ 超短线的教训
    /// （如「次日冲高回落就走」）会被原样注入长线分析的 prompt，跨周期污染决策。
    /// 值域 = `axagent_harness::holding_period::Period::as_str()`，不得另造字符串。
    #[sea_orm(indexed)]
    pub horizon: Option<String>,
    /// JSON 数组：适用场景标签（如 ["短线", "高估值"]）
    pub applicable_scenarios: Option<String>,
    /// 已应用次数
    pub times_applied: i32,
    /// 应用后成功次数
    pub success_count: i32,
    /// 规则置信度 0-1
    pub confidence: f64,
    /// active / deprecated
    #[sea_orm(indexed)]
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
