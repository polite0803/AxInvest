// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求实现命令层（P2 转化链）
//!
//! 打通「发现 → 实现」断点（全链路审计断点 2）：把需求线索一键转化为
//! 可执行的工作流模板，复用 workflow_templates + WorkEngine 底座，
//! 模板生成模式与 `capability_gap_workflow.rs` 一致（Trigger + LLM + End）。
//!
//! - 状态机：`opc_update_lead_status`（new → evaluated → contacted → won/lost）
//! - 转化：`opc_convert_lead_to_workflow`（生成模板 + 回灌能力索引 + 回写关联）
//! - 执行：`opc_run_lead_workflow`（启动 WorkEngine 执行并写 implemented_at）
//!
//! 设计决策：
//! - 不引入 project/work_item 新实体（OPC 后端已瘦身，工作流模板即工单）
//! - route_path L1 用 `/automation/`（CapabilityDomain 合法值，避免前端"未分类"）
//! - 错误码复用 opc_setup::INTERNAL / common::INVALID_INPUT，不新增 11 语言翻译
//! - 转化不改 status（status 语义归状态机），只写 linked_workflow_id

use crate::AppState;
use crate::commands::error_code::common as common_err;
use crate::commands::error_code::opc_setup as opc_setup_err;
use axagent_dao::repo::workflow_template::{
    build_active_model_from_data, insert_workflow_template,
};
use axagent_harness::types::DemandLeadDto;
use axagent_harness::util_fns::truncate_to_char_boundary;
use axagent_harness::workflow_types::WorkflowTemplateData;
use tauri::State;

/// 模板描述/标题的字节截断上限（防线索标题超长撑爆列表 UI）
const MAX_TEXT_BYTES: usize = 120;

/// 更新线索生命周期状态
///
/// 合法迁移：new→evaluated/contacted/lost、evaluated→contacted/lost、
/// contacted→won/lost；won/lost 为终态；同状态重复设置为幂等成功。
#[tauri::command]
pub async fn opc_update_lead_status(
    state: State<'_, AppState>,
    lead_id: String,
    status: String,
) -> Result<DemandLeadDto, String> {
    axagent_dao::repo::opc_demand::update_lead_status(state.harness.db(), &lead_id, &status)
        .await
        .map_err(err)
}

/// 把需求线索转化为实现工作流模板
///
/// 流程：读线索 → 生成 Trigger(Manual) + LLMNode（需求简报注入）+ End 模板
/// → 写入 workflow_templates → 回灌能力索引（否则本会话内不可路由）
/// → 回写线索 linked_workflow_id。
///
/// 已转化的线索重复调用返回错误（前端按 linkedWorkflowId 禁用按钮）。
#[tauri::command]
pub async fn opc_convert_lead_to_workflow(
    state: State<'_, AppState>,
    lead_id: String,
) -> Result<WorkflowTemplateData, String> {
    let db = state.harness.db();
    let lead = axagent_dao::repo::opc_demand::get_lead(db, &lead_id).await.map_err(err)?;

    if let Some(wid) = &lead.linked_workflow_id {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            common_err::INVALID_INPUT,
            format!("线索已转化，工作流 ID: {wid}"),
        ));
    }

    let template = generate_lead_workflow_template(&lead);
    let active_model = build_active_model_from_data(&template);
    insert_workflow_template(db, active_model).await.map_err(|e| {
        crate::commands::error::ErrorResponse::err_with_detail(
            opc_setup_err::INTERNAL,
            format!("工作流模板创建失败: {e}"),
        )
    })?;

    // 回灌能力索引：不回灌则刚生成的模板在本会话内不可路由
    crate::commands::workflow_template::sync_template_passport(&state, &template).await;

    axagent_dao::repo::opc_demand::link_lead_to_workflow(db, &lead_id, &template.id)
        .await
        .map_err(err)?;

    tracing::info!(lead_id, workflow_id = %template.id, "[opc_demand] 线索已转化为工作流");
    Ok(template)
}

/// 启动线索实现工作流的执行
///
/// 前置：线索已完成转化（linked_workflow_id 非空）。启动的输入为需求简报
/// （与 LLM 节点 prompt 同源），成功后回写 implemented_at。
#[tauri::command]
pub async fn opc_run_lead_workflow(
    state: State<'_, AppState>,
    lead_id: String,
) -> Result<String, String> {
    let lead =
        axagent_dao::repo::opc_demand::get_lead(state.harness.db(), &lead_id).await.map_err(err)?;

    let workflow_id = lead.linked_workflow_id.clone().ok_or_else(|| {
        crate::commands::error::ErrorResponse::err_with_detail(
            common_err::INVALID_INPUT,
            "线索尚未转化为工作流，请先调用 opc_convert_lead_to_workflow",
        )
    })?;

    let engine = state.work_engine.clone();
    let execution_id = engine
        .start_workflow(
            &workflow_id,
            serde_json::json!({ "lead_brief": build_lead_brief(&lead) }),
            None,
        )
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    axagent_dao::repo::opc_demand::mark_lead_implemented(state.harness.db(), &lead_id)
        .await
        .map_err(err)?;

    tracing::info!(lead_id, workflow_id, execution_id, "[opc_demand] 线索实现工作流已启动");
    Ok(execution_id)
}

/// 从线索生成最小可用的实现工作流模板（Trigger + LLM + End）
fn generate_lead_workflow_template(lead: &DemandLeadDto) -> WorkflowTemplateData {
    use axagent_harness::capability::Visibility;
    use axagent_harness::workflow_types::{
        EdgeType, EndNode, EndNodeConfig, LLMNode, LLMNodeConfig, Position, RetryConfig,
        TriggerConfig, TriggerNode, TriggerType, WorkflowEdge, WorkflowNode, WorkflowNodeBase,
    };
    use chrono::Utc;

    let now = Utc::now().timestamp_millis();
    let brief = build_lead_brief(lead);
    let title = truncate_to_char_boundary(&lead.title, MAX_TEXT_BYTES);

    let trigger_base = WorkflowNodeBase {
        id: "trigger".to_string(),
        title: "开始".to_string(),
        description: Some("用户触发".to_string()),
        position: Position { x: 100.0, y: 80.0 },
        retry: RetryConfig::default(),
        timeout: None,
        enabled: true,
        parent_id: None,
        compensation: None,
        continue_on_fail: false,
    };

    let llm_base = WorkflowNodeBase {
        id: "llm_implement".to_string(),
        title: format!("需求实现: {title}"),
        description: Some(truncate_to_char_boundary(&lead.description, MAX_TEXT_BYTES).to_string()),
        position: Position { x: 400.0, y: 80.0 },
        retry: RetryConfig::default(),
        timeout: None,
        enabled: true,
        parent_id: None,
        compensation: None,
        continue_on_fail: false,
    };

    let end_base = WorkflowNodeBase {
        id: "end".to_string(),
        title: "结束".to_string(),
        description: Some("输出实施方案".to_string()),
        position: Position { x: 700.0, y: 80.0 },
        retry: RetryConfig::default(),
        timeout: None,
        enabled: true,
        parent_id: None,
        compensation: None,
        continue_on_fail: false,
    };

    let llm_config = LLMNodeConfig {
        model: String::new(),
        prompt: format!(
            "你是 OPC（一人公司）的需求实现助手。请针对以下需求线索产出实施方案，\
             包含：①需求理解与范围边界 ②实施方案（分步骤）③交付物清单 \
             ④工作量与报价建议 ⑤风险与前提。只使用需求简报中的事实，不得编造。\n\n{brief}"
        ),
        messages: None,
        temperature: Some(0.7),
        max_tokens: Some(2048),
        tools: None,
        functions: None,
        consistency_check: None,
        max_context_tokens: None,
        reserved_output_tokens: None,
    };

    WorkflowTemplateData {
        id: format!("demand:lead:{}", lead.id),
        name: format!("需求实现: {title}"),
        description: Some(truncate_to_char_boundary(&lead.description, MAX_TEXT_BYTES).to_string()),
        icon: "target".to_string(),
        tags: vec!["opc_demand".to_string(), "demand_implement".to_string()],
        version: 1,
        is_preset: false,
        is_editable: true,
        is_public: false,
        visibility: Visibility::Public,
        trigger_config: Some(TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({}),
        }),
        nodes: vec![
            WorkflowNode::Trigger(TriggerNode {
                base: trigger_base,
                config: TriggerConfig {
                    trigger_type: TriggerType::Manual,
                    config: serde_json::json!({}),
                },
            }),
            WorkflowNode::Llm(LLMNode { base: llm_base, config: llm_config }),
            WorkflowNode::End(EndNode {
                base: end_base,
                config: EndNodeConfig { output_var: None },
            }),
        ],
        edges: vec![
            WorkflowEdge {
                id: "e1".to_string(),
                source: "trigger".to_string(),
                source_handle: None,
                target: "llm_implement".to_string(),
                target_handle: None,
                edge_type: EdgeType::Direct,
                label: None,
            },
            WorkflowEdge {
                id: "e2".to_string(),
                source: "llm_implement".to_string(),
                source_handle: None,
                target: "end".to_string(),
                target_handle: None,
                edge_type: EdgeType::Direct,
                label: None,
            },
        ],
        input_schema: None,
        output_schema: None,
        variables: vec![],
        error_config: None,
        error_workflow_id: None,
        tool_defs: vec![],
        mission_hash: None,
        cluster_id: Some("demand_implement".to_string()),
        route_path: Some(format!("/automation/demand/{}", lead.id)),
        created_at: now,
        updated_at: now,
    }
}

/// 构建需求简报（转化 prompt 与执行输入共用，保证单一事实源）
fn build_lead_brief(lead: &DemandLeadDto) -> String {
    let budget = match (lead.budget_min, lead.budget_max) {
        (Some(min), Some(max)) => {
            format!("{} {} – {} {}", min, lead.budget_currency, max, lead.budget_currency)
        },
        (Some(v), None) | (None, Some(v)) => format!("{v} {}", lead.budget_currency),
        _ => "未提供".to_string(),
    };
    format!(
        "【需求简报】\n标题: {}\n描述: {}\n来源平台: {}\n需求类型: {}\n预算: {}\n商业价值分: {:.0}（{}）\n原文链接: {}\n线索 ID: {}",
        lead.title,
        if lead.description.is_empty() {
            "（无描述）"
        } else {
            &lead.description
        },
        lead.platform,
        lead.demand_type,
        budget,
        lead.commercial_value_score,
        lead.opportunity_level,
        lead.source_url.as_deref().unwrap_or("（无）"),
        lead.id,
    )
}

/// DAO 错误 → 命令层错误串（走错误码映射层）
fn err(e: axagent_harness::core_error::AxAgentError) -> String {
    String::from(crate::commands::error::ErrorResponse::from_error(
        e,
        crate::commands::error::ErrorCategory::Unrecoverable,
    ))
}
