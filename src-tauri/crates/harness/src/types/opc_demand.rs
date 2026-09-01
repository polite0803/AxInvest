// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现（Demand Discovery）DTO
//!
//! 三张概念实体：
//! - [`DemandPlatform`]：需求平台配置（连接器类型 / 启用状态 / 同步状态）
//! - [`DemandLeadDto`]：扫描并评估后的需求线索（含评分因子）
//! - [`DiscoverLeadsSummary`]：一轮「扫描 → 评估 → 持久化」的执行摘要
//!
//! 命令层契约见 `commands::opc_demand_discovery` / `commands::opc_demand_subscription`；
//! 数据落地见 `axagent_dao::repo::opc_demand`（v131 平台+线索 / v133 订阅）。

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
    /// 转化生成的实现工作流模板 ID（v132；NULL = 未转化）
    pub linked_workflow_id: Option<String>,
    /// 首次启动实现工作流执行的时间戳（秒；NULL = 未执行）
    pub implemented_at: Option<i64>,
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
    /// 高价值线索明细（最多 20 条，全局按分数降序，非本轮独有）
    pub leads: Vec<DemandLeadDto>,
    /// 本轮实际扫描评估到的线索明细（按分数降序）
    ///
    /// 与 `leads` 区别：`leads` 是全局高价值榜单（前端列表用），`round_leads`
    /// 只含本轮命中的线索 —— 订阅定时扫描按 `min_score` 过滤推送时必须有
    /// 本轮口径，否则每个订阅词都会推送到与自己无关的线索。
    #[serde(default)]
    pub round_leads: Vec<DemandLeadDto>,
}

/// 需求订阅（长期跟踪的关键词，v133）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemandSubscription {
    pub id: String,
    /// 订阅关键词（唯一）
    pub keyword: String,
    /// 是否启用扫描
    pub enabled: bool,
    /// 扫描间隔（小时）
    pub interval_hours: i32,
    /// 推送门槛：商业价值分低于此值不计入高价值命中
    pub min_score: f64,
    /// 限定平台 ID 列表；空数组 = 跟随全局启用的平台
    pub platforms: Vec<String>,
    /// 最近一次扫描时间戳（秒），NULL = 从未扫描
    pub last_scanned_at: Option<i64>,
    /// 最近一次扫描的高价值命中数
    pub last_hit_count: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 保存（新增或更新）需求订阅的输入
///
/// `id` 为空表示新增（自动生成 id）；`keyword` 唯一，重复会被 DB 唯一索引拒绝。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDemandSubscriptionInput {
    pub id: Option<String>,
    pub keyword: Option<String>,
    pub enabled: Option<bool>,
    pub interval_hours: Option<i32>,
    pub min_score: Option<f64>,
    pub platforms: Option<Vec<String>>,
}

/// 单个订阅词的扫描结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeywordScanOutcome {
    pub subscription_id: String,
    pub keyword: String,
    /// 扫描是否成功（平台全部失败时为 false）
    pub ok: bool,
    /// 失败原因（ok=false 时）
    pub error: Option<String>,
    /// 本词命中的高价值线索（已按 min_score 过滤）
    pub hits: Vec<DemandLeadDto>,
}

/// 一轮订阅扫描的汇总（定时任务与手动触发共用）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionScanSummary {
    /// 本轮扫描的订阅词数
    pub scanned_subscriptions: u32,
    /// 新入库线索总数
    pub total_saved: u32,
    /// 刷新评分的线索总数
    pub total_refreshed: u32,
    /// 命中推送门槛的高价值线索总数
    pub high_value_hits: u32,
    /// 逐词结果
    pub outcomes: Vec<KeywordScanOutcome>,
}

// ── 能力匹配（v133，需求全链路 P3）──────────────────────────────────

/// 单条能力的匹配命中项
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityMatchItem {
    pub capability_id: String,
    pub name: String,
    /// 能力类型（`CapabilityKind::as_str()`）
    pub kind: String,
    /// 业务域（`CapabilityDomain::as_str()`）
    pub domain: String,
    /// 综合检索分（semantic*0.6 + keyword*0.2 + tag*0.2，0.0-1.0）
    pub retrieval_score: f64,
    /// 一句话摘要（渐进式披露 L0，未声明时为 None）
    pub summary: Option<String>,
}

/// 线索的能力匹配结论
///
/// `verdict` 三档：`ready`（已有能力可直接接）/ `partial`（部分覆盖）/
/// `missing`（能力库基本没有对应能力）。
/// `missing_domains` 是需求类型推断出的必需能力域中未被覆盖的部分 —— 即「缺什么」。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadCapabilityMatch {
    pub lead_id: String,
    /// ready / partial / missing
    pub verdict: String,
    /// 最高检索分（无命中时为 0）
    pub best_score: f64,
    /// 命中的能力（按检索分降序）
    pub matches: Vec<CapabilityMatchItem>,
    /// 该需求类型要求的能力域（`CapabilityDomain::as_str()`）
    pub required_domains: Vec<String>,
    /// required_domains 中未被命中的部分 = 缺口
    pub missing_domains: Vec<String>,
    /// 缺口说明（供后续生成补齐工作流用）；无缺口时为 None
    pub gap_hint: Option<String>,
}

// ── 交付闭环（P4，v134） ──

/// 交付发票：won 线索的账本行
///
/// 状态机 `draft → sent → paid` 单向推进（DAO 层校验，同状态幂等）。
/// `amount` + `currency` 多币种并存，汇总按币种分组，不假装能换算。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryInvoiceDto {
    pub id: String,
    pub lead_id: String,
    /// P2 转化出的交付工作流（可空：人工交付无工作流）
    pub linked_workflow_id: Option<String>,
    pub title: String,
    pub amount: f64,
    pub currency: String,
    /// draft / sent / paid
    pub status: String,
    pub issued_at: Option<i64>,
    pub paid_at: Option<i64>,
    pub notes: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 开票入参：缺省时从线索元数据自动填充（标题/预算/币种）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CreateInvoiceFromLeadInput {
    /// 缺省 = 线索标题
    pub title: Option<String>,
    /// 缺省 = 线索预算上限（budget_max），无预算时 0
    pub amount: Option<f64>,
    /// 缺省 = 线索预算币种（budget_currency），无预算时 CNY
    pub currency: Option<String>,
    pub notes: Option<String>,
}

/// 单币种的回款小计
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevenueByCurrency {
    pub currency: String,
    /// 已回款（paid）总额
    pub paid_total: f64,
    /// 已开出（sent + paid）总额
    pub issued_total: f64,
}

/// 交付环节汇总（`opc_get_delivery_summary`）
///
/// `conversion_rate` = won / (全部线索 − lost)。
/// 只做统计暴露，不自动回写评分权重 —— 样本量不够时回写是过拟合。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliverySummary {
    /// won 线索数
    pub won_leads: u32,
    /// 非 lost 线索总数（分母）
    pub active_leads: u32,
    pub invoice_count: u32,
    pub paid_count: u32,
    pub revenues: Vec<RevenueByCurrency>,
    /// won / active，active 为 0 时为 0.0
    pub conversion_rate: f64,
}
