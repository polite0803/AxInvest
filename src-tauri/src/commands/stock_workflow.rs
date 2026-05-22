//! 工作流驱动的股票分析 — 统一对话页和分析页的触发入口。
//!
//! 与现有 start_stock_analysis (Orchestrator 硬编码管线) 并行存在,
//! 等前端全部迁移后废弃旧命令。

use crate::AppState;
use axagent_core::entity::stock_analyses;
use axagent_core::types::ProviderProxyConfig;
use axagent_providers::{ProviderAdapter, ProviderRequestContext, resolve_base_url_for_type};
use axagent_rt_workflow::workflow_engine::{StepExecutor, WorkflowRunner, WorkflowStep};
use axagent_runtime::agent_roles::AgentRole;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Emitter, State};

/// 构建 WorkflowStep — 每个节点关联一个种子化的 AgentProfile
fn wf_step(
    id: &str,
    expert_id: &str,
    goal: &str,
    needs: Vec<String>,
    system_prompt: &str,
    data_ctx: &str,
) -> WorkflowStep {
    WorkflowStep {
        id: id.into(),
        goal: goal.into(),
        context: Some(format!("{}\n\n{}", system_prompt, data_ctx)),
        needs,
        agent_profile_id: Some(format!("stock-{}", expert_id)),
        agent_role: AgentRole::Researcher,
        ..Default::default()
    }
}

/// 构建 StepExecutor — 每步走 SessionManager.run_turn_with_tools() 标准路径
fn build_executor(
    sm: Arc<axagent_agent::session_manager::SessionManager>,
    adapter: Arc<dyn ProviderAdapter>,
    pctx: ProviderRequestContext,
    model: String,
    conversation_id: String,
) -> StepExecutor {
    Arc::new(move |step: WorkflowStep, deps: HashMap<String, String>| {
        let sm = sm.clone();
        let adapter = adapter.clone();
        let pctx = pctx.clone();
        let model = model.clone();
        let cid = conversation_id.clone();
        Box::pin(async move {
            let session_id = format!("{}-{}", cid, step.id);
            let sess = sm
                .get_or_create_session(pctx.provider_id.clone(), session_id)
                .await
                .map_err(|e| format!("session: {e}"))?;
            let client = axagent_agent::provider_adapter::AxAgentApiClient::new(adapter, pctx)
                .with_model(&model)
                .with_temperature(Some(0.3))
                .with_max_tokens(Some(4096));
            let sys_prompt = step.context.unwrap_or_default();
            let deps_text = deps
                .iter()
                .map(|(k, v)| format!("{}:\n{}", k, v))
                .collect::<Vec<_>>()
                .join("\n\n");
            let user_prompt = format!("{}\n\n前置步骤结果:\n{}", step.goal, deps_text);
            let (summary, _) = sm
                .run_turn_with_tools(
                    &sess.session().session_id,
                    user_prompt,
                    client,
                    axagent_tools::registry::UnifiedToolRegistry::new(),
                    vec![sys_prompt],
                    cid,
                    axagent_runtime_core::PermissionMode::Allow,
                    Arc::new(tokio::sync::Mutex::new(HashMap::new())),
                    None,
                )
                .await
                .map_err(|e| format!("LLM: {e}"))?;
            Ok(summary
                .assistant_messages
                .iter()
                .flat_map(|m| &m.blocks)
                .filter_map(|b| match b {
                    axagent_runtime_core::ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"))
        })
    })
}

#[tauri::command]
pub async fn run_stock_workflow(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<serde_json::Value, String> {
    // ── 1. 数据 ──
    let quote = state
        .astock_client
        .get_quote(&stock_code)
        .await
        .map_err(|e| format!("行情失败: {e}"))?;
    let prompts = super::stock_analysis::load_stock_analysis_prompts(&state.sea_db).await;
    let now = chrono::Utc::now().timestamp_millis();
    let conv_id = uuid::Uuid::new_v4().to_string();
    let analysis_id = uuid::Uuid::new_v4().to_string();

    let model = stock_analyses::ActiveModel {
        id: Set(analysis_id.clone()),
        stock_code: Set(stock_code.clone()),
        stock_name: Set(quote.name.clone()),
        analysis_date: Set(chrono::Utc::now().format("%Y-%m-%d").to_string()),
        provider_id: Set("workflow".into()),
        conversation_id: Set(conv_id.clone()),
        status: Set("running".into()),
        decision_action: Set(None),
        decision_position_pct: Set(None),
        decision_reasoning: Set(None),
        decision_json: Set(None),
        blackboard_snapshot: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    model
        .insert(&state.sea_db)
        .await
        .map_err(|e| format!("DB: {e}"))?;

    // ── 2. Provider ──
    let prov_list = axagent_core::repo::provider::list_providers(&state.sea_db)
        .await
        .map_err(|e| e.to_string())?;
    let prov = prov_list
        .iter()
        .find(|p| p.enabled)
        .ok_or("没有启用的 Provider")?;
    let key = prov
        .keys
        .iter()
        .find(|k| k.enabled)
        .ok_or("没有启用的 API Key")?;
    let api_key = axagent_core::crypto::decrypt_key(&key.key_encrypted, &state.master_key)
        .map_err(|e| format!("密钥: {e}"))?;
    let settings = axagent_core::repo::settings::get_settings(&state.sea_db)
        .await
        .unwrap_or_default();
    let pctx = ProviderRequestContext {
        api_key,
        key_id: key.id.clone(),
        provider_id: prov.id.clone(),
        base_url: Some(resolve_base_url_for_type(&prov.api_host, &prov.provider_type)),
        api_path: prov.api_path.clone(),
        proxy_config: ProviderProxyConfig::resolve(&prov.proxy_config, &settings),
        custom_headers: prov
            .custom_headers
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok()),
        api_mode: None,
        conversation: None,
        previous_response_id: None,
        store_response: None,
    };
    let adapter: Arc<dyn ProviderAdapter> = match prov.provider_type {
        axagent_core::types::ProviderType::OpenAI => {
            Arc::new(axagent_providers::openai::OpenAIAdapter::new())
        },
        axagent_core::types::ProviderType::Anthropic => {
            Arc::new(axagent_providers::anthropic::AnthropicAdapter::new())
        },
        axagent_core::types::ProviderType::Gemini => {
            Arc::new(axagent_providers::gemini::GeminiAdapter::new())
        },
        axagent_core::types::ProviderType::Ollama => {
            Arc::new(axagent_providers::ollama::OllamaAdapter::new())
        },
        _ => Arc::new(axagent_providers::openai::OpenAIAdapter::new()),
    };
    let model_id = prov
        .models
        .iter()
        .find(|m| m.enabled)
        .map(|m| m.model_id.clone())
        .unwrap_or_default();

    // ── 3. 行情上下文 ──
    let data_ctx = format!(
        "{} ({})\n现价:¥{:.2} 涨跌:{:.2}% PE:{} PB:{} 市值:{}",
        quote.name,
        stock_code,
        quote.price,
        quote.change_pct,
        quote.pe.map_or("N/A".into(), |v| format!("{:.1}", v)),
        quote.pb.map_or("N/A".into(), |v| format!("{:.1}", v)),
        quote
            .total_mv
            .map_or("N/A".into(), |v| format!("{:.0}亿", v / 1e8)),
    );

    // ── 4. 构建 DAG ──
    let prompt = |id: &str| -> String {
        prompts
            .get(id)
            .cloned()
            .unwrap_or_else(|| format!("你是{}，基于数据分析，只输出JSON。", id))
    };
    let s = |id: &str, expert, goal: &str, needs: Vec<String>| {
        wf_step(id, expert, goal, needs, &prompt(expert), &data_ctx)
    };

    let mut steps = Vec::new();
    let analysts = [
        "market-analyst",
        "sentiment-analyst",
        "news-analyst",
        "fundamentals-analyst",
        "policy-analyst",
        "hot-money-tracker",
        "lockup-watcher",
        "research-analyst",
        "sector-analyst",
    ];
    let a_ids: Vec<String> = analysts.iter().map(|a| format!("a-{}", a)).collect();
    for a in analysts {
        let step_id = format!("a-{}", a);
        let goal = format!("作为{}分析{}", a, stock_code);
        steps.push(s(&step_id, a, &goal, vec![]));
    }
    // 辩论
    steps.push(s("bull-r1", "bull-researcher", "多方第1轮", a_ids.clone()));
    steps.push(s("bear-r1", "bear-researcher", "空方第1轮", vec!["bull-r1".into()]));
    steps.push(s("bull-r2", "bull-researcher", "多方第2轮", vec!["bear-r1".into()]));
    steps.push(s("bear-r2", "bear-researcher", "空方第2轮", vec!["bull-r2".into()]));
    steps.push(s("bull-r3", "bull-researcher", "多方第3轮", vec!["bear-r2".into()]));
    steps.push(s("bear-r3", "bear-researcher", "空方第3轮", vec!["bull-r3".into()]));
    // 风险
    steps.push(s("risk-agg", "aggressive-debator", "激进风险评估", vec!["bear-r3".into()]));
    steps.push(s("risk-con", "conservative-debator", "保守风险评估", vec!["bear-r3".into()]));
    steps.push(s("risk-neu", "neutral-debator", "中性风险评估", vec!["bear-r3".into()]));
    steps.push(s(
        "research-mgr",
        "research-manager",
        "综合风险总评",
        vec!["risk-agg".into(), "risk-con".into(), "risk-neu".into()],
    ));
    steps.push(s("trader", "trader", "制定A股交易方案", vec!["research-mgr".into()]));
    steps.push(s("portfolio-mgr", "portfolio-manager", "最终投资决策", vec!["trader".into()]));

    // ── 5. 执行 ──
    let sm = state.agent_session_manager.clone();
    let executor = build_executor(sm, adapter, pctx, model_id, conv_id.clone());
    let wf_engine = state.workflow_engine.clone();
    let wf_name = format!("stock-analysis-{}", stock_code);
    let workflow = wf_engine
        .create_workflow(&wf_name, steps)
        .map_err(|e| format!("workflow: {e}"))?;
    let wf_id = workflow.id.clone();
    let wf_id_ret = wf_id.clone();
    let app_h = app.clone();
    let db = state.sea_db.clone();
    let aid = analysis_id.clone();
    let cid = conv_id.clone();

    tokio::spawn(async move {
        let runner = WorkflowRunner::new(wf_engine, executor);
        match runner.run(&wf_id).await {
            Ok(result) => {
                let _ = app_h.emit(
                    "workflow-completed",
                    serde_json::json!({
                        "workflowId": wf_id, "conversationId": cid, "results": result.results,
                    }),
                );
                let result_json = serde_json::to_string(&result.results).unwrap_or_default();
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(stock_analyses::Column::Status, Expr::value("completed"))
                    .col_expr(stock_analyses::Column::DecisionJson, Expr::value(&result_json))
                    .col_expr(
                        stock_analyses::Column::UpdatedAt,
                        Expr::value(chrono::Utc::now().timestamp_millis()),
                    )
                    .filter(stock_analyses::Column::Id.eq(&aid))
                    .exec(&db)
                    .await;
            },
            Err(e) => {
                let _ = app_h.emit(
                    "workflow-error",
                    serde_json::json!({
                        "workflowId": wf_id, "conversationId": cid, "error": e.to_string(),
                    }),
                );
                let _ = stock_analyses::Entity::update_many()
                    .col_expr(stock_analyses::Column::Status, Expr::value(format!("failed: {e}")))
                    .col_expr(
                        stock_analyses::Column::UpdatedAt,
                        Expr::value(chrono::Utc::now().timestamp_millis()),
                    )
                    .filter(stock_analyses::Column::Id.eq(&aid))
                    .exec(&db)
                    .await;
            },
        }
    });

    Ok(serde_json::json!({
        "analysisId": analysis_id,
        "conversationId": conv_id,
        "workflowId": wf_id_ret,
        "stockCode": stock_code,
        "stockName": quote.name,
    }))
}
