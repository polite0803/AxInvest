use super::core::fetch_stock_lessons;
use super::decision::{load_and_inject_template, resolve_runtime_options};
use super::serenity::extract_agent_output;
use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;
use axagent_agent_macro::agent_command;
use axagent_analysis_engine::recommender::Period;
use axagent_astock_data::as_of::{self, AsOfContext};
use axagent_entities::stock_analyses;
use axagent_harness::{ActionKind, normalize_action};
use sea_orm::DatabaseConnection;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use std::sync::Arc;
use tauri::State;

/// 从反思输出中按 key 取值，兼容三种**实测存在**的落库形态。
///
/// 实测依据（`stock_reflections` 11 条 completed，2026-09-23）：
/// - ① 顶层直接命中：`{"verdict": "..."}`
/// - ② 一层嵌套对象：`{"reflection": {"verdict": "..."}}`（1 条，code_diff_proposal 形态）
/// - ③ 内层 JSON 字符串（双重编码）：`{"report": "{\"verdict\": \"partial\", ...}"}`（10 条）
///
/// 旧实现一律 `json.get(key)` 只读顶层 ⇒ ② ③ 全部取不到，
/// 使 `verdict` / `lesson_summary` / `alpha_cited` 恒 NULL，
/// 进而把 `was_correct` 压成 0（详见 PLAN-analysis-data-repair.md 缺陷 D1/D2）。
///
/// 与入口的关系：`extract_agent_output` 已在入口对 `report` 做解包（serenity.rs），
/// 但**历史数据不经入口回灌**，且其它包装键（如 `reflection`）不由入口处理 ⇒
/// 本函数是消费侧的第二道防线，与入口解包**二者不可只留其一**。
fn deep_get_reflection_field(v: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    let obj = v.as_object()?;
    // ① 顶层命中
    if let Some(hit) = obj.get(key) {
        return Some(hit.clone());
    }
    // ② / ③ 下探一层子值
    for child in obj.values() {
        if let Some(inner) = child.as_object() {
            if let Some(hit) = inner.get(key) {
                return Some(hit.clone());
            }
        } else if let Some(parsed) =
            child.as_str().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        {
            if let Some(hit) = parsed.get(key) {
                return Some(hit.clone());
            }
        }
    }
    None
}

/// `deep_get_reflection_field` 的字符串便捷取法（反思字段绝大多数是字符串）。
fn deep_get_reflection_str(v: &serde_json::Value, key: &str) -> Option<String> {
    deep_get_reflection_field(v, key).and_then(|x| x.as_str().map(str::to_string))
}

/// `deep_get_reflection_field` 的「文本」取法：字符串直取，数组 / 对象做 JSON 序列化。
///
/// 为什么需要它：`missed_signals` 实测落库形态是**数组**（`jsonb_typeof = array`），
/// 用 `as_str()` 取会恒为 `None` ⇒ ExperiencePipeline 的 error_patterns 永远缺这一项。
/// 与 `what_went_wrong`（落库为 `string`）不同，二者不能共用同一个取法。
fn deep_get_reflection_text(v: &serde_json::Value, key: &str) -> Option<String> {
    match deep_get_reflection_field(v, key)? {
        serde_json::Value::String(s) => Some(s),
        other => Some(other.to_string()),
    }
}

/// 组装反思 prompt 的「可调参数清单」（v31，v72 收窄）。
///
/// 清单来源 = `PORTFOLIO_MGR_TUNABLE_PARAMS`（单一权威源，与 portfolio-mgr 的
/// `input_mapping` 同源），逐项从变量表 `build_template_variables()` 取默认值与说明。
///
/// 为什么必须要它：`params_suggestion` 的 `param` 字段此前是自由文本，LLM 并不知道
/// 存在 `regime_prior_*` / `action_*` / `risk_*` 这些可调参数，只能凭训练先验自造
/// 名字 → 反思侧对市况先验等参数提出建议的概率≈0，「先验可被反思优化」实际不成立。
///
/// 为什么不用名字前缀扫全变量表（v31 的做法）：那会把 `risk_free_rate` /
/// `risk_hhi_*` / `risk_sharpe_annualization` / `pos_max_turnover_pct` 这类
/// **模型级参数**一并列出（实测 41 项）。这些参数改变的是估值与风险的计算结果，
/// 不移动 portfolio-mgr 的决策边界，列给 LLM 只会稀释注意力、诱发无效建议。
/// 收窄到 28 项后顺序即「决策相关性顺序」（先验 → 行动阈值 → 仓位 → 上限 →
/// 风险分类 → 成本），决策链上游参数优先被 LLM 看到。
fn build_tunable_params_catalog() -> String {
    use crate::commands::stock_analysis_setup::seed_stock_analysis::PORTFOLIO_MGR_TUNABLE_PARAMS;
    use axagent_harness::workflow_types::Variable;
    use std::collections::HashMap;

    let vars = crate::commands::stock_analysis_setup::seed_variables::build_template_variables();
    let by_name: HashMap<&str, &Variable> = vars.iter().map(|v| (v.name.as_str(), v)).collect();

    let mut lines: Vec<String> = Vec::new();
    for name in PORTFOLIO_MGR_TUNABLE_PARAMS {
        let Some(v) = by_name.get(name) else {
            // 权威源登记了、变量表却没定义 —— 运行时该参数必然静默走 rhai 硬编码默认值
            // （「配置项空接线」）。这里显式告警而不是静默跳过，避免再次无声漂移。
            tracing::warn!(param = name, "可调参数未在 seed_variables.rs 定义，已从反思清单剔除");
            continue;
        };
        let default = match &v.value {
            serde_json::Value::Number(n) => n.to_string(),
            _ => "-".to_string(),
        };
        lines.push(format!(
            "- {}（默认 {}）：{}",
            v.name,
            default,
            v.description.as_deref().unwrap_or("")
        ));
    }
    if lines.is_empty() {
        return "（当前模板未声明可调参数，本次不要输出 params_suggestion）".to_string();
    }
    format!("共 {} 项，按决策相关性排序（决策链上游优先）：\n{}", lines.len(), lines.join("\n"))
}

/// 反思复盘工作流：从原始分析的 blackboard_snapshot 记忆中反思。
/// 结果写入独立的 `stock_reflections` 表。
#[allow(clippy::too_many_arguments)]
pub async fn run_reflection_workflow(
    db: &DatabaseConnection,
    client: &axagent_astock_data::AStockClient,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    vector_store: &axagent_search::vector_store::VectorStore,
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
    // [方向3] 轨迹存储，用于持久化反思执行轨迹。
    // 传 None 则跳过 Trajectory 持久化（手动反思等不需要轨迹的场景）。
    trajectory_storage: Option<&std::sync::Arc<axagent_trajectory::TrajectoryStorage>>,
    // [实际行情] 「当前实际行情」快照 —— 由调用方在反思前用前复权 K 线确定性算出
    // （见 `compute_market_snapshot`）。承载价格层事实：入场基准价 / 最新价 /
    // 涨跌幅 / 最大回撤 / 超额收益 / 目标价实现度。
    //
    // None = 行情不可用（K 线获取失败等）⇒ 注入空占位变量，让上游 comparator 与
    // reflection-agent 显式知道"本次无行情事实"，而不是拿到伪造的 0% 收益
    // 把实际涨跌误判成「横盘」。
    market_snapshot: Option<&MarketSnapshot>,
) -> Result<String, String> {
    use axagent_entities::stock_reflections;
    use sea_orm::sea_query::Expr;

    let now_ms = chrono::Utc::now().timestamp_millis();

    // ── [B2 借鉴] 幂等守卫: 如果 reflection_id 已 completed,直接返回 cached ──
    if let Some(ref rid) = reflection_id {
        if let Some(existing) =
            stock_reflections::Entity::find_by_id(rid.clone()).one(db).await.map_err(|e| {
                ErrorResponse::new(wf_err::INTERNAL)
                    .with_detail(format!("B2 查询已存在反思失败: {e}"))
            })?
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
    let analysis_id = reflection_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    if let Some(ref rid) = reflection_id {
        let _ = stock_reflections::Entity::update_many()
            .col_expr(stock_reflections::Column::Status, Expr::value("running"))
            .col_expr(stock_reflections::Column::UpdatedAt, Expr::value(now_ms))
            .filter(stock_reflections::Column::Id.eq(rid.clone()))
            .exec(db)
            .await
            .map_err(|e| {
                ErrorResponse::new(wf_err::INTERNAL)
                    .with_detail(format!("B3 UPDATE pending→running 失败: {e}"))
            })?;
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
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("DB 写入失败: {e}"))
        })?;
    }

    // 2. 加载反思复盘模板（stock-reflection，DAG 结构与 stock-analysis 一致）
    let loaded = load_and_inject_template(db, stock_code, stock_name, "stock-reflection").await?;

    // 注入 vendor 启用状态过滤器（与 stock-analysis 主工作流一致）
    super::decision::inject_vendor_state(client, loaded.variables.as_ref());

    let (max_concurrent, step_timeout, _total_timeout) =
        resolve_runtime_options(loaded.variables.as_deref());

    // 3. 创建嵌套工作流
    let wf_name = format!("stock-reflection-{stock_code}");
    // 统一 with_hooks 模式：模板未来声明钩子时不会静默丢失
    let workflow = engine
        .create_workflow_with_hooks(&wf_name, loaded.nodes, loaded.edges, loaded.hooks_config)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("创建反思工作流失败: {e}"))
        })?;
    let wf_id = workflow.id.clone();

    // 4. 加载原始分析记录：时间维度 + blackboard_snapshot（分析工作流记忆）
    // [v2] 不再通过 sub-analysis SubWorkflowNode 重跑完整 stock-analysis DAG，
    //      而是从 stock_analyses.blackboard_snapshot 加载已保存的分析结果作为记忆，
    //      构造名为 "sub-analysis" 的变量注入工作流。
    //      context_sources / input_mapping 路径（如 sub-analysis.trader.content.action）
    //      保持不变，resolve_var_path 会从注入的变量中按路径下钻。
    //
    // 手动触发时 original_analysis_id="" → 无记忆可加载，注入空对象降级。
    // 但反思 prompt 模板 (reflection.md:17-18) hard-code 引用
    // {{original_time_horizon}} / {{original_holding_days}},所以必须注入占位值
    // (否则 work_engine 报 VARIABLE_NOT_FOUND,reflection-agent 节点 Failed,
    // 数据库 what_went_wrong 等字段全 null)。
    let original_analysis: Option<stock_analyses::Model> = if original_analysis_id.is_empty() {
        None
    } else {
        stock_analyses::Entity::find_by_id(original_analysis_id).one(db).await.ok().flatten()
    };

    let original_ctx: Option<(String, i64)> = original_analysis.as_ref().and_then(|a| {
        let t = a.decision_time_horizon.clone()?;
        let h = a.decision_expected_holding_days?;
        Some((t, h))
    });

    // 4a. 从 blackboard_snapshot 构造 sub-analysis 变量（分析工作流记忆）
    let sub_analysis_memory: serde_json::Value = match &original_analysis {
        Some(analysis) => {
            build_sub_analysis_from_snapshot(analysis.blackboard_snapshot.as_deref(), stock_code)
        },
        None => {
            tracing::warn!(
                "[reflection] {}: 无 original_analysis_id 或记录不存在,注入空 sub-analysis 记忆",
                stock_code
            );
            serde_json::json!({})
        },
    };

    // 5. 注入变量
    let mut variables = vec![
        // [v2] sub-analysis 变量：从 blackboard_snapshot 加载的分析工作流记忆。
        // 替代原 SubWorkflowNode 嵌套重放，避免重跑完整 stock-analysis DAG。
        // reflection-comparator 的 input_mapping (如 sub-analysis.trader.content.action)
        // 和 reflection-agent 的 context_sources 都引用此变量名。
        axagent_harness::workflow_types::Variable {
            name: "sub-analysis".into(),
            var_type: "object".into(),
            value: sub_analysis_memory,
            description: Some(
                "原始股票分析工作流的记忆（从 blackboard_snapshot._raw.* 恢复的节点输出）".into(),
            ),
            is_secret: false,
        },
        // 内联 system_prompt (src/commands/stock_analysis_setup/mod.rs:1525) 引用了
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
        // [时间旅行模式] 注入 hindsight_date 让 reflection-agent LLM 知道评估时点。
        // 工具调用也以此日期为 AS_OF 锚点，查看"截至此日期的实际走势"。
        axagent_harness::workflow_types::Variable {
            name: "hindsight_date".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(hindsight_date.to_string()),
            description: Some("反思评估时点（YYYY-MM-DD），工具调用和数据查看的时间锚点".into()),
            is_secret: false,
        },
        // ── [v31] 可调参数清单 ──
        // reflection-agent 的 system_prompt 以 {{tunable_params_catalog}} 引用。
        // 不注入会让占位符渲染为空（甚至 VARIABLE_NOT_FOUND），反思产出的
        // params_suggestion 就永远只有 LLM 自造的参数名 → 优化链路形同虚设。
        axagent_harness::workflow_types::Variable {
            name: "tunable_params_catalog".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(build_tunable_params_catalog()),
            description: Some("可调决策参数清单（由变量表派生），供 params_suggestion 使用".into()),
            is_secret: false,
        },
        // [C3 借鉴] 4 个结构化 outcome 变量（reflection.md prompt 引用但原未注入）
        // 不注入会导致 VARIABLE_NOT_FOUND 或 LLM 看到空值，影响反思质量。
        axagent_harness::workflow_types::Variable {
            name: "raw_return_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(raw_return.unwrap_or(0.0)),
            description: Some("实际原始收益率百分比（如 -8.0 表示跌 8%）".into()),
            is_secret: false,
        },
        axagent_harness::workflow_types::Variable {
            name: "alpha_return_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(alpha_return.unwrap_or(0.0)),
            description: Some("相对基准的超额收益百分比".into()),
            is_secret: false,
        },
        axagent_harness::workflow_types::Variable {
            name: "holding_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(holding_days.unwrap_or(0)),
            description: Some("实际持仓天数".into()),
            is_secret: false,
        },
        axagent_harness::workflow_types::Variable {
            name: "benchmark_name".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(benchmark_name.unwrap_or("沪深300").to_string()),
            description: Some("对比基准名称（如沪深300/中证500）".into()),
            is_secret: false,
        },
        // 反思 prompt 模板里引用了 {{stock_lessons}},必须显式注入,
        // 否则 work_engine 报 VARIABLE_NOT_FOUND 导致反思节点 Failed。
        // 数据源: 该股最近 3 个月的反思记录(去重排除当前正在创建的记录)。
        // P2-F15: fetch_stock_lessons 返回 (Option<String>, Vec<String>) 元组，
        // .0 是教训文本，.1 是被引用的 lesson_ids（在此场景不写入 lesson_applications，
        // 因为 reflection 工作流不是决策分析，不需要追踪 lesson 应用）。
        axagent_harness::workflow_types::Variable {
            name: "stock_lessons".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(
                fetch_stock_lessons(stock_code, db)
                    .await
                    .0
                    .unwrap_or_else(|| "（暂无历史反思）".to_string()),
            ),
            description: Some("该股历史反思教训（错因/被忽视信号/改进建议）".into()),
            is_secret: false,
        },
    ];

    // ── [实际行情] 「已完成的分析结论」vs「当前实际行情」的价格层事实 ──
    //
    // 与上方 raw_return_pct 的分工：
    //   - raw_return_pct 是**标量**（收益率数字），comparator 用它做方向/收益分类；
    //   - actual_market_text / actual_market_json 是**价格事实**（入场价、最新价、
    //     区间高低、最大回撤、目标价实现度），让反思 LLM 能引用具体数字做归因，
    //     而不是对着一个百分比"反思"。
    //
    // 两个变量必须**无条件注入**：comparator 的 input_mapping 与 reflection-agent
    // 的 system_prompt 都引用它们，缺失会触发 VARIABLE_NOT_FOUND 使整条链 Failed。
    let (market_text, market_json) = match market_snapshot {
        Some(snap) => (snap.render_text(), serde_json::to_value(snap).unwrap_or_default()),
        None => (
            "【当前实际行情】行情数据不可用（K 线获取失败）。\
             本次反思请仅基于定性记忆，不要对收益方向/幅度下结论。"
                .to_string(),
            serde_json::json!({ "status": "unavailable" }),
        ),
    };
    variables.push(axagent_harness::workflow_types::Variable {
        name: "actual_market_text".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(market_text),
        description: Some("当前实际行情文本块（价格/涨跌/回撤/超额/目标价实现度）".into()),
        is_secret: false,
    });
    variables.push(axagent_harness::workflow_types::Variable {
        name: "actual_market_json".into(),
        var_type: "object".into(),
        value: market_json,
        description: Some("当前实际行情结构化快照（供 comparator 按路径下钻）".into()),
        is_secret: false,
    });

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
        // [v2] 不再有 sub-analysis SubWorkflowNode，无需为子工作流传 input。
        // stock_code / stock_name / as_of_date 已通过 variables 顶层注入。
        input: None,
        input_schema: loaded.input_schema,
        output_schema: loaded.output_schema,
        dry_run: false,
        variables: Some(variables),
        // 接线激活 strict_mode：VERDICT 缺失兜底重试 / strict JSON 校验与降级
        tool_permissions: Some(super::strict_tool_permissions()),
        ..Default::default()
    };

    // [时间旅行模式] 用 hindsight_date 作为 AS_OF 锚点包装工作流执行。
    // reflection-agent 调用的 K 线/公告工具会以 hindsight_date 为时间锚点，
    // 查看"截至评估时点的实际走势"，而非今天的全部数据。
    // - as_of_date 是原始分析日期（记忆锚点）
    // - hindsight_date 是反思评估时点（工具调用锚点）
    // 二者解耦：分析记忆从 blackboard_snapshot 加载（无 AS_OF），工具调用走 AS_OF(hindsight_date)
    let hindsight_ctx = AsOfContext::parse(hindsight_date).map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("hindsight_date 解析失败: {e}"))
    })?;

    let result = as_of::AS_OF
        .scope(Some(hindsight_ctx), async move { engine.run_workflow(&wf_id, opts).await })
        .await;

    // 6. 处理结果
    match result {
        Ok(wf) => {
            // 通过 extract_agent_output 管线提取规范化 JSON（兼容多模型输出格式）
            let reflection_raw =
                wf.results.get("reflection").cloned().unwrap_or(serde_json::Value::Null);
            let reflection_json = extract_agent_output(reflection_raw).await;
            // 兜底与字段提取统一走 `deep_get_reflection_*`：它们自带
            // 「顶层 / 一层嵌套 / 双重编码字符串」三形态容错。因此原先此处手写的
            // 「as_object() 失败则二次 parse」与「reflection 子对象 lookup」两段特殊处理
            // 已无必要 —— 且两者都漏了双重编码，正是 D1 的成因。
            //
            // 三种实际输出结构（实测见 deep_get_reflection_field 注释）：
            //   A) 直接:     {what_went_wrong, missed_signals, fix_for_future, params_suggestion}
            //   B) 嵌套:     {reflection: {what_went_wrong, ...}, params_suggestion}
            //   C) 双重编码: {report: "{\"what_went_wrong\": ...}"}
            // 内联 system_prompt 要求 A，reflection.md 外部 expert prompt 要求 B，
            // 而库内实测 C 占 10/11 ⇒ 三者都必须容错。
            let what_went_wrong = deep_get_reflection_str(&reflection_json, "what_went_wrong");
            let missed_signals = deep_get_reflection_text(&reflection_json, "missed_signals");
            let fix_for_future = deep_get_reflection_str(&reflection_json, "fix_for_future");
            let params_suggestion_json =
                deep_get_reflection_text(&reflection_json, "params_suggestion");

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
                .col_expr(
                    stock_reflections::Column::FixForFuture,
                    // [方向2] fix_for_future 在此处被 move,提前 clone 一份供 ExperiencePipeline 用
                    Expr::value(fix_for_future.clone()),
                )
                .col_expr(
                    stock_reflections::Column::ParameterSuggestionsJson,
                    Expr::value(params_suggestion_json.clone()),
                )
                .col_expr(stock_reflections::Column::BlackboardSnapshot, Expr::value(bb_text))
                // v008 (C2 借鉴): 回写 verdict / alpha_cited / lesson_summary
                // D1 修复：三个字段改走 deep_get_reflection_field 兼容
                // 「一层嵌套」与「双重编码 JSON 字符串」两种实际落库形态。
                .col_expr(
                    stock_reflections::Column::Verdict,
                    Expr::value(deep_get_reflection_str(&reflection_json, "verdict")),
                )
                .col_expr(
                    stock_reflections::Column::AlphaCited,
                    Expr::value(deep_get_reflection_str(&reflection_json, "alpha_cited")),
                )
                .col_expr(
                    stock_reflections::Column::LessonSummary,
                    Expr::value(deep_get_reflection_str(&reflection_json, "lesson_summary")),
                )
                // D10 修复：raw_return / alpha_return / holding_days 此前只注入提示词变量、
                // 只写入 strategy_performance，**从未回写 stock_reflections 自身** ⇒
                // 三列实测 11/11 全 NULL。而同源的 strategy_performance.return_pct
                // （下方写入时取自同一个 raw_return）却有真值（-23.90% ~ +5.32%）
                // —— 这组「同源不同落库」就是缺口存在的直接证据。
                // 回归判据：本次反思后，三列应与 strategy_performance 对应行一致。
                .col_expr(stock_reflections::Column::RawReturn, Expr::value(raw_return))
                .col_expr(stock_reflections::Column::AlphaReturn, Expr::value(alpha_return))
                .col_expr(stock_reflections::Column::HoldingDays, Expr::value(holding_days))
                .filter(stock_reflections::Column::Id.eq(&analysis_id))
                .exec(db)
                .await;

            // ── Path 2: 反思参数建议自动解析 ──
            // D1 修复：verdict 改走 deep_get（兼容「一层嵌套」与「双重编码字符串」）。
            let verdict_opt = deep_get_reflection_str(&reflection_json, "verdict");
            let verdict_str = verdict_opt.as_deref().unwrap_or("");

            // ── Gap 1: 确定性判定 → Strategy Performance 自动写入 ──
            // M1 改写：was_correct 不再由反思 agent 的 verdict（LLM 自评）映射，
            // 改由行情快照**确定性反推**（deterministic_was_correct，见下方纯函数）。
            // verdict 仍写 stock_reflections 供反思阅读；但判胜依据（weight_decay.rs:79
            // 以 `was_correct != 0` 判胜）改用客观符号判定，消除 LLM 幻觉污染。
            //
            // 保留 D2 语义：「未判定」与「判定为错」在数据上可区分 ——
            // 无法判定（无行情 / 中性档 / 期中观察）→ **不写行**：
            // strategy_performance.was_correct 是 i32 非空列，表达不了第三态，
            // 而"没写入"本身就是"没有判定依据"的正确表达。
            let was_correct: Option<i32> = deterministic_was_correct(
                original_analysis.as_ref().and_then(|a| a.decision_action.as_deref()),
                market_snapshot,
            );

            if let Some(was_correct) = was_correct {
                use axagent_entities::strategy_performance;
                let sp_id = uuid::Uuid::new_v4().to_string();
                let decision_at = now_ms - (holding_days.unwrap_or(30) as i64 * 86_400_000);
                // D3 修复：decision_confidence 不再硬编码 0。
                // 来源 = 原分析的决策置信度（`decision_json.confidence`，回退 `decisionConfidence`），
                // 实测量纲已是 0-100（min 0 / max 75，非 0-1 小数）。
                // ⚠ 已知残留：该列为 i32 非空，取不到置信度时只能落 0，
                //   与"置信度极低"不可区分 —— 属性缺口，已在 PLAN 登记。
                let decision_confidence = original_analysis
                    .as_ref()
                    .and_then(|a| a.decision_json.as_deref())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                    .and_then(|dj| {
                        dj.get("confidence")
                            .or_else(|| dj.get("decisionConfidence"))
                            .and_then(|v| v.as_f64())
                    })
                    .map(|c| c.round().clamp(0.0, 100.0) as i32)
                    .unwrap_or(0);
                let sp_insert = strategy_performance::ActiveModel {
                    id: Set(sp_id.clone()),
                    strategy_id: Set("reflection_verdict".to_string()),
                    period: Set("reflection".to_string()),
                    stock_code: Set(stock_code.to_string()),
                    stock_name: Set(stock_name.to_string()),
                    decision_at: Set(decision_at),
                    exit_at: Set(now_ms),
                    holding_days: Set(holding_days.unwrap_or(30)),
                    return_pct: Set(raw_return.unwrap_or(0.0)),
                    was_correct: Set(was_correct),
                    decision_confidence: Set(decision_confidence),
                    horizon_pnl_json: Set(None),
                    agreement_score: Set(None),
                    created_at: Set(now_ms),
                }
                .insert(db)
                .await;
                match sp_insert {
                    Ok(_) => tracing::info!(
                        "[reflection] Gap1: 写入 strategy_performance {sp_id}: \
                         verdict={verdict_str} was_correct(deterministic)={was_correct} \
                         decision_confidence={decision_confidence}"
                    ),
                    Err(e) => tracing::warn!("[reflection] 写入 strategy_performance 失败: {e}"),
                }
            } else {
                // 无判定依据（无行情 / 中性档 / 期中观察）⇒ 不写 strategy_performance，
                // 避免把「无法判定」计为「判定为错」而污染胜率统计。
                tracing::warn!(
                    "[reflection] {}: 无判定依据，跳过 strategy_performance 写入\
                     （避免把「无判定」计为「判定为错」）",
                    stock_code
                );
            }

            // ── Gap 3: 攒够 N 条一致建议自动触发 WFO 校准 ──
            // 当连续 3+ 条反思对某个参数提出同方向调整时，自动跑校准。
            if verdict_str == "wrong" || verdict_str == "partial" {
                if let Some(ref pj) = params_suggestion_json {
                    use axagent_analysis_engine::portfolio_formula::try_parse_param_suggestion;
                    if let Some(suggested) = try_parse_param_suggestion(pj) {
                        tracing::info!(
                            "[reflection] Gap3: 解析到参数建议 buy={:.2} capHi={:.0}, 检查一致性...",
                            suggested.buy_threshold,
                            suggested.cap_high
                        );
                        // 查询最近 10 条有参数建议的反思
                        use axagent_entities::stock_reflections as sr;
                        use sea_orm::QuerySelect;
                        let recent = sr::Entity::find()
                            .filter(sr::Column::Status.eq("completed"))
                            .filter(sr::Column::ParameterSuggestionsJson.is_not_null())
                            .order_by(sr::Column::CreatedAt, sea_orm::Order::Desc)
                            .limit(10)
                            .all(db)
                            .await;
                        if let Ok(rows) = recent {
                            let mut same_direction = 1; // 当前这条算 1
                            for r in &rows {
                                if r.id == analysis_id {
                                    continue;
                                }
                                if let Some(pj2) = r.parameter_suggestions_json.as_deref() {
                                    if let Some(prev) = try_parse_param_suggestion(pj2) {
                                        // 检查 buy_threshold 的调整方向是否一致
                                        let def = axagent_analysis_engine::portfolio_formula::PortfolioMgrParamSet::v56_default();
                                        let current_dir =
                                            suggested.buy_threshold < def.buy_threshold;
                                        let prev_dir = prev.buy_threshold < def.buy_threshold;
                                        if current_dir == prev_dir {
                                            same_direction += 1;
                                        } else {
                                            break; // 方向不同就停止计数
                                        }
                                        if same_direction >= 3 {
                                            tracing::info!(
                                                "[reflection] Gap3: 连续 {same_direction} 条建议降低 buy_threshold, \
                                                 自动触发 WFO 校准"
                                            );
                                            // 为了避免异步阻塞 reflection 主流程，只记录不实际执行
                                            // 实际自动校准由 scheduler/cron 层接管
                                            break;
                                        }
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
            }

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

            // ── [方向3] 持久化反思轨迹到 TrajectoryStorage ──
            // 为后续的 ExperiencePipeline（方向2）和 DreamConsolidator（方向6）提供数据基础。
            // 从反思结果中提取 verdict / lesson_summary / what_went_wrong 构造 Trajectory，
            // 用 TrajectoryScorer 自动计算 quality 和 value_score。
            if let Some(storage) = trajectory_storage {
                use axagent_harness::trajectory_scorer::TrajectoryScorer;
                use axagent_harness::trajectory_types::{
                    MessageRole, Trajectory, TrajectoryOutcome, TrajectoryStep,
                };

                let verdict_opt = deep_get_reflection_str(&reflection_json, "verdict");
                let verdict_str = verdict_opt.as_deref();
                let outcome = match verdict_str {
                    Some("correct") => TrajectoryOutcome::Success,
                    Some("partial") => TrajectoryOutcome::Partial,
                    Some("wrong") => TrajectoryOutcome::Failure,
                    _ if status_text.starts_with("failed") => TrajectoryOutcome::Abandoned,
                    _ => TrajectoryOutcome::Partial,
                };

                let lesson =
                    deep_get_reflection_str(&reflection_json, "lesson_summary").unwrap_or_default();
                let reasoning_text = what_went_wrong.as_deref().unwrap_or("");
                let duration_ms = (chrono::Utc::now().timestamp_millis() - now_ms).max(0) as u64;

                let steps = vec![
                    TrajectoryStep {
                        timestamp_ms: now_ms.max(0) as u64,
                        role: MessageRole::User,
                        content: format!(
                            "反思 {} ({}) 预测时间={} 评估时间={} 实际={}",
                            stock_code, stock_name, as_of_date, hindsight_date, actual_outcome
                        ),
                        reasoning: None,
                        tool_calls: None,
                        tool_results: None,
                    },
                    TrajectoryStep {
                        timestamp_ms: duration_ms,
                        role: MessageRole::Assistant,
                        content: lesson.to_string(),
                        reasoning: if reasoning_text.is_empty() {
                            None
                        } else {
                            Some(reasoning_text.to_string())
                        },
                        tool_calls: None,
                        tool_results: None,
                    },
                ];

                let mut trajectory = Trajectory::new(
                    analysis_id.clone(),
                    // [方向6] topic 包含股票代码，让 DreamConsolidator 按股票分组蒸馏
                    format!("stock_reflection:{}", stock_code),
                    format!("{} {} 反思", stock_code, stock_name),
                    lesson.to_string(),
                    outcome,
                    duration_ms,
                    steps,
                );
                TrajectoryScorer::apply(&mut trajectory);

                if let Err(e) = storage.save_trajectory(&trajectory).await {
                    tracing::warn!("[reflection] 保存 trajectory 失败: {e}");
                } else {
                    tracing::info!(
                        "[reflection] trajectory {} 已持久化 (outcome={:?} quality={:.2} value={:.2})",
                        &trajectory.id,
                        outcome,
                        trajectory.quality.overall,
                        trajectory.value_score
                    );
                }
            }

            // ── [F1 借鉴] 反思完成后自动提取 lesson 为可重用规则 ──
            // 借鉴 TradingAgents 反思→规则提取机制:反思完成后把 lesson_summary
            // 提取为可重用的规则存入 reflection_lessons 表,下次决策可查询。
            if status_text == "completed" {
                // D1 修复：lesson_summary / verdict 走 deep_get ——
                // 旧实现取不到 ⇒ 该条件永不满足 ⇒ reflection_lessons 恒 0 行（PLAN D5）。
                if let Some(ls) = deep_get_reflection_str(&reflection_json, "lesson_summary") {
                    let verdict_for_rule = deep_get_reflection_str(&reflection_json, "verdict");
                    let _ = extract_lesson_to_rule(
                        db,
                        stock_code,
                        &analysis_id,
                        &ls,
                        verdict_for_rule.as_deref(),
                    )
                    .await;
                }
            }

            // ── [方向2] 接入 ExperiencePipeline，把反思转为 Experience 喂给 RLOptimizer ──
            // 设计要点:
            // - 用 verdict 映射 quality_score:correct=9, partial=5, wrong=2, 其他=4
            // - 异步提交,不阻塞反思主流程
            // - 失败不影响反思结果(只记 warn 日志)
            {
                use axagent_agent::Reflection;

                let verdict_opt = deep_get_reflection_str(&reflection_json, "verdict");
                let verdict_str = verdict_opt.as_deref();
                let quality_score: u8 = match verdict_str {
                    Some("correct") => 9,
                    Some("partial") => 5,
                    Some("wrong") => 2,
                    _ => 4,
                };

                let lesson_summary =
                    deep_get_reflection_str(&reflection_json, "lesson_summary").unwrap_or_default();
                let what_went_wrong_text = what_went_wrong.clone().unwrap_or_default();
                // missed_signals 落库是数组 ⇒ 必须用 text 取法（见 deep_get_reflection_text 注释）。
                let missed = deep_get_reflection_text(&reflection_json, "missed_signals")
                    .unwrap_or_default();
                let fix_text = fix_for_future.clone().unwrap_or_default();

                let mut error_patterns: Vec<String> = Vec::new();
                if !what_went_wrong_text.is_empty() {
                    error_patterns.push(what_went_wrong_text.clone());
                }
                if !missed.is_empty() {
                    error_patterns.push(missed.to_string());
                }
                let mut improvements: Vec<String> = Vec::new();
                if !fix_text.is_empty() {
                    improvements.push(fix_text.clone());
                }

                let quality_analysis = format!(
                    "verdict={:?} stock={} hindsight={} actual={}",
                    verdict_str, stock_code, hindsight_date, actual_outcome
                );

                let reflection = Reflection::new(analysis_id.clone())
                    .with_quality(quality_score, quality_analysis)
                    .with_patterns(error_patterns.clone(), Vec::new())
                    .with_improvements(improvements.clone())
                    .with_summary(lesson_summary.to_string());

                let pipeline = crate::commands::_shared_state::SHARED_PIPELINE.clone();
                let aid = analysis_id.clone();
                tokio::task::spawn(async move {
                    let mut pipeline = pipeline.write().await;
                    let exp = pipeline.process_reflection(&reflection).await;
                    tracing::info!(
                        "[reflection] ExperiencePipeline: 已吸收 reflection {} -> reward={:.3} done={}",
                        aid,
                        exp.reward,
                        exp.done
                    );
                });
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
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description =  "批量处理持仓到期反思")]
#[tauri::command]
pub async fn run_batch_reflection(
    state: State<'_, AppState>,
    max_count: Option<u32>,
) -> Result<serde_json::Value, String> {
    use axagent_entities::stock_analyses;
    use axagent_entities::stock_reflections;

    let max_count = max_count.unwrap_or(20) as usize;
    let db = state.harness.db();

    // 1. 扫所有 pending row,按 created_at ASC(最老的先处理,避免积压)
    let pendings: Vec<stock_reflections::Model> = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.eq("pending"))
        .order_by_asc(stock_reflections::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("D1 扫 pending row 失败: {e}"))
        })?;

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
        let analysis =
            match stock_analyses::Entity::find_by_id(&p.original_analysis_id).one(db).await {
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
        let expected_days = analysis.decision_expected_holding_days.unwrap_or(28);
        let analysis_date = analysis.as_of_date.as_deref().unwrap_or(&p.as_of_date);

        // [时间旅行模式] 评估时点由 pending row 的 hindsight_date 决定。
        // - hindsight_date 在未来 → 还没到反思时点，skip
        // - hindsight_date <= today → 可以反思，传给 run_reflection_workflow
        //   作为 AS_OF 锚点查看"截至评估时点的实际走势"
        // - days_held 基于 hindsight_date - analysis_date 计算
        let hindsight_date = p.hindsight_date.as_str();
        // P3-#13 修复：时区错位
        // 原实现用 `chrono::Utc::now().timestamp_millis()` 与 `NaiveDate.and_utc().timestamp_millis()`
        // 比较和相减，会因 UTC vs Asia/Shanghai 8 小时偏差导致跨日 days_held 计算偏少 1 天。
        // 例：北京时间 2026-07-14 02:00 = UTC 2026-07-13 18:00；若 analysis_date="2026-07-13"，
        //     hindsight_date="2026-07-14"，原实现 (hindsight_ms - analysis_ms) / 86400000 = 0，
        //     实际应为 1 天。
        // 修复：用 NaiveDate 直接相减，彻底绕开时区转换。today 也按 Asia/Shanghai 时区取 NaiveDate。
        let analysis_nd = chrono::NaiveDate::parse_from_str(analysis_date, "%Y-%m-%d").ok();
        let hindsight_nd = chrono::NaiveDate::parse_from_str(hindsight_date, "%Y-%m-%d").ok();

        // today 按 Asia/Shanghai 时区取 NaiveDate（A 股交易日历以北京时间为准）
        let today_nd = {
            use chrono::TimeZone;
            // FixedOffset 8 小时 = Asia/Shanghai（chrono 内置无 IANA 时区数据库依赖）
            let offset = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
            offset.from_utc_datetime(&chrono::Utc::now().naive_utc()).date_naive()
        };

        // [实际行情] 原「hindsight_date 在未来 ⇒ skip」已移除：反思改为以
        // **执行时的最新行情**为对比基准，计划评估时点只作提示（下方仅记录日志）。
        if let Some(h) = hindsight_nd {
            if h > today_nd {
                tracing::info!(
                    "[D1] pending {} ({}) 计划评估时点 {} 未到,按最新行情提前反思",
                    p.id,
                    p.stock_code,
                    hindsight_date
                );
            }
        }

        // days_held = hindsight_date - analysis_date（日历日相减，无时区偏差）
        let days_held = match (analysis_nd, hindsight_nd) {
            (Some(a), Some(h)) => (h - a).num_days().max(0),
            // 解析失败时回退到 timestamp_ms 计算（保留旧行为兼容脏数据）
            _ => {
                let analysis_ms = chrono::NaiveDate::parse_from_str(analysis_date, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|dt| dt.and_utc().timestamp_millis())
                    .unwrap_or(p.created_at);
                let hindsight_ms = chrono::NaiveDate::parse_from_str(hindsight_date, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|dt| dt.and_utc().timestamp_millis())
                    .unwrap_or(today_ms);
                (hindsight_ms - analysis_ms).max(0) / 86_400_000
            },
        };

        // [实际行情] 不再用「期望持有期」做硬门槛：反思现在以「当前实际行情」
        // 为对比基准，未到期也可做**期中观察**（快照会带 `within_expected_horizon`
        // 标记，反思 prompt 明确要求对期中观察降低结论权重）。
        // 真正的门槛下移到行情侧：分析日之后至少要有 1 个交易日的新行情。
        let within_horizon = days_held < expected_days;
        if within_horizon {
            tracing::info!(
                "[D1] pending {} ({}) 持仓 {}/{} 天,未到期,按期中观察反思",
                p.id,
                p.stock_code,
                days_held,
                expected_days
            );
        }

        // 2c. [实际行情] 拉取「分析日 → 最新交易日」的真实行情（前复权 K 线，确定性计算）。
        //     必须在调用前算完：run_reflection_workflow 只负责注入变量，不负责取数。
        let target_price = extract_target_price(&analysis);
        let snapshot = match compute_market_snapshot(
            &state.astock_client,
            &p.stock_code,
            analysis_date,
            analysis.decision_expected_holding_days,
            target_price,
        )
        .await
        {
            Ok(s) if s.trading_days >= 1 => Some(s),
            Ok(s) => {
                tracing::info!(
                    "[D1] pending {} ({}) 分析日之后仅 {} 个交易日,行情样本不足 skip",
                    p.id,
                    p.stock_code,
                    s.trading_days
                );
                skipped_young += 1;
                continue;
            },
            Err(e) => {
                tracing::warn!(
                    "[D1] pending {} ({}) 行情快照失败,降级为无行情反思: {e}",
                    p.id,
                    p.stock_code
                );
                None
            },
        };

        // actual_outcome 改为**事实描述**（价格从哪到哪、涨跌多少），
        // 而不是旧实现的 "correct"/"wrong" 结论词 —— 结论应由反思 agent 给出。
        let actual_outcome = snapshot
            .as_ref()
            .map(|s| s.render_outcome_short())
            .unwrap_or_else(|| p.actual_outcome.clone());
        let today_str = today_nd.format("%Y-%m-%d").to_string();

        // 2d. 调 run_reflection_workflow(B3 UPDATE 路径)
        let r = run_reflection_workflow(
            db,
            &state.astock_client,
            &state.work_engine,
            &state.vector_store,
            state.harness.master_key(),
            &p.stock_code,
            &p.stock_name,
            &p.original_analysis_id,
            &actual_outcome,
            // [修复] 原实现此处传 None —— pending 阶段未回测 ⇒ raw_return_pct 注入 0.0
            // ⇒ comparator 里 actual_direction 恒「横盘」、direction_match 恒 false
            // ⇒ agent 基于假数据反思。现改用行情快照的净收益（扣双边成本）。
            snapshot.as_ref().map(|s| s.net_return_pct),
            snapshot.as_ref().and_then(|s| s.alpha_pct),
            Some(snapshot.as_ref().map(|s| s.trading_days as i32).unwrap_or(days_held as i32)),
            snapshot.as_ref().map(|_| "沪深300"),
            analysis_date,
            // [实际行情] 行情终点是「最新交易日」⇒ AS_OF 锚点必须用**今天**，
            // agent 的 K 线工具才能看到最新数据。原传 pending 的 hindsight_date
            // 会把 agent 的时间锚点锁在过去，看不见"当前实际行情"。
            &today_str,
            // [2026-09-13] 消费 pending row 自带的阈值与深度。
            // 此前这里是硬编码 `0u8` / `"light"`，于是 `stock_reflections` 的
            // `min_confidence_threshold` / `reflection_depth` 成了**死字段** ——
            // 用户在反思面板设的阈值与深度永远不生效（同构问题见铁律 12）。
            p.min_confidence_threshold.clamp(0, 255) as u8,
            p.reflection_depth.as_str(),
            Some(p.id.clone()),              // [B2/B3] 走 UPDATE 路径
            Some(&state.trajectory_storage), // [方向3] 持久化轨迹
            snapshot.as_ref(),               // [实际行情] 价格层事实
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
    //
    // P3-#10 修复：原实现一次 DELETE 可能影响数十万条 row,阻塞 SQLite WAL。
    // 改为分批循环：每批 SELECT 1000 个 id → DELETE WHERE id IN(...)，
    // 直到无超龄 row。单批事务短，避免锁表。
    use sea_orm::QuerySelect;
    let ninety_days_ago_ms = today_ms - 90 * 86_400_000;
    let mut cleaned_up: u64 = 0;
    loop {
        // 取 1000 条超龄 completed row 的 id（按 updated_at ASC 优先删最老的）
        let stale_ids: Vec<String> = stock_reflections::Entity::find()
            .select_only()
            .column(stock_reflections::Column::Id)
            .filter(stock_reflections::Column::Status.eq("completed"))
            .filter(stock_reflections::Column::UpdatedAt.lt(ninety_days_ago_ms))
            .order_by_asc(stock_reflections::Column::UpdatedAt)
            .limit(1000)
            .into_tuple()
            .all(db)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("[D2] FIFO 查询超龄 row 失败: {e}");
                Vec::new()
            });
        if stale_ids.is_empty() {
            break;
        }
        let batch_size = stale_ids.len() as u64;
        let _ = stock_reflections::Entity::delete_many()
            .filter(stock_reflections::Column::Id.is_in(stale_ids))
            .exec(db)
            .await
            .map(|r| {
                cleaned_up += r.rows_affected;
            })
            .map_err(|e| {
                tracing::warn!("[D2] FIFO 批量删除失败: {e}");
            });
        // 若本批不足 1000 条,说明已无超龄 row,退出避免无限循环
        if batch_size < 1000 {
            break;
        }
    }
    tracing::info!("[D2 fifo_cleanup] 分批删除 {} 条超龄 completed row", cleaned_up);

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
    use axagent_entities::reflection_lessons;
    use sea_orm::ActiveModelTrait;
    use sea_orm::ColumnTrait;
    use sea_orm::EntityTrait;
    use sea_orm::QueryFilter;
    use sea_orm::Set;

    // 短文本过短或无实际建议性内容则跳过
    let trimmed = lesson_summary.trim();
    if trimmed.len() < 10 || trimmed.len() > 250 {
        return Ok(());
    }

    // [P2-#9 修复] 去重：检查相同 lesson_summary 是否已存在
    // 同一只股票多次相似反思会产生大量重复规则，此处按 stock_code + lesson_summary 去重。
    // 若已存在，更新 source_reflection_id 和 updated_at（保留原有 times_applied/success_count）。
    let existing = reflection_lessons::Entity::find()
        .filter(reflection_lessons::Column::StockCode.eq(stock_code))
        .filter(reflection_lessons::Column::LessonSummary.eq(trimmed))
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("F1 查询重复 lesson 失败: {e}"))
                .to_string()
        })?;

    if let Some(existing_model) = existing {
        // 已存在相同规则，更新 source_reflection_id 和 updated_at，保留应用统计
        let mut active: reflection_lessons::ActiveModel = existing_model.into();
        active.source_reflection_id = Set(Some(source_reflection_id.to_string()));
        active.updated_at = Set(chrono::Utc::now().timestamp_millis());
        active.update(db).await.map(|_| ()).map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("F1 更新重复 lesson 失败: {e}"))
                .to_string()
        })?;
        tracing::debug!("[F1] lesson_summary 已存在，更新 source_reflection_id: {}", trimmed);
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
    .map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL)
            .with_detail(format!("F1 写入 reflection_lessons 失败: {e}"))
            .to_string()
    })
}

// ── [P1-#5 修复] 反思规则有效性验证 ──
//
// 接入原死代码 `reflection_lesson_validator.rs`（adjust_lesson_confidence 等函数）。
// 追踪 reflection_lessons 表中规则被引用后的决策表现，调整 confidence。
// 适合作为 cron 任务定期执行（如每日一次）。
//
// 验证维度：
// - 规则被引用次数（times_applied）：通过 stock_reflections 中 lesson_summary 模糊匹配
// - 引用后决策成功率（success_count）：基于 stock_analyses 表的 posterior 字段
// - 规则置信度衰减/提升：基于实际表现调整 confidence
//
// 调用方式：cron 调度器或 Tauri 命令 `run_lesson_validation_command`。
// 已接入 start_background_services 的 start_lesson_validation 定时任务。
pub async fn run_lesson_validation(
    db: &sea_orm::DatabaseConnection,
) -> Result<serde_json::Value, String> {
    use axagent_analysis_engine::reflection_lesson_validator::{
        build_lesson_validation, build_lesson_validation_report,
    };
    use axagent_entities::lesson_applications;
    use axagent_entities::reflection_lessons;
    use axagent_entities::stock_reflections;

    // 0. P2-F15 预处理：同步 lesson_applications.outcome_at_validation
    // 扫描所有 outcome_at_validation IS NULL 的行，从 stock_analyses.outcome 回写。
    // 确保 success_count 统计尽可能精确。
    let synced = super::core::sync_lesson_application_outcomes(db).await;
    if synced > 0 {
        tracing::info!(
            "[lesson-validation] 预处理: 从 stock_analyses.outcome 回写 {synced} 条 lesson_applications"
        );
    }

    // 1. 加载所有 active 规则
    let lessons: Vec<reflection_lessons::Model> = reflection_lessons::Entity::find()
        .filter(reflection_lessons::Column::Status.eq("active"))
        .all(db)
        .await
        .map_err(|e| format!("加载 reflection_lessons 失败: {e}"))?;

    tracing::info!("[lesson-validation] 加载 {} 条 active 规则", lessons.len());

    let mut validations = Vec::new();
    let mut updated_count = 0u32;
    // P2-F15 统计：精确统计 vs 模糊匹配的使用情况
    let mut precise_count = 0u32;
    let mut fallback_count = 0u32;

    for lesson in &lessons {
        // ── P2-F15 切入点 3：优先用 lesson_applications 精确统计 ──
        // 旧的 lesson_summary.contains() 模糊匹配存在误匹配/漏匹配问题，
        // 且统计的是"反思时提到该 lesson 的次数"而非"决策时应用了该 lesson 的次数"。
        // 现在优先用 lesson_applications 表精确统计 times_applied。
        //
        // success_count 优先用 lesson_applications.outcome_at_validation = 'win' 精确统计；
        // 如果所有 outcome_at_validation 都是 NULL（outcome 链路未打通），
        // 回退到旧的 stock_reflections.verdict 模糊匹配，避免误判所有规则为 0 成功率。
        let apps: Vec<lesson_applications::Model> = lesson_applications::Entity::find()
            .filter(lesson_applications::Column::LessonId.eq(&lesson.id))
            .all(db)
            .await
            .unwrap_or_default();

        let (applied_count, success_count, used_precise) = if !apps.is_empty() {
            // 精确统计路径
            let precise_applied = apps.len() as i32;
            // 统计 outcome_at_validation = 'win' 的数量
            let precise_success =
                apps.iter().filter(|a| a.outcome_at_validation.as_deref() == Some("win")).count()
                    as i32;

            // 检查是否有任何 outcome_at_validation 已被填充
            let has_any_outcome = apps.iter().any(|a| a.outcome_at_validation.is_some());

            if has_any_outcome {
                // outcome 链路已打通，完全使用精确统计
                (precise_applied, precise_success, true)
            } else {
                // outcome_at_validation 全部为 NULL（链路未打通）
                // times_applied 用精确统计，success_count 回退到模糊匹配
                let fallback_success = stock_reflections::Entity::find()
                    .filter(
                        stock_reflections::Column::LessonSummary.contains(&lesson.lesson_summary),
                    )
                    .filter(stock_reflections::Column::Status.eq("completed"))
                    .filter(stock_reflections::Column::Verdict.is_in(vec!["correct", "partial"]))
                    .all(db)
                    .await
                    .map(|v| v.len() as i32)
                    .unwrap_or(0);
                (precise_applied, fallback_success, true)
            }
        } else {
            // lesson_applications 表中无记录（旧数据或未接入），
            // 完全回退到旧的模糊匹配逻辑
            let fuzzy_applied = stock_reflections::Entity::find()
                .filter(stock_reflections::Column::LessonSummary.contains(&lesson.lesson_summary))
                .filter(stock_reflections::Column::Status.eq("completed"))
                .all(db)
                .await
                .map(|v| v.len() as i32)
                .unwrap_or(0);

            let fuzzy_success = stock_reflections::Entity::find()
                .filter(stock_reflections::Column::LessonSummary.contains(&lesson.lesson_summary))
                .filter(stock_reflections::Column::Status.eq("completed"))
                .filter(stock_reflections::Column::Verdict.is_in(vec!["correct", "partial"]))
                .all(db)
                .await
                .map(|v| v.len() as i32)
                .unwrap_or(0);

            (fuzzy_applied, fuzzy_success, false)
        };

        if used_precise {
            precise_count += 1;
        } else {
            fallback_count += 1;
        }

        // 4. 构建验证记录
        let validation = build_lesson_validation(
            lesson.id.clone(),
            lesson.lesson_summary.clone(),
            lesson.source_reflection_id.clone().unwrap_or_default(),
            lesson.stock_code.clone(),
            applied_count,
            success_count,
            lesson.confidence,
        );
        validations.push(validation.clone());

        // 5. 更新 reflection_lessons 表的 times_applied/success_count/confidence
        let new_status = if validation.adjusted_confidence < 0.2 {
            "deprecated"
        } else {
            "active"
        };

        let _ = reflection_lessons::Entity::update_many()
            .col_expr(
                reflection_lessons::Column::TimesApplied,
                sea_orm::sea_query::Expr::value(applied_count),
            )
            .col_expr(
                reflection_lessons::Column::SuccessCount,
                sea_orm::sea_query::Expr::value(success_count),
            )
            .col_expr(
                reflection_lessons::Column::Confidence,
                sea_orm::sea_query::Expr::value(validation.adjusted_confidence),
            )
            .col_expr(
                reflection_lessons::Column::Status,
                sea_orm::sea_query::Expr::value(new_status),
            )
            .filter(reflection_lessons::Column::Id.eq(&lesson.id))
            .exec(db)
            .await;

        updated_count += 1;
    }

    // 6. 生成验证报告
    let report = build_lesson_validation_report(&validations);

    tracing::info!(
        "[lesson-validation] 完成: validated={} deprecated={} avg_success_rate={:.2} | 精确统计={} 模糊回退={}",
        report.validated_lessons,
        report.deprecated_lessons,
        report.avg_success_rate,
        precise_count,
        fallback_count
    );

    Ok(serde_json::json!({
        "totalLessons": report.total_lessons,
        "validatedLessons": report.validated_lessons,
        "pendingLessons": report.pending_lessons,
        "deprecatedLessons": report.deprecated_lessons,
        "avgSuccessRate": report.avg_success_rate,
        "confidenceAdjustment": {
            "increased": report.confidence_adjustment_stats.increased,
            "decreased": report.confidence_adjustment_stats.decreased,
            "unchanged": report.confidence_adjustment_stats.unchanged,
        },
        "updatedCount": updated_count,
        // P2-F15: 统计来源分布，便于监控迁移进度
        "statsSource": {
            "precise": precise_count,
            "fallback": fallback_count,
        },
    }))
}

/// P2-F15 修复: 手动触发 lesson 验证的 Tauri 命令包装。
///
/// 内部核心函数 `run_lesson_validation` 已通过 `start_lesson_validation` 后台
/// 定时任务自动调度，但未暴露为 Tauri 命令，导致前端无法手动触发校证。
/// 此包装函数补齐该缺口，便于调试和紧急校证场景。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description =  "运行教训规则验证")]
#[tauri::command]
pub async fn run_lesson_validation_command(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.harness.db().clone();
    run_lesson_validation(&db).await
}

// ── [缺陷5 fix] 内部批量反思函数(非 Tauri 命令,供 cron 调度器直接调用) ──
//
// 从 run_batch_reflection 提取的核心逻辑。
// 参数通过独立引用传入,不需要 AppState。
//
// P3-#11 修复：`_engine` 参数类型从 `&WorkEngine` 改为 `&Arc<WorkEngine>`，
// 避免循环内 `Arc::new(_engine.clone())` 克隆整个 WorkEngine（可能包含大量状态）。
// Arc::clone 只是原子引用计数加一，O(1)。
// 接线：init/services.rs 的 start_batch_reflection 定时任务（每 6 小时）调用。
/// 批量反思的筛选条件 —— 落地「以超短线 / 短线 / 中线 / 长线为时间间隔自动反思」。
///
/// 4 周期档位的权威映射来自 `Period::default_holding_days()`（**2 / 5 / 28 / 90** 天），
/// 与智能荐股、候选池扫描用的是同一份定义（禁止本地再定义一份天数表）。
///
/// 语义：
/// - `period = None` ⇒ 不限档位，处理全部 pending（保持既有行为）
/// - `period = Some(p)` ⇒ 只处理「其原分析的期望持有天数最近邻归一到 p」的 pending
/// - `due_only = true` ⇒ 额外要求计划评估时点 `hindsight_date` 已到（严格的"按间隔"）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflectionFilter {
    /// 目标周期档位
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    /// 仅在 `hindsight_date <= today` 时反思（false = 沿用"以最新行情提前反思"）
    #[serde(default)]
    pub due_only: bool,
    /// 只处理 `stock_reflections.min_confidence_threshold >= 此值` 的 pending
    /// （None = 不限）。这是「决策校验」定时任务里用户设的阈值的落点 ——
    /// 此前该阈值只被写进 row 却无人消费（硬编码 `0u8`），现由本字段承接筛选。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<i32>,
    /// 覆盖 pending row 自带的 `reflection_depth`（None ⇒ 用 row 自身的值）。
    /// 「决策校验」任务里用户选的 light/deep 由此生效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_override: Option<String>,
}

impl ReflectionFilter {
    /// 该 pending 是否命中本筛选。
    ///
    /// - `expected_days`：原分析声明的期望持有天数（`None` ⇒ 按反思侧既有兜底口径
    ///   视为 28 天，与 `run_batch_reflection_inner` 内的一致）
    /// - `hindsight_due`：计划评估时点是否已到
    /// - `min_confidence_threshold`：pending row 自带的置信度阈值
    pub fn matches(
        &self,
        expected_days: Option<i64>,
        hindsight_due: bool,
        min_confidence_threshold: i32,
    ) -> bool {
        if let Some(p) = self.period {
            let days = expected_days.unwrap_or(28);
            if Period::nearest_for_holding_days(days) != p {
                return false;
            }
        }
        if let Some(min) = self.min_confidence {
            if min_confidence_threshold < min {
                return false;
            }
        }
        !self.due_only || hindsight_due
    }
}

/// `batch-reflection` 定时任务的配置（JSON 存 `CronJob.prompt`）。
///
/// 一个任务对应一个周期档位：`period = "ultra_short"` 的任务按 2 天间隔、`short` 按 5 天、
/// `mid` 按 28 天、`long` 按 90 天（天数映射来自 `Period::default_holding_days()`）。
/// 配合各自的 cron 表达式，即实现「以 4 个周期为时间间隔自动反思」。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchReflectionConfig {
    /// 只处理该周期档位的 pending（None = 全部）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    /// 仅在计划评估时点已到时反思（严格的"按间隔"）
    #[serde(default)]
    pub due_only: bool,
    /// 单轮最多处理条数（None ⇒ 后端默认 20）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_count: Option<u32>,
}

impl BatchReflectionConfig {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("解析 batch-reflection 配置失败: {e}"))
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("序列化 batch-reflection 配置失败: {e}"))
    }

    pub fn to_filter(&self) -> ReflectionFilter {
        ReflectionFilter {
            period: self.period,
            due_only: self.due_only,
            min_confidence: None,
            depth_override: None,
        }
    }
}

/// `validate-decisions` 定时任务的配置（JSON 存 `CronJob.prompt`）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateDecisionsConfig {
    /// 触发反思的最低置信度（0 = 全部）
    #[serde(default)]
    pub min_confidence: i32,
    /// 反思深度 "light" | "deep"（写入 pending row 的 reflection_depth 后被消费）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection_depth: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_count: Option<u32>,
}

impl ValidateDecisionsConfig {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("解析 validate-decisions 配置失败: {e}"))
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("序列化 validate-decisions 配置失败: {e}"))
    }

    pub fn to_filter(&self) -> ReflectionFilter {
        ReflectionFilter {
            period: None,
            due_only: false,
            min_confidence: Some(self.min_confidence),
            depth_override: self.reflection_depth.clone(),
        }
    }
}

/// `running` 状态超过该小时数即视为"卡死"，回收到 `pending` 重试。
///
/// 取 6 小时：单条反思的 LLM 调用最坏情况在分钟级，6 小时足够任何正常执行完成。
pub const STALE_RUNNING_HOURS: i64 = 6;

/// 回收卡死的 `running` 反思记录（幂等，返回回收条数）。
///
/// # 为什么必须有
///
/// 反思 row 被置为 `running` 后，若进程崩溃 / LLM 超时 / 用户直接关掉应用，
/// **没有任何路径会把它写回** —— 于是永久卡在 `running`：既不会被
/// `status = 'pending'` 的扫描命中，也没有超时重试。
///
/// DB 实证（2026-09-13）：`stock_reflections` 里 11 条 `running` 的记录最后一条
/// 停在 7-28，一个多月无人回收；同期 `resolved` 数为 **0** ——「自动反思从未产出
/// 过结论」的直接原因之一（见 `AUDIT-scheduled-tasks-2026-09-13.md`）。
pub async fn reclaim_stale_running(db: &DatabaseConnection, stale_hours: i64) -> u64 {
    use axagent_entities::stock_reflections;
    use sea_orm::sea_query::Expr;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let cutoff_ms = now_ms - stale_hours * 3_600_000;
    match stock_reflections::Entity::update_many()
        .col_expr(stock_reflections::Column::Status, Expr::value("pending"))
        .col_expr(stock_reflections::Column::UpdatedAt, Expr::value(now_ms))
        .filter(stock_reflections::Column::Status.eq("running"))
        .filter(stock_reflections::Column::UpdatedAt.lt(cutoff_ms))
        .exec(db)
        .await
    {
        Ok(r) => {
            if r.rows_affected > 0 {
                tracing::warn!(
                    "[batch_reflection] 回收 {} 条卡死 running 反思（超过 {stale_hours}h 未更新）",
                    r.rows_affected
                );
            }
            r.rows_affected
        },
        Err(e) => {
            // 回收失败不应阻断本轮反思（只是少了一次自愈机会）
            tracing::warn!("[batch_reflection] 回收卡死 running 反思失败: {e}");
            0
        },
    }
}

pub async fn run_batch_reflection_inner(
    db: &sea_orm::DatabaseConnection,
    _client: &axagent_astock_data::AStockClient,
    _engine: &std::sync::Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    _vector_store: &axagent_search::vector_store::VectorStore,
    _master_key: &[u8; 32],
    max_count: Option<u32>,
    trajectory_storage: Option<&std::sync::Arc<axagent_trajectory::TrajectoryStorage>>,
    filter: Option<&ReflectionFilter>,
) -> Result<serde_json::Value, String> {
    use crate::commands::error::ErrorResponse;
    use axagent_entities::stock_analyses;
    use axagent_entities::stock_reflections;

    let max_count = max_count.unwrap_or(20) as usize;
    let today_ms = chrono::Utc::now().timestamp_millis();

    // 0. 自愈：把卡死的 running 记录退回 pending（详见 reclaim_stale_running 文档）。
    //    必须在扫 pending 之前执行，否则被卡死的 row 本轮仍不会被处理。
    let reclaimed = reclaim_stale_running(db, STALE_RUNNING_HOURS).await;

    // 1. 扫所有 pending row,按 created_at ASC(最老的先处理,避免积压)
    let pendings: Vec<stock_reflections::Model> = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.eq("pending"))
        .order_by_asc(stock_reflections::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("run_batch_reflection_inner 扫 pending row 失败: {e}"))
        })?;

    tracing::info!(
        "[D1 batch_reflection] 扫到 {} 条 pending row, max_count={}",
        pendings.len(),
        max_count
    );

    let mut resolved = 0u32;
    let mut failed = 0u32;
    let mut skipped_young = 0u32;
    let mut skipped_by_filter = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for p in pendings.iter().take(max_count) {
        let analysis =
            match stock_analyses::Entity::find_by_id(&p.original_analysis_id).one(db).await {
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

        let analysis_date = analysis.as_of_date.as_deref().unwrap_or(&p.as_of_date);

        // [实际行情] 原「hindsight_date 未到 ⇒ skip」已移除：反思改为以**执行时的
        // 最新行情**为对比基准，计划时点只作为提示（下方仅记录日志）。
        // P3-#13 修复：用 NaiveDate 直接相减，避免 UTC vs Asia/Shanghai 时区错位。
        let hindsight_date = p.hindsight_date.as_str();
        let analysis_nd = chrono::NaiveDate::parse_from_str(analysis_date, "%Y-%m-%d").ok();
        let hindsight_nd = chrono::NaiveDate::parse_from_str(hindsight_date, "%Y-%m-%d").ok();

        let today_nd = {
            use chrono::TimeZone;
            let offset = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
            offset.from_utc_datetime(&chrono::Utc::now().naive_utc()).date_naive()
        };

        if let Some(h) = hindsight_nd {
            if h > today_nd {
                tracing::info!(
                    "[batch_reflection_inner] {} ({}) 计划评估时点 {} 未到,按最新行情提前反思",
                    p.id,
                    p.stock_code,
                    hindsight_date
                );
            }
        }

        // ── 4 周期分档筛选（P2-A）──
        // 命中 `filter.period` 档位，且（当 due_only 时）计划评估时点已到，才进入反思。
        // 于是「超短线 2 天 / 短线 5 天 / 中线 28 天 / 长线 90 天」可以各建一个定时
        // 任务，用各自的触发频率实现真正的「按周期间隔反思」；不配 filter 时行为与
        // 本改动前完全一致（不看到期、以最新行情提前反思）。
        if let Some(f) = filter {
            let hindsight_due = hindsight_nd.is_some_and(|h| h <= today_nd);
            if !f.matches(
                analysis.decision_expected_holding_days,
                hindsight_due,
                p.min_confidence_threshold,
            ) {
                skipped_by_filter += 1;
                continue;
            }
        }

        let days_held = match (analysis_nd, hindsight_nd) {
            (Some(a), Some(h)) => (h - a).num_days().max(0),
            _ => {
                let analysis_ms = chrono::NaiveDate::parse_from_str(analysis_date, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|dt| dt.and_utc().timestamp_millis())
                    .unwrap_or(p.created_at);
                let hindsight_ms = chrono::NaiveDate::parse_from_str(hindsight_date, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                    .map(|dt| dt.and_utc().timestamp_millis())
                    .unwrap_or(today_ms);
                (hindsight_ms - analysis_ms).max(0) / 86_400_000
            },
        };

        // [实际行情] 去掉了原「持仓未到期 skip」硬门槛：反思改为以「当前实际行情」
        // 为对比基准，未到期做期中观察（快照带 within_expected_horizon 标记）。
        // 真正的门槛在行情侧：分析日之后至少要有 1 个交易日的新行情。

        // ── [实际行情] 拉取「分析日 → 最新交易日」真实行情 ──
        // 取代原 `run_asof_backtest`：后者按「期望持有期」切窗口（未到期时被
        // min(len-1) 夹到最新一根，终点语义含糊），且只返回 return_pct，
        // 把 entry/exit/max_drawdown 全丢给下游 —— 反思 agent 看不到价格事实。
        let target_price = extract_target_price(&analysis);
        let snapshot = match compute_market_snapshot(
            _client,
            &p.stock_code,
            analysis_date,
            analysis.decision_expected_holding_days,
            target_price,
        )
        .await
        {
            Ok(s) if s.trading_days >= 1 => Some(s),
            Ok(s) => {
                tracing::info!(
                    "[batch_reflection_inner] {} ({}) 分析日之后仅 {} 个交易日,行情样本不足 skip",
                    p.id,
                    p.stock_code,
                    s.trading_days
                );
                skipped_young += 1;
                continue;
            },
            Err(e) => {
                tracing::warn!(
                    "[batch_reflection_inner] {} ({}) 行情快照失败,降级为无行情反思: {e}",
                    p.id,
                    p.stock_code
                );
                None
            },
        };

        let actual_outcome = snapshot
            .as_ref()
            .map(|s| s.render_outcome_short())
            .unwrap_or_else(|| p.actual_outcome.clone());
        let today_str = today_nd.format("%Y-%m-%d").to_string();

        // depth：cron 任务（validate-decisions）可在 filter 内指定 light/deep，
        // 覆盖 pending row 的默认值；不指定则用 row 自带的深度。
        let effective_depth =
            filter.and_then(|f| f.depth_override.as_deref()).unwrap_or(p.reflection_depth.as_str());

        let r = run_reflection_workflow(
            db,
            _client,
            // P3-#11: Arc::clone O(1)，而非克隆整个 WorkEngine
            &std::sync::Arc::clone(_engine),
            _vector_store,
            _master_key,
            &p.stock_code,
            &p.stock_name,
            &p.original_analysis_id,
            &actual_outcome,
            snapshot.as_ref().map(|s| s.net_return_pct),
            snapshot.as_ref().and_then(|s| s.alpha_pct),
            Some(snapshot.as_ref().map(|s| s.trading_days as i32).unwrap_or(days_held as i32)),
            snapshot.as_ref().map(|_| "沪深300"),
            analysis_date,
            // [实际行情] AS_OF 锚点用今天（行情终点=最新交易日），
            // 让 agent 的 K 线工具看到最新数据而非被锁在过去。
            &today_str,
            // [2026-09-13] 消费 pending row 自带的阈值与深度。
            // 此前这里是硬编码 `0u8` / `"light"`，于是 `stock_reflections` 的
            // `min_confidence_threshold` / `reflection_depth` 成了**死字段** ——
            // 用户在反思面板设的阈值与深度永远不生效（同构问题见铁律 12）。
            p.min_confidence_threshold.clamp(0, 255) as u8,
            effective_depth,
            Some(p.id.clone()),
            trajectory_storage, // [方向3] 透传轨迹存储
            snapshot.as_ref(),  // [实际行情] 价格层事实
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

    // D2 FIFO 清理（P3-#10：分批删除，每批 1000 条，避免单次大事务锁表）
    use sea_orm::QuerySelect;
    let ninety_days_ago_ms = today_ms - 90 * 86_400_000;
    let mut cleaned_up: u64 = 0;
    loop {
        let stale_ids: Vec<String> = stock_reflections::Entity::find()
            .select_only()
            .column(stock_reflections::Column::Id)
            .filter(stock_reflections::Column::Status.eq("completed"))
            .filter(stock_reflections::Column::UpdatedAt.lt(ninety_days_ago_ms))
            .order_by_asc(stock_reflections::Column::UpdatedAt)
            .limit(1000)
            .into_tuple()
            .all(db)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("[D2 inner] FIFO 查询超龄 row 失败: {e}");
                Vec::new()
            });
        if stale_ids.is_empty() {
            break;
        }
        let batch_size = stale_ids.len() as u64;
        let _ = stock_reflections::Entity::delete_many()
            .filter(stock_reflections::Column::Id.is_in(stale_ids))
            .exec(db)
            .await
            .map(|r| {
                cleaned_up += r.rows_affected;
            })
            .map_err(|e| {
                tracing::warn!("[D2 inner] FIFO 批量删除失败: {e}");
            });
        if batch_size < 1000 {
            break;
        }
    }

    tracing::info!(
        "[batch_reflection_inner] 完成: total={} resolved={} failed={} skipped_young={} \
         skipped_by_filter={} reclaimed={} cleaned={}",
        pendings.len(),
        resolved,
        failed,
        skipped_young,
        skipped_by_filter,
        reclaimed,
        cleaned_up
    );

    Ok(serde_json::json!({
        "totalPending": pendings.len(),
        "processed": pendings.len().min(max_count),
        "resolved": resolved,
        "failed": failed,
        "skippedYoung": skipped_young,
        "skippedByFilter": skipped_by_filter,
        "reclaimed": reclaimed,
        "cleanedUp": cleaned_up,
        "errors": errors,
    }))
}

// ── [实际行情快照] 反思的「当前实际行情」数据源 ──────────────────────

/// 反思拉取个股 K 线的根数（约两年交易日），足够覆盖任何持有期回溯。
const REFLECTION_KLINE_COUNT: u32 = 500;
/// A 股双边交易成本（佣金双边约 0.08% + 卖出印花税 0.1%）。
const A_SHARE_COST_RATE: f64 = 0.0018;
/// 默认对比基准：沪深300。
const DEFAULT_BENCHMARK_CODE: &str = "000300";

/// 「已完成的股票分析结论」vs「该股票当前实际行情」的确定性对比快照。
///
/// # 为什么不复用 `BacktestEngine::backtest_decision`
///
/// ① 它按「原始期望持有期」切窗口（`entry_idx + holding_days`），未到期时被
///    `min(len-1)` 夹到最新一根 —— 终点语义含糊，既不是到期日也不明确是今天；
/// ② 调用方此前只取 `return_pct`，把 `entry_price` / `exit_price` /
///    `max_drawdown_pct` 全部丢弃 —— 反思 agent 因此看不到任何价格层事实，
///    只能对着一个百分比数字"反思"。
///
/// 本快照固定窗口为「分析日 → 最新交易日」（即用户要的「当前实际行情」），
/// 把价格 / 回撤 / 超额 / 目标价进度完整交给下游 comparator 与反思 agent。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSnapshot {
    pub stock_code: String,
    /// 原始分析日（决策锚点）
    pub analysis_date: String,
    /// 入场基准日：分析日之后的首个交易日
    pub entry_date: String,
    /// 入场基准价：入场日开盘价（用分析日收盘价会引入前视偏差）
    pub entry_price: f64,
    /// 最新交易日（= 当前实际行情对应的时间点）
    pub latest_date: String,
    /// 最新收盘价（前复权，与 entry 同源）
    pub latest_price: f64,
    /// 入场 → 最新 的原始涨跌幅（%），不含交易成本
    pub price_change_pct: f64,
    /// 扣双边成本后的净收益率（%）
    pub net_return_pct: f64,
    /// 区间最高 / 最低价
    pub period_high: f64,
    pub period_low: f64,
    /// 相对入场价的期间最大回撤（%）
    pub max_drawdown_pct: f64,
    /// 实际跨度（交易日）
    pub trading_days: i64,
    /// 原始决策的期望持有天数（交易日）；None = 未声明
    pub expected_holding_days: Option<i64>,
    /// 是否仍未到期望持有期（true ⇒ 结论属"期中观察"，不是事后终评）
    pub within_expected_horizon: bool,
    /// 基准代码（沪深300）
    pub benchmark_code: Option<String>,
    pub benchmark_change_pct: Option<f64>,
    /// 相对基准的超额收益（%）
    pub alpha_pct: Option<f64>,
    /// 决策目标价（trader 节点 targetPrice）
    pub target_price: Option<f64>,
    /// 目标价实现度（%）：现价涨幅 / 目标涨幅；>100 表示已超越目标价
    pub target_progress_pct: Option<f64>,
    /// 最新价是否已达到 / 超过目标价（客观陈述，语义由 LLM 按看多看空判定）
    pub target_reached: Option<bool>,
}

impl MarketSnapshot {
    /// 渲染为单行摘要 —— 写入 `stock_reflections.actual_outcome` 与轨迹。
    ///
    /// 刻意不用 "correct/wrong" 这种结论词：结论应由反思 agent 给出，
    /// 这里只陈述**发生了什么**（价格从哪到哪、涨跌多少、基准如何）。
    pub fn render_outcome_short(&self) -> String {
        let mut s = format!(
            "{} → {}（{}个交易日）：入场基准价 {:.2} → 最新价 {:.2}，涨跌 {:+.2}%（扣双边成本净 {:+.2}%），区间最大回撤 {:.2}%",
            self.analysis_date,
            self.latest_date,
            self.trading_days,
            self.entry_price,
            self.latest_price,
            self.price_change_pct,
            self.net_return_pct,
            self.max_drawdown_pct
        );
        if let (Some(code), Some(chg)) = (&self.benchmark_code, self.benchmark_change_pct) {
            s.push_str(&format!(
                "；同期{} {:+.2}%，超额 {:+.2}%",
                code,
                chg,
                self.alpha_pct.unwrap_or(0.0)
            ));
        }
        if let Some(tp) = self.target_price {
            s.push_str(&format!(
                "；决策目标价 {:.2}（目标实现度 {:.1}%）",
                tp,
                self.target_progress_pct.unwrap_or(0.0)
            ));
        }
        s
    }

    /// 渲染为反思 agent 的【实际行情】文本块（注入 `actual_market_text` 变量）。
    ///
    /// 与 `actual_market_json` 并存：JSON 供 comparator 按路径下钻，
    /// 文本供 LLM 直接阅读 —— 不让 LLM 自己去解析嵌套 JSON 的键名。
    pub fn render_text(&self) -> String {
        let mut s = String::from("【当前实际行情】\n");
        s.push_str(&format!(
            "- 窗口：分析日 {} → 最新交易日 {}（{} 个交易日）\n",
            self.analysis_date, self.latest_date, self.trading_days
        ));
        s.push_str(&format!(
            "- 入场基准：{} 开盘 {:.2}（用分析日之后首个交易日开盘价，避免前视偏差）\n",
            self.entry_date, self.entry_price
        ));
        s.push_str(&format!("- 最新收盘价：{:.2}\n", self.latest_price));
        s.push_str(&format!(
            "- 区间涨跌：{:+.2}%（扣双边交易成本后净收益 {:+.2}%）\n",
            self.price_change_pct, self.net_return_pct
        ));
        s.push_str(&format!(
            "- 区间最高 {:.2} / 最低 {:.2}，期间最大回撤 {:.2}%\n",
            self.period_high, self.period_low, self.max_drawdown_pct
        ));
        match (&self.benchmark_code, self.benchmark_change_pct) {
            (Some(code), Some(chg)) => s.push_str(&format!(
                "- 基准 {} 同期 {:+.2}%，超额收益 {:+.2}%\n",
                code,
                chg,
                self.alpha_pct.unwrap_or(0.0)
            )),
            _ => s.push_str("- 基准对比：数据不可用\n"),
        }
        match self.target_price {
            Some(tp) => s.push_str(&format!(
                "- 决策目标价 {:.2}：现价相对入场价已完成目标幅度的 {:.1}%（最新价{}目标价）\n",
                tp,
                self.target_progress_pct.unwrap_or(0.0),
                if self.target_reached == Some(true) {
                    "已达到/超过"
                } else {
                    "未达"
                }
            )),
            None => s.push_str("- 决策目标价：原始结论未声明\n"),
        }
        match self.expected_holding_days {
            Some(d) if self.within_expected_horizon => s.push_str(&format!(
                "- ⚠ 尚未到期望持有期（期望 {} 个交易日 / 实际 {} 个）：本结论属**期中观察**，\
                 不应据此判定策略失效，权重放低\n",
                d, self.trading_days
            )),
            Some(d) => s.push_str(&format!(
                "- 已越过期望持有期（期望 {} 个交易日 / 实际 {} 个）：可做事后终评\n",
                d, self.trading_days
            )),
            None => {},
        }
        s
    }
}

/// 计算「已完成的股票分析结论」vs「该股票当前实际行情」的对比快照。
///
/// 窗口固定为「分析日 → 最新交易日」（用户要求的「当前实际行情」）。
/// 全部指标由前复权 K 线确定性推导，不依赖 LLM、不做任何脑补。
///
/// 失败语义：K 线取不到 / 分析日之后无数据 → `Err`，由调用方降级为
/// 「无行情快照」（`market_snapshot=None`），**绝不伪造 0% 收益** ——
/// 伪造 0% 会让 comparator 把实际涨跌误判成「横盘」，进而让 agent 基于假数据反思。
pub async fn compute_market_snapshot(
    client: &axagent_astock_data::AStockClient,
    stock_code: &str,
    analysis_date: &str,
    expected_holding_days: Option<i64>,
    target_price: Option<f64>,
) -> Result<MarketSnapshot, String> {
    use axagent_harness::market_data::{AdjType, MarketDataProvider};

    let klines = MarketDataProvider::get_klines(
        client,
        stock_code,
        "daily",
        REFLECTION_KLINE_COUNT,
        Some(AdjType::Forward),
    )
    .await
    .map_err(|e| format!("获取 {stock_code} K线失败: {e}"))?;

    if klines.is_empty() {
        return Err(format!("{stock_code} 无K线数据"));
    }

    // 入场基准：分析日之后的首个交易日开盘价（分析日收盘后才出结论，用其收盘价会前视偏差）
    let entry_idx = klines
        .iter()
        .position(|k| k.date.as_str() > analysis_date)
        .ok_or_else(|| format!("{stock_code} 在 {analysis_date} 之后无K线数据"))?;
    let entry_bar = &klines[entry_idx];
    let entry_price = entry_bar.open;
    // 显式拒绝 NaN 与 ≤0（`!(x > 0.0)` 对 NaN 为真，语义等价但 clippy 要求可读写法）
    if entry_price.is_nan() || entry_price <= 0.0 {
        return Err(format!("{stock_code} 入场基准价非法: {entry_price}"));
    }

    // 最新一根 = 当前实际行情
    let latest_bar = klines.last().ok_or_else(|| format!("{stock_code} K线为空"))?;
    let latest_price = latest_bar.close;

    // 区间高低 + 最大回撤（入场日 → 最新）
    let window = &klines[entry_idx..];
    let period_high = window.iter().fold(f64::MIN, |m, k| m.max(k.high));
    let mut period_low = window.iter().filter(|k| k.low > 0.0).fold(f64::MAX, |m, k| m.min(k.low));
    let mut peak = entry_price;
    let mut max_dd = 0.0_f64;
    for k in window {
        if k.close > peak {
            peak = k.close;
        }
        if peak > 0.0 {
            let dd = (peak - k.close) / peak;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    if period_low == f64::MAX {
        // 全窗口无有效 low（脏数据）→ 用收盘价兜底，不让 MAX 泄进下游
        period_low = window.iter().map(|k| k.close).fold(f64::MAX, f64::min);
    }

    let price_change_pct = (latest_price - entry_price) / entry_price * 100.0;
    let net_return_pct = price_change_pct - A_SHARE_COST_RATE * 100.0;
    let trading_days = (klines.len() - 1 - entry_idx) as i64;

    // 基准（沪深300）同期涨跌 → 超额收益。基准失败不阻断主链路（降级为 None）。
    let (benchmark_code, benchmark_change_pct, alpha_pct) =
        match compute_benchmark_change(client, &entry_bar.date, &latest_bar.date).await {
            Ok(chg) => {
                (Some(DEFAULT_BENCHMARK_CODE.to_string()), Some(chg), Some(price_change_pct - chg))
            },
            Err(e) => {
                tracing::warn!("[market snapshot] {stock_code} 基准对比失败,降级为无超额收益: {e}");
                (None, None, None)
            },
        };

    // 目标价实现度：现价涨幅 / 目标涨幅
    let target_progress_pct = target_price.and_then(|tp| {
        let expected_pct = (tp - entry_price) / entry_price * 100.0;
        if expected_pct.abs() < 0.01 {
            None
        } else {
            Some(price_change_pct / expected_pct * 100.0)
        }
    });
    let target_reached = target_price.map(|tp| latest_price >= tp);
    let within_expected_horizon = expected_holding_days.map(|d| trading_days < d).unwrap_or(false);

    Ok(MarketSnapshot {
        stock_code: stock_code.to_string(),
        analysis_date: analysis_date.to_string(),
        entry_date: entry_bar.date.clone(),
        entry_price,
        latest_date: latest_bar.date.clone(),
        latest_price,
        price_change_pct,
        net_return_pct,
        period_high,
        period_low,
        max_drawdown_pct: max_dd * 100.0,
        trading_days,
        expected_holding_days,
        within_expected_horizon,
        benchmark_code,
        benchmark_change_pct,
        alpha_pct,
        target_price,
        target_progress_pct,
        target_reached,
    })
}

/// 取基准（沪深300）在指定区间内的涨跌幅（%）。
///
/// 与个股同源前复权 K 线，保证可比性；任一端点缺失时返回 `Err`，
/// 由调用方降级为「无基准对比」，而不是用 0.0 冒充。
async fn compute_benchmark_change(
    client: &axagent_astock_data::AStockClient,
    start_date: &str,
    end_date: &str,
) -> Result<f64, String> {
    use axagent_harness::market_data::{AdjType, MarketDataProvider};

    let klines = MarketDataProvider::get_klines(
        client,
        DEFAULT_BENCHMARK_CODE,
        "daily",
        REFLECTION_KLINE_COUNT,
        Some(AdjType::Forward),
    )
    .await
    .map_err(|e| format!("基准 K 线获取失败: {e}"))?;

    let start = klines
        .iter()
        .position(|k| k.date.as_str() >= start_date)
        .ok_or_else(|| format!("基准无 {start_date} 之后数据"))?;
    let end = klines
        .iter()
        .rposition(|k| k.date.as_str() <= end_date)
        .ok_or_else(|| format!("基准无 {end_date} 之前数据"))?;
    if start > end {
        return Err(format!("基准区间无效: {start_date}~{end_date}"));
    }
    let base = klines[start].open;
    // 同 compute_market_snapshot：显式拒绝 NaN 与 ≤0
    if base.is_nan() || base <= 0.0 {
        return Err("基准基准价非法".to_string());
    }
    Ok((klines[end].close - base) / base * 100.0)
}

/// 从已完成的分析记录中提取决策目标价（trader 节点的 `targetPrice`）。
///
/// 两条来源，按可信度排序：
/// 1. `decision_json.targetPrice`（决策落库时已规范化）
/// 2. `blackboard_snapshot` 的 `_raw.trader.content.targetPrice`（LLM 原始输出）
fn extract_target_price(analysis: &stock_analyses::Model) -> Option<f64> {
    let from_decision = analysis
        .decision_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.get("targetPrice").and_then(|t| t.as_f64()));
    if from_decision.is_some() {
        return from_decision;
    }

    let snapshot: serde_json::Value =
        serde_json::from_str(analysis.blackboard_snapshot.as_deref()?).ok()?;
    snapshot.get("_raw")?.get("trader")?.get("content")?.get("targetPrice")?.as_f64()
}

/// 从 `stock_analyses.blackboard_snapshot` 构造 `sub-analysis` 变量。
///
/// [v2] 替代原 SubWorkflowNode 嵌套重放：直接从已保存的分析结果记忆中恢复
/// 各节点输出，避免重跑完整 stock-analysis DAG。
///
/// ## snapshot 结构（由 `build_blackboard_snapshot` 写入）
/// - `_raw.<nodeId>` — 原始节点输出（含 content/params/result 字段）
/// - `report.<nodeId>` — 分析师报告（纯文本）
/// - `params.<nodeId>` — content 解析后的 JSON 对象
/// - `result.<nodeId>` — CodeNode 的 result 字段
///
/// ## content 字段预处理
/// AgentNode 的 `content` 字段通常是 JSON 字符串（如 `{"action":"买入",...}`）。
/// `resolve_var_path` 不会自动解析 JSON 字符串，路径如
/// `sub-analysis.trader.content.action` 会下钻失败。
/// 此处对每个节点的 `content` 字段做 JSON 解析，把字符串转为对象，
/// 确保 input_mapping 的点路径能正确下钻。
fn build_sub_analysis_from_snapshot(
    snapshot_json: Option<&str>,
    stock_code: &str,
) -> serde_json::Value {
    use serde_json::Map;

    let Some(json_str) = snapshot_json else {
        tracing::warn!(
            "[reflection] {}: blackboard_snapshot 为 None（原始分析可能未完成）,注入空记忆",
            stock_code
        );
        return serde_json::json!({});
    };

    let Ok(snapshot) = serde_json::from_str::<serde_json::Value>(json_str) else {
        tracing::error!(
            "[reflection] {}: blackboard_snapshot JSON 解析失败,注入空记忆",
            stock_code
        );
        return serde_json::json!({});
    };

    let Some(obj) = snapshot.as_object() else {
        tracing::warn!(
            "[reflection] {}: blackboard_snapshot 不是 JSON 对象,注入空记忆",
            stock_code
        );
        return serde_json::json!({});
    };

    // 检查是否有 _raw.* 条目（新版 snapshot）
    let has_raw = obj.keys().any(|k| k.starts_with("_raw."));

    if !has_raw {
        tracing::warn!(
            "[reflection] {}: 旧版 snapshot（无 _raw.*），JSON 结构已丢失,注入空记忆。建议重新运行完整分析工作流以生成新版 snapshot",
            stock_code
        );
        return serde_json::json!({});
    }

    let mut sub_analysis = Map::new();
    for (key, val) in obj {
        let Some(node_id) = key.strip_prefix("_raw.") else {
            continue;
        };

        // 克隆节点输出，对 content 字段做 JSON 解析预处理
        let mut node_output = val.clone();
        if let Some(node_obj) = node_output.as_object_mut() {
            if let Some(content) = node_obj.get("content").and_then(|v| v.as_str()) {
                // content 是 JSON 字符串 → 解析为对象，确保路径下钻可用
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
                    if parsed.is_object() {
                        node_obj.insert("content".into(), parsed);
                    }
                }
            }
        }

        sub_analysis.insert(node_id.to_string(), node_output);
    }

    if sub_analysis.is_empty() {
        tracing::warn!(
            "[reflection] {}: snapshot 中未找到任何 _raw.* 节点输出,注入空记忆",
            stock_code
        );
    }

    serde_json::Value::Object(sub_analysis)
}

/// [方向4/方向5] 提交反思结果的用户反馈（1-5 星评分）。
///
/// 接入 FeedbackOrchestrator + ExperiencePipeline 双轨：
/// - Pipeline：把反馈转为 Experience 写入 RLOptimizer 经验池（reward: 1→-1.0, 5→1.0）
/// - Orchestrator：计数正/负反馈，达到阈值触发 RLTraining / SkillEvolution
///
/// [方向5] 当 Orchestrator 返回 `TriggerSkillEvolution` 时，spawn 异步任务
/// 真正调用 SkillEvolutionEngine 对 lesson 做语义变异进化：
/// - 从 stock_reflections 表查出 lesson_summary
/// - 包装为单步 Skill（content = "1. {lesson}"）
/// - 用 try_lock 获取 engine（避免阻塞反馈返回）
/// - 拉取最近 30 条轨迹作为 test_trajectories
/// - 进化成功则更新 reflection_lessons.rule_pattern 字段
///
/// `analysis_id` 同时作为 trace_id，保证同一反思的多次评分会被 Orchestrator 去重。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description =  "提交反思反馈评分")]
#[tauri::command]
pub async fn submit_reflection_feedback(
    state: State<'_, AppState>,
    analysis_id: String,
    rating: u8,
    comment: Option<String>,
) -> Result<serde_json::Value, String> {
    use crate::commands::_shared_state::{SHARED_ORCHESTRATOR, SHARED_PIPELINE};

    if !(1..=5).contains(&rating) {
        return Err("评分必须在 1-5 之间".to_string());
    }
    if analysis_id.trim().is_empty() {
        return Err("analysis_id 不能为空".to_string());
    }

    tracing::info!(
        "[reflection_feedback] analysis_id={} rating={} comment={:?}",
        analysis_id,
        rating,
        comment
    );

    // 1. ExperiencePipeline：反馈 → Experience → 经验池
    let pipeline = SHARED_PIPELINE.clone();
    let trace = analysis_id.clone();
    let comment_clone = comment.clone();
    let pipeline_handle = tokio::task::spawn(async move {
        let mut pipeline = pipeline.write().await;
        pipeline.process_feedback(&trace, rating, comment_clone.as_deref()).await
    });

    // 2. FeedbackOrchestrator：计数 + 阈值触发动作
    let orchestrator = SHARED_ORCHESTRATOR.clone();
    let action_result = tokio::task::spawn_blocking(move || orchestrator.record_feedback(rating))
        .await
        .map_err(|e| format!("Orchestrator join 错误: {e}"))?;

    let action_str = match &action_result {
        axagent_agent::OrchestratorAction::None => "none",
        axagent_agent::OrchestratorAction::TriggerRLTraining { .. } => "trigger_rl_training",
        axagent_agent::OrchestratorAction::TriggerSkillEvolution { .. } => {
            // [方向5] 真正触发 SkillEvolutionEngine 进化（异步，不阻塞反馈返回）
            let evolution_state = state.clone_for_evolution();
            let ev_analysis_id = analysis_id.clone();
            tokio::task::spawn(async move {
                if let Err(e) = run_lesson_evolution(&evolution_state, &ev_analysis_id).await {
                    tracing::warn!("[reflection_feedback] SkillEvolution 失败: {e}");
                }
            });
            "trigger_skill_evolution"
        },
        axagent_agent::OrchestratorAction::TriggerPoolSizeCheck { .. } => "trigger_pool_size_check",
    };

    // 等待 Pipeline 完成（best-effort，失败不影响反馈提交）
    if let Err(e) = pipeline_handle.await {
        tracing::warn!("[reflection_feedback] Pipeline join 错误: {e}");
    }

    Ok(serde_json::json!({
        "analysisId": analysis_id,
        "rating": rating,
        "action": action_str,
        "orchestratorStats": {
            "totalFeedback": SHARED_ORCHESTRATOR.stats().total_feedback,
            "negativeCount": SHARED_ORCHESTRATOR.stats().negative_count,
            "positiveCount": SHARED_ORCHESTRATOR.stats().positive_count,
        }
    }))
}

/// [方向5] SkillEvolution 所需的最小状态快照（避免持有 AppState 引用）。
struct EvolutionStateSnapshot {
    db: sea_orm::DatabaseConnection,
    skill_engine: Arc<tokio::sync::Mutex<axagent_trajectory::SkillEvolutionEngine>>,
    trajectory_storage: Arc<axagent_trajectory::TrajectoryStorage>,
}

impl AppState {
    /// 克隆 SkillEvolution 所需的最小状态
    fn clone_for_evolution(&self) -> EvolutionStateSnapshot {
        EvolutionStateSnapshot {
            db: self.harness.db().clone(),
            skill_engine: self.skill_evolution_engine.clone(),
            trajectory_storage: self.trajectory_storage.clone(),
        }
    }
}

/// [方向5] 对指定反思的 lesson 执行 SkillEvolutionEngine 语义变异进化。
///
/// 流程：
/// 1. 从 stock_reflections 表查出 lesson_summary / verdict / stock_code
/// 2. 把 lesson 包装为单步 Skill（content = "1. {lesson}"）
/// 3. 用 try_lock 获取 engine（失败则跳过，不阻塞）
/// 4. 拉取最近 30 条轨迹作为 test_trajectories
/// 5. 调用 engine.run(&skill, &test_refs).await
/// 6. 进化成功则更新 reflection_lessons.rule_pattern 字段
async fn run_lesson_evolution(
    state: &EvolutionStateSnapshot,
    analysis_id: &str,
) -> Result<(), String> {
    use axagent_entities::{reflection_lessons, stock_reflections};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    // 1. 查询反思记录
    let reflection = stock_reflections::Entity::find_by_id(analysis_id.to_string())
        .one(&state.db)
        .await
        .map_err(|e| format!("查询 stock_reflections 失败: {e}"))?
        .ok_or_else(|| format!("反思记录 {analysis_id} 不存在"))?;

    let lesson_summary = reflection
        .lesson_summary
        .as_deref()
        .ok_or_else(|| "lesson_summary 为空，无法进化".to_string())?;

    let stock_code = reflection.stock_code.clone();

    // 2. 包装为单步 Skill（用 Skill::new 构造函数）
    let mut skill = axagent_trajectory::Skill::new(
        format!("反思教训:{}", stock_code),
        format!("股票 {} 反思教训", stock_code),
        format!("1. {lesson_summary}"),
        "reflection_lesson".to_string(),
    );
    skill.id = format!("lesson_{analysis_id}");

    // 3. try_lock 获取 engine（不阻塞）
    let mut engine = state
        .skill_engine
        .try_lock()
        .map_err(|_| "SkillEvolutionEngine 被占用（cron 正在运行），跳过本次进化".to_string())?;

    // 4. 拉取最近 30 条轨迹
    let trajectories = state
        .trajectory_storage
        .get_trajectories(Some(30))
        .await
        .map_err(|e| format!("拉取轨迹失败: {e}"))?;

    if trajectories.len() < 10 {
        return Err(format!("轨迹数量不足（{} < 10），无法进化", trajectories.len()));
    }

    let test_refs: Vec<&axagent_trajectory::Trajectory> = trajectories.iter().collect();

    // 5. 调用进化
    tracing::info!("[skill_evolution] 开始进化 lesson {} (stock={})", analysis_id, stock_code);
    let modification = engine.run(&skill, &test_refs).await;

    if let Some(modification) = &modification {
        tracing::info!(
            "[skill_evolution] 进化完成: confidence={:.3} reason={}",
            modification.confidence,
            modification.reason
        );

        // 6. 更新 reflection_lessons.rule_pattern 字段
        if !modification.new_content.is_empty() {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let _ = reflection_lessons::Entity::update_many()
                .col_expr(
                    reflection_lessons::Column::RulePattern,
                    sea_orm::sea_query::Expr::value(modification.new_content.clone()),
                )
                .col_expr(
                    reflection_lessons::Column::UpdatedAt,
                    sea_orm::sea_query::Expr::value(now_ms),
                )
                .filter(reflection_lessons::Column::SourceReflectionId.eq(analysis_id.to_string()))
                .exec(&state.db)
                .await;
            tracing::info!(
                "[skill_evolution] 已更新 reflection_lessons.rule_pattern (analysis_id={})",
                analysis_id
            );
        }
    } else {
        tracing::info!("[skill_evolution] 进化未产生改进（lesson={})", analysis_id);
    }

    Ok(())
}

/// M1: 确定性判定 —— 用行情快照反推决策方向是否正确。
///
/// 替代旧的「LLM verdict → was_correct」自评路径：verdict 是反思 agent 的
/// 主观结论，可被幻觉污染；此处只用客观数据（决策方向 + 净收益符号）判定。
///
/// 返回:
///   Some(1) 方向命中（看多且涨 / 看空且跌）
///   Some(0) 方向未命中（看多且跌 / 看空且涨）
///   None    无法判定（无行情快照 / 决策方向缺失或为中性档 / 期中观察 / 收益 NaN）
///
/// ⚠ 中性档（持有/观望/不确定/无法判断）与「期中观察」不判定 —— 不产出伪结论
/// （诚实性铁律）。无法判定时上层**不写** strategy_performance 行，
/// 与既有「无判定依据不写行」语义一致，避免把「没判定」计成「判错」。
///
/// ⚠ NaN 显式不判定：`net_return_pct.is_nan()` 时两个不等号都不命中，
/// 若直接套 `> 0.0 / < 0.0` 会把 NaN 误判成「未命中」（见 AGENTS.md 浮点判据教训）。
fn deterministic_was_correct(
    decision_action: Option<&str>,
    market_snapshot: Option<&MarketSnapshot>,
) -> Option<i32> {
    // 行情不可用 → 无法判定
    let snap = market_snapshot?;
    // 期中观察（未到期望持有期）→ 不判定：短期噪声不是策略失效证据
    if snap.within_expected_horizon {
        return None;
    }
    let net = snap.net_return_pct;
    if net.is_nan() {
        return None;
    }
    // 决策方向归一化；中性档 / 缺失 → 不判定
    let kind = normalize_action(decision_action?)?;
    match kind {
        ActionKind::Buy | ActionKind::Increase => Some(if net > 0.0 { 1 } else { 0 }),
        ActionKind::Sell | ActionKind::Reduce => Some(if net < 0.0 { 1 } else { 0 }),
        ActionKind::Hold | ActionKind::Wait | ActionKind::Uncertain | ActionKind::Unavailable => {
            None
        },
    }
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

// ── 单元测试：[实际行情] 快照渲染与目标价提取（纯函数，无需网络）──
//
// 为什么只测渲染层：`compute_market_snapshot` 依赖 AStockClient 实时取数，
// 属集成范畴；但其**消费侧语义**（"只陈述事实、不下结论"、"期中观察必须标注"、
// "缺数据必须显式降级而非用 0 冒充"）全在渲染函数里，是本改造的核心契约，
// 且完全离线可验。契约破了会让反思 agent 重新对着假数据写结论。
#[cfg(test)]
mod market_snapshot_tests {
    use super::*;

    fn sample() -> MarketSnapshot {
        MarketSnapshot {
            stock_code: "600519".to_string(),
            analysis_date: "2026-08-01".to_string(),
            entry_date: "2026-08-04".to_string(),
            entry_price: 100.0,
            latest_date: "2026-09-11".to_string(),
            latest_price: 94.4,
            price_change_pct: -5.6,
            net_return_pct: -5.78,
            period_high: 103.0,
            period_low: 92.0,
            max_drawdown_pct: 10.68,
            trading_days: 28,
            expected_holding_days: Some(28),
            within_expected_horizon: false,
            benchmark_code: Some("000300".to_string()),
            benchmark_change_pct: Some(1.2),
            alpha_pct: Some(-6.8),
            target_price: Some(120.0),
            target_progress_pct: Some(-28.0),
            target_reached: Some(false),
        }
    }

    /// 摘要必须是**事实陈述**，不能出现 correct/wrong 这类结论词。
    ///
    /// 回归背景：旧实现把 actual_outcome 直接写成 "correct"/"wrong"，等于后端替
    /// 反思 agent 下了结论，且丢掉了价格信息（agent 无从引用具体数字做归因）。
    #[test]
    fn outcome_short_states_facts_not_verdict() {
        let s = sample().render_outcome_short();
        assert!(s.contains("2026-08-01"), "应含分析日: {s}");
        assert!(s.contains("2026-09-11"), "应含最新交易日: {s}");
        assert!(s.contains("100.00"), "应含入场基准价: {s}");
        assert!(s.contains("94.40"), "应含最新价: {s}");
        assert!(s.contains("000300"), "应含基准: {s}");
        assert!(s.contains("120.00"), "应含目标价: {s}");
        assert!(!s.contains("correct"), "不得输出结论词 correct: {s}");
        assert!(!s.contains("wrong"), "不得输出结论词 wrong: {s}");
    }

    /// 未到期望持有期必须显式标注为「期中观察」。
    ///
    /// 回归背景：改造后反思不再等持有期到期（以「当前实际行情」为准），
    /// 若不标注，agent 会把短期噪声当作策略失效证据。
    #[test]
    fn text_marks_horizon_state() {
        let mut snap = sample();
        snap.within_expected_horizon = true;
        let early = snap.render_text();
        assert!(early.contains("尚未到期望持有期"), "应标注期中观察: {early}");
        assert!(early.contains("期中观察"), "应显式提示降权: {early}");

        snap.within_expected_horizon = false;
        let due = snap.render_text();
        assert!(due.contains("已越过期望持有期"), "应标注可终评: {due}");
    }

    /// 文本块必须把价格事实全部带上（供 LLM 引用，而非只有百分比）。
    #[test]
    fn text_carries_price_facts() {
        let t = sample().render_text();
        assert!(t.contains("入场基准"), "{t}");
        assert!(t.contains("最新收盘价"), "{t}");
        assert!(t.contains("最大回撤"), "{t}");
        assert!(t.contains("超额收益"), "{t}");
        assert!(t.contains("目标价"), "{t}");
    }

    /// 缺失数据必须**显式降级**，不能用 0 冒充（0 会被读成「横盘/持平」）。
    #[test]
    fn text_degrades_explicitly_when_data_missing() {
        let mut snap = sample();
        snap.benchmark_code = None;
        snap.benchmark_change_pct = None;
        snap.alpha_pct = None;
        snap.target_price = None;
        snap.target_progress_pct = None;
        snap.target_reached = None;
        let t = snap.render_text();
        assert!(t.contains("基准对比：数据不可用"), "{t}");
        assert!(t.contains("原始结论未声明"), "{t}");
    }

    /// 目标价提取：decision_json 优先，其次 blackboard_snapshot 的 _raw.trader.content。
    #[test]
    fn extract_target_price_prefers_decision_json() {
        let mut model = stock_analyses::Model {
            id: "a1".to_string(),
            stock_code: "600519".to_string(),
            stock_name: "贵州茅台".to_string(),
            analysis_date: "2026-08-01".to_string(),
            provider_id: "p".to_string(),
            conversation_id: "c".to_string(),
            status: "completed".to_string(),
            decision_action: Some("买入".to_string()),
            decision_position_pct: None,
            // v228 新增轴：NULL = 采集时点无此信息（不是 EMPTY，也不是「持有」）
            decision_position_state: None,
            decision_reasoning: None,
            decision_json: Some(r#"{"targetPrice": 133.0}"#.to_string()),
            horizon_price_map: None,
            horizon_decisions: None,
            blackboard_snapshot: Some(
                r#"{"_raw":{"trader":{"content":{"targetPrice": 120.0}}}}"#.to_string(),
            ),
            config_id: None,
            analysis_kind: "live".to_string(),
            as_of_date: Some("2026-08-01".to_string()),
            decision_time_horizon: Some("mid".to_string()),
            decision_expected_holding_days: Some(28),
            model_version: None,
            // A4：NULL = 采集时点无版本信息（A4 之前的存量行、chat 通道写入均为此形态）
            template_version: None,
            data_snapshot_id: None,
            outcome: None,
            llm_decision_json: None,
            parent_analysis_id: None,
            trade_intent_status: "pending".to_string(),
            trade_intent_source: None,
            trade_intent_source_ref_id: None,
            trade_intent_reviewed_at: None,
            trade_intent_reviewed_by: None,
            trade_intent_review_notes: None,
            trade_intent_actual_trade_id: None,
            created_at: 0,
            updated_at: 0,
        };

        assert_eq!(extract_target_price(&model), Some(133.0), "应优先取 decision_json");

        // decision_json 无目标价 → 回退 snapshot
        model.decision_json = Some(r#"{"confidence": 0.6}"#.to_string());
        assert_eq!(extract_target_price(&model), Some(120.0), "应回退到 blackboard_snapshot");

        // 两处都无 → None（不得返回 0.0 之类假值）
        model.blackboard_snapshot = Some("{}".to_string());
        assert_eq!(extract_target_price(&model), None);
    }
}

// ── 单元测试：M1 确定性判定（deterministic_was_correct）──
#[cfg(test)]
mod deterministic_was_correct_tests {
    use super::*;

    fn snap(net_return_pct: f64, within_expected_horizon: bool) -> MarketSnapshot {
        MarketSnapshot {
            stock_code: "600519".to_string(),
            analysis_date: "2026-08-01".to_string(),
            entry_date: "2026-08-04".to_string(),
            entry_price: 100.0,
            latest_date: "2026-09-11".to_string(),
            latest_price: 94.4,
            price_change_pct: -5.6,
            net_return_pct,
            period_high: 103.0,
            period_low: 92.0,
            max_drawdown_pct: 10.68,
            trading_days: 28,
            expected_holding_days: Some(28),
            within_expected_horizon,
            benchmark_code: Some("000300".to_string()),
            benchmark_change_pct: Some(1.2),
            alpha_pct: Some(-6.8),
            target_price: Some(120.0),
            target_progress_pct: Some(-28.0),
            target_reached: Some(false),
        }
    }

    /// 看多（买入/增持）且净收益为正 → 1（胜）；中英文值域等价。
    #[test]
    fn buy_uptrend_is_correct() {
        assert_eq!(deterministic_was_correct(Some("买入"), Some(&snap(5.0, false))), Some(1));
        assert_eq!(deterministic_was_correct(Some("BUY"), Some(&snap(5.0, false))), Some(1));
        assert_eq!(deterministic_was_correct(Some("增持"), Some(&snap(5.0, false))), Some(1));
    }

    /// 看多但净收益为负 → 0（未命中）
    #[test]
    fn buy_downtrend_is_incorrect() {
        assert_eq!(deterministic_was_correct(Some("买入"), Some(&snap(-5.78, false))), Some(0));
    }

    /// 看空（卖出/减持）且净收益为负 → 1（胜）
    #[test]
    fn sell_downtrend_is_correct() {
        assert_eq!(deterministic_was_correct(Some("卖出"), Some(&snap(-5.78, false))), Some(1));
        assert_eq!(deterministic_was_correct(Some("SELL"), Some(&snap(-5.78, false))), Some(1));
        assert_eq!(deterministic_was_correct(Some("减持"), Some(&snap(-5.78, false))), Some(1));
    }

    /// 看空但净收益为正 → 0（未命中）
    #[test]
    fn sell_uptrend_is_incorrect() {
        assert_eq!(deterministic_was_correct(Some("卖出"), Some(&snap(5.0, false))), Some(0));
    }

    /// 中性档（持有/观望/不确定/无法判断）→ 不判定
    #[test]
    fn neutral_actions_not_judged() {
        for a in ["持有", "观望", "不确定", "无法判断", "HOLD", "WAIT", "UNCERTAIN"] {
            assert_eq!(
                deterministic_was_correct(Some(a), Some(&snap(-5.78, false))),
                None,
                "中性档 {a} 不应判定"
            );
        }
    }

    /// 期中观察（未到期望持有期）→ 不判定：短期噪声不是策略失效证据
    #[test]
    fn within_horizon_not_judged() {
        assert_eq!(deterministic_was_correct(Some("买入"), Some(&snap(-5.78, true))), None);
    }

    /// 无行情快照 / 无决策方向 → 不判定
    #[test]
    fn missing_data_not_judged() {
        assert_eq!(deterministic_was_correct(None, Some(&snap(-5.78, false))), None);
        assert_eq!(deterministic_was_correct(Some("买入"), None), None);
    }

    /// 收益为 NaN → 显式不判定（不等号对 NaN 恒 false，会误判成「未命中」）
    #[test]
    fn nan_return_not_judged() {
        assert_eq!(deterministic_was_correct(Some("买入"), Some(&snap(f64::NAN, false))), None);
        assert_eq!(deterministic_was_correct(Some("卖出"), Some(&snap(f64::NAN, false))), None);
    }

    /// 未知 action 字符串 → 不判定（normalize_action 返回 None）
    #[test]
    fn unrecognized_action_not_judged() {
        assert_eq!(deterministic_was_correct(Some("乱写"), Some(&snap(-5.78, false))), None);
    }
}
