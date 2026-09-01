// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现数据访问层（v131）
//!
//! 平台配置 CRUD + 需求线索持久化。评估逻辑在 `axagent_tools`
//! 的 `marketplace_scanner`，本模块只做存储与查询。

use sea_orm::*;

use axagent_entities::{opc_demand_leads, opc_demand_platforms, opc_demand_subscriptions};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{
    DemandLeadDto, DemandPlatform, DemandSubscription, SaveDemandPlatformInput,
    SaveDemandSubscriptionInput,
};
use axagent_harness::util_fns::{gen_id, now_ts};

// ── 实体 ↔ DTO 转换 ──────────────────────────────────────────

fn platform_from_entity(m: opc_demand_platforms::Model) -> DemandPlatform {
    DemandPlatform {
        id: m.id,
        name: m.name,
        platform_type: m.platform_type,
        enabled: m.enabled != 0,
        base_url: m.base_url,
        config: serde_json::from_str(&m.config_json).unwrap_or(serde_json::Value::Null),
        last_sync_at: m.last_sync_at,
        status: m.status,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

fn subscription_from_entity(m: opc_demand_subscriptions::Model) -> DemandSubscription {
    DemandSubscription {
        id: m.id,
        keyword: m.keyword,
        enabled: m.enabled != 0,
        interval_hours: m.interval_hours,
        min_score: m.min_score,
        platforms: serde_json::from_str(&m.platforms_json).unwrap_or_default(),
        last_scanned_at: m.last_scanned_at,
        last_hit_count: m.last_hit_count,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

fn lead_from_entity(m: opc_demand_leads::Model) -> DemandLeadDto {
    let opportunity_level = match m.commercial_value_score {
        v if v >= 80.0 => "very_high",
        v if v >= 60.0 => "high",
        v if v >= 40.0 => "medium",
        _ => "low",
    };
    DemandLeadDto {
        id: m.id,
        platform: m.platform,
        title: m.title,
        description: m.description,
        budget_min: m.budget_min,
        budget_max: m.budget_max,
        budget_currency: m.budget_currency,
        contact_name: m.contact_name,
        contact_email: m.contact_email,
        contact_phone: m.contact_phone,
        source_url: m.source_url,
        status: m.status,
        confidence: m.confidence,
        pain_score: m.pain_score,
        market_gap_score: m.market_gap_score,
        commercial_value_score: m.commercial_value_score,
        opportunity_level: opportunity_level.to_string(),
        demand_type: m.demand_type,
        linked_workflow_id: m.linked_workflow_id,
        implemented_at: m.implemented_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

// ── 平台配置 ─────────────────────────────────────────────────

/// 内置默认平台清单（与前端 mock preset 对齐；platform_type 一律 "scanner"）
///
/// 三元组：`(id, name, 默认启用)`。数据源治理（P1-1）：真正默认可用的
/// 免费源只有 8 个（HN/GitHub×2/SO/arXiv/HF/package_ecosystem + Reddit
/// 观察名单）；其余 10 个无公开检索 API 或需凭证（Twitter/LinkedIn/CSDN/
/// 掘金/Dribbble/ProductHunt/Upwork/知乎/猪八戒/闲鱼），默认禁用 ——
/// 否则每轮各占并发额度并刷"合规跳过"状态。配置了 api_token 后可手动启用。
pub const DEFAULT_PLATFORMS: &[(&str, &str, bool)] = &[
    ("reddit", "Reddit", true),
    ("hackernews", "HackerNews", true),
    ("github_issue", "GitHub Issues", true),
    ("github_discussion", "GitHub Discussions", true),
    ("stackoverflow", "StackOverflow", true),
    ("producthunt", "Product Hunt", false),
    ("huggingface", "HuggingFace", true),
    ("package_ecosystem", "Package Ecosystem", true),
    ("arxiv", "arXiv", true),
    ("twitter", "Twitter/X", false),
    ("zhubajie", "猪八戒", false),
    ("xianyu", "闲鱼", false),
    ("linkedin", "LinkedIn", false),
    ("zhihu", "知乎", false),
    ("csdn", "CSDN", false),
    ("juejin", "掘金", false),
    ("dribbble", "Dribbble", false),
    ("upwork", "Upwork", false),
];

/// 平台表为空时插入内置默认平台（幂等：只在空表时执行）
pub async fn seed_default_platforms_if_empty(db: &DatabaseConnection) -> Result<()> {
    let count = opc_demand_platforms::Entity::find().count(db).await?;
    if count > 0 {
        return Ok(());
    }
    let now = now_ts();
    for (id, name, default_enabled) in DEFAULT_PLATFORMS {
        let row = opc_demand_platforms::ActiveModel {
            id: Set(String::from(*id)),
            name: Set(String::from(*name)),
            platform_type: Set("scanner".to_string()),
            enabled: Set(i32::from(*default_enabled)),
            base_url: Set(None),
            config_json: Set(serde_json::json!({
                "description": if *default_enabled {
                    format!("{} 扫描器", name)
                } else {
                    format!("{} 扫描器（无公开检索 API 或需官方凭证，配置 Token 后启用）", name)
                },
                "auto_sync": true,
            })
            .to_string()),
            last_sync_at: Set(None),
            status: Set("idle".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        row.insert(db).await?;
    }
    tracing::info!(count = DEFAULT_PLATFORMS.len(), "[opc_demand] 已填充默认平台配置");
    Ok(())
}

/// 列出全部平台配置
pub async fn list_platforms(db: &DatabaseConnection) -> Result<Vec<DemandPlatform>> {
    let rows = opc_demand_platforms::Entity::find()
        .order_by_asc(opc_demand_platforms::Column::Id)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(platform_from_entity).collect())
}

/// 新增或更新平台配置（`id` 为空则新增，否则部分更新）
pub async fn save_platform(
    db: &DatabaseConnection,
    input: SaveDemandPlatformInput,
) -> Result<DemandPlatform> {
    let now = now_ts();
    match input.id {
        Some(id) => {
            let existing = opc_demand_platforms::Entity::find_by_id(&id)
                .one(db)
                .await?
                .ok_or_else(|| AxAgentError::Internal(format!("平台不存在: {}", id)))?;

            let mut am: opc_demand_platforms::ActiveModel = existing.into();
            if let Some(name) = input.name {
                am.name = Set(name);
            }
            if let Some(pt) = input.platform_type {
                am.platform_type = Set(pt);
            }
            if let Some(enabled) = input.enabled {
                am.enabled = Set(if enabled { 1 } else { 0 });
            }
            if let Some(url) = input.base_url {
                am.base_url = Set(if url.is_empty() { None } else { Some(url) });
            }
            if let Some(config) = input.config {
                am.config_json = Set(config.to_string());
            }
            am.updated_at = Set(now);
            let saved = am.update(db).await?;
            Ok(platform_from_entity(saved))
        },
        None => {
            let id = format!("mp-{}", gen_id());
            let am = opc_demand_platforms::ActiveModel {
                id: Set(id.clone()),
                name: Set(input.name.unwrap_or_else(|| "新平台".to_string())),
                platform_type: Set(input.platform_type.unwrap_or_else(|| "manual".to_string())),
                enabled: Set(if input.enabled.unwrap_or(true) { 1 } else { 0 }),
                base_url: Set(input.base_url.filter(|u| !u.is_empty())),
                config_json: Set(input
                    .config
                    .unwrap_or(serde_json::Value::Object(Default::default()))
                    .to_string()),
                last_sync_at: Set(None),
                status: Set("idle".to_string()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            let saved = am.insert(db).await?;
            Ok(platform_from_entity(saved))
        },
    }
}

/// 删除平台配置
pub async fn delete_platform(db: &DatabaseConnection, id: &str) -> Result<()> {
    opc_demand_platforms::Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

/// 扫描结束后更新平台同步状态
pub async fn mark_platform_synced(
    db: &DatabaseConnection,
    platform_id: &str,
    ok: bool,
) -> Result<()> {
    if let Some(existing) = opc_demand_platforms::Entity::find_by_id(platform_id).one(db).await? {
        let mut am: opc_demand_platforms::ActiveModel = existing.into();
        am.last_sync_at = Set(Some(now_ts()));
        am.status = Set(if ok {
            "ok".to_string()
        } else {
            "error".to_string()
        });
        am.updated_at = Set(now_ts());
        am.update(db).await?;
    }
    Ok(())
}

/// 列出启用的平台配置
pub async fn list_enabled_platforms(db: &DatabaseConnection) -> Result<Vec<DemandPlatform>> {
    let rows = opc_demand_platforms::Entity::find()
        .filter(opc_demand_platforms::Column::Enabled.eq(1))
        .all(db)
        .await?;
    Ok(rows.into_iter().map(platform_from_entity).collect())
}

// ── 需求线索 ─────────────────────────────────────────────────

/// 新线索写入入库参数（扫描 + 评估的结果）
#[derive(Debug, Clone)]
pub struct NewLeadRow {
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
    /// 内容指纹（标题+描述归一化哈希，v136）：去重主键
    pub content_fingerprint: Option<String>,
    pub raw_snapshot: serde_json::Value,
    pub confidence: f64,
    pub pain_score: f64,
    pub market_gap_score: f64,
    pub commercial_value_score: f64,
    pub demand_type: String,
}

/// 线索写入结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadWriteOutcome {
    /// 新入库
    Inserted,
    /// 命中去重窗口**外**的同源线索：已刷新内容与评分（不新增行）
    Refreshed,
    /// 命中去重窗口**内**的同源线索：判定为同一轮需求的重复曝光，跳过
    Skipped,
}

/// 写入一条线索，按去重时间窗口决定「插入 / 刷新 / 跳过」
///
/// 去重主键是**唯一索引** `(platform, content_fingerprint)`（v136）：指纹只看
/// 标题+描述内容，与 URL 无关 —— 同一条需求换个链接重发会被识别，而共享
/// 同一搜索页 URL 的不同线索（闲鱼等）各自成立。旧键 `(platform, source_url)`
/// 已废弃（唯一索引随 v136 删除），仅对无指纹的存量/空内容线索做兜底查重。
///
/// 窗口语义（作用于刷新/跳过判定，与键选择无关）：
/// - `window_secs = None`：永久去重，只要库里存在同指纹线索就跳过
/// - `window_secs = Some(s)`：仅当既有行的 `created_at` 落在 `[now - s, now]` 内才跳过
pub async fn upsert_lead_within_window(
    db: &DatabaseConnection,
    row: NewLeadRow,
    window_secs: Option<i64>,
) -> Result<LeadWriteOutcome> {
    // 去重查找：内容指纹优先（与唯一索引对齐）；无指纹时按 URL 兜底
    // （存量行指纹为 NULL，兜底可继续抑制明显的同 URL 重复刷新）
    let dup = if let Some(fp) = &row.content_fingerprint {
        opc_demand_leads::Entity::find()
            .filter(opc_demand_leads::Column::Platform.eq(&row.platform))
            .filter(opc_demand_leads::Column::ContentFingerprint.eq(fp))
            .one(db)
            .await?
    } else if let Some(url) = &row.source_url {
        opc_demand_leads::Entity::find()
            .filter(opc_demand_leads::Column::Platform.eq(&row.platform))
            .filter(opc_demand_leads::Column::SourceUrl.eq(url))
            .one(db)
            .await?
    } else {
        None
    };

    if let Some(existing) = dup {
        let within_window = match window_secs {
            None => true,
            Some(secs) => existing.created_at >= now_ts() - secs,
        };
        if within_window {
            return Ok(LeadWriteOutcome::Skipped);
        }

        let now = now_ts();
        let mut am: opc_demand_leads::ActiveModel = existing.into();
        am.title = Set(row.title);
        am.description = Set(row.description);
        am.budget_min = Set(row.budget_min);
        am.budget_max = Set(row.budget_max);
        am.budget_currency = Set(row.budget_currency);
        am.contact_name = Set(row.contact_name);
        am.contact_email = Set(row.contact_email);
        am.contact_phone = Set(row.contact_phone);
        // 指纹线索可能换了落地页 URL，一并刷新
        if let Some(fp) = row.content_fingerprint {
            am.content_fingerprint = Set(Some(fp));
        }
        am.source_url = Set(row.source_url);
        am.confidence = Set(row.confidence);
        am.pain_score = Set(row.pain_score);
        am.market_gap_score = Set(row.market_gap_score);
        am.commercial_value_score = Set(row.commercial_value_score);
        am.demand_type = Set(row.demand_type);
        am.raw_snapshot = Set(row.raw_snapshot.to_string());
        am.updated_at = Set(now);
        am.update(db).await?;
        return Ok(LeadWriteOutcome::Refreshed);
    }

    let now = now_ts();
    let am = opc_demand_leads::ActiveModel {
        id: Set(row.id),
        platform: Set(row.platform),
        title: Set(row.title),
        description: Set(row.description),
        budget_min: Set(row.budget_min),
        budget_max: Set(row.budget_max),
        budget_currency: Set(row.budget_currency),
        contact_name: Set(row.contact_name),
        contact_email: Set(row.contact_email),
        contact_phone: Set(row.contact_phone),
        source_url: Set(row.source_url),
        content_fingerprint: Set(row.content_fingerprint),
        raw_snapshot: Set(row.raw_snapshot.to_string()),
        status: Set("new".to_string()),
        confidence: Set(row.confidence),
        pain_score: Set(row.pain_score),
        market_gap_score: Set(row.market_gap_score),
        commercial_value_score: Set(row.commercial_value_score),
        demand_type: Set(row.demand_type),
        linked_workflow_id: Set(None),
        implemented_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    am.insert(db).await?;
    Ok(LeadWriteOutcome::Inserted)
}

/// 手动录入需求线索（P1-4：此前「手动补录」只有提示日志，`opc_create_lead`
/// 命令与前端入口均不存在）
///
/// 复用 [`upsert_lead_within_window`] 的去重语义：窗口内同指纹 → Skipped
/// （读回既有行返回），窗口外 → Refreshed，全新 → Inserted。始终返回**生效行**
/// 的 DTO —— 手动重复录入时前端能看到已存在的线索而不是报错。
pub async fn create_manual_lead(
    db: &DatabaseConnection,
    row: NewLeadRow,
    window_secs: Option<i64>,
) -> Result<DemandLeadDto> {
    let outcome = upsert_lead_within_window(db, row.clone(), window_secs).await?;
    let found = match outcome {
        LeadWriteOutcome::Inserted => opc_demand_leads::Entity::find_by_id(row.id).one(db).await?,
        LeadWriteOutcome::Refreshed | LeadWriteOutcome::Skipped => {
            // 生效行是既有行：按与 upsert 相同的键读回（指纹优先，URL 兜底）
            let by_fingerprint = if let Some(fp) = &row.content_fingerprint {
                opc_demand_leads::Entity::find()
                    .filter(opc_demand_leads::Column::Platform.eq(&row.platform))
                    .filter(opc_demand_leads::Column::ContentFingerprint.eq(fp))
                    .one(db)
                    .await?
            } else {
                None
            };
            match by_fingerprint {
                Some(m) => Some(m),
                None => match &row.source_url {
                    Some(url) if !url.is_empty() => {
                        opc_demand_leads::Entity::find()
                            .filter(opc_demand_leads::Column::Platform.eq(&row.platform))
                            .filter(opc_demand_leads::Column::SourceUrl.eq(url))
                            .one(db)
                            .await?
                    },
                    _ => None,
                },
            }
        },
    };
    found
        .map(lead_from_entity)
        .ok_or_else(|| AxAgentError::Internal("线索入库后无法读回".to_string()))
}

/// 按商业价值分降序列出线索（可按生命周期状态过滤）
pub async fn list_leads(
    db: &DatabaseConnection,
    limit: u64,
    min_score: Option<f64>,
    status: Option<String>,
) -> Result<Vec<DemandLeadDto>> {
    let mut select = opc_demand_leads::Entity::find();
    if let Some(min) = min_score {
        select = select.filter(opc_demand_leads::Column::CommercialValueScore.gte(min));
    }
    if let Some(st) = status {
        select = select.filter(opc_demand_leads::Column::Status.eq(st));
    }
    let rows = select
        .order_by_desc(opc_demand_leads::Column::CommercialValueScore)
        .order_by_desc(opc_demand_leads::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(lead_from_entity).collect())
}

/// 按 ID 批量读取线索（回填本轮扫描明细用，保持分数降序）
pub async fn list_leads_by_ids(
    db: &DatabaseConnection,
    ids: &[String],
) -> Result<Vec<DemandLeadDto>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = opc_demand_leads::Entity::find()
        .filter(opc_demand_leads::Column::Id.is_in(ids.to_vec()))
        .order_by_desc(opc_demand_leads::Column::CommercialValueScore)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(lead_from_entity).collect())
}

/// 读取单条线索
pub async fn get_lead(db: &DatabaseConnection, lead_id: &str) -> Result<DemandLeadDto> {
    let row = opc_demand_leads::Entity::find_by_id(lead_id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("需求线索不存在: {}", lead_id)))?;
    Ok(lead_from_entity(row))
}

/// 线索生命周期合法迁移表
///
/// `new → evaluated → contacted → won/lost`；`new/evaluated` 可直达 `lost`。
/// `won` / `lost` 为终态，同状态重复设置视为幂等成功。
fn is_legal_status_transition(from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    matches!(
        (from, to),
        ("new", "evaluated")
            | ("new", "contacted")
            | ("new", "lost")
            | ("evaluated", "contacted")
            | ("evaluated", "lost")
            | ("contacted", "won")
            | ("contacted", "lost")
    )
}

/// 更新线索生命周期状态（非法迁移报 Validation 错误）
pub async fn update_lead_status(
    db: &DatabaseConnection,
    lead_id: &str,
    new_status: &str,
) -> Result<DemandLeadDto> {
    const VALID_STATUSES: &[&str] = &["new", "evaluated", "contacted", "won", "lost"];
    if !VALID_STATUSES.contains(&new_status) {
        return Err(AxAgentError::Validation(format!(
            "非法状态值: {new_status}（合法值: new/evaluated/contacted/won/lost）"
        )));
    }

    let existing = opc_demand_leads::Entity::find_by_id(lead_id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("需求线索不存在: {}", lead_id)))?;

    if !is_legal_status_transition(&existing.status, new_status) {
        return Err(AxAgentError::Validation(format!(
            "非法状态迁移: {} → {new_status}（won/lost 为终态，contacted 才可标记 won）",
            existing.status
        )));
    }

    let mut am: opc_demand_leads::ActiveModel = existing.into();
    am.status = Set(new_status.to_string());
    am.updated_at = Set(now_ts());
    let saved = am.update(db).await?;
    tracing::info!(lead_id, new_status, "[opc_demand] 线索状态已更新");
    Ok(lead_from_entity(saved))
}

/// 记录线索 → 实现工作流的转化（status 语义归 update_lead_status，此处不碰）
pub async fn link_lead_to_workflow(
    db: &DatabaseConnection,
    lead_id: &str,
    workflow_id: &str,
) -> Result<()> {
    let existing = opc_demand_leads::Entity::find_by_id(lead_id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("需求线索不存在: {}", lead_id)))?;
    let mut am: opc_demand_leads::ActiveModel = existing.into();
    am.linked_workflow_id = Set(Some(workflow_id.to_string()));
    am.updated_at = Set(now_ts());
    am.update(db).await?;
    Ok(())
}

/// 标记线索的实现工作流已开始执行（首次执行写入时间戳）
pub async fn mark_lead_implemented(db: &DatabaseConnection, lead_id: &str) -> Result<()> {
    let existing = opc_demand_leads::Entity::find_by_id(lead_id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("需求线索不存在: {}", lead_id)))?;
    let mut am: opc_demand_leads::ActiveModel = existing.into();
    am.implemented_at = Set(Some(now_ts()));
    am.updated_at = Set(now_ts());
    am.update(db).await?;
    Ok(())
}

// ── 需求订阅（v133）─────────────────────────────────────────

/// 订阅间隔钳制区间（小时）：过短会打爆平台限流，过长失去订阅意义
pub const INTERVAL_HOURS_MIN: i32 = 1;
pub const INTERVAL_HOURS_MAX: i32 = 24 * 30;

/// 列出全部订阅（按创建时间升序）
pub async fn list_subscriptions(db: &DatabaseConnection) -> Result<Vec<DemandSubscription>> {
    let rows = opc_demand_subscriptions::Entity::find()
        .order_by_asc(opc_demand_subscriptions::Column::CreatedAt)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(subscription_from_entity).collect())
}

/// 列出到期待扫描的订阅
///
/// 到期判定：`enabled = 1` 且（`last_scanned_at` 为 NULL（从未扫描）
/// 或 `last_scanned_at + interval_hours * 3600 <= now`）。
pub async fn list_due_subscriptions(
    db: &DatabaseConnection,
    now: i64,
) -> Result<Vec<DemandSubscription>> {
    let rows = opc_demand_subscriptions::Entity::find()
        .filter(opc_demand_subscriptions::Column::Enabled.eq(1))
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(subscription_from_entity)
        .filter(|s| is_subscription_due(s.last_scanned_at, s.interval_hours, now))
        .collect())
}

/// 到期判定（纯函数，便于单测）：未扫描过 → 立即到期
fn is_subscription_due(last_scanned_at: Option<i64>, interval_hours: i32, now: i64) -> bool {
    match last_scanned_at {
        None => true,
        Some(last) => {
            // 用 std::cmp::max 显式限定：sea_orm 的 ExprTrait 也为整数实现了 max，
            // 裸调用 .max() 会因 trait 歧义编译失败（E0034）。
            let interval_secs = i64::from(std::cmp::max(interval_hours, INTERVAL_HOURS_MIN)) * 3600;
            now.saturating_sub(last) >= interval_secs
        },
    }
}

/// 新增或更新订阅（`id` 为空则新增，否则部分更新）
pub async fn save_subscription(
    db: &DatabaseConnection,
    input: SaveDemandSubscriptionInput,
) -> Result<DemandSubscription> {
    let now = now_ts();
    match input.id {
        Some(id) => {
            let existing = opc_demand_subscriptions::Entity::find_by_id(&id)
                .one(db)
                .await?
                .ok_or_else(|| AxAgentError::NotFound(format!("需求订阅不存在: {}", id)))?;
            let mut am: opc_demand_subscriptions::ActiveModel = existing.into();
            if let Some(keyword) = input.keyword {
                let keyword = keyword.trim().to_string();
                if keyword.is_empty() {
                    return Err(AxAgentError::Validation("订阅关键词不能为空".to_string()));
                }
                am.keyword = Set(keyword);
            }
            if let Some(enabled) = input.enabled {
                am.enabled = Set(i32::from(enabled));
            }
            if let Some(hours) = input.interval_hours {
                am.interval_hours = Set(hours.clamp(INTERVAL_HOURS_MIN, INTERVAL_HOURS_MAX));
            }
            if let Some(score) = input.min_score {
                am.min_score = Set(score.clamp(0.0, 100.0));
            }
            if let Some(platforms) = input.platforms {
                am.platforms_json =
                    Set(serde_json::to_string(&platforms).unwrap_or_else(|_| "[]".to_string()));
            }
            am.updated_at = Set(now);
            Ok(subscription_from_entity(am.update(db).await?))
        },
        None => {
            let keyword = input.keyword.unwrap_or_default().trim().to_string();
            if keyword.is_empty() {
                return Err(AxAgentError::Validation("订阅关键词不能为空".to_string()));
            }
            let row = opc_demand_subscriptions::ActiveModel {
                id: Set(gen_id()),
                keyword: Set(keyword),
                enabled: Set(i32::from(input.enabled.unwrap_or(true))),
                interval_hours: Set(input
                    .interval_hours
                    .unwrap_or(6)
                    .clamp(INTERVAL_HOURS_MIN, INTERVAL_HOURS_MAX)),
                min_score: Set(input.min_score.unwrap_or(60.0).clamp(0.0, 100.0)),
                platforms_json: Set(serde_json::to_string(&input.platforms.unwrap_or_default())
                    .unwrap_or_else(|_| "[]".to_string())),
                last_scanned_at: Set(None),
                last_hit_count: Set(0),
                created_at: Set(now),
                updated_at: Set(now),
            };
            Ok(subscription_from_entity(row.insert(db).await?))
        },
    }
}

/// 删除订阅
pub async fn delete_subscription(db: &DatabaseConnection, id: &str) -> Result<()> {
    let res = opc_demand_subscriptions::Entity::delete_by_id(id).exec(db).await?;
    if res.rows_affected == 0 {
        return Err(AxAgentError::NotFound(format!("需求订阅不存在: {}", id)));
    }
    Ok(())
}

/// 记录一次订阅扫描结果（推进到期时间 + 更新命中数）
pub async fn mark_subscription_scanned(
    db: &DatabaseConnection,
    id: &str,
    hit_count: i32,
) -> Result<()> {
    let existing = opc_demand_subscriptions::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("需求订阅不存在: {}", id)))?;
    let mut am: opc_demand_subscriptions::ActiveModel = existing.into();
    am.last_scanned_at = Set(Some(now_ts()));
    am.last_hit_count = Set(hit_count);
    am.updated_at = Set(now_ts());
    am.update(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_legal_status_transition, is_subscription_due};

    #[test]
    fn status_transitions_follow_lifecycle() {
        // 合法路径
        assert!(is_legal_status_transition("new", "evaluated"));
        assert!(is_legal_status_transition("new", "contacted"));
        assert!(is_legal_status_transition("new", "lost"));
        assert!(is_legal_status_transition("evaluated", "contacted"));
        assert!(is_legal_status_transition("evaluated", "lost"));
        assert!(is_legal_status_transition("contacted", "won"));
        assert!(is_legal_status_transition("contacted", "lost"));

        // 同状态幂等
        for s in ["new", "evaluated", "contacted", "won", "lost"] {
            assert!(is_legal_status_transition(s, s), "同状态应幂等: {s}");
        }
    }

    #[test]
    fn status_transitions_reject_illegal() {
        // new/evaluated 不能直接成交：必须先 contacted
        assert!(!is_legal_status_transition("new", "won"));
        assert!(!is_legal_status_transition("evaluated", "won"));
        // 终态不可回退/再迁移
        assert!(!is_legal_status_transition("won", "contacted"));
        assert!(!is_legal_status_transition("won", "lost"));
        assert!(!is_legal_status_transition("lost", "new"));
        assert!(!is_legal_status_transition("lost", "won"));
        // 不得跳级倒退
        assert!(!is_legal_status_transition("contacted", "evaluated"));
        assert!(!is_legal_status_transition("contacted", "new"));
        // 未知状态一律拒绝（防脏数据扩散）
        assert!(!is_legal_status_transition("unknown", "won"));
        assert!(!is_legal_status_transition("new", "unknown"));
    }

    const HOUR: i64 = 3600;

    #[test]
    fn never_scanned_subscription_is_due() {
        assert!(is_subscription_due(None, 6, 1_700_000_000));
    }

    #[test]
    fn subscription_due_after_interval() {
        let last = 1_700_000_000;
        // 未到间隔 → 不到期
        assert!(!is_subscription_due(Some(last), 6, last + 5 * HOUR));
        // 正好一个间隔 → 到期（边界 inclusive）
        assert!(is_subscription_due(Some(last), 6, last + 6 * HOUR));
        // 超过间隔 → 到期
        assert!(is_subscription_due(Some(last), 6, last + 100 * HOUR));
    }

    #[test]
    fn subscription_interval_clamped_to_min_one_hour() {
        // interval_hours=0 不能让订阅永不到期（会被 max(1) 兜底）
        let last = 1_700_000_000;
        assert!(!is_subscription_due(Some(last), 0, last + 59 * 60));
        assert!(is_subscription_due(Some(last), 0, last + HOUR));
    }

    #[test]
    fn subscription_due_handles_clock_skew() {
        // now < last（时钟回拨）→ saturating_sub 归零，不到期，不会 panic
        let last = 1_700_000_000;
        assert!(!is_subscription_due(Some(last), 1, last - 10 * HOUR));
    }

    /// 手动补录的读回语义：窗口内同指纹重复录入 → 返回既有生效行
    #[tokio::test]
    async fn create_manual_lead_returns_existing_row_on_duplicate() {
        use crate::migrations::{
            v132_opc_demand_discovery, v133_lead_workflow_link, v136_demand_lead_dedupe_fingerprint,
        };
        use sea_orm::Database;

        let db = Database::connect("sqlite::memory:").await.unwrap();
        v132_opc_demand_discovery::up(db.clone()).await.unwrap();
        v133_lead_workflow_link::up(db.clone()).await.unwrap();
        v136_demand_lead_dedupe_fingerprint::up(db.clone()).await.unwrap();

        let row = super::NewLeadRow {
            id: "lead-manual-1".to_string(),
            platform: "manual".to_string(),
            title: "需要一个自动周报工具".to_string(),
            description: "每周要读 50+ 篇论文，人工筛选太慢".to_string(),
            budget_min: Some(500.0),
            budget_max: Some(2000.0),
            budget_currency: "USD".to_string(),
            contact_name: None,
            contact_email: None,
            contact_phone: None,
            source_url: None,
            content_fingerprint: Some("fp-abc".to_string()),
            raw_snapshot: serde_json::json!({ "source": "manual" }),
            confidence: 0.7,
            pain_score: 80.0,
            market_gap_score: 60.0,
            commercial_value_score: 75.0,
            demand_type: "content_creation".to_string(),
        };

        // 首次录入 → Inserted，返回本行
        let first = super::create_manual_lead(&db, row.clone(), Some(86400)).await.unwrap();
        assert_eq!(first.id, "lead-manual-1");

        // 窗口内同指纹 → Skipped，读回既有行（id 与原评分不变）
        let dup = super::NewLeadRow {
            id: "lead-manual-2".to_string(),
            commercial_value_score: 66.0,
            ..row
        };
        let second = super::create_manual_lead(&db, dup, Some(86400)).await.unwrap();
        assert_eq!(second.id, "lead-manual-1");
        assert!((second.commercial_value_score - 75.0).abs() < f64::EPSILON);
    }
}
