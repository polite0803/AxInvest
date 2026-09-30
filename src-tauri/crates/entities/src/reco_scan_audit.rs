//! 荐股扫描 L3 留痕 —— 「策略已算出该票、但被组内 top-N 截断」的行。
//!
//! ## 为什么只留这一层（不是全池逐票分数）
//!
//! `PLAN-mover-recall-attribution.md` Phase 2 的 L3 目标是判「入池但被评分压出 top-N」。
//! 全池逐票分数需要各策略在 `scan_one` 里报出**否决码**，而当前 6 个策略的
//! `scan_one` 一律返回 `Option<RecoPick>`：`None` 同时表示「不是本策略的候选」
//! （如价值策略看不上高 PE 的动量票）与「算了但分低」两种语义。若把 `None` 一律
//! 记成「被评分压出」，就会把**结构上不可能选它的风格**也罚进去 —— 那是把留痕
//! 缺口伪装成算法缺陷。
//!
//! 因此本表只落**无歧义的那一类**：策略确实返回了该票（即它认为这是候选）、
//! 只是在组内 top-N 排序里被截掉。截断发生在 `group_by_style_and_trim` 的
//! 单点，`rank` 即组内名次（1 = 第一个被截掉的）。
//!
//! ## 语义边界（消费方必须知道）
//!
//! - **有行** ⇒ 该 (风格, 档位, 票) 在本批被该风格评分后淘汰，可归因、可用于降权；
//! - **无行** ⇒ 只说明「没有这项留痕」，**不等于**该风格把票选出去了
//!   （它可能压根没把该票纳入评分）⇒ 归因侧落 `Unexplained`，不得据此降权。
//!
//! 与 `reco_picks` 同一次扫描用同一个 `generated_at`（批次键），便于按批对齐。

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "reco_scan_audit")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    /// 批次时刻（ISO 8601），与同批 `reco_picks.generated_at` 同值
    #[sea_orm(indexed)]
    pub generated_at: String,
    /// 周期: "ultra_short" | "short" | "mid" | "long"
    pub period: String,
    #[sea_orm(indexed)]
    pub stock_code: String,
    pub stock_name: String,
    /// 风格键（矩阵名目，如 "trend" / "serenity"）
    pub style: String,
    /// 该风格给出的原始置信度（0-100，截断前的值）
    pub confidence: i32,
    /// 组内名次（1 = 第一个被截断的）
    pub rank: i32,
    pub created_at: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
