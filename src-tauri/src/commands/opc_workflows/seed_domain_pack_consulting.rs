// SPDX-License-Identifier: AGPL-3.0-only

//! 产业咨询流程域包工作流模板种子化（v4 丰富拓扑：LLM 条件门 + 修正分支 + 汇合）。
//! 模板 ID：consulting_harness_workflow

use axagent_harness::capability::Visibility;
use axagent_harness::workflow_types::{EdgeType, TriggerConfig, TriggerType, WorkflowTemplateData};
use sea_orm::DatabaseConnection;

use super::seed_domain_helpers::*;

const TEMPLATE_ID: &str = "consulting_harness_workflow";
const TEMPLATE_VERSION: i32 = 4;

pub async fn seed_domain_pack_consulting_workflow_template(
    db: &DatabaseConnection,
) -> Result<(), String> {
    let should_seed = super::check_template_version(db, TEMPLATE_ID, TEMPLATE_VERSION).await?;
    if !should_seed {
        return Ok(());
    }

    let nodes = vec![
        make_trigger(0.0, 0.0),
        make_agent_node(
            "step_consulting",
            "行业扫描",
            "你是行业扫描专家。执行「行业扫描」：结合上游输入，输出结构化 JSON 结果（含关键指标、结论与建议）。",
            vec![td("OpcSearchWiki"), td("WebSearch")],
            None,
            "step_consulting",
            0.0,
            180.0,
        ),
        make_agent_node_full(
            "step2_consulting",
            "进入评估",
            "你是进入评估专家。执行「进入评估」：结合上游输入，输出结构化 JSON 结果（含关键指标、结论与建议）。",
            vec![td("OpcSearchWiki"), td("OpcGetDashboard")],
            None,
            "step2_consulting",
            vec![("input", "step_consulting")],
            vec!["step_consulting"],
            0.0,
            360.0,
        ),
        make_condition_node_llm(
            "c-consulting-gate",
            "质量门",
            "根据进入评估结果判断：目标行业是否值得进入（是→true 战略制定，否→false 风险报告）",
            "step2_consulting",
            0.0,
            540.0,
        ),
        make_agent_node_full(
            "step3_consulting",
            "战略制定",
            "你是战略制定专家。执行「战略制定」：结合上游输入，输出结构化 JSON 结果（含关键指标、结论与建议）。",
            vec![td("OpcCreateContentAsset"), td("FileWrite")],
            None,
            "step3_consulting",
            vec![("input", "step2_consulting")],
            vec!["step2_consulting"],
            -250.0,
            720.0,
        ),
        make_agent_node_full(
            "fix-consulting",
            "风险报告",
            "行业不宜进入，输出风险报告与替代建议。输出 JSON：{\"risks\":[], \"alternatives\":[], \"recommendation\":\"\"}",
            vec![],
            None,
            "fix-consulting",
            vec![("input", "step2_consulting")],
            vec!["step2_consulting"],
            250.0,
            720.0,
        ),
        make_merge_node("m-consulting", "汇合", 0.0, 900.0),
        make_end(0.0, 1080.0),
    ];

    let edges = vec![
        edge("e-trigger-step_consulting", "trigger", "step_consulting"),
        edge("e-step_consulting-step2_consulting", "step_consulting", "step2_consulting"),
        edge("e-step2_consulting-gate", "step2_consulting", "c-consulting-gate"),
        edge_cond(
            "e-gate-main",
            "c-consulting-gate",
            "true",
            "step3_consulting",
            EdgeType::ConditionTrue,
        ),
        edge_cond(
            "e-gate-fix",
            "c-consulting-gate",
            "false",
            "fix-consulting",
            EdgeType::ConditionFalse,
        ),
        edge("e-main-merge", "step3_consulting", "m-consulting"),
        edge("e-fix-merge", "fix-consulting", "m-consulting"),
        edge("e-m-consulting-end", "m-consulting", "end"),
    ];

    let now = chrono::Utc::now().timestamp_millis();
    let template_data = WorkflowTemplateData {
        hooks_config: None,
        id: TEMPLATE_ID.to_string(),
        name: "产业咨询流程".to_string(),
        description: Some("行业扫描 → 进入评估 → 战略制定。产业咨询全流程。".to_string()),
        icon: "💼".to_string(),
        cluster_id: None,
        route_path: None,
        tags: vec!["opc".to_string(), "domain_pack".to_string(), "consulting".to_string()],
        version: TEMPLATE_VERSION,
        is_preset: true,
        is_editable: true,
        is_public: false,
        visibility: Visibility::Public,
        trigger_config: Some(TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({}),
        }),
        nodes,
        edges,
        input_schema: None,
        output_schema: None,
        variables: vec![],
        error_config: None,
        error_workflow_id: None,
        tool_defs: vec![],
        mission_hash: None,
        created_at: now,
        updated_at: now,
    };

    super::upsert_template(db, template_data).await
}
