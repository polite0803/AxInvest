// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现（Demand Discovery）DTO
//!
//! 三张概念实体：
//! - [`DemandPlatform`]：需求平台配置（连接器类型 / 启用状态 / 同步状态）
//! - [`DemandLeadDto`]：扫描并评估后的需求线索（含评分因子）
//! - [`DiscoverLeadsSummary`]：一轮「扫描 → 评估 → 持久化」的执行摘要
//!
//! 命令层契约见 `commands::opc_demand_discovery`；数据落地见
//! `axagent_dao::repo::opc_demand`（v131 migration）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 需求平台配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemandPlatform {
    /// 平台标识（与内置扫描器 `platform()` 返回值一致，如 "reddit"）
    pub id: String,
    /// 展示名
    pub name: String,
    /// 连接器类型：api / scanner / mock / manual
    pub platform_type: String,
    /// 是否启用
    pub enabled: bool,
    /// 平台基础 URL（api 类型可覆盖默认端点）
    pub base_url: Option<String>,
    /// 连接器扩展配置（描述、auto_sync 等）
    pub config: Value,
    /// 最近一次扫描成功时间戳（秒），NULL 表示从未扫描
    pub last_sync_at: Option<i64>,
    /// 连接器状态：idle / ok / error
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 保存（新增或更新）需求平台配置的输入
///
/// 全部字段可选：`id` 为空表示新增（自动生成 id），否则按 id 部分更新。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDemandPlatformInput {
    pub id: Option<String>,
    pub name: Option<String>,
    pub platform_type: Option<String>,
    pub enabled: Option<bool>,
    pub base_url: Option<String>,
    pub config: Option<Value>,
}

/// 需求线索（带评估结果）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemandLeadDto {
    pub id: String,
    /// 来源平台标识
    pub platform: String,
    pub title: String,
    pub description: String,
    pub budget_min: Option<f64>,
    pub budget_max: Option<f64>,
    pub budget_currency: String,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub source_url: Option<String>,
    /// 生命周期：new / evaluated / contacted / won / lost
    pub status: String,
    /// 评估置信度 0-1
    pub confidence: f64,
    /// 痛点强度 0-100
    pub pain_score: f64,
    /// 市场空白度 0-100
    pub market_gap_score: f64,
    /// 商业价值综合分 0-100（very_high ≥ 80）
    pub commercial_value_score: f64,
    /// 等级：low / medium / high / very_high
    pub opportunity_level: String,
    /// 需求类型（demand_type 小写标识）
    pub demand_type: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 一轮「扫描 → 评估 → 持久化」的执行摘要
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverLeadsSummary {
    /// 本轮扫到的原始线索总数
    pub total_scanned: u32,
    /// 完成评估的线索数
    pub total_evaluated: u32,
    /// 新入库数（去重后跳过的不计）
    pub total_saved: u32,
    /// 命中去重窗口外的同源线索、被刷新评分的条数
    pub total_refreshed: u32,
    /// 其中高价值（commercial_value_score ≥ 60）数量
    pub high_value_count: u32,
    /// 高价值线索明细（最多 20 条）
    pub leads: Vec<DemandLeadDto>,
}
