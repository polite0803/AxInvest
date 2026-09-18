// SPDX-License-Identifier: AGPL-3.0-only
//! 能力发现 Tauri 命令集
//!
//! 暴露能力注册、发现、索引管理等命令给前端调用。
//! 所有命令从 AppState 获取已注入的路由器和索引器实例。

use crate::AppState;
use crate::commands::error::{CommandError, ErrorCategory, ErrorResponse};
use axagent_agent_macro::agent_command;
use axagent_dao::repo::agent_profile as agent_profile_repo;
use axagent_dao::repo::agent_role as agent_role_repo;
use axagent_dao::repo::capability_domain_override as domain_override_repo;
use axagent_entities::{agency_experts, agent_roles};
use axagent_harness::trajectory_types::TrajectoryOutcome;
use axagent_harness::{
    CapabilityDiscoveryRequest, CapabilityDiscoveryResult, CapabilityDomain,
    CapabilityEvolvability, CapabilityIndexer, CapabilityKind, CapabilityLevel,
    CapabilityPassportDto, CapabilityQuery, CapabilitySource, DiscoveryWeights, FilterContext,
    Reflection, SessionBudget, Visibility,
};
use axagent_trajectory::{
    ComputationGraph, ComputationNode, NodeType, TextGradConfig, TextGradEngine,
};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use tauri::State;

/// 运行时能力注册表检视 DTO（P3：外部插件注册的可查询闭环）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRegistrationDetailDto {
    pub id: String,
    pub version: String,
    pub contract: String,
    pub description: String,
    pub origin: String,
    pub plugin_id: Option<String>,
}

// ── DTO 类型 ──────────────────────────────────────

/// 注册能力护照的请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterPassportRequest {
    pub passport: CapabilityPassportDto,
}

/// 能力发现的请求
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverRequest {
    pub user_input: String,
    #[serde(default)]
    pub filter_context: Option<FilterContext>,
    #[serde(default)]
    pub query: Option<CapabilityQuery>,
    #[serde(default)]
    pub weights: Option<DiscoveryWeights>,
    #[serde(default)]
    pub budget: Option<SessionBudget>,
    #[serde(default = "default_true")]
    pub enable_completion: bool,
    #[serde(default = "default_false")]
    pub enable_circuit_breaker: bool,
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

// ── Tauri 命令 ────────────────────────────────────

/// 注册一个能力护照到索引
#[agent_command(domain = capability, safety = Caution, call_mode = StateInput, description = "注册能力护照")]
#[tauri::command]
pub async fn capability_register_passport(
    state: State<'_, AppState>,
    request: RegisterPassportRequest,
) -> Result<axagent_harness::IndexResult, CommandError> {
    state.capability_indexer.index_passport(&request.passport).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability::REGISTER_FAILED,
            e,
            ErrorCategory::Unrecoverable,
        )
    })
}

/// 批量注册能力护照
#[agent_command(domain = capability, safety = Caution, call_mode = StateInput, description = "批量注册能力护照")]
#[tauri::command]
pub async fn capability_register_batch(
    state: State<'_, AppState>,
    passports: Vec<CapabilityPassportDto>,
) -> Result<Vec<axagent_harness::IndexResult>, CommandError> {
    Ok(state.capability_indexer.index_batch(&passports).await)
}

/// 执行能力发现管线
#[agent_command(domain = capability, safety = Safe, call_mode = StateInput, description = "执行能力发现管线")]
#[tauri::command]
pub async fn capability_discover(
    state: State<'_, AppState>,
    request: DiscoverRequest,
) -> Result<CapabilityDiscoveryResult, CommandError> {
    let mut query = request.query.unwrap_or_default();
    // 确保 query.user_input 使用用户实际输入
    if query.user_input.is_empty() {
        query.user_input = request.user_input.clone();
    }
    let filter_context = request.filter_context.unwrap_or_default();
    let weights = request.weights.unwrap_or_default();
    let budget = request.budget.unwrap_or_default();

    let discovery_request = CapabilityDiscoveryRequest {
        user_input: request.user_input,
        filter_context,
        query,
        weights,
        budget,
        enable_completion: request.enable_completion,
        enable_circuit_breaker: request.enable_circuit_breaker,
        enable_rar: false,
        rar_top_k: 5,
        task_shape: None,
    };

    axagent_harness::CapabilityRouter::discover(
        state.capability_router.as_ref(),
        &discovery_request,
    )
    .await
    .map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability::DISCOVER_FAILED,
            e,
            ErrorCategory::Retryable,
        )
    })
}

/// 列出已注册的能力
#[agent_command(domain = capability, safety = Safe, call_mode = StateOnly, description = "列出已注册的能力")]
#[tauri::command]
pub async fn capability_list_passports(
    state: State<'_, AppState>,
) -> Result<Vec<CapabilityPassportDto>, CommandError> {
    let ids = state.capability_indexer.list_capability_ids().await;
    let mut passports = Vec::new();
    for id in ids {
        if let Some(passport) = state.capability_indexer.get_passport(&id).await {
            passports.push(passport);
        }
    }
    Ok(passports)
}

/// 删除一个能力
#[agent_command(domain = capability, safety = Dangerous, call_mode = StateInput, description = "删除能力护照")]
#[tauri::command]
pub async fn capability_remove_passport(
    state: State<'_, AppState>,
    capability_id: String,
) -> Result<(), CommandError> {
    state.capability_indexer.remove_index(&capability_id).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability::NOT_FOUND,
            e,
            ErrorCategory::Unrecoverable,
        )
    })
}

/// 获取索引统计信息
#[agent_command(domain = capability, safety = Safe, call_mode = StateOnly, description = "获取能力索引统计信息")]
#[tauri::command]
pub async fn capability_get_stats(
    state: State<'_, AppState>,
) -> Result<axagent_harness::CapabilityIndexStats, CommandError> {
    state.capability_indexer.get_stats().await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability::STATS_FAILED,
            e,
            ErrorCategory::Retryable,
        )
    })
}

/// 列出运行时能力注册表（P3：内置与外部插件平权的可查询检视闭环）。
///
/// 返回全部已注册能力及其来源；外部插件注册的能力额外标注来源插件 ID。
#[agent_command(domain = plugin, safety = Safe, call_mode = StateOnly, description = "列出运行时能力注册表")]
#[tauri::command]
pub async fn capability_registry_dump()
-> Result<Vec<CapabilityRegistrationDetailDto>, ErrorResponse> {
    Ok(axagent_harness::get_capability_registry()
        .list_with_details()
        .into_iter()
        .map(|d| CapabilityRegistrationDetailDto {
            id: d.definition.id,
            version: d.definition.version,
            contract: d.definition.contract,
            description: d.definition.description,
            origin: d.origin.as_str().to_string(),
            plugin_id: d.plugin_id,
        })
        .collect())
}

// ── 能力进化：按 kind 分发到 skill / workflow 进化引擎 ─────────────

/// 能力进化请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvolveCapabilityRequest {
    /// 能力护照 ID（如 `workflow:{template_id}` / `skill:{name}`）
    pub capability_id: String,
    /// 工作流进化反思上下文（可选；缺省走启发式变异）
    #[serde(default)]
    pub reflections: Vec<Reflection>,
}

/// 能力进化结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvolveCapabilityResult {
    pub capability_id: String,
    /// 进化是否产生有效改进
    pub improved: bool,
    /// 进化前等级
    pub old_level: CapabilityLevel,
    /// 进化后等级
    pub new_level: CapabilityLevel,
    /// 进化引擎返回的原始结果摘要（技能改进 / 工作流变异详情）
    pub detail: serde_json::Value,
}

/// 能力进化策略（由来源 × 可进化性联合决定）。
///
/// 作为纯函数提取，便于对进化边界做单元测试（不依赖 AppState）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvolutionPolicy {
    /// 拒绝进化（`evolvable = None`：外部插件只读能力）
    Reject,
    /// 就地提升等级（`Local`：内置能力 / 插件本地可写载体）
    InPlace,
    /// 生成派生副本，原护照不变（`Derived`：插件声明能力）
    Derived,
}

/// 解析能力进化策略。
///
/// 边界规则：
/// - `None` → 拒绝（任何来源都不可进化）；
/// - `Derived` + 插件来源 → 派生副本；
/// - `Local` → 就地提升；`Derived` + 内置来源（异常组合）回退就地提升。
pub(crate) fn resolve_evolution_policy(passport: &CapabilityPassportDto) -> EvolutionPolicy {
    match (passport.source, passport.evolvable) {
        (_, CapabilityEvolvability::None) => EvolutionPolicy::Reject,
        (CapabilitySource::Plugin, CapabilityEvolvability::Derived) => EvolutionPolicy::Derived,
        (CapabilitySource::Builtin, CapabilityEvolvability::Derived)
        | (_, CapabilityEvolvability::Local) => EvolutionPolicy::InPlace,
    }
}

/// 一键进化能力以提升等级。
///
/// 按能力类型分发到对应进化引擎（技能 → 技能进化引擎，工作流 → 工作流进化器），
/// 进化成功后把护照等级提升一级（L5 封顶）。适用于能力发现面板中低等级（L1/L2）
/// 能力的「进化提升」入口。
#[agent_command(domain = capability, safety = Caution, call_mode = StateInput, description = "进化能力以提升等级")]
#[tauri::command]
pub async fn capability_evolve(
    state: State<'_, AppState>,
    request: EvolveCapabilityRequest,
) -> Result<EvolveCapabilityResult, CommandError> {
    let evolve_err = |e: String| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability::EVOLVE_FAILED,
            e,
            ErrorCategory::Unrecoverable,
        )
    };

    // 1. 定位护照并记录进化前等级
    let passport =
        state.capability_indexer.get_passport(&request.capability_id).await.ok_or_else(|| {
            CommandError::new(crate::commands::error_code::capability::NOT_FOUND)
                .with_category(ErrorCategory::Unrecoverable)
                .with_detail(format!("capability '{}' not found", request.capability_id))
        })?;
    let old_level = passport.level;

    // 1.5 进化边界检查：不可进化的能力（外部插件只读能力）直接拒绝；
    //    插件声明的能力（Derived）以派生副本方式进化：产出新护照，原护照保持不变
    let policy = resolve_evolution_policy(&passport);
    if policy == EvolutionPolicy::Reject {
        return Err(CommandError::new(crate::commands::error_code::capability::NOT_EVOLVABLE)
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail("该能力不可进化（外部插件只读能力）"));
    }
    let derived = policy == EvolutionPolicy::Derived;

    // 2. 按能力类型分发进化
    let (improved, detail) = match passport.kind {
        CapabilityKind::Skill => {
            let skill_id =
                request.capability_id.strip_prefix("skill:").unwrap_or(&request.capability_id);
            evolve_skill(&state, skill_id).await.map_err(evolve_err)?
        },
        CapabilityKind::Workflow => {
            let template_id =
                request.capability_id.strip_prefix("workflow:").unwrap_or(&request.capability_id);
            // 进化器经 workflow.evolver 能力接缝获取（与 WorkEngine / 命令层同源）
            let evolver = axagent_harness::get_capability_registry()
                .get_workflow_evolver()
                .ok_or_else(|| evolve_err("workflow.evolver 接缝未注册".to_string()))?;
            let modification =
                evolver.run(template_id, &request.reflections).await.map_err(evolve_err)?;
            let improved = !modification.changes.is_empty() && modification.fitness_delta > 0.0;
            let detail = serde_json::json!({
                "generation": modification.generation,
                "fitness_delta": modification.fitness_delta,
                "changes": modification.changes.len(),
                "reasoning": modification.reasoning,
            });
            (improved, detail)
        },
        CapabilityKind::Agent => {
            // 角色 / 专家：用 TextGrad 优化系统提示词并写回载体
            evolve_agent(&state, &request.capability_id).await.map_err(evolve_err)?
        },
        other => {
            return Err(CommandError::new(crate::commands::error_code::capability::EVOLVE_FAILED)
                .with_category(ErrorCategory::Unrecoverable)
                .with_detail(format!("capability kind '{}' 暂不支持一键进化", other.as_str())));
        },
    };

    // 3. 进化成功后提升一级（L5 封顶）；
    //    Derived（插件声明能力）注册派生副本，原护照等级不变
    let new_level = old_level.promote();
    if derived {
        let mut derived_passport = passport.clone();
        derived_passport.capability_id = format!("{}:derived", request.capability_id);
        derived_passport.name = format!("{}（进化副本）", passport.name);
        derived_passport.level = new_level;
        state.capability_indexer.index_passport(&derived_passport).await.map_err(evolve_err)?;
        tracing::info!(
            capability_id = %request.capability_id,
            derived_id = %derived_passport.capability_id,
            ?old_level,
            ?new_level,
            improved,
            "🧬 插件能力派生进化完成：注册进化副本，原护照不变"
        );
    } else {
        state
            .capability_indexer
            .update_level(&request.capability_id, new_level)
            .await
            .map_err(evolve_err)?;

        tracing::info!(
            capability_id = %request.capability_id,
            ?old_level,
            ?new_level,
            improved,
            "🧬 能力进化完成：等级已提升"
        );
    }

    Ok(EvolveCapabilityResult {
        capability_id: request.capability_id,
        improved,
        old_level,
        new_level,
        detail,
    })
}

/// 执行技能进化（复用 trajectory 技能进化引擎），成功时把改进内容写回技能库。
async fn evolve_skill(
    state: &AppState,
    skill_id: &str,
) -> Result<(bool, serde_json::Value), String> {
    let skill = state
        .trajectory_storage
        .get_skill(skill_id)
        .await
        .map_err(|e| {
            ErrorResponse::from_error_with_code(
                crate::commands::error_code::capability::EVOLVE_FAILED,
                e,
                ErrorCategory::Unrecoverable,
            )
            .to_string()
        })?
        .ok_or_else(|| format!("Skill '{skill_id}' not found"))?;

    let trajectories = state.trajectory_storage.get_trajectories(Some(30)).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability::EVOLVE_FAILED,
            e,
            ErrorCategory::Unrecoverable,
        )
        .to_string()
    })?;
    let test_refs: Vec<_> = trajectories.iter().collect();

    let mut engine = state.skill_evolution_engine.lock().await;
    let result = engine.run(&skill, &test_refs).await;
    match result {
        Some(modification) => {
            let improved = modification.validation_result.as_ref().is_some_and(|v| v.success);
            if improved {
                let mut updated = skill.clone();
                updated.content = modification.new_content.clone();
                updated.quality_score = modification.confidence;
                if let Err(e) = state.trajectory_storage.save_skill(&updated).await {
                    tracing::warn!("[capability_evolve] 保存进化技能失败: {}", e);
                }
            }
            Ok((
                improved,
                serde_json::json!({
                    "reason": modification.reason,
                    "confidence": modification.confidence,
                    "quality_delta": modification.validation_result.as_ref().map(|v| v.quality_delta),
                }),
            ))
        },
        None => Ok((
            false,
            serde_json::json!({
                "reason": "Evolution did not produce a result",
                "confidence": 0.0,
            }),
        )),
    }
}

/// 执行 Agent 能力（角色/专家）进化。
///
/// Agent 能力的可进化载体是系统提示词：
/// - 角色护照（`agent_role:{id}`）→ 直接优化 `AgentRole.system_prompt` 并写回；
/// - 专家护照（`agent:{id}`）→ 解析执行载体（优先 `expert_id` 关联的机构专家，
///   其次 `agent_role` 关联的角色），优化其系统提示词并写回。
async fn evolve_agent(
    state: &AppState,
    capability_id: &str,
) -> Result<(bool, serde_json::Value), String> {
    if let Some(role_id) = capability_id.strip_prefix("agent_role:") {
        return evolve_agent_role(state, role_id).await;
    }
    if let Some(profile_id) = capability_id.strip_prefix("agent:") {
        return evolve_agent_profile(state, profile_id).await;
    }
    Err(format!("无法识别 Agent 能力载体: {capability_id}"))
}

/// 角色进化：用 TextGrad 优化角色系统提示词，写回 `agent_roles` 表。
async fn evolve_agent_role(
    state: &AppState,
    role_id: &str,
) -> Result<(bool, serde_json::Value), String> {
    let db = state.harness.db();
    let role = agent_role_repo::get_agent_role(db, role_id)
        .await
        .map_err(|e| {
            ErrorResponse::from_error_with_code(
                crate::commands::error_code::capability::EVOLVE_FAILED,
                e,
                ErrorCategory::Unrecoverable,
            )
            .to_string()
        })?
        .ok_or_else(|| format!("AgentRole '{role_id}' not found"))?;

    let feedback = build_agent_feedback(state, &role.name).await;
    let llm_provider = build_agent_llm_bridge(state).await;
    let (new_prompt, improved) =
        optimize_prompt_with_text_grad(&role.system_prompt, &feedback, llm_provider).await?;
    if !improved {
        return Ok((
            false,
            serde_json::json!({
                "kind": "agent_role",
                "roleId": role_id,
                "reason": "TextGrad 未产生有效改进",
            }),
        ));
    }

    // 局部写回 system_prompt，保留其余字段（避免重置 sort_order / created_at）
    let row = agent_roles::Entity::find_by_id(role_id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::from_error_with_code(
                crate::commands::error_code::capability::EVOLVE_FAILED,
                e,
                ErrorCategory::Unrecoverable,
            )
            .to_string()
        })?
        .ok_or_else(|| format!("AgentRole '{role_id}' not found"))?;
    let mut am: agent_roles::ActiveModel = row.into();
    am.system_prompt = Set(new_prompt.clone());
    am.updated_at = Set(axagent_harness::util_fns::now_ts());
    am.update(db).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability::EVOLVE_FAILED,
            e,
            ErrorCategory::Unrecoverable,
        )
        .to_string()
    })?;

    Ok((
        true,
        serde_json::json!({
            "kind": "agent_role",
            "roleId": role_id,
            "promptDelta": prompt_delta(&role.system_prompt, &new_prompt),
        }),
    ))
}

/// 专家进化：定位专家执行载体（机构专家 / 角色）的系统提示词，优化后写回对应表。
async fn evolve_agent_profile(
    state: &AppState,
    profile_id: &str,
) -> Result<(bool, serde_json::Value), String> {
    let db = state.harness.db();
    let profile = agent_profile_repo::get_agent_profile(db, profile_id).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability::EVOLVE_FAILED,
            e,
            ErrorCategory::Unrecoverable,
        )
        .to_string()
    })?;

    // 载体一：关联的机构专家（expert_id → agency_experts.system_prompt）
    if let Some(expert_id) = profile.expert_id.as_deref() {
        let expert = agency_experts::Entity::find_by_id(expert_id)
            .one(db)
            .await
            .map_err(|e| {
                ErrorResponse::from_error_with_code(
                    crate::commands::error_code::capability::EVOLVE_FAILED,
                    e,
                    ErrorCategory::Unrecoverable,
                )
                .to_string()
            })?
            .ok_or_else(|| format!("AgencyExpert '{expert_id}' not found"))?;

        let feedback = build_agent_feedback(state, &profile.name).await;
        let old_prompt = expert.system_prompt.clone();
        let llm_provider = build_agent_llm_bridge(state).await;
        let (new_prompt, improved) =
            optimize_prompt_with_text_grad(&old_prompt, &feedback, llm_provider).await?;
        if !improved {
            return Ok((
                false,
                serde_json::json!({
                    "kind": "agent_profile",
                    "profileId": profile_id,
                    "carrier": "agency_expert",
                    "expertId": expert_id,
                    "reason": "TextGrad 未产生有效改进",
                }),
            ));
        }

        let mut am: agency_experts::ActiveModel = expert.into();
        am.system_prompt = Set(new_prompt.clone());
        am.update(db).await.map_err(|e| {
            ErrorResponse::from_error_with_code(
                crate::commands::error_code::capability::EVOLVE_FAILED,
                e,
                ErrorCategory::Unrecoverable,
            )
            .to_string()
        })?;

        return Ok((
            true,
            serde_json::json!({
                "kind": "agent_profile",
                "profileId": profile_id,
                "carrier": "agency_expert",
                "expertId": expert_id,
                "promptDelta": prompt_delta(&old_prompt, &new_prompt),
            }),
        ));
    }

    // 载体二：关联的角色（agent_role → agent_roles.system_prompt）
    if let Some(role_id) = profile.agent_role.as_deref() {
        let (improved, mut detail) = evolve_agent_role(state, role_id).await?;
        detail["profileId"] = serde_json::Value::String(profile_id.to_string());
        detail["carrier"] = serde_json::Value::String("agent_role".to_string());
        return Ok((improved, detail));
    }

    Ok((
        false,
        serde_json::json!({
            "kind": "agent_profile",
            "profileId": profile_id,
            "reason": "该专家未绑定可进化的系统提示词载体（expert_id / agent_role）",
        }),
    ))
}

/// 尝试从 DB 构建 LLM bridge，对齐技能/工作流进化策略：
/// 存在启用的 provider → 注入 LLM 语义梯度；否则返回 None，回退本地启发式。
async fn build_agent_llm_bridge(state: &AppState) -> Option<axagent_agent::ProviderLlmBridge> {
    axagent_runtime::llm_bridge::build_llm_bridge_from_db_with(
        state.harness.master_key(),
        state.harness.provider_registry(),
        None,
        None,
    )
    .await
}

/// 用独立的 TextGrad 引擎对系统提示词做单轮梯度优化。
///
/// 使用独立实例，避免污染 AppState 中共享的全局 `text_grad_engine`
/// （其计算图节点会随 `run_text_grad_optimize` 调用累积）。
/// 传入可用的 LLM bridge 时注入语义梯度；否则用内置启发式 provider（纯本地）。
/// 返回 (新提示词, 是否产生有效修改)。
async fn optimize_prompt_with_text_grad(
    current: &str,
    feedback: &str,
    llm_provider: Option<axagent_agent::ProviderLlmBridge>,
) -> Result<(String, bool), String> {
    let mut graph = ComputationGraph::new();
    graph.add_node(
        ComputationNode::new(NodeType::Prompt, current.to_string()).with_id("system_prompt"),
    );
    let mut engine = TextGradEngine::new(graph, TextGradConfig::default());
    if let Some(bridge) = llm_provider {
        engine.set_provider(bridge);
    }

    engine.backward_text_grad(feedback).await.map_err(|e| format!("提示词梯度计算失败: {e}"))?;

    let modifications = engine.apply_gradients();

    let new_prompt = engine
        .graph()
        .get_node("system_prompt")
        .map(|n| n.content.clone())
        .unwrap_or_else(|| current.to_string());

    let improved = !modifications.is_empty() && new_prompt != current;
    Ok((new_prompt, improved))
}

/// 从轨迹库构建 Agent 能力进化的反馈文本。
///
/// 先按 Agent 名称过滤轨迹（topic / summary 文本匹配，与技能进化同款策略），
/// 确保反馈只反映该 Agent 的执行情况；若无匹配则回退全局最近轨迹，避免
/// 证据不足导致进化空转。统计执行结果（成功/失败/部分完成）并提取失败案例，
/// 作为 TextGrad 的反向传播信号（feedback）。
async fn build_agent_feedback(state: &AppState, agent_name: &str) -> String {
    let trajectories =
        state.trajectory_storage.get_trajectories(Some(200)).await.unwrap_or_default();

    // 按 Agent 名称过滤：优先精确匹配结构化 `agent_name` 字段（记录时即带标识），
    // 无结构化的旧轨迹再回退 topic / summary 文本匹配，双通道聚合同一 Agent 证据。
    let name = agent_name.to_lowercase();
    let filtered: Vec<&_> = trajectories
        .iter()
        .filter(|t| {
            t.agent_name.as_deref().map(|n| n.to_lowercase().contains(&name)).unwrap_or(false)
                || t.topic.to_lowercase().contains(&name)
                || t.summary.to_lowercase().contains(&name)
        })
        .collect();
    let (evidence, fallback) = if filtered.is_empty() {
        (trajectories.iter().collect::<Vec<_>>(), true)
    } else {
        (filtered, false)
    };

    let total = evidence.len();

    let success = evidence.iter().filter(|t| t.outcome == TrajectoryOutcome::Success).count();
    let failure = evidence.iter().filter(|t| t.outcome == TrajectoryOutcome::Failure).count();
    let partial = evidence.iter().filter(|t| t.outcome == TrajectoryOutcome::Partial).count();

    let mut feedback = String::new();
    feedback.push_str(&format!("Execution feedback for agent capability '{}'.\n", agent_name));
    if fallback {
        feedback.push_str(
            "No direct traces matched this agent name; using recent global trajectories as fallback.\n",
        );
    }
    feedback.push_str(&format!(
        "Tracked executions: total {}, success {}, failure {}, partial {}.\n",
        total, success, failure, partial
    ));
    if total > 0 {
        let rate = success as f64 / total as f64;
        feedback.push_str(&format!("Overall success rate: {:.2}.\n", rate));
    }

    for t in evidence.iter().filter(|t| t.outcome == TrajectoryOutcome::Failure).take(5) {
        feedback.push_str(&format!("Failed task: {} | summary: {}\n", t.topic, t.summary));
    }

    if failure == 0 && partial == 0 {
        feedback.push_str(
            "No significant errors observed. Focus on improving efficiency, clarity, and robustness.\n",
        );
    }

    feedback
}

/// 生成提示词变更摘要（新旧各前 120 字符），用于进化结果详情展示。
fn prompt_delta(old: &str, new: &str) -> String {
    let old_preview: String = old.chars().take(120).collect();
    let new_preview: String = new.chars().take(120).collect();
    format!("{old_preview} → {new_preview}")
}

// ── 自我进化：产物护照/图谱注册（T0.11）──────────────────────────

/// 注册进化/补齐产物为能力护照并同步进工作流图谱（T0.11）。
///
/// 通道一（能力补齐）与通道二（能力偏弱进化改进）的产物统一在此注册：
/// 1. 生成 `evolution:workflow:{product_id}` 能力护照（`auto_evolved` 标签
///    + `/evolution/` 前缀），注册进能力索引（L2 混合检索可见）；
/// 2. 同步进工作流图谱（L3 `system_workflow_graph_router` 可见），
///    使下一轮用户输入的路由决策可命中该产物。
///
/// 调用方负责先征求用户显式同意（铁律），再调用本函数完成注册。
/// 失败时返回带 `GAP_PROPOSAL_PENDING` 错误码的 `CommandError`（前端按码翻译）。
pub(crate) async fn register_evolution_product(
    state: &AppState,
    product_id: &str,
    display_name: &str,
    description: &str,
) -> Result<(), CommandError> {
    // 1. 生成能力护照（L2 混合检索可见）
    let capability_id = format!("evolution:workflow:{product_id}");
    let route_tag = format!("route:/evolution/auto_generated/workflow/{product_id}");
    let passport = CapabilityPassportDto {
        capability_id: capability_id.clone(),
        name: display_name.to_string(),
        description: description.to_string(),
        kind: CapabilityKind::Workflow,
        domain: CapabilityDomain::General,
        sub_category: "auto_generated".to_string(),
        visibility: Visibility::Public,
        tags: vec![
            "auto_evolved".to_string(),
            route_tag,
            "evolvable".to_string(),      // 标记为可进化
            "capability_gap".to_string(), // 标记来源：能力补齐
        ],
        ..Default::default()
    };
    state.capability_indexer.index_passport(&passport).await.map_err(|e| {
        CommandError::new(axagent_harness::error_codes::cognitive::GAP_PROPOSAL_PENDING)
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail(format!("能力护照注册失败: {e}"))
    })?;
    // 2. 同步进工作流图谱（L3 图谱路由可见）
    state
        .cognitive_router
        .sync_evolved_workflow("general", "auto_generated", product_id, display_name)
        .await;
    tracing::info!(
        capability_id = %capability_id,
        "🗺️ 进化产物已注册护照并同步进工作流图谱"
    );
    Ok(())
}

// ── 能力域注册表（P2：覆盖层 —— 域从「只读契约」升级为「可治理的一层」）──
//
// 设计要点（见 `docs/plans/PLAN-domain-single-source.md` §9.3）：
//
// - **声明**：内置默认永远是 `DOMAIN_NODES`（编译期）；本层只写**覆盖**。
// - **不开放 id 空间**：9 个 id 只读、不可增删（`capability.rs` 明令禁止自定义域）。
// - **验收标准不是「有 UI」**，而是「改完之后某个代码路径的行为真的变了」——
//   故这两个命令的产物必须被 L1 分类器 prompt / L1 路由 / 能力过滤三处真实消费
//   （接线见 `init/state.rs`、`domain_router.rs`、`capability_filter_impl.rs`）。

/// 追加别名的条数上限。
///
/// 不是为了省资源（数量级微不足道），而是防「把别名当标签库用」：
/// 别名会进入每次用户输入 / LLM 输出的解析路径，几百条近义写法里几乎必然出现
/// 语义重叠，而重叠的后果是**解析顺序决定了命中哪个域**（静默、难查）。
const MAX_EXTRA_ALIASES: usize = 32;

/// 单条别名的字符数上限（按 `char` 计，中文别名不会被按字节误伤）。
const MAX_ALIAS_CHARS: usize = 64;

/// 「能力域」注册表条目 —— 前端「能力域」面板的数据源（内置声明 ∪ 覆盖层）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDomainEntryDto {
    /// 域 id（协议 slug）。**只读**：由 `CapabilityDomain` 枚举决定，不可增删。
    pub id: String,
    /// i18n 显示名 key（`capabilityDomain.<id>`，派生自 `DomainNode::label_key()`）。
    pub label_key: String,
    /// 导航路径（`None` = 内部域，不进入导航）。
    pub nav_path: Option<String>,
    /// 导航顺序（`None` = 内部域）。
    pub nav_order: Option<u8>,
    /// 是否内部域（`system`）。
    pub is_system: bool,
    /// 是否允许被停用（`general` / `system` 不可，理由见 [`axagent_harness::toggle_block_reason`]）。
    pub toggleable: bool,
    /// 不可停用的**原因码**（`None` = 可停用）；前端按码查 i18n（11 语言）。
    pub toggle_block_reason: Option<String>,
    /// 当前是否启用（合并视图：覆盖层 ∪ 内置默认）。
    pub enabled: bool,
    /// 是否有覆盖行（UI 用它区分「内置默认」与「用户改过」；也让「重置」按钮有状态可依）。
    pub has_override: bool,
    /// 内置别名（**只读**；27 条存量兼容别名，不可删 —— PLAN §7.2）。
    pub builtin_aliases: Vec<String>,
    /// 追加别名（用户可改；语义是「在内置之上追加」，不是替换）。
    pub extra_aliases: Vec<String>,
    /// 有效别名 = 内置 ∪ 追加（UI 展示用，避免前端自己合并出一份副本）。
    pub effective_aliases: Vec<String>,
}

/// 由「声明节点 + 覆盖集」构造条目。
///
/// 刻意做成纯函数：`list`（读库快照）与 `update`（落库后回读）共用它，
/// 保证两条路径**不会渲染出形状不同的条目**。
fn build_domain_entry(
    node: &axagent_harness::DomainNode,
    overrides: &[axagent_harness::DomainOverride],
) -> CapabilityDomainEntryDto {
    CapabilityDomainEntryDto {
        id: node.slug().to_string(),
        label_key: node.label_key(),
        nav_path: node.nav_path.map(|p| p.to_string()),
        nav_order: node.nav_order,
        is_system: node.domain.is_system(),
        toggleable: axagent_harness::is_toggleable(node.domain),
        toggle_block_reason: axagent_harness::toggle_block_reason(node.domain)
            .map(|s| s.to_string()),
        enabled: axagent_harness::enabled_with(overrides, node.domain),
        has_override: axagent_harness::has_override_with(overrides, node.domain),
        builtin_aliases: node.aliases.iter().map(|a| (*a).to_string()).collect(),
        extra_aliases: axagent_harness::extra_aliases_with(overrides, node.domain),
        effective_aliases: axagent_harness::effective_aliases_with(overrides, node.domain),
    }
}

/// 列出「能力域」注册表（内置声明 ∪ 覆盖层）。
///
/// ⚠ 本命令**不修改**任何全局状态（不顺手 apply 覆盖层）：
/// 读命令带副作用会让并发请求看到「别人触发的刷新」，且难以推理。
/// 它取数据库快照后交给 harness 的 `*_with` 纯函数族渲染 ——
/// 与运行时路径用**同一套判据**，不产生第二份「启用语义」。
#[agent_command(domain = capability, safety = Safe, call_mode = StateOnly, description = "列出能力域注册表")]
#[tauri::command]
pub async fn list_capability_domain_registry(
    state: State<'_, AppState>,
) -> Result<Vec<CapabilityDomainEntryDto>, CommandError> {
    let db = state.harness.db();
    let (overrides, unknown) = domain_override_repo::snapshot_for_view(db).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability_domain::LIST_FAILED,
            e,
            ErrorCategory::Retryable,
        )
    })?;

    // 坏行（`domain` 解析不出枚举）不表现为「已覆盖」，但也不该无声无息：
    // 用户在界面上会看到「无覆盖」，日志里必须能查到原因。
    if !unknown.is_empty() {
        tracing::warn!(
            ?unknown,
            "capability_domain_overrides 存在无法解析的域行（界面不会显示为「已覆盖」）"
        );
    }

    Ok(axagent_harness::DOMAIN_NODES.iter().map(|n| build_domain_entry(n, &overrides)).collect())
}

/// 更新能力域覆盖的请求（**局部更新**：未提供的字段保持原值）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCapabilityDomainRequest {
    /// 域 id（协议 slug；解析成 `CapabilityDomain` 枚举后才会被接受）。
    pub domain: String,
    /// 启用 / 停用（`None` = 不改）。
    ///
    /// `Some(false)` 对 `general` / `system` 会被拒绝（见 `is_toggleable`）。
    #[serde(default)]
    pub enabled: Option<bool>,
    /// 追加别名**整体替换**（`None` = 不改；`Some([])` = 清空追加别名）。
    #[serde(default)]
    pub extra_aliases: Option<Vec<String>>,
}

/// 更新一个能力域的覆盖（启用状态 / 追加别名），落库并**立刻**刷新运行时覆盖层。
///
/// # 为什么返回值是「落库后回读」而不是「我们的意图」
///
/// 写命令返回自己构造的 DTO 是最容易的写法，但它掩盖两类失败：
/// ① 写入被静默改写（列类型 / 触发器 / 并发覆盖）；② 落库的行在回读路径上解析失败。
/// 回读一次多一条查询，换来「界面显示的必然是**真实可读的当前状态**」。
#[agent_command(domain = capability, safety = Caution, call_mode = StateInput, description = "更新能力域覆盖")]
#[tauri::command]
pub async fn update_capability_domain(
    state: State<'_, AppState>,
    request: UpdateCapabilityDomainRequest,
) -> Result<CapabilityDomainEntryDto, CommandError> {
    let db = state.harness.db();

    // ── 1. 域必须存在：解析成**枚举**（枚举是单点真相源，不是字符串表）──
    //
    // 刻意不用「与 9 个 slug 字符串比对」：那会在这里造出第 N 份 id 副本，
    // 且新增域时漏改只是静默拒绝写入。解析枚举则由编译器强制走查。
    let domain: CapabilityDomain = request.domain.trim().to_lowercase().parse().map_err(|_| {
        CommandError::new(crate::commands::error_code::capability_domain::UNKNOWN)
            .with_category(ErrorCategory::Validation)
            .with_detail(format!("未知的能力域标识：{}", request.domain))
    })?;
    let node = axagent_harness::node_of(domain);

    // ── 2. 停用守卫（与运行时读侧**同一判据** `is_toggleable`）──
    if request.enabled == Some(false) && !axagent_harness::is_toggleable(domain) {
        return Err(CommandError::new(
            crate::commands::error_code::capability_domain::NOT_TOGGLEABLE,
        )
        .with_category(ErrorCategory::Validation)
        .with_detail(
            axagent_harness::toggle_block_reason(domain).unwrap_or("not_toggleable").to_string(),
        ));
    }

    // ── 3. 读当前覆盖 → 合成新值（局部更新）──
    let (current, _unknown) = domain_override_repo::snapshot_for_view(db).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability_domain::LIST_FAILED,
            e,
            ErrorCategory::Retryable,
        )
    })?;

    let next_enabled =
        request.enabled.unwrap_or_else(|| axagent_harness::enabled_with(&current, domain));
    let next_extra = match &request.extra_aliases {
        Some(list) => validate_extra_aliases(list, domain, &current)?,
        None => axagent_harness::extra_aliases_with(&current, domain),
    };

    // ── 4. 落库 → 灌运行时（让改动**不必重启**即刻影响三个消费端）──
    domain_override_repo::upsert_override(db, node.slug(), next_enabled, &next_extra)
        .await
        .map_err(|e| {
            ErrorResponse::from_error_with_code(
                crate::commands::error_code::capability_domain::UPDATE_FAILED,
                e,
                ErrorCategory::Unrecoverable,
            )
        })?;
    let applied = domain_override_repo::load_into_runtime(db).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability_domain::UPDATE_FAILED,
            e,
            ErrorCategory::Unrecoverable,
        )
    })?;

    tracing::info!(
        domain = node.slug(),
        enabled = next_enabled,
        extra_aliases = next_extra.len(),
        applied,
        "能力域覆盖已更新（L1 分类器 prompt / L1 路由 / 能力过滤三处即刻生效）"
    );

    // ── 5. 回读合并视图（返回**持久化后**的真实状态，而不是我们的意图）──
    let (after, _) = domain_override_repo::snapshot_for_view(db).await.map_err(|e| {
        ErrorResponse::from_error_with_code(
            crate::commands::error_code::capability_domain::LIST_FAILED,
            e,
            ErrorCategory::Retryable,
        )
    })?;
    Ok(build_domain_entry(node, &after))
}

/// 校验并规范化追加别名。
///
/// # 规则与理由
///
/// | 规则 | 违反后果（若不拦） |
/// |---|---|
/// | 非空、≤[`MAX_ALIAS_CHARS`] 字符、**不含空白** | 解析入口是「整串比对」，含空白的别名只在「用户整句恰好等于它」时命中 —— 是个陷阱而非功能 |
/// | 条数 ≤ [`MAX_EXTRA_ALIASES`] | 别名越多，语义重叠概率越高，而重叠时**解析顺序决定命中域**（静默、难查） |
/// | **不得等于任何规范 id** | 会**遮蔽**那个域：`finance` 作为别名后，输入 `finance` 可能先命中别名持有者 ⇒ 该域永远解析不出来（与 `domain_registry::tests::test_aliases_do_not_shadow_canonical_ids` 同一条不变量） |
/// | **不得与其它域的有效别名冲突** | `resolve_enabled_domain` 按协议顺序取**第一个**命中 ⇒ 冲突时后者永久不可达，且不报错 |
///
/// 空串 / 纯空白项被**静默丢弃**（它们是 UI 标签输入框的常见残留，不含信息量）；
/// 其余违规**逐条报错**，不回显全部规则（错误码负责翻译，细节负责定位）。
fn validate_extra_aliases(
    raw: &[String],
    target: CapabilityDomain,
    overrides: &[axagent_harness::DomainOverride],
) -> Result<Vec<String>, CommandError> {
    let invalid = |detail: String| {
        CommandError::new(crate::commands::error_code::capability_domain::ALIAS_INVALID)
            .with_category(ErrorCategory::Validation)
            .with_detail(detail)
    };
    let conflict = |detail: String| {
        CommandError::new(crate::commands::error_code::capability_domain::ALIAS_CONFLICT)
            .with_category(ErrorCategory::Validation)
            .with_detail(detail)
    };

    // 规范化：trim + 原序去重（大小写不敏感，与解析侧同口径）
    let mut out: Vec<String> = Vec::new();
    for a in raw {
        let t = a.trim();
        if t.is_empty() {
            continue;
        }
        if out.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            continue;
        }
        out.push(t.to_string());
    }

    if out.len() > MAX_EXTRA_ALIASES {
        return Err(invalid(format!("追加别名最多 {MAX_EXTRA_ALIASES} 条，收到 {}", out.len())));
    }

    // 其它域的**有效别名**（内置 ∪ 追加）—— 一次性算好，避免 O(n²) 反复读全局
    let mut taken_by_others: Vec<(String, &'static str)> = Vec::new();
    for other in axagent_harness::DOMAIN_NODES {
        if other.domain == target {
            continue;
        }
        for a in axagent_harness::effective_aliases_with(overrides, other.domain) {
            taken_by_others.push((a, other.slug()));
        }
    }

    for a in &out {
        if a.chars().count() > MAX_ALIAS_CHARS {
            return Err(invalid(format!("别名「{a}」超过 {MAX_ALIAS_CHARS} 个字符")));
        }
        if a.chars().any(|c| c.is_whitespace()) {
            return Err(invalid(format!(
                "别名「{a}」含空白 —— 别名按「整串比对」解析，含空白者几乎永不命中"
            )));
        }
        if axagent_harness::DOMAIN_NODES.iter().any(|n| n.slug() == a.to_lowercase()) {
            return Err(conflict(format!(
                "别名「{a}」与规范域 id 同名 ⇒ 会遮蔽该域（该域将永远解析不出来）"
            )));
        }
        if let Some((_, owner)) = taken_by_others.iter().find(|(x, _)| x.eq_ignore_ascii_case(a)) {
            return Err(conflict(format!("别名「{a}」已被域 {owner} 占用")));
        }
    }

    Ok(out)
}

// ── 单元测试：进化边界 ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个仅关注来源 × 可进化性的护照（其余字段用默认值）。
    fn passport(
        source: CapabilitySource,
        evolvable: CapabilityEvolvability,
    ) -> CapabilityPassportDto {
        CapabilityPassportDto {
            capability_id: "cap.test".into(),
            source,
            evolvable,
            ..Default::default()
        }
    }

    #[test]
    fn none_always_rejects_regardless_of_source() {
        // 外部插件只读能力：拒绝进化
        assert_eq!(
            resolve_evolution_policy(&passport(
                CapabilitySource::Plugin,
                CapabilityEvolvability::None
            )),
            EvolutionPolicy::Reject,
        );
        // 内置能力也不可进化（防御性：None 永远拒绝）
        assert_eq!(
            resolve_evolution_policy(&passport(
                CapabilitySource::Builtin,
                CapabilityEvolvability::None
            )),
            EvolutionPolicy::Reject,
        );
    }

    #[test]
    fn plugin_derived_yields_copy_preserving_original() {
        // 插件声明能力：派生副本，原护照不变
        assert_eq!(
            resolve_evolution_policy(&passport(
                CapabilitySource::Plugin,
                CapabilityEvolvability::Derived
            )),
            EvolutionPolicy::Derived,
        );
    }

    #[test]
    fn local_evolves_in_place() {
        // 内置能力：就地提升等级
        assert_eq!(
            resolve_evolution_policy(&passport(
                CapabilitySource::Builtin,
                CapabilityEvolvability::Local
            )),
            EvolutionPolicy::InPlace,
        );
        // 插件本地可写载体（技能 / Agent）：就地提升
        assert_eq!(
            resolve_evolution_policy(&passport(
                CapabilitySource::Plugin,
                CapabilityEvolvability::Local
            )),
            EvolutionPolicy::InPlace,
        );
    }

    #[test]
    fn builtin_derived_anomaly_falls_back_to_in_place() {
        // 防御性：内置来源 + Derived（异常组合）回退就地提升
        assert_eq!(
            resolve_evolution_policy(&passport(
                CapabilitySource::Builtin,
                CapabilityEvolvability::Derived
            )),
            EvolutionPolicy::InPlace,
        );
    }
}
