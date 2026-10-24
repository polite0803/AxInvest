// SPDX-License-Identifier: AGPL-3.0-only

//! 股票**执行层反思**报告（工作流质量），与市场结果反思（`stock_reflections`）分表。
//!
//! ## 为什么不塞进 `stock_reflections`
//!
//! 两者回答的问题不同：
//! - 本表：这次**工作流执行**的质量（节点瓶颈 / 失败分类 / 五维得分 / 改进建议）；
//! - `stock_reflections`：这次**预测 vs 之后实际走势**（verdict / alpha / 逐档命中）。
//!
//! 合成一张表会让两套分数并排成可比（历史上已经吃过 65 制 / 90 制混尺的亏）。
//! 两表用 `analysis_id` 关联，面板分栏展示，**互不换算**。
//!
//! `analysis_id` 为空的行只应来自旧调用方（没有分析行的纯工作流执行）；新写入必须带。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "stock_execution_reflections")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 关联的 `stock_analyses.id`（面板按此与市场结果反思并排）
    #[sea_orm(indexed)]
    pub analysis_id: Option<String>,
    pub stock_code: String,
    /// 工作流执行 id（`WorkflowExecutionRecord.execution_id`）
    pub execution_id: String,
    pub workflow_id: String,
    /// 总体质量分（1-10，执行层口径；**不是**预测胜率）
    pub overall_score: i32,
    /// 五维得分 JSON（signalAccuracy / riskAssessment / decisionQuality / analysisDepth / executionEfficiency）
    #[sea_orm(column_type = "Text")]
    pub dimension_scores_json: String,
    /// 瓶颈节点 JSON 数组
    #[sea_orm(column_type = "Text")]
    pub bottleneck_nodes_json: String,
    /// 错误模式 JSON 数组
    #[sea_orm(column_type = "Text")]
    pub error_patterns_json: String,
    /// 可复用模式 JSON 数组
    #[sea_orm(column_type = "Text")]
    pub reusable_patterns_json: String,
    /// 改进建议 JSON 数组
    #[sea_orm(column_type = "Text")]
    pub improvement_suggestions_json: String,
    /// 是否建议触发自我进化
    pub should_trigger_evolution: bool,
    pub evolution_trigger_reason: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
