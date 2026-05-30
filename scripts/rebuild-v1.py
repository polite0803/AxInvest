import os

filepath = 'src-tauri/src/commands/stock_analysis_setup.rs'
with open(filepath, 'r', encoding='utf-8') as f:
    content = f.read()

# Find the seed_stock_analysis_workflow_template function boundaries
fn_start = content.index('async fn seed_stock_analysis_workflow_template(')
fn_end = content.index('\npub async fn seed_debate_subworkflow', fn_start)

# The template version, imports, and core DAG
new_fn = '''async fn seed_stock_analysis_workflow_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    use axagent_core::entity::workflow_template;
    use axagent_core::workflow_types::{
        AgentNode, AgentNodeConfig, EdgeType, ErrorConfig, JsonSchema, JsonSchemaProperty,
        OnFailureAction, OutputMode, Position, RetryConfig, RetryPolicy, ToolDef, ToolNode,
        ToolNodeConfig, TriggerConfig, TriggerNode, TriggerType, Variable, WorkflowEdge,
        WorkflowNode, WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    const TEMPLATE_ID: &str = "stock-analysis";
    const TEMPLATE_VERSION: i32 = 1;

    if let Some(existing) = workflow_template::Entity::find_by_id(TEMPLATE_ID)
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
    {
        if existing.version >= TEMPLATE_VERSION {
            return Ok(());
        }
        workflow_template::Entity::delete_by_id(TEMPLATE_ID)
            .exec(db)
            .await
            .map_err(|e| format!("删除旧模板失败: {e}"))?;
        tracing::info!(
            "[stock_analysis_setup] 更新股票分析工作流模板 v{} → v{TEMPLATE_VERSION}",
            existing.version
        );
    }

    let now = chrono::Utc::now().timestamp_millis();

    let tool_node =
        |id: &str, title: &str, tool_name: &str, output_var: &str, arg_key: &str| -> WorkflowNode {
            let mut input_mapping = std::collections::HashMap::new();
            input_mapping.insert(arg_key.to_string(), "trigger.config.stock_code".to_string());
            WorkflowNode::Tool(ToolNode {
                base: WorkflowNodeBase {
                    id: id.into(), title: title.into(),
                    description: Some(format!("获取数据: {tool_name}")),
                    position: Position { x: 0.0, y: 0.0 },
                    retry: RetryConfig { enabled: true, max_retries: 2, ..Default::default() },
                    timeout: Some(120), enabled: true,
                },
                config: ToolNodeConfig {
                    tool_name: tool_name.into(), input_mapping, output_var: output_var.into(),
                },
            })
        };

    // ── ToolDefs (unchanged from original v17) ──
'''
# We need the original ToolDef definitions, agent helper, edge helper, analyst list, tool assignments, debate pairs, risk section, and final nodes.
# Rather than replacing everything manually, let me just fix the imports and DAG structure.
# The simplest approach: fix the imports, remove the bad node types, and ensure correct edges.

# Actually, let me just fix the key issues:
# 1. Import block - remove experimental types
# 2. Remove ParallelNode, ConditionNode, Conditional edges, SubWorkflow, Validation
# 3. Restore original edge connections

# Fix imports
old_imports = '''    use axagent_core::entity::workflow_template;
    use axagent_core::workflow_types::{
        AgentNode, AgentNodeConfig, Branch, ConditionNode, ConditionNodeConfig, EdgeType,
        ErrorConfig, JsonSchema, JsonSchemaProperty, LogicalOperator, OnFailureAction, OutputMode,
        ParallelNode, ParallelNodeConfig, Position, RetryConfig, RetryPolicy, ToolDef, ToolNode,
        ToolNodeConfig, TriggerConfig, TriggerNode, TriggerType, ValidationAssertion,
        ValidationNode, ValidationNodeConfig, Variable, WorkflowEdge, WorkflowNode,
        WorkflowNodeBase,
    };'''

new_imports = '''    use axagent_core::entity::workflow_template;
    use axagent_core::workflow_types::{
        AgentNode, AgentNodeConfig, EdgeType, ErrorConfig, JsonSchema, JsonSchemaProperty,
        OnFailureAction, OutputMode, Position, RetryConfig, RetryPolicy, ToolDef, ToolNode,
        ToolNodeConfig, TriggerConfig, TriggerNode, TriggerType, Variable, WorkflowEdge,
        WorkflowNode, WorkflowNodeBase,
    };'''

content = content.replace(old_imports, new_imports)

# Fix trigger config - use Manual instead of Schedule
old_trigger = '''        trigger_config: Set(Some(
            serde_json::to_string(&TriggerConfig {
                trigger_type: TriggerType::Schedule,
                config: serde_json::json!({
                    "schedules": {
                        "morning": "0 9 * * 1-5",
                        "afternoon": "0 14 * * 1-5",
                    },
                    "enabled": false,
                    "timezone": "Asia/Shanghai",
                }),
            }).unwrap_or_default()
        )),'''

new_trigger = '        trigger_config: Set(None),'

content = content.replace(old_trigger, new_trigger)

# Fix the analyst section - replace ParallelNode approach with original simple for-loop
# Find the section from "Phase 1: ParallelNode" to the debate section
old_analyst_section_start = content.find('    // ── Phase 1:')
old_debate_section = content.find('    // 辩论 6 轮 — bull-r1 直接依赖 c-need-debate')
# Remove the conditional edges closure and fix the debate deps
# bull-r1 should depend on all analyst IDs, not c-need-debate

# Remove ConditionNode
cond_start = content.find('    // ── Phase 2:')
cond_end = content.find('    edges.push(edge("e-p-analysts-c-debate"')
if cond_start > 0 and cond_end > cond_start:
    content = content[:cond_start] + content[cond_end:]

# Fix: Replace ParallelNode section with original for-loop
# Find "Phase 1" section
ph1_start = content.find('    // ── Phase 1:')
ph1_end = content.find('    edges.push(edge("e-trigger-p-analysts"')

if ph1_start > 0 and ph1_end > ph1_start:
    # Create replacement: original tool assignments for-loop
    replacement = '''    // 9 个分析师 + 对应数据 ToolNode
    let tool_assignments: &[(&str, &str, &str, &str)] = &[
        ("t-market-data", "获取K线+行情", "get_stock_kline", "stock_code"),
        ("t-sentiment-data", "获取新闻+热门", "get_hot_stocks", "stock_code"),
        ("t-news-data", "获取新闻+公告", "get_announcements", "stock_code"),
        ("t-fundamentals-data", "获取财务+一致预期", "get_consensus_eps", "stock_code"),
        ("t-policy-data", "获取新闻+公告", "get_announcements", "stock_code"),
        ("t-hotmoney-data", "获取资金流向", "get_stock_money_flow", "stock_code"),
        ("t-lockup-data", "获取财务+公告", "get_announcements", "stock_code"),
        ("t-research-data", "获取新闻+一致预期", "get_consensus_eps", "stock_code"),
        ("t-sector-data", "获取行情+行业排名", "get_industry_ranking", "stock_code"),
    ];

    for (i, (tool_id, tool_title, tool_name, arg_key)) in tool_assignments.iter().enumerate() {
        let analyst_id = a_ids[i];
        nodes.push(tool_node(tool_id, tool_title, tool_name, tool_id, arg_key));
        edges.push(edge(&format!("e-trigger-{tool_id}"), "trigger", tool_id));
        edges.push(edge(&format!("e-{tool_id}-{analyst_id}"), tool_id, analyst_id));
    }

    // 工具由模板节点 config.tools 统一管理'''
    p_analysts_edge = '    edges.push(edge("e-trigger-p-analysts"'
    content = content[:ph1_start] + replacement + content[ph1_end + len(p_analysts_edge):]

print('Fixed imports, trigger, analyst section')

# Fix debate deps: bull-r1 depends on all analyst IDs, not c-need-debate
old_bull_deps = '&["c-need-debate"][..]'
new_bull_deps = '&a_ids[..]'
content = content.replace(old_bull_deps, new_bull_deps)

# Remove the c-need-debate node if it still exists
cnd = content.find('    nodes.push(WorkflowNode::Condition')
if cnd > 0:
    cnd_end = content.find('    }));\n', cnd) + len('    }));\n')
    content = content[:cnd] + content[cnd_end:]

# Remove the p-analysts edge from c-need-debate
content = content.replace(
    '    edges.push(edge("e-p-analysts-c-debate", "p-analysts", "c-need-debate"));\n\n',
    '\n'
)

# Remove ValidationNode
v_start = content.find('    // ── Validation:')
v_end = content.find('    edges.push(edge("e-v-validate')
if v_start > 0:
    v_end2 = content.find('    // research-mgr', v_end)
    content = content[:v_start] + content[v_end2:]

# Fix: change v-validate edge back to t-risk edge
content = content.replace(
    'edges.push(edge("e-v-validate-research-mgr", "v-validate", "research-mgr"));',
    'edges.push(edge("e-t-risk-research-mgr", "t-risk", "research-mgr"));'
)

# Remove RAG source IDs and Plan mode
content = content.replace(
    '                a.config.rag_source_ids = vec![\n                    "knowledge:stock-faq".into(),\n                    "memory:investment-rules".into(),\n                ];\n',
    ''
)
content = content.replace(
    '            a.config.rag_source_ids = vec![\n                "knowledge:trading-rules".into(),\n                "memory:risk-limits".into(),\n            ];\n',
    ''
)
content = content.replace(
    '            a.config.rag_source_ids = vec![\n                "knowledge:portfolio-guidelines".into(),\n                "memory:decision-history".into(),\n            ];\n            a.config.execution_mode = Some("plan".into());\n',
    ''
)

with open(filepath, 'w', encoding='utf-8') as f:
    f.write(content)
print('DONE - v1 template rebuilt with proven DAG structure')
