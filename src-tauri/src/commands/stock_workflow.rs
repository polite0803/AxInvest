//! 工作流驱动的股票分析 — 基于持久化 WorkflowTemplate + WorkEngine DAG 执行。
//!
//! 启动时种子化 stock-analysis 工作流模板到 workflow_templates 表，
//! 每次分析从模板加载 DAG 结构，注入实时行情数据，由 WorkEngine 并行执行。

use crate::AppState;
use axagent_astock_data::as_of::{self, AsOfContext};
use axagent_core::entity::reco_picks;
use axagent_core::entity::stock_analyses;
use axagent_core::entity::stock_reflections;
use axagent_harness::workflow_types::{JsonSchema, Variable, WorkflowEdge, WorkflowNode};
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use axagent_stock_analysis::blackboard::build_blackboard_snapshot;
use sea_orm::DatabaseConnection;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;
use std::sync::Arc;
use tauri::{Emitter, State};

/// 数据质量预检结果
enum QualityPrecheckResult {
    /// 数据充分，可以执行
    Pass,
    /// 部分数据缺失但可继续
    Partial(String),
    /// 数据不足，跳过
    Insufficient(String),
}

/// P1-3: 单数据源预检结果(供多源聚合用)
#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceCheck {
    /// 该源充分
    Ok,
    /// 该源部分缺失,但可继续
    Partial(String),
    /// 该源完全失败(数据为零或 vendor 报错)
    Failed(String),
}

/// P1-3: 聚合 5 个核心数据源的预检结果, 取最差等级
fn aggregate_precheck(sources: Vec<(&str, SourceCheck)>) -> QualityPrecheckResult {
    let mut partial_msgs: Vec<String> = Vec::new();
    let mut first_failure: Option<String> = None;
    for (name, c) in sources {
        match c {
            SourceCheck::Ok => {},
            SourceCheck::Partial(reason) => partial_msgs.push(format!("{name}: {reason}")),
            SourceCheck::Failed(reason) => {
                if first_failure.is_none() {
                    first_failure = Some(format!("{name}: {reason}"));
                }
            },
        }
    }
    if let Some(reason) = first_failure {
        QualityPrecheckResult::Insufficient(reason)
    } else if !partial_msgs.is_empty() {
        QualityPrecheckResult::Partial(partial_msgs.join("; "))
    } else {
        QualityPrecheckResult::Pass
    }
}

/// 在启动 DAG 前执行快速数据质量检查。
///
/// P1-3 修复: 扩展预检覆盖 5 个核心数据源(quote / financials / klines / news /
/// money_flow),任一完全失败则整体 Insufficient;部分缺失则 Partial。as-of 模式下
/// 所有 vendor 调用走 as-of scope, 预检结果反映"截至 as_of_date 的数据是否够用"。
///
/// API 调用成本: 5 次 vs 原 2 次, 仍远低于 15~20 次 LLM 调用。
async fn data_quality_precheck(
    client: &axagent_astock_data::AStockClient,
    stock_code: &str,
    quote: &axagent_astock_data::StockQuote,
) -> QualityPrecheckResult {
    // 1. quote — 已在参数中传入, 直接检查
    let quote_check = if quote.price <= 0.0 && quote.name.is_empty() {
        SourceCheck::Failed("价格为空、股票代码不存在或未上市".into())
    } else {
        SourceCheck::Ok
    };

    // 2. financials
    let fin_check = match client.get_financials(stock_code).await {
        Ok(financials) => {
            let has_revenue = financials.iter().any(|f| f.revenue.unwrap_or(0.0) > 0.0);
            let has_profit = financials.iter().any(|f| f.net_profit.unwrap_or(0.0) > 0.0);
            if !has_revenue && !has_profit {
                SourceCheck::Partial("营收/利润缺失".into())
            } else {
                SourceCheck::Ok
            }
        },
        Err(e) => SourceCheck::Partial(format!("获取失败: {e}")),
    };

    // P1-3 新增: 3. klines (取 60 日, 验证历史数据可拉到)
    // AStockClient::get_klines 是 3 参 wrapper (内部默认 None 复权)
    let kline_check = match client.get_klines(stock_code, "daily", 60).await {
        Ok(klines) if klines.len() >= 30 => SourceCheck::Ok,
        Ok(klines) if !klines.is_empty() => {
            SourceCheck::Partial(format!("仅 {} 行, 不足 30 日", klines.len()))
        },
        Ok(_) => SourceCheck::Failed("K 线为空".into()),
        Err(e) => SourceCheck::Failed(format!("K 线获取失败: {e}")),
    };

    // P1-3 新增: 4. news (取最近 10 条)
    let news_check = match client.get_news(stock_code, 10).await {
        Ok(news) if !news.is_empty() => SourceCheck::Ok,
        Ok(_) => SourceCheck::Partial("无新闻数据".into()),
        Err(e) => SourceCheck::Partial(format!("新闻获取失败: {e}")),
    };

    // P1-3 新增: 5. money_flow
    let money_flow_check = match client.get_money_flow(stock_code).await {
        Ok(Some(_)) => SourceCheck::Ok,
        Ok(None) => SourceCheck::Partial("无资金流数据".into()),
        Err(e) => SourceCheck::Partial(format!("资金流获取失败: {e}")),
    };

    aggregate_precheck(vec![
        ("quote", quote_check),
        ("financials", fin_check),
        ("klines", kline_check),
        ("news", news_check),
        ("money_flow", money_flow_check),
    ])
}

struct LoadedTemplate {
    nodes: Vec<WorkflowNode>,
    edges: Vec<WorkflowEdge>,
    input_schema: Option<JsonSchema>,
    output_schema: Option<JsonSchema>,
    variables: Option<Vec<Variable>>,
}

#[cfg(test)]
mod precheck_tests {
    use super::*;

    // P1-3: aggregate_precheck 取最差等级
    #[test]
    fn aggregate_all_ok_returns_pass() {
        let r = aggregate_precheck(vec![
            ("quote", SourceCheck::Ok),
            ("financials", SourceCheck::Ok),
            ("klines", SourceCheck::Ok),
        ]);
        assert!(matches!(r, QualityPrecheckResult::Pass));
    }

    #[test]
    fn aggregate_one_partial_returns_partial_with_joined_message() {
        let r = aggregate_precheck(vec![
            ("quote", SourceCheck::Ok),
            ("financials", SourceCheck::Partial("营收缺失".into())),
            ("klines", SourceCheck::Ok),
        ]);
        match r {
            QualityPrecheckResult::Partial(msg) => {
                assert!(msg.contains("financials"), "partial msg 应含 source 名: {msg}");
                assert!(msg.contains("营收缺失"));
            },
            _ => panic!("expected Partial"),
        }
    }

    #[test]
    fn aggregate_any_failure_returns_insufficient() {
        let r = aggregate_precheck(vec![
            ("quote", SourceCheck::Ok),
            ("klines", SourceCheck::Failed("K 线获取失败".into())),
        ]);
        match r {
            QualityPrecheckResult::Insufficient(msg) => {
                assert!(msg.contains("klines"), "insufficient msg 应含 source 名: {msg}");
                assert!(msg.contains("K 线获取失败"));
            },
            _ => panic!("expected Insufficient"),
        }
    }

    #[test]
    fn aggregate_failure_beats_partial() {
        // 5 源: 2 partial + 1 failed → overall Insufficient
        let r = aggregate_precheck(vec![
            ("quote", SourceCheck::Ok),
            ("financials", SourceCheck::Partial("缺".into())),
            ("klines", SourceCheck::Failed("空了".into())),
            ("news", SourceCheck::Partial("无".into())),
            ("money_flow", SourceCheck::Ok),
        ]);
        assert!(matches!(r, QualityPrecheckResult::Insufficient(_)));
    }
}

async fn load_and_inject_template(
    db: &sea_orm::DatabaseConnection,
    stock_code: &str,
    _stock_name: &str,
    template_id: &str,
) -> Result<LoadedTemplate, String> {
    use axagent_core::entity::workflow_template;

    let template = workflow_template::Entity::find_by_id(template_id)
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
        .ok_or(format!("工作流模板 {template_id} 未种子化，请重启应用"))?;

    let mut nodes: Vec<WorkflowNode> =
        serde_json::from_str(&template.nodes).map_err(|e| format!("解析模板节点失败: {e}"))?;
    let edges: Vec<WorkflowEdge> =
        serde_json::from_str(&template.edges).map_err(|e| format!("解析模板边失败: {e}"))?;

    if nodes.is_empty() {
        tracing::warn!("[stock_workflow] 模板节点为空，自动重新种子化");
        crate::commands::stock_analysis_setup::ensure_stock_analysis_experts_seeded(db).await?;
        let template = workflow_template::Entity::find_by_id("stock-analysis")
            .one(db)
            .await
            .map_err(|e| format!("重查模板失败: {e}"))?
            .ok_or("模板种子化后仍不存在")?;
        nodes =
            serde_json::from_str(&template.nodes).map_err(|e| format!("解析模板节点失败: {e}"))?;
    }

    for node in &mut nodes {
        if let WorkflowNode::Trigger(tn) = node {
            if let Some(sc) = tn.config.config.get_mut("stock_code") {
                *sc = serde_json::Value::String(stock_code.to_string());
            }
        }
    }

    // stock_code/stock_name 已通过 AgentNodeConfig.input_mapping 自动注入到每个 Agent 节点的 system_prompt，
    // 不再需要手动遍历追加（参见 stock_analysis_setup.rs 中 agent() 宏的 input_mapping 配置）。

    let input_schema: Option<JsonSchema> = template
        .input_schema
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());
    let output_schema: Option<JsonSchema> = template
        .output_schema
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());
    let variables: Option<Vec<Variable>> = template
        .variables
        .as_ref()
        .and_then(|v| serde_json::from_str(v).ok());

    Ok(LoadedTemplate {
        nodes,
        edges,
        input_schema,
        output_schema,
        variables,
    })
}

/// 工作流结果 → blackboard_snapshot — 现已委托给 axagent-stock-analysis::blackboard 模块
/// 此处保留占位以便未来重新内联
#[allow(clippy::type_complexity)]
fn extract_decision_fields(
    decision_json: &Option<String>,
) -> (Option<String>, Option<f64>, Option<String>, Option<String>, Option<u32>) {
    let raw = match decision_json {
        Some(s) if !s.is_empty() => s,
        _ => return (None, None, None, None, None),
    };
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return (None, None, None, None, None),
    };
    let action = parsed
        .get("action")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let position_pct = parsed
        .get("positionPct")
        .or_else(|| parsed.get("position_pct"))
        .and_then(|v| v.as_f64());
    let reasoning = parsed
        .get("reasoning")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let time_horizon = parsed
        .get("timeHorizon")
        .or_else(|| parsed.get("time_horizon"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expected_holding_days = parsed
        .get("expectedHoldingDays")
        .or_else(|| parsed.get("expected_holding_days"))
        .and_then(|v| {
            if v.is_number() {
                v.as_u64().map(|n| n as u32)
            } else {
                None
            }
        });
    (action, position_pct, reasoning, time_horizon, expected_holding_days)
}

/// 解析 as_of_date 入参：None/空串 → None（live），Some(s) → 解析为 AsOfContext
/// 抽出供单测：未来日期 / 错误格式必须 4xx-style 错误
pub(crate) fn parse_asof_param(s: Option<String>) -> Result<Option<AsOfContext>, String> {
    AsOfContext::parse_optional(s.as_deref())
}

/// 默认值，与 stock-analysis 模板的 defaults 保持一致；
/// 改动这里请同步 `StockAnalysisConfigPanel.getDefaultVariables()`。
const DEFAULT_MAX_CONCURRENT: usize = 12;
const DEFAULT_STEP_TIMEOUT_SECS: u64 = 300;

/// 从模板 variables 中解析 RunOptions 关键参数。
///
/// 用户在「股票分析设置 → 参数」中调整 `max_concurrent` /
/// `agent_timeout_secs` 后，这里读到的就是新值；如果模板里没有这两个
/// key（旧版本 / 用户清空）则用默认值。
///
/// 容错策略：
///   * 越界 / 非法类型 → 用默认值；
///   * max_concurrent ∈ [1, 32]，过小会让并发退化为串行，过大会拖垮 LLM 速率。
///   * step_timeout ∈ [10, 3600] 秒，避免 0 或极端大值。
pub(crate) fn resolve_runtime_options(
    variables: Option<&[axagent_harness::workflow_types::Variable]>,
) -> (usize, std::time::Duration) {
    let lookup = |name: &str| -> Option<serde_json::Value> {
        variables
            .and_then(|vs| vs.iter().find(|v| v.name == name))
            .map(|v| v.value.clone())
    };

    let max_concurrent = lookup("max_concurrent")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, 32) as usize)
        .unwrap_or(DEFAULT_MAX_CONCURRENT);

    let step_timeout_secs = lookup("agent_timeout_secs")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(10, 3600))
        .unwrap_or(DEFAULT_STEP_TIMEOUT_SECS);

    (max_concurrent, std::time::Duration::from_secs(step_timeout_secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::workflow_types::Variable;
    use serde_json::json;

    #[test]
    fn resolve_runtime_options_uses_defaults_when_missing() {
        let (mc, to) = resolve_runtime_options(None);
        assert_eq!(mc, DEFAULT_MAX_CONCURRENT);
        assert_eq!(to.as_secs(), DEFAULT_STEP_TIMEOUT_SECS);
    }

    #[test]
    fn resolve_runtime_options_reads_template_vars() {
        let vars = vec![
            Variable {
                name: "max_concurrent".into(),
                var_type: "number".into(),
                value: json!(20),
                description: None,
                is_secret: false,
            },
            Variable {
                name: "agent_timeout_secs".into(),
                var_type: "number".into(),
                value: json!(120),
                description: None,
                is_secret: false,
            },
        ];
        let (mc, to) = resolve_runtime_options(Some(&vars));
        assert_eq!(mc, 20);
        assert_eq!(to.as_secs(), 120);
    }

    #[test]
    fn resolve_runtime_options_clamps_extremes() {
        let vars = vec![
            Variable {
                name: "max_concurrent".into(),
                var_type: "number".into(),
                value: json!(0), // 0 → clamp 到 1
                description: None,
                is_secret: false,
            },
            Variable {
                name: "agent_timeout_secs".into(),
                var_type: "number".into(),
                value: json!(99999), // 过大 → clamp 到 3600
                description: None,
                is_secret: false,
            },
        ];
        let (mc, to) = resolve_runtime_options(Some(&vars));
        assert_eq!(mc, 1);
        assert_eq!(to.as_secs(), 3600);
    }

    #[test]
    fn resolve_runtime_options_falls_back_on_bad_types() {
        let vars = vec![Variable {
            name: "max_concurrent".into(),
            var_type: "string".into(),
            value: json!("not a number"),
            description: None,
            is_secret: false,
        }];
        let (mc, _) = resolve_runtime_options(Some(&vars));
        assert_eq!(mc, DEFAULT_MAX_CONCURRENT);
    }
}

#[tauri::command]
pub async fn run_stock_workflow(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
    dry_run: Option<bool>,
    as_of_date: Option<String>,
) -> Result<serde_json::Value, String> {
    // 解析 as_of_date；非法或未来日期直接 4xx-style 错误
    let as_of_ctx = parse_asof_param(as_of_date.clone())?;

    if let Some(ctx) = as_of_ctx {
        as_of::AS_OF
            .scope(Some(ctx), async {
                run_stock_workflow_inner(app, state, stock_code, dry_run, as_of_date).await
            })
            .await
    } else {
        run_stock_workflow_inner(app, state, stock_code, dry_run, None).await
    }
}

async fn run_stock_workflow_inner(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
    dry_run: Option<bool>,
    as_of_date: Option<String>,
) -> Result<serde_json::Value, String> {
    let quote = state
        .astock_client
        .get_quote(&stock_code)
        .await
        .map_err(|e| format!("行情获取失败: {e}"))?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let analysis_id = uuid::Uuid::new_v4().to_string();

    stock_analyses::ActiveModel {
        id: Set(analysis_id.clone()),
        stock_code: Set(stock_code.clone()),
        stock_name: Set(quote.name.clone()),
        // B12: 在 as-of 模式下,analysis_date 必须是 as-of 截止日,而不是 today
        // —— spec §4.1 闭世界假设要求工作流产物日期 = 截断日,否则回放历史会串味
        analysis_date: Set(as_of::current_as_of()
            .map(|c| c.as_string())
            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string())),
        provider_id: Set("workflow".into()),
        conversation_id: Set(uuid::Uuid::new_v4().to_string()),
        status: Set("running".into()),
        decision_action: Set(None),
        decision_position_pct: Set(None),
        decision_reasoning: Set(None),
        decision_json: Set(None),
        blackboard_snapshot: Set(None),
        config_id: Set(None),
        // Time-travel metadata: 标记该 analysis 为 replay 模式 + 截止日
        analysis_kind: Set(if as_of_date.is_some() {
            "replay".into()
        } else {
            "live".into()
        }),
        as_of_date: Set(as_of_date.clone()),
        model_version: Set(None),
        data_snapshot_id: Set(None),
        outcome: Set(None),
        decision_time_horizon: Set(None),
        decision_expected_holding_days: Set(None),
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
    }
    .insert(state.harness.db())
    .await
    .map_err(|e| format!("DB 写入失败: {e}"))?;

    // ── 数据质量预检：在发起 DAG 执行前检查关键数据是否完整 ──
    let stock_code_for_check = stock_code.clone();
    let quality_check =
        data_quality_precheck(&state.astock_client, &stock_code_for_check, &quote).await;
    match quality_check {
        QualityPrecheckResult::Insufficient(reason) => {
            tracing::warn!(
                "[stock_workflow] 数据质量不足，跳过 DAG 执行: {reason} ({}",
                stock_code_for_check
            );
            // 更新 stock_analyses 状态
            let _ = stock_analyses::Entity::update(stock_analyses::ActiveModel {
                id: Set(analysis_id.clone()),
                status: Set("failed".into()),
                decision_json: Set(Some(
                    json!({
                        "action": "skip",
                        "reasoning": format!("数据不足，跳过分析: {reason}"),
                    })
                    .to_string(),
                )),
                updated_at: Set(chrono::Utc::now().timestamp_millis()),
                ..Default::default()
            })
            .exec(state.harness.db())
            .await;
            return Ok(json!({
                "status": "skipped",
                "reason": reason,
                "analysis_id": analysis_id,
            }));
        },
        QualityPrecheckResult::Pass => {
            // 数据充分，正常执行
        },
        QualityPrecheckResult::Partial(reason) => {
            tracing::info!("stock_workflow] 数据质量部分缺失，继续分析: {reason}");
        },
    }

    let loaded =
        load_and_inject_template(state.harness.db(), &stock_code, &quote.name, "stock-analysis")
            .await?;

    if let Some(ref vars) = loaded.variables {
        for v in vars {
            if v.name == "vendor_iwencai_key" {
                if let serde_json::Value::String(ref key) = v.value {
                    if !key.is_empty() {
                        *state.astock_client.iwencai_key.write().await = key.clone();
                    }
                }
            }
            if v.name == "vendor_xueqiu_token" {
                if let serde_json::Value::String(ref token) = v.value {
                    if !token.is_empty() {
                        if let Some(ref xq) = state.astock_client.xq_token {
                            *xq.write().await = token.clone();
                        }
                    }
                }
            }
        }
    }

    let engine = Arc::clone(&state.work_engine);

    // ── 从模板变量中解析执行参数 ──
    // max_concurrent / step_timeout 之前在 RunOptions 中硬编码为 9/300，
    // 现在通过模板变量 `max_concurrent` / `agent_timeout_secs` 让用户在设置面板调整。
    let (max_concurrent, step_timeout) = resolve_runtime_options(loaded.variables.as_deref());

    let wf_name = format!("stock-analysis-{stock_code}");
    let workflow = engine
        .create_workflow(&wf_name, loaded.nodes, loaded.edges)
        .await
        .map_err(|e| format!("创建工作流失败: {e}"))?;
    let wf_id = workflow.id.clone();
    let wf_id_ret = wf_id.clone();
    let app_h = app.clone();
    let db = state.harness.db().clone();
    let aid = analysis_id.clone();

    let progress_app = app.clone();
    let progress_wf_id = wf_id.clone();
    let progress_cb: ProgressCallback = Arc::new(move |event: StepProgressEvent| {
        let app = progress_app.clone();
        let wf_id = progress_wf_id.clone();
        Box::pin(async move {
            let payload = serde_json::json!({
                "workflowId": wf_id,
                "nodeId": event.node_id,
                "status": event.status,
                "totalNodes": event.total_nodes,
                "completedNodes": event.completed_nodes,
                "executionId": event.execution_id,
                "output": event.output,
                "error": event.error,
                "elapsedMs": event.elapsed_ms,
            });
            let _ = app.emit("workflow-step-done", payload);
        })
    });

    let input_schema = loaded.input_schema;
    let output_schema = loaded.output_schema;
    let template_vars = loaded.variables;

    // 读取 min_confidence 阈值（在 tokio::spawn 之外读取，捕获到闭包中）
    // 来自 StockAnalysisConfigPanel 的 "min_confidence" 变量，默认 60
    let min_confidence: u8 = template_vars
        .as_deref()
        .and_then(|vars| vars.iter().find(|v| v.name == "min_confidence"))
        .and_then(|v| v.value.as_f64())
        .map(|n| n.clamp(0.0, 100.0) as u8)
        .unwrap_or(0);

    let sc_for_ret = stock_code.clone();
    let sc_name = quote.name.clone();
    let sc_name_for_spawn = sc_name.clone();
    let vector_store = state.vector_store.clone();
    let master_key = state.harness.master_key_owned();
    // 在 spawn 前拉取市场状态（沪深300判断牛/熊/震荡），捕获到闭包中
    let market_regime_json: Option<serde_json::Value> = state
        .astock_client
        .get_klines("000300", "daily", 60)
        .await
        .ok()
        .and_then(|klines| {
            if klines.is_empty() {
                return None;
            }
            let r = axagent_stock_analysis::market_regime::classify_regime(&klines);
            Some(serde_json::json!({
                "regime": r.regime,
                "confidence": r.confidence,
                "volatility": r.volatility,
                "description": r.description,
            }))
        });
    tokio::spawn(async move {
        let mut opts = RunOptions {
            max_concurrent,
            step_timeout,
            progress_callback: Some(progress_cb),
            input: Some(json!({"stock_code": &stock_code})),
            input_schema: input_schema.clone(),
            output_schema: output_schema.clone(),
            dry_run: dry_run.unwrap_or(false),
            ..Default::default()
        };
        let mut merged_vars: Vec<axagent_harness::workflow_types::Variable> = vec![
            axagent_harness::workflow_types::Variable {
                name: "stock_code".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(stock_code.clone()),
                description: Some("当前分析的股票代码".into()),
                is_secret: false,
            },
            axagent_harness::workflow_types::Variable {
                name: "stock_name".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(sc_name_for_spawn.clone()),
                description: Some("当前分析的股票名称".into()),
                is_secret: false,
            },
        ];
        if let Some(d) = as_of_date.as_deref() {
            merged_vars.push(axagent_harness::workflow_types::Variable {
                name: "as_of_date".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(d.to_string()),
                description: Some("时间旅行模式截止日 (YYYY-MM-DD)；live 模式为空".into()),
                is_secret: false,
            });
        }
        if let Some(v) = template_vars {
            for tv in v {
                if !merged_vars.iter().any(|mv| mv.name == tv.name) {
                    merged_vars.push(tv);
                }
            }
        }
        // 注入相似历史决策案例（失败案例优先，最多 5 条）
        let similar_cases_str = fetch_similar_cases(&stock_code, &db).await;
        if let Some(ref cases) = similar_cases_str {
            merged_vars.push(axagent_harness::workflow_types::Variable {
                name: "similar_cases".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(cases.clone()),
                description: Some("相似历史决策（失败案例，供避免重复错误）".into()),
                is_secret: false,
            });
        }
        // 注入市场状态（沪深300判断牛/熊/震荡）
        if let Some(ref regime) = market_regime_json {
            merged_vars.push(axagent_harness::workflow_types::Variable {
                name: "market_regime".into(),
                var_type: "object".into(),
                value: regime.clone(),
                description: Some("当前市场状态(bull/bear/sideways)+波动率+描述".into()),
                is_secret: false,
            });
        }
        // 注入历史反思教训（从 stock_reflections 表取最近的结构化反思结果）
        let lessons_str = fetch_stock_lessons(&stock_code, &db).await;
        if let Some(ref lessons) = lessons_str {
            merged_vars.push(axagent_harness::workflow_types::Variable {
                name: "stock_lessons".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(lessons.clone()),
                description: Some("该股历史反思教训（错因/被忽视信号/改进建议）".into()),
                is_secret: false,
            });
        }
        opts.variables = Some(merged_vars);

        match engine.run_workflow(&wf_id, opts).await {
            Ok(result) => {
                let wf_status = result.status;
                match wf_status {
                    axagent_rt_workflow::workflow_engine::WorkflowStatus::Cancelled => {
                        let _ = app_h.emit(
                            "workflow-error",
                            serde_json::json!({ "workflowId": wf_id, "error": "分析已被取消" }),
                        );
                        let _ = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("cancelled"))
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await;
                    },
                    axagent_rt_workflow::workflow_engine::WorkflowStatus::Failed => {
                        tracing::warn!(%wf_id, status=?wf_status, "工作流以 Failed 状态结束");
                        let _ = app_h.emit(
                            "workflow-error",
                            serde_json::json!({
                                "workflowId": wf_id,
                                "error": "部分分析步骤失败",
                                "results": result.results,
                                "output": result.output,
                            }),
                        );
                        let _ = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("failed"))
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await;
                    },
                    _ => {
                        let _ = app_h.emit(
                            "workflow-completed",
                            serde_json::json!({
                                "workflowId": wf_id,
                                "results": result.results,
                                "output": result.output,
                            }),
                        );
                        let decision_json = result
                            .output
                            .and_then(|v| serde_json::to_string(&v).ok())
                            .or_else(|| {
                                result
                                    .results
                                    .get("portfolio-mgr")
                                    .and_then(|v| serde_json::to_string(v).ok())
                            });
                        let (
                            mut action,
                            position_pct,
                            mut reasoning,
                            time_horizon,
                            expected_holding_days,
                        ) = extract_decision_fields(&decision_json);
                        // Level 1: min_confidence 过滤 — 若 LLM 自报置信度低于阈值，
                        // 将 action 覆盖为 "uncertain" 并标注在 reasoning 中
                        if min_confidence > 0 {
                            if let Some(conf) = decision_json
                                .as_ref()
                                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                                .and_then(|v| v.get("confidence").and_then(|c| c.as_f64()))
                            {
                                if conf < min_confidence as f64 {
                                    let orig_action = action.clone().unwrap_or_default();
                                    let orig_reason = reasoning.clone().unwrap_or_default();
                                    action = Some("uncertain".to_string());
                                    reasoning = Some(format!(
                                        "置信度 {:.0} 低于阈值 {min_confidence}，建议观望。原分析: {}\n原动作: {}",
                                        conf, orig_reason, orig_action
                                    ));
                                }
                            }
                        }
                        // 克隆决策字段供 Memory RAG 索引（原值将被 DB 写入消费）
                        let mem_action = action.clone();
                        let mem_reasoning = reasoning.clone();
                        let mem_dj = decision_json.clone();
                        // 持久化工作流结果到 blackboard_snapshot，供历史回放/报告
                        // 生成/跨日 key_levels 聚合使用。修复 Defect #2。
                        // B7: 消费 take_asof_degradation_report() 写入 `degraded` 块
                        // (spec §4.1: vendor 降级报告)
                        let as_of_for_meta: Option<AsOfContext> = as_of::current_as_of();
                        let degradation_report = as_of::take_asof_degradation_report();
                        let bb_snapshot = serde_json::to_string(&build_blackboard_snapshot(
                            &result.results,
                            as_of_for_meta.as_ref(),
                            &degradation_report,
                        ))
                        .unwrap_or_else(|_| "{}".to_string());
                        let _ = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("completed"))
                            .col_expr(stock_analyses::Column::DecisionAction, Expr::value(action))
                            .col_expr(
                                stock_analyses::Column::DecisionPositionPct,
                                Expr::value(position_pct),
                            )
                            .col_expr(
                                stock_analyses::Column::DecisionReasoning,
                                Expr::value(reasoning),
                            )
                            .col_expr(
                                stock_analyses::Column::DecisionJson,
                                Expr::value(decision_json),
                            )
                            .col_expr(
                                stock_analyses::Column::BlackboardSnapshot,
                                Expr::value(bb_snapshot),
                            )
                            .col_expr(
                                stock_analyses::Column::DecisionTimeHorizon,
                                Expr::value(time_horizon),
                            )
                            .col_expr(
                                stock_analyses::Column::DecisionExpectedHoldingDays,
                                Expr::value(expected_holding_days.map(|d| d as i64)),
                            )
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await;

                        // 索引决策到 Memory RAG（best-effort，失败不阻塞）
                        if let Some(ref dj) = mem_dj {
                            if !dj.is_empty() {
                                let confidence_str = serde_json::from_str::<serde_json::Value>(dj)
                                    .ok()
                                    .and_then(|v| v.get("confidence").and_then(|c| c.as_f64()))
                                    .map(|c| format!("{:.0}", c))
                                    .unwrap_or_else(|| "?".to_string());
                                let memory_content = format!(
                                    "股票:{} {} 决策:{} 置信度:{} 日期:{}\n{}",
                                    stock_code,
                                    sc_name_for_spawn,
                                    mem_action.as_deref().unwrap_or(""),
                                    confidence_str,
                                    chrono::Utc::now().format("%Y-%m-%d"),
                                    mem_reasoning.as_deref().unwrap_or(""),
                                );
                                let _ = crate::indexing::index_memory_item(
                                    &db,
                                    &master_key,
                                    &vector_store,
                                    "stock_decisions",
                                    &aid,
                                    &memory_content,
                                    "openai::text-embedding-3-small",
                                    None,
                                )
                                .await;
                            }
                        }
                    },
                }
            },
            Err(e) => {
                let _ = app_h.emit(
                    "workflow-error",
                    serde_json::json!({ "workflowId": wf_id, "error": e.to_string() }),
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
        "workflowId": wf_id_ret,
        "stockCode": sc_for_ret,
        "stockName": sc_name,
    }))
}

/// 取消正在运行的股票分析工作流
#[tauri::command]
pub async fn cancel_stock_workflow(
    state: State<'_, AppState>,
    workflow_id: String,
) -> Result<(), String> {
    state
        .work_engine
        .cancel_workflow(&workflow_id)
        .await
        .map(|_| ())
        .map_err(|e| format!("取消工作流失败: {e}"))
}

// ── 批量/定时分析入口（无 Tauri State 依赖，供 CronExecutor 调用）──

/// 对单只股票执行完整分析（无 Tauri 事件发射，适合批量定时扫描）
///
/// 与 `run_stock_workflow_inner` 逻辑相同但：
/// - 不发射 `workflow-step-done` 事件（无前端监听）
/// - 不需要 `as_of_date` 参数（使用当前时间，非回放模式）
/// - 不需要 `dry_run`（总是完整执行）
/// - 参数是独立引用而非 Tauri State
pub async fn run_single_stock_analysis(
    db: &DatabaseConnection,
    client: &axagent_astock_data::AStockClient,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    stock_code: &str,
    stock_name: &str,
) -> Result<String, String> {
    // 1. 创建 stock_analyses 记录
    let now_ms = chrono::Utc::now().timestamp_millis();
    let analysis_id = uuid::Uuid::new_v4().to_string();

    stock_analyses::ActiveModel {
        id: Set(analysis_id.clone()),
        stock_code: Set(stock_code.to_string()),
        stock_name: Set(stock_name.to_string()),
        analysis_date: Set(chrono::Utc::now().format("%Y-%m-%d").to_string()),
        provider_id: Set("workflow".into()),
        conversation_id: Set(uuid::Uuid::new_v4().to_string()),
        status: Set("running".into()),
        decision_action: Set(None),
        decision_position_pct: Set(None),
        decision_reasoning: Set(None),
        decision_json: Set(None),
        blackboard_snapshot: Set(None),
        config_id: Set(None),
        analysis_kind: Set("live".into()),
        as_of_date: Set(None),
        model_version: Set(None),
        data_snapshot_id: Set(None),
        outcome: Set(None),
        decision_time_horizon: Set(None),
        decision_expected_holding_days: Set(None),
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
    }
    .insert(db)
    .await
    .map_err(|e| format!("DB 写入失败: {e}"))?;

    // 2. 获取行情（用于数据预检和 stock name）
    let quote = client
        .get_quote(stock_code)
        .await
        .map_err(|e| format!("行情获取失败: {e}"))?;

    // 3. 数据质量预检
    match data_quality_precheck(client, stock_code, &quote).await {
        QualityPrecheckResult::Insufficient(reason) => {
            let _ = stock_analyses::Entity::update(stock_analyses::ActiveModel {
                id: Set(analysis_id.clone()),
                status: Set("failed".into()),
                decision_json: Set(Some(
                    json!({
                        "action": "skip",
                        "reasoning": format!("数据不足，跳过分析: {reason}"),
                    })
                    .to_string(),
                )),
                updated_at: Set(chrono::Utc::now().timestamp_millis()),
                ..Default::default()
            })
            .exec(db)
            .await;
            return Err(reason);
        },
        QualityPrecheckResult::Pass | QualityPrecheckResult::Partial(_) => {
            // 继续执行
        },
    }

    // 4. 加载模板并注入 stock_code
    let loaded = load_and_inject_template(db, stock_code, stock_name, "stock-analysis").await?;

    // 5. 解析运行时参数
    let (max_concurrent, step_timeout) = resolve_runtime_options(loaded.variables.as_deref());

    // 6. 创建并运行工作流
    let wf_name = format!("stock-analysis-{stock_code}-batch");
    let workflow = engine
        .create_workflow(&wf_name, loaded.nodes, loaded.edges)
        .await
        .map_err(|e| format!("创建工作流失败: {e}"))?;
    let wf_id = workflow.id.clone();

    let opts = RunOptions {
        max_concurrent,
        step_timeout,
        progress_callback: None,
        input: Some(json!({"stock_code": stock_code})),
        input_schema: loaded.input_schema.clone(),
        output_schema: loaded.output_schema.clone(),
        dry_run: false,
        ..Default::default()
    };

    let result = engine.run_workflow(&wf_id, opts).await;

    match result {
        Ok(wf) => {
            // 更新为完成状态
            let decision_output = wf
                .results
                .get("portfolio-mgr")
                .and_then(|v| serde_json::from_value::<serde_json::Value>(v.clone()).ok());

            let decision_action = decision_output.as_ref().and_then(|d| {
                d.get("action")
                    .and_then(|a| a.as_str().map(|s| s.to_string()))
            });

            let _ = stock_analyses::Entity::update(stock_analyses::ActiveModel {
                id: Set(analysis_id.clone()),
                status: Set("completed".into()),
                decision_action: Set(decision_action),
                decision_json: Set(decision_output.map(|d| d.to_string())),
                updated_at: Set(chrono::Utc::now().timestamp_millis()),
                ..Default::default()
            })
            .exec(db)
            .await;

            tracing::info!(
                "[batch_analysis] {stock_code} ({stock_name}) 完成, status={:?}",
                wf.status
            );
            Ok(analysis_id)
        },
        Err(e) => {
            let err_msg = format!("{:?}", e);
            let _ = stock_analyses::Entity::update(stock_analyses::ActiveModel {
                id: Set(analysis_id.clone()),
                status: Set("failed".into()),
                decision_json: Set(Some(
                    json!({
                        "action": "error",
                        "reasoning": err_msg.clone(),
                    })
                    .to_string(),
                )),
                updated_at: Set(chrono::Utc::now().timestamp_millis()),
                ..Default::default()
            })
            .exec(db)
            .await;

            tracing::error!("[batch_analysis] {stock_code} 失败: {err_msg}");
            Err(err_msg)
        },
    }
}

/// 从 stock_analyses 表查询同股票过去 3 个月的失败案例，返回格式化文本。
async fn fetch_similar_cases(stock_code: &str, db: &sea_orm::DatabaseConnection) -> Option<String> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    let three_months_ago = (chrono::Utc::now() - chrono::Duration::days(90))
        .format("%Y-%m-%d")
        .to_string();
    let all = stock_analyses::Entity::find()
        .filter(stock_analyses::Column::StockCode.eq(stock_code))
        .filter(stock_analyses::Column::Outcome.eq("loss"))
        .filter(stock_analyses::Column::AnalysisDate.gte(&three_months_ago))
        .order_by(stock_analyses::Column::AnalysisDate, sea_orm::Order::Desc)
        .all(db)
        .await
        .unwrap_or_default();
    let similar: Vec<_> = all.into_iter().take(5).collect();
    if similar.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    for s in similar {
        let conf = s
            .decision_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
            .and_then(|v| v.get("confidence").and_then(|c| c.as_f64()))
            .map(|c| format!("{}", c as u8))
            .unwrap_or_else(|| "?".to_string());
        let action = s.decision_action.as_deref().unwrap_or("?");
        let reasoning = s.decision_reasoning.as_deref().unwrap_or("");
        let abbr = if reasoning.len() > 60 {
            &reasoning[..60]
        } else {
            reasoning
        };
        lines.push(format!(
            "- 日期:{} 决策:{} 置信度:{} → 失败。要点:{}",
            s.analysis_date, action, conf, abbr
        ));
    }
    Some(lines.join("\n"))
}
/// 从 stock_reflections 表查询该股最近的结构化反思教训（错因/被忽视信号/改进建议），返回格式化文本。
async fn fetch_stock_lessons(stock_code: &str, db: &sea_orm::DatabaseConnection) -> Option<String> {
    use chrono::Utc;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
    let three_months_ago = Utc::now() - chrono::Duration::days(90);
    let all = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::StockCode.eq(stock_code))
        .filter(stock_reflections::Column::CreatedAt.gte(three_months_ago.timestamp_millis()))
        .order_by_desc(stock_reflections::Column::CreatedAt)
        .all(db)
        .await
        .unwrap_or_default();
    let lessons: Vec<_> = all.into_iter().take(3).collect();
    if lessons.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    for (i, l) in lessons.iter().enumerate() {
        lines.push(format!("#{} 反思于 {}", i + 1, l.hindsight_date));
        if let Some(ref w) = l.what_went_wrong {
            lines.push(format!("  - 错因：{}", w));
        }
        if let Some(ref m) = l.missed_signals {
            lines.push(format!("  - 被忽视信号：{}", m));
        }
        if let Some(ref f) = l.fix_for_future {
            lines.push(format!("  - 改进建议：{}", f));
        }
    }
    Some(lines.join("\n"))
}

/// 反思复盘工作流：嵌套原股票分析工作流的 as-of，取后见信息对比，反思。
///
/// 加载与 [run_single_stock_analysis] 相同的 stock-analysis DAG，
/// 设置 as_of_date 回到原始分析日期（数据与原分析一致），
/// 注入 `actual_outcome` 变量让 portfolio-manager 产生反思。
///
/// 结果写入独立的 `stock_reflections` 表。
pub async fn run_reflection_workflow(
    db: &DatabaseConnection,
    _client: &axagent_astock_data::AStockClient,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    vector_store: &axagent_core::vector_store::VectorStore,
    master_key: &[u8; 32],
    stock_code: &str,
    stock_name: &str,
    original_analysis_id: &str,
    actual_outcome: &str,
    as_of_date: &str,
    hindsight_date: &str,
    min_confidence_threshold: u8,
    reflection_depth: &str,
) -> Result<String, String> {
    use axagent_astock_data::as_of;
    use axagent_core::entity::stock_reflections;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let analysis_id = uuid::Uuid::new_v4().to_string();

    // 1. 插入反思记录（初始状态）
    stock_reflections::ActiveModel {
        id: Set(analysis_id.clone()),
        stock_code: Set(stock_code.to_string()),
        stock_name: Set(stock_name.to_string()),
        original_analysis_id: Set(original_analysis_id.to_string()),
        as_of_date: Set(as_of_date.to_string()),
        hindsight_date: Set(hindsight_date.to_string()),
        min_confidence_threshold: Set(min_confidence_threshold as i32),
        reflection_depth: Set(reflection_depth.to_string()),
        actual_outcome: Set(actual_outcome.to_string()),
        what_went_wrong: Set(None),
        missed_signals: Set(None),
        fix_for_future: Set(None),
        parameter_suggestions_json: Set(None),
        decision_json: Set(None),
        blackboard_snapshot: Set(None),
        model_version: Set(None),
        status: Set("running".to_string()),
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
    }
    .insert(db)
    .await
    .map_err(|e| format!("DB 写入失败: {e}"))?;

    // 2. 加载反思复盘模板（stock-reflection，DAG 结构与 stock-analysis 一致）
    let loaded = load_and_inject_template(db, stock_code, stock_name, "stock-reflection").await?;
    let (max_concurrent, step_timeout) = resolve_runtime_options(loaded.variables.as_deref());

    // 3. 创建嵌套工作流
    let wf_name = format!("stock-reflection-{stock_code}");
    let workflow = engine
        .create_workflow(&wf_name, loaded.nodes, loaded.edges)
        .await
        .map_err(|e| format!("创建反思工作流失败: {e}"))?;
    let wf_id = workflow.id.clone();

    // 4. 加载原始决策的时间维度信息
    // 手动触发时 original_analysis_id="" —— 不注入这俩变量,让工作流模板
    // 自己决定怎么处理"无原始分析上下文"的情况。盲目注入 "unknown" / 0
    // 会污染反思推理的前提(持仓期对齐、时间维度匹配)。
    let original_ctx: Option<(String, i64)> = if original_analysis_id.is_empty() {
        None
    } else {
        let time_horizon = stock_analyses::Entity::find_by_id(original_analysis_id)
            .one(db)
            .await
            .ok()
            .flatten()
            .and_then(|a| a.decision_time_horizon);
        let holding_days = stock_analyses::Entity::find_by_id(original_analysis_id)
            .one(db)
            .await
            .ok()
            .flatten()
            .and_then(|a| a.decision_expected_holding_days.map(|d| d as i64));
        match (time_horizon, holding_days) {
            (Some(t), Some(h)) => Some((t, h)),
            _ => None,
        }
    };

    // 5. 注入变量
    let mut variables = vec![
        axagent_harness::workflow_types::Variable {
            name: "actual_outcome".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(actual_outcome.to_string()),
            description: Some("实际走势结果，格式如 '30天跌8% → 失败'".into()),
            is_secret: false,
        },
        axagent_harness::workflow_types::Variable {
            name: "reflection_depth".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(reflection_depth.to_string()),
            description: Some("反思深度：light(简要) / deep(详细推理链)".into()),
            is_secret: false,
        },
    ];
    if let Some((time_horizon, holding_days)) = original_ctx {
        variables.push(axagent_harness::workflow_types::Variable {
            name: "original_time_horizon".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(time_horizon),
            description: Some(
                "原始决策的时间维度：ultra_short(1-3天)/short(5天)/mid(28天)/long(90天+)".into(),
            ),
            is_secret: false,
        });
        variables.push(axagent_harness::workflow_types::Variable {
            name: "original_holding_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(holding_days),
            description: Some("原始决策期望持有天数（交易日）".into()),
            is_secret: false,
        });
    } else {
        tracing::info!(
            "[reflection] {}: 无原始分析上下文(original_analysis_id={:?}),跳过 original_* 变量注入",
            stock_code,
            original_analysis_id
        );
    }
    let opts = axagent_rt_workflow::work_engine::RunOptions {
        max_concurrent,
        step_timeout,
        progress_callback: None,
        input: Some(json!({"stock_code": stock_code})),
        input_schema: loaded.input_schema,
        output_schema: loaded.output_schema,
        dry_run: false,
        variables: Some(variables),
        ..Default::default()
    };

    // 5. as-of 范围执行
    let ctx = AsOfContext::parse(as_of_date).map_err(|e| format!("as_of 解析失败: {e}"))?;
    let result = as_of::AS_OF
        .scope(Some(ctx), async move { engine.run_workflow(&wf_id, opts).await })
        .await;

    // 6. 处理结果
    match result {
        Ok(wf) => {
            let reflection_json = wf
                .results
                .get("reflection")
                .and_then(|v| serde_json::from_value::<serde_json::Value>(v.clone()).ok());

            let (what_went_wrong, missed_signals, fix_for_future, params_suggestion_json) =
                reflection_json
                    .as_ref()
                    .and_then(|v| v.get("reflection"))
                    .map(|r| {
                        let w = r
                            .get("what_went_wrong")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let m = r.get("missed_signals").map(|v| v.to_string());
                        let f = r
                            .get("fix_for_future")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let p = r.get("params_suggestion").map(|v| v.to_string());
                        (w, m, f, p)
                    })
                    .unwrap_or((None, None, None, None));

            let bb_text = serde_json::to_string(&wf.results).unwrap_or_default();
            let dj_text = reflection_json.as_ref().map(|v| v.to_string());

            let _ = stock_reflections::Entity::update_many()
                .col_expr(stock_reflections::Column::Status, Expr::value("completed".to_string()))
                .col_expr(stock_reflections::Column::DecisionJson, Expr::value(dj_text))
                .col_expr(
                    stock_reflections::Column::WhatWentWrong,
                    Expr::value(what_went_wrong.clone()),
                )
                .col_expr(stock_reflections::Column::MissedSignals, Expr::value(missed_signals))
                .col_expr(stock_reflections::Column::FixForFuture, Expr::value(fix_for_future))
                .col_expr(
                    stock_reflections::Column::ParameterSuggestionsJson,
                    Expr::value(params_suggestion_json),
                )
                .col_expr(stock_reflections::Column::BlackboardSnapshot, Expr::value(bb_text))
                .filter(stock_reflections::Column::Id.eq(&analysis_id))
                .exec(db)
                .await;

            // 索引到 Memory RAG
            if let Some(ref w) = what_went_wrong {
                let memory_content = format!(
                    "反思:股票:{} {} 原始决策时间:{} 结果:{}\n错因:{}",
                    stock_code, stock_name, as_of_date, actual_outcome, w
                );
                let _ = crate::indexing::index_memory_item(
                    db,
                    master_key,
                    vector_store,
                    "stock_reflections",
                    &analysis_id,
                    &memory_content,
                    "openai::text-embedding-3-small",
                    None,
                )
                .await;
            }

            tracing::info!("[reflection] {}: 反思完成", stock_code);
            Ok(analysis_id)
        },
        Err(e) => {
            let err_msg = format!("反思工作流失败: {e}");
            let _ = stock_reflections::Entity::update_many()
                .col_expr(
                    stock_reflections::Column::Status,
                    Expr::value(format!("failed: {err_msg}")),
                )
                .filter(stock_reflections::Column::Id.eq(&analysis_id))
                .exec(db)
                .await;
            Err(err_msg)
        },
    }
}

// ── Serenity 瓶颈筛选工作流 ──

/// 运行 Serenity 瓶颈筛选工作流（serenity-screening 模板）。
///
/// 与 run_stock_workflow 不同：
///   - 不需要 stock_code 输入（自驱动，从市场数据发现趋势）
///   - 不写 stock_analyses 表
///   - 返回候选股清单（而非单只股票的分析结论）
#[tauri::command]
pub async fn run_serenity_screening(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    as_of_date: Option<String>,
) -> Result<serde_json::Value, String> {
    let engine = Arc::clone(&state.work_engine);

    // 解析 as_of_date（支持回放模式）
    let as_of_ctx = parse_asof_param(as_of_date.clone())?;

    // 1. 加载 serenity-screening 模板
    let loaded = load_and_inject_template(state.harness.db(), "", "", "serenity-screening").await?;

    let (max_concurrent, step_timeout) = resolve_runtime_options(loaded.variables.as_deref());

    // 2. 创建 Workflow
    let wf_name = format!("serenity-screening-{}", chrono::Utc::now().timestamp_millis());
    let workflow = engine
        .create_workflow(&wf_name, loaded.nodes, loaded.edges)
        .await
        .map_err(|e| format!("创建工作流失败: {e}"))?;
    let wf_id = workflow.id.clone();
    let wf_id_ret = wf_id.clone();
    let app_h = app.clone();

    // 3. 进度回调
    let progress_app = app.clone();
    let progress_wf_id = wf_id.clone();
    let progress_cb: ProgressCallback = Arc::new(move |event: StepProgressEvent| {
        let app = progress_app.clone();
        let wf_id = progress_wf_id.clone();
        Box::pin(async move {
            let payload = serde_json::json!({
                "workflowId": wf_id,
                "type": "serenity-screening",
                "nodeId": event.node_id,
                "status": event.status,
                "totalNodes": event.total_nodes,
                "completedNodes": event.completed_nodes,
                "output": event.output,
                "elapsedMs": event.elapsed_ms,
            });
            let _ = app.emit("serenity-screening-step", payload);
        })
    });

    // 4. 运行（支持 as-of 时间截断）
    let opts = RunOptions {
        max_concurrent,
        step_timeout,
        progress_callback: Some(progress_cb),
        input: None,
        input_schema: loaded.input_schema.clone(),
        output_schema: loaded.output_schema.clone(),
        dry_run: false,
        ..Default::default()
    };

    let exec = async { engine.run_workflow(&wf_id, opts).await };

    let result = if let Some(ctx) = as_of_ctx {
        as_of::AS_OF.scope(Some(ctx), exec).await
    } else {
        exec.await
    };

    match result {
        Ok(wf_result) => {
            let candidates_raw = wf_result
                .results
                .get("a-candidate-mapper")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            // Agent executor 将结构化输出存在 "params" 字段中，
            // 顶层是 { content, params, thinking, ... } 包装对象
            let has_params = candidates_raw
                .as_object()
                .and_then(|obj| obj.get("params"))
                .is_some();
            let candidates = if has_params {
                candidates_raw["params"].clone()
            } else {
                candidates_raw.clone()
            };

            tracing::info!(
                "[serenity] a-candidate-mapper 输出类型: {}",
                if has_params {
                    "有 params 字段，已提取"
                } else {
                    "无 params 字段，直接使用原始值"
                }
            );

            // 持久化 Serenity 候选到 reco_picks 表（style="serenity"）
            {
                let db = state.harness.db();
                let now_str = chrono::Utc::now().to_rfc3339();
                let ts_ms = chrono::Utc::now().timestamp_millis();
                // candidates 可能是 { candidates: [...] } 对象，也可能是原始数组
                let candidate_list: Vec<&serde_json::Value> = candidates
                    .as_object()
                    .and_then(|obj| obj.get("candidates"))
                    .and_then(|v| v.as_array())
                    .or_else(|| candidates.as_array())
                    .map(|arr| arr.iter().collect())
                    .unwrap_or_default();
                for (i, c) in candidate_list.iter().enumerate() {
                    let code = c["stock_code"].as_str().unwrap_or("");
                    let name = c["stock_name"].as_str().unwrap_or("");
                    let conf = c["confidence"].as_i64().unwrap_or(50) as i32;
                    if code.is_empty() {
                        continue;
                    }
                    let pick_id = format!("serenity-{ts_ms}-{i}-{code}");
                    reco_picks::ActiveModel {
                        id: Set(pick_id),
                        generated_at: Set(now_str.clone()),
                        period: Set("mid".to_string()),
                        stock_code: Set(code.to_string()),
                        stock_name: Set(name.to_string()),
                        style: Set("serenity".to_string()),
                        confidence: Set(conf),
                        synthetic: Set(0),
                        seed_pool_json: Set(None),
                        strategy_weights_json: Set(None),
                        created_at: Set(now_str.clone()),
                    }
                    .insert(db)
                    .await
                    .map_err(|e| format!("写入 Serenity 候选到 reco_picks 失败: {e}"))?;
                }
                // 同步 Serenity 候选到全局种子，供 SerenityStrategy 读取
                if !candidate_list.is_empty() {
                    let serenity_seed: Vec<(String, String, Option<String>)> = candidate_list
                        .iter()
                        .map(|c| {
                            let code = c["stock_code"].as_str().unwrap_or("").to_string();
                            let name = c["stock_name"].as_str().unwrap_or("").to_string();
                            (code, name, None)
                        })
                        .collect();
                    axagent_stock_analysis::recommender::set_serenity_seed(serenity_seed);
                }
            }

            let _ = app_h.emit(
                "serenity-screening-completed",
                serde_json::json!({
                    "workflowId": wf_id_ret,
                    "status": "completed",
                    "result": candidates,
                }),
            );
            Ok(serde_json::json!({
                "status": "completed",
                "candidates": candidates,
            }))
        },
        Err(e) => {
            let err_msg = format!("Serenity 筛选工作流失败: {e}");
            let _ = app_h.emit(
                "serenity-screening-completed",
                serde_json::json!({
                    "workflowId": wf_id_ret,
                    "status": "failed",
                    "error": err_msg,
                }),
            );
            Err(err_msg)
        },
    }
}
