// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现数据访问层（v131）
//!
//! 平台配置 CRUD + 需求线索持久化。评估逻辑在 `axagent_tools`
//! 的 `marketplace_scanner`，本模块只做存储与查询。

use sea_orm::*;

use axagent_entities::{opc_demand_leads, opc_demand_platforms};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{DemandLeadDto, DemandPlatform, SaveDemandPlatformInput};
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
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

// ── 平台配置 ─────────────────────────────────────────────────

/// 内置默认平台清单（与前端 mock preset 对齐；platform_type 一律 "scanner"）
pub const DEFAULT_PLATFORMS: &[(&str, &str)] = &[
    ("reddit", "Reddit"),
    ("hackernews", "HackerNews"),
    ("github_issue", "GitHub Issues"),
    ("github_discussion", "GitHub Discussions"),
    ("stackoverflow", "StackOverflow"),
    ("producthunt", "Product Hunt"),
    ("huggingface", "HuggingFace"),
    ("package_ecosystem", "Package Ecosystem"),
    ("arxiv", "arXiv"),
    ("twitter", "Twitter/X"),
    ("zhubajie", "猪八戒"),
    ("xianyu", "闲鱼"),
    ("linkedin", "LinkedIn"),
    ("zhihu", "知乎"),
    ("csdn", "CSDN"),
    ("juejin", "掘金"),
    ("dribbble", "Dribbble"),
    ("upwork", "Upwork"),
];

/// 平台表为空时插入内置默认平台（幂等：只在空表时执行）
pub async fn seed_default_platforms_if_empty(db: &DatabaseConnection) -> Result<()> {
    let count = opc_demand_platforms::Entity::find().count(db).await?;
    if count > 0 {
        return Ok(());
    }
    let now = now_ts();
    for (id, name) in DEFAULT_PLATFORMS {
        let row = opc_demand_platforms::ActiveModel {
            id: Set(String::from(*id)),
            name: Set(String::from(*name)),
            platform_type: Set("scanner".to_string()),
            enabled: Set(1),
            base_url: Set(None),
            config_json: Set(
                serde_json::json!({ "description": format!("{} 扫描器", name), "auto_sync": true })
                    .to_string(),
            ),
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
/// 去重键是**唯一索引** `(platform, source_url)`，所以窗口外再次命中同一条需求时
/// 不能插入（必然撞唯一约束），只能刷新既有行——这正是
/// `scanDeduplicateWindowHours` 的语义：窗口内抑制重复曝光，窗口外允许刷新评分。
///
/// - `window_secs = None`：永久去重，只要库里存在同源线索就跳过
/// - `window_secs = Some(s)`：仅当既有行的 `created_at` 落在 `[now - s, now]` 内才跳过
pub async fn upsert_lead_within_window(
    db: &DatabaseConnection,
    row: NewLeadRow,
    window_secs: Option<i64>,
) -> Result<LeadWriteOutcome> {
    if let Some(url) = &row.source_url {
        let dup = opc_demand_leads::Entity::find()
            .filter(opc_demand_leads::Column::Platform.eq(&row.platform))
            .filter(opc_demand_leads::Column::SourceUrl.eq(url))
            .one(db)
            .await?;

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
        raw_snapshot: Set(row.raw_snapshot.to_string()),
        status: Set("new".to_string()),
        confidence: Set(row.confidence),
        pain_score: Set(row.pain_score),
        market_gap_score: Set(row.market_gap_score),
        commercial_value_score: Set(row.commercial_value_score),
        demand_type: Set(row.demand_type),
        created_at: Set(now),
        updated_at: Set(now),
    };
    am.insert(db).await?;
    Ok(LeadWriteOutcome::Inserted)
}

/// 按商业价值分降序列出线索
pub async fn list_leads(
    db: &DatabaseConnection,
    limit: u64,
    min_score: Option<f64>,
) -> Result<Vec<DemandLeadDto>> {
    let mut select = opc_demand_leads::Entity::find();
    if let Some(min) = min_score {
        select = select.filter(opc_demand_leads::Column::CommercialValueScore.gte(min));
    }
    let rows = select
        .order_by_desc(opc_demand_leads::Column::CommercialValueScore)
        .order_by_desc(opc_demand_leads::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(lead_from_entity).collect())
}
