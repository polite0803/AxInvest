// SPDX-License-Identifier: AGPL-3.0-only

//! 分析工作流**参数变更台账**（反思建议 → 模板变量）。
//!
//! ## 为什么必须有这张表
//!
//! `apply_param_suggestions` 过去只做两件事：改 `workflow_templates.variables`、
//! 触发策略权重重算。于是同一份配置有三处不可回答的问题：
//! - **改了什么**：旧值被覆盖，模板没有版本快照（`version` 不 bump）；
//! - **依据什么改**：没有列记录来源反思 id；
//! - **改得对不对**：没有观察窗口，也没有「到期保留 / 建议回滚」的判定载体。
//!
//! 这张表一行 = 一次参数变更（一次应用里的每个参数各一行），记录换代前后版本、
//! 旧/新值、来源反思、观察到期时刻，以及到期评估后的两代命中率。
//!
//! ## 状态机
//!
//! - `pending`：已应用，未到观察期；
//! - `insufficient`：已到期，但变更前/后任一侧的 completed 反思不足 `PARAM_EVAL_MIN_SAMPLES`；
//! - `kept`：到期且变更后命中率不低于变更前 ⇒ 保留；
//! - `rollback_suggested`：到期且变更后更低 ⇒ 建议回滚（**不自动回滚**，模板是共享配置）；
//! - `rolled_back`：人已执行回滚（`rollback_version` 记回滚那一代）。
//!
//! `status` 的值域只此一处；写侧不写其它字符串，读侧不得把未知值折成 `pending`。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "workflow_param_changes")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// = `wpc-{uuid}`
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 目标模板（当前只有 `stock-analysis`）
    pub template_id: String,
    /// 已解析的变量名（不是模型自报的别名）
    pub param: String,
    /// 变更前的模板版本（快照会以该版本落 `workflow_template_versions`）
    pub from_version: i32,
    /// 变更后的模板版本
    pub to_version: i32,
    /// 旧值（JSON 文本，保留数值/类型）
    pub old_value_json: String,
    /// 新值（JSON 文本）
    pub new_value_json: String,
    /// 来源反思行 id（模型建议时由前端带上；网格校准可为空）
    pub reflection_id: Option<String>,
    /// 变更理由（模型给的 reason 或人工备注）
    pub reason: Option<String>,
    /// pending / insufficient / kept / rollback_suggested / rolled_back
    #[sea_orm(indexed)]
    pub status: String,
    /// 观察到期时刻（ms）：到期后才做保留/回滚判定
    pub review_after_ms: i64,
    /// 变更**前**若干代的命中率（completed 反思中 correct / 三态样本）
    pub baseline_rate: Option<f64>,
    pub baseline_samples: Option<i32>,
    /// 变更**后**若干代的命中率
    pub after_rate: Option<f64>,
    pub after_samples: Option<i32>,
    /// 判定说明（样本不足时写明还差多少条；分歧时写明两代读数）
    pub verdict_note: Option<String>,
    /// 回滚落到的新版本号（仅 rolled_back 行）
    pub rollback_version: Option<i32>,
    pub applied_at_ms: i64,
    pub evaluated_at_ms: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
