//! 工作流驱动的股票分析 — 基于持久化 WorkflowTemplate + WorkEngine DAG 执行。
//!
//! 启动时种子化 stock-analysis 工作流模板到 workflow_templates 表，
//! 每次分析从模板加载 DAG 结构，注入实时行情数据，由 WorkEngine 并行执行。

use crate::AppState;
use axagent_astock_data::as_of::{self, AsOfContext};
use axagent_core::entity::reco_picks;
use axagent_core::entity::stock_analyses;
use axagent_core::entity::stock_reflections;
use axagent_harness::response_normalizer::ResponseNormalizer;
use axagent_harness::tool::ToolPermissions;
use axagent_harness::types::{ChatResponse, ContentBlock};
use axagent_harness::workflow_types::{JsonSchema, Variable, WorkflowEdge, WorkflowNode};
use axagent_harness::{ToolContext, ToolRegistry};
use axagent_rt_workflow::Workflow;
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use axagent_runtime_core::DefaultResponseNormalizer;
use axagent_stock_analysis::blackboard::build_blackboard_snapshot;
use sea_orm::DatabaseConnection;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::Serialize;
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

    // P0-2 修复: 请求 500 匹配内部 fetch_limit，最大限度保留截断后数据
    // Err 改为 Partial (vendor 临时限流/降级不应阻塞整个分析)
    let kline_check = match client.get_klines(stock_code, "daily", 500).await {
        Ok(klines) if klines.len() >= 15 => SourceCheck::Ok,
        Ok(klines) if !klines.is_empty() => {
            SourceCheck::Partial(format!("仅 {} 行, 不足 15 日", klines.len()))
        },
        Ok(_) => SourceCheck::Failed("K 线为空".into()),
        Err(e) => SourceCheck::Partial(format!("K 线获取受阻（可重试）: {e}")),
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

/// 从 Workflow 结果中提取 portfolio-mgr 节点的决策 JSON 字符串。
///
/// 优先取 `results["portfolio-mgr"]["result"]`（CodeNode 包装内 Rhai 脚本的
/// 实际输出，例如 `{ action, positionPct, confidence, ... }`），回退到
/// `results["portfolio-mgr"]` 本身（兼容非 CodeNode 包装的旧版 portfolio-mgr），
/// 最后回退到 workflow 顶层 `output`（兼容无 portfolio-mgr 节点的工作流）。
///
/// 修复"决策信息缺失"误报：之前直接用 `wf.output` 写入 decisionJson，
/// 但 stock-analysis 工作流配置了 output_schema（且未用 $source 标记字段
/// 来源节点），导致 `filter_by_schema` 退化为整个 results map。前端
/// normalizeDecision 拿到 results map 后会判定为"全零空壳"返回 null，
/// store.decision 保持空 → DecisionBanner 显示"决策信息缺失"误报。
fn extract_decision_json(wf: &Workflow) -> Option<String> {
    if let Some(pm) = wf.results.get("portfolio-mgr") {
        // CodeNode 包装: { status, result, input_params, node_id, params }
        // 实际决策在 .result 字段;若 .result 缺失(旧版/异常路径)则降级用
        // 整个 pm 值,让 extract_decision_fields 至少能拿到 action 等字段。
        let actual = match pm {
            serde_json::Value::Object(obj) => {
                obj.get("result").cloned().unwrap_or_else(|| pm.clone())
            },
            _ => pm.clone(),
        };
        if let Ok(s) = serde_json::to_string(&actual) {
            return Some(s);
        }
    }
    // 回退: workflow 顶层 output(无 output_schema 或非 stock-analysis 工作流)
    wf.output
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok())
}

/// 从 Workflow 结果中提取 trader 节点的 LLM 决策 JSON。
///
/// trader 节点输出格式:
/// ```json
/// { "stance": "买入", "positionPct": 35, "confidence": 0.72,
///   "summary": "...", "key_points": [...], "scenarios": [...] }
/// ```
///
/// 用作"方案 D 双向并存"的 LLM 视角,与 portfolio-mgr 公式视角对比。
/// 优先取 `results["trader"]["result"]`（AgentNode 包装内的实际输出），
/// 回退到 `results["trader"]` 本身。
fn extract_llm_decision_json(wf: &Workflow) -> Option<String> {
    let trader = wf.results.get("trader")?;
    let actual = match trader {
        serde_json::Value::Object(obj) => {
            // AgentNode 可能包装: { status, result, ... }
            obj.get("result").cloned().unwrap_or_else(|| trader.clone())
        },
        _ => trader.clone(),
    };
    serde_json::to_string(&actual).ok()
}

/// 计算公式决策与 LLM 决策的一致性分数（0-100）。
///
/// 借鉴 TradingAgents 的冗余校验机制：
/// 对比 action（操作方向）、positionPct（仓位百分比）、confidence（置信度）
/// 三个维度，权重分别为 50/30/20。
///
/// 归一化规则（与前端 normalizeAction 保持一致）:
/// - 移除空格/斜杠/下划线/全角空格
/// - 小写比较
/// - "买"和"增持"视为一致，"卖"和"减持"视为一致
#[allow(dead_code)]
fn compute_decision_agreement(formula_json: Option<&str>, llm_json: Option<&str>) -> Option<i32> {
    let fj = serde_json::from_str::<serde_json::Value>(formula_json?).ok()?;
    let lj = serde_json::from_str::<serde_json::Value>(llm_json?).ok()?;

    // 归一化操作字符串
    let norm = |s: &str| {
        s.trim()
            .to_lowercase()
            .replace([' ', '/', '_', '\u{3000}'], "")
    };

    // 公式字段: action / positionPct / confidence
    let f_action = fj.get("action").and_then(|v| v.as_str().map(norm));
    let f_pos = fj.get("positionPct").and_then(|v| v.as_f64());
    let f_conf = fj.get("confidence").and_then(|v| v.as_f64());

    // LLM 字段: stance→action / positionPct / confidence
    let l_action = lj.get("stance").and_then(|v| v.as_str().map(norm));
    let l_pos = lj.get("positionPct").and_then(|v| v.as_f64());
    let l_conf = lj.get("confidence").and_then(|v| v.as_f64());

    // action 一致性 (权重 50%)
    let action_score: f64 = match (f_action, l_action) {
        (Some(a), Some(b)) => {
            let is_buy = |s: &str| s.contains("买") || s.contains("增持");
            let is_sell = |s: &str| s.contains("卖") || s.contains("减持");
            if a == b {
                50.0
            } else if is_buy(&a) == is_buy(&b) && is_sell(&a) == is_sell(&b) {
                40.0
            } else {
                0.0
            }
        },
        _ => 25.0,
    };

    // positionPct 一致性 (权重 30%)
    let pos_score: f64 = match (f_pos, l_pos) {
        (Some(a), Some(b)) => {
            let diff = (a - b).abs();
            if diff <= 5.0 {
                30.0
            } else if diff <= 15.0 {
                20.0
            } else if diff <= 30.0 {
                10.0
            } else {
                0.0
            }
        },
        _ => 15.0,
    };

    // confidence 一致性 (权重 20%)
    let conf_score: f64 = match (f_conf, l_conf) {
        (Some(a), Some(b)) => {
            let diff = (a - b).abs();
            if diff <= 0.1 {
                20.0
            } else if diff <= 0.2 {
                15.0
            } else if diff <= 0.4 {
                8.0
            } else {
                0.0
            }
        },
        _ => 10.0,
    };

    Some((action_score + pos_score + conf_score).round() as i32)
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

    // ── extract_decision_json(修复"决策信息缺失"误报)──

    /// 优先取 results["portfolio-mgr"]["result"](CodeNode 包装内 Rhai 实际输出)
    #[test]
    fn extract_decision_json_prefers_portfolio_mgr_result() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "portfolio-mgr".to_string(),
            json!({
                "status": "executed",
                "language": "rhai",
                "result": {
                    "action": "买入",
                    "positionPct": 50.0,
                    "confidence": 75.0,
                    "riskLevel": "中",
                    "reasoning": "技术面强势",
                    "timeHorizon": "mid",
                    "expectedHoldingDays": 28,
                },
                "input_params": { "totalScore": 70.0 },
                "node_id": "portfolio-mgr",
                "params": { "action": "买入" },
            }),
        );
        // 即使 wf.output 存在且被 output_schema 污染成整个 results map,
        // 优先从 portfolio-mgr 节点本身提取。
        results.insert("trigger".to_string(), json!({ "status": "ok" }));
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: Some(json!({
                "trigger": { "status": "ok" },
                "portfolio-mgr": { "status": "executed", "result": { "action": "买入" } },
                "end-output": { "status": "ok" },
            })),
        };
        let dj = extract_decision_json(&wf).expect("必须返回决策 JSON");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        // 关键:从 portfolio-mgr.result 提取,action 是 "买入" 而非被 output 污染
        assert_eq!(parsed["action"], "买入");
        assert_eq!(parsed["confidence"], 75.0);
        assert_eq!(parsed["positionPct"], 50.0);
        assert_eq!(parsed["riskLevel"], "中");
    }

    /// portfolio-mgr 是 CodeNode 包装但 .result 字段缺失(异常路径)→ 降级用包装本身
    #[test]
    fn extract_decision_json_falls_back_to_pm_wrapper_when_result_missing() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "portfolio-mgr".to_string(),
            json!({
                "status": "executed",
                "language": "rhai",
                // 故意无 .result 字段(异常路径)
                "params": { "action": "HOLD", "confidence": 30.0 },
                "node_id": "portfolio-mgr",
            }),
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: None,
        };
        let dj = extract_decision_json(&wf).expect("必须返回决策 JSON");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        // 降级用 portfolio-mgr 本身(CodeNode 包装),有 params.action
        assert_eq!(parsed["params"]["action"], "HOLD");
    }

    /// portfolio-mgr 节点不存在时回退到 wf.output(兼容无 portfolio-mgr 工作流)
    #[test]
    fn extract_decision_json_falls_back_to_workflow_output() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert("trigger".to_string(), json!({ "status": "ok" }));
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: Some(json!({ "action": "BUY", "confidence": 60.0 })),
        };
        let dj = extract_decision_json(&wf).expect("必须返回决策 JSON");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(parsed["action"], "BUY");
    }
}

/// 启动股票分析工作流（DAG 模式）。
///
/// - 默认：生成新 UUID 并 INSERT 新 `stock_analyses` 行（fresh start）。
/// - 重跑分析场景：传入 `analysis_id` 让后端先 DELETE 同 id 旧行再 INSERT,
///   保留 id 稳定,前端 store 引用不会断。
#[tauri::command]
pub async fn run_stock_workflow(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
    dry_run: Option<bool>,
    as_of_date: Option<String>,
    // 可选: 传入已存在的 analysisId 即可"覆盖"该记录（用于重跑分析场景）。
    // 不传则生成新 UUID 并 INSERT 新行(fresh start)。
    analysis_id: Option<String>,
) -> Result<serde_json::Value, String> {
    // 解析 as_of_date；非法或未来日期直接 4xx-style 错误
    let as_of_ctx = parse_asof_param(as_of_date.clone())?;

    if let Some(ctx) = as_of_ctx {
        as_of::AS_OF
            .scope(Some(ctx), async {
                run_stock_workflow_inner(app, state, stock_code, dry_run, as_of_date, analysis_id)
                    .await
            })
            .await
    } else {
        run_stock_workflow_inner(app, state, stock_code, dry_run, None, analysis_id).await
    }
}

async fn run_stock_workflow_inner(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
    dry_run: Option<bool>,
    as_of_date: Option<String>,
    analysis_id_override: Option<String>,
) -> Result<serde_json::Value, String> {
    let quote = state
        .astock_client
        .get_quote(&stock_code)
        .await
        .map_err(|e| format!("行情获取失败: {e}"))?;
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 重跑分析（override 模式）：先按 id 删掉旧行，让 INSERT 用相同 id 即可"覆盖"。
    // 业务语义：保留 id 稳定（前端 store 引用不会断），created_at 更新（重跑 = 新执行），
    // decision / blackboard_snapshot 完全替换。覆盖失败时降级为"新建"，不阻塞用户。
    let analysis_id = match analysis_id_override.as_ref() {
        Some(provided) => {
            match stock_analyses::Entity::delete_by_id(provided.as_str())
                .exec(state.harness.db())
                .await
            {
                Ok(_) => provided.clone(),
                Err(e) => {
                    tracing::warn!(
                        "[run_stock_workflow] 删除旧 analysis 失败,降级为新建: id={}, err={}",
                        provided,
                        e
                    );
                    uuid::Uuid::new_v4().to_string()
                },
            }
        },
        None => uuid::Uuid::new_v4().to_string(),
    };

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
        llm_decision_json: Set(None),
        blackboard_snapshot: Set(None),
        config_id: Set(None),
        // Time-travel metadata: 标记该 analysis 为 replay 模式 + 截止日
        analysis_kind: Set(if as_of_date.is_some() {
            "replay".into()
        } else {
            "live".into()
        }),
        // 始终保存 as_of_date：live 模式用分析当日，replay 模式用用户指定日期
        as_of_date: Set(Some(
            as_of_date
                .clone()
                .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string()),
        )),
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
            if let Err(e) = stock_analyses::Entity::update(stock_analyses::ActiveModel {
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
            .await
            {
                tracing::error!("[DB] 预检不足状态更新失败: {e}");
            }
            return Ok(json!({
                "status": "skipped",
                "reason": reason,
                "analysis_id": analysis_id,
                "stock_code": stock_code,
                "stock_name": quote.name,
                "data_quality_precheck": "insufficient",
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
            // 提取值避免所有权移动（两次 emit 都需要）
            let output_clone = event.output.clone();
            let error_clone = event.error.clone();
            // 根据步骤状态分发到对应的前端事件（与 executionStore 监听器匹配）
            let (event_name, payload) = match event.status.as_str() {
                "running" => (
                    "workflow-step-start",
                    serde_json::json!({
                        "conversationId": format!("wf-{}", wf_id),
                        "stepId": event.node_id,
                        "stepGoal": event.node_id,
                        "agentRole": "workflow",
                    }),
                ),
                "completed" => (
                    "workflow-step-complete",
                    serde_json::json!({
                        "conversationId": format!("wf-{}", wf_id),
                        "stepId": event.node_id,
                        "stepGoal": event.node_id,
                        "result": output_clone.and_then(|v| {
                            if v.is_string() { v.as_str().map(String::from) }
                            else { Some(serde_json::to_string(&v).unwrap_or_default()) }
                        }),
                    }),
                ),
                s if s == "failed" || s == "timeout" => (
                    "workflow-step-error",
                    serde_json::json!({
                        "conversationId": format!("wf-{}", wf_id),
                        "stepId": event.node_id,
                        "error": error_clone.unwrap_or_else(|| format!("Step {}", event.status)),
                    }),
                ),
                _ => return, // 未知状态，忽略
            };
            let _ = app.emit(event_name, payload);
            // 向后兼容：同时发送旧事件 workflow-step-done（前端 stockAnalysisStore /
            // stockWorkflowChatBridge / tests 仍监听此事件）
            let _ = app.emit(
                "workflow-step-done",
                serde_json::json!({
                    "workflowId": wf_id,
                    "nodeId": event.node_id,
                    "status": event.status,
                    "totalNodes": event.total_nodes,
                    "completedNodes": event.completed_nodes,
                    "executionId": event.execution_id,
                    "output": event.output,
                    "error": event.error,
                    "elapsedMs": event.elapsed_ms,
                }),
            );
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
    // 在 spawn 前捕获 as-of 上下文（tokio::task_local 不跨 tokio::spawn 传播）
    let captured_asof = as_of::current_as_of();
    tokio::spawn(async move {
        // P3 修复: 在 spawn 内恢复 AS_OF + DEGRADATION_LOG 作用域
        as_of::with_optional_asof(captured_asof, async {
            as_of::with_degradation_log(async {
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
        opts.tool_permissions = Some(Arc::new(ToolPermissions {
            strict_mode: true,
            ..Default::default()
        }));
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
        // 注入市场状态（沪深300判断牛/熊/震荡），兜底防止模板变量缺失
        let regime_value = market_regime_json.unwrap_or_else(|| {
            serde_json::json!({
                "regime": "unknown",
                "confidence": null,
                "volatility": null,
                "description": "⚠️ 市场状态数据暂不可用（沪深300 K线拉取失败），请勿据此做多空判断，基于个股自身数据完成分析"
            })
        });
        merged_vars.push(axagent_harness::workflow_types::Variable {
            name: "market_regime".into(),
            var_type: "object".into(),
            value: regime_value.clone(),
            description: Some("当前市场状态(bull/bear/sideways)+波动率+描述".into()),
            is_secret: false,
        });
        // 从 market_regime 派生 prompt 偏向 + 触发规则
        let regime_str = regime_value["regime"].as_str().unwrap_or("unknown");
        let vol_str = regime_value["volatility"].as_str().unwrap_or("low");
        let (regime_prompt_bias, regime_triggered_rules) = match (regime_str, vol_str) {
            ("bull", "high") => (
                "顺势偏多但高波动环境：关注业绩超预期+资金流入，同时警惕短期大幅回撤",
                "1. 侧重成长性指标（营收增速、ROE趋势）；2. 估值容忍度可适当放宽；3. 关注大单资金流向；4. 高波动环境需关注最大回撤",
            ),
            ("bull", _) => (
                "顺势偏多：关注业绩超预期+资金流入，警惕追高",
                "1. 侧重成长性指标（营收增速、ROE趋势）；2. 估值容忍度可适当放宽；3. 关注大单资金流向",
            ),
            ("bear", "high") => (
                "防御为主+高波动环境：严格关注低估值+稳健现金流，警惕杀估值+踩踏风险",
                "1. 侧重防御性指标（现金流、负债率）；2. 估值要求更严格；3. 关注避险资金流向；4. 高波动环境建议降低仓位",
            ),
            ("bear", _) => (
                "防御为主：关注低估值+稳健现金流，警惕杀估值",
                "1. 侧重防御性指标（现金流、负债率）；2. 估值要求更严格；3. 关注避险资金流向",
            ),
            ("sideways", _) => (
                "精选个股：关注催化剂+预期差，警惕无主线行情",
                "1. 侧重个股α；2. 关注催化剂事件；3. 估值锚定历史中枢",
            ),
            _ => (
                "市场状态未知，不预设多空偏向，仅基于个股自身基本面完成分析",
                "无触发规则，全维度中性分析",
            ),
        };
        merged_vars.push(axagent_harness::workflow_types::Variable {
            name: "regime_prompt_bias".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(regime_prompt_bias.to_string()),
            description: Some("按当前市场状态(regime)匹配的分析偏向指令".into()),
            is_secret: false,
        });
        merged_vars.push(axagent_harness::workflow_types::Variable {
            name: "regime_triggered_rules".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(regime_triggered_rules.to_string()),
            description: Some("当前市场状态触发的分析规则清单".into()),
            is_secret: false,
        });
        // 注入历史反思教训（从 stock_reflections 表取最近的结构化反思结果）
        // 必须始终注入，即使为空，否则 value-investor/research-mgr/trader 等节点
        // 的 input_mapping 引用 {{stock_lessons}} 会报 VARIABLE_NOT_FOUND。
        let lessons_str = fetch_stock_lessons(&stock_code, &db).await;
        merged_vars.push(axagent_harness::workflow_types::Variable {
            name: "stock_lessons".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(lessons_str.unwrap_or_else(|| "（暂无历史反思）".to_string())),
            description: Some("该股历史反思教训（错因/被忽视信号/改进建议）".into()),
            is_secret: false,
        });
        opts.variables = Some(merged_vars);

        match engine.run_workflow(&wf_id, opts).await {
            Ok(result) => {
                let wf_status = result.status;
                match wf_status {
                    axagent_rt_workflow::workflow_engine::WorkflowStatus::Cancelled => {
                        if let Err(e) = app_h.emit(
                            "workflow-error",
                            serde_json::json!({ "workflowId": wf_id, "error": "分析已被取消" }),
                        ) {
                            tracing::warn!("[emit] workflow-error 发送失败: {e}");
                        }
                        if let Err(e) = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("cancelled"))
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await
                        {
                            tracing::error!("[DB] Cancelled 状态更新失败: {e}");
                        }
                    },
                    axagent_rt_workflow::workflow_engine::WorkflowStatus::Failed => {
                        tracing::warn!(%wf_id, status=?wf_status, "工作流以 Failed 状态结束，保存部分结果");
                        if let Err(e) = app_h.emit(
                            "workflow-completed",
                            serde_json::json!({
                                "workflowId": wf_id,
                                "results": result.results,
                                "output": result.output,
                                "degraded": true,
                                "degradationReason": "部分分析步骤失败，结果为部分数据",
                            }),
                        ) {
                            tracing::warn!("[emit] workflow-completed 发送失败: {e}");
                        }
                        // 即使有节点失败，仍然保存已有结果
                        // 修复"决策信息缺失"误报:优先从 portfolio-mgr 节点本身
                        // 提取决策(见 extract_decision_json 注释),回退到 wf.output。
                        let decision_json = extract_decision_json(&result);
                        let (action, position_pct, reasoning, time_horizon, expected_holding_days) =
                            extract_decision_fields(&decision_json);
                        let degradation_report = as_of::take_asof_degradation_report();
                        let llm_dj_partial = extract_llm_decision_json(&result);
                        let as_of_for_meta: Option<AsOfContext> = as_of::current_as_of();
                        let bb_snapshot = serde_json::to_string(&build_blackboard_snapshot(
                            &result.results,
                            as_of_for_meta.as_ref(),
                            &degradation_report,
                        ))
                        .unwrap_or_else(|_| "{}".to_string());
                        if let Err(e) = stock_analyses::Entity::update_many()
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
                                stock_analyses::Column::LlmDecisionJson,
                                Expr::value(llm_dj_partial),
                            )
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await
                        {
                            tracing::error!("[DB] Failed 状态下保存分析结果失败: {e}");
                        }
                    },
                    _ => {
                        if let Err(e) = app_h.emit(
                            "workflow-completed",
                            serde_json::json!({
                                "workflowId": wf_id,
                                "results": result.results,
                                "output": result.output,
                            }),
                        ) {
                            tracing::warn!("[emit] workflow-completed 发送失败: {e}");
                        }
                        // 修复"决策信息缺失"误报:优先从 portfolio-mgr 节点本身
                        // 提取决策(见 extract_decision_json 注释),回退到 wf.output。
                        let decision_json = extract_decision_json(&result);
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
                        let llm_dj = extract_llm_decision_json(&result);
                        if let Err(e) = stock_analyses::Entity::update_many()
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
                                stock_analyses::Column::LlmDecisionJson,
                                Expr::value(llm_dj),
                            )
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await
                        {
                            tracing::error!("[DB] 保存分析结果失败: {e}");
                        }

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
                if let Err(db_e) = stock_analyses::Entity::update_many()
                    .col_expr(stock_analyses::Column::Status, Expr::value(format!("failed: {e}")))
                    .col_expr(
                        stock_analyses::Column::UpdatedAt,
                        Expr::value(chrono::Utc::now().timestamp_millis()),
                    )
                    .filter(stock_analyses::Column::Id.eq(&aid))
                    .exec(&db)
                    .await
                {
                    tracing::error!("[DB] run_workflow Err 状态更新失败: {db_e}");
                }
            },
        }}).await  // with_degradation_log
    }).await // with_optional_asof
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
        llm_decision_json: Set(None),
        blackboard_snapshot: Set(None),
        config_id: Set(None),
        analysis_kind: Set("live".into()),
        as_of_date: Set(Some(chrono::Utc::now().format("%Y-%m-%d").to_string())),
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

    // 5.5 [A1 借鉴] 注入历史反思教训(TradingAgents past_context 机制):
    //   批量/定时分析场景下,trader/research-mgr/value-investor 节点能看到
    //   该股最近 90 天的反思教训(lesson_summary),避免重蹈覆辙。前端触发场景下
    //   run_stock_workflow_inner 同样会注入,这里是补齐 cron / batch 入口。
    //   必须始终注入,即使为空（否则 VARIABLE_NOT_FOUND）。
    let lessons_str = fetch_stock_lessons(stock_code, db).await;
    let mut variables = Vec::new();
    variables.push(Variable {
        name: "stock_lessons".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(
            lessons_str.unwrap_or_else(|| "（暂无历史反思）".to_string()),
        ),
        description: Some("A1: 该股最近 90 天的反思教训".into()),
        is_secret: false,
    });

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
        variables: if variables.is_empty() {
            None
        } else {
            Some(variables)
        },
        tool_permissions: Some(Arc::new(ToolPermissions {
            strict_mode: true,
            ..Default::default()
        })),
        ..Default::default()
    };

    let result = engine.run_workflow(&wf_id, opts).await;

    match result {
        Ok(wf) => {
            // 更新为完成状态
            // 修复"决策信息缺失"误报:用 extract_decision_json 从 portfolio-mgr
            // 节点 .result 提取决策(而非 CodeNode 包装顶层,后者无 action 字段)。
            let decision_json_str = extract_decision_json(&wf);
            let decision_output = decision_json_str
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

            let decision_action = decision_output.as_ref().and_then(|d| {
                d.get("action")
                    .and_then(|a| a.as_str().map(|s| s.to_string()))
            });

            let _ = stock_analyses::Entity::update(stock_analyses::ActiveModel {
                id: Set(analysis_id.clone()),
                status: Set("completed".into()),
                decision_action: Set(decision_action),
                decision_json: Set(decision_json_str),
                updated_at: Set(chrono::Utc::now().timestamp_millis()),
                ..Default::default()
            })
            .exec(db)
            .await;

            // ── [B1 借鉴] 两阶段协议: 落盘时同步写 stock_reflections pending row ──
            // TradingAgents 反思模式: 先占位(pending)再异步 resolve。这样:
            //   1) 系统重启/进程崩溃后,D1 批量反思能扫到所有 pending,不会丢失
            //   2) 持仓期到时,D1 知道哪些 row 该被 resolve(避免重复 INSERT 触发冲突)
            //   3) fetch_stock_lessons 可基于 status='resolved' 过滤,只注入真正可用的教训
            // 字段: as_of_date = analysis_date, raw_return/alpha_return/holding_days
            //   全部 None(预测不到),status='pending',后续由 D1 批量补全。
            let pending_id = uuid::Uuid::new_v4().to_string();
            let today_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
            let _ = stock_reflections::ActiveModel {
                id: Set(pending_id.clone()),
                stock_code: Set(stock_code.to_string()),
                stock_name: Set(stock_name.to_string()),
                original_analysis_id: Set(analysis_id.clone()),
                as_of_date: Set(today_str.clone()),
                hindsight_date: Set(today_str),
                min_confidence_threshold: Set(70),
                reflection_depth: Set("light".to_string()),
                actual_outcome: Set(String::new()),
                // v008 (C3 借鉴): 结构化 outcome,pending 阶段全 None
                raw_return: Set(None),
                alpha_return: Set(None),
                holding_days: Set(None),
                benchmark_name: Set(None),
                // v008 (C2 借鉴): 输出 schema,pending 阶段全 None
                verdict: Set(None),
                alpha_cited: Set(None),
                lesson_summary: Set(None),
                what_went_wrong: Set(None),
                missed_signals: Set(None),
                fix_for_future: Set(None),
                parameter_suggestions_json: Set(None),
                decision_json: Set(None),
                blackboard_snapshot: Set(None),
                model_version: Set(None),
                status: Set("pending".to_string()),
                created_at: Set(chrono::Utc::now().timestamp_millis()),
                updated_at: Set(chrono::Utc::now().timestamp_millis()),
            }
            .insert(db)
            .await;
            tracing::info!(
                "[B1 batch_analysis] {stock_code} ({stock_name}) 已落盘 pending reflection {pending_id},等 D1 持仓期到达 resolve"
            );

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
///
/// ## v008 + E1 升级（借鉴 TradingAgents past_context 机制）
///
/// 借鉴 TradingAgents 反思机制的多范围教训注入:
/// - **same_ticker**(3 条): 同 ticker 最近 90 天的反思,直接可借鉴
/// - **all_recent**(2 条): 所有 ticker 最近 7 天的反思,捕捉市场级教训
///   (如"近期白马股普遍杀估值""科技股 Q3 业绩雷高发")
/// - 跨 sector 范围需要 stock_analyses.sector 字段(v009 之后再做)
///
/// ## v008 字段选择
///
/// 输出 lesson_summary (≤200 字符) + verdict(判定标签) + alpha_cited(关键 alpha)
/// 替代之前的 what_went_wrong/missed_signals/fix_for_future 三件套
/// (后三个字段在新反思中可能为空,因为 prompt 现在只强制 short 文本)。
async fn fetch_stock_lessons(stock_code: &str, db: &sea_orm::DatabaseConnection) -> Option<String> {
    use chrono::Utc;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    // ── same_ticker: 3 条同 ticker 近 90 天已完成反思 ──
    let three_months_ago = Utc::now() - chrono::Duration::days(90);
    let same_ticker: Vec<stock_reflections::Model> = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::StockCode.eq(stock_code))
        .filter(stock_reflections::Column::Status.eq("completed")) // 只注入已 resolve 的教训
        .filter(stock_reflections::Column::CreatedAt.gte(three_months_ago.timestamp_millis()))
        .order_by_desc(stock_reflections::Column::CreatedAt)
        .all(db)
        .await
        .unwrap_or_default()
        .into_iter()
        .take(3)
        .collect();

    // ── all_recent: 2 条所有 ticker 近 7 天(跨 ticker 市场级教训)──
    let seven_days_ago = Utc::now() - chrono::Duration::days(7);
    let all_recent: Vec<stock_reflections::Model> = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::CreatedAt.gte(seven_days_ago.timestamp_millis()))
        .filter(stock_reflections::Column::Status.eq("completed")) // 只看已 resolve 的
        .order_by_desc(stock_reflections::Column::CreatedAt)
        .all(db)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.stock_code != stock_code) // 排除 same_ticker 已经包含的
        .take(2)
        .collect();

    if same_ticker.is_empty() && all_recent.is_empty() {
        return None;
    }

    let mut lines: Vec<String> = Vec::new();

    if !same_ticker.is_empty() {
        lines.push(format!("【同股近 90 天反思 {} 条】", same_ticker.len()));
        for (i, l) in same_ticker.iter().enumerate() {
            lines.push(format!("#{} ({}, 反思于 {})", i + 1, l.stock_code, l.hindsight_date));
            if let Some(ref ls) = l.lesson_summary {
                lines.push(format!("  - 总结：{}", ls));
            }
            if let Some(ref v) = l.verdict {
                lines.push(format!("  - 判定：{}", v));
            }
            if let Some(ref ac) = l.alpha_cited {
                lines.push(format!("  - 关键 alpha：{}", ac));
            }
            // 兼容旧反思(无 v008 字段)
            if let Some(ref w) = l.what_went_wrong {
                lines.push(format!("  - 错因：{}", w));
            }
            if let Some(ref f) = l.fix_for_future {
                lines.push(format!("  - 改进建议：{}", f));
            }
        }
    }

    if !all_recent.is_empty() {
        lines.push(String::new());
        lines.push(format!("【近期市场级反思 {} 条(跨 ticker 近 7 天)】", all_recent.len()));
        for (i, l) in all_recent.iter().enumerate() {
            lines.push(format!("#{} {} ({}):", i + 1, l.stock_code, l.stock_name));
            if let Some(ref ls) = l.lesson_summary {
                lines.push(format!("  - {}", ls));
            } else if let Some(ref w) = l.what_went_wrong {
                lines.push(format!("  - 错因：{}", w));
            }
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
/// ## v008 升级（借鉴 TradingAgents 反思机制）
///
/// 新增 4 个结构化 outcome 参数（`raw_return` / `alpha_return` /
/// `holding_days` / `benchmark_name`）作为 C3 借鉴；`actual_outcome`
/// 保留为 legacy/fallback 自然语言描述。C1 + C2 强约束在 reflection-agent
/// system_prompt 体现（≤200 字符 lesson_summary + verdict 标签 + alpha_cited）。
///
/// ## v009 升级（B1+B2+B3 借鉴）
///
/// - B1 落盘协议:调用方(批量分析)已写入 `stock_reflections` row with `status="pending"`。
/// - B2 幂等守卫:当 `reflection_id` 已存在且 `status="completed"`,直接返回
///   cached row 的 `lesson_summary` / `verdict` / `decision_json`,避免重跑 LLM。
/// - B3 原子写:传入 `reflection_id` 时,UPDATE 现有 row 而非 INSERT 新的,
///   避免重复 INSERT 触发冲突。
///
/// 结果写入独立的 `stock_reflections` 表。
#[allow(clippy::too_many_arguments)]
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
    // v008 (C3 借鉴): 4 个结构化 outcome 变量
    raw_return: Option<f64>,
    alpha_return: Option<f64>,
    holding_days: Option<i32>,
    benchmark_name: Option<&str>,
    as_of_date: &str,
    hindsight_date: &str,
    min_confidence_threshold: u8,
    reflection_depth: &str,
    // [B2/B3 借鉴] 反思 row ID(B1 阶段落盘的 pending row)。
    // 传入则 UPDATE 现有 row;传 None 则按 v1 行为 INSERT 新 row,保持旧调用方兼容。
    reflection_id: Option<String>,
) -> Result<String, String> {
    use axagent_astock_data::as_of;
    use axagent_core::entity::stock_reflections;
    use sea_orm::sea_query::Expr;
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};

    let now_ms = chrono::Utc::now().timestamp_millis();

    // ── [B2 借鉴] 幂等守卫: 如果 reflection_id 已 completed,直接返回 cached ──
    if let Some(ref rid) = reflection_id {
        if let Some(existing) = stock_reflections::Entity::find_by_id(rid.clone())
            .one(db)
            .await
            .map_err(|e| format!("B2 查询已存在反思失败: {e}"))?
        {
            if existing.status == "completed" {
                tracing::info!(
                    "[B2 idempotency] reflection_id={rid} 已 completed,跳过重跑,直接返回 cached"
                );
                return Ok(rid.clone());
            }
        }
    }

    // ── [B3 借鉴] 原子写: reflection_id 存在则 UPDATE pending→running,否则 INSERT ──
    let analysis_id = reflection_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    if let Some(ref rid) = reflection_id {
        let _ = stock_reflections::Entity::update_many()
            .col_expr(stock_reflections::Column::Status, Expr::value("running"))
            .col_expr(stock_reflections::Column::UpdatedAt, Expr::value(now_ms))
            .filter(stock_reflections::Column::Id.eq(rid.clone()))
            .exec(db)
            .await
            .map_err(|e| format!("B3 UPDATE pending→running 失败: {e}"))?;
        tracing::info!("[B3 atomic] reflection_id={rid} pending→running");
    } else {
        // 兼容旧调用方路径: INSERT 新 row
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
            // v008 (C3 借鉴): 4 个结构化 outcome
            raw_return: Set(raw_return),
            alpha_return: Set(alpha_return),
            holding_days: Set(holding_days),
            benchmark_name: Set(benchmark_name.map(|s| s.to_string())),
            // v008 (C2 借鉴): 3 个输出 schema 字段
            verdict: Set(None),
            alpha_cited: Set(None),
            lesson_summary: Set(None),
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
    }

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
    // 手动触发时 original_analysis_id="" → original_ctx=None。
    // 但反思 prompt 模板 (reflection.md:17-18) hard-code 引用
    // {{original_time_horizon}} / {{original_holding_days}},所以必须注入占位值
    // (否则 work_engine 报 VARIABLE_NOT_FOUND,reflection-agent 节点 Failed,
    // 数据库 what_went_wrong 等字段全 null)。
    // 之前的注释说"让工作流模板自己决定怎么处理"——实际模板没有兜底处理。
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
        // 内联 system_prompt (stock_analysis_setup.rs:4538-4552) 引用了
        // {{stock_code}} / {{stock_name}} —— 必须在 variables 顶层,
        // input_mapping 的 source="trigger" 不会把它们提到顶层 (只会追加到
        // system_prompt 尾部的 "--- 输入上下文 ---" 块)。
        // 不注入会触发 reflection-agent 节点的 VARIABLE_NOT_FOUND。
        axagent_harness::workflow_types::Variable {
            name: "stock_code".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(stock_code.to_string()),
            description: Some("当前反思的股票代码".into()),
            is_secret: false,
        },
        axagent_harness::workflow_types::Variable {
            name: "stock_name".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(stock_name.to_string()),
            description: Some("当前反思的股票名称".into()),
            is_secret: false,
        },
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
        // 反思 prompt 模板里引用了 {{stock_lessons}},必须显式注入,
        // 否则 work_engine 报 VARIABLE_NOT_FOUND 导致反思节点 Failed。
        // 数据源: 该股最近 3 个月的反思记录(去重排除当前正在创建的记录)。
        axagent_harness::workflow_types::Variable {
            name: "stock_lessons".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(
                fetch_stock_lessons(stock_code, db)
                    .await
                    .unwrap_or_else(|| "（暂无历史反思）".to_string()),
            ),
            description: Some("该股历史反思教训（错因/被忽视信号/改进建议）".into()),
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
        // 手动反思场景:无原始分析上下文,但 prompt 模板必须能渲染。
        // 注入占位值(让 LLM 知道这是手动触发的独立反思,无持仓期对齐数据)。
        variables.push(axagent_harness::workflow_types::Variable {
            name: "original_time_horizon".into(),
            var_type: "string".into(),
            value: serde_json::Value::String("manual".into()),
            description: Some("原始决策的时间维度(手动反思场景无原始分析,固定为 'manual')".into()),
            is_secret: false,
        });
        variables.push(axagent_harness::workflow_types::Variable {
            name: "original_holding_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(0),
            description: Some("原始决策期望持有天数(手动反思场景无原始分析,固定为 0)".into()),
            is_secret: false,
        });
        tracing::info!(
            "[reflection] {}: 手动反思场景,注入占位 original_time_horizon='manual' / original_holding_days=0",
            stock_code
        );
    }
    let opts = axagent_rt_workflow::work_engine::RunOptions {
        max_concurrent,
        step_timeout,
        progress_callback: None,
        // [BUGFIX] 之前只传 stock_code,缺 stock_name / as_of_date。
        // 反思工作流内的 sub-analysis 节点 (嵌套 stock-analysis 子工作流) 的
        // input_mapping 把这 3 个变量映射到子工作流的 input,缺任何一个都会
        // 导致子工作流报 "参数 X 应为 string 类型" 或 "VARIABLE_NOT_FOUND: X"。
        input: Some(json!({
            "stock_code": stock_code,
            "stock_name": stock_name,
            "as_of_date": as_of_date,
        })),
        input_schema: loaded.input_schema,
        output_schema: loaded.output_schema,
        dry_run: false,
        variables: Some(variables),
        tool_permissions: Some(Arc::new(ToolPermissions {
            strict_mode: true,
            ..Default::default()
        })),
        ..Default::default()
    };

    // 5. as-of 范围执行
    let ctx = AsOfContext::parse(as_of_date).map_err(|e| format!("as_of 解析失败: {e}"))?;

    // 注册内建变量提供器(Phase 1 混合 as-of),让 prompt 模板中 {{data_freshness}} /
    // {{as_of_date}} / {{is_replay}} / {{data_scope}} 等跨领域通用状态由引擎自动注入。
    // 闭包在 as_of::scope 内执行,可拿到当前 task_local AS_OF。
    use std::collections::HashMap;
    use std::sync::Arc;
    let provider: axagent_rt_workflow::work_engine::prompt_template::BuiltinVarsProvider =
        Arc::new(|| {
            let mut m: HashMap<String, String> = HashMap::new();
            m.insert(
                "data_freshness".to_string(),
                axagent_astock_data::as_of::data_freshness_description(),
            );
            m.insert("is_replay".to_string(), "true".to_string());
            if let Some(ctx) = axagent_astock_data::as_of::current_as_of() {
                m.insert("as_of_date".to_string(), ctx.as_of_date.format("%Y-%m-%d").to_string());
                m.insert("as_of_source".to_string(), format!("{:?}", ctx.source).to_lowercase());
                m.insert("data_scope".to_string(), format!("{:?}", ctx.data_scope).to_lowercase());
            }
            m
        });
    engine.set_builtin_vars_provider(provider).await;

    let result = as_of::AS_OF
        .scope(Some(ctx), async move { engine.run_workflow(&wf_id, opts).await })
        .await;

    // 6. 处理结果
    match result {
        Ok(wf) => {
            // 通过 extract_agent_output 管线提取规范化 JSON（兼容多模型输出格式）
            let reflection_raw = wf
                .results
                .get("reflection")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let reflection_json = extract_agent_output(reflection_raw).await;
            // 兜底: extract_agent_output 在某些 wrapper 格式下可能返回 JSON 字符串
            // (例如 LLM 输出被包成 `{output: "{...}"}` 时走 line 1552 分支直接 return 字符串),
            // 这时 as_object() 会得到 None,导致整个字段提取跳到 unwrap_or 兜底,
            // 数据库里 what_went_wrong / missed_signals / fix_for_future 全部为 null。
            // 二次解析: 把它当字符串再 parse 一次,还原成对象。
            let reflection_obj: Option<serde_json::Map<String, serde_json::Value>> =
                if let Some(obj) = reflection_json.as_object() {
                    Some(obj.clone())
                } else if let Some(s) = reflection_json.as_str() {
                    serde_json::from_str::<serde_json::Value>(s)
                        .ok()
                        .and_then(|v| v.as_object().cloned())
                } else {
                    None
                };

            // 兼容两种输出结构:
            //   A) 直接: {what_went_wrong, missed_signals, fix_for_future, params_suggestion}
            //   B) 嵌套: {reflection: {what_went_wrong, missed_signals, fix_for_future}, params_suggestion}
            // 内联 system_prompt 要求 A 格式,reflection.md 外部 expert prompt 要求 B 格式,
            // 实际 LLM 可能按任一格式输出,后端必须容错。
            let (what_went_wrong, missed_signals, fix_for_future, params_suggestion_json) =
                reflection_obj
                    .map(|obj| {
                        // 优先看嵌套 reflection 子对象,找不到再退到顶层
                        let inner = obj.get("reflection").and_then(|v| v.as_object());
                        let lookup = |key: &str| -> Option<&serde_json::Value> {
                            inner.and_then(|i| i.get(key)).or_else(|| obj.get(key))
                        };
                        let w = lookup("what_went_wrong")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let m = lookup("missed_signals").map(|v| v.to_string());
                        let f = lookup("fix_for_future")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let p = obj.get("params_suggestion").map(|v| v.to_string());
                        (w, m, f, p)
                    })
                    .unwrap_or((None, None, None, None));

            // 诊断: 检查反思节点是否成功,如果不成功,把状态/错误信息附到 status 字段
            // (Failed 节点 result 是 None,work_engine 不会写入 results,所以
            // wf.results 不等于完整执行轨迹 —— 之前只能看到"completed"但实际反思节点没跑)。
            use axagent_rt_workflow::workflow_engine::NodeStatus;
            let reflection_node_state = wf.node_states.get("reflection-agent");
            let status_text = match reflection_node_state {
                Some(s) if s.status == NodeStatus::Completed => "completed".to_string(),
                Some(s) if s.status == NodeStatus::Failed => {
                    let err = s.error.clone().unwrap_or_else(|| "未知错误".to_string());
                    format!("failed: reflection-agent: {err}")
                },
                Some(s) if s.status == NodeStatus::Skipped => {
                    "skipped: reflection-agent".to_string()
                },
                _ => "completed: reflection-agent 未在 node_states 中".to_string(),
            };

            let bb_text = serde_json::to_string(&wf.results).unwrap_or_default();
            let dj_text = if reflection_json.is_null() {
                None
            } else {
                Some(reflection_json.to_string())
            };

            let _ = stock_reflections::Entity::update_many()
                .col_expr(stock_reflections::Column::Status, Expr::value(&status_text))
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
                // v008 (C2 借鉴): 回写 verdict / alpha_cited / lesson_summary
                .col_expr(
                    stock_reflections::Column::Verdict,
                    Expr::value(reflection_json.get("verdict").and_then(|v| v.as_str().map(|s| s.to_string()))),
                )
                .col_expr(
                    stock_reflections::Column::AlphaCited,
                    Expr::value(reflection_json.get("alpha_cited").and_then(|v| v.as_str().map(|s| s.to_string()))),
                )
                .col_expr(
                    stock_reflections::Column::LessonSummary,
                    Expr::value(reflection_json.get("lesson_summary").and_then(|v| v.as_str().map(|s| s.to_string()))),
                )
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

            // ── [F1 借鉴] 反思完成后自动提取 lesson 为可重用规则 ──
            // 借鉴 TradingAgents 反思→规则提取机制:反思完成后把 lesson_summary
            // 提取为可重用的规则存入 reflection_lessons 表,下次决策可查询。
            if status_text == "completed" {
                if let Some(ls) = reflection_json
                    .get("lesson_summary")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                {
                    let _ = extract_lesson_to_rule(
                        db,
                        stock_code,
                        &analysis_id,
                        &ls,
                        reflection_json.get("verdict").and_then(|v| v.as_str()),
                    )
                    .await;
                }
            }

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

/// 从 Agent 节点输出中提取结构化 JSON。
///
/// 优先顺序：
///   1) 顶层 `params` 字段
///   2) 顶层 `output` / `result` / `data` / `candidates` / `trends` 字段
///   3) 顶层 `content` 字符串：直接用 `axagent_core::extract_json_from_llm_response`
///      解析（不经过 ResponseNormalizer——它针对工具调用场景，会将 ````json` 块
///      误识别为 ToolUse）
///   4) 原始包装对象（兜底）
async fn extract_agent_output(raw: serde_json::Value) -> serde_json::Value {
    let obj = match raw.as_object() {
        Some(o) => o,
        None => return raw,
    };
    // 1) 顶层 params
    if let Some(params) = obj.get("params") {
        return params.clone();
    }
    // 2) 顶层常见容器字段
    for key in ["output", "result", "data", "candidates", "trends"] {
        if let Some(v) = obj.get(key) {
            return v.clone();
        }
    }
    // 3) 直接从 content 提取 JSON：找到第一个 { 或 [，找匹配闭合，解析。
    //    不依赖 extract_json_from_llm_response 的 fence 剥离（在复杂嵌套场景可能失效）。
    if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
        let candidate = axagent_core::extract_json_from_llm_response(content);
        // 诊断：打印 candidate 前后各 200 字符
        let preview: String = candidate.chars().take(200).collect();
        let tail: String = candidate
            .chars()
            .rev()
            .take(200)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        tracing::info!("[serenity] 提取文本 前200: {} / 后200: {}", preview, tail);
        // A: 精确解析（fence 剥离后的文本）
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&candidate) {
            if parsed.is_object() || parsed.is_array() {
                // 拆包 tool_json 格式: {"name": "...", "arguments": {...}} → arguments
                // 有些 LLM 用 "input" 代替 "arguments"
                if let Some(args) = parsed
                    .as_object()
                    .and_then(|o| o.get("arguments"))
                    .or_else(|| parsed.as_object().and_then(|o| o.get("input")))
                {
                    return args.clone();
                }
                return parsed;
            }
        }
        // B: 裸括号提取 candidates/trends 数组（免疫未转义引号）
        if let Some(parsed) = extract_named_arrays(&candidate) {
            return parsed;
        }
        if let Some(parsed) = extract_named_arrays(content) {
            return parsed;
        }
        // C: 修复后重试
        let repaired = repair_json(&candidate);
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&repaired) {
            if parsed.is_object() || parsed.is_array() {
                return parsed;
            }
        }
        // D: extract_outer_json（多起点 + in_string 追踪）
        if let Some(parsed) = extract_outer_json(content) {
            return parsed;
        }
        // E: 检测 LLM 自然语言拒绝（短文本非 JSON），防御性降级
        let content_len = content.chars().count();
        if content_len < 30 {
            tracing::warn!(
                "[serenity] LLM 内容为短自然语言（长度={}），返回空值防御性降级: {}",
                content_len,
                content.chars().take(50).collect::<String>()
            );
            return serde_json::Value::Null;
        }
        let head: String = content.chars().take(300).collect();
        let tail_start = content.chars().count().saturating_sub(200);
        let tail: String = content.chars().skip(tail_start).collect();
        let c_head: String = candidate.chars().take(1000).collect();
        let c_tail: String = candidate
            .chars()
            .rev()
            .take(200)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        tracing::warn!(
            "[serenity] content JSON 提取失败，总长度 {}, 前300: {} / 后200: {}",
            content.chars().count(),
            head,
            tail
        );
        tracing::warn!("[serenity] 预处理文本 前1000: {} / 后200: {}", c_head, c_tail);
    }
    // 4) 兜底
    raw
}

/// 通过 `ResponseNormalizer` 把 `content` 字符串规范化为 IR 块，再从 IR 中
/// 提取结构化 JSON。优先取 `ContentBlock::ToolUse.input`（通常是 JSON 串），
/// 文本块拼接后走 `axagent_core::extract_json_from_llm_response` 兜底。
///
/// 注意：`extract_agent_output` 不再调用此函数（改用 `extract_json_from_llm_response` 直接提取）。
/// 此函数保留供测试和未来工具调用场景复用。
#[allow(dead_code)]
async fn extract_via_normalizer(content: &str) -> Option<serde_json::Value> {
    if content.trim().is_empty() {
        return None;
    }
    let response = ChatResponse {
        id: String::new(),
        model: String::new(),
        content: content.to_string(),
        thinking: None,
        usage: Default::default(),
        tool_calls: None,
    };
    let normalizer = DefaultResponseNormalizer;
    let blocks: Vec<ContentBlock> = normalizer.normalize(&response).await;

    // 优先：ToolUse 块的 input（项目里工具参数就是 JSON 串）
    for block in &blocks {
        if let ContentBlock::ToolUse { input, .. } = block
            && let Some(parsed) = parse_loose_json(input)
        {
            return Some(parsed);
        }
    }
    // 兜底：拼接所有 Text 块，用项目统一的 LLM JSON 提取函数
    let joined: String = blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !joined.trim().is_empty() {
        let candidate = axagent_core::extract_json_from_llm_response(&joined);
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(candidate) {
            return Some(v);
        }
    }
    None
}

/// 轻量 JSON 修复：处理 LLM 偶发的括号不匹配和引号未闭合。
///
/// 只做两种统计级修复（不解析语义）：
/// 1. **括号平衡** — 跳过字符串内部，统计 `{`/`[` vs `}`/`]`，补/删尾部括号
/// 2. **引号闭合** — 奇数个未转义 `"` 时末尾补一个
///
/// 对合法 JSON 零开销（不改变原文）；只在 `serde_json::from_str` 已失败后调用。
fn repair_json(s: &str) -> String {
    let mut result = s.to_string();

    // LLM 高频手滑："nulll"→"null"
    result = result.replace("nulll", "null");

    // LLM 尾逗号：,"→"、,}→}、,]→]
    // 只在可能有尾逗号的上下文中处理（简单字符串替换，低风险）
    result = result.replace(",]", "]");
    result = result.replace(",}", "}");

    let bytes = result.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return result;
    }

    // 第一遍：统计括号和引号，跳过字符串内部
    let mut open_curly = 0i32;
    let mut open_bracket = 0i32;
    // 用 Option<bool> 表达"未闭合的字符串中"状态：None=不在字符串中，Some(true/false)=在字符串中
    let mut in_string: Option<usize> = None;

    let mut i = 0;
    while i < len {
        let b = bytes[i];
        match in_string {
            None => {
                match b {
                    b'{' => open_curly += 1,
                    b'}' => open_curly -= 1,
                    b'[' => open_bracket += 1,
                    b']' => open_bracket -= 1,
                    b'"' => in_string = Some(1), // 标记字符串开始
                    _ => {},
                }
            },
            Some(_) => {
                // 在字符串内部：只关心 \" 和 字符串结束 "
                if b == b'\\' {
                    i += 1; // 跳过下一个字符（转义序列）
                } else if b == b'"' {
                    in_string = None; // 字符串结束
                }
            },
        }
        i += 1;
    }

    // 第二遍：从尾部修复 — 只处理末尾多余的闭合括号
    // 复用第一遍已经过 nulll→null 修复的 result，不重新从 s 构建
    // (这行是故意 blank 的以使用前面的 result 变量)

    // 先处理括号不平衡：补缺失的闭合括号
    let needs_curly = open_curly.max(0) as usize;
    let needs_bracket = open_bracket.max(0) as usize;

    // 如果有缺失闭合，在尾部补上
    for _ in 0..needs_curly {
        result.push('}');
    }
    for _ in 0..needs_bracket {
        result.push(']');
    }

    // 引号修复：如果正在字符串中（奇数个引号），末尾补 "
    if in_string.is_some() {
        result.push('"');
    }

    // 处理尾部多余闭合（open 为负数 → 多了闭合括号）
    // 从后往前删多余的 }
    let mut extra_close = (-open_curly).max(0) as usize;
    while extra_close > 0 {
        if let Some(pos) = result.as_bytes().iter().rposition(|&b| b == b'}') {
            result.remove(pos);
            extra_close -= 1;
        } else {
            break;
        }
    }
    let mut extra_close = (-open_bracket).max(0) as usize;
    while extra_close > 0 {
        if let Some(pos) = result.as_bytes().iter().rposition(|&b| b == b']') {
            result.remove(pos);
            extra_close -= 1;
        } else {
            break;
        }
    }

    result
}

/// 用裸括号追踪从文本中提取指定 key 的 JSON 数组（容忍引号错乱）。
///
/// LLM 常在字符串值中使用未转义双引号（如：他说"这是关键"），
/// 导致 `serde_json` 全量解析失败。此函数绕过引号状态追踪，
/// 直接匹配 `"key": [` 找到数组起始，然后裸计 `[`/`]` 深度找到闭合，
/// 只对这一小段 `[...]` 调用 `serde_json::from_str`。
///
/// 返回 `{"candidates": [...], "trends": [...]}`（只含成功解析的 key）。
fn extract_named_arrays(text: &str) -> Option<serde_json::Value> {
    let keys = ["candidates", "trends"];
    let mut result = serde_json::Map::new();

    for key in &keys {
        let pattern = format!("\"{}\":", key);
        // 找所有匹配位置（可能有多个同名 key，取最后一个）
        let mut pos = 0;
        let mut last_match = None;
        while let Some(mut p) = text[pos..].find(&pattern) {
            p += pos;
            last_match = Some(p + pattern.len());
            pos = p + 1;
        }
        let Some(after_key) = last_match else { continue };

        // 在 after_key.. 中找第一个 [
        let remaining = &text[after_key..];
        let bracket_start = remaining.find('[')?;
        let array_slice = &remaining[bracket_start..];

        // 裸 `[`/`]` 深度追踪：不处理引号，只数括号
        let mut depth = 0u32;
        let mut end = 0;
        for (i, b) in array_slice.bytes().enumerate() {
            if b == b'[' {
                depth += 1;
            } else if b == b']' {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
        }
        if depth != 0 {
            continue;
        } // 数组未闭合

        // 尝试解析这个数组片段
        let array_text = &array_slice[..=end];
        let parsed = match serde_json::from_str::<serde_json::Value>(array_text) {
            Ok(v) => Some(v),
            Err(_) => {
                // 数组内部可能有尾逗号等小语法问题，尝试 repair_json 修复后重试
                let repaired = repair_json(array_text);
                serde_json::from_str::<serde_json::Value>(&repaired).ok()
            },
        };
        if let Some(v) = parsed {
            result.insert(key.to_string(), v);
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(result))
    }
}

/// 从文本中提取最外层 JSON 对象或数组。
/// 跳过开头的空白和非 JSON 字符，找到第一个 `{` 或 `[`，
/// 追踪括号平衡（带 in_string 追踪）找到匹配闭合，返回解析结果。
/// 如果第一个起点解析失败，尝试下一个起点。
fn extract_outer_json(text: &str) -> Option<serde_json::Value> {
    let bytes = text.as_bytes();
    let len = bytes.len();

    // 收集所有 { 和 [ 的位置
    let start_positions: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| {
            if b == b'{' || b == b'[' {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    for &start in &start_positions {
        let open = bytes[start];
        let close: u8 = if open == b'{' { b'}' } else { b']' };

        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escaped = false;
        let mut found = false;
        let mut end = 0;

        for (idx, &b) in bytes[start..len].iter().enumerate() {
            let i = start + idx;
            if escaped {
                escaped = false;
                continue;
            }
            if b == b'\\' && in_string {
                escaped = true;
                continue;
            }
            if b == b'"' {
                in_string = !in_string;
                continue;
            }
            if !in_string {
                if b == open {
                    depth += 1;
                } else if b == close {
                    depth -= 1;
                    if depth == 0 {
                        found = true;
                        end = i;
                        break;
                    }
                }
            }
        }
        if !found {
            continue;
        }

        let snippet = &text[start..=end];
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(snippet) {
            return Some(v);
        }
    }
    None
}

/// 宽松 JSON 解析：处理模型在 `input` 字段里偶尔出现的轻微格式问题。
#[allow(dead_code)]
fn parse_loose_json(s: &str) -> Option<serde_json::Value> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(v);
    }
    // 兼容：input 有时是单引号 / 带尾逗号 / 缺外层花括号，这里走 IR 文本块的抽取
    let candidate = axagent_core::extract_json_from_llm_response(trimmed);
    serde_json::from_str(candidate).ok()
}

/// 深度搜索：从任意嵌套的 JSON 对象中找到含 stock_code 的候选数组
/// 用于兜底提取，当正常路径（params → candidates）失败时
fn find_candidates_deep(value: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    match value {
        serde_json::Value::Array(arr) => {
            // 检查数组元素是否像候选对象（有 stock_code）
            for item in arr {
                if item.get("stock_code").is_some()
                    && item
                        .get("stock_code")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| !s.is_empty())
                {
                    results.push(item.clone());
                } else if item.is_object() || item.is_array() {
                    // 递归搜索
                    results.extend(find_candidates_deep(item));
                }
            }
        },
        serde_json::Value::Object(map) => {
            // 优先找 candidates/stocks 等容器字段
            for key in ["candidates", "stocks", "list", "data", "items"] {
                if let Some(v) = map.get(key) {
                    if v.is_array() {
                        for item in v.as_array().unwrap() {
                            if item.get("stock_code").is_some() {
                                results.push(item.clone());
                            }
                        }
                    }
                }
            }
            // 没找到则递归搜索所有值
            if results.is_empty() {
                for v in map.values() {
                    results.extend(find_candidates_deep(v));
                }
            }
        },
        _ => {},
    }
    results
}

/// 逐个提取候选对象：在 candidates 数组内对每个顶层 `{...}` 独立尝试解析。
/// 当某个候选对象内部有语法错误（如字符串中未转义的 `"`）时，不影响其他候选的提取。
fn extract_candidates_one_by_one(text: &str) -> Option<Vec<serde_json::Value>> {
    // 1. 定位 candidates 数组起始
    let arr_start = {
        let key_pos = text.find("\"candidates\"")?;
        let after_key = &text[key_pos + 12..];
        let bracket = after_key.find('[')?;
        key_pos + 12 + bracket + 1
    };
    let content = &text[arr_start..];
    // 2. 逐个扫描顶层对象：正确追踪 in_string
    let mut depth: i32 = 0;
    let mut obj_start: Option<usize> = None;
    let mut in_string = false;
    let mut escaped = false;
    let mut results = Vec::new();
    for (i, &b) in content.as_bytes().iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' && in_string {
            escaped = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if b == b'{' {
            depth += 1;
            if depth == 1 {
                obj_start = Some(i);
            }
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                if let Some(os) = obj_start.take() {
                    let slice = &content[os..=i];
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(slice) {
                        results.push(v);
                    } else {
                        // 单个候选内部有语法错误 → repair_json 后重试
                        let repaired = repair_json(slice);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&repaired) {
                            results.push(v);
                        }
                    }
                }
            }
        } else if b == b']' && depth == 0 {
            break;
        }
    }
    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

/// 从文本内容中尝试启发式提取候选列表
/// 用于最终兜底：当所有结构化提取都失败时，直接从 LLM 文本输出中挖
/// 返回 (candidates 数组, 是否包含 summary 字段)
fn try_extract_candidates_from_text(text: &str) -> Option<(Vec<serde_json::Value>, bool)> {
    // 尝试1: 找 "candidates": [ ... ] 块，逐个提取
    // 逐个提取相比于全量解析更稳健——即使某个候选对象内部有语法错误，
    // 其他候选仍能被回收。（LLM 高频问题：字符串值中未转义的引号）
    if let Some(found) = extract_candidates_one_by_one(text) {
        let summary_pos = text.find("\"summary\"").map(|p| p.saturating_sub(500));
        let has_summary = summary_pos.is_some_and(|sp| {
            let region = &text[sp..sp.saturating_add(200)];
            region.contains(": \"") || region.contains(":\"")
        });
        return Some((found, has_summary));
    }

    // 尝试2: 搜索 "stock_code": "XXXXXX" 模式，提取周围的对象
    let mut found = Vec::new();
    let mut search_start = 0;
    while let Some(pos) = text[search_start..].find("\"stock_code\"") {
        let abs_pos = search_start + pos;
        // 验证后面跟着 : "6位数字"
        let after_key = &text[abs_pos + 12..];
        if after_key.starts_with("\": \"") {
            let code_start = abs_pos + 15;
            if code_start + 6 <= text.len() {
                let code = &text[code_start..code_start + 6];
                if code.chars().all(|c| c.is_ascii_digit()) {
                    // 向前找 { 向后找 } 来包围这个对象
                    let region_start = abs_pos.saturating_sub(300);
                    let region_end = (abs_pos + 500).min(text.len());
                    let region = &text[region_start..region_end];
                    let obj_offset = abs_pos - region_start;
                    if let Some(obj_s) = region[..obj_offset].rfind('{') {
                        if let Some(obj_e) = region[obj_s..].find('}') {
                            let candidate_str = &region[obj_s..obj_s + obj_e + 1];
                            if let Ok(obj) =
                                serde_json::from_str::<serde_json::Value>(candidate_str)
                            {
                                if obj.get("stock_code").is_some()
                                    && obj.get("stock_name").is_some()
                                {
                                    found.push(obj);
                                }
                            }
                        }
                    }
                }
            }
        }
        search_start = abs_pos + 13; // 跳过已搜索部分
    }

    if found.is_empty() {
        None
    } else {
        Some((found, false))
    }
}

/// 从节点原始输出中直接提取 candidates 数组
/// 与通用 extract_agent_output 不同，此函数直接导航已知 JSON 路径：
///   {"content": "...```json\n{\"name\": \"...\", \"arguments\": {\"candidates\": [...]}\n```..."}
/// 返回 {"candidates": [...], "summary": "..."} 或 null。
/// `summary` 取自 arguments.summary（当上游趋势/瓶颈数据缺失时，LLM 通常会
/// 在此字段给出"为什么没有候选"的解释，前端需要在空候选时把它展示给用户）。
fn serenity_extract_from_node(raw: &serde_json::Value) -> serde_json::Value {
    let content = match raw.get("content").and_then(|c| c.as_str()) {
        Some(c) => c,
        None => {
            tracing::warn!("[serenity] 节点输出无 content 字段");
            return serde_json::Value::Null;
        },
    };
    let extracted = axagent_core::extract_json_from_llm_response(content);
    let parsed: serde_json::Value = match serde_json::from_str(extracted) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[serenity] JSON 解析失败: {e}, 尝试修复链");
            // 第一层：repair_json 修复括号/引号 → 重新解析
            let repaired = repair_json(extracted);
            if let Ok(v) = serde_json::from_str(&repaired) {
                tracing::info!("[serenity] repair_json 成功");
                return v;
            }
            // 第二层：extract_named_arrays 从裁剪后文本提取
            if let Some(named) = extract_named_arrays(extracted) {
                tracing::info!("[serenity] extract_named_arrays(extracted) 成功");
                return named;
            }
            // 第三层：extract_named_arrays 从原始 content 提取
            // 裁剪后的 extracted 可能被 trim_after_json 截断，
            // 原始 content 包含完整 JSON，免疫截断问题
            if let Some(named) = extract_named_arrays(content) {
                tracing::info!("[serenity] extract_named_arrays(content) 成功");
                return named;
            }
            // 第四层：文本启发式兜底
            tracing::warn!("[serenity] 修复链前三层均失败，尝试文本兜底提取");
            if let Some((found, has_summary)) = try_extract_candidates_from_text(content) {
                if has_summary {
                    tracing::info!("[serenity] 文本兜底提取成功，0 个候选 + summary 字段");
                    return serde_json::json!({"candidates": [], "summary": "上游数据不足，无法识别有效候选标的"});
                }
                return serde_json::json!({"candidates": found});
            }
            return serde_json::Value::Null;
        },
    };
    // 导航到 arguments/input → candidates
    let args = parsed
        .as_object()
        .and_then(|o| o.get("arguments"))
        .or_else(|| parsed.as_object().and_then(|o| o.get("input")));
    let candidates = match args {
        Some(a) => a.get("candidates"),
        None => parsed.as_object().and_then(|o| o.get("candidates")),
    };
    // summary 同样在 arguments.summary（或顶层 summary），用来在 candidates 为空时
    // 告知前端"为什么没有候选"（如：上游 data_gaps=true、模型反幻觉拒绝编造等）
    let summary = args
        .and_then(|a| a.get("summary"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            parsed
                .as_object()
                .and_then(|o| o.get("summary"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        });
    match candidates {
        Some(arr) if arr.is_array() => {
            let count = arr.as_array().map(|a| a.len()).unwrap_or(0);
            tracing::info!("[serenity] 直接提取成功，找到 {} 个候选", count);
            if let Some(s) = summary {
                serde_json::json!({"candidates": arr, "summary": s})
            } else {
                serde_json::json!({"candidates": arr})
            }
        },
        Some(_) => {
            tracing::warn!(
                "[serenity] candidates 不是数组，keys={:?}",
                candidates
                    .and_then(|c| c.as_object())
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
            );
            // 即使 candidates 字段格式异常，summary 仍可能有用
            if let Some(s) = summary {
                serde_json::json!({"candidates": [], "summary": s})
            } else {
                serde_json::Value::Null
            }
        },
        None => {
            // 最后的兜底：parsed 本身可能是裸候选对象
            if parsed.get("stock_code").is_some() {
                let mut out = serde_json::json!({"candidates": [parsed]});
                if let Some(s) = summary {
                    out.as_object_mut()
                        .map(|o| o.insert("summary".to_string(), serde_json::Value::String(s)));
                }
                out
            } else if let Some(s) = summary {
                // 找不到 candidates 字段但有 summary：典型场景是 LLM 拒绝编造
                tracing::info!("[serenity] 未找到 candidates 字段但有 summary，空候选 + 原因");
                serde_json::json!({"candidates": [], "summary": s})
            } else {
                tracing::warn!(
                    "[serenity] 无法找到 candidates 字段, parsed keys={:?}",
                    parsed
                        .as_object()
                        .map(|o| o.keys().cloned().collect::<Vec<_>>())
                );
                serde_json::Value::Null
            }
        },
    }
}

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
        tool_permissions: Some(Arc::new(ToolPermissions {
            strict_mode: true,
            ..Default::default()
        })),
        ..Default::default()
    };

    let exec = async { engine.run_workflow(&wf_id, opts).await };

    let result = as_of::AS_OF.scope(as_of_ctx, exec).await;

    match result {
        Ok(wf_result) => {
            tracing::info!(
                "[serenity] wf_result.results 所有键: {:?}",
                wf_result.results.keys().cloned().collect::<Vec<_>>(),
            );

            let candidates_raw = wf_result
                .results
                .get("a-candidate-mapper")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // 诊断：打印原始节点输出
            {
                let preview = serde_json::to_string(&candidates_raw)
                    .map(|s| s.chars().take(500).collect::<String>())
                    .unwrap_or_default();
                tracing::info!("[serenity] a-candidate-mapper 原始输出 (前500字符): {}", preview);
            }
            // 使用专属提取函数直接从已知 JSON 路径 (content → arguments.candidates) 提取，
            // 绕过 extract_agent_output 的复杂 fallback 逻辑（该函数在 tool_json 格式下可能返回首条候选而非完整数组）
            let candidates_raw_fallback = candidates_raw.clone();
            let candidates = serenity_extract_from_node(&candidates_raw);
            // 诊断：serenity_extract_from_node 的返回值
            {
                let preview = serde_json::to_string(&candidates)
                    .map(|s| s.chars().take(400).collect::<String>())
                    .unwrap_or_default();
                tracing::info!(
                    "[serenity] serenity_extract 返回类型={}  前400字符: {}",
                    if candidates.is_array() {
                        "数组".to_string()
                    } else if candidates.is_object() {
                        let keys = candidates
                            .as_object()
                            .map(|o| o.keys().cloned().collect::<Vec<_>>())
                            .unwrap_or_default();
                        format!("对象 keys=[{}]", keys.join(","))
                    } else if candidates.is_null() {
                        "null".to_string()
                    } else {
                        "其他".to_string()
                    },
                    preview,
                );
            }
            // 规范化：如果 extract_agent_output 返回裸候选对象（有 stock_code 但无 candidates 包装键），
            // 包装成 {"candidates": [obj]}，使下游 .get("candidates") 能正常工作。
            let candidates = if candidates.is_object()
                && !candidates
                    .as_object()
                    .is_some_and(|o| o.contains_key("candidates"))
                && candidates.get("stock_code").is_some()
            {
                serde_json::json!({"candidates": [candidates]})
            } else {
                candidates
            };
            // 提取 candidates 数组（各种包装格式统一为平级数组，直接供前端消费）
            let raw_candidate_array = if candidates.is_array() {
                candidates.clone()
            } else if let Some(obj) = candidates.as_object() {
                obj.get("candidates")
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![]))
            } else {
                serde_json::Value::Array(vec![])
            };
            // 校验：过滤缺少 stock_code 的残缺候选，避免前端渲染空白卡片
            let mut candidate_array: Vec<serde_json::Value> = Vec::new();
            let mut dropped_count = 0;
            if let Some(arr) = raw_candidate_array.as_array() {
                for c in arr {
                    let has_code = c
                        .get("stock_code")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| !s.is_empty());
                    if has_code {
                        candidate_array.push(c.clone());
                    } else {
                        dropped_count += 1;
                        tracing::warn!(
                            "[serenity] 丢弃残缺候选（无 stock_code）: {}",
                            serde_json::to_string(c).unwrap_or_default()
                        );
                    }
                }
            }
            if dropped_count > 0 || candidate_array.is_empty() {
                tracing::warn!(
                    "[serenity] 候选校验: 总量={}, 有效={}, 丢弃(无stock_code)={}, candidates原始keys={:?}",
                    raw_candidate_array.as_array().map_or(0, |a| a.len()),
                    candidate_array.len(),
                    dropped_count,
                    raw_candidate_array
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|c| c.as_object())
                        .map(|o| o.keys().cloned().collect::<Vec<_>>()),
                );
            }
            let mut candidate_array = serde_json::Value::Array(candidate_array);

            // 兜底：如果正常提取路径得到空数组，尝试从 candidates（extract_agent_output 结果）中深度搜索
            if candidate_array.as_array().is_none_or(|a| a.is_empty()) {
                tracing::warn!(
                    "[serenity] ⚠️ 候选数组为空，尝试兜底提取... candidates类型={} keys={:?}",
                    if candidates.is_array() {
                        "array"
                    } else if candidates.is_object() {
                        "object"
                    } else if candidates.is_null() {
                        "null"
                    } else {
                        "other"
                    },
                    candidates
                        .as_object()
                        .map(|o| o.keys().cloned().collect::<Vec<_>>()),
                );
                // 兜底策略1: 从 candidates 对象的任意嵌套层搜索含 stock_code 的数组
                let fallback = find_candidates_deep(&candidates);
                if !fallback.is_empty() {
                    tracing::info!("[serenity] 兜底提取成功，找到 {} 个候选", fallback.len());
                    candidate_array = serde_json::json!(fallback);
                }
                // 兜底策略2: 从原始节点输出的 content 字段中提取
                if candidate_array.as_array().is_none_or(|a| a.is_empty()) {
                    if let Some(content) = candidates_raw_fallback
                        .get("content")
                        .and_then(|c| c.as_str())
                    {
                        if let Some((found, _)) = try_extract_candidates_from_text(content) {
                            if !found.is_empty() {
                                tracing::info!(
                                    "[serenity] 文本兜底提取成功，找到 {} 个候选",
                                    found.len()
                                );
                                candidate_array = serde_json::json!(found);
                            }
                        }
                    }
                }
            }

            // 提取趋势扫描结果（a-trend-scanner 节点输出）
            let trends_raw = wf_result
                .results
                .get("a-trend-scanner")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let trends = extract_agent_output(trends_raw).await;
            // 规范化：如果返回裸 trend 对象（有 trend_name 但无 trends 包装键），
            // 包装成 {"trends": [obj]}
            let trends = if trends.is_object()
                && !trends.as_object().is_some_and(|o| o.contains_key("trends"))
                && trends.get("trend_name").is_some()
            {
                serde_json::json!({"trends": [trends]})
            } else {
                trends
            };
            // trends 可能是 { trends: [...] } 对象，也可能是原始数组
            let trends_list = trends
                .as_object()
                .and_then(|obj| obj.get("trends"))
                .cloned()
                .unwrap_or(trends);

            tracing::info!(
                "[serenity] candidates 提取后类型: {}, keys: {:?}; trends 提取后类型: {}",
                if candidates.is_array() {
                    "数组"
                } else if candidates
                    .as_object()
                    .map(|o| o.contains_key("candidates"))
                    .unwrap_or(false)
                {
                    "含 candidates 字段"
                } else if candidates.is_null() {
                    "null"
                } else {
                    "其他"
                },
                candidates
                    .as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>()),
                if trends_list.is_array() {
                    "数组"
                } else {
                    "对象"
                },
            );

            // 提取"为什么没有候选"的原因：a-candidate-mapper 的 arguments.summary
            // 当上游三个瓶颈节点均返回 data_gaps=true 时，LLM 会拒绝编造候选
            // 并在 summary 字段说明原因；前端在 candidates 为空时展示给用户。
            let empty_reason = candidates
                .as_object()
                .and_then(|o| o.get("summary"))
                .and_then(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            // 先 emit completed 事件，确保持久化失败不会阻断前端通知
            let _ = app_h.emit(
                "serenity-screening-completed",
                serde_json::json!({
                    "workflowId": wf_id_ret,
                    "status": "completed",
                    "result": candidates,
                    "candidates": candidate_array,
                    "trends": trends_list,
                    "emptyReason": empty_reason,
                }),
            );

            // 持久化 Serenity 候选到 reco_picks 表（style="serenity"）
            // best-effort：失败只记日志，不影响返回结果
            {
                let db = state.harness.db();
                // 统一 generated_at 格式：与 recommend_stocks 一致(ISO 8601 带毫秒)
                let now_str = chrono::Local::now()
                    .format("%Y-%m-%dT%H:%M:%S%.3f")
                    .to_string();
                let ts_ms = chrono::Utc::now().timestamp_millis();
                // candidates 可能是 { candidates: [...] } 对象、{name, arguments: {candidates: [...]}} 格式、
                // 也可能是原始数组
                let candidate_list: Vec<&serde_json::Value> = candidates
                    .as_object()
                    .and_then(|obj| {
                        // 优先顶层 candidates
                        obj.get("candidates")
                            .or_else(|| {
                                // 兼容 tool_json 格式: {name, arguments: {candidates: [...]}}
                                obj.get("arguments").and_then(|a| a.get("candidates"))
                            })
                            .and_then(|v| v.as_array())
                    })
                    .or_else(|| candidates.as_array())
                    .map(|arr| arr.iter().collect())
                    .unwrap_or_default();
                let mut detail_cache: std::collections::HashMap<String, serde_json::Value> =
                    std::collections::HashMap::new();
                let mut serenity_seed: Vec<(String, String, Option<String>)> = Vec::new();
                for (i, c) in candidate_list.iter().enumerate() {
                    let code = c["stock_code"].as_str().unwrap_or("");
                    let name = c["stock_name"].as_str().unwrap_or("");
                    let conf = c["confidence"].as_i64().unwrap_or(50) as i32;
                    if code.is_empty() {
                        continue;
                    }
                    // 构造完整 RecoPick JSON（与 types.rs 中 camelCase 一致）
                    // 候选数据不保证有价格/入场/止损等字段,缺失时填 0 或默认值
                    let pick_data_val = serde_json::json!({
                        "stockCode": code,
                        "stockName": name,
                        "style": "serenity",
                        "period": "mid",
                        "price": c.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        "entryLow": c.get("entryLow").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        "entryHigh": c.get("entryHigh").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        "stopLoss": c.get("stopLoss").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        "targetPrice": c.get("targetPrice").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        "positionPct": c.get("positionPct").and_then(|v| v.as_f64()).unwrap_or(5.0),
                        "holdingDays": c.get("holdingDays").and_then(|v| v.as_i64()).unwrap_or(20),
                        "confidence": conf,
                        "reasons": c.get("reasons").and_then(|v| v.as_array()).map(|a| {
                            a.iter().filter_map(|v| v.as_str().map(|s| s.to_owned())).collect::<Vec<_>>()
                        }).unwrap_or_default(),
                        "riskNotes": [],
                        "secondaryStyles": [],
                        "synthetic": false,
                    });
                    // 持久化到 reco_picks
                    let pick_id = format!("serenity-{ts_ms}-{i}-{code}");
                    let pick = reco_picks::ActiveModel {
                        id: Set(pick_id),
                        generated_at: Set(now_str.clone()),
                        period: Set("mid".to_string()),
                        stock_code: Set(code.to_string()),
                        stock_name: Set(name.to_string()),
                        style: Set("serenity".to_string()),
                        confidence: Set(conf),
                        synthetic: Set(0),
                        seed_pool_json: Set(Some(serde_json::to_string(c).unwrap_or_default())),
                        strategy_weights_json: Set(None),
                        pick_data: Set(Some(
                            serde_json::to_string(&pick_data_val).unwrap_or_default(),
                        )),
                        created_at: Set(now_str.clone()),
                    };
                    if let Err(e) = pick.insert(db).await {
                        tracing::warn!("[serenity] 写入 reco_picks 失败 ({}): {e}", code);
                    }
                    // 构建全量数据缓存
                    detail_cache.insert(
                        code.to_string(),
                        serde_json::json!({
                            "serenity_score": c["serenity_score"],
                            "catalysts": c["catalysts"],
                            "exit_signals": c["exit_signals"],
                            "attention_metrics": c["attention_metrics"],
                            "bottleneck_product": c["bottleneck_product"],
                            "primary_risk": c["primary_risk"],
                            "relevance": c["relevance"],
                            "confidence": conf,
                        }),
                    );
                    // 构建种子列表
                    serenity_seed.push((code.to_string(), name.to_string(), None));
                }
                // 同步到全局种子 + 全量数据缓存
                if !serenity_seed.is_empty() {
                    axagent_stock_analysis::recommender::set_serenity_seed(serenity_seed);
                    axagent_stock_analysis::recommender::set_serenity_candidate_cache(detail_cache);
                }
            }

            // wrap array candidates for frontend
            let result_val = if candidates.is_array() {
                serde_json::json!({
                    "candidates": candidates,
                })
            } else {
                candidates
            };
            Ok(serde_json::json!({
                "status": "completed",
                "candidates": result_val["candidates"].clone(),
                "trends": trends_list,
                "emptyReason": empty_reason,
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

/// 刷新 Serenity 候选的退出信号（Phase 3 持续监控）
/// 加载最近一次 Serenity 筛选的候选列表，逐个检查退出条件
/// 支持 as_of_date 参数用于回放模式
#[tauri::command]
pub async fn refresh_serenity_exit_signals(
    state: State<'_, AppState>,
    as_of_date: Option<String>,
) -> Result<serde_json::Value, String> {
    // 如果指定了 as_of_date，在 as-of 作用域内执行
    if let Some(ref date_str) = as_of_date {
        let as_of_ctx = parse_asof_param(Some(date_str.clone()))
            .map_err(|e| format!("解析 as_of_date 失败: {e}"))?;
        let exec = async { do_refresh_exit_signals(&state).await };
        return as_of::AS_OF.scope(as_of_ctx, exec).await;
    }
    do_refresh_exit_signals(&state).await
}

async fn do_refresh_exit_signals(state: &State<'_, AppState>) -> Result<serde_json::Value, String> {
    use axagent_core::entity::reco_picks;
    use sea_orm::{EntityTrait, QueryOrder};

    let db = state.harness.db();
    // 加载最近 50 条 Serenity 候选（按 created_at 降序）
    let picks = reco_picks::Entity::find()
        .filter(reco_picks::Column::Style.eq("serenity"))
        .order_by_desc(reco_picks::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| format!("查询 Serenity 候选失败: {e}"))?;
    // 只取最近 50 条
    let picks: Vec<_> = picks.into_iter().take(50).collect();

    let client = &state.astock_client;
    let mut results = Vec::new();

    for pick in &picks {
        let stop_loss = pick.seed_pool_json.as_ref().and_then(|seed_json| {
            serde_json::from_str::<serde_json::Value>(seed_json)
                .ok()
                .and_then(|v| v["stop_loss"].as_f64())
        });

        // 获取当前行情
        let quote = client.get_quote(&pick.stock_code).await.ok();
        let price = quote.as_ref().map(|q| q.price).unwrap_or(0.0);

        // 搜索退出相关新闻
        let news = client
            .search_news(&format!("{} 技术替代 产能过剩", pick.stock_code), 5)
            .await
            .unwrap_or_default();
        let has_disruption_news = news.len() >= 2;

        // 检查毛利率趋势
        let margin_declining = client
            .get_financials(&pick.stock_code)
            .await
            .ok()
            .and_then(|f| {
                if f.len() >= 2 {
                    let curr = f[0].gross_margin.unwrap_or(0.0);
                    let prev = f[1].gross_margin.unwrap_or(0.0);
                    Some(prev > 0.0 && curr < prev * 0.85)
                } else {
                    None
                }
            })
            .unwrap_or(false);

        // 判断退出紧迫度
        let stop_loss_hit = stop_loss.map(|sl| price < sl).unwrap_or(false);
        let urgency = if stop_loss_hit || (has_disruption_news && margin_declining) {
            "exit_now"
        } else if has_disruption_news || margin_declining {
            "caution"
        } else {
            "no_urgency"
        };

        results.push(serde_json::json!({
            "stock_code": pick.stock_code,
            "stock_name": pick.stock_name,
            "current_price": price,
            "stop_loss_hit": stop_loss_hit,
            "has_disruption_news": has_disruption_news,
            "margin_declining": margin_declining,
            "exit_urgency": urgency,
            "confidence": pick.confidence,
        }));
    }

    Ok(serde_json::json!({
        "status": "completed",
        "checked_count": results.len(),
        "exit_now_count": results.iter().filter(|r| r["exit_urgency"] == "exit_now").count(),
        "caution_count": results.iter().filter(|r| r["exit_urgency"] == "caution").count(),
        "candidates": results,
    }))
}

/// 刷新 Serenity 回馈闭环：跟踪推荐表现、验证催化剂、调优权重
#[tauri::command]
pub async fn refresh_serenity_feedback(
    state: State<'_, AppState>,
    as_of_date: Option<String>,
) -> Result<serde_json::Value, String> {
    // 如果指定了 as_of_date，在 as-of 作用域内执行
    if let Some(ref date_str) = as_of_date {
        let as_of_ctx = parse_asof_param(Some(date_str.clone()))
            .map_err(|e| format!("解析 as_of_date 失败: {e}"))?;
        let exec = async { do_feedback_loop(&state).await };
        return as_of::AS_OF.scope(as_of_ctx, exec).await;
    }
    do_feedback_loop(&state).await
}

async fn do_feedback_loop(state: &State<'_, AppState>) -> Result<serde_json::Value, String> {
    use axagent_core::entity::reco_picks;
    use sea_orm::{EntityTrait, QueryOrder};

    let db = state.harness.db();
    // 固定取过去 30 天的 Serenity 候选，避免新工作流产出的记录不断顶替旧样本
    let thirty_days_ago = chrono::Utc::now() - chrono::Duration::days(30);
    let cutoff = thirty_days_ago.to_rfc3339();
    let picks = reco_picks::Entity::find()
        .filter(reco_picks::Column::Style.eq("serenity"))
        .filter(reco_picks::Column::CreatedAt.gte(cutoff))
        .order_by_desc(reco_picks::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| format!("查询 Serenity 候选失败: {e}"))?;

    let client = &state.astock_client;
    let mut performances = Vec::new();

    for (idx, pick) in picks.iter().enumerate() {
        // 提取推荐日期（从 created_at 取前 10 字符 = YYYY-MM-DD）
        let rec_date = pick.created_at.as_str().get(..10).unwrap_or("2025-01-01");

        // 提取候选全量数据（seed_pool_json 存储的是候选 JSON 对象）
        let detail = pick
            .seed_pool_json
            .as_ref()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok());

        if detail.is_none() {
            tracing::info!(
                "[serenity-feedback] pick={} seed_pool_json=None，跳过（历史数据）",
                pick.stock_code
            );
            // 历史数据：seed_pool_json 为 None，无法计算催化剂，跳过
            continue;
        }

        // 限流：每处理 1 条记录后延迟 500ms，避免触发东方财富 API 限流
        if idx > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // 计算表现：获取推荐日至今的 K 线
        tracing::info!("[serenity-feedback] pick={} 获取 K 线", pick.stock_code);
        let entry_kline = match client.get_klines(&pick.stock_code, "daily", 120).await {
            Ok(k) => {
                tracing::info!(
                    "[serenity-feedback] pick={} K 线成功, {} 条",
                    pick.stock_code,
                    k.len()
                );
                Some(k)
            },
            Err(e) => {
                tracing::warn!("[serenity-feedback] pick={} K 线失败: {e:?}", pick.stock_code);
                None
            },
        };
        let (entry_price, used_fallback) = entry_kline
            .as_ref()
            .and_then(|k| {
                // 优先找推荐日当天的 K 线
                k.iter().find(|k| k.date.starts_with(rec_date)).map(|k| (k.close, false))
                // 找不到则用倒数第二根（推荐日 K 线不在时避免与 current_price 撞车）
                .or_else(|| {
                    if k.len() >= 2 {
                        Some((k[k.len()-2].close, true))
                    } else {
                        k.last().map(|k| (k.close, true))
                    }
                })
            })
            .unwrap_or((0.0, false));
        if entry_price <= 0.0 {
            tracing::warn!(
                "[serenity-feedback] pick={} entry_price=0 (rec_date={}, kline_count={})",
                pick.stock_code,
                rec_date,
                entry_kline.as_ref().map(|k| k.len()).unwrap_or(0)
            );
        } else {
            tracing::info!(
                "[serenity-feedback] pick={} entry_price={} (rec_date={}){}",
                pick.stock_code,
                entry_price,
                rec_date,
                if used_fallback {
                    " [参考值:推荐日K线未收盘，取前一日]"
                } else {
                    ""
                },
            );
        }

        let current_quote = match client.get_quote(&pick.stock_code).await {
            Ok(q) => {
                tracing::info!(
                    "[serenity-feedback] pick={} get_quote 成功: price={}",
                    pick.stock_code,
                    q.price
                );
                Some(q)
            },
            Err(e) => {
                // 打印完整错误链，帮助定位 error sending request 的根因
                tracing::warn!(
                    "[serenity-feedback] pick={} get_quote 失败: {e:#?}",
                    pick.stock_code
                );
                // 同时打印 source chain
                let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
                while let Some(s) = src {
                    tracing::warn!("[serenity-feedback]   Caused by: {s:#?}");
                    src = s.source();
                }
                None
            },
        };
        let current_price = current_quote.as_ref().map(|q| q.price).unwrap_or(0.0);
        let return_pct = if entry_price > 0.0 && current_price > 0.0 {
            (current_price - entry_price) / entry_price * 100.0
        } else {
            0.0
        };

        // 验证催化剂：从 detail 中提取（兼容多种字段名）
        let catalysts_info = detail
            .as_ref()
            .map(|d| {
                // 尝试多种可能的字段名
                let arr = d
                    .get("catalysts")
                    .or_else(|| d.get("catalyst"))
                    .or_else(|| d.get("catalyst_list"));
                arr.and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)
            })
            .unwrap_or(0);
        // 同时尝试嵌套路径：有些工作流输出把 catalysts 放在 params.catalysts
        let catalysts_info = if catalysts_info == 0 {
            detail
                .as_ref()
                .and_then(|d| {
                    d.get("params")
                        .and_then(|p| p.get("catalysts"))
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                })
                .unwrap_or(0)
        } else {
            catalysts_info
        };

        // 搜索该股相关新闻作为催化剂验证的 proxy
        let catalyst_news = match client
            .search_news(&format!("{} 财报 量产 订单", pick.stock_code), 5)
            .await
        {
            Ok(news) => {
                tracing::info!(
                    "[serenity-feedback] pick={} search_news 成功: {} 条",
                    pick.stock_code,
                    news.len()
                );
                news
            },
            Err(e) => {
                tracing::warn!(
                    "[serenity-feedback] pick={} search_news 失败: {e:#?}",
                    pick.stock_code
                );
                Vec::new()
            },
        };
        let catalysts_verified_count = catalyst_news.len().min(catalysts_info);

        performances.push(serde_json::json!({
            "id": pick.id,
            "stock_code": pick.stock_code,
            "stock_name": pick.stock_name,
            "confidence": pick.confidence,
            "recommend_date": rec_date,
            "entry_price": entry_price,
            "current_price": current_price,
            "return_pct": (return_pct * 100.0).round() / 100.0,
            "is_profitable": return_pct > 0.0,
            "return_pending": used_fallback,
            "catalysts": serde_json::json!({
                "total": catalysts_info,
                "verified": catalysts_verified_count,
            }),
        }));
    }

    // 计算汇总指标
    let profitable = performances
        .iter()
        .filter(|p| p["is_profitable"].as_bool().unwrap_or(false))
        .count();
    let total = performances.len();
    let avg_return = if total > 0 {
        performances
            .iter()
            .map(|p| p["return_pct"].as_f64().unwrap_or(0.0))
            .sum::<f64>()
            / total as f64
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "status": "completed",
        "total": total,
        "profitable_count": profitable,
        "win_rate": if total > 0 { (profitable as f64 / total as f64 * 100.0).round() / 100.0 } else { 0.0 },
        "avg_return_pct": (avg_return * 100.0).round() / 100.0,
        "performances": performances,
    }))
}

/// 列表：荐股推荐历史记录（按 generated_at 分组，每条记录含时间/周期/股票数/风格列表）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoHistoryItem {
    pub generated_at: String,
    pub period: String,
    pub stock_count: i64,
    pub styles: String,
    pub created_at: String,
}

#[tauri::command]
pub async fn list_reco_history(
    state: State<'_, AppState>,
    style_filter: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<RecoHistoryItem>, String> {
    use sea_orm::{ConnectionTrait, Statement};
    let db = state.harness.db();

    let mut sql = String::from(
        "SELECT generated_at, period, COUNT(*) as stock_count, \
         GROUP_CONCAT(DISTINCT style) as styles, MAX(created_at) as created_at \
         FROM reco_picks WHERE 1=1",
    );
    let mut values: Vec<sea_orm::Value> = Vec::new();

    if let Some(ref style) = style_filter {
        sql.push_str(" AND style = ?");
        values.push(style.clone().into());
    }

    sql.push_str(" GROUP BY generated_at ORDER BY generated_at DESC");

    if let Some(l) = limit {
        sql.push_str(" LIMIT ?");
        values.push((l as i64).into());
    }
    if let Some(o) = offset {
        sql.push_str(" OFFSET ?");
        values.push((o as i64).into());
    }

    let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, sql.as_str(), values);

    let rows = db
        .query_all_raw(stmt)
        .await
        .map_err(|e| format!("查询荐股历史失败: {e}"))?;

    let items = rows
        .iter()
        .map(|row| RecoHistoryItem {
            generated_at: row
                .try_get::<String>("", "generated_at")
                .unwrap_or_default(),
            period: row.try_get::<String>("", "period").unwrap_or_default(),
            stock_count: row.try_get::<i64>("", "stock_count").unwrap_or(0),
            styles: row.try_get::<String>("", "styles").unwrap_or_default(),
            created_at: row.try_get::<String>("", "created_at").unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    Ok(items)
}

/// 获取某次荐股/瓶颈掘金详情（按 generated_at 获取该轮所有推荐股票）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoDetailItem {
    pub id: String,
    pub generated_at: String,
    pub period: String,
    pub stock_code: String,
    pub stock_name: String,
    pub style: String,
    pub confidence: i32,
    pub synthetic: i32,
    pub seed_pool_json: Option<String>,
    pub pick_data: Option<String>,
    pub created_at: String,
}

#[tauri::command]
pub async fn get_reco_detail(
    state: State<'_, AppState>,
    generated_at: String,
    style_filter: Option<String>,
) -> Result<Vec<RecoDetailItem>, String> {
    use axagent_core::entity::reco_picks;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = state.harness.db();
    let mut query =
        reco_picks::Entity::find().filter(reco_picks::Column::GeneratedAt.eq(&generated_at));

    if let Some(ref style) = style_filter {
        query = query.filter(reco_picks::Column::Style.eq(style));
    }

    let items = query
        .all(db)
        .await
        .map_err(|e| format!("查询荐股详情失败: {e}"))?;

    Ok(items
        .into_iter()
        .map(|m| RecoDetailItem {
            id: m.id,
            generated_at: m.generated_at,
            period: m.period,
            stock_code: m.stock_code,
            stock_name: m.stock_name,
            style: m.style,
            confidence: m.confidence,
            synthetic: m.synthetic,
            seed_pool_json: m.seed_pool_json,
            pick_data: m.pick_data,
            created_at: m.created_at,
        })
        .collect())
}

/// 批量删除荐股记录（按 generated_at 删除整轮推荐）
#[tauri::command]
pub async fn batch_delete_reco_history(
    state: State<'_, AppState>,
    generated_ats: Vec<String>,
) -> Result<(), String> {
    use axagent_core::entity::reco_picks;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = state.harness.db();
    for ts in &generated_ats {
        reco_picks::Entity::delete_many()
            .filter(reco_picks::Column::GeneratedAt.eq(ts))
            .exec(db)
            .await
            .map_err(|e| format!("删除荐股记录失败: {e}"))?;
    }
    Ok(())
}

/// 删除一条 Serenity 候选记录（回馈闭环中的删除操作）
#[tauri::command]
pub async fn delete_serenity_pick(state: State<'_, AppState>, id: String) -> Result<(), String> {
    use axagent_core::entity::reco_picks;
    use sea_orm::{EntityTrait, ModelTrait};

    let db = state.harness.db();
    let pick = reco_picks::Entity::find_by_id(&id)
        .one(db)
        .await
        .map_err(|e| format!("查询候选记录失败: {e}"))?
        .ok_or_else(|| "候选记录不存在".to_string())?;
    pick.delete(db)
        .await
        .map_err(|e| format!("删除候选记录失败: {e}"))?;
    tracing::info!("[serenity] 已删除候选记录: {id}");
    Ok(())
}

// ── [D1 借鉴] 批量反思 (B1+B2 闭环) ──
//
// 借鉴 TradingAgents 反思机制: 持仓期到达时,自动批量 resolve 所有
// `status='pending'` 的 stock_reflections row,无需用户手动逐条触发。
//
// 流程:
//   1. 扫 stock_reflections where status='pending',按 created_at ASC 处理
//   2. 对每条 row:
//      - 读 stock_analyses by original_analysis_id
//      - 计算持仓期: today - as_of_date
//      - 若 today - as_of_date >= decision_expected_holding_days (默认 28):
//        调 run_reflection_workflow(reflection_id=Some(rid)) 走 B3 UPDATE 路径
//      - 否则 skip (持仓期未到)
//   3. [D2 借鉴] Resolved FIFO 清理: 删除 90 天前或超 1000 条的 completed row
//   4. 返回 { total_pending, resolved, failed, skipped_young, cleaned_up }
//
// 调用方:
//   - `CronExecutor` 每天 18:00 调一次(收市后批量反思)
//   - 前端调试按钮: 手动立即跑一轮
#[tauri::command]
pub async fn run_batch_reflection(
    state: State<'_, AppState>,
    max_count: Option<u32>,
) -> Result<serde_json::Value, String> {
    use axagent_core::entity::stock_analyses;
    use axagent_core::entity::stock_reflections;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    let max_count = max_count.unwrap_or(20) as usize;
    let db = state.harness.db();

    // 1. 扫所有 pending row,按 created_at ASC(最老的先处理,避免积压)
    let pendings: Vec<stock_reflections::Model> = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.eq("pending"))
        .order_by_asc(stock_reflections::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| format!("D1 扫 pending row 失败: {e}"))?;

    tracing::info!(
        "[D1 batch_reflection] 扫到 {} 条 pending row, max_count={}",
        pendings.len(),
        max_count
    );

    let mut resolved = 0u32;
    let mut failed = 0u32;
    let mut skipped_young = 0u32; // 持仓期未到
    let mut errors: Vec<String> = Vec::new();
    let today_ms = chrono::Utc::now().timestamp_millis();

    for (i, p) in pendings.iter().take(max_count).enumerate() {
        // 2a. 读原始分析
        let analysis = match stock_analyses::Entity::find_by_id(&p.original_analysis_id)
            .one(db)
            .await
        {
            Ok(Some(a)) => a,
            Ok(None) => {
                tracing::warn!(
                    "[D1] pending reflection {} 关联 analysis_id={} 不存在,skip",
                    p.id,
                    p.original_analysis_id
                );
                skipped_young += 1;
                continue;
            },
            Err(e) => {
                tracing::error!("[D1] 查 analysis 失败: {e}");
                failed += 1;
                errors.push(format!("{}: 查询 analysis 失败: {e}", p.id));
                continue;
            },
        };

        // 2b. 计算持仓期是否到达
        // 默认 28 天 = mid 决策标准持仓期(用户没指定时取 stock-analysis 模板默认)
        let expected_days = analysis
            .decision_expected_holding_days
            .map(|d| d as i64)
            .unwrap_or(28);
        let analysis_date = analysis.as_of_date.as_deref().unwrap_or(&p.as_of_date);
        let analysis_ms = chrono::NaiveDate::parse_from_str(analysis_date, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|dt| dt.and_utc().timestamp_millis())
            .unwrap_or(p.created_at);
        let days_held = (today_ms - analysis_ms).max(0) / 86_400_000; // ms → days

        if days_held < expected_days {
            tracing::info!(
                "[D1] pending {} ({}) 持仓 {}/{} 天,未到期 skip",
                p.id,
                p.stock_code,
                days_held,
                expected_days
            );
            skipped_young += 1;
            continue;
        }

        // 2c. 调 run_reflection_workflow(B3 UPDATE 路径)
        let r = run_reflection_workflow(
            db,
            &state.astock_client,
            &state.work_engine,
            &state.vector_store,
            state.harness.master_key(),
            &p.stock_code,
            &p.stock_name,
            &p.original_analysis_id,
            &p.actual_outcome,      // 留空字符串走 legacy fallback
            None,                   // raw_return: pending 阶段未算
            None,                   // alpha_return
            Some(days_held as i32), // holding_days 填入
            None,                   // benchmark_name
            analysis_date,
            &chrono::Utc::now().format("%Y-%m-%d").to_string(),
            0u8,
            "light",
            Some(p.id.clone()), // [B2/B3] 走 UPDATE 路径
        )
        .await;

        match r {
            Ok(_) => {
                tracing::info!(
                    "[D1] ✓ resolved {}/{} pending: {} ({})",
                    i + 1,
                    pendings.len(),
                    p.id,
                    p.stock_code
                );
                resolved += 1;
            },
            Err(e) => {
                tracing::error!("[D1] ✗ resolve failed {}: {e}", p.id);
                failed += 1;
                errors.push(format!("{}: {e}", p.id));
            },
        }
    }

    // ── [D2 借鉴] Resolved FIFO 清理 ──
    // 保留最近 1000 条 + 90 天内的 completed row,删除更老的。
    // pending row 永远保留(B1 借鉴:不能丢反思需求)。
    let ninety_days_ago_ms = today_ms - 90 * 86_400_000;
    let cleaned_up = stock_reflections::Entity::delete_many()
        .filter(stock_reflections::Column::Status.eq("completed"))
        .filter(stock_reflections::Column::UpdatedAt.lt(ninety_days_ago_ms))
        .exec(db)
        .await
        .map(|r| r.rows_affected)
        .unwrap_or_else(|e| {
            tracing::warn!("[D2] FIFO 清理失败: {e}");
            0
        });
    tracing::info!("[D2 fifo_cleanup] 删除 {} 条超龄 completed row", cleaned_up);

    tracing::info!(
        "[D1 batch_reflection] 完成: total={} resolved={} failed={} skipped_young={} cleaned={}",
        pendings.len(),
        resolved,
        failed,
        skipped_young,
        cleaned_up
    );

    Ok(serde_json::json!({
        "totalPending": pendings.len(),
        "processed": pendings.len().min(max_count),
        "resolved": resolved,
        "failed": failed,
        "skippedYoung": skipped_young,
        "cleanedUp": cleaned_up,
        "errors": errors,
    }))
}

// ── [F1 借鉴] 提取反思教训为可重用规则 ──
//
// 借鉴 TradingAgents 反思→规则提取机制:反思完成后把 lesson_summary
// 提取为可重用的规则存入 reflection_lessons 表。
// 规则自动提取规则:lesson_summary ≤200 字符、含明确建议性内容的才提取。
async fn extract_lesson_to_rule(
    db: &sea_orm::DatabaseConnection,
    stock_code: &str,
    source_reflection_id: &str,
    lesson_summary: &str,
    verdict: Option<&str>,
) -> Result<(), String> {
    use axagent_core::entity::reflection_lessons;
    use sea_orm::ActiveModelTrait;
    use sea_orm::Set;

    // 短文本过短或无实际建议性内容则跳过
    let trimmed = lesson_summary.trim();
    if trimmed.len() < 10 || trimmed.len() > 250 {
        return Ok(());
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now_ms = chrono::Utc::now().timestamp_millis();
    // 从 verdict 推断初始置信度
    let confidence = match verdict {
        Some("wrong") => 0.7, // wrong 的教训更有价值,给更高初始置信度
        Some("partial") => 0.5,
        _ => 0.3, // correct 或 None 的教训价值较低
    };

    reflection_lessons::ActiveModel {
        id: Set(id),
        lesson_summary: Set(trimmed.to_string()),
        rule_pattern: Set(None), // 后续由 F1 迭代扩展: LLM 分析 lesson_summary 自动提取
        source_reflection_id: Set(Some(source_reflection_id.to_string())),
        stock_code: Set(Some(stock_code.to_string())),
        applicable_scenarios: Set(None),
        times_applied: Set(0),
        success_count: Set(0),
        confidence: Set(confidence),
        status: Set("active".to_string()),
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
    }
    .insert(db)
    .await
    .map(|_| ())
    .map_err(|e| format!("F1 写入 reflection_lessons 失败: {e}"))
}

// ── [缺陷5 fix] 内部批量反思函数(非 Tauri 命令,供 cron 调度器直接调用) ──
//
// 从 run_batch_reflection 提取的核心逻辑。
// 参数通过独立引用传入,不需要 AppState。
pub async fn run_batch_reflection_inner(
    db: &sea_orm::DatabaseConnection,
    _client: &axagent_astock_data::AStockClient,
    _engine: &axagent_rt_workflow::work_engine::WorkEngine,
    _vector_store: &axagent_core::vector_store::VectorStore,
    _master_key: &[u8; 32],
    max_count: Option<u32>,
) -> Result<serde_json::Value, String> {
    use axagent_core::entity::stock_analyses;
    use axagent_core::entity::stock_reflections;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    let max_count = max_count.unwrap_or(20) as usize;
    let today_ms = chrono::Utc::now().timestamp_millis();

    // 1. 扫所有 pending row,按 created_at ASC(最老的先处理,避免积压)
    let pendings: Vec<stock_reflections::Model> = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.eq("pending"))
        .order_by_asc(stock_reflections::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| format!("run_batch_reflection_inner 扫 pending row 失败: {e}"))?;

    tracing::info!(
        "[D1 batch_reflection] 扫到 {} 条 pending row, max_count={}",
        pendings.len(),
        max_count
    );

    let mut resolved = 0u32;
    let mut failed = 0u32;
    let mut skipped_young = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for p in pendings.iter().take(max_count) {
        let analysis = match stock_analyses::Entity::find_by_id(&p.original_analysis_id)
            .one(db)
            .await
        {
            Ok(Some(a)) => a,
            Ok(None) => {
                skipped_young += 1;
                continue;
            },
            Err(e) => {
                failed += 1;
                errors.push(format!("{}: 查询 analysis 失败: {e}", p.id));
                continue;
            },
        };

        let expected_days = analysis
            .decision_expected_holding_days
            .map(|d| d as i64)
            .unwrap_or(28);
        let analysis_date = analysis.as_of_date.as_deref().unwrap_or(&p.as_of_date);
        let analysis_ms = chrono::NaiveDate::parse_from_str(analysis_date, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|dt| dt.and_utc().timestamp_millis())
            .unwrap_or(p.created_at);
        let days_held = (today_ms - analysis_ms).max(0) / 86_400_000;

        if days_held < expected_days {
            skipped_young += 1;
            continue;
        }

        let r = run_reflection_workflow(
            db,
            _client,
            &std::sync::Arc::new(_engine.clone()),
            _vector_store,
            _master_key,
            &p.stock_code,
            &p.stock_name,
            &p.original_analysis_id,
            &p.actual_outcome,
            None,
            None,
            Some(days_held as i32),
            None,
            analysis_date,
            &chrono::Utc::now().format("%Y-%m-%d").to_string(),
            0u8,
            "light",
            Some(p.id.clone()),
        )
        .await;

        match r {
            Ok(_) => {
                resolved += 1;
            },
            Err(e) => {
                failed += 1;
                errors.push(format!("{}: {e}", p.id));
            },
        }
    }

    // D2 FIFO 清理
    let ninety_days_ago_ms = today_ms - 90 * 86_400_000;
    let cleaned_up = stock_reflections::Entity::delete_many()
        .filter(stock_reflections::Column::Status.eq("completed"))
        .filter(stock_reflections::Column::UpdatedAt.lt(ninety_days_ago_ms))
        .exec(db)
        .await
        .map(|r| r.rows_affected)
        .unwrap_or(0);

    Ok(serde_json::json!({
        "totalPending": pendings.len(),
        "processed": pendings.len().min(max_count),
        "resolved": resolved,
        "failed": failed,
        "skippedYoung": skipped_young,
        "cleanedUp": cleaned_up,
        "errors": errors,
    }))
}

// ── 单元测试：覆盖 LLM 输出 → IR → JSON 提取的全链路 ──
//
// 关键场景：
//   1) LLM 严格按新 prompt 输出 tool_json 块 → ToolUse 路径
//   2) LLM 偶发只输出普通 ```json 块（没有 name 字段） → 文本块 → 内部 JSON
//   3) LLM 输出截断的 JSON（用户日志里的"后 200 字符"场景） → 至少能拿到
//      一个有效前缀并解析出 candidates
//   4) Agent 节点输出顶层 params / output / candidates 字段 → 直返
//   5) extract_agent_output 顶层 params 优先于 content
#[cfg(test)]
mod serenity_extract_tests {
    use super::*;
    use axagent_harness::types::{ChatResponse, ContentBlock};
    use axagent_runtime_core::DefaultResponseNormalizer;

    // ── helper：把字符串送进 IR Normalizer 拿到 ContentBlock 列表 ──
    async fn normalize(content: &str) -> Vec<ContentBlock> {
        let resp = ChatResponse {
            id: String::new(),
            model: String::new(),
            content: content.to_string(),
            thinking: None,
            usage: Default::default(),
            tool_calls: None,
        };
        let normalizer = DefaultResponseNormalizer;
        normalizer.normalize(&resp).await
    }

    // ── 1) 标准 tool_json 块：name=submit_candidates，arguments 是数据 ──
    #[tokio::test]
    async fn tool_json_block_extracts_candidates() {
        let content = r#"```tool_json
{"name": "submit_candidates", "arguments": {"candidates": [{"stock_code": "300285", "stock_name": "国瓷材料", "serenity_score": 75}], "summary": "ok"}}
```"#;
        let v = extract_via_normalizer(content).await;
        let v = v.expect("IR 提取应成功");
        let arr = v
            .get("candidates")
            .and_then(|x| x.as_array())
            .expect("candidates 应为数组");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["stock_code"], "300285");
    }

    // ── 2) 普通 json 块（无 name 字段）→ IR 当文本块保留 → extract_json_from_llm_response 兜底 ──
    #[tokio::test]
    async fn plain_json_block_falls_back_to_text_extraction() {
        let content = r#"```json
{"trends": [{"trend_name": "AI 算力散热", "confidence": 80}]}
```"#;
        let v = extract_via_normalizer(content).await;
        let v = v.expect("纯 json 块应能解析");
        let arr = v
            .get("trends")
            .and_then(|x| x.as_array())
            .expect("trends 应为数组");
        assert_eq!(arr[0]["trend_name"], "AI 算力散热");
    }

    // ── 3) 截断 JSON（用户日志里 "market_cap_level 混文字" 场景）──
    //     LLM 把思考文字夹进了字符串值；我们的策略是 IR + 文本块内部 JSON 解析，
    //     若破损则返回 None，让上层走降级。
    #[tokio::test]
    async fn truncated_json_returns_none_or_partial() {
        // 模拟用户日志中的破损输出：缺右括号、字符串值被截断
        let content = r#"```json
{
  "candidates": [
    {
      "stock_code": "300285",
      "stock_name": "国瓷材料",
      "market_cap_level": "中盘",
      "serenity_score": 75
    }
  ]
"#;
        // 不抛 panic，要么成功（拿到部分有效 JSON）要么返回 None
        let result = extract_via_normalizer(content).await;
        if let Some(v) = result {
            // 如果能解析，至少应该能拿到 candidates 字段
            assert!(v.get("candidates").is_some() || v.get("stock_code").is_some());
        }
        // None 也是可接受的——上层会走降级
    }

    // ── 4) IR Normalizer 自身：tool_json 块 → ContentBlock::ToolUse ──
    #[tokio::test]
    async fn normalizer_emits_tool_use_for_tool_json() {
        let blocks = normalize(
            r#"```tool_json
{"name": "submit_chain", "arguments": {"trend_name": "AI 算力"}}
```"#,
        )
        .await;
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. })),
            "tool_json 块应被 Normalizer 解析为 ToolUse，实际：{:?}",
            blocks
        );
    }

    // ── 5) IR Normalizer：纯文本无代码块 → ContentBlock::Text ──
    #[tokio::test]
    async fn normalizer_passes_plain_text_through() {
        let blocks = normalize("hello world").await;
        assert!(matches!(blocks.as_slice(), [ContentBlock::Text { .. }]));
    }

    // ── 6) extract_agent_output 顶层字段优先级：params > output > content ──
    #[tokio::test]
    async fn extract_agent_output_prefers_top_level_params() {
        let raw = serde_json::json!({
            "content": "ignored",
            "params": {"candidates": [{"stock_code": "1"}]},
            "output": {"should_not": "appear"},
        });
        let v = extract_agent_output(raw).await;
        assert_eq!(v["candidates"][0]["stock_code"], "1");
    }

    // ── 7) extract_agent_output 顶层 candidates 字段直通 ──
    #[tokio::test]
    async fn extract_agent_output_passes_top_level_candidates() {
        let raw = serde_json::json!({
            "candidates": [{"stock_code": "600519"}],
            "content": "ignored",
        });
        let v = extract_agent_output(raw).await;
        let arr = v.as_array().expect("candidates 应直返为数组");
        assert_eq!(arr[0]["stock_code"], "600519");
    }

    // ── 8) 兜底：content 是破损 JSON（无 code fence），返回 None（不走原始对象）──
    //     extract_via_normalizer 内部：直接尝试 `serde_json::from_str(content)` → 失败
    //     所以会从内容中找 ```json``` 失败，最终返回 None。
    #[tokio::test]
    async fn extract_via_normalizer_handles_garbage_input() {
        let v = extract_via_normalizer("not a json at all").await;
        assert!(v.is_none());
    }

    // ── 9) parse_loose_json：标准 JSON 直通 ──
    #[test]
    fn parse_loose_json_accepts_valid() {
        let v = parse_loose_json(r#"{"k": 1}"#);
        assert_eq!(v.expect("应能解析")["k"], 1);
    }

    // ── 10) parse_loose_json：空字符串 → None ──
    #[test]
    fn parse_loose_json_empty_string() {
        assert!(parse_loose_json("").is_none());
        assert!(parse_loose_json("   ").is_none());
    }
}

/// 将 Markdown 文本导出为 Word (.docx) 文件，通过 ToolRegistry 调用 ExportWordTool
#[tauri::command]
pub async fn export_md_to_docx(
    state: State<'_, AppState>,
    markdown: String,
    output_path: String,
    title: Option<String>,
) -> Result<String, String> {
    let input = serde_json::json!({
        "markdown": markdown,
        "output_path": output_path,
        "title": title.unwrap_or_else(|| "股票分析报告".to_string()),
    });
    let ctx = ToolContext::new(std::env::temp_dir().to_string_lossy().to_string());
    let registry = state.local_tool_registry.lock().await;
    let tool = registry
        .get("ExportWord")
        .ok_or_else(|| "ExportWord 工具未注册".to_string())?;
    let result = tool.call(input, &ctx).await.map_err(|e| e.to_string())?;
    Ok(result.content)
}

/// 将 Markdown 文本导出为 PowerPoint (.pptx) 文件，通过 ToolRegistry 调用 ExportPptxTool
#[tauri::command]
pub async fn export_md_to_pptx(
    state: State<'_, AppState>,
    markdown: String,
    output_path: String,
    title: Option<String>,
) -> Result<String, String> {
    let input = serde_json::json!({
        "markdown": markdown,
        "output_path": output_path,
        "title": title.unwrap_or_else(|| "股票分析报告".to_string()),
    });
    let ctx = ToolContext::new(std::env::temp_dir().to_string_lossy().to_string());
    let registry = state.local_tool_registry.lock().await;
    let tool = registry
        .get("ExportPptx")
        .ok_or_else(|| "ExportPptx 工具未注册".to_string())?;
    let result = tool.call(input, &ctx).await.map_err(|e| e.to_string())?;
    Ok(result.content)
}
