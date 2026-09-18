// SPDX-License-Identifier: AGPL-3.0-only

//! Agent 系列 repository 的 dao 实现 + 全局注册。
//!
//! harness 的 repository 注册约定是"owner crate 在自身初始化时注册"；
//! dao 拥有的 agent 系列（profile / expert / role）与 workflow 系列此前未注册，
//! 导致 consumer crate 调用 `xxx_repository()` 访问器时 panic。
//! `register_repositories` 在 DB 初始化后统一注册这些 repo。

use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::*;

use axagent_entities::{agency_experts, agent_profiles, agent_roles};
use axagent_harness::repo_dtos::{AgencyExpertDto, AgentRoleDto};
use axagent_harness::repositories::{
    AgencyExpertRepository, AgentProfileRepository, AgentRoleRepository,
};
use axagent_harness::types::AgentProfile;

pub struct DaoAgentProfileRepository {
    pub db: DatabaseConnection,
}

pub struct DaoAgencyExpertRepository {
    pub db: DatabaseConnection,
}

pub struct DaoAgentRoleRepository {
    pub db: DatabaseConnection,
}

fn parse_json_arr(raw: &Option<String>) -> Vec<String> {
    raw.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()
}

#[async_trait]
impl AgentProfileRepository for DaoAgentProfileRepository {
    async fn get_agent_profile(&self, id: &str) -> Result<Option<AgentProfile>, String> {
        let row = agent_profiles::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(|m| AgentProfile {
            id: m.id,
            name: m.name,
            description: m.description,
            category: m.category,
            icon: m.icon,
            agent_role: m.agent_role,
            source: m.source,
            tags: parse_json_arr(&m.tags),
            suggested_provider_id: m.suggested_provider_id,
            suggested_model_id: m.suggested_model_id,
            suggested_temperature: m.suggested_temperature,
            suggested_max_tokens: m.suggested_max_tokens.map(|v| v as u32),
            search_enabled: m.search_enabled,
            recommend_permission_mode: m.recommend_permission_mode,
            recommended_tools: parse_json_arr(&m.recommended_tools),
            disallowed_tools: parse_json_arr(&m.disallowed_tools),
            recommended_workflows: parse_json_arr(&m.recommended_workflows),
            sort_order: m.sort_order,
            is_enabled: m.is_enabled != 0,
            expert_id: m.expert_id,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }))
    }
}

/// 把 agency_experts::Model 转换为 AgencyExpertDto（含 v100 consolidated 新增的 6 个字段）。
fn expert_from_model(m: agency_experts::Model) -> AgencyExpertDto {
    AgencyExpertDto {
        id: m.id,
        name: m.name,
        description: m.description,
        category: m.category,
        system_prompt: m.system_prompt,
        color: m.color,
        source_dir: m.source_dir,
        is_enabled: m.is_enabled != 0,
        imported_at: m.imported_at,
        recommended_workflows: m.recommended_workflows,
        recommended_tools: m.recommended_tools,
        active_domains: m.active_domains,
        seniority: m.seniority,
        specialties: m.specialties,
        success_rate: m.success_rate,
        avg_latency_ms: m.avg_latency_ms,
        avg_token_cost: m.avg_token_cost,
    }
}

#[async_trait]
impl AgencyExpertRepository for DaoAgencyExpertRepository {
    async fn get_agency_expert(&self, id: &str) -> Result<Option<AgencyExpertDto>, String> {
        let row = agency_experts::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(expert_from_model))
    }

    async fn list_agency_experts(&self) -> Result<Vec<AgencyExpertDto>, String> {
        // 仅返回 is_enabled=true 的记录，按 name 排序
        let rows = agency_experts::Entity::find()
            .filter(agency_experts::Column::IsEnabled.eq(1))
            .order_by_asc(agency_experts::Column::Name)
            .all(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(expert_from_model).collect())
    }
}

#[async_trait]
impl AgentRoleRepository for DaoAgentRoleRepository {
    async fn get_agent_role(&self, id: &str) -> Result<Option<AgentRoleDto>, String> {
        let row =
            agent_roles::Entity::find_by_id(id).one(&self.db).await.map_err(|e| e.to_string())?;
        Ok(row.map(agent_role_from_model))
    }

    async fn list_agent_roles(&self) -> Result<Vec<AgentRoleDto>, String> {
        let rows = agent_roles::Entity::find()
            .order_by_asc(agent_roles::Column::Source)
            .order_by_asc(agent_roles::Column::SortOrder)
            .order_by_asc(agent_roles::Column::Name)
            .all(&self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(agent_role_from_model).collect())
    }
}

fn agent_role_from_model(m: agent_roles::Model) -> AgentRoleDto {
    let tools: Vec<String> = parse_json_arr(&m.default_tools);
    let domains: Vec<String> = parse_json_arr(&m.active_domains);
    AgentRoleDto {
        id: m.id,
        name: m.name,
        description: m.description,
        system_prompt: m.system_prompt,
        default_tools: tools,
        active_domains: domains,
        max_concurrent: m.max_concurrent,
        timeout_seconds: m.timeout_seconds,
        source: m.source,
        sort_order: m.sort_order,
        created_at: m.created_at,
        updated_at: m.updated_at,
        responsibilities: m.responsibilities,
        decision_authority: m.decision_authority,
        reports_to: m.reports_to,
        managed_expert_ids: m.managed_expert_ids,
        required_certifications: m.required_certifications,
        icon: m.icon,
        color: m.color,
        is_enabled: m.is_enabled != 0,
    }
}

/// 将 dao 实现的 repository 注册进 harness 全局服务注册表。
///
/// 在 DB 初始化后调用（见 `src/init/database.rs`），统一注册：
/// - agent 系列：agent_profile / agency_expert / agent_role
/// - workflow 系列：workflow_execution / loop_checkpoint / workflow_template
/// - settings/tools 系列：settings
pub fn register_repositories(db: &DatabaseConnection) {
    axagent_harness::repositories::set_agent_profile_repository(Arc::new(
        DaoAgentProfileRepository { db: db.clone() },
    ));
    axagent_harness::repositories::set_agency_expert_repository(Arc::new(
        DaoAgencyExpertRepository { db: db.clone() },
    ));
    axagent_harness::repositories::set_agent_role_repository(Arc::new(DaoAgentRoleRepository {
        db: db.clone(),
    }));
    axagent_harness::repositories::set_workflow_execution_repository(Arc::new(
        crate::workflow_execution_repository::DaoWorkflowExecutionRepository {
            db: Arc::new(db.clone()),
        },
    ));
    axagent_harness::repositories::set_loop_checkpoint_repository(Arc::new(
        crate::loop_checkpoint_repository::DaoLoopCheckpointRepository { db: Arc::new(db.clone()) },
    ));
    axagent_harness::repositories::set_workflow_template_repository(Arc::new(
        crate::workflow_template_repository::DaoWorkflowTemplateRepository {
            db: Arc::new(db.clone()),
        },
    ));
    // wiki / note 域 repository（dao 实现，wiki_dtos 为权威 trait）
    axagent_harness::repositories::set_note_repository(Arc::new(
        crate::repo::note_repository::DaoNoteRepository::new(Arc::new(db.clone())),
    ));
    axagent_harness::repositories::set_wiki_repository(Arc::new(
        crate::repo::wiki_repository::DaoWikiRepository::new(Arc::new(db.clone())),
    ));
    axagent_harness::repositories::set_wiki_page_repository(Arc::new(
        crate::repo::wiki_page_repository::DaoWikiPageRepository::new(Arc::new(db.clone())),
    ));
    axagent_harness::repositories::set_wiki_source_repository(Arc::new(
        crate::repo::wiki_source_repository::DaoWikiSourceRepository::new(Arc::new(db.clone())),
    ));
    axagent_harness::repositories::set_wiki_operation_repository(Arc::new(
        crate::repo::wiki_operation_repository::DaoWikiOperationRepository::new(Arc::new(
            db.clone(),
        )),
    ));
    axagent_harness::repositories::set_note_backlink_repository(Arc::new(
        crate::repo::note_backlink_repository::DaoNoteBacklinkRepository::new(Arc::new(db.clone())),
    ));
    axagent_harness::repositories::set_settings_repository(Arc::new(
        crate::settings_repository::DaoSettingsRepository::new(db.clone()),
    ));
    axagent_harness::repositories::set_provider_repository(Arc::new(
        crate::provider_repository::DaoProviderRepository::new(db.clone()),
    ));

    // ── 以下 9 个仓库是 2026-09-12 补的（P1-C 前置修复）──────────────────
    //
    // 它们**此前从未被注册**，而各自的 `xxx_repository()` getter 内部是
    // `.expect("XxxRepository not initialized.")` —— 于是所有消费点不是「返回空」
    // 而是**直接 panic**。被影响的活跃链路：
    //
    // | 仓库 | 消费点 | 影响 |
    // |---|---|---|
    // | `background_task` | `tools/task_system.rs` 的 6 个工具 | TaskCreate/Get/List/Stop/Update/Output 全部不可用 |
    // | `tool_execution` | `tools/recorder.rs`（经 `ToolRegistry.recorder`） | 每次 MCP 工具调用成功后的审计记录 panic |
    // | `conversation` | `rt-messaging/.../platform_bridge.rs` | 平台消息桥无法建会话 |
    // | `platform_config` | `rt-messaging/.../platform_manager.rs` | 平台路由表读取 panic |
    // | `generated_tool` | `runtime/tool_generator/persistence.rs` | 动态生成工具的持久化 panic |
    // | `knowledge_{document,entity,flow,interface}` | `tools/knowledge.rs` 的 4 个工具 | 知识图谱工具全部不可用 |
    //
    // 形态说明：这是「**声明了但无入边供给**」的注册表版本 —— 契约（trait）、
    // 实现（`DaoXxxRepository`）、消费点（getter 调用）三样齐全，唯独少了
    // 「把实现塞进注册表」这一句。`cargo check` 全绿、单测全绿、只有真跑到那行才炸。
    axagent_harness::repositories::set_background_task_repository(Arc::new(
        crate::background_task_repository::DaoBackgroundTaskRepository::new(db.clone()),
    ));
    axagent_harness::repositories::set_conversation_repository(Arc::new(
        crate::conversation_repository::DaoConversationRepository::new(db.clone()),
    ));
    axagent_harness::repositories::set_generated_tool_repository(Arc::new(
        crate::generated_tool_repository::DaoGeneratedToolRepository::new(db.clone()),
    ));
    axagent_harness::repositories::set_platform_config_repository(Arc::new(
        crate::platform_config_repository::DaoPlatformConfigRepository::new(db.clone()),
    ));
    axagent_harness::repositories::set_tool_execution_repository(Arc::new(
        crate::tool_execution_repository::DaoToolExecutionRepository::new(Arc::new(db.clone())),
    ));
    // 知识图谱四件套（同一文件内四个结构体，trait 各自独立）
    axagent_harness::repositories::set_knowledge_entity_repository(Arc::new(
        crate::knowledge_crud_repository::DaoKnowledgeEntityRepository::new(db.clone()),
    ));
    axagent_harness::repositories::set_knowledge_flow_repository(Arc::new(
        crate::knowledge_crud_repository::DaoKnowledgeFlowRepository::new(db.clone()),
    ));
    axagent_harness::repositories::set_knowledge_interface_repository(Arc::new(
        crate::knowledge_crud_repository::DaoKnowledgeInterfaceRepository::new(db.clone()),
    ));
    axagent_harness::repositories::set_knowledge_document_repository(Arc::new(
        crate::knowledge_crud_repository::DaoKnowledgeDocumentRepository::new(db.clone()),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    /// **回归网**：注册表里的每个 getter 在被唯一消费之前，都必须已注册。
    ///
    /// 这条测试的存在理由是本文件上方注释记录的那次事故：契约、实现、消费点
    /// 三样齐全，唯独漏了注册这一句 —— 而 `cargo check` 与其余单测全绿，
    /// 唯一表现是运行期 `.expect` panic。
    ///
    /// 因此这里**逐个调用** getter（getter 内部就是 `.expect`，未注册即 panic）
    /// 而不是断言注册表字段非空 —— 后者需要暴露内部结构，且测的是实现细节而非行为。
    /// 断言「调用不 panic」才是消费点真正依赖的契约。
    #[tokio::test]
    async fn all_registered_repositories_are_retrievable() {
        let db = Database::connect("sqlite::memory:").await.expect("in-memory db");
        register_repositories(&db);

        // 有活跃消费点的仓库（缺一即在运行期 panic）
        let _ = axagent_harness::repositories::agent_profile_repository();
        let _ = axagent_harness::repositories::agency_expert_repository();
        let _ = axagent_harness::repositories::agent_role_repository();
        let _ = axagent_harness::repositories::workflow_execution_repository();
        let _ = axagent_harness::repositories::loop_checkpoint_repository();
        let _ = axagent_harness::repositories::workflow_template_repository();
        let _ = axagent_harness::repositories::note_repository();
        let _ = axagent_harness::repositories::wiki_repository();
        let _ = axagent_harness::repositories::wiki_page_repository();
        let _ = axagent_harness::repositories::wiki_source_repository();
        let _ = axagent_harness::repositories::wiki_operation_repository();
        let _ = axagent_harness::repositories::note_backlink_repository();
        let _ = axagent_harness::repositories::settings_repository();
        let _ = axagent_harness::repositories::provider_repository();

        // 2026-09-12 补注册的 9 个
        let _ = axagent_harness::repositories::background_task_repository();
        let _ = axagent_harness::repositories::conversation_repository();
        let _ = axagent_harness::repositories::generated_tool_repository();
        let _ = axagent_harness::repositories::platform_config_repository();
        let _ = axagent_harness::repositories::tool_execution_repository();
        let _ = axagent_harness::repositories::knowledge_entity_repository();
        let _ = axagent_harness::repositories::knowledge_flow_repository();
        let _ = axagent_harness::repositories::knowledge_interface_repository();
        let _ = axagent_harness::repositories::knowledge_document_repository();
    }
}
