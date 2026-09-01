// SPDX-License-Identifier: AGPL-3.0-only

//! 市场平台扫描工具
//!
//! 抽象统一的需求线索采集接口，供闲鱼 / 猪八戒等平台连接器实现。
//! **合法合规优先**：所有连接器均通过平台**官方开放 API** 调用，不涉及爬虫或非法抓取。
//!
//! 三种连接器模式：
//! - `ApiMarketplaceScanner`：官方 API 调用（Token 认证）
//! - `MockMarketplaceScanner`：Mock 数据（测试/演示）
//! - `ManualMarketplaceScanner`：手动补录（最轻路径）

use crate::tools::arxiv_scanner::ArxivScanner;
use crate::tools::csdn_scanner::CsdnScanner;
use crate::tools::dribbble_scanner::DribbbleScanner;
use crate::tools::github_discussions_scanner::GitHubDiscussionsScanner;
use crate::tools::github_issue_scanner::GitHubIssueScanner;
use crate::tools::hacker_news_scanner::HackerNewsScanner;
use crate::tools::huggingface_scanner::HuggingFaceScanner;
use crate::tools::linkedin_scanner::LinkedInScanner;
use crate::tools::package_ecosystem_scanner::PackageEcosystemScanner;
use crate::tools::product_hunt_scanner::ProductHuntScanner;
use crate::tools::reddit_scanner::RedditScanner;
use crate::tools::scan_policy::ScanPolicy;
use crate::tools::scanner_common;
use crate::tools::stackoverflow_scanner::StackOverflowScanner;
use crate::tools::twitter_scanner::TwitterScanner;
use crate::tools::upwork_scanner::UpworkScanner;
use crate::tools::xianyu_scanner::XianyuScanner;
use crate::tools::zhihu_scanner::ZhihuScanner;
use crate::tools::zhubajie_scanner::ZhubajieScanner;
use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

// ── 内联评估类型（原 axagent-analysis-engine::opc::evaluator 精简版） ──

/// 需求类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum DemandType {
    #[default]
    Unknown,
    ToolSoftware,
    ContentCreation,
    Design,
    Development,
    Operations,
    Marketing,
    Education,
    EnterpriseService,
    Outsourcing,
    Consulting,
}

impl DemandType {
    /// snake_case 标识（与 serde 序列化一致），用于落库与跨层传递
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::ToolSoftware => "tool_software",
            Self::ContentCreation => "content_creation",
            Self::Design => "design",
            Self::Development => "development",
            Self::Operations => "operations",
            Self::Marketing => "marketing",
            Self::Education => "education",
            Self::EnterpriseService => "enterprise_service",
            Self::Outsourcing => "outsourcing",
            Self::Consulting => "consulting",
        }
    }
}

impl std::str::FromStr for DemandType {
    type Err = ();

    /// 解析 `as_str()` 产出的 snake_case 标识；无法识别返回 `Err(())`
    ///
    /// 反向查表复用 `as_str()`，不重复写一遍字面量 —— 两处各写一遍必然漂移。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const ALL: [DemandType; 11] = [
            DemandType::Unknown,
            DemandType::ToolSoftware,
            DemandType::ContentCreation,
            DemandType::Design,
            DemandType::Development,
            DemandType::Operations,
            DemandType::Marketing,
            DemandType::Education,
            DemandType::EnterpriseService,
            DemandType::Outsourcing,
            DemandType::Consulting,
        ];
        ALL.into_iter().find(|d| d.as_str() == s).ok_or(())
    }
}

/// 价格区间
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PriceRange {
    min: f64,
    max: f64,
    currency: String,
    confidence: f64,
}

/// 需求价值评估结果
///
/// `opportunity_level` 不落字段：等级由 `commercial_value_score` 推导
/// （见同名方法），存两份必然漂移 —— 曾经字段/方法重名导致方法被字段
/// 遮蔽，`opportunity_level()` 恒返回空串。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemandEvaluation {
    demand_id: String,
    pain_score: f64,
    existing_solutions: u32,
    market_gap_score: f64,
    commercial_value_score: f64,
    confidence: f64,
    demand_type: DemandType,
    extracted_price_range: Option<PriceRange>,
    market_fit_score: f64,
}

impl DemandEvaluation {
    pub fn opportunity_level(&self) -> &str {
        match self.commercial_value_score {
            v if v >= 80.0 => "very_high",
            v if v >= 60.0 => "high",
            v if v >= 40.0 => "medium",
            _ => "low",
        }
    }

    /// 痛点强度分（持久化用）
    pub fn pain_score(&self) -> f64 {
        self.pain_score
    }

    /// 市场空白度分（持久化用）
    pub fn market_gap_score(&self) -> f64 {
        self.market_gap_score
    }

    /// 商业价值综合分（持久化用）
    pub fn commercial_value_score(&self) -> f64 {
        self.commercial_value_score
    }

    /// 评估置信度（持久化用）
    pub fn confidence(&self) -> f64 {
        self.confidence
    }

    /// 需求类型（持久化用）
    pub fn demand_type(&self) -> &DemandType {
        &self.demand_type
    }
}

// ── 评分引擎 ──────────────────────────────────────────────────
//
// 旧实现有两个硬伤，导致 `opportunity_level()` 的 `very_high` 档永远不可达：
//   1. `pain_score` 被 clamp 到 90、`market_gap_score` 硬编码 50，
//      加权后上限 = 90×0.5 + 50×0.5 = 70 < 80（`very_high` 门槛）；
//   2. `market_gap_score` 不看竞争情况、`budget`/热度完全不参与评分。
//
// 现改为多因子加权，各因子均归一化到 0-100，权重合计 1.0。

/// 痛点强度权重
const W_PAIN: f64 = 0.35;
/// 市场空白度权重
const W_MARKET_GAP: f64 = 0.20;
/// 预算/商业价值权重
const W_BUDGET: f64 = 0.25;
/// 社区热度权重
const W_ENGAGEMENT: f64 = 0.20;

/// 痛点关键词
///
/// 必须去重：旧实现里 `"urgent"` 出现两次，`filter().count()` 会把它算两遍。
const PAIN_KEYWORDS: &[&str] = &[
    "urgent",
    "critical",
    "frustrated",
    "painful",
    "deadline",
    "asap",
    "急需",
    "痛点",
    "麻烦",
    "困难",
    "求助",
    "崩溃",
    "卡住",
];

/// 命中多少个痛点关键词算「饱和」
const PAIN_SATURATION_HITS: f64 = 3.0;
/// 预算归一化上限（人民币）；对数曲线，10 万元视为饱和
const BUDGET_SATURATION: f64 = 100_000.0;
/// 热度归一化上限（互动量）；1000 次互动视为饱和
const ENGAGEMENT_SATURATION: f64 = 1_000.0;
/// 无预算信息时的预算分（中性偏保守，不假设高价）
const BUDGET_FALLBACK_SCORE: f64 = 40.0;
/// 无热度信息时的热度分（无证据即给低分，不虚高）
const ENGAGEMENT_FALLBACK_SCORE: f64 = 20.0;

/// 热度字段名候选（各平台命名不一）
const ENGAGEMENT_KEYS: &[&str] = &[
    "score",
    "points",
    "ups",
    "likes",
    "views",
    "comments",
    "num_comments",
    "reactions",
    "heat",
    "hot",
    "votes",
    "阅读",
    "点赞",
    "评论",
    "浏览",
    "热度",
];

/// 痛点强度评分（0-100）
///
/// 命中数达到 [`PAIN_SATURATION_HITS`] 即饱和，避免靠堆砌关键词刷分。
fn score_pain(text: &str) -> f64 {
    let hits = PAIN_KEYWORDS.iter().filter(|k| text.contains(*k)).count() as f64;
    (hits / PAIN_SATURATION_HITS).min(1.0) * 100.0
}

/// 市场空白度评分（0-100）
///
/// 已有解决方案越多，空白越小；无数据时取中位。
fn score_market_gap(known_competitors: Option<u32>) -> f64 {
    match known_competitors {
        Some(n) => (100.0 - n.min(10) as f64 * 10.0).max(0.0),
        None => 50.0,
    }
}

/// 预算评分（0-100）与价格区间
///
/// 对数归一化：1 万元 ≈ 66 分，10 万元及以上 = 100 分，避免线性尺度下
/// 少数天价需求把分值拉爆。
fn score_budget(lead: &DemandLead) -> (f64, Option<PriceRange>) {
    let upper = match (lead.budget_min, lead.budget_max) {
        (Some(min), Some(max)) => Some(min.max(max)),
        (Some(v), None) | (None, Some(v)) => Some(v),
        (None, None) => None,
    };

    let Some(upper) = upper.filter(|v| v.is_finite() && *v > 0.0) else {
        return (BUDGET_FALLBACK_SCORE, None);
    };

    let score = ((1.0 + upper).ln() / (1.0 + BUDGET_SATURATION).ln() * 100.0).clamp(0.0, 100.0);
    let range = PriceRange {
        min: lead.budget_min.unwrap_or(0.0),
        max: lead.budget_max.unwrap_or(upper),
        currency: lead.budget_currency.clone(),
        confidence: 0.5,
    };
    (score, Some(range))
}

/// 社区热度评分（0-100）
///
/// 从 `raw_snapshot` 中按 [`ENGAGEMENT_KEYS`] 找最大互动量，对数归一化。
fn score_engagement(snapshot: &serde_json::Value) -> f64 {
    let peak = collect_engagement_numbers(snapshot, 0).into_iter().fold(0.0, f64::max);
    if peak <= 0.0 {
        return ENGAGEMENT_FALLBACK_SCORE;
    }
    ((1.0 + peak).ln() / (1.0 + ENGAGEMENT_SATURATION).ln() * 100.0).clamp(0.0, 100.0)
}

/// 递归收集热度数值（最多下钻两层，避免遍历整个快照）
fn collect_engagement_numbers(value: &serde_json::Value, depth: usize) -> Vec<f64> {
    if depth > 2 {
        return Vec::new();
    }
    let mut found = Vec::new();
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                // clippy::collapsible_if — 合并为单层条件
                if ENGAGEMENT_KEYS.contains(&key.as_str())
                    && let Some(n) = val.as_f64()
                {
                    found.push(n);
                }
                found.extend(collect_engagement_numbers(val, depth + 1));
            }
        },
        serde_json::Value::Array(items) => {
            for item in items {
                found.extend(collect_engagement_numbers(item, depth + 1));
            }
        },
        _ => {},
    }
    found
}

/// 从平台配置中提取 API Token（空串视为未配置）
///
/// DB `config_json.api_token` 优先；各扫描器的 `with_config` 内部在
/// token 为 None 时再回退读环境变量。
fn config_token(config: &serde_json::Value) -> Option<String> {
    config
        .get("api_token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
}

/// 按平台名返回内置扫描器实例（带凭证与端点透传）
///
/// 平台名与各扫描器 `platform()` 返回值一致；无匹配时返回 `None`，
/// 由调用方决定回退策略（如手动补录）。
///
/// 凭证来源优先级：`config.api_token`（前端平台配置）> 环境变量（各扫描器
/// `with_config` 内部兜底）。`base_url` 来自 DB 平台配置，覆盖扫描器默认端点。
/// 此前本函数不接收任何配置，DB/前端配的 token 全被扔掉，桌面 GUI 进程
/// 几乎不带环境变量 → 11 个需凭证平台永远「合规跳过」。
fn builtin_scanner_for(
    platform: &str,
    base_url: Option<&str>,
    config: &serde_json::Value,
) -> Option<Box<dyn MarketplaceScanner>> {
    use super::{
        arxiv_scanner::ArxivScanner, csdn_scanner::CsdnScanner, dribbble_scanner::DribbbleScanner,
        github_discussions_scanner::GitHubDiscussionsScanner,
        github_issue_scanner::GitHubIssueScanner, hacker_news_scanner::HackerNewsScanner,
        huggingface_scanner::HuggingFaceScanner, linkedin_scanner::LinkedInScanner,
        package_ecosystem_scanner::PackageEcosystemScanner,
        product_hunt_scanner::ProductHuntScanner, reddit_scanner::RedditScanner,
        stackoverflow_scanner::StackOverflowScanner, twitter_scanner::TwitterScanner,
        upwork_scanner::UpworkScanner, xianyu_scanner::XianyuScanner, zhihu_scanner::ZhihuScanner,
        zhubajie_scanner::ZhubajieScanner,
    };
    let token = config_token(config);
    let base = base_url.map(str::to_string);
    let scanner: Box<dyn MarketplaceScanner> = match platform {
        // 免费公开 API，无需凭证；端点固定，不透传配置
        "arxiv" => Box::new(ArxivScanner::new()),
        "hackernews" => Box::new(HackerNewsScanner::new()),
        "package_ecosystem" => Box::new(PackageEcosystemScanner::new()),
        "reddit" => Box::new(RedditScanner::new()),
        "csdn" => Box::new(CsdnScanner::csdn()),
        "juejin" => Box::new(CsdnScanner::juejin()),
        // 可选/必需凭证平台：透传 config token + base_url
        "dribbble" => Box::new(DribbbleScanner::with_config(token, base)),
        "github_issue" => Box::new(GitHubIssueScanner::with_config(token, base)),
        "github_discussion" => Box::new(GitHubDiscussionsScanner::with_config(token, base)),
        "huggingface" => Box::new(HuggingFaceScanner::with_config(token, base)),
        "linkedin" => Box::new(LinkedInScanner::with_config(token, base)),
        "producthunt" => Box::new(ProductHuntScanner::with_config(token, base)),
        "stackoverflow" => Box::new(StackOverflowScanner::with_config(token, base)),
        "twitter" => Box::new(TwitterScanner::with_config(token, base)),
        "upwork" => Box::new(UpworkScanner::with_config(token, base)),
        "xianyu" => Box::new(XianyuScanner::with_config(token, base)),
        "zhihu" => Box::new(ZhihuScanner::with_config(token, base)),
        "zhubajie" => Box::new(ZhubajieScanner::with_config(token, base)),
        _ => return None,
    };
    Some(scanner)
}

/// 需求类型分类（关键词启发式）
///
/// P1-3 修正：旧实现单关键词命中即返回且"外包"排最前，几乎一切含
/// "开发"的长文本都被错分为 Outsourcing。现在按各类别**命中数**择优，
/// 平分时保持规则表优先级序。
fn classify_demand(text: &str) -> DemandType {
    let rules: &[(DemandType, &[&str])] = &[
        (DemandType::Outsourcing, &["外包", "outsourc", "freelance", "兼职", "接单"]),
        (DemandType::Design, &["设计", "design", "logo", "ui", "ux", "插画"]),
        (DemandType::Development, &["开发", "develop", "程序", "小程序", "app", "网站", "code"]),
        (DemandType::ToolSoftware, &["工具", "tool", "软件", "software", "saas", "插件"]),
        (DemandType::ContentCreation, &["文案", "content", "视频", "剪辑", "写作", "writing"]),
        (DemandType::Marketing, &["营销", "marketing", "推广", "seo", "增长", "growth"]),
        (DemandType::Operations, &["运营", "operation", "客服", "运维"]),
        (DemandType::Education, &["培训", "教育", "课程", "education", "教学"]),
        (DemandType::EnterpriseService, &["企业", "enterprise", "erp", "crm", "数字化"]),
        (DemandType::Consulting, &["咨询", "consult", "顾问", "方案"]),
    ];
    let mut best: Option<(&DemandType, usize)> = None;
    for (demand_type, keywords) in rules {
        let hits = keywords.iter().filter(|k| text.contains(*k)).count();
        if hits == 0 {
            continue;
        }
        match best {
            Some((_, n)) if n >= hits => {},
            _ => best = Some((demand_type, hits)),
        }
    }
    best.map(|(t, _)| t.clone()).unwrap_or(DemandType::Unknown)
}

/// 评估单条需求线索
///
/// 综合痛点、市场空白、预算、热度四个因子加权得出商业价值分（0-100）。
pub fn evaluate_lead(lead: &DemandLead) -> DemandEvaluation {
    evaluate_lead_with_competitors(lead, None)
}

/// 评估单条需求线索（可指定已知竞品数量）
pub fn evaluate_lead_with_competitors(
    lead: &DemandLead,
    known_competitors: Option<u32>,
) -> DemandEvaluation {
    let text = format!("{} {}", lead.title, lead.description).to_lowercase();

    let pain_score = score_pain(&text);
    let market_gap_score = score_market_gap(known_competitors);
    let (budget_score, price_range) = score_budget(lead);
    let engagement_score = score_engagement(&lead.raw_snapshot);

    let commercial_value_score = (pain_score * W_PAIN
        + market_gap_score * W_MARKET_GAP
        + budget_score * W_BUDGET
        + engagement_score * W_ENGAGEMENT)
        .round()
        .clamp(0.0, 100.0);

    // 置信度：拿到的信号越多越可信
    let mut confidence: f64 = 0.2;
    if price_range.is_some() {
        confidence += 0.3;
    }
    if engagement_score != ENGAGEMENT_FALLBACK_SCORE {
        confidence += 0.2;
    }
    if pain_score > 0.0 {
        confidence += 0.2;
    }

    DemandEvaluation {
        demand_id: lead.id.clone(),
        pain_score,
        existing_solutions: known_competitors.unwrap_or(0),
        market_gap_score,
        commercial_value_score,
        confidence: confidence.clamp(0.1, 0.9),
        demand_type: classify_demand(&text),
        extracted_price_range: price_range,
        // 与「我们能力的契合度」需要能力画像才能计算，当前无输入源，保持中性
        market_fit_score: 50.0,
    }
}

// ── DTO 定义 ──────────────────────────────────────────────────

/// 原始线索（平台返回的原始数据，未经归一化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawLead {
    pub platform: String,
    pub title: String,
    pub description: String,
    pub url: String,
    pub price_text: Option<String>,
    pub contact: Option<String>,
    pub contact_email: Option<String>,
    pub contact_phone: Option<String>,
    pub snapshot: serde_json::Value,
}

/// 归一化后的需求线索
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemandLead {
    pub id: String,
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
    pub raw_snapshot: serde_json::Value,
    pub status: String,
    pub confidence: f64,
    /// 内容指纹（标题+描述归一化哈希）：去重主键。
    /// 旧键 `(platform, source_url)` 在所有线索共享同一搜索页 URL 的平台
    /// （闲鱼等）上会把一轮线索压成 1 条 —— 指纹只看内容，与 URL 无关。
    pub content_fingerprint: Option<String>,
}

impl DemandLead {
    pub fn new_from_raw(raw: RawLead) -> Self {
        let id = format!("{}_{}", raw.platform, uuid::Uuid::new_v4().simple());
        // 预算：从价格文本解析（此前恒为 None，预算因子空转 —— P0-2）
        let (budget_min, budget_max, budget_currency) =
            parse_budget_info(raw.price_text.as_deref());
        // 联系方式：结构化字段缺失时对 title+description 兜底提取（此前恒 None —— P0-4）
        let mut contact_email = raw.contact_email;
        let mut contact_phone = raw.contact_phone;
        let mut snapshot = raw.snapshot;
        if contact_email.is_none() || contact_phone.is_none() {
            let text = format!("{} {}", raw.title, raw.description);
            let extracted = scanner_common::extract_contacts(&text);
            if contact_email.is_none() {
                contact_email = extracted.email.clone();
            }
            if contact_phone.is_none() {
                contact_phone = extracted.phone.clone();
            }
            // 微信号不进 phone 字段（语义污染）：落到快照的 contact_wechat。
            // 快照为 Null 时升级为空对象（否则 as_object_mut 拿 None，微信丢失）
            if let Some(wechat) = extracted.wechat {
                if snapshot.is_null() {
                    snapshot = serde_json::json!({});
                }
                if let Some(obj) = snapshot.as_object_mut() {
                    obj.entry("contact_wechat".to_string())
                        .or_insert(serde_json::Value::String(wechat));
                }
            }
        }
        // 内容指纹：去重键（P0-5）。同一条需求换个 URL 重发也能被识别。
        // 先于 Self 构造计算 —— title/description 随后 move 进结构体
        let content_fingerprint = scanner_common::content_fingerprint(&raw.title, &raw.description);
        Self {
            id,
            platform: raw.platform,
            title: raw.title,
            description: raw.description,
            budget_min,
            budget_max,
            budget_currency,
            contact_name: raw.contact,
            contact_email,
            contact_phone,
            source_url: Some(raw.url),
            raw_snapshot: snapshot,
            status: "new".to_string(),
            confidence: 0.0,
            content_fingerprint,
        }
    }
}

/// 带评估结果的需求线索
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatedDemandLead {
    pub lead: DemandLead,
    pub evaluation: DemandEvaluation,
}

impl EvaluatedDemandLead {
    pub fn new(lead: DemandLead, evaluation: DemandEvaluation) -> Self {
        Self { lead, evaluation }
    }

    pub fn value_score(&self) -> f64 {
        self.evaluation.commercial_value_score
    }

    pub fn opportunity_level(&self) -> String {
        self.evaluation.opportunity_level().to_string()
    }
}

// ── 扫描 trait ────────────────────────────────────────────────

/// 市场平台扫描器统一接口
///
/// 各平台实现此 trait，负责**通过官方 API** 定向检索与原始数据采集。
#[async_trait]
pub trait MarketplaceScanner: Send + Sync {
    /// 平台标识（如 "xianyu" / "zhubajie"）
    ///
    /// 返回 `String` 而非 `&'static str`：平台名可能来自运行时配置，
    /// 用静态生命周期会逼迫实现方 `Box::leak` 堆内存（原实现有 3 处泄漏）。
    fn platform(&self) -> String;

    /// 按关键词搜索需求线索（通过官方 API）
    async fn search(&self, q: &str) -> Result<Vec<RawLead>, String>;
}

// ── 聚合扫描器 ────────────────────────────────────────────────

/// 单平台一轮扫描的结果
///
/// 并发扫描下需要把「失败」与「跳过」显式带回来，供上层统计与回写同步状态；
/// 旧实现只在 warn 日志里丢一句，调用方无法区分「该平台挂了」和「该平台没数据」。
#[derive(Debug, Clone)]
pub struct PlatformScanResult {
    /// 平台标识
    pub platform: String,
    /// 扫到的线索（已转为 `DemandLead`）
    pub leads: Vec<DemandLead>,
    /// 失败原因；`None` 表示成功
    pub error: Option<String>,
    /// 实际尝试次数（含首次）
    pub attempts: u32,
    /// 合规跳过：未配置官方 API 凭证，主动放弃而非失败
    pub compliance_skipped: bool,
}

/// 单平台一轮「扫描 + 评估」的结果
#[derive(Debug, Clone)]
pub struct PlatformEvaluatedResult {
    /// 平台标识
    pub platform: String,
    /// 已评估的线索
    pub leads: Vec<EvaluatedDemandLead>,
    /// 失败原因；`None` 表示成功
    pub error: Option<String>,
    /// 实际尝试次数（含首次）
    pub attempts: u32,
    /// 合规跳过：未配置官方 API 凭证
    pub compliance_skipped: bool,
}

/// 全局速率闸门
///
/// 用「预约下一个可用时刻」而非「持锁睡眠」实现：拿到时刻后立即放锁再睡，
/// 这样并发度不会被限流器压成 1。
struct RateGate {
    min_interval: Option<Duration>,
    next_available: Mutex<Instant>,
}

impl RateGate {
    fn new(min_interval: Option<Duration>) -> Self {
        Self { min_interval, next_available: Mutex::new(Instant::now()) }
    }

    async fn wait(&self) {
        let Some(interval) = self.min_interval else {
            return;
        };
        let start = {
            let mut guard = self.next_available.lock().await;
            let now = Instant::now();
            let slot = (*guard).max(now);
            *guard = slot + interval;
            slot
        };
        if let Some(delay) = start.checked_duration_since(Instant::now())
            && !delay.is_zero()
        {
            tokio::time::sleep(delay).await;
        }
    }
}

/// 进程级共享速率闸门（P1-5）
///
/// 闸门必须跨扫描调用共享：订阅扫描逐词新建聚合扫描器，闸门若随扫描器
/// 重建，N 个订阅词 = N 倍真实请求速率，`rate_limit_per_min` 名存实亡。
/// 限流保护的是外部 API 配额，进程全局共享才是正确语义；间隔配置变更时
/// 重建闸门（旧预约时刻作废可接受）。不限速时返回空闸门，不占共享槽。
/// 共享闸门槽：`(配置的请求间隔, 闸门实例)`
type SharedGateSlot = Option<(Duration, std::sync::Arc<RateGate>)>;
static SHARED_RATE_GATE: std::sync::OnceLock<tokio::sync::Mutex<SharedGateSlot>> =
    std::sync::OnceLock::new();

async fn shared_rate_gate(min_interval: Option<Duration>) -> std::sync::Arc<RateGate> {
    let Some(interval) = min_interval else {
        return std::sync::Arc::new(RateGate::new(None));
    };
    let cell = SHARED_RATE_GATE.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut guard = cell.lock().await;
    match guard.as_ref() {
        Some((configured, gate)) if *configured == interval => gate.clone(),
        _ => {
            let gate = std::sync::Arc::new(RateGate::new(Some(interval)));
            *guard = Some((interval, gate.clone()));
            gate
        },
    }
}

/// 带超时与指数退避重试的单平台搜索
///
/// 返回 `(线索, 失败原因, 尝试次数, 是否合规跳过)`。
/// 合规跳过（无官方 API 凭证）**不重试**——重试只会放大无效日志与等待。
async fn search_with_retry(
    scanner: &(dyn MarketplaceScanner + Sync),
    q: &str,
    policy: &ScanPolicy,
) -> (Vec<DemandLead>, Option<String>, u32, bool) {
    let mut attempts: u32 = 0;
    loop {
        attempts += 1;
        match tokio::time::timeout(policy.timeout(), scanner.search(q)).await {
            Ok(Ok(raw)) => {
                return (
                    raw.into_iter().map(DemandLead::new_from_raw).collect(),
                    None,
                    attempts,
                    false,
                );
            },
            Ok(Err(e)) => {
                if e == crate::tools::scanner_common::NO_CREDENTIAL_SKIP_REASON {
                    return (Vec::new(), None, attempts, true);
                }
                // P2-3：非网络类错误（401 配置错误、400 参数错误、JSON 解析
                // 失败等）结果确定，重试只会放大无效请求 —— 立即失败
                if !is_network_env_error(&e) {
                    tracing::warn!(
                        platform = scanner.platform(),
                        attempt = attempts,
                        error = %e,
                        "[search_with_retry] 非网络类错误，不重试"
                    );
                    return (Vec::new(), Some(e), attempts, false);
                }
                if attempts > policy.retry_max {
                    return (Vec::new(), Some(e), attempts, false);
                }
                tracing::warn!(
                    platform = scanner.platform(),
                    attempt = attempts,
                    error = %e,
                    "[search_with_retry] 扫描失败，准备重试"
                );
            },
            Err(_) => {
                let msg = format!("扫描超时（{}s）", policy.timeout().as_secs());
                if attempts > policy.retry_max {
                    return (Vec::new(), Some(msg), attempts, false);
                }
                tracing::warn!(
                    platform = scanner.platform(),
                    attempt = attempts,
                    "[search_with_retry] 扫描超时，准备重试"
                );
            },
        }
        tokio::time::sleep(policy.retry_backoff(attempts)).await;
    }
}

/// 多平台聚合扫描器
pub struct AggregateMarketplaceScanner {
    scanners: Vec<Box<dyn MarketplaceScanner>>,
    /// 记录被禁用的平台名称
    disabled_platforms: std::collections::HashSet<String>,
    /// 扫描策略（并发 / 限流 / 重试 / 超时）
    policy: ScanPolicy,
}

impl AggregateMarketplaceScanner {
    pub fn new() -> Self {
        Self {
            scanners: Vec::new(),
            disabled_platforms: std::collections::HashSet::new(),
            policy: ScanPolicy::default(),
        }
    }

    /// 以指定策略构造（默认策略见 [`ScanPolicy::default`]）
    pub fn with_policy(policy: ScanPolicy) -> Self {
        Self {
            scanners: Vec::new(),
            disabled_platforms: std::collections::HashSet::new(),
            policy: policy.normalized(),
        }
    }

    /// 覆盖扫描策略
    pub fn set_policy(&mut self, policy: ScanPolicy) {
        self.policy = policy.normalized();
    }

    /// 当前生效的扫描策略
    pub fn policy(&self) -> &ScanPolicy {
        &self.policy
    }

    pub fn add_scanner(&mut self, scanner: Box<dyn MarketplaceScanner>) {
        self.scanners.push(scanner);
    }

    /// 禁用指定平台的扫描器
    pub fn disable_scanner(&mut self, platform: &str) {
        self.disabled_platforms.insert(platform.to_string());
        tracing::info!(platform = platform, "[AggregateMarketplaceScanner] 已禁用扫描器");
    }

    /// 启用指定平台的扫描器
    pub fn enable_scanner(&mut self, platform: &str) {
        self.disabled_platforms.remove(platform);
        tracing::info!(platform = platform, "[AggregateMarketplaceScanner] 已启用扫描器");
    }

    /// 检查扫描器是否启用
    pub fn is_scanner_enabled(&self, platform: &str) -> bool {
        !self.disabled_platforms.contains(platform)
    }

    /// 列出所有已注册的平台及其启用状态
    pub fn list_scanners(&self) -> Vec<(String, bool)> {
        self.scanners
            .iter()
            .map(|s| {
                let p = s.platform();
                let enabled = !self.disabled_platforms.contains(&p);
                (p, enabled)
            })
            .collect()
    }

    /// 从平台配置批量注册扫描器
    ///
    /// `platform_type` 对应三种连接器：
    /// - `"api"` → `ApiMarketplaceScanner`（官方 API，需配置 API Token）
    /// - `"scanner"` → 按平台名路由到对应的内置平台扫描器（详见 [`builtin_scanner_for`]）
    /// - `"mock"` → `MockMarketplaceScanner`（模拟数据，用于测试）
    /// - `"manual"` / 其他 → `ManualMarketplaceScanner`（手动补录）
    pub fn add_platform(
        &mut self,
        platform: &str,
        platform_type: &str,
        base_url: Option<&str>,
        config: &serde_json::Value,
    ) {
        match platform_type {
            "api" => {
                self.add_scanner(Box::new(ApiMarketplaceScanner::new(platform, base_url, config)));
            },
            "scanner" => match builtin_scanner_for(platform, base_url, config) {
                Some(s) => self.add_scanner(s),
                None => {
                    tracing::warn!(
                        platform = platform,
                        "[AggregateMarketplaceScanner] 无内置扫描器，回退为手动补录"
                    );
                    self.add_scanner(Box::new(ManualMarketplaceScanner::new(
                        platform, base_url, config,
                    )));
                },
            },
            "mock" => {
                // P2-4：mock 连接器只进调试构建，或配置显式 allow_mock ——
                // 否则误配 "mock" 类型会把 3 条假线索真实入库（占去重键、污染统计）
                let allowed = cfg!(debug_assertions)
                    || config.get("allow_mock").and_then(|v| v.as_bool()).unwrap_or(false);
                if allowed {
                    self.add_scanner(Box::new(MockMarketplaceScanner::new(platform)));
                } else {
                    tracing::warn!(
                        platform = platform,
                        "[AggregateMarketplaceScanner] mock 连接器已在生产配置下禁用，回退为手动补录"
                    );
                    self.add_scanner(Box::new(ManualMarketplaceScanner::new(
                        platform, base_url, config,
                    )));
                }
            },
            _ => self
                .add_scanner(Box::new(ManualMarketplaceScanner::new(platform, base_url, config))),
        }
    }

    /// 并发扫描全部启用的平台
    ///
    /// 行为由 [`ScanPolicy`] 驱动：
    /// - `concurrency` → `buffer_unordered` 的并发上限（原来是串行 `for` 循环）
    /// - `rate_limit_per_min` → 全局最小请求间隔闸门
    /// - `retry_max` / `retry_backoff_ms` → 指数退避重试
    /// - `timeout_secs` → 单平台单次请求超时
    ///
    /// 被禁用（`disable_scanner`）的平台直接跳过，不占用并发与限流额度。
    pub async fn scan_platforms(&self, q: &str) -> Vec<PlatformScanResult> {
        // normalized() 按值接收 self，&self 方法里需先 clone 字段
        let policy = self.policy.clone().normalized();
        let concurrency = policy.concurrency();
        // 共享进程级闸门：多次扫描调用（订阅逐词扫描）共用同一个预约时刻轴
        let gate = shared_rate_gate(policy.min_request_interval()).await;

        let enabled: Vec<&Box<dyn MarketplaceScanner>> = self
            .scanners
            .iter()
            .filter(|s| {
                let enabled = self.is_scanner_enabled(&s.platform());
                if !enabled {
                    tracing::debug!(
                        platform = s.platform(),
                        "[AggregateMarketplaceScanner] 扫描器已禁用，跳过"
                    );
                }
                enabled
            })
            .collect();

        // 用显式 for 循环构造 future，而不是 `.map(|scanner| async move { .. })`。
        // edition 2024 下 `map` 闭包的参数是引用类型时会被推断成高阶生命周期（HRTB），
        // 但 async block 捕获的是**固定**生命周期的 `&Box<dyn MarketplaceScanner>`，两者
        // 不匹配 → `implementation of FnOnce is not general enough`（仅 lib test 构建触发，
        // `cargo check` / `clippy` 反而不报）。去掉闭包后借用直接进 async block，问题消失。
        let mut tasks = Vec::with_capacity(enabled.len());
        for scanner in enabled {
            let gate = Arc::clone(&gate);
            let policy = policy.clone();
            let q = q.to_string();
            tasks.push(async move {
                gate.wait().await;
                let (leads, error, attempts, compliance_skipped) =
                    search_with_retry(scanner.as_ref(), &q, &policy).await;
                if let Some(e) = &error {
                    tracing::warn!(
                        platform = scanner.platform(),
                        attempts = attempts,
                        error = %e,
                        "[AggregateMarketplaceScanner] 扫描器最终失败，跳过"
                    );
                }
                PlatformScanResult {
                    platform: scanner.platform(),
                    leads,
                    error,
                    attempts,
                    compliance_skipped,
                }
            });
        }

        stream::iter(tasks).buffer_unordered(concurrency).collect().await
    }

    /// 扫描全部启用平台并汇总线索（不区分来源成败）
    ///
    /// 结果按 `max_leads_per_scan` 截断。需要逐平台成败明细时改用 [`Self::scan_platforms`]。
    pub async fn search_all(&self, q: &str) -> Result<Vec<DemandLead>, String> {
        let limit = self.policy.max_leads_per_scan;
        let mut leads: Vec<DemandLead> = Vec::new();
        for result in self.scan_platforms(q).await {
            for lead in result.leads {
                if leads.len() >= limit {
                    return Ok(leads);
                }
                leads.push(lead);
            }
        }
        Ok(leads)
    }

    /// 逐平台「扫描 + 评估」，保留每个平台的成败明细
    ///
    /// 命令层需要它来做两件 `search_and_evaluate` 做不到的事：
    /// 1. 回写**单个平台**的同步状态（成功 / 失败 / 合规跳过）
    /// 2. 区分「该平台挂了」与「该平台确实没数据」
    pub async fn scan_and_evaluate_platforms(&self, q: &str) -> Vec<PlatformEvaluatedResult> {
        self.scan_platforms(q)
            .await
            .into_iter()
            .map(|r| PlatformEvaluatedResult {
                platform: r.platform,
                // 直接评估完整 lead：预算与热度快照参与评分。
                // 旧实现走 `to_evaluation_input()` 重建假 lead（budget=None /
                // snapshot=Null），四因子只有痛点真实生效，全库分数被压到 ≤59，
                // very_high/high 档与订阅推送（min_score=60）永不可达。
                leads: r
                    .leads
                    .into_iter()
                    .map(|lead| {
                        let evaluation = evaluate_lead(&lead);
                        EvaluatedDemandLead::new(lead, evaluation)
                    })
                    .collect(),
                error: r.error,
                attempts: r.attempts,
                compliance_skipped: r.compliance_skipped,
            })
            .collect()
    }

    /// 搜索需求线索并执行价值评估
    ///
    /// 完整流水线：扫描 → 评估 → 筛选高价值 → 排序
    pub async fn search_and_evaluate(&self, q: &str) -> Result<Vec<EvaluatedDemandLead>, String> {
        let limit = self.policy.max_leads_per_scan;
        let mut evaluated: Vec<EvaluatedDemandLead> =
            self.scan_and_evaluate_platforms(q).await.into_iter().flat_map(|r| r.leads).collect();

        // 按价值分排序
        evaluated.sort_by(|a, b| {
            b.value_score().partial_cmp(&a.value_score()).unwrap_or(std::cmp::Ordering::Equal)
        });
        evaluated.truncate(limit);

        Ok(evaluated)
    }

    /// 搜索并筛选高价值需求
    ///
    /// # 参数
    /// - `q`: 搜索关键词
    /// - `min_score`: 最低价值分阈值（默认 50.0）
    ///
    /// # 返回
    /// 高价值需求列表（已按价值分排序）
    pub async fn search_high_value(
        &self,
        q: &str,
        min_score: f64,
    ) -> Result<Vec<EvaluatedDemandLead>, String> {
        let evaluated = self.search_and_evaluate(q).await?;
        let filtered: Vec<EvaluatedDemandLead> =
            evaluated.into_iter().filter(|e| e.value_score() >= min_score).collect();
        Ok(filtered)
    }

    /// 对已有线索进行批量评估
    ///
    /// 与 [`Self::search_and_evaluate`] 行为对齐：结果按价值分降序。
    pub fn evaluate_leads(&self, leads: Vec<DemandLead>) -> Vec<EvaluatedDemandLead> {
        let mut evaluated: Vec<EvaluatedDemandLead> = leads
            .into_iter()
            .map(|lead| {
                let evaluation = evaluate_lead(&lead);
                EvaluatedDemandLead::new(lead, evaluation)
            })
            .collect();
        evaluated.sort_by(|a, b| {
            b.value_score().partial_cmp(&a.value_score()).unwrap_or(std::cmp::Ordering::Equal)
        });
        evaluated
    }
}

impl Default for AggregateMarketplaceScanner {
    fn default() -> Self {
        let mut scanner = Self::new();
        // 注册技术社区扫描器
        scanner.add_scanner(Box::new(RedditScanner::new()));
        scanner.add_scanner(Box::new(HackerNewsScanner::new()));
        scanner.add_scanner(Box::new(GitHubIssueScanner::new()));
        scanner.add_scanner(Box::new(GitHubDiscussionsScanner::new()));
        scanner.add_scanner(Box::new(StackOverflowScanner::new()));
        // 注册产品生态扫描器
        scanner.add_scanner(Box::new(ProductHuntScanner::new()));
        scanner.add_scanner(Box::new(HuggingFaceScanner::new()));
        scanner.add_scanner(Box::new(PackageEcosystemScanner::new()));
        // 注册研究动态扫描器
        scanner.add_scanner(Box::new(ArxivScanner::new()));
        // 注册社交媒体扫描器
        scanner.add_scanner(Box::new(TwitterScanner::new()));
        // 注册中国市场扫描器
        scanner.add_scanner(Box::new(ZhubajieScanner::new()));
        scanner.add_scanner(Box::new(XianyuScanner::new()));
        // 注册 B2B/企业需求扫描器
        scanner.add_scanner(Box::new(LinkedInScanner::new()));
        // 注册中国开发者社区扫描器
        scanner.add_scanner(Box::new(ZhihuScanner::new()));
        scanner.add_scanner(Box::new(CsdnScanner::csdn()));
        scanner.add_scanner(Box::new(CsdnScanner::juejin()));
        // 注册设计需求扫描器
        scanner.add_scanner(Box::new(DribbbleScanner::new()));
        // 注册国际外包市场扫描器
        scanner.add_scanner(Box::new(UpworkScanner::new()));
        scanner
    }
}

// ── 官方 API 连接器 ──────────────────────────────────────────

/// 官方 API 型平台连接器
///
/// 通过平台**官方开放 API** 进行合法合规的需求线索采集。
/// 支持 Token 认证（Bearer / API Key / 自定义 Header）和完整的请求/响应配置。
///
/// ## 配置字段 (`config`)
///
/// | 字段 | 说明 | 默认值 |
/// |------|------|--------|
/// | `api_token` | API 认证 Token（必需） | - |
/// | `auth_type` | 认证方式: `"bearer"` / `"api_key"` / `"custom_header"` | `"bearer"` |
/// | `auth_header` | 自定义认证头名称（`custom_header` 时使用） | `"Authorization"` |
/// | `http_method` | HTTP 方法: `"get"` / `"post"` | `"get"` |
/// | `search_path` | 搜索 API 路径 | `"/api/v1/search"` |
/// | `query_param` | 搜索关键词参数名（GET）或字段名（POST） | `"q"` |
/// | `keyword_field` | 响应中标题字段名 | `"title"` |
/// | `description_field` | 响应中描述字段名 | `"description"` |
/// | `data_wrapper` | 数据包装字段（如 `data`、`results`） | `"data"` |
/// | `timeout_sec` | 请求超时（秒） | `10` |
pub struct ApiMarketplaceScanner {
    platform: String,
    base_url: String,
    api_token: String,
    auth_type: String,
    auth_header: String,
    http_method: String,
    search_path: String,
    query_param: String,
    keyword_field: String,
    description_field: String,
    data_wrapper: String,
    timeout_sec: u64,
}

impl ApiMarketplaceScanner {
    pub fn new(platform: &str, base_url: Option<&str>, config: &serde_json::Value) -> Self {
        Self {
            platform: platform.to_string(),
            base_url: base_url
                .map(|u| u.to_string())
                .unwrap_or_else(|| "https://api.example.com".to_string()),
            api_token: config.get("api_token").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            auth_type: config
                .get("auth_type")
                .and_then(|v| v.as_str())
                .unwrap_or("bearer")
                .to_string(),
            auth_header: config
                .get("auth_header")
                .and_then(|v| v.as_str())
                .unwrap_or("Authorization")
                .to_string(),
            http_method: config
                .get("http_method")
                .and_then(|v| v.as_str())
                .unwrap_or("get")
                .to_string(),
            search_path: config
                .get("search_path")
                .and_then(|v| v.as_str())
                .unwrap_or("/api/v1/search")
                .to_string(),
            query_param: config
                .get("query_param")
                .and_then(|v| v.as_str())
                .unwrap_or("q")
                .to_string(),
            keyword_field: config
                .get("keyword_field")
                .and_then(|v| v.as_str())
                .unwrap_or("title")
                .to_string(),
            description_field: config
                .get("description_field")
                .and_then(|v| v.as_str())
                .unwrap_or("description")
                .to_string(),
            data_wrapper: config
                .get("data_wrapper")
                .and_then(|v| v.as_str())
                .unwrap_or("data")
                .to_string(),
            timeout_sec: config.get("timeout_sec").and_then(|v| v.as_u64()).unwrap_or(10),
        }
    }

    /// 构建认证头
    fn build_auth_header(&self) -> (String, String) {
        match self.auth_type.as_str() {
            "bearer" => ("Authorization".to_string(), format!("Bearer {}", self.api_token)),
            "api_key" => ("X-API-Key".to_string(), self.api_token.clone()),
            "custom_header" => (self.auth_header.clone(), self.api_token.clone()),
            _ => ("Authorization".to_string(), format!("Bearer {}", self.api_token)),
        }
    }
}

#[async_trait]
impl MarketplaceScanner for ApiMarketplaceScanner {
    fn platform(&self) -> String {
        self.platform.clone()
    }

    async fn search(&self, q: &str) -> Result<Vec<RawLead>, String> {
        if self.api_token.is_empty() {
            return Err(format!("[{}] 未配置 API Token，无法调用官方 API", self.platform));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_sec))
            .user_agent("AxAgent/1.0 (demand-discovery)")
            .build()
            .map_err(|e| format!("HTTP 客户端构建失败: {}", e))?;

        let (auth_key, auth_value) = self.build_auth_header();

        let url = format!("{}{}", self.base_url.trim_end_matches('/'), self.search_path);
        tracing::info!(
            platform = self.platform,
            url = %url,
            auth_type = %self.auth_type,
            "[ApiMarketplaceScanner] 发起官方 API 请求"
        );

        // P2-6：方法名大小写不敏感（配置写 "POST" 旧实现会静默降级 GET）
        let resp = if self.http_method.eq_ignore_ascii_case("post") {
            let body = serde_json::json!({
                &self.query_param: q,
            });
            client
                .post(&url)
                .header(&auth_key, &auth_value)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("POST 请求失败: {}", e))?
        } else {
            // P2-1：查询串必须走 encode_query —— 手拼 "?{}={}" 在 q 含
            // `&`/`#`/空格时会被截断或注入额外参数
            let full_url = format!(
                "{}?{}={}",
                url,
                scanner_common::encode_query(&self.query_param),
                scanner_common::encode_query(q)
            );
            client
                .get(&full_url)
                .header(&auth_key, &auth_value)
                .send()
                .await
                .map_err(|e| format!("GET 请求失败: {}", e))?
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::error!(
                platform = self.platform,
                status = %status,
                body = %body,
                "[ApiMarketplaceScanner] API 响应异常"
            );
            return Err(format!(
                "API 返回状态码 {}: {}",
                status,
                body.chars().take(200).collect::<String>()
            ));
        }

        let body = resp.text().await.map_err(|e| format!("响应体读取失败: {}", e))?;

        // P2-2：解析失败必须报错而非吞成空数组 —— 接口挂了应触发重试与
        // 平台失败状态，而不是伪装成"没需求"
        let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            tracing::warn!(
                platform = self.platform,
                error = %e,
                "[ApiMarketplaceScanner] 响应不是合法 JSON"
            );
            format!("响应 JSON 解析失败: {e}")
        })?;

        // 解析响应数据：支持 { "data": [...] } / { "results": [...] } / 直接数组
        let items = if let Some(arr) = parsed.as_array() {
            arr.clone()
        } else if let Some(obj) = parsed.as_object() {
            if let Some(arr) = obj.get(&self.data_wrapper).and_then(|v| v.as_array()) {
                arr.clone()
            } else if let Some(arr) = obj.get("results").and_then(|v| v.as_array()) {
                arr.clone()
            } else {
                tracing::warn!(
                    platform = self.platform,
                    "[ApiMarketplaceScanner] 未找到数据数组字段"
                );
                return Ok(Vec::new());
            }
        } else {
            return Ok(Vec::new());
        };

        let mut leads: Vec<RawLead> = Vec::new();
        for item in &items {
            let title =
                item.get(&self.keyword_field).and_then(|v| v.as_str()).unwrap_or("").to_string();

            if title.is_empty() {
                continue;
            }

            let description = item
                .get(&self.description_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let item_url = item
                .get("url")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("link").and_then(|v| v.as_str()))
                .or_else(|| item.get("source_url").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string();

            // 提取价格：字符串与数字均接受（数字型 price 此前被 as_str 丢弃）
            let price_field = |keys: &[&str]| -> Option<String> {
                keys.iter().find_map(|key| {
                    item.get(*key).and_then(|v| {
                        v.as_str().map(str::to_string).or_else(|| v.as_f64().map(|n| n.to_string()))
                    })
                })
            };
            let price_text = price_field(&["price", "budget", "price_text"]);

            let contact = item
                .get("contact_name")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("contact").and_then(|v| v.as_str()))
                .or_else(|| {
                    // 尝试从 name / author / owner 字段提取
                    item.get("name")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("author").and_then(|v| v.as_str()))
                        .or_else(|| item.get("owner").and_then(|v| v.as_str()))
                })
                .map(|s| s.to_string());

            // 提取邮箱：尝试多个常见字段名
            let contact_email = item
                .get("contact_email")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("email").and_then(|v| v.as_str()))
                .or_else(|| item.get("e-mail").and_then(|v| v.as_str()))
                .or_else(|| item.get("mail").and_then(|v| v.as_str()))
                .map(|s| s.to_string())
                .or_else(|| extract_email_from_text(&description));

            // 提取电话：只接受真正的电话字段；微信号单独归档到快照的
            // contact_wechat（旧实现把 wechat 塞进 phone 字段，语义污染）
            let contact_phone = item
                .get("contact_phone")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("phone").and_then(|v| v.as_str()))
                .or_else(|| item.get("mobile").and_then(|v| v.as_str()))
                .or_else(|| item.get("tel").and_then(|v| v.as_str()))
                .map(|s| s.to_string());

            let mut snapshot = item.clone();
            if let Some(wechat) = item
                .get("wechat")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("weixin").and_then(|v| v.as_str()))
                .filter(|s| !s.trim().is_empty())
                && let Some(obj) = snapshot.as_object_mut()
            {
                obj.entry("contact_wechat".to_string())
                    .or_insert(serde_json::Value::String(wechat.to_string()));
            }

            leads.push(RawLead {
                platform: self.platform.to_string(),
                title,
                description,
                url: item_url,
                price_text,
                contact,
                contact_email,
                contact_phone,
                snapshot,
            });
        }

        tracing::info!(
            platform = self.platform,
            count = leads.len(),
            "[ApiMarketplaceScanner] API 解析完成"
        );

        Ok(leads)
    }
}

// ── Mock / 测试连接器 ────────────────────────────────────────

/// Mock 平台连接器（用于测试和演示，返回固定的模拟数据）
pub struct MockMarketplaceScanner {
    platform: String,
}

impl MockMarketplaceScanner {
    pub fn new(platform: &str) -> Self {
        Self { platform: platform.to_string() }
    }
}

#[async_trait]
impl MarketplaceScanner for MockMarketplaceScanner {
    fn platform(&self) -> String {
        self.platform.clone()
    }

    async fn search(&self, q: &str) -> Result<Vec<RawLead>, String> {
        let mock_data = vec![
            RawLead {
                platform: self.platform.to_string(),
                title: format!("官网建设 - 中小型企业展示型网站 (关键词: {})", q),
                description: "需要一个响应式官网，5-8个页面，包含产品展示、关于我们、联系方式。需要支持移动端。联系邮箱：zhang@example.com".to_string(),
                url: "https://example.com/lead/1".to_string(),
                price_text: Some("8000-15000元".to_string()),
                contact: Some("张经理".to_string()),
                contact_email: Some("zhang@example.com".to_string()),
                contact_phone: Some("13800138000".to_string()),
                snapshot: serde_json::json!({
                    "source": "mock",
                    "category": "web_development",
                    "posted_at": "2026-08-10"
                }),
            },
            RawLead {
                platform: self.platform.to_string(),
                title: format!("Logo 设计 + VI 视觉系统 (关键词: {})", q),
                description: "新品牌需要 Logo 设计，以及完整的 VI 视觉识别系统，包括名片、信封、PPT 模板等。".to_string(),
                url: "https://example.com/lead/2".to_string(),
                price_text: Some("3000-5000元".to_string()),
                contact: Some("李总".to_string()),
                contact_email: Some("li@design-studio.com".to_string()),
                contact_phone: None,
                snapshot: serde_json::json!({
                    "source": "mock",
                    "category": "design",
                    "posted_at": "2026-08-09"
                }),
            },
            RawLead {
                platform: self.platform.to_string(),
                title: format!("微信小程序开发 - 预约系统 (关键词: {})", q),
                // 微信号写在描述文本里，由归一化层提取进快照 contact_wechat，
                // 不再塞进 phone 字段（微信号不是电话）
                description: "开发一个微信小程序，用户可以在线预约服务、查看订单、支付。管理员后台管理预约。微信号：wangzhuren_biz".to_string(),
                url: "https://example.com/lead/3".to_string(),
                price_text: Some("20000-30000元".to_string()),
                contact: Some("王主任".to_string()),
                contact_email: None,
                contact_phone: None,
                snapshot: serde_json::json!({
                    "source": "mock",
                    "category": "mini_program",
                    "posted_at": "2026-08-11"
                }),
            },
        ];

        Ok(mock_data)
    }
}

// ── 手动补录连接器 ──────────────────────────────────────────

/// 手动补录连接器（最轻路径）
///
/// 当平台暂未接入官方 API 时，使用此连接器返回空结果，
/// 引导用户通过 `opc_create_lead` 手动录入需求线索。
pub struct ManualMarketplaceScanner {
    platform: String,
    base_url: Option<String>,
}

impl ManualMarketplaceScanner {
    pub fn new(platform: &str, base_url: Option<&str>, _config: &serde_json::Value) -> Self {
        Self { platform: platform.to_string(), base_url: base_url.map(|u| u.to_string()) }
    }
}

#[async_trait]
impl MarketplaceScanner for ManualMarketplaceScanner {
    fn platform(&self) -> String {
        self.platform.clone()
    }

    async fn search(&self, q: &str) -> Result<Vec<RawLead>, String> {
        tracing::info!(
            platform = self.platform,
            base_url = self.base_url.as_deref().unwrap_or("-"),
            query = q,
            "[ManualMarketplaceScanner] 请在 OPC 需求发现面板手动录入需求线索"
        );
        Ok(Vec::new())
    }
}

// ── 工具函数 ──────────────────────────────────────────────────

/// 从价格文本解析预算区间与币种
///
/// 支持的格式（见 [`scanner_common::parse_price_range`]）：
/// `"8000-15000元"` / `"¥500"` / `"2万"` / 数字字符串。
/// 无法解析时预算为 None（评分引擎回退 [`BUDGET_FALLBACK_SCORE`]）。
fn parse_budget_info(price_text: Option<&str>) -> (Option<f64>, Option<f64>, String) {
    let Some(text) = price_text.map(str::trim).filter(|t| !t.is_empty()) else {
        return (None, None, "CNY".to_string());
    };
    let currency = if text.contains('$') || text.to_lowercase().contains("usd") {
        "USD"
    } else {
        "CNY"
    };
    match scanner_common::parse_price_range(text) {
        Some((min, max)) => (Some(min), Some(max), currency.to_string()),
        None => (None, None, "CNY".to_string()),
    }
}

/// 从文本中提取邮箱地址
///
/// 简单兜底手段，主要依赖 API 返回的结构化字段。
fn extract_email_from_text(text: &str) -> Option<String> {
    scanner_common::extract_email_from_text(text)
}

/// 判断错误是否为网络环境问题（可重试）
///
/// 覆盖两类环境性失败：
/// - 网络层错误（连接失败、DNS 解析失败、超时等，来自 reqwest 等）
/// - HTTP 限流/服务端错误（429 限流、403 拒绝、5xx 临时故障）
///
/// 两个消费方：
/// - [`search_with_retry`]：仅网络类错误才重试，401/400/参数错误等确定性
///   失败立即返回，避免放大无效请求（P2-3）
/// - 网络集成测试：「离线/CI 网络不可达或服务端限流时跳过」
///
/// 注意：不匹配 4xx 中的其他错误（如 400），避免掩盖请求构造类的真实逻辑缺陷。
pub(crate) fn is_network_env_error(err: &str) -> bool {
    let err_lower = err.to_lowercase();

    // 网络层错误
    if err_lower.contains("connection")
        || err_lower.contains("dns")
        || err_lower.contains("timed out")
        || err_lower.contains("error sending request")
        || err_lower.contains("timeout")
    {
        return true;
    }

    // HTTP 限流 / 服务端临时错误（各平台错误格式略有差异，如「状态码 429」/「状态码: 429」/「status 429」）
    [
        "状态码 429",
        "状态码: 429",
        "status 429",
        "status: 429",
        "状态码 403",
        "状态码: 403",
        "status 403",
        "status: 403",
        "状态码 5",
        "状态码: 5",
        "status 5",
        "status: 5",
    ]
    .iter()
    .any(|pattern| err_lower.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_aggregate_scanner() {
        let scanner = AggregateMarketplaceScanner::new();
        assert!(scanner.disabled_platforms.is_empty());
    }

    #[test]
    fn test_is_network_env_error() {
        // 网络层错误（reqwest 等）
        assert!(is_network_env_error("ArXiv API 请求失败: error sending request for url"));
        assert!(is_network_env_error("连接失败: connection reset by peer"));
        assert!(is_network_env_error("DNS 解析失败"));
        assert!(is_network_env_error("请求超时: operation timed out"));

        // HTTP 限流 / 服务端临时错误（CI 数据中心 IP 常见）
        assert!(is_network_env_error("ArXiv API 返回状态码 429"));
        assert!(is_network_env_error("HN Algolia API 返回状态码: 429"));
        assert!(is_network_env_error("API 返回状态码 503: Service Unavailable"));
        assert!(is_network_env_error("ArXiv API 返回状态码 403"));

        // 真实逻辑错误不应被跳过
        assert!(!is_network_env_error("ArXiv API 返回状态码 400"));
        assert!(!is_network_env_error("未配置 API Token，无法调用官方 API"));
        assert!(!is_network_env_error("响应解析失败: unexpected token"));
    }

    #[test]
    fn test_disable_and_enable_scanner() {
        let mut scanner = AggregateMarketplaceScanner::new();
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("test_platform")));

        // 默认启用
        assert!(scanner.is_scanner_enabled("test_platform"));

        // 禁用
        scanner.disable_scanner("test_platform");
        assert!(!scanner.is_scanner_enabled("test_platform"));

        // 重新启用
        scanner.enable_scanner("test_platform");
        assert!(scanner.is_scanner_enabled("test_platform"));
    }

    #[test]
    fn test_list_scanners_status() {
        let mut scanner = AggregateMarketplaceScanner::new();
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("platform_a")));
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("platform_b")));

        // 禁用 platform_a
        scanner.disable_scanner("platform_a");

        let status = scanner.list_scanners();
        assert_eq!(status.len(), 2);

        let platform_a_status = status.iter().find(|(p, _)| p == "platform_a");
        assert!(platform_a_status.is_some());
        assert!(!platform_a_status.unwrap().1); // disabled

        let platform_b_status = status.iter().find(|(p, _)| p == "platform_b");
        assert!(platform_b_status.is_some());
        assert!(platform_b_status.unwrap().1); // enabled
    }

    #[tokio::test]
    async fn test_search_all_skips_disabled_scanners() {
        let mut scanner = AggregateMarketplaceScanner::new();
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("enabled_platform")));
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("disabled_platform")));

        // 禁用一个扫描器
        scanner.disable_scanner("disabled_platform");

        // 搜索
        let results = scanner.search_all("test").await.unwrap();

        // 应该只包含 enabled_platform 的结果
        assert!(results.iter().all(|r| r.platform != "disabled_platform"));
        assert!(results.iter().any(|r| r.platform == "enabled_platform"));
    }

    #[tokio::test]
    async fn test_search_all_without_disabled_scanners() {
        let mut scanner = AggregateMarketplaceScanner::new();
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("platform_a")));

        // 不禁用任何扫描器
        let results = scanner.search_all("test").await.unwrap();

        // 应该包含 platform_a 的结果
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.platform == "platform_a"));
    }

    #[test]
    fn test_disable_nonexistent_platform() {
        let mut scanner = AggregateMarketplaceScanner::new();
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("test_platform")));

        // 禁用不存在的平台不应报错
        scanner.disable_scanner("nonexistent_platform");

        // 原平台仍应启用
        assert!(scanner.is_scanner_enabled("test_platform"));
    }

    #[tokio::test]
    async fn test_search_and_evaluate() {
        let mut scanner = AggregateMarketplaceScanner::new();
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("test")));

        let results = scanner.search_and_evaluate("test").await.unwrap();

        assert!(!results.is_empty(), "应返回评估后的线索");
        for evaluated in &results {
            assert!(evaluated.value_score() >= 0.0 && evaluated.value_score() <= 100.0);
            assert!(!evaluated.opportunity_level().is_empty());
        }

        // 验证已按价值分排序
        for i in 0..results.len().saturating_sub(1) {
            assert!(results[i].value_score() >= results[i + 1].value_score());
        }
    }

    #[tokio::test]
    async fn test_search_high_value() {
        let mut scanner = AggregateMarketplaceScanner::new();
        scanner.add_scanner(Box::new(MockMarketplaceScanner::new("test")));

        let all = scanner.search_and_evaluate("test").await.unwrap();
        assert!(!all.is_empty());

        let results = scanner.search_high_value("test", 0.0).await.unwrap();
        assert_eq!(results.len(), all.len(), "阈值为0时应返回所有结果");

        // 过滤语义：value_score >= min_score
        let max_score = all.iter().fold(0.0_f64, |m, e| m.max(e.value_score()));
        let results = scanner.search_high_value("test", max_score).await.unwrap();
        assert!(!results.is_empty(), "阈值等于最高分时应至少保留该条");

        let results = scanner.search_high_value("test", 100.0).await.unwrap();
        assert!(results.is_empty(), "阈值为100时应无结果（模拟数据达不到满分）");
    }

    #[test]
    fn test_evaluate_leads() {
        let scanner = AggregateMarketplaceScanner::new();

        let leads = vec![DemandLead {
            id: "test-1".to_string(),
            platform: "test".to_string(),
            title: "高价值需求".to_string(),
            description: "这是一个非常紧急且昂贵的痛点问题".to_string(),
            budget_min: None,
            budget_max: None,
            budget_currency: "CNY".to_string(),
            contact_name: None,
            contact_email: None,
            contact_phone: None,
            source_url: None,
            raw_snapshot: serde_json::Value::Null,
            status: "new".to_string(),
            confidence: 0.0,
            content_fingerprint: None,
        }];

        let evaluated = scanner.evaluate_leads(leads);
        assert_eq!(evaluated.len(), 1);
        assert!(evaluated[0].value_score() >= 0.0);
    }

    #[test]
    fn test_evaluated_demand_lead() {
        let lead = DemandLead {
            id: "test".to_string(),
            platform: "test".to_string(),
            title: "Test".to_string(),
            description: "Description".to_string(),
            budget_min: None,
            budget_max: None,
            budget_currency: "CNY".to_string(),
            contact_name: None,
            contact_email: None,
            contact_phone: None,
            source_url: None,
            raw_snapshot: serde_json::Value::Null,
            status: "new".to_string(),
            confidence: 0.0,
            content_fingerprint: None,
        };

        let evaluation = evaluate_lead(&lead);

        let evaluated = EvaluatedDemandLead::new(lead, evaluation);
        assert!(evaluated.value_score() >= 0.0);
        assert!(!evaluated.opportunity_level().is_empty());
    }

    /// 回归测试（P0-1）：真实 lead 评估 vs 旧「假 lead」评估分数必须不同。
    ///
    /// 旧实现把 lead 拆成 (id, title, desc) 再重建 budget=None / snapshot=Null
    /// 的假 lead，预算与热度因子恒为兜底值，全库分数被压到 ≤59，
    /// very_high(≥80)/high(≥60) 档与订阅推送（min_score=60）永不可达。
    #[test]
    fn test_evaluate_lead_uses_budget_and_engagement() {
        let rich_lead = DemandLead {
            id: "rich".to_string(),
            platform: "test".to_string(),
            title: "急需 崩溃 deadline 企业官网开发".to_string(),
            description: "项目延期，预算 10 万，紧急找团队".to_string(),
            budget_min: Some(80_000.0),
            budget_max: Some(100_000.0),
            budget_currency: "CNY".to_string(),
            contact_name: None,
            contact_email: None,
            contact_phone: None,
            source_url: None,
            raw_snapshot: serde_json::json!({ "score": 1000 }),
            status: "new".to_string(),
            confidence: 0.0,
            content_fingerprint: None,
        };
        let bare_lead = DemandLead {
            budget_min: None,
            budget_max: None,
            raw_snapshot: serde_json::Value::Null,
            ..rich_lead.clone()
        };

        let rich_score = evaluate_lead(&rich_lead).commercial_value_score();
        let bare_score = evaluate_lead(&bare_lead).commercial_value_score();

        // 预算与热度因子真实生效：富 lead 分数显著高于裸 lead
        assert!(rich_score > bare_score, "rich={rich_score} 应大于 bare={bare_score}");
        // 富 lead 应可达到 high 档（≥60）—— 旧实现全库上限 59，永不可达
        assert!(rich_score >= 60.0, "富 lead 分数 {rich_score} 应达到 high 档(≥60)");
    }

    /// 回归测试（P0-2）：price_text 在归一化时解析为预算，不再丢弃
    #[test]
    fn test_new_from_raw_parses_price_text() {
        let raw = RawLead {
            platform: "test".to_string(),
            title: "定制开发".to_string(),
            description: "需要一个小程序".to_string(),
            url: "https://example.com".to_string(),
            price_text: Some("8000-15000元".to_string()),
            contact: None,
            contact_email: None,
            contact_phone: None,
            snapshot: serde_json::Value::Null,
        };
        let lead = DemandLead::new_from_raw(raw);
        assert_eq!(lead.budget_min, Some(8000.0));
        assert_eq!(lead.budget_max, Some(15000.0));
        assert_eq!(lead.budget_currency, "CNY");

        // 单值价格
        let raw = RawLead {
            price_text: Some("¥500".to_string()),
            ..RawLead {
                platform: "test".to_string(),
                title: "t".to_string(),
                description: String::new(),
                url: String::new(),
                price_text: None,
                contact: None,
                contact_email: None,
                contact_phone: None,
                snapshot: serde_json::Value::Null,
            }
        };
        let lead = DemandLead::new_from_raw(raw);
        assert_eq!(lead.budget_min, Some(500.0));
        assert_eq!(lead.budget_max, Some(500.0));
    }

    /// 回归测试（P0-4）：归一化时对 title+description 兜底提取联系方式；
    /// 微信号单独归档，不进 phone 字段
    #[test]
    fn test_new_from_raw_extracts_contacts() {
        let raw = RawLead {
            platform: "test".to_string(),
            title: "找人做官网".to_string(),
            description: "请联系 zhang@example.com，电话 13800138000，微信：wang_biz123"
                .to_string(),
            url: "https://example.com".to_string(),
            price_text: None,
            contact: None,
            contact_email: None,
            contact_phone: None,
            snapshot: serde_json::Value::Null,
        };
        let lead = DemandLead::new_from_raw(raw);
        assert_eq!(lead.contact_email.as_deref(), Some("zhang@example.com"));
        assert_eq!(lead.contact_phone.as_deref(), Some("13800138000"));
        let wechat =
            lead.raw_snapshot.get("contact_wechat").and_then(|v| v.as_str()).unwrap_or_default();
        assert_eq!(wechat, "wang_biz123");
    }

    /// 回归测试（P0-5）：相同内容的线索生成稳定指纹，不同内容指纹不同
    #[test]
    fn test_new_from_raw_content_fingerprint() {
        let make = |title: &str, desc: &str, url: &str| {
            DemandLead::new_from_raw(RawLead {
                platform: "test".to_string(),
                title: title.to_string(),
                description: desc.to_string(),
                url: url.to_string(),
                price_text: None,
                contact: None,
                contact_email: None,
                contact_phone: None,
                snapshot: serde_json::Value::Null,
            })
        };
        // 同内容不同 URL（换链接重发）→ 指纹一致，可被去重
        let a = make("求购二手相机", "成色好", "https://a.com/1");
        let b = make("求购二手相机", "成色好", "https://a.com/2");
        assert_eq!(a.content_fingerprint, b.content_fingerprint);
        assert!(a.content_fingerprint.is_some());
        // 不同内容 → 指纹不同
        let c = make("出售二手相机", "成色好", "https://a.com/1");
        assert_ne!(a.content_fingerprint, c.content_fingerprint);
        // 空内容 → 无指纹（不参与去重）
        let empty = make("", "", "https://a.com/3");
        assert!(empty.content_fingerprint.is_none());
    }

    #[test]
    fn test_extract_email_from_text() {
        // 测试标准邮箱格式
        let text = "请联系我：test@example.com 获取详细信息";
        let email = extract_email_from_text(text);
        assert_eq!(email, Some("test@example.com".to_string()));

        // 测试多个邮箱时只提取第一个
        let text = "联系 a@b.com 或 c@d.org";
        let email = extract_email_from_text(text);
        assert_eq!(email, Some("a@b.com".to_string()));

        // 测试无邮箱的情况
        let text = "这是一段没有邮箱的文本";
        let email = extract_email_from_text(text);
        assert!(email.is_none());

        // 测试带特殊字符的邮箱
        let text = "我的邮箱是 user.name+tag@domain.co.uk";
        let email = extract_email_from_text(text);
        assert!(email.is_some());
    }

    #[test]
    fn test_contact_info_in_demand_lead() {
        // 测试 RawLead 到 DemandLead 的联系方式转换
        let raw = RawLead {
            platform: "test".to_string(),
            title: "Test".to_string(),
            description: "Description".to_string(),
            url: "https://example.com".to_string(),
            price_text: None,
            contact: Some("张三".to_string()),
            contact_email: Some("zhangsan@example.com".to_string()),
            contact_phone: Some("13800000000".to_string()),
            snapshot: serde_json::Value::Null,
        };

        let lead = DemandLead::new_from_raw(raw);
        assert_eq!(lead.contact_name, Some("张三".to_string()));
        assert_eq!(lead.contact_email, Some("zhangsan@example.com".to_string()));
        assert_eq!(lead.contact_phone, Some("13800000000".to_string()));
    }

    /// P1-3 回归：分类按命中数择优，平分才按规则表优先级
    #[test]
    fn test_classify_demand_prefers_most_hits() {
        use super::{DemandType, classify_demand};
        // 平分（外包 1 : 开发 1）→ 规则表优先级，Outsourcing 在前
        assert_eq!(classify_demand("外包开发一套系统"), DemandType::Outsourcing);
        // 旧实现会把这句错分为 Outsourcing（单关键词短路 + 外包排最前）
        assert_eq!(classify_demand("定制开发一个小程序"), DemandType::Development);
        assert_eq!(classify_demand("需要一个 ui 设计和 logo 设计"), DemandType::Design);
        assert_eq!(classify_demand("全新未拆封商品"), DemandType::Unknown);
    }

    /// P2-3 回归：非网络类错误不重试，网络类错误重试至 retry_max
    struct FailingScanner {
        err: String,
    }

    #[async_trait::async_trait]
    impl MarketplaceScanner for FailingScanner {
        fn platform(&self) -> String {
            "failing".to_string()
        }
        async fn search(&self, _q: &str) -> Result<Vec<RawLead>, String> {
            Err(self.err.clone())
        }
    }

    #[tokio::test]
    async fn test_search_with_retry_skips_non_network_errors() {
        let policy = ScanPolicy { retry_max: 2, ..ScanPolicy::default() };

        let s = FailingScanner { err: "API 返回状态码 401: Unauthorized".to_string() };
        let (leads, err, attempts, skipped) = search_with_retry(&s, "q", &policy).await;
        assert!(leads.is_empty());
        assert!(err.is_some());
        assert!(!skipped);
        assert_eq!(attempts, 1, "非网络类错误不应重试");

        let s = FailingScanner { err: "API 返回状态码 429: Too Many Requests".to_string() };
        let (_, _, attempts, _) = search_with_retry(&s, "q", &policy).await;
        assert_eq!(attempts, 3, "网络类错误应重试至 retry_max（1 次首发 + 2 次重试）");
    }
}
