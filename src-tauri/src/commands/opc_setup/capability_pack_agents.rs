// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 域包专家种子化（专家 → Profile）
//!
//! 注意：角色对应岗位，在 agent 节点中可以为空。
//! 专家（Expert）是核心，Profile 可以只绑定专家，不绑定角色。

use axagent_entities::{agency_experts, agent_profiles};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

use super::capability_pack_experts;

// ── AI Research 域包 ──

/// 4 个 ai-research 专家（编译期嵌入）
const AI_RESEARCH_EXPERTS: &[(&str, &str, &str)] = &[
    (
        "ai-research-director",
        "AI 研究负责人",
        include_str!("../../../agency_experts/opc/ai-research-director.md"),
    ),
    (
        "ai-literature-analyst",
        "AI 文献分析师",
        include_str!("../../../agency_experts/opc/ai-literature-analyst.md"),
    ),
    (
        "ai-benchmark-analyst",
        "AI 模型评测专家",
        include_str!("../../../agency_experts/opc/ai-benchmark-analyst.md"),
    ),
    (
        "ai-report-analyst",
        "AI 报告分析师",
        include_str!("../../../agency_experts/opc/ai-report-analyst.md"),
    ),
];

const AI_RESEARCH_PROFILE_TOOLS: &[(&str, &[&str])] = &[
    ("ai-research-director", &["OpcListProjects", "OpcCreateProject", "OpcSearchWiki"]),
    ("ai-literature-analyst", &["WebSearch", "FileRead", "OpcSearchWiki"]),
    ("ai-benchmark-analyst", &["Bash", "FileRead", "FileWrite"]),
    ("ai-report-analyst", &["FileWrite", "OpcListKpis", "OpcRecordKpi", "OpcSendNotification"]),
];

// ── 通用种子化函数 ──

/// 种子化专家到 agency_experts 表
async fn seed_experts(
    db: &DatabaseConnection,
    experts: &[(&str, &str, &str)],
    category: &str,
    color: &str,
) -> Result<(), String> {
    let mut count = 0u32;
    for (id, name, content) in experts {
        let expert_id = format!("opc-{id}");
        let domain = content
            .lines()
            .find(|l| l.starts_with("domain:"))
            .and_then(|l| l.strip_prefix("domain:").map(|s| s.trim().to_string()))
            .unwrap_or_default();

        let am = agency_experts::ActiveModel {
            id: Set(expert_id.clone()),
            name: Set(name.to_string()),
            description: Set(Some(format!("OPC {category} — {name}"))),
            category: Set(category.into()),
            system_prompt: Set(content.to_string()),
            color: Set(Some(color.to_string())),
            source_dir: Set("opc".into()),
            is_enabled: Set(1),
            imported_at: Set(chrono::Utc::now().timestamp()),
            recommended_workflows: Set(None),
            recommended_tools: Set(None),
            active_domains: Set(Some(serde_json::to_string(&vec![domain]).unwrap_or_default())),
            seniority: Set(None),
            specialties: Set(None),
            parent_role_id: Set(None),
            success_rate: Set(None),
            avg_latency_ms: Set(None),
            avg_token_cost: Set(None),
        };

        agency_experts::Entity::insert(am)
            .on_conflict(
                sea_orm::sea_query::OnConflict::columns([agency_experts::Column::Id])
                    .update_columns([
                        agency_experts::Column::Name,
                        agency_experts::Column::Description,
                        agency_experts::Column::SystemPrompt,
                        agency_experts::Column::ActiveDomains,
                        agency_experts::Column::IsEnabled,
                    ])
                    .to_owned(),
            )
            .exec(db)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
        count += 1;
    }
    tracing::info!("[opc-domain-pack] 种子化 {count} 个 {category} agency_experts");
    Ok(())
}

/// 种子化 AgentProfile（仅绑定专家，角色可选）
async fn seed_profiles(
    db: &DatabaseConnection,
    experts: &[(&str, &str, &str)],
    profile_tools: &[(&str, &[&str])],
    category: &str,
    icon: &str,
) -> Result<(), String> {
    let mut count = 0u32;
    for (expert_key, name, _) in experts {
        let profile_id = format!("opc-{expert_key}");
        let expert_id = format!("opc-{expert_key}");

        let display_name = name.to_string();

        let tools_json = profile_tools
            .iter()
            .find(|(k, _)| k == expert_key)
            .map(|(_, tools)| serde_json::to_string(tools).unwrap_or_default());

        let now = chrono::Utc::now().timestamp_millis();
        let existing =
            agent_profiles::Entity::find_by_id(&profile_id).one(db).await.map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;

        let am = agent_profiles::ActiveModel {
            id: Set(profile_id.clone()),
            name: Set(format!("{icon} {display_name}")),
            description: Set(Some(format!("OPC {category} — 专家 {expert_key}"))),
            category: Set(category.into()),
            icon: Set(icon.into()),
            agent_role: Set(None), // 角色可选，专家独立存在
            source: Set("opc-capability_pack".into()),
            tags: Set(None),
            suggested_provider_id: Set(None),
            suggested_model_id: Set(None),
            suggested_temperature: Set(None),
            suggested_max_tokens: Set(None),
            search_enabled: Set(None),
            recommend_permission_mode: Set(None),
            recommended_tools: Set(tools_json),
            disallowed_tools: Set(None),
            recommended_workflows: Set(None),
            sort_order: Set(0),
            is_enabled: Set(1),
            expert_id: Set(Some(expert_id)),
            created_at: Set(now),
            updated_at: Set(now),
        };

        if existing.is_some() {
            am.update(db).await.map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
        } else {
            am.insert(db).await.map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
        }
        count += 1;
    }
    tracing::info!("[opc-domain-pack] 种子化 {count} 个 {category} agent_profiles (专家驱动)");
    Ok(())
}

// ── AI Research 域包 ──

/// 主入口：种子化 ai-research 域包专家和 Profile
pub async fn seed_ai_research_agents(db: &DatabaseConnection) -> Result<(), String> {
    seed_experts(db, AI_RESEARCH_EXPERTS, "opc-capability_pack", "#722ed1").await?;
    seed_profiles(db, AI_RESEARCH_EXPERTS, AI_RESEARCH_PROFILE_TOOLS, "opc-capability_pack", "🤖")
        .await?;
    tracing::info!("[opc-domain-pack] ai-research 域包专家种子化完成");
    Ok(())
}

// ── 12 个新域包 ──

/// 种子化所有 12 个新域包的专家和 Profile
pub async fn seed_all_capability_pack_agents(db: &DatabaseConnection) -> Result<(), String> {
    // 1. 会计与财务管理
    seed_capability_pack_experts_inner(
        db,
        "accounting",
        capability_pack_experts::ACCOUNTING_EXPERTS,
        capability_pack_experts::ACCOUNTING_PROFILE_TOOLS,
        "opc-capability_pack",
        "#1890ff",
        "📊",
    )
    .await?;

    // 2. 金融投资
    seed_capability_pack_experts_inner(
        db,
        "finance-invest",
        capability_pack_experts::FINANCE_INVEST_EXPERTS,
        capability_pack_experts::FINANCE_INVEST_PROFILE_TOOLS,
        "opc-capability_pack",
        "#faad14",
        "💰",
    )
    .await?;

    // 3. 游戏开发
    seed_capability_pack_experts_inner(
        db,
        "game-dev",
        capability_pack_experts::GAME_DEV_EXPERTS,
        capability_pack_experts::GAME_DEV_PROFILE_TOOLS,
        "opc-capability_pack",
        "#52c41a",
        "🎮",
    )
    .await?;

    // 4. 设计
    seed_capability_pack_experts_inner(
        db,
        "design",
        capability_pack_experts::DESIGN_EXPERTS,
        capability_pack_experts::DESIGN_PROFILE_TOOLS,
        "opc-capability_pack",
        "#eb2f96",
        "🎨",
    )
    .await?;

    // 5. 电子商务
    seed_capability_pack_experts_inner(
        db,
        "ecommerce",
        capability_pack_experts::ECOMMERCE_EXPERTS,
        capability_pack_experts::ECOMMERCE_PROFILE_TOOLS,
        "opc-capability_pack",
        "#13c2c2",
        "🛒",
    )
    .await?;

    // 6. 教育培训
    seed_capability_pack_experts_inner(
        db,
        "education",
        capability_pack_experts::EDUCATION_EXPERTS,
        capability_pack_experts::EDUCATION_PROFILE_TOOLS,
        "opc-capability_pack",
        "#2f54eb",
        "📚",
    )
    .await?;

    // 7. 地理信息
    seed_capability_pack_experts_inner(
        db,
        "geospatial",
        capability_pack_experts::GEOSPATIAL_EXPERTS,
        capability_pack_experts::GEOSPATIAL_PROFILE_TOOLS,
        "opc-capability_pack",
        "#389e0d",
        "🗺️",
    )
    .await?;

    // 8. 咨询
    seed_capability_pack_experts_inner(
        db,
        "consulting",
        capability_pack_experts::CONSULTING_EXPERTS,
        capability_pack_experts::CONSULTING_PROFILE_TOOLS,
        "opc-capability_pack",
        "#722ed1",
        "💼",
    )
    .await?;

    // 9. 项目管理
    seed_capability_pack_experts_inner(
        db,
        "project-management",
        capability_pack_experts::PROJECT_MANAGEMENT_EXPERTS,
        capability_pack_experts::PROJECT_MANAGEMENT_PROFILE_TOOLS,
        "opc-capability_pack",
        "#fa8c16",
        "📋",
    )
    .await?;

    // 10. 销售增长
    seed_capability_pack_experts_inner(
        db,
        "sales-growth",
        capability_pack_experts::SALES_GROWTH_EXPERTS,
        capability_pack_experts::SALES_GROWTH_PROFILE_TOOLS,
        "opc-capability_pack",
        "#f5222d",
        "📈",
    )
    .await?;

    // 11. 安全合规
    seed_capability_pack_experts_inner(
        db,
        "security",
        capability_pack_experts::SECURITY_EXPERTS,
        capability_pack_experts::SECURITY_PROFILE_TOOLS,
        "opc-capability_pack",
        "#000000",
        "🔒",
    )
    .await?;

    // 12. 软件开发
    seed_capability_pack_experts_inner(
        db,
        "software-dev",
        capability_pack_experts::SOFTWARE_DEV_EXPERTS,
        capability_pack_experts::SOFTWARE_DEV_PROFILE_TOOLS,
        "opc-capability_pack",
        "#2f54eb",
        "💻",
    )
    .await?;

    tracing::info!("[opc-domain-pack] 12 个新域包专家种子化完成");
    Ok(())
}

/// 内部通用函数：种子化单个域包的专家和 Profile
async fn seed_capability_pack_experts_inner(
    db: &DatabaseConnection,
    capability_pack_name: &str,
    experts: &[(&str, &str, &str)],
    profile_tools: &[(&str, &[&str])],
    category: &str,
    icon: &str,
    color: &str,
) -> Result<(), String> {
    seed_experts(db, experts, category, color).await?;
    seed_profiles(db, experts, profile_tools, category, icon).await?;
    tracing::info!("[opc-domain-pack] {capability_pack_name} 域包专家种子化完成");
    Ok(())
}

// ── 域包 → 专家名册查询面（PLAN-office-auto-provision.md 阶段 1）──
//
// pack_id 用 snake_case，与 `config/opc/domain_packs/` 目录名、前端 sceneTemplateSlug 对齐。
// seed 调用仍逐包手写（各包 icon/color 不同），但新增域包必须同步
// `seed_all_capability_pack_agents` 与本注册表 —— 由 roster_covers_seed_lists 测试钉住。
// 已知缺口：content_media 域包无专家 seed（名册覆盖 13/14），roster 返回 None。

type ExpertTriples = &'static [(&'static str, &'static str, &'static str)];
type ProfileTools = &'static [(&'static str, &'static [&'static str])];

/// 名册覆盖的域包 id（13 个），仅供测试盘点（生产消费面是 `domain_pack_roster`）。
#[cfg(test)]
const ROSTER_DOMAIN_PACK_IDS: &[&str] = &[
    "accounting",
    "ai_research",
    "consulting",
    "design",
    "ecommerce",
    "education",
    "finance_invest",
    "game_dev",
    "geospatial",
    "project_management",
    "sales_growth",
    "security",
    "software_dev",
];

/// 域包专家名册的权威查询面：返回 (专家三元组, profile 工具白名单)。
///
/// 未知域包返回 `None`（content_media 亦为 None —— 无 seed 的域包「配不出队」是事实而非错误）。
pub fn domain_pack_roster(pack_id: &str) -> Option<(ExpertTriples, ProfileTools)> {
    Some(match pack_id {
        "accounting" => (
            capability_pack_experts::ACCOUNTING_EXPERTS,
            capability_pack_experts::ACCOUNTING_PROFILE_TOOLS,
        ),
        "ai_research" => (AI_RESEARCH_EXPERTS, AI_RESEARCH_PROFILE_TOOLS),
        "consulting" => (
            capability_pack_experts::CONSULTING_EXPERTS,
            capability_pack_experts::CONSULTING_PROFILE_TOOLS,
        ),
        "design" => {
            (capability_pack_experts::DESIGN_EXPERTS, capability_pack_experts::DESIGN_PROFILE_TOOLS)
        },
        "ecommerce" => (
            capability_pack_experts::ECOMMERCE_EXPERTS,
            capability_pack_experts::ECOMMERCE_PROFILE_TOOLS,
        ),
        "education" => (
            capability_pack_experts::EDUCATION_EXPERTS,
            capability_pack_experts::EDUCATION_PROFILE_TOOLS,
        ),
        "finance_invest" => (
            capability_pack_experts::FINANCE_INVEST_EXPERTS,
            capability_pack_experts::FINANCE_INVEST_PROFILE_TOOLS,
        ),
        "game_dev" => (
            capability_pack_experts::GAME_DEV_EXPERTS,
            capability_pack_experts::GAME_DEV_PROFILE_TOOLS,
        ),
        "geospatial" => (
            capability_pack_experts::GEOSPATIAL_EXPERTS,
            capability_pack_experts::GEOSPATIAL_PROFILE_TOOLS,
        ),
        "project_management" => (
            capability_pack_experts::PROJECT_MANAGEMENT_EXPERTS,
            capability_pack_experts::PROJECT_MANAGEMENT_PROFILE_TOOLS,
        ),
        "sales_growth" => (
            capability_pack_experts::SALES_GROWTH_EXPERTS,
            capability_pack_experts::SALES_GROWTH_PROFILE_TOOLS,
        ),
        "security" => (
            capability_pack_experts::SECURITY_EXPERTS,
            capability_pack_experts::SECURITY_PROFILE_TOOLS,
        ),
        "software_dev" => (
            capability_pack_experts::SOFTWARE_DEV_EXPERTS,
            capability_pack_experts::SOFTWARE_DEV_PROFILE_TOOLS,
        ),
        _ => return None,
    })
}

/// 域包 profile id 约定：`opc-<expert_key>`（seed_profiles 与消费方的共同契约）
pub fn domain_pack_profile_id(expert_key: &str) -> String {
    format!("opc-{expert_key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_covers_all_domain_pack_ids() {
        assert_eq!(ROSTER_DOMAIN_PACK_IDS.len(), 13);
        for id in ROSTER_DOMAIN_PACK_IDS {
            let (experts, _tools) =
                domain_pack_roster(id).unwrap_or_else(|| panic!("域包 {id} 名册缺失"));
            assert!(!experts.is_empty(), "域包 {id} 专家清单为空");
            for (key, name, content) in experts {
                assert!(!key.is_empty() && !name.is_empty());
                assert!(content.contains("role:"), "专家 {key} 的 md 缺少 frontmatter");
            }
        }
    }

    #[test]
    fn unknown_and_unseeded_packs_return_none() {
        // content_media 有域包目录但无专家 seed —— 缺口被显式锁定，补 seed 时必须一并更名册
        assert!(domain_pack_roster("content_media").is_none());
        assert!(domain_pack_roster("no_such_pack").is_none());
    }

    #[tokio::test]
    async fn roster_profile_ids_match_seeded_profiles() {
        use axagent_entities::agent_profiles;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let h = axagent_dao::db::create_test_pool().await.unwrap();
        let db = &h.conn;
        seed_ai_research_agents(db).await.unwrap();

        let (experts, _) = domain_pack_roster("ai_research").unwrap();
        let ids: Vec<String> = experts.iter().map(|(k, _, _)| domain_pack_profile_id(k)).collect();
        let rows = agent_profiles::Entity::find()
            .filter(agent_profiles::Column::Id.is_in(ids))
            .all(db)
            .await
            .unwrap();
        assert_eq!(rows.len(), experts.len(), "seed 后名册 profile id 必须逐条命中 DB");
    }
}
