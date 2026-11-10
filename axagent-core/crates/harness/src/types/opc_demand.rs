// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现（Demand Discovery）DTO
//!
//! 三张概念实体：
//! - [`DemandPlatform`]：需求平台配置（连接器类型 / 启用状态 / 同步状态）
//! - [`DemandLeadDto`]：扫描并评估后的需求线索（含评分因子）
//! - [`DiscoverLeadsSummary`]：一轮「扫描 → 评估 → 持久化」的执行摘要
//!
//! 命令层契约见插件命令 `opc_*`（`plugin-opc`）与 `commands::opc_demand_subscription`；
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
    /// 连接器状态：idle / ok / error / skipped（合规跳过）
    pub status: String,
    /// 最近一次扫描失败原因；NULL = 成功 / 从未扫描 / 合规跳过
    pub last_error: Option<String>,
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
    /// 评估且有产出的线索数（新入库或刷新；窗口内重复跳过的不计）
    pub total_evaluated: u32,
    /// 新入库数（去重后跳过的不计）
    pub total_saved: u32,
    /// 命中去重窗口外的同源线索、被刷新评分的条数
    pub total_refreshed: u32,
    /// 其中高价值（commercial_value_score ≥ 60）数量
    pub high_value_count: u32,
    /// 本轮高价值线索明细（最多 20 条，按分数降序）
    pub leads: Vec<DemandLeadDto>,
    /// 本轮实际入库/刷新的线索明细（按分数降序）
    ///
    /// 与 `leads` 区别：`leads` 是 `round_leads` 中 ≥60 分的前 20 条摘要，
    /// `round_leads` 是完整轮次明细 —— 订阅定时扫描按 `min_score` 过滤推送
    /// 时必须有本轮口径，否则每个订阅词都会推送到与自己无关的线索。
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

/// 手动补录一条需求线索的输入（P1-4）
///
/// 复用扫描管线的评分/去重/联系方式提取逻辑；`platform` 固定为 `manual`。
/// `title` / `description` 必填；预算字段若填写则覆盖自动提取结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDemandLeadInput {
    pub title: String,
    pub description: String,
    pub budget_min: Option<f64>,
    pub budget_max: Option<f64>,
    pub budget_currency: Option<String>,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub source_url: Option<String>,
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

/// 扫描策略在通用设置表中的键名
pub const SCAN_POLICY_SETTING_KEY: &str = "opc.demand.scanPolicy";

/// 需求发现扫描策略
///
/// 所有字段都有默认值且会做范围钳制，避免脏配置把扫描管线上打挂
/// （如 `concurrency = 0` 导致永不执行、或 `retry_max` 过大导致长尾阻塞）。
///
/// 2026-11-11 自 `axagent-tools` 上收：`opc.store` 接缝的 `get_scan_policy` /
/// `save_scan_policy` 两个 op 要把该类型跨进程传给插件 worker，而 harness 不能
/// 反向依赖实现层。`axagent_tools::tools::scan_policy` 仍以 `pub use` 转出同名类型，
/// 既有调用方无需改动。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScanPolicy {
    /// 并发扫描的平台数上限（>= 1）
    pub concurrency: usize,
    /// 全局速率上限（次/分钟）；0 表示不限速
    pub rate_limit_per_min: u32,
    /// 单个平台失败后的最大重试次数（不含首次尝试）
    pub retry_max: u32,
    /// 重试退避基数（毫秒）；第 n 次重试等待 `base * 2^(n-1)`
    pub retry_backoff_ms: u64,
    /// 单平台单次请求超时（秒）
    pub timeout_secs: u64,
    /// 去重时间窗口（小时）
    ///
    /// 命中 `(platform, source_url)` 的既有线索：
    /// - 在窗口内 → 跳过（判定为同一轮需求的重复曝光）
    /// - 超出窗口 → **刷新**该行（更新评分与 `updated_at`），而不是插入新行
    ///   —— 去重唯一索引是 `(platform, source_url)`，插入必然冲突
    pub dedup_window_hours: u32,
    /// 单次扫描保留的线索数上限（防御扫描器异常返回海量数据）
    pub max_leads_per_scan: usize,
    /// 是否启用 LLM 精评（规则评分入库后，对通过预筛的候选做一轮批量 LLM 重新打分）
    pub llm_eval_enabled: bool,
    /// 单轮送入 LLM 精评的候选数上限（控制 token 成本；按规则分降序截断）
    pub llm_eval_max_leads: usize,
    /// LLM 精评预筛阈值：规则商业价值分 ≥ 此值的候选才送评（0 = 全送）
    pub llm_eval_min_rule_score: f64,
}

impl Default for ScanPolicy {
    fn default() -> Self {
        Self {
            concurrency: 4,
            rate_limit_per_min: 60,
            retry_max: 2,
            retry_backoff_ms: 500,
            timeout_secs: 15,
            dedup_window_hours: 24 * 7,
            max_leads_per_scan: 200,
            llm_eval_enabled: true,
            llm_eval_max_leads: 20,
            llm_eval_min_rule_score: 30.0,
        }
    }
}

impl ScanPolicy {
    /// 并发数（已钳到 >= 1）
    pub fn concurrency(&self) -> usize {
        self.concurrency.max(1)
    }

    /// 单次请求超时（已钳到 >= 1s）
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout_secs.max(1))
    }

    /// 相邻两次扫描请求的最小间隔；`None` 表示不限速
    pub fn min_request_interval(&self) -> Option<std::time::Duration> {
        if self.rate_limit_per_min == 0 {
            return None;
        }
        let per_request = 60.0 / f64::from(self.rate_limit_per_min);
        Some(std::time::Duration::from_secs_f64(per_request.max(0.0)))
    }

    /// 第 `attempt` 次重试的退避时长（`attempt` 从 1 开始）
    pub fn retry_backoff(&self, attempt: u32) -> std::time::Duration {
        let shift = attempt.saturating_sub(1).min(10);
        let factor = 1u64 << shift;
        std::time::Duration::from_millis(self.retry_backoff_ms.saturating_mul(factor))
    }

    /// 去重窗口（秒）；`None` 表示永久去重
    pub fn dedup_window_secs(&self) -> Option<i64> {
        if self.dedup_window_hours == 0 {
            return None;
        }
        Some(i64::from(self.dedup_window_hours) * 3600)
    }

    /// 从 JSON 反序列化；失败或字段越界时回落到默认值并做范围钳制
    pub fn from_json(json: &str) -> Self {
        match serde_json::from_str::<Self>(json) {
            Ok(p) => p.normalized(),
            Err(e) => {
                tracing::warn!(error = %e, "[ScanPolicy] 解析失败，回落到默认策略");
                Self::default()
            },
        }
    }

    /// 序列化为 JSON（用于写回设置表）
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        // normalized() 按值接收 self，而这里只有 &self，必须先 clone
        serde_json::to_string(&self.clone().normalized())
    }

    /// 范围钳制：并发/超时/上限设下限，重试次数与退避设上限
    pub fn normalized(mut self) -> Self {
        self.concurrency = self.concurrency.clamp(1, 32);
        self.rate_limit_per_min = self.rate_limit_per_min.min(6000);
        self.retry_max = self.retry_max.min(5);
        self.retry_backoff_ms = self.retry_backoff_ms.clamp(0, 30_000);
        self.timeout_secs = self.timeout_secs.clamp(1, 120);
        self.dedup_window_hours = self.dedup_window_hours.min(24 * 365);
        self.max_leads_per_scan = self.max_leads_per_scan.clamp(1, 5_000);
        self.llm_eval_max_leads = self.llm_eval_max_leads.clamp(1, 100);
        self.llm_eval_min_rule_score = self.llm_eval_min_rule_score.clamp(0.0, 100.0);
        self
    }
}

#[cfg(test)]
mod scan_policy_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn default_policy_is_valid() {
        let p = ScanPolicy::default();
        assert!(p.concurrency() >= 1);
        assert!(p.timeout() >= Duration::from_secs(1));
        assert!(p.retry_max <= 5);
    }

    #[test]
    fn zero_values_are_clamped() {
        let p = ScanPolicy {
            concurrency: 0,
            rate_limit_per_min: 0,
            retry_max: 0,
            retry_backoff_ms: 0,
            timeout_secs: 0,
            dedup_window_hours: 0,
            max_leads_per_scan: 0,
            ..Default::default()
        }
        .normalized();

        assert_eq!(p.concurrency(), 1);
        assert_eq!(p.timeout(), Duration::from_secs(1));
        assert_eq!(p.min_request_interval(), None, "rate_limit=0 表示不限速");
        assert_eq!(p.max_leads_per_scan, 1);
        assert_eq!(p.dedup_window_secs(), None, "窗口为 0 表示永久去重");
    }

    #[test]
    fn rate_limit_maps_to_interval() {
        let p = ScanPolicy { rate_limit_per_min: 120, ..Default::default() }.normalized();
        assert_eq!(p.min_request_interval(), Some(Duration::from_millis(500)));
    }

    #[test]
    fn retry_backoff_grows_exponentially() {
        let p = ScanPolicy { retry_backoff_ms: 500, ..Default::default() };
        assert_eq!(p.retry_backoff(1), Duration::from_millis(500));
        assert_eq!(p.retry_backoff(2), Duration::from_millis(1000));
        assert_eq!(p.retry_backoff(3), Duration::from_millis(2000));
    }

    #[test]
    fn dedup_window_converts_to_seconds() {
        let p = ScanPolicy { dedup_window_hours: 24, ..Default::default() };
        assert_eq!(p.dedup_window_secs(), Some(86_400));
    }

    #[test]
    fn json_roundtrip_preserves_fields() {
        let p = ScanPolicy { concurrency: 8, retry_max: 3, ..Default::default() };
        let json = p.to_json().expect("序列化应成功");
        let back = ScanPolicy::from_json(&json);
        assert_eq!(back.concurrency, 8);
        assert_eq!(back.retry_max, 3);
    }

    #[test]
    fn malformed_json_falls_back_to_default() {
        assert_eq!(ScanPolicy::from_json("{not json"), ScanPolicy::default());
        // 缺字段走 serde(default)，不应整体回落
        assert_eq!(ScanPolicy::from_json("{}"), ScanPolicy::default());
    }

    #[test]
    fn out_of_range_json_is_clamped() {
        let p = ScanPolicy::from_json(r#"{"concurrency":9999,"retryMax":99,"timeoutSecs":99999}"#);
        assert_eq!(p.concurrency, 32);
        assert_eq!(p.retry_max, 5);
        assert_eq!(p.timeout_secs, 120);
    }

    #[test]
    fn llm_eval_fields_have_defaults_and_clamps() {
        let p = ScanPolicy::default();
        assert!(p.llm_eval_enabled);
        assert_eq!(p.llm_eval_max_leads, 20);
        assert_eq!(p.llm_eval_min_rule_score, 30.0);

        // 旧策略 JSON 缺精评字段 → serde(default) 生效，不整体回落
        let legacy = ScanPolicy::from_json(r#"{"concurrency":6}"#);
        assert_eq!(legacy.concurrency, 6);
        assert!(legacy.llm_eval_enabled);
        assert_eq!(legacy.llm_eval_max_leads, 20);

        // 越界钳制
        let clamped = ScanPolicy {
            llm_eval_max_leads: 0,
            llm_eval_min_rule_score: 999.0,
            ..Default::default()
        }
        .normalized();
        assert_eq!(clamped.llm_eval_max_leads, 1);
        assert_eq!(clamped.llm_eval_min_rule_score, 100.0);
    }
}
