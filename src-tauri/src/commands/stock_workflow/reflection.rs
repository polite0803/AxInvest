use super::core::fetch_stock_lessons;
use super::decision::{load_and_inject_template, resolve_runtime_options};
use super::serenity::extract_agent_output;
use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;
use axagent_agent_macro::agent_command;
use axagent_analysis_engine::recommender::Period;
use axagent_analysis_engine::reflection_stats::{
    HorizonStatus, default_expected_holding_days, deterministic_horizon_was_correct,
    parse_horizon_decisions,
};
use axagent_astock_data::as_of::{self, AsOfContext};
use axagent_entities::{stock_analyses, stock_reflections};
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

/// P3：构造注入反思模板的妖股漏检专项上下文（`{{mover_recall_context}}`）。
///
/// `None`（常规到期反思）⇒ 占位句：reflection.md 用自然语言声明忽略专项三问。
/// `Some(事件)` ⇒ 事实块：系统机械判定的权威归因层 + 窗口硬数字 + 固定三问要求。
///
/// 纯函数（事件字段都是标量）便于单测锁住措辞要点；三问口径与 plan P3 第 1 条逐字对齐：
/// 启动前可见信号 / 卡在哪一层（与权威层对照）/ 未来判据如何修（可执行）。
fn mover_recall_context_text(
    event: Option<&axagent_dao::repo::mover_recall_events::EventModel>,
) -> String {
    let Some(ev) = event else {
        return "（本次为常规到期反思，无妖股漏检事件背景：忽略漏检专项三问，按常规单档复盘即可。）"
            .to_string();
    };

    // 层码 → 给 LLM 看的中文语义（前端展示翻译另走 i18n，两处不同用途、不共享文案）。
    // pool_degraded 理论上不会走到（P2 落库即 excluded、不进补分析），保留兜底防沉默。
    let layer_semantic = match ev.miss_layer.as_str() {
        "not_in_pool" => "窗口起步时该股**未进入候选池**（入池判据/扫描覆盖遗漏）",
        "l3_scored_out" => "已入池但在 **L3 评分阶段被截断**（评分/阈值判据没让它进入推荐）",
        "other_period" => "窗口内有过其他档位的推荐（**档位错配**：推荐档与实际暴涨档不一致）",
        "unexplained" => "现有留痕不足以机械归因（你需要在结论中说明还缺哪类留痕才能判定）",
        "pool_degraded" => "核查时候选池数据可达性降级（数据质量问题，谨慎归因为算法判据）",
        other => other,
    };

    format!(
        "【妖股漏检回放背景】\n\
         本次复盘的不是一笔实盘推荐，而是一笔「窗口内涨幅达标、但系统当时漏掉」的历史**回放补分析**。\n\
         - 权威归因层（系统机械判定，不是你的自评）：{layer} —— {semantic}\n\
         - 达标窗口：{start} → {end}（{days} 个交易日），窗口累计涨幅 +{cum:.1}%，窗口内最大单日涨幅 +{daily:.1}%\n\
         - 本事件档位：{period}\n\n\
         【漏检专项三问 —— 除常规复盘内容外必须逐条回答】\n\
         ① missed_signals：只允许列窗口起点 {start} 当时系统**已经可见**的具体信号，逐条注明出处\
         （哪个分析师报告 / 哪类数据节点）；禁止写窗口右端才出现的事后信息，禁止泛泛罗列。\n\
         ② what_went_wrong：必须正面回答「卡在哪一层」——入池判据、L3 评分、还是档位错配，\
         并与上面的权威归因层（{layer}）对照：证据一致就解释该层判据为何漏掉它，\
         证据不一致要明确指出矛盾并给依据，既不得附和也不得无视权威层。\n\
         ③ fix_for_future：给出**可执行的判据修正**（调哪个入池条件 / 评分阈值 / 档位映射，\
         调整方向与触发条件），禁止「注意风险」「谨慎操作」「综合判断」这类不可执行的空话。",
        layer = ev.miss_layer,
        semantic = layer_semantic,
        start = ev.window_start,
        end = ev.anchor_date,
        days = ev.window_days,
        cum = ev.cum_gain_pct,
        daily = ev.max_daily_pct,
        period = ev.period,
    )
}

/// #10 P7：算本条逐档反思行的妖股标签（判据只在 `mover_recall::mover_label_for` 一处）。
///
/// 两条输入都取**既有权威**，不在这里另立口径：
///   · 阈值 = 面板变量的**实际服务值**（`load_reco_served_vars_db` ⇒ 用户在面板调过
///     `mover_gain_*` 后标签跟着变；`tier_rules_from_vars` 就是达标核查用的同一个组装函数）；
///   · 窗口是否已满 = 该档快照自己的 `within_expected_horizon`（不是第二份天数表）。
///
/// 两处「拿不到」分开写、都不冒充 `normal`：快照不可得 ⇒ `no_market_data`；
/// 阈值判据不可用（变量缺失/非正，或变量表读取失败 —— 两种成因在日志里分述）⇒ `rule_unavailable`。
async fn compute_mover_label(
    db: &sea_orm::DatabaseConnection,
    horizon: &str,
    snap: Option<&MarketSnapshot>,
) -> Option<String> {
    use axagent_analysis_engine::mover_recall::{mover_label_for, tier_rules_from_vars};
    let served = match axagent_analysis_engine::recommender::reco_loop::load_reco_served_vars_db(db)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "[reflection] 妖股阈值变量读取失败（本行标签记为 rule_unavailable，不冒充未达标）: {e}"
            );
            return Some("rule_unavailable".to_string());
        },
    };
    let rules = tier_rules_from_vars(&served);
    let threshold = rules.iter().find(|r| r.period.as_str() == horizon).map(|r| r.gain_pct);
    let gross = snap.map(|s| s.price_change_pct);
    // 「已满」的判据用快照自带的布尔位；无快照时由 mover_label_for 走 no_market_data 分支
    let complete = snap.map(|s| !s.within_expected_horizon).unwrap_or(false);
    match mover_label_for(threshold, gross, complete) {
        Some(label) => Some(label.to_string()),
        None => {
            tracing::warn!(
                "[reflection] 档位 {horizon} 的 mover_gain_* 阈值缺失或非正 ⇒ 该档判据不成立，本行记 rule_unavailable"
            );
            Some("rule_unavailable".to_string())
        },
    }
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
    // 〇-B v2 第 4 条：本次反思**只复盘一个周期档**。
    //   `Some(x)` = 调用方显式指定（手动反思 / 分档定时任务）；
    //   `None`    = 沿用原分析的**公式主档** `decision_time_horizon`（v2 下恒为 formula 产出）。
    //   值域 = `Period::as_str()`；命令入口已校验，非法值不会进到这里。
    review_horizon: Option<&str>,
    // [B2/B3 借鉴] 反思 row ID(B1 阶段落盘的 pending row)。
    // 传入则 UPDATE 现有 row;传 None 则按 v1 行为 INSERT 新 row,保持旧调用方兼容。
    reflection_id: Option<String>,
    // [方向3] 轨迹存储，用于持久化反思执行轨迹。
    // 传 None 则跳过 Trajectory 持久化（手动反思等不需要轨迹的场景）。
    trajectory_storage: Option<&std::sync::Arc<axagent_trajectory::TrajectoryStorage>>,
    // [实际行情] 「当前实际行情」四周期快照 —— 由调用方在反思前用 `compute_market_snapshots`
    // 一次性拉取 K 线并拆分四个目标交易日窗口确定性算出（见 PLAN-stock-reflection-four-horizon 批次 3）。
    // 承载每周期独立的价格层事实：入场基准价 / 周期末收盘价 / 涨跌幅 / 最大回撤 / 超额收益 / 目标价实现度。
    //
    // Key = 周期名（ultra_short / short / mid / long），Value = Some(MarketSnapshot) 或 None（该周期行情不可用）。
    horizon_snapshots: &std::collections::BTreeMap<String, Option<MarketSnapshot>>,
) -> Result<String, String> {
    use sea_orm::sea_query::Expr;

    let now_ms = chrono::Utc::now().timestamp_millis();

    // ── [B2 借鉴] 幂等守卫: 如果 reflection_id 已 completed,直接返回 cached ──
    // §七十一 按版归属：顺手把 pending 行的**建点盖章**带出来（`extract_lesson_to_rule` 用它）。
    let mut pending_generation: Option<i32> = None;
    if let Some(ref rid) = reflection_id {
        if let Some(existing) =
            stock_reflections::Entity::find_by_id(rid.clone()).one(db).await.map_err(|e| {
                ErrorResponse::new(wf_err::INTERNAL)
                    .with_detail(format!("B2 查询已存在反思失败: {e}"))
            })?
        {
            // 这一行的 `template_version` 是「被复盘那笔决策当时属于哪一代」的唯一残留：
            // 分析行此后若被重跑，它的 `template_version` 会跟着变新一代，而本行复盘的仍是
            // 老一代的结论 ⇒ 归属必须跟着建点盖章走，不能跟着分析行的现在时。
            pending_generation = existing.template_version;
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
            // 〇-B v2 第 4 条：兼容旧调用方的 INSERT 路径同样显式置 NULL，
            //   复盘档由反思收尾按 `primary_horizon` 盖章（见 HorizonResultsJson 处）。
            horizon: Set(None),
            actual_outcome: Set(actual_outcome.to_string()),
            // v008 (C3 借鉴): 4 个结构化 outcome
            raw_return: Set(raw_return),
            mover_label: Set(None),
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
            horizon_results_json: Set(None),
            decision_json: Set(None),
            blackboard_snapshot: Set(None),
            // §七十一 按版归属：本 INSERT 分支只有**手动反思**会走到，而那条路径
            // `original_analysis_id` 恒为空串（「手动触发时无原始决策」，见 `stock_analysis.rs`
            // 的 `run_reflection_now` 传参）⇒ 代际是**按构造天然未知**，与 chat 通道
            // 「有分析但链路不经过 workflow_templates」的 NULL 是两种成因，不可混记；
            // 两者都不得被读侧当成「第 0 代」。业务反思的代际由建 pending 行时盖
            // （`build_pending_reflection_rows_for`），UPDATE 路径不改写它。
            template_version: Set(None),
            // P3：手动反思 INSERT 兼容路径恒为常规到期反思；漏检回放反思只可能经
            // pending 行 UPDATE 路径产生（建点在 `build_pending_reflection_rows_for`）。
            source: Set("due".to_string()),
            miss_layer: Set(None),
            failure_modes: Set(None),
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
    // 但两处 prompt 都会 hard-code 引用变量，缺一个就整条链 Failed：
    //   - 专家档案 reflection.md「你收到的输入包括」+「单档复盘原则」两节引用
    //     {{review_horizon}} / {{review_expected_holding_days}} / {{analysis_primary_horizon}}
    //     / {{actual_outcome}} / {{reflection_depth}} / {{stock_lessons}} / {{tunable_params_catalog}}；
    //   - reflection-comparator 的 input_mapping 另消费 original_time_horizon /
    //     original_holding_days（值已按复盘档口径，见下方 `review_days`）。
    // (缺失会 work_engine 报 VARIABLE_NOT_FOUND,reflection-agent 节点 Failed,
    // 数据库 what_went_wrong 等字段全 null)。
    let original_analysis: Option<stock_analyses::Model> = if original_analysis_id.is_empty() {
        None
    } else {
        stock_analyses::Entity::find_by_id(original_analysis_id).one(db).await.ok().flatten()
    };

    // §七十一 按版归属：本代 = **被复盘那条决策**所属的算法代际。
    //   权威是 `stock_analyses.template_version`（单一写入口径不变），这里取一次、供
    //   规则化 lesson 建点盖章。`None` = 代际未知，两种成因都要如实保留：手动反思
    //   （`original_analysis_id` 空 ⇒ 连被复盘的决策都没有）与 chat 通道（分析行本身 NULL，
    //   A4 裁定）。**不得**退回编译期 `TEMPLATE_VERSION` 冒充 —— 那会把「反思发生在哪一代」
    //   误记成「被复盘的决策属于哪一代」，而反思跑在旧代分析上恰恰是重放场景的常态。
    //   ⚠ 优先级：pending 行的建点盖章 > 分析行的现在值（后者可能被重跑改写过，见上方 B2 段）。
    let reflected_generation: Option<i32> =
        pending_generation.or(original_analysis.as_ref().and_then(|a| a.template_version));

    let original_ctx: Option<(String, i64)> = original_analysis.as_ref().and_then(|a| {
        let t = a.decision_time_horizon.clone()?;
        let h = a.decision_expected_holding_days?;
        Some((t, h))
    });

    // ── P3 妖股漏检回放：取权威归因事件 ──
    // 仅被复盘分析行是 mover_recall 补分析时查（anchor_event_id 在 P2 补分析落行时盖章）。
    // 查不到只 warn 不阻断：反思仍按常规流程跑，只是漏检专项上下文退化为占位句。
    let mover_event: Option<axagent_dao::repo::mover_recall_events::EventModel> = match (
        original_analysis.as_ref(),
        original_analysis.as_ref().and_then(|a| a.anchor_event_id.as_deref()),
    ) {
        (Some(a), Some(eid)) if a.source == REFLECTION_SOURCE_MOVER_RECALL => {
            match axagent_dao::repo::mover_recall_events::fetch_by_id(db, eid).await {
                Ok(ev) => {
                    if ev.is_none() {
                        tracing::warn!(
                            "[reflection] 分析 {original_analysis_id} 标注 anchor_event_id={eid} \
                             但 mover_recall_events 查无此行 ⇒ 漏检专项上下文降级为占位句"
                        );
                    }
                    ev
                },
                Err(e) => {
                    tracing::warn!("[reflection] 查询漏检事件 {eid} 失败（不阻断反思）: {e}");
                    None
                },
            }
        },
        _ => None,
    };
    let mover_recall_context = mover_recall_context_text(mover_event.as_ref());

    // 派生主周期快照：用于 comparator 顶层变量注入 + strategy_performance 单条写入
    let primary_horizon = review_horizon.unwrap_or_else(|| {
        original_analysis.as_ref().and_then(|a| a.decision_time_horizon.as_deref()).unwrap_or_else(
            || {
                tracing::warn!(
                    "[reflection] 无显式复盘档且原分析无主档 ⇒ 按 short 复盘（代理，非事实陈述）"
                );
                "short"
            },
        )
    });
    let primary_snapshot = horizon_snapshots.get(primary_horizon).and_then(|s| s.as_ref());

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
        // 内联 system_prompt (src/commands/stock_analysis_setup/mod.rs:1702) 引用了
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
                // 〇-B v2 第 4 条：反思自身也只喂**本次复盘档**的教训
                // §五十一-②：代际维度同理只喂**起算代及之后**（+ 代际未知但会被声明）的教训 ——
                // 用旧代算法总结出的规则去评判新代决策，判据就不是同一条了。
                // 起算代在 `fetch_stock_lessons` 内部解析（与统计侧同形）。
                fetch_stock_lessons(stock_code, db, Some(primary_horizon))
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
    let (market_text, market_json) = match primary_snapshot {
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

    // 〇-B v2 第 4 条：把「本次只复盘这一档」显式交给生成层。
    //   值取盖章后的 `primary_horizon`（不是入参原值）⇒ 手动 / 批量两条路径同源，
    //   不会出现行上标 A 档、prompt 里写 B 档。
    // 本次复盘档的期望持有天数：该档快照自带，行情不可用时退回 Period 默认天数表。
    //   ⚠ comparator 的 `horizon_mismatch` 用 (original_time_horizon, original_holding_days)
    //   与 `holding_days` 三者对比（`reflection-comparator.rhai:192`）⇒ 复盘档既已收窄成
    //   单一档位，这一对就必须**同档**，否则复盘 long 时拿主档 5 天去比 90 天窗口。
    let review_days = primary_snapshot
        .and_then(|s| s.expected_holding_days)
        .unwrap_or_else(|| default_expected_holding_days(primary_horizon));
    variables.push(axagent_harness::workflow_types::Variable {
        name: "review_horizon".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(primary_horizon.to_string()),
        description: Some("本次反思唯一复盘的周期档（ultra_short/short/mid/long）".into()),
        is_secret: false,
    });
    variables.push(axagent_harness::workflow_types::Variable {
        name: "review_expected_holding_days".into(),
        var_type: "number".into(),
        value: serde_json::json!(review_days),
        description: Some("本次复盘档的期望持有天数（交易日）".into()),
        is_secret: false,
    });

    // ── P3 妖股漏检回放专项上下文（无条件注入）──
    // reflection.md 的「漏检回放专项复盘」节引用 {{mover_recall_context}}，缺注入会
    // VARIABLE_NOT_FOUND 使 reflection-agent 节点 Failed ⇒ 常规到期反思也必须注入，
    // 此时值为占位句，模板用自然语言声明「忽略专项三问」。
    variables.push(axagent_harness::workflow_types::Variable {
        name: "mover_recall_context".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(mover_recall_context),
        description: Some(
            "妖股漏检回放背景（权威归因层/窗口事实/专项三问）；常规到期反思为占位句".into(),
        ),
        is_secret: false,
    });

    // ── P2-b 失败模式受控词表（无条件注入）──
    // 词表由引擎侧 FailureMode 渲染，模板不抄值 ⇒ 增删一个模式只改一处（反手抄漂移）。
    variables.push(axagent_harness::workflow_types::Variable {
        name: "failure_mode_catalog".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(
            axagent_analysis_engine::failure_mode::FailureMode::catalog_for_prompt(),
        ),
        description: Some("失败模式受控词表（反思 failure_modes 字段的唯一值域）".into()),
        is_secret: false,
    });

    if let Some((time_horizon, _analysis_holding_days)) = original_ctx {
        variables.push(axagent_harness::workflow_types::Variable {
            name: "original_time_horizon".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(time_horizon.clone()),
            description: Some(
                "本次复盘档（反思链与 review_horizon 同源，供既有模板继续消费）".into(),
            ),
            is_secret: false,
        });
        variables.push(axagent_harness::workflow_types::Variable {
            name: "original_holding_days".into(),
            var_type: "number".into(),
            // 名字保留 `original_*` 是给既有 comparator / 模板继续消费；**值**已换成复盘档口径
            value: serde_json::json!(review_days),
            description: Some(
                "本次复盘档的期望持有天数（交易日；与 review_expected_holding_days 同值）".into(),
            ),
            is_secret: false,
        });
        // 复盘档 ≠ 原分析主档时（用户手动逐档复盘），必须把两者分开陈述：
        // 混为一谈会让 LLM 拿别的档的决策口径来解释本档结果。
        variables.push(axagent_harness::workflow_types::Variable {
            name: "analysis_primary_horizon".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(time_horizon),
            description: Some("原分析公式定档出的主周期档（未必等于本次复盘档）".into()),
            is_secret: false,
        });
    } else {
        // 手动反思场景:无原始分析上下文,但 prompt 模板必须能渲染。
        // 注入占位值(让 LLM 知道这是手动触发的独立反思,无持仓期对齐数据)。
        variables.push(axagent_harness::workflow_types::Variable {
            name: "original_time_horizon".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(primary_horizon.to_string()),
            description: Some("本次复盘档(手动反思无原始分析,与 review_horizon 同值)".into()),
            is_secret: false,
        });
        variables.push(axagent_harness::workflow_types::Variable {
            name: "original_holding_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(review_days),
            description: Some(
                "本次复盘档的期望持有天数（手动反思无原始分析，与 review_expected_holding_days 同值）".into(),
            ),
            is_secret: false,
        });
        variables.push(axagent_harness::workflow_types::Variable {
            name: "analysis_primary_horizon".into(),
            var_type: "string".into(),
            // 手动反思无原分析 ⇒ 无主档可陈述。值必须是显式字面量，不能漏注入：
            // reflection.md 引用该变量，缺失会让 work_engine 报 VARIABLE_NOT_FOUND 使整条链 Failed。
            value: serde_json::Value::String("none".into()),
            description: Some("原分析主周期档(手动反思场景无原始分析,固定为 'none')".into()),
            is_secret: false,
        });
        tracing::info!(
            "[reflection] {}: 手动反思场景,复盘档 {primary_horizon},注入 original_holding_days=0",
            stock_code
        );
    }

    // ── 四周期行情事实 + 周期决策（批次 3）──
    // 将每个周期的 MarketSnapshot 序列化为 JSON 注入 comparator，
    // 供 Rhai 脚本逐周期做 horizon_correct 确定性判定。
    {
        let mut facts = serde_json::Map::new();
        for (key, snap_opt) in horizon_snapshots.iter() {
            facts.insert(
                key.clone(),
                snap_opt
                    .as_ref()
                    .map(|s| serde_json::to_value(s).unwrap_or_default())
                    .unwrap_or(serde_json::Value::Null),
            );
        }
        variables.push(axagent_harness::workflow_types::Variable {
            name: "horizon_market_facts".into(),
            var_type: "object".into(),
            value: serde_json::Value::Object(facts),
            description: Some(
                "四周期行情快照 {ultra_short/short/mid/long: MarketSnapshot|null}（批次 3）".into(),
            ),
            is_secret: false,
        });
    }

    // 四周期决策源 JSON（原样透传给 comparator，用于 horizon_correct 阶段注入）
    let horizon_decisions_raw =
        original_analysis.as_ref().and_then(|a| a.horizon_decisions.as_deref()).unwrap_or("");
    variables.push(axagent_harness::workflow_types::Variable {
        name: "horizon_decisions_json".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(horizon_decisions_raw.to_string()),
        description: Some(
            "四周期决策 JSON 源串（原样透传给 comparator，用于 horizon_correct 注入）".into(),
        ),
        is_secret: false,
    });

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

            // P2-b：失败模式白名单化。列值与「模板未产出」的区分口径见 `failure_modes_column_of`；
            // 被丢弃的词表外 token 只记日志（含本行 id），面板的「词表外归因」另有来源（见引擎模块注释）。
            let fm_raw = deep_get_reflection_text(&reflection_json, "failure_modes");
            let (failure_modes_column, fm_rejected) = failure_modes_column_of(fm_raw.as_deref());
            if !fm_rejected.is_empty() {
                tracing::warn!(
                    "[reflection] 反思自报了词表外的失败模式，已丢弃：{:?}（analysis_id={}）",
                    fm_rejected,
                    analysis_id
                );
            }

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

            // ── 四周期反思结果 JSON 构建（批次 2）──
            let horizon_json = build_horizon_results_json(
                original_analysis.as_ref(),
                horizon_snapshots,
                &reflection_json,
            );

            // #10 P7：本行妖股标签 —— 用**本行自己的档与快照**（primary_snapshot 与 raw_return
            // 同源），不引第二套窗口天数表；判据单源见 compute_mover_label。
            let mover_label = compute_mover_label(db, primary_horizon, primary_snapshot).await;

            let _ = stock_reflections::Entity::update_many()
                .col_expr(stock_reflections::Column::Status, Expr::value(&status_text))
                .col_expr(stock_reflections::Column::DecisionJson, Expr::value(dj_text))
                .col_expr(
                    stock_reflections::Column::HorizonResultsJson,
                    // clone：UPDATE 后还要用同一份 JSON 做 #31 四档成熟样本物化。
                    Expr::value(horizon_json.clone()),
                )
                // 〇-B v2 第 4 条：本行**只复盘这一档**（其余三档只有客观判定，
                // 没有反思叙述/教训）。该章是 lesson 与档位参数建议的归属依据。
                .col_expr(
                    stock_reflections::Column::Horizon,
                    Expr::value(Some(primary_horizon.to_string())),
                )
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
                // #10 P7：四态标签；NULL=尚未走到收尾（pending/running/failed 或本列引入前的存量行）
                .col_expr(stock_reflections::Column::MoverLabel, Expr::value(mover_label))
                // P3：权威归因层盖章。仅 mover_recall 反思（mover_event=Some）写层码，
                // 这是**系统机械判定**不是 LLM 自报；常规到期反思恒 NULL。
                .col_expr(
                    stock_reflections::Column::MissLayer,
                    Expr::value(mover_event.as_ref().map(|e| e.miss_layer.clone())),
                )
                // P2-b：失败模式受控标注（值域权威 = analysis_engine::failure_mode::FailureMode，
                // 白名单外的自造 token 不落本列，只记日志）。NULL = 模板未产出该字段。
                .col_expr(
                    stock_reflections::Column::FailureModes,
                    Expr::value(failure_modes_column),
                )
                .filter(stock_reflections::Column::Id.eq(&analysis_id))
                .exec(db)
                .await;

            // P2-a：本次反思刚产出一条「腿信号 × 到期结果」配对 ⇒ 顺手刷新预测力报告（裁决层写侧）。
            // 刷新失败只降级为「权重仍按先验」，不影响本次反思落库 ⇒ 不向上传错。
            if let Err(e) = refresh_leg_ic_report(db).await {
                tracing::warn!("[leg_ic] 报告刷新失败（权重仍按先验）: {e}");
            }

            // ── Path 2: 反思参数建议自动解析 ──
            // D1 修复：verdict 改走 deep_get（兼容「一层嵌套」与「双重编码字符串」）。
            let verdict_opt = deep_get_reflection_str(&reflection_json, "verdict");
            let verdict_str = verdict_opt.as_deref().unwrap_or("");

            // ── Gap 1: 确定性判定 → Strategy Performance（P1-1②：与失败路径共用同一幂等落库）──
            // M1 改写：was_correct 由行情快照**确定性反推**（`deterministic_was_correct`），
            // 消除 LLM 幻觉污染；「未判定」与「判定为错」在数据上可区分 ——
            // 无法判定（无行情 / 中性档 / 期中观察）→ **不写行**（strategy_performance
            // 的 was_correct 是 i32 非空列，表达不了第三态；「没写入」即「没有判定依据」）。
            // 原实现整块（构造 17 列 + insert）2026-10-16 抽为 `persist_m1_sample_if_absent`，
            // 使失败路径（工作流 Err）也能留下确定性战绩。
            match persist_m1_sample_if_absent(
                db,
                &M1SampleInputs {
                    reflection_id: &analysis_id,
                    original_analysis: original_analysis.as_ref(),
                    primary_snapshot,
                    stock_code,
                    stock_name,
                    raw_return,
                    holding_days,
                    primary_horizon,
                    reflected_generation,
                    now_ms,
                },
            )
            .await
            {
                Ok(true) => {},
                Ok(false) => tracing::warn!(
                    "[reflection] {}: 无判定依据或该反思行已有样本，跳过 strategy_performance 写入",
                    stock_code
                ),
                Err(e) => tracing::warn!("[reflection] 写入 strategy_performance 失败: {e}"),
            }

            // ── #31 代际血缘管道：四档成熟样本 → 真实策略格 strategy_performance ──
            // 与上方 Gap1 的分工：Gap1 只写 primary 一档到 `reflection_verdict` 格（供既有
            // dashboard / 前缀消费方），其 strategy_id 不在参数进化权重矩阵里；这里把
            // horizon_results_json 中**每一个成熟可判定档**按 action→真实策略格
            // （trend/value/capital/reversion/watchlist）× 真实档（ultra_short/short/mid/long）
            // 幂等 upsert，并盖被复盘决策的真实代际，供 load_performance_window 喂参数进化。
            // best-effort：物化失败只告警，不阻断反思主流程落库。
            // 物化必须锚到「被复盘决策」（original_analysis）—— 幂等键用决策 id 而非本反思行
            // id（一行一档：同一决策的短档会在多条反思行重复成熟，按决策去重才不重复计票），
            // decision_at 取决策建点时刻。拿不到原始决策（如手动触发无决策）或旧记录无四周期
            // JSON（horizon_json=None）时无血缘可物化，直接跳过（旧记录走 legacy / Gap1）。
            if let (Some(oa), Some(horizon_json)) =
                (original_analysis.as_ref(), horizon_json.as_ref())
            {
                match axagent_analysis_engine::evolution_drift::upsert_reflection_horizon_performance(
                    db,
                    &oa.id,
                    stock_code,
                    stock_name,
                    oa.created_at,
                    now_ms,
                    horizon_json,
                    reflected_generation,
                )
                .await
                {
                    Ok(n) if n > 0 => tracing::info!(
                        "[reflection] #31 四档成熟样本物化 {n} 条 → strategy_performance\
                         （analysis={} reflection={analysis_id}）",
                        oa.id
                    ),
                    Ok(_) => tracing::debug!(
                        "[reflection] #31 暂无成熟可判定档，未物化\
                         （analysis={} reflection={analysis_id}）",
                        oa.id
                    ),
                    Err(e) => {
                        tracing::warn!("[reflection] #31 四档样本物化失败（不阻断反思落库）: {e}")
                    }
                }
            }

            // ── Gap 3: 同方向建议的留痕计数（当前**不自动执行**任何校准）──
            // 逐参数比方向，且只认「这条建议确实提了该参数」的行。
            // 2026-10-14 两处更正：① 旧实现用 ParamSet 形态判方向，而解析器当时对任何非空
            //   数组都返回出厂默认 ⇒ `suggested < default` 与 `prev < default` 恒同为 false，
            //   「连续同方向」是必然成立的假信号；② 旧注释称「实际自动校准由 scheduler/cron
            //   层接管」并不存在（全仓调 `calibrate_portfolio_mgr_params` 的只有漂移面板按钮），
            //   本分支从头到尾只写日志，措辞按事实改为「仅留痕」。
            if verdict_str == "wrong" || verdict_str == "partial" {
                use axagent_analysis_engine::portfolio_formula::parse_param_suggestion_entries;
                let current_entries = match params_suggestion_json.as_deref() {
                    Some(pj) => parse_param_suggestion_entries(pj),
                    None => Vec::new(),
                };
                if !current_entries.is_empty() {
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
                        for (name, value, base) in &current_entries {
                            // 方向 = 建议值相对「清单当时的现值」升降。清单现值由变量表派生
                            // （`build_tunable_params_catalog`），与面板改过的库里值可能有差
                            // ⇒ 本计数只作留痕，不作为自动执行依据。
                            let Some(base) = base else {
                                tracing::info!(
                                    "[reflection] Gap3: 参数 {name} 未带 current_value ⇒ 方向不可判，跳过计数"
                                );
                                continue;
                            };
                            let cur_dir = *value < *base;
                            let mut same_direction = 1usize; // 当前这条算 1
                            for r in &rows {
                                if r.id == analysis_id {
                                    continue;
                                }
                                let pj =
                                    r.parameter_suggestions_json.as_deref().unwrap_or_default();
                                let prev = parse_param_suggestion_entries(pj);
                                match prev.into_iter().find(|(n, _, _)| n == name) {
                                    Some((_, pv, Some(pbase))) if (pv < pbase) == cur_dir => {
                                        same_direction += 1;
                                    },
                                    _ => break, // 该参数没被提 / 方向不同 / 现值缺失 ⇒ 连续性断开
                                }
                            }
                            if same_direction >= 3 {
                                tracing::info!(
                                    "[reflection] Gap3: 参数 {name} 连续 {same_direction} 条同方向建议（{}），\
                                     仅留痕 —— 自动校准通路不存在，唯一入口是漂移面板按钮",
                                    if cur_dir { "降低" } else { "提高" }
                                );
                            }
                        }
                    }
                } else if params_suggestion_json.is_some() {
                    // 有建议文本、但一个可校准参数都没提到（自造名字或 `[]`）⇒ 显式留痕，
                    // 不让「反思说过参数」被下游当成「已参与优化」。
                    tracing::info!(
                        "[reflection] Gap3: 本次 params_suggestion 无可校准参数，不参与同方向统计"
                    );
                }
            }

            // 索引到 Memory RAG
            if let Some(ref w) = what_went_wrong {
                // 检索片段必须自带**复盘档**：向量检索按语义相似度命中，不看档位
                // （`search_with_filter` 只能按 document_id 列表过滤，档不是 document_id）。
                // 与其改检索接口，不如让每条片段自述档位 ⇒ LLM 能据此拒绝跨档套用，
                // 也让「另一档的教训」在上下文里就是可识别的（缺席不冒充通用）。
                let memory_content = format!(
                    "反思:股票:{} {} 复盘档:{} 分析日:{} 该档结果:{}\n错因:{}",
                    stock_code, stock_name, primary_horizon, as_of_date, actual_outcome, w
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
                        primary_horizon,
                        // §七十一：代际来自被复盘的分析行，不是 LLM 输出、也不是当前代码版本
                        reflected_generation,
                    )
                    .await;
                }
            }

            // ── P3 妖股漏检事件闭环盖章 ──
            // 一个事件对应多档反思行（四档各一行），只让**首个 completed 的档**盖章：
            // UPDATE 带 `status != 'reflected'` 守卫形成行级竞争，rows_affected=0 即
            // 「已被更早完成的档盖章」，这是正常幂等不是冲突。reflection_id 记盖章档的行 id。
            if status_text == "completed" {
                if let Some(ev) = mover_event.as_ref() {
                    // UPDATE 守卫 `status != 'reflected'` 在 dao::stamp_reflected 内：
                    // rows=0 即「已被更早完成的档盖章」，正常幂等不是冲突。
                    match axagent_dao::repo::mover_recall_events::stamp_reflected(
                        db,
                        &ev.id,
                        Some(&analysis_id),
                        now_ms,
                    )
                    .await
                    {
                        Ok(rows) if rows > 0 => tracing::info!(
                            "[reflection] 漏检事件 {} 已由本档（{}）反思完成，盖章 reflected",
                            ev.id,
                            primary_horizon
                        ),
                        Ok(_) => tracing::info!(
                            "[reflection] 漏检事件 {} 已由更早 completed 的档盖章，本行不重复回填",
                            ev.id
                        ),
                        Err(e) => tracing::warn!(
                            "[reflection] 漏检事件 {} 盖章 reflected 失败（反思结论已落库，不影响）: {e}",
                            ev.id
                        ),
                    }
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

            // P1-1②（PLAN-stock-decision-reliability R4/R12）：工作流失败 ≠ 无样本 ——
            // M1 的原料（主档动作 + 行情快照）在失败前已具备 ⇒ 借确定性判定留下战绩，
            // 让「LLM 侧失败」不再等于系统对已发生的错误完全失忆；幂等（重试成功后
            // 的二次物化会被同反思行查重跳过）。
            match persist_m1_sample_if_absent(
                db,
                &M1SampleInputs {
                    reflection_id: &analysis_id,
                    original_analysis: original_analysis.as_ref(),
                    primary_snapshot,
                    stock_code,
                    stock_name,
                    raw_return,
                    holding_days,
                    primary_horizon,
                    reflected_generation,
                    now_ms,
                },
            )
            .await
            {
                Ok(true) => tracing::info!(
                    "[reflection] {}: 失败路径已补落 M1 确定性样本（reflection_id={analysis_id}）",
                    stock_code
                ),
                Ok(false) => {},
                Err(e2) => {
                    tracing::warn!("[reflection] 失败路径 M1 物化异常（不影响 failed 落库）: {e2}")
                },
            }

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
/// P0-8 死信重试：把可恢复的 failed 反思行**原地**置回 pending，由下一轮到期反思自动消费。
///
/// - `ids` 为 None 时扫描全部 failed 行；传入 id 列表时仅重试指定行（前端勾选/单条重试）。
/// - 两类结构性终局（`ORIGINAL_ANALYSIS_MISSING` / `ANALYSIS_SNAPSHOT_MISSING`）不重试，
///   原样在 `skippedTerminal` 返回；快照缺失需先重跑原分析。
/// - 原地 UPDATE 同一行（不新建），天然幂等；重复调用只是再次把行置回 pending。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description = "重试失败的持仓反思")]
#[tauri::command]
pub async fn retry_failed_reflections(
    state: State<'_, AppState>,
    ids: Option<Vec<String>>,
    limit: Option<u32>,
) -> Result<serde_json::Value, String> {
    let db = state.harness.db();
    // 无 attempts 列：用单轮上限控制节奏，避免持续失败行每轮被重置空烧 LLM 配额。
    let limit = u64::from(limit.unwrap_or(20)).clamp(1, 200);
    let outcome = axagent_dao::repo::stock_lesson_queries::requeue_failed_reflections(
        db,
        chrono::Utc::now().timestamp_millis(),
        ids.as_deref(),
        limit,
    )
    .await
    .map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("重试失败反思行失败: {e}"))
    })?;
    tracing::info!(
        "[D1 retry_failed_reflections] 置回 pending {} 条，结构性终局跳过 {} 条",
        outcome.requeued.len(),
        outcome.skipped_terminal.len()
    );
    Ok(serde_json::json!({
        "requeued": outcome.requeued,
        "requeuedCount": outcome.requeued.len(),
        "skippedTerminal": outcome.skipped_terminal,
        "skippedTerminalCount": outcome.skipped_terminal.len(),
        "note": "已置回 pending，将由下一轮到期反思自动消费；快照缺失需先重跑原分析",
    }))
}

#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description =  "批量处理持仓到期反思")]
#[tauri::command]
pub async fn run_batch_reflection(
    state: State<'_, AppState>,
    max_count: Option<u32>,
) -> Result<serde_json::Value, String> {
    use axagent_entities::stock_analyses;

    let max_count = max_count.unwrap_or(20) as usize;
    let db = state.harness.db();
    // 自愈：把「原始分析已不存在」的孤儿 pending 行落成终局，必须在扫 pending 之前
    //（与下方 reclaim_stale_running 同一处置时点）。它们若留在 pending 就会被每轮重复捞起。
    let orphans_retired =
        match axagent_dao::repo::stock_lesson_queries::retire_orphan_pending_reflections(
            db,
            chrono::Utc::now().timestamp_millis(),
        )
        .await
        {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("[D1] 孤儿反思行自愈失败（本轮仍会照常处理其它行，下轮重试）: {e}");
                0
            },
        };

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
    let mut orphans = 0u32; // 原始分析已不存在 ⇒ 本轮落成终局，退出队列
    let mut errors: Vec<String> = Vec::new();
    let today_ms = chrono::Utc::now().timestamp_millis();

    for (i, p) in pendings.iter().take(max_count).enumerate() {
        // 2a. 读原始分析
        let analysis =
            match stock_analyses::Entity::find_by_id(&p.original_analysis_id).one(db).await {
                Ok(Some(a)) => a,
                Ok(None) => {
                    // 原始分析已不存在 ⇒ 这是**孤儿行**，不是「太新未到期」。旧形态只
                    // `skipped_young += 1` 后 continue ⇒ 状态仍留 pending，每天 18:00 重新
                    // 捞到、重新空转，队列永不自愈（现网实证：6 条 7 月的行连续数月
                    // processed=6 / skippedYoung=6，而它们的到期日早已过去两个月）。
                    tracing::warn!(
                        "[D1] pending reflection {} 关联 analysis_id={} 不存在 ⇒ 置孤儿终局",
                        p.id,
                        p.original_analysis_id
                    );
                    // 不在此写库：孤儿终局化由本轮开头的 dao 自愈统一做（两个入口都已接），
                    // 这里只如实计数，下一轮开头会把它们处置掉。
                    orphans += 1;
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

        // 2c. [实际行情] 四周期行情拉取 + 反思调用 → perform_single_reflection（技术债已清）。
        let r = perform_single_reflection(
            "D1",
            db,
            &state.astock_client,
            &state.work_engine,
            &state.vector_store,
            state.harness.master_key(),
            Some(&state.trajectory_storage),
            p,
            &analysis,
            analysis_date,
            today_nd,
            days_held,
            p.reflection_depth.as_str(),
        )
        .await;

        match r {
            Ok(ReflectionOutcome::SkippedYoung) => {
                skipped_young += 1;
                continue;
            },
            Ok(ReflectionOutcome::Completed) => {
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
        "[D1 batch_reflection] 完成: total={} resolved={} failed={} skipped_young={} \
         orphans={} cleaned={}",
        pendings.len(),
        resolved,
        failed,
        skipped_young,
        orphans,
        cleaned_up
    );

    Ok(serde_json::json!({
        "totalPending": pendings.len(),
        "processed": pendings.len().min(max_count),
        "resolved": resolved,
        "failed": failed,
        "skippedYoung": skipped_young,
        "orphans": orphans,
        // 本轮开头由 dao 自愈落成终局的孤儿行数（与上面的"本轮撞到但没写库"分两件报）
        "orphansRetired": orphans_retired,
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
    // 本次反思复盘的档位（〇-B v2 第 4 条：一次反思 = 一个周期）。
    // 由调用点传入 `primary_horizon`，**不是** LLM 自由填写的字段。
    horizon: &str,
    // 被复盘那条决策的代际（§七十一 按版归属）。同样**不是** LLM 字段，由调用点从
    // `stock_analyses.template_version` 取；`None` = 代际未知，按原样落 NULL。
    analysis_template_version: Option<i32>,
) -> Result<(), String> {
    use axagent_entities::reflection_lessons;
    use sea_orm::ActiveModelTrait;
    use sea_orm::ColumnTrait;
    use sea_orm::EntityTrait;
    use sea_orm::QueryFilter;
    use sea_orm::Set;

    // P3 入模前校验：长度（10..=200，按字符非字节）/ 行动词 / 空洞短语 / 纯疑问。
    // 与旧实现的区别：① 上界 250 → 200，与 Rhai 硬裁决及 prompt 的 ≤200 对齐；
    // ② len()（UTF-8 字节，中文×3）→ chars().count()；
    // ③ 不再静默跳过——不合格也建行**留档**（status=validation_rejected、confidence=0、
    //   驳回原因写进 rule_pattern），消费侧只查 active+confidence>0.5 ⇒ 天然不生效，
    //   面板上仍能看到「这条为什么没入模」。
    let trimmed = lesson_summary.trim();
    let admission =
        axagent_analysis_engine::reflection_lesson_validator::validate_lesson_for_admission(
            trimmed,
        );

    // [P2-#9 修复] 去重：检查相同 lesson_summary 是否已存在
    // 同一只股票多次相似反思会产生大量重复规则，此处按 stock_code + lesson_summary 去重。
    // 若已存在，更新 source_reflection_id 和 updated_at（保留原有 times_applied/success_count）。
    let existing = reflection_lessons::Entity::find()
        .filter(reflection_lessons::Column::StockCode.eq(stock_code))
        .filter(reflection_lessons::Column::LessonSummary.eq(trimmed))
        // 去重键含 horizon：同一条文字教训在**不同档**是两条规则（短线与长线的
        // 「止损节奏」本就不同），合表会把两条压成一条并静默丢掉另一档的统计。
        .filter(reflection_lessons::Column::Horizon.eq(horizon))
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("F1 查询重复 lesson 失败: {e}"))
                .to_string()
        })?;

    if let Some(existing_model) = existing {
        // 已存在相同规则，更新 source_reflection_id 和 updated_at，保留应用统计
        // ⚠ `template_version` **不在**这里改写（§七十一）：同一句教训在新代再次出现，
        //   记录的是「它最早由哪一代产出」，改写会把它悄悄变成「最近被引用」，
        //   统计按代筛样要的前者就没了。要问「哪些代重申过」应由引用计数层回答，不是这列。
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
    let verdict_confidence = match verdict {
        Some("wrong") => 0.7, // wrong 的教训更有价值,给更高初始置信度
        Some("partial") => 0.5,
        _ => 0.3, // correct 或 None 的教训价值较低
    };

    // P3：校验通过 ⇒ active + verdict 置信度 + rule_pattern=NULL；
    // 校验失败 ⇒ validation_rejected + confidence=0 + 驳回原因 JSON 留档（不生效）。
    let (status, confidence, rule_pattern) = match &admission {
        Ok(()) => ("active".to_string(), verdict_confidence, None),
        Err(reasons) => {
            tracing::warn!(
                "[F1] lesson 入模校验未过（只留档不生效）stock={stock_code} horizon={horizon}: {reasons:?}"
            );
            (
                "validation_rejected".to_string(),
                0.0,
                Some(serde_json::json!({ "rejectedReasons": reasons }).to_string()),
            )
        },
    };

    reflection_lessons::ActiveModel {
        id: Set(id),
        lesson_summary: Set(trimmed.to_string()),
        // active 行为 None（沿用原语义，后续 F1 扩展）；rejected 行写驳回原因 JSON
        rule_pattern: Set(rule_pattern),
        source_reflection_id: Set(Some(source_reflection_id.to_string())),
        stock_code: Set(Some(stock_code.to_string())),
        // 周期章：由本次复盘档位盖章（值域 = Period::as_str），读侧据此同档过滤
        horizon: Set(Some(horizon.to_string())),
        // §七十一 按版归属：建行即盖章，此后不改（见实体注释里「最早由哪一代产出」的语义）。
        template_version: Set(analysis_template_version),
        applicable_scenarios: Set(None),
        times_applied: Set(0),
        success_count: Set(0),
        confidence: Set(confidence),
        status: Set(status),
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
// `lesson-validation` 任务类型标识已按「定时任务 task_type 一律归 commands/constants.rs」
// 的裁定收口到 `crate::commands::constants::LESSON_VALIDATION_TASK_TYPE`，本模块只引用，
// 不再就地定义（避免长出第二权威）。
//
// **单点声明**：`init/services.rs` 的分派分支与 `init/stock_automation_cron_seed.rs` 的内置
// 任务都引用它，不各写一份字面量（同 `backtest_validation::DECISION_BACKTEST_TASK_TYPE` 的
// 处置）。改名会静默切断派发 —— 而本函数一旦没有触发方，教训的 `confidence` / `success_count`
// / 退役三件事就全停摆，且界面看不出来（就是 2026-10-14 之前的状态）。

// 调用方式：内置定时任务 `lesson-validation-daily`（工作日 21:00，排在 18:00 到期反思与
// 20:00 进化聚合之后，见 `init/stock_automation_cron_seed.rs`）+ Tauri 命令
// `run_lesson_validation_command`。
//
// ⚠ 2026-10-14 更正：本块原写「已接入 start_background_services 的 start_lesson_validation
// 定时任务」，而该符号全仓不存在 ⇒ `run_lesson_validation` 实际**没有任何触发方**
// （现网佐证：`reflection_lessons` 7 行里 `success_count>0` 为 0 行、`lesson_applications` 仅 1 行）。
// 触发通路本轮补齐，见 PLAN-reflection-optimization-closure §三。
pub async fn run_lesson_validation(
    db: &sea_orm::DatabaseConnection,
) -> Result<serde_json::Value, String> {
    use axagent_analysis_engine::reflection_lesson_validator::{
        build_lesson_validation, build_lesson_validation_report,
    };
    use axagent_entities::lesson_applications;
    use axagent_entities::reflection_lessons;

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
/// 内部核心函数 `run_lesson_validation` 的 Tauri 包装，供手动校证 / 紧急重算使用。
///
/// ⚠ 2026-10-14 更正：本块原文称核心函数「已通过 `start_lesson_validation` 后台定时任务
/// 自动调度」—— 该符号全仓不存在，当时它**没有任何自动触发方**（现网 `reflection_lessons`
/// 的 `success_count` 全 0 即证）。自动调度本轮由内置任务 `lesson-validation-daily` 补上
/// （`init/stock_automation_cron_seed.rs` 第 8 条 + `init/services.rs` 的分派分支）。
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
/// - `period = Some(p)` ⇒ 只处理**复盘档为 p** 的 pending（一行一档，档在建点即盖章）。
///   该列 NULL 的老行退回「原分析期望持有天数最近邻归一到 p」的旧口径。
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
    /// - `row_horizon`：本行**自己的**复盘档（〇-B v2 第 4 条起建点即写）。
    ///   `Some(h)` ⇒ 直接按档比对，不再用天数反推；
    ///   `None`（该列引入前的老行）⇒ 退回 `expected_days` 最近邻归一的旧口径。
    /// - `expected_days`：原分析声明的期望持有天数（`None` ⇒ 按反思侧既有兜底口径
    ///   视为 28 天，与 `run_batch_reflection_inner` 内的一致）
    /// - `hindsight_due`：计划评估时点是否已到
    /// - `min_confidence_threshold`：pending row 自带的置信度阈值
    pub fn matches(
        &self,
        row_horizon: Option<&str>,
        expected_days: Option<i64>,
        hindsight_due: bool,
        min_confidence_threshold: i32,
    ) -> bool {
        if let Some(p) = self.period {
            let hit = match row_horizon.and_then(|h| h.parse::<Period>().ok()) {
                Some(h) => h == p,
                None => Period::nearest_for_holding_days(expected_days.unwrap_or(28)) == p,
            };
            if !hit {
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

// ── 一行 pending = 一个复盘档（PLAN-reflection-per-horizon-row §2）──

/// 反思产品默认置信度阈值。三个建点此前各抄一份字面量 `70`
/// （`bin/axagent-batch-rerun.rs` 的注释自证：「手抄常量，三处各写一份」），
/// 现收在唯一常量，建 pending 行只能经 `build_pending_reflection_rows`。
pub const DEFAULT_REFLECTION_MIN_CONFIDENCE: u8 = 70;
/// 反思深度默认值（同上，此前手抄三处）。
pub const DEFAULT_REFLECTION_DEPTH: &str = "light";

// ── P3 妖股漏检闭环：反思样本来源取值 ──
/// 常规到期反思（live / chat / 回放补建三通道默认值）。
pub const REFLECTION_SOURCE_DUE: &str = "due";
/// 妖股漏检 as-of 补分析触发的漏检回放反思。
pub const REFLECTION_SOURCE_MOVER_RECALL: &str = "mover_recall";
/// 历史 live 分析的口径补建（P1-6，`PLAN-stock-decision-reliability.md`）：
/// 10-14 之前 live 分析几乎零反思行，离线 bin 按缺档补建时盖此标记便于回溯，
/// 与常规 `due` 反思维度分开统计。
pub const REFLECTION_SOURCE_BACKFILL: &str = "backfill";

/// 建 pending 行所需的**最小**分析侧事实。
pub struct PendingReflectionSeed<'a> {
    pub analysis_id: &'a str,
    pub stock_code: &'a str,
    pub stock_name: &'a str,
    /// 分析锚点日（`YYYY-MM-DD`）。live 通道 = 今天；as-of 重放通道 = `as_of_date`。
    pub analysis_date: &'a str,
    /// `stock_analyses.horizon_decisions` 原文（四档独立决策）。None / 空 ⇒ 无逐档方向可判，
    /// 退回**只复盘主档一行**（不伪造其余三档的方向）。
    pub horizon_decisions: Option<&'a str>,
    /// `stock_analyses.decision_time_horizon`（公式主档）
    pub primary_horizon: Option<&'a str>,
    /// `stock_analyses.decision_expected_holding_days`
    pub primary_holding_days: Option<i64>,
    pub min_confidence_threshold: u8,
    pub reflection_depth: &'a str,
    /// 被复盘那条分析的算法代际（`stock_analyses.template_version` 原值，**不做兜底**）。
    ///
    /// §七十一 按版归属：代际必须在建 pending 行这一刻抄下来，因为分析行之后可能被重跑、
    /// 换代、甚至清空 ⇒ 事后靠 join 回查会拿到「今天的代际」而不是「当时那笔决策的代际」。
    ///
    /// `None` 的三种真实成因（读侧必须当「代际未知」，不得当第 0 代）：
    /// ① 对话直执行通道（链路不经 `workflow_templates`，A4 已裁定其分析行恒 NULL）；
    /// ② 建行时 `template_version` 还没写（占位行阶段，见 `core.rs` 的写入顺序）；
    /// ③ 本列引入前的存量分析行。
    pub analysis_template_version: Option<i32>,
    /// 反思样本来源（P3）：`"due"`（常规到期反思）/ `"mover_recall"`（妖股漏检回放）。
    ///
    /// 与被复盘分析行的 `stock_analyses.source` 同源透传：`core.rs` 单股通道直接传
    /// `run_single_stock_analysis` 的 source 参数；chat / 回放补建两通道固定 `"due"`。
    /// 建点即盖章，此后 UPDATE 不改写。
    pub source: &'a str,
    pub now_ms: i64,
}

/// 持有/观望档的反思建点抽样率（百分比）。方向动作全量建点；中性档没有多空可判，
/// 全量建只会灌爆反思 LLM 配额，按此比例做确定性抽样（P1-9）。
const NEUTRAL_SAMPLE_PCT: u64 = 20;

/// 确定性抽样：FNV-1a(`token:horizon`) 落在 [0,100) 的前 [`NEUTRAL_SAMPLE_PCT`] 段。
/// 刻意不用随机数：同一分析重跑/重放必须得到同一结果（幂等），换股自然重抽。
fn sampled_in(token: &str, horizon: &str) -> bool {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in token.bytes().chain(*b":").chain(horizon.bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h % 100 < NEUTRAL_SAMPLE_PCT
}

/// 本次可复盘的档集 = 四档里**决策有明确方向**的那些（裁定 Q1），
/// 外加**持有/观望档的确定性抽样**（P1-9，`PLAN-stock-decision-reliability.md`）。
///
/// 方向动作全量建点；持有/观望档没有多空可判（`deterministic_was_correct` 对它们返回
/// `None`），全量建只会灌爆反思 LLM 配额，故按 [`NEUTRAL_SAMPLE_PCT`] 抽样 ——
/// 让「该买没买 / 该拿没拿」这类中性决策也有少量样本可回看（反思端对这些行不产出
/// 伪结论，只落区间行情事实）。不确定/无法判断/未识别仍不建行。
/// 返回 `(档名, 该档期望持有交易日)`；天数优先取该档决策声明值，缺失回 `Period` 默认表。
///
/// `sample_token`：中性档抽样种子（调用方传 `analysis_id`）——同一分析重跑/重放
/// 结果稳定，同一批分析中各股独立抽样。
pub fn reviewable_horizons(
    horizon_decisions: Option<&str>,
    primary_horizon: Option<&str>,
    primary_holding_days: Option<i64>,
    sample_token: &str,
) -> Vec<(String, i64)> {
    let parsed = horizon_decisions
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_object().map(|o| parse_horizon_decisions(o).unwrap_or_default()))
        .unwrap_or_default();

    let keep = |action: &str, p: &Period| -> bool {
        match normalize_action(action) {
            Some(
                ActionKind::Buy | ActionKind::Increase | ActionKind::Sell | ActionKind::Reduce,
            ) => true,
            Some(ActionKind::Hold | ActionKind::Wait) => sampled_in(sample_token, p.as_str()),
            _ => false,
        }
    };

    let mut out: Vec<(String, i64)> = Period::ALL
        .iter()
        .filter_map(|p| {
            // 双键取：`decisionsByHorizon` 的键是 **camelCase**（`ultraShort`；
            // `Period::camel_key` 为权威拼写），而本函数输出/落库用 snake（`as_str`）。
            // 旧实现只取 snake ⇒ **ultra 档在逐档路径系统性 miss**（snake==camel 的
            // short/mid/long 无此问题）——这正是「超短档反思样本长期只有主档兜底
            // 那几条」的根因（2026-10-16 查：全库 101 条分析均为 camel 键、ultra 逐档
            // 行仅 6 条且全部来自主档恰为 ultra 的兜底路径）。
            let d = parsed.get(p.as_str()).or_else(|| parsed.get(p.camel_key()))?;
            if !keep(&d.action, p) {
                return None;
            }
            Some((
                p.as_str().to_string(),
                d.expected_holding_days
                    .unwrap_or_else(|| default_expected_holding_days(p.as_str())),
            ))
        })
        .collect();
    if !out.is_empty() {
        return out;
    }
    // 有逐档决策、但过滤后为空（四档全为中性且未抽中）⇒ 如实返回空表。
    // ⚠ 不能掉进下方「无逐档决策」的兜底：那会无条件给主档建行，让「未抽中」全部作弊命中。
    if !parsed.is_empty() {
        return out;
    }

    // 无逐档决策（老模板 / 该次分析未产出 decisionsByHorizon）⇒ 只有主档可复盘。
    // 主档缺失时**不采信 LLM 自报的 timeHorizon**（〇-B v2 第 2 条），改由期望天数
    // 最近邻归一到 Period —— 与 `ReflectionFilter::matches` 同一口径。
    let primary = match primary_horizon.and_then(|h| h.parse::<Period>().ok()) {
        Some(p) => p,
        None => match primary_holding_days.filter(|d| *d > 0) {
            Some(d) => {
                let p = Period::nearest_for_holding_days(d);
                tracing::warn!(
                    "[reflection pending] 无逐档决策且原分析无主档 ⇒ 按期望 {d} 天最近邻归到 {} 建单行（代理，非事实陈述）",
                    p.as_str()
                );
                p
            },
            None => {
                tracing::warn!(
                    "[reflection pending] 无逐档决策、无主档、无期望天数 ⇒ 按 short 建单行（代理，非事实陈述）"
                );
                Period::Short
            },
        },
    };
    let days = primary_holding_days
        .filter(|d| *d > 0)
        .unwrap_or_else(|| default_expected_holding_days(primary.as_str()));
    out.push((primary.as_str().to_string(), days));
    out
}

/// 取该分析某档的期望持有交易日：决策声明优先、`Period` 默认兜底。
///
/// P1-9：存量 pending 的 `hindsight_date` 重算必须与建点（`reviewable_horizons`）
/// 用同一把尺 —— 两处都走「声明优先、默认兜底」，否则重算会把口径改出第三种。
pub fn expected_holding_days_for(horizon_decisions: Option<&str>, horizon: &str) -> i64 {
    horizon_decisions
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_object().map(|o| parse_horizon_decisions(o).unwrap_or_default()))
        .and_then(|parsed| parsed.get(horizon).and_then(|d| d.expected_holding_days))
        .unwrap_or_else(|| default_expected_holding_days(horizon))
}

/// 按复盘档集生成 pending 行（每档一行，`horizon` 建点即写、不留 NULL）。
///
/// 三个建点（`core.rs` 单股通道 / `hooks.rs` 对话通道 / `bin/axagent-batch-rerun.rs`
/// 回放补建）必须统一走本函数 —— 各自 `stock_reflections::ActiveModel {` 字面量构造
/// 是缺陷 ④ 的成因（默认值手抄、档口径分叉）。
pub fn build_pending_reflection_rows(
    seed: &PendingReflectionSeed<'_>,
) -> Vec<stock_reflections::ActiveModel> {
    build_pending_reflection_rows_for(
        seed,
        reviewable_horizons(
            seed.horizon_decisions,
            seed.primary_horizon,
            seed.primary_holding_days,
            seed.analysis_id,
        ),
    )
}

/// 按**给定档集**建 pending 行。
///
/// 与 `build_pending_reflection_rows` 分开，是给「已有部分档行、只补缺档」的调用方用的
/// （`bin/axagent-batch-rerun.rs` 的重跑补建：同一条分析可能已有 ultra_short 行，
/// 本次只需补 short/mid/long）。
pub fn build_pending_reflection_rows_for(
    seed: &PendingReflectionSeed<'_>,
    horizons: Vec<(String, i64)>,
) -> Vec<stock_reflections::ActiveModel> {
    let analysis_nd = chrono::NaiveDate::parse_from_str(seed.analysis_date, "%Y-%m-%d")
        .unwrap_or_else(|_| {
            tracing::warn!(
                "[reflection pending] 锚点日 {:?} 解析失败 ⇒ 退回本地日期",
                seed.analysis_date
            );
            chrono::Local::now().date_naive()
        });

    // P1-9（`PLAN-stock-decision-reliability.md`，2026-10-16）：四档 hindsight **统一**
    // 按交易日外推（T+2/T+5/T+28/T+90 交易日）—— 旧实现 live 走自然日、仅 as_of 批次走
    // 交易日；自然日口径让长档样本成熟额外推迟约 40%（90 自然日 ≈ 63 交易日），长假后失真。
    // 未来交易日历缺口由 is_trading_day「工作日默认开市」兜底，名义到期日误差仅限未公告假期，
    // 实际反思取数另有「≤ hindsight 最后一根真实 K 线」自证。
    horizons
        .into_iter()
        .map(|(horizon, days)| {
            let hindsight_nd =
                axagent_astock_data::calendar::nth_trading_day(analysis_nd, days as i32);
            let hindsight_date = hindsight_nd.format("%Y-%m-%d").to_string();
            stock_reflections::ActiveModel {
                id: Set(uuid::Uuid::new_v4().to_string()),
                stock_code: Set(seed.stock_code.to_string()),
                stock_name: Set(seed.stock_name.to_string()),
                original_analysis_id: Set(seed.analysis_id.to_string()),
                as_of_date: Set(seed.analysis_date.to_string()),
                hindsight_date: Set(hindsight_date),
                min_confidence_threshold: Set(seed.min_confidence_threshold as i32),
                reflection_depth: Set(seed.reflection_depth.to_string()),
                // 〇-B v2 第 4 条：本行从建点起就**只属于这一档**（不再留 NULL 等收尾盖章）
                horizon: Set(Some(horizon)),
                actual_outcome: Set(String::new()),
                raw_return: Set(None),
                alpha_return: Set(None),
                holding_days: Set(None),
                // #10 P7：pending 行还没复盘，标签留 NULL（不是「不算妖股」）
                mover_label: Set(None),
                benchmark_name: Set(None),
                verdict: Set(None),
                alpha_cited: Set(None),
                lesson_summary: Set(None),
                what_went_wrong: Set(None),
                missed_signals: Set(None),
                fix_for_future: Set(None),
                parameter_suggestions_json: Set(None),
                horizon_results_json: Set(None),
                decision_json: Set(None),
                blackboard_snapshot: Set(None),
                // §七十一 按版归属：三个建点（单股 / 对话 / 回放补建）共用的唯一盖章处。
                // §七十一 按版归属：逐档各行**同一代际** —— 它们复盘的是同一笔决策。
                template_version: Set(seed.analysis_template_version),
                // P3：来源建点即盖章（与被复盘分析行同源）；权威归因层在反思收尾时
                // 从 mover_recall_events 回查盖章，建点时拿不到，先留 NULL。
                source: Set(seed.source.to_string()),
                miss_layer: Set(None),
                failure_modes: Set(None),
                status: Set("pending".to_string()),
                created_at: Set(seed.now_ms),
                updated_at: Set(seed.now_ms),
            }
        })
        .collect()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReflectionOutcome {
    SkippedYoung,
    Completed,
}

#[allow(clippy::too_many_arguments)]
async fn perform_single_reflection(
    log_prefix: &str,
    db: &DatabaseConnection,
    client: &axagent_astock_data::AStockClient,
    engine: &Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    vector_store: &axagent_search::vector_store::VectorStore,
    master_key: &[u8; 32],
    trajectory_storage: Option<&Arc<axagent_trajectory::TrajectoryStorage>>,
    pending_row: &axagent_entities::stock_reflections::Model,
    analysis: &axagent_entities::stock_analyses::Model,
    analysis_date: &str,
    today_nd: chrono::NaiveDate,
    days_held: i64,
    effective_depth: &str,
) -> Result<ReflectionOutcome, String> {
    let target_price = extract_target_price(analysis);

    let horizon_days: std::collections::BTreeMap<String, i64> = {
        let mut map = std::collections::BTreeMap::new();
        let decisions = analysis
            .horizon_decisions
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.as_object().map(|obj| parse_horizon_decisions(obj).unwrap_or_default()))
            .unwrap_or_default();
        for key in &["ultra_short", "short", "mid", "long"] {
            let days = decisions
                .get(*key)
                .and_then(|d| d.expected_holding_days)
                .unwrap_or_else(|| default_expected_holding_days(key));
            map.insert(key.to_string(), days);
        }
        map
    };

    let snapshots_raw = compute_market_snapshots(
        client,
        &pending_row.stock_code,
        analysis_date,
        &horizon_days,
        target_price,
    )
    .await;

    // 〇-B v2 第 4 条 / `PLAN-reflection-per-horizon-row`：**本行的复盘档 = 行自身的 horizon**
    //   （建 pending 时即盖章，一行一档）。老 pending 行（该列 NULL）⇒ 退回原分析主档并 warn，
    //   不再无条件读 `analysis.decision_time_horizon` —— 那会让四个档共用主档复盘。
    let primary_horizon = match pending_row.horizon.as_deref() {
        Some(h) => h,
        None => analysis.decision_time_horizon.as_deref().unwrap_or_else(|| {
            tracing::warn!(
                "[{log_prefix}] {} ({}) 本行无复盘档且原分析无主档 ⇒ 按 short 复盘（代理，非事实陈述）",
                pending_row.id,
                pending_row.stock_code
            );
            "short"
        }),
    };
    // 主档快照的处置判定统一走 `take_primary_snapshot`（「样本不足 vs 取数失败」的唯一判据点）。
    // ⚠ 跳过必须**先于**降级：2026-10-16 前取数侧把「不足」写进 Err 通道，这里的
    // `Some(s) if s.trading_days < 1` 永远不可达 ⇒ 幼龄行被写成 completed 盲反思。
    let primary_snap_opt = match take_primary_snapshot(snapshots_raw.get(primary_horizon)) {
        Ok(s) => Some(s.clone()),
        Err(SnapshotGap::Young) => {
            tracing::info!(
                "[{log_prefix}] {} ({}) {primary_horizon} 分析日之后尚无完整交易日的新行情,样本不足 skip",
                pending_row.id,
                pending_row.stock_code
            );
            return Ok(ReflectionOutcome::SkippedYoung);
        },
        Err(SnapshotGap::Failed(reason)) => {
            tracing::warn!(
                "[{log_prefix}] {} ({}) {primary_horizon} 行情快照不可用({reason}),降级",
                pending_row.id,
                pending_row.stock_code
            );
            None
        },
    };

    let horizon_snapshots: std::collections::BTreeMap<String, Option<MarketSnapshot>> =
        snapshots_raw.into_iter().map(|(k, v)| (k, v.ok())).collect();

    let actual_outcome = primary_snap_opt
        .as_ref()
        .map(|s| s.render_outcome_short())
        .unwrap_or_else(|| pending_row.actual_outcome.clone());
    let today_str = today_nd.format("%Y-%m-%d").to_string();

    run_reflection_workflow(
        db,
        client,
        engine,
        vector_store,
        master_key,
        &pending_row.stock_code,
        &pending_row.stock_name,
        &pending_row.original_analysis_id,
        &actual_outcome,
        primary_snap_opt.as_ref().map(|s| s.net_return_pct),
        primary_snap_opt.as_ref().and_then(|s| s.alpha_pct),
        Some(primary_snap_opt.as_ref().map(|s| s.trading_days as i32).unwrap_or(days_held as i32)),
        primary_snap_opt.as_ref().map(|_| "沪深300"),
        analysis_date,
        &today_str,
        pending_row.min_confidence_threshold.clamp(0, 255) as u8,
        effective_depth,
        Some(primary_horizon),
        Some(pending_row.id.clone()),
        trajectory_storage,
        &horizon_snapshots,
    )
    .await?;

    Ok(ReflectionOutcome::Completed)
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
    // 时间旅行批次限定：只处理这批 `stock_analyses.id` 对应的 pending 行；
    // None = 全表（live cron / batch-rerun 既有行为）。
    analysis_ids: Option<&[String]>,
) -> Result<serde_json::Value, String> {
    use crate::commands::error::ErrorResponse;
    use axagent_entities::stock_analyses;

    let max_count = max_count.unwrap_or(20) as usize;
    let today_ms = chrono::Utc::now().timestamp_millis();
    // 自愈孤儿 pending 行（同 `run_batch_reflection`，两份实现都要有，否则 cron 与手动
    // 入口行为不一致）；必须在扫 pending 之前。
    let orphans_retired =
        match axagent_dao::repo::stock_lesson_queries::retire_orphan_pending_reflections(
            db, today_ms,
        )
        .await
        {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(
                    "[D1 batch_reflection] 孤儿反思行自愈失败（下轮重试，不静默当成没有）: {e}"
                );
                0
            },
        };

    // 0. 自愈：把卡死的 running 记录退回 pending（详见 reclaim_stale_running 文档）。
    //    必须在扫 pending 之前执行，否则被卡死的 row 本轮仍不会被处理。
    let reclaimed = reclaim_stale_running(db, STALE_RUNNING_HOURS).await;

    // 1. 扫所有 pending row,按 created_at ASC(最老的先处理,避免积压)
    //
    // ⚠ 到期筛选必须下推到**本查询**，而不是留给循环里的 `filter.matches`：
    // 下方 `take(max_count)` 发生在 filter **之前** —— 若在 SQL 里不过滤到期，
    // 一堆未到期的老 pending 会耗光配额，使当日到期记录一条都轮不到
    // （「18:00 只反思到期记录」的诉求会静默失效）。
    let today_str = {
        use chrono::TimeZone;
        let offset = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        offset
            .from_utc_datetime(&chrono::Utc::now().naive_utc())
            .date_naive()
            .format("%Y-%m-%d")
            .to_string()
    };
    let mut pending_query =
        stock_reflections::Entity::find().filter(stock_reflections::Column::Status.eq("pending"));
    if filter.is_some_and(|f| f.due_only) {
        // hindsight_date 是 YYYY-MM-DD ⇒ 字典序与日期序一致；NULL 行（该列引入前的
        // 老数据）不满足 lte ⇒ 不进到期队列，诚实缺席（提前反思通路仍由
        // due_only=false 的 6h 兜底服务承担）。
        pending_query =
            pending_query.filter(stock_reflections::Column::HindsightDate.lte(&today_str));
    }
    if let Some(p) = filter.and_then(|f| f.period) {
        // 档位筛选同样必须下推（与上面 due_only 同因）：一行一档之后 pending 池 ×4，
        // 若留给循环里的 `filter.matches`，`take(max_count)` 会被其它档的行占满，
        // 「只反思 long 档」的定时任务可能整轮 0 条却仍报 totalPending>0。
        // NULL 老行留在队列内（该列引入前建的），由 matches() 按天数最近邻兜旧口径。
        pending_query =
            pending_query.filter(axagent_dao::repo::stock_lesson_queries::horizon_eq_or_null(
                stock_reflections::Column::Horizon,
                p.as_str(),
            ));
    }
    if let Some(ids) = analysis_ids {
        // 时间旅行批次：把反思范围严格收口到本批 replay 分析，绝不误反思其它 pending
        // （全表 inner 无此参数前会串到 live/历史积压行）。空集直接给一个永假谓词。
        if ids.is_empty() {
            pending_query =
                pending_query.filter(stock_reflections::Column::OriginalAnalysisId.eq("__none__"));
        } else {
            pending_query =
                pending_query.filter(stock_reflections::Column::OriginalAnalysisId.is_in(ids));
        }
    }
    let pendings: Vec<stock_reflections::Model> =
        pending_query.order_by_asc(stock_reflections::Column::CreatedAt).all(db).await.map_err(
            |e| {
                ErrorResponse::new(wf_err::INTERNAL)
                    .with_detail(format!("run_batch_reflection_inner 扫 pending row 失败: {e}"))
            },
        )?;

    tracing::info!(
        "[D1 batch_reflection] 扫到 {} 条 pending row, max_count={}",
        pendings.len(),
        max_count
    );

    let mut resolved = 0u32;
    let mut failed = 0u32;
    let mut skipped_young = 0u32;
    let mut orphans = 0u32;
    let mut skipped_by_filter = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for p in pendings.iter().take(max_count) {
        let analysis =
            match stock_analyses::Entity::find_by_id(&p.original_analysis_id).one(db).await {
                Ok(Some(a)) => a,
                Ok(None) => {
                    // 同 `run_batch_reflection`：孤儿行必须落成终局并单独计数，
                    // 不得继续冒充「太新未到期」被每轮重复捞起。
                    tracing::warn!(
                        "[D1 batch_reflection] pending reflection {} 关联 analysis_id={} \
                         不存在 ⇒ 置孤儿终局",
                        p.id,
                        p.original_analysis_id
                    );
                    // 不在此写库：孤儿终局化由本轮开头的 dao 自愈统一做（两个入口都已接），
                    // 这里只如实计数，下一轮开头会把它们处置掉。
                    orphans += 1;
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

        let today_nd = chrono::NaiveDate::parse_from_str(&today_str, "%Y-%m-%d")
            .expect("today_str 由本函数按同一格式生成");

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
                p.horizon.as_deref(),
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

        // ── [实际行情] 四周期行情拉取 + 反思调用 → perform_single_reflection（技术债已清）──
        let depth =
            filter.and_then(|f| f.depth_override.as_deref()).unwrap_or(p.reflection_depth.as_str());

        let r = perform_single_reflection(
            "batch_reflection_inner",
            db,
            _client,
            &std::sync::Arc::clone(_engine),
            _vector_store,
            _master_key,
            trajectory_storage,
            p,
            &analysis,
            analysis_date,
            today_nd,
            days_held,
            depth,
        )
        .await;

        match r {
            Ok(ReflectionOutcome::SkippedYoung) => {
                skipped_young += 1;
                continue;
            },
            Ok(ReflectionOutcome::Completed) => {
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
         orphans={} skipped_by_filter={} reclaimed={} cleaned={}",
        pendings.len(),
        resolved,
        failed,
        skipped_young,
        orphans,
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
        "orphans": orphans,
        // 本轮开头由 dao 自愈落成终局的孤儿行数
        "orphansRetired": orphans_retired,
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

/// 单档行情快照「拿不到」的两种成因 —— 两条消费链的处置**完全不同**，
/// 不能再压进同一个 `Err(String)` 通道（2026-10-16 现场：幼龄行被降级成 completed 盲反思）：
///
/// - `Young`：分析日之后**尚未跨过 1 个完整交易日的新行情**（一根 K 线都没有，或只有
///   入场那一根）⇒ 样本不足。批量链**跳过并保持行 pending**（下个批次再看），
///   手动链如实报「不足以构成对比」；
/// - `Failed(String)`：取数或数据本身异常 ⇒ 如实降级，原因随值带回（旧实现把原因
///   吞在 `Err` 里只留一句「不可用」，2026-10-14 取证时只能靠翻库反推）。
#[derive(Debug, Clone)]
pub enum SnapshotGap {
    Young,
    Failed(String),
}

/// 单档窗口的「出场脚标 + 已走完的交易日数」；`None` = 分析日之后尚未跨过
/// 1 个完整交易日（只有入场那一根）⇒ 样本不足。
///
/// 「分析日之后仅 N 个交易日 ⇒ 跳过」这条判据的唯一出处。⚠ 历史教训：不足曾被打包成
/// `Err(String)` 出函数，而消费侧的跳过判据全建在 `Ok(快照)` 支上 ⇒ 判据静默失效三周
/// （11aa58772 起，002371 / 10-14 批次实证），修好前不要再引入第三处口径。
fn horizon_window(klines_len: usize, entry_idx: usize, expected_days: i64) -> Option<(usize, i64)> {
    let exit_idx = std::cmp::min(klines_len.saturating_sub(1), entry_idx + expected_days as usize);
    let trading_days = (exit_idx - entry_idx) as i64;
    (trading_days >= 1).then_some((exit_idx, trading_days))
}

/// 取主档快照并判其处置：`None`（快照表中缺该档键，理论不可达）折成 `Failed` 归因 ——
/// 「样本不足 vs 取数失败」的**唯一**判据点，批量链与手动链都走这里，不各写一份 match。
pub fn take_primary_snapshot(
    cell: Option<&Result<MarketSnapshot, SnapshotGap>>,
) -> Result<&MarketSnapshot, SnapshotGap> {
    match cell {
        Some(Ok(s)) => Ok(s),
        Some(Err(SnapshotGap::Young)) => Err(SnapshotGap::Young),
        Some(Err(SnapshotGap::Failed(reason))) => Err(SnapshotGap::Failed(reason.clone())),
        None => Err(SnapshotGap::Failed("快照表中缺该档位键".into())),
    }
}

/// ── 四周期行情快照（PLAN-stock-reflection-four-horizon 批次 3）──
///
/// 一次 K 线读取，按四个目标交易日索引分别生成窗口快照，避免每周期重复请求。
/// 每个周期的 exit 点 = min(klines.len()-1, entry_idx + expected_holding_days)。
///
/// 状态语义（对每个周期）：
/// - `Ok` + `within_expected_horizon=false` → mature（数据充足，可做终评）
/// - `Ok` + `within_expected_horizon=true` → immature（期中观察，出场=最新一根）
/// - `Err(SnapshotGap::Young)` → 样本不足（分析日之后尚未跨过完整交易日，等后续批次）
/// - `Err(SnapshotGap::Failed)` → unavailable（行情源失败 / 数据异常）
///
/// 基准（沪深300）仅计算一次（从 entry 到全窗口最新），
/// 所有周期共享同一个基准涨幅（不同窗口的基准偏差在统计上远小于方向误差）。
pub async fn compute_market_snapshots(
    client: &axagent_astock_data::AStockClient,
    stock_code: &str,
    analysis_date: &str,
    horizon_expected_days: &std::collections::BTreeMap<String, i64>,
    target_price: Option<f64>,
) -> std::collections::BTreeMap<String, Result<MarketSnapshot, SnapshotGap>> {
    use axagent_harness::market_data::{AdjType, MarketDataProvider};

    let mut results = std::collections::BTreeMap::new();

    let klines = match MarketDataProvider::get_klines(
        client,
        stock_code,
        "daily",
        REFLECTION_KLINE_COUNT,
        Some(AdjType::Forward),
    )
    .await
    {
        Ok(k) if !k.is_empty() => k,
        Ok(_) => {
            let err = format!("{stock_code} 无K线数据");
            for key in horizon_expected_days.keys() {
                results.insert(key.clone(), Err(SnapshotGap::Failed(err.clone())));
            }
            return results;
        },
        Err(e) => {
            let err = format!("获取 {stock_code} K线失败: {e}");
            for key in horizon_expected_days.keys() {
                results.insert(key.clone(), Err(SnapshotGap::Failed(err.clone())));
            }
            return results;
        },
    };

    let entry_idx = match klines.iter().position(|k| k.date.as_str() > analysis_date) {
        Some(i) => i,
        None => {
            // 分析日之后**一根新 K 线都没有**（含「分析日 = 最新交易日」的当天场景，
            // 02:00 自动化次日凌晨跑反思即此形态）⇒ 同样是样本不足，等后续批次。
            // ⚠ 旧实现把这支写成 `Err("...在 X 之后无K线数据")`，消费侧无法与真故障区分，
            // 每根幼龄行都被降级后写成 completed 盲反思（2026-10-16 修复）。
            for key in horizon_expected_days.keys() {
                results.insert(key.clone(), Err(SnapshotGap::Young));
            }
            return results;
        },
    };

    let entry_bar = &klines[entry_idx];
    let entry_price = entry_bar.open;
    if entry_price.is_nan() || entry_price <= 0.0 {
        let err = format!("{stock_code} 入场基准价非法: {entry_price}");
        for key in horizon_expected_days.keys() {
            results.insert(key.clone(), Err(SnapshotGap::Failed(err.clone())));
        }
        return results;
    }

    // 基准：只算一次（从 entry 到全窗口最新）
    let latest_bar_all = klines.last().expect("klines non-empty");
    let (benchmark_code, benchmark_change_pct) =
        match compute_benchmark_change(client, &entry_bar.date, &latest_bar_all.date).await {
            Ok(chg) => (Some(DEFAULT_BENCHMARK_CODE.to_string()), Some(chg)),
            Err(e) => {
                tracing::warn!(
                    "[market snapshots] {stock_code} 基准对比失败,降级为无超额收益: {e}"
                );
                (None, None)
            },
        };

    for (horizon_key, &expected_days) in horizon_expected_days {
        if expected_days <= 0 {
            results.insert(
                horizon_key.clone(),
                Err(SnapshotGap::Failed(format!("{horizon_key}: 无效持有期 {expected_days}"))),
            );
            continue;
        }

        let Some((exit_idx, trading_days)) = horizon_window(klines.len(), entry_idx, expected_days)
        else {
            // 尚未跨过 1 个完整交易日（只有入场那一根）⇒ 样本不足，不是故障
            results.insert(horizon_key.clone(), Err(SnapshotGap::Young));
            continue;
        };
        let within_expected_horizon = exit_idx < entry_idx + expected_days as usize;
        let exit_bar = &klines[exit_idx];
        let latest_price = exit_bar.close;

        // 区间高低 + 最大回撤（entry_idx..=exit_idx）
        let window = &klines[entry_idx..=exit_idx];
        let period_high = window.iter().fold(f64::MIN, |m, k| m.max(k.high));
        let mut period_low =
            window.iter().filter(|k| k.low > 0.0).fold(f64::MAX, |m, k| m.min(k.low));
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
            period_low = window.iter().map(|k| k.close).fold(f64::MAX, f64::min);
        }

        let price_change_pct = (latest_price - entry_price) / entry_price * 100.0;
        let net_return_pct = price_change_pct - A_SHARE_COST_RATE * 100.0;
        let alpha_pct = benchmark_change_pct.map(|chg| price_change_pct - chg);

        let target_progress_pct = target_price.and_then(|tp| {
            let expected_pct = (tp - entry_price) / entry_price * 100.0;
            if expected_pct.abs() < 0.01 {
                None
            } else {
                Some(price_change_pct / expected_pct * 100.0)
            }
        });
        let target_reached = target_price.map(|tp| latest_price >= tp);

        results.insert(
            horizon_key.clone(),
            Ok(MarketSnapshot {
                stock_code: stock_code.to_string(),
                analysis_date: analysis_date.to_string(),
                entry_date: entry_bar.date.clone(),
                entry_price,
                latest_date: exit_bar.date.clone(),
                latest_price,
                price_change_pct,
                net_return_pct,
                period_high,
                period_low,
                max_drawdown_pct: max_dd * 100.0,
                trading_days,
                expected_holding_days: Some(expected_days),
                within_expected_horizon,
                benchmark_code: benchmark_code.clone(),
                benchmark_change_pct,
                alpha_pct,
                target_price,
                target_progress_pct,
                target_reached,
            }),
        );
    }

    results
}

/// 取基准（沪深300）在指定区间内的涨跌幅（%）。
///
/// 与个股同源前复权 K 线，保证可比性；任一端点缺失时返回 `Err`，
/// 由调用方降级为「无基准对比」，而不是用 0.0 冒充。
///
/// `pub(crate)`：智选（serenity）的 T+N 超额命中通道也用它（`commands/backtest_validation.rs`），
/// 两处必须用同一个基准与同一份取数，否则「超额收益」在两条链上不是同一个数。
pub(crate) async fn compute_benchmark_change(
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
    // 同 compute_market_snapshots：显式拒绝 NaN 与 ≤0
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

        // 空 node_id（键字面为 `_raw.`）是 build_blackboard_snapshot 侧的畸形重复
        // 写入（实测与 debate-bull-bear 节点同尺寸重复，见 2026-10-14 故障取证）。
        // input_mapping 不可能引用空名节点，跳过以免污染并白占上下文。
        if node_id.is_empty() {
            tracing::warn!(
                "[reflection] {}: snapshot 含空 node_id 的 _raw 键（疑似重复写入），已跳过",
                stock_code
            );
            continue;
        }

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

    // 上下文预算裁剪：_raw.* 全量恢复实测 57~73 万字符（raw-data 原始行情 JSON
    // 单块 8~12 万、且 raw-data / raw-data-aggregated / portfolio-mgr.input_params
    // 多处重复）⇒ 反思 LLM 输入达 72.7 万 token，超 agnes-2.5-flash 的 524k 上限
    // 整条反思失败（2026-10-14 实证 4 行 ContextWindowExceeded）。
    enforce_reflection_memory_budget(&mut sub_analysis);

    serde_json::Value::Object(sub_analysis)
}

/// 反思记忆（`sub-analysis` = 全部 `_raw.*` 节点输出）的总字符预算。
///
/// P1-2（`PLAN-stock-decision-reliability.md`）：从 12 万收紧到 3.6 万字符 ——
/// 目标是「只传决策 JSON + 逐档腿信号 + 区间行情摘要」，整体上下文压在数万 token
/// （中文混合 JSON 实测约 0.7~1 token/字 ⇒ 3.6 万字符 ≈ 25k~36k token）。
/// ⚠ 收紧依赖第 3 层的「非保护节点优先淘汰」：否则按体积从大到小硬砍会把
/// 四档决策节点（约 2.5 万字符级）一并砍掉 —— 反思反而读不到决策。
const REFLECTION_MEMORY_TOTAL_CHAR_BUDGET: usize = 36_000;
/// 单节点序列化预算：超过后逐趟压缩该节点内的字符串叶子。
const REFLECTION_MEMORY_NODE_CHAR_BUDGET: usize = 8_000;
/// 第一遍单字符串叶子预算（分析师结论 / 辩论共识通常不超过此量级）。
const REFLECTION_MEMORY_LEAF_CHAR_BUDGET: usize = 4_000;
/// 节点超预算时，叶预算逐档收紧的档位（4000 已在第一遍施加）。
const REFLECTION_MEMORY_NODE_SHRINK_CAPS: [usize; 3] = [2_000, 1_000, 400];

/// 反思记忆硬引用保护名单（P1-2）：总预算淘汰时**最后**才动这些节点。
///
/// 来源 = `stock_analysis_setup/mod.rs` 反思模板 `reflection-comparator` 的
/// input_mapping 中全部 `sub-analysis.<node>.*` 直接路径（trader_action /
/// portfolio_action / debate_consensus / total_score / 六个 `__untrusted` 等）——
/// 被整体淘汰会让映射变量静默变空。改模板映射时必须同步本表。
const REFLECTION_MEMORY_PROTECTED_NODES: &[&str] = &[
    "trader",
    "portfolio-mgr",
    "debate-convergence",
    "t-scoring",
    "research-mgr",
    "a-catalyst",
    "data-quality",
    "risk-convergence",
];

/// 该节点是否属「反思硬引用面」（见 [`REFLECTION_MEMORY_PROTECTED_NODES`]）。
fn is_protected_reflection_node(node_id: &str) -> bool {
    REFLECTION_MEMORY_PROTECTED_NODES.contains(&node_id)
}

/// 估算 JSON 值序列化后的**字符**数（非字节：中文按字符计更贴近 token 量级）。
fn reflection_json_chars(value: &serde_json::Value) -> usize {
    serde_json::to_string(value).map(|s| s.chars().count()).unwrap_or(0)
}

/// 就地截断一个字符串，保留前 `max_chars` 个**字符**（按 Unicode scalar，不切碎
/// 汉字）并追加裁剪标记。已在预算内则不动。
///
/// 标记刻意不记「原文长度」：节点压缩会对同一叶子多趟截断（8000→4000→…），
/// 二趟起能看到的只是上一趟截断后的串，数字无法还原真原始长度，写了反而误导。
fn cap_reflection_string(s: &mut String, max_chars: usize) {
    if s.chars().count() <= max_chars {
        return;
    }
    let kept: String = s.chars().take(max_chars).collect();
    *s = format!("{kept}…[反思上下文裁剪：仅保留前 {max_chars} 字符，其余省略]");
}

/// 递归给 JSON 值内的所有字符串叶子施加同一预算（对象 / 数组逐层下钻）。
fn cap_reflection_strings(value: &mut serde_json::Value, max_chars: usize) {
    match value {
        serde_json::Value::String(s) => cap_reflection_string(s, max_chars),
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                cap_reflection_strings(item, max_chars);
            }
        },
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                cap_reflection_strings(item, max_chars);
            }
        },
        _ => {},
    }
}

/// 三层预算裁剪，保住「决策 / 结论节点」、砍掉「原始数据体积」：
///
/// 1. **叶裁剪**：所有节点内字符串叶子统一限到 [`REFLECTION_MEMORY_LEAF_CHAR_BUDGET`]
///    —— 结论类文本几乎不受影响，被砍的是 raw-data 里整块行情 / 指标 JSON 这类叶子；
/// 2. **节点压缩**：序列化仍超 [`REFLECTION_MEMORY_NODE_CHAR_BUDGET`] 的节点，
///    叶预算按 [`REFLECTION_MEMORY_NODE_SHRINK_CAPS`] 逐档收紧；
/// 3. **总量淘汰（分两趟，P1-2）**：先按体积从大到小淘汰**非保护**节点（原始数据 /
///    工具节点），仍超预算才动保护名单（[`REFLECTION_MEMORY_PROTECTED_NODES`]）里的
///    硬引用节点 —— 旧实现单趟「最肥先砍」在 12 万预算下能靠几个十几万字符的数据块
///    垫背；预算收紧到 3.6 万后，单趟会把四档决策节点（2.5 万级）一并砍掉。
///    占位对象保留原节点的 `__untrusted` 哨兵（comparator 的健康度映射依赖它）。
///
/// 只动 `_raw` 恢复出来的记忆变量，不触碰数据库与其他 prompt 变量。
fn enforce_reflection_memory_budget(nodes: &mut serde_json::Map<String, serde_json::Value>) {
    let before: usize = nodes.values().map(reflection_json_chars).sum();

    // 第 1 层：统一叶预算
    for node in nodes.values_mut() {
        cap_reflection_strings(node, REFLECTION_MEMORY_LEAF_CHAR_BUDGET);
    }

    // 第 2 层：单节点仍超预算 ⇒ 叶预算逐档收紧
    for cap in REFLECTION_MEMORY_NODE_SHRINK_CAPS {
        for node in nodes.values_mut() {
            if reflection_json_chars(node) > REFLECTION_MEMORY_NODE_CHAR_BUDGET {
                cap_reflection_strings(node, cap);
            }
        }
    }

    // 第 3 层：总预算淘汰（非保护节点优先，保护节点最后）
    let mut sizes: Vec<(String, usize)> =
        nodes.iter().map(|(id, node)| (id.clone(), reflection_json_chars(node))).collect();
    sizes.sort_by_key(|(_, size)| std::cmp::Reverse(*size));

    let mut total: usize = sizes.iter().map(|(_, size)| *size).sum();
    for protect_pass in [false, true] {
        if total <= REFLECTION_MEMORY_TOTAL_CHAR_BUDGET {
            break;
        }
        for (node_id, size) in &sizes {
            if total <= REFLECTION_MEMORY_TOTAL_CHAR_BUDGET {
                break;
            }
            if is_protected_reflection_node(node_id) != protect_pass {
                continue;
            }
            let Some(node) = nodes.get(node_id) else { continue };
            if node.get("_truncated").is_some() {
                // 上游趟次已淘汰（sizes 快照按替换前体积记账，这里防重复扣减）
                continue;
            }
            let untrusted = node.get("__untrusted").cloned();
            let mut placeholder = serde_json::json!({
                "_truncated": format!(
                    "节点 {node_id} 原始载荷约 {size} 字符，超出反思上下文总预算已整体省略（原始数据仍可通过工具按 hindsight_date 复查）"
                )
            });
            if let Some(u) = untrusted {
                // 硬引用面里的 `u_*` 映射只取 `__untrusted`：占位保留该哨兵，
                // 让「该节点输出是否可信」在节点被省略后仍可判定。
                placeholder["__untrusted"] = u;
            }
            let placeholder_chars = reflection_json_chars(&placeholder);
            nodes.insert(node_id.clone(), placeholder);
            // 用占位体积替换原体积（占位远小于原体，总量单调下降）
            total = total.saturating_sub(*size).saturating_add(placeholder_chars);
        }
    }

    let after: usize = nodes.values().map(reflection_json_chars).sum();
    if after < before {
        tracing::warn!(
            "[reflection] sub-analysis 上下文预算裁剪: {before} → {after} 字符（总预算 {REFLECTION_MEMORY_TOTAL_CHAR_BUDGET}）"
        );
    }
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

/// P1-1②（`PLAN-stock-decision-reliability.md` R4/R12）：M1 确定性样本的**幂等落库**。
///
/// **成功 / 失败两条路径共用本函数** —— 工作流失败 ≠ 无样本：M1 的原料（主档动作 +
/// 行情快照）在失败前已具备，旧实现只在成功路径物化 ⇒ LLM 侧失败的行在统计里完全
/// 消失（R4「系统永远没有记忆」的一角；2026-10-16 现网实证：failed 行对应的
/// strategy_performance 恒 0 行）。
///
/// **幂等**：同一条反思行（`reflection_id`）至多一行 `reflection_verdict` 样本。
/// 这不改变「重跑」语义 —— 反思行绑定的是固定决策，重跑同行的旧样本与新样本等价，
/// 追加即双重计数（旧注释「不去重、每次重跑追加」正是 bin `delete_stale_reflection_perf`
/// 要擦的那类重复；此处从源头上收敛）。
/// 返回 `Ok(true)` = 本次落了行；`Ok(false)` = 无判定依据（无行情 / 中性档 / 期中观察，
/// D2 语义：不写行）或已有样本（幂等跳过）。
struct M1SampleInputs<'a> {
    reflection_id: &'a str,
    original_analysis: Option<&'a stock_analyses::Model>,
    primary_snapshot: Option<&'a MarketSnapshot>,
    stock_code: &'a str,
    stock_name: &'a str,
    raw_return: Option<f64>,
    holding_days: Option<i32>,
    primary_horizon: &'a str,
    reflected_generation: Option<i32>,
    now_ms: i64,
}

/// 重算「腿 × 档」预测力报告并落 `settings`（P2-a 裁决层的**写侧**；读侧在 `hooks.rs` 注入分支表时）。
///
/// 键名与判据都在 `analysis_engine::leg_weight_overlay`（单一权威源），本函数只管取数—算—存。
async fn refresh_leg_ic_report(db: &DatabaseConnection) -> Result<(), String> {
    use axagent_analysis_engine::leg_weight_overlay as lo;
    let pairs = axagent_dao::repo::stock_lesson_queries::leg_ic_pairs(db).await.map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("读取腿配对失败: {e}")).to_string()
    })?;
    let report = lo::build_report(&pairs, chrono::Utc::now().timestamp_millis());
    axagent_dao::repo::settings::set_setting(db, lo::LEG_IC_SETTING_KEY, &report.to_json_text())
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("写入腿预测力报告失败: {e}"))
                .to_string()
        })?;
    let zeroable = report.cells.iter().filter(|c| lo::only_zeroing_status(&c.status)).count();
    tracing::info!(
        "[leg_ic] 报告刷新：配对 {} 行 / 观测 {} 条 / 分格 {} 个 / 判「无预测力」{} 格（门槛 n>={}）",
        pairs.len(),
        report.observations,
        report.cells.len(),
        zeroable,
        report.min_samples
    );
    Ok(())
}

/// 把反思输出的 `failure_modes` 字段变成落库值（P2-b 的列语义唯一口径）。
///
/// 返回 `(列值, 被丢弃的词表外 token)`。两态必须分得开：
/// - `None` = 模板**没产出**该字段（结构性缺口，面板要报「未标注」）；
/// - `Some("[]")` = 产出了但**一个都不在词表内**（判据缺口，token 已回传供记日志）。
///
/// 入参来自 `deep_get_reflection_text`，故实测三种形态都要吃：数组 / 裸串 / 双重编码的 JSON 字符串。
fn failure_modes_column_of(raw: Option<&str>) -> (Option<String>, Vec<String>) {
    use axagent_analysis_engine::failure_mode::parse_failure_modes;
    let Some(text) = raw else { return (None, Vec::new()) };
    let parsed = parse_failure_modes(text);
    (Some(parsed.to_json_text()), parsed.rejected)
}

async fn persist_m1_sample_if_absent(
    db: &DatabaseConnection,
    inputs: &M1SampleInputs<'_>,
) -> Result<bool, String> {
    // M1 判定（与成功路径同一纯函数、同一输入口径）。
    let Some(was_correct) = deterministic_was_correct(
        inputs.original_analysis.and_then(|a| a.decision_action.as_deref()),
        inputs.primary_snapshot,
    ) else {
        return Ok(false);
    };
    // 幂等查重（见函数文档）。
    let exists =
        axagent_dao::repo::stock_lesson_queries::has_reflection_m1_sample(db, inputs.reflection_id)
            .await
            .map_err(|e| {
                ErrorResponse::new(wf_err::INTERNAL)
                    .with_detail(format!("M1 样本查重失败: {e}"))
                    .to_string()
            })?;
    if exists {
        return Ok(false);
    }
    // decision_confidence：与旧成功路径同源（原分析 `decision_json.confidence`，
    // 回退 `decisionConfidence`；量纲 0-100）。⚠ 取不到时落 0 的已知残留同旧实现。
    let decision_confidence = inputs
        .original_analysis
        .and_then(|a| a.decision_json.as_deref())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|dj| {
            dj.get("confidence").or_else(|| dj.get("decisionConfidence")).and_then(|v| v.as_f64())
        })
        .map(|c| c.round().clamp(0.0, 100.0) as i32)
        .unwrap_or(0);
    let decision_at = inputs.now_ms - (inputs.holding_days.unwrap_or(30) as i64 * 86_400_000);
    let sp_id = uuid::Uuid::new_v4().to_string();
    // P1-1 物化切片：键 = **复盘档名**（与字段文档「各 horizon 的盈亏序列」同形；其余三档在
    // `horizon_results_json`）。⚠ 2026-10-16 上午的中间态把键写成裸 ident `primary_horizon`
    // （该名不在本函数作用域 ⇒ E0425，整棵树编译不过）——按同文件 `results.insert(key, …)`
    // 的既有形态改为显式 Map，语义 = 动态键 `inputs.primary_horizon`。
    let horizon_pnl_json = serde_json::Value::Object({
        let mut m = serde_json::Map::new();
        m.insert(
            inputs.primary_horizon.to_string(),
            serde_json::json!({
                "returnPct": inputs.raw_return,
                "wasCorrect": was_correct,
                "holdingDays": inputs.holding_days,
            }),
        );
        m
    })
    .to_string();
    let draft = axagent_dao::repo::stock_lesson_queries::ReflectionM1Draft {
        id: sp_id.clone(),
        reflection_id: inputs.reflection_id.to_string(),
        stock_code: inputs.stock_code.to_string(),
        stock_name: inputs.stock_name.to_string(),
        period: format!("reflection:{}", inputs.primary_horizon),
        decision_at,
        exit_at: inputs.now_ms,
        holding_days: inputs.holding_days.unwrap_or(30),
        return_pct: inputs.raw_return.unwrap_or(0.0),
        was_correct,
        decision_confidence,
        // #31：与反思行同一个建点盖章 —— 被复盘决策的代际（权威仍是分析行）。
        template_version: inputs.reflected_generation,
        // 复盘档的客观判定切片（其余三档在 horizon_results_json）。
        horizon_pnl_json: Some(horizon_pnl_json),
    };
    axagent_dao::repo::stock_lesson_queries::insert_reflection_m1_sample(db, draft).await.map_err(
        |e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("写入 strategy_performance 失败: {e}"))
                .to_string()
        },
    )?;
    tracing::info!(
        "[reflection] M1 样本落库 {sp_id}: reflection_id={} was_correct={was_correct} conf={decision_confidence}",
        inputs.reflection_id
    );
    Ok(true)
}

/// 构建四周期反思结果 JSON（PLAN-stock-reflection-four-horizon 批次 2+3）。
///
/// 从 `horizon_decisions` 解析四周期决策，结合四周期行情快照与 LLM 反思输出，
/// 组装为符合目标数据契约的 `horizon_results_json`。
///
/// **状态判定**（对每个周期）：
/// - 有快照 + 已到期望持有期 → `mature`
/// - 有快照 + 未到期望持有期 → `immature`
/// - 有决策但无快照 → `unavailable`
/// - 无四周期决策的旧记录 → `legacy` 单周期回退
fn build_horizon_results_json(
    original_analysis: Option<&stock_analyses::Model>,
    horizon_snapshots: &std::collections::BTreeMap<String, Option<MarketSnapshot>>,
    reflection_json: &serde_json::Value,
) -> Option<String> {
    let primary_horizon =
        original_analysis.and_then(|a| a.decision_time_horizon.as_deref()).unwrap_or("short");

    let horizon_decisions: Option<std::collections::HashMap<String, _>> = original_analysis
        .and_then(|a| a.horizon_decisions.as_deref())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .and_then(|obj| parse_horizon_decisions(&obj).ok());

    let horizon_keys = ["ultra_short", "short", "mid", "long"];

    // 如果四周期决策全缺且旧字段也无决策 → 不生成 JSON（旧记录无数据）
    let has_horizon_decisions = horizon_decisions.is_some();
    let has_legacy_data = original_analysis.and_then(|a| a.decision_action.as_deref()).is_some();

    if !has_horizon_decisions && !has_legacy_data {
        return None;
    }

    // 提取 LLM 反思字段（与 UPDATE 路径共用同一提取逻辑）
    let verdict = deep_get_reflection_str(reflection_json, "verdict");
    let what_went_wrong = deep_get_reflection_str(reflection_json, "what_went_wrong");
    let missed_signals = deep_get_reflection_text(reflection_json, "missed_signals");
    let fix_for_future = deep_get_reflection_str(reflection_json, "fix_for_future");
    let lesson_summary = deep_get_reflection_str(reflection_json, "lesson_summary");

    let mut results = serde_json::Map::new();

    if let Some(ref decisions) = horizon_decisions {
        // 有四周期决策：每个周期独立构造结果
        for &key in &horizon_keys {
            let decision = match decisions.get(key) {
                Some(d) => d,
                None => {
                    results.insert(key.to_string(), serde_json::Value::Null);
                    continue;
                },
            };

            let expected_holding_days = decision
                .expected_holding_days
                .unwrap_or_else(|| default_expected_holding_days(key));

            let snap = horizon_snapshots.get(key).and_then(|s| s.as_ref());

            // 状态判定：每周期独立查找自己的快照
            let (status, market_json, eval_json, reflection_section) = match snap {
                Some(snap) => {
                    let status = if snap.within_expected_horizon {
                        HorizonStatus::Immature
                    } else {
                        HorizonStatus::Mature
                    };
                    let was_correct = deterministic_horizon_was_correct(
                        &decision.action,
                        snap.net_return_pct,
                        snap.within_expected_horizon,
                    );

                    let mut market = serde_json::json!({
                        "entryPrice": snap.entry_price,
                        "exitPrice": snap.latest_price,
                        "returnPct": snap.net_return_pct,
                        "alphaPct": snap.alpha_pct,
                        "maxDrawdownPct": snap.max_drawdown_pct,
                        "targetReached": snap.target_reached,
                    });
                    market["stopLossTriggered"] = serde_json::Value::Null;

                    let mut evaluation = serde_json::json!({
                        "wasCorrect": was_correct,
                        "directionMatch": was_correct.map(|v| v == 1),
                        "targetHit": snap.target_reached,
                    });
                    evaluation["mismatch"] = serde_json::Value::Null;

                    let reflection = if reflection_json.is_null() {
                        serde_json::json!({
                            "verdict": verdict,
                            "whatWentWrong": what_went_wrong,
                            "missedSignals": missed_signals,
                            "fixForFuture": fix_for_future,
                            "lessonSummary": lesson_summary,
                        })
                    } else {
                        reflection_json.clone()
                    };

                    (status, Some(market), Some(evaluation), Some(reflection))
                },
                None => (HorizonStatus::Unavailable, None, None, None),
            };

            results.insert(
                key.to_string(),
                serde_json::json!({
                    "status": status,
                    "expectedHoldingDays": expected_holding_days,
                    "actualHoldingDays": snap.map(|s| s.trading_days),
                    "decision": {
                        "action": decision.action,
                        "positionPct": decision.position_pct,
                        "targetPrice": decision.target_price,
                        "stopLoss": decision.stop_loss,
                        "confidence": decision.confidence,
                        // 判定口径水印（v104 起才有）：缺它 = 上一代 confidence 刻度，
                        // rank IC 必须把它挡在分母外（详见 reflection_stats 的同名测试）。
                        "snrAnchorDays": decision.snr_anchor_days,
                    },
                    "market": market_json,
                    "evaluation": eval_json,
                    "reflection": reflection_section,
                }),
            );
        }
    } else {
        // 无四周期决策（旧记录）：为每个旧周期键生成 legacy 条目
        for &key in &horizon_keys {
            let is_primary = key == primary_horizon;
            if !is_primary {
                results.insert(key.to_string(), serde_json::Value::Null);
                continue;
            }

            let action = original_analysis.and_then(|a| a.decision_action.as_deref()).unwrap_or("");
            let expected_holding_days = original_analysis
                .and_then(|a| a.decision_expected_holding_days)
                .unwrap_or_else(|| default_expected_holding_days(key));

            let snap = horizon_snapshots.get(key).and_then(|s| s.as_ref());

            let (status, market_json, eval_json, reflection_section) = match snap {
                Some(snap) => {
                    let was_correct = if action.is_empty() || snap.within_expected_horizon {
                        None
                    } else {
                        deterministic_was_correct(Some(action), Some(snap))
                    };

                    let mut market = serde_json::json!({
                        "entryPrice": snap.entry_price,
                        "exitPrice": snap.latest_price,
                        "returnPct": snap.net_return_pct,
                        "alphaPct": snap.alpha_pct,
                        "maxDrawdownPct": snap.max_drawdown_pct,
                        "targetReached": snap.target_reached,
                    });
                    market["stopLossTriggered"] = serde_json::Value::Null;

                    let mut evaluation = serde_json::json!({
                        "wasCorrect": was_correct,
                        "directionMatch": was_correct.map(|v| v == 1),
                        "targetHit": snap.target_reached,
                    });
                    evaluation["mismatch"] = serde_json::Value::Null;

                    let reflection = if reflection_json.is_null() {
                        serde_json::json!({
                            "verdict": verdict,
                            "whatWentWrong": what_went_wrong,
                            "missedSignals": missed_signals,
                            "fixForFuture": fix_for_future,
                            "lessonSummary": lesson_summary,
                        })
                    } else {
                        reflection_json.clone()
                    };

                    (HorizonStatus::Legacy, Some(market), Some(evaluation), Some(reflection))
                },
                None => (HorizonStatus::Unavailable, None, None, None),
            };

            results.insert(
                key.to_string(),
                serde_json::json!({
                    "status": status,
                    "expectedHoldingDays": expected_holding_days,
                    "actualHoldingDays": snap.map(|s| s.trading_days),
                    "decision": {
                        "action": action,
                        "positionPct": serde_json::Value::Null,
                        "targetPrice": snap.and_then(|s| s.target_price),
                        "stopLoss": serde_json::Value::Null,
                        "confidence": serde_json::Value::Null,
                    },
                    "market": market_json,
                    "evaluation": eval_json,
                    "reflection": reflection_section,
                }),
            );
        }
    }

    // 追加 schema 版本号
    results.insert(
        "schemaVersion".to_string(),
        serde_json::Value::Number(serde_json::Number::from(1)),
    );

    serde_json::to_string(&serde_json::Value::Object(results)).ok()
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
// 为什么只测渲染层：`compute_market_snapshots` 依赖 AStockClient 实时取数，
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
            decision_horizon_source: None,
            decision_expected_holding_days: Some(28),
            // A4：NULL = 采集时点无版本信息（A4 之前的存量行、chat 通道写入均为此形态）
            template_version: None,
            // 模板 id：本测试用例不关心链路归属，按「未知」置 NULL（语义同上方）
            template_id: None,
            outcome: None,
            llm_decision_json: None,
            parent_analysis_id: None,
            // P2 溯源三列：本夹具是普通 live 分析（非补分析）
            source: "live".to_string(),
            synthetic: 0,
            anchor_event_id: None,
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

    // ── 「样本不足」与「取数失败」的分道（2026-10-16 现场回归）──
    //
    // 现场：002371（as_of 10-14）在 10-16 凌晨批次里报「行情快照不可用,降级」并被写成
    // completed —— 当时东财已有 10-15 的 K 线（只有一根、未跨过完整交易日），不是取数故障。
    // 根因：取数侧把「不足」塞进 `Err`，消费侧却把「跳过」判在 `Ok(快照)` 支上。

    /// 只有入场那一根 ⇒ 未跨过完整交易日 ⇒ `None`（样本不足）；多出一根才可用。
    #[test]
    fn horizon_window_requires_a_full_day_after_entry() {
        // 002371 形态：2 根 K 线、入场=最后一根
        assert_eq!(horizon_window(2, 1, 2), None, "仅入场一根不得算可用窗口");
        // 负控（另一头）：入场后又多出一根 ⇒ 可用，天数=1
        assert_eq!(horizon_window(3, 1, 2), Some((2, 1)));
        // 出场被期望持有期截断
        assert_eq!(horizon_window(10, 1, 2), Some((3, 2)));
        // 尚未到期望持有期（期中观察）：出场=最新一根，天数如实
        assert_eq!(horizon_window(4, 1, 90), Some((3, 2)));
    }

    /// 三种缺口必须分道：`Young` 保 `Young`、`Failed` 保因、可用不被截走。
    #[test]
    fn primary_snapshot_routes_young_and_failed_apart() {
        let young: Result<MarketSnapshot, SnapshotGap> = Err(SnapshotGap::Young);
        assert!(
            matches!(take_primary_snapshot(Some(&young)), Err(SnapshotGap::Young)),
            "样本不足必须原样透出为 Young（它决定「跳过留 pending」而不是「降级写盲反思」）"
        );

        let failed: Result<MarketSnapshot, SnapshotGap> =
            Err(SnapshotGap::Failed("基准 K 线获取失败: timeout".into()));
        match take_primary_snapshot(Some(&failed)) {
            Err(SnapshotGap::Failed(r)) => assert!(r.contains("timeout"), "原因不得被吞: {r}"),
            Err(SnapshotGap::Young) => panic!("取数失败被误判成样本不足"),
            Ok(_) => panic!("取数失败不得被当作可用快照"),
        }

        // 负控：可用快照不得被任何缺口分支截走
        let ready: Result<MarketSnapshot, SnapshotGap> = Ok(sample());
        assert!(take_primary_snapshot(Some(&ready)).is_ok());

        // 键缺失（理论不可达）归取数失败：至少降级留痕，不伪装成「再等等」
        assert!(matches!(take_primary_snapshot(None), Err(SnapshotGap::Failed(_))));
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
    /// Phase 5（〇-B v2 第 4 条）：教训按周期档隔离 —— 这是「四周期闭环」的最后一环。
    ///
    /// 缺陷原形态（PLAN 断点⑤，改前实证）：`reflection_lessons` **没有周期列**，
    /// 去重键 = `(stock_code, lesson_summary)`，消费入口 `fetch_stock_lessons(stock_code)`
    /// 也不带档 ⇒ 超短线的教训（「次日冲高回落就走」）被原样注入长线分析的 prompt。
    ///
    /// 本测试同时验三件事：
    ///   ① 新列由「实体声明 + schema 自愈」自动建出（**没有**手写 migration）；
    ///   ② 同档与通用（NULL）教训可读，**跨档教训被排除**；
    ///   ③ 同一条文字教训在不同档**不被去重合并**（否则两档统计互相吞掉）。
    #[tokio::test]
    async fn lessons_are_isolated_per_horizon() {
        use axagent_entities::reflection_lessons;
        use sea_orm::{ActiveValue, Set};

        let db = axagent_dao::db::create_test_pool().await.expect("测试库应可创建").conn;
        let code = "600519";
        let now = 1_700_000_000_000_i64;
        let mk = |id: &str, h: Option<&str>| reflection_lessons::ActiveModel {
            id: Set(id.to_string()),
            lesson_summary: Set(format!("教训文本 {id}，足够长可以过长度门")),
            rule_pattern: ActiveValue::NotSet,
            source_reflection_id: Set(Some("r-1".to_string())),
            stock_code: Set(Some(code.to_string())),
            horizon: Set(h.map(str::to_string)),
            // §七十一：本夹具只验档隔离，代际留 NULL（= 代际未知，随同代一起注入）
            template_version: ActiveValue::NotSet,
            applicable_scenarios: ActiveValue::NotSet,
            times_applied: Set(0),
            success_count: Set(0),
            confidence: Set(0.9),
            status: Set("active".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        for (id, h) in [("l-same", Some("long")), ("l-other", Some("short")), ("l-any", None)] {
            mk(id, h).insert(&db).await.expect("插入 lesson 应成功");
        }

        let (text, ids) = super::super::core::fetch_stock_lessons(code, &db, Some("long")).await;
        let text = text.unwrap_or_default();
        assert!(text.contains("教训文本 l-same"), "同档教训应被注入: {text}");
        assert!(text.contains("教训文本 l-any"), "通用（NULL 档）教训应被注入: {text}");
        assert!(!text.contains("教训文本 l-other"), "跨档（short）教训不得进长线 prompt: {text}");
        let _ = ids;

        // ③ 去重键含 horizon：同一文字教训在两档是两行
        async fn cnt(db: &sea_orm::DatabaseConnection, h: &str) -> usize {
            reflection_lessons::Entity::find()
                .filter(
                    reflection_lessons::Column::LessonSummary.eq("重复教训文本，长度足够通过门槛"),
                )
                .filter(reflection_lessons::Column::Horizon.eq(h))
                .all(db)
                .await
                .unwrap()
                .len()
        }
        let dup = |id: &str, h: &str| reflection_lessons::ActiveModel {
            id: Set(id.to_string()),
            lesson_summary: Set("重复教训文本，长度足够通过门槛".to_string()),
            rule_pattern: ActiveValue::NotSet,
            source_reflection_id: Set(Some("r-2".to_string())),
            stock_code: Set(Some(code.to_string())),
            horizon: Set(Some(h.to_string())),
            template_version: ActiveValue::NotSet,
            applicable_scenarios: ActiveValue::NotSet,
            times_applied: Set(0),
            success_count: Set(0),
            confidence: Set(0.7),
            status: Set("active".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        dup("d-long", "long").insert(&db).await.expect("插入 long 档重复教训");
        dup("d-short", "short").insert(&db).await.expect("插入 short 档重复教训");
        assert_eq!(cnt(&db, "long").await, 1, "long 档应恰有一行");
        assert_eq!(cnt(&db, "short").await, 1, "short 档应恰有一行（未被 long 档去重吞掉）");
    }

    /// §五十一-② 起算代际 / §七十一 按版归属：错题本（`reflection_lessons` + `stock_reflections`）
    /// 按**算法代际下限**隔离。
    ///
    /// 与上一条按档隔离同族，但风险高一档：换代后旧代的教训**看起来仍是「同股同档」**，
    /// 按档门对它完全无感。这里按数值/文本断四件事：
    ///   ① 两个新列由「实体声明 + schema 自愈」自动建出（**没有**手写 migration ——
    ///      插得进带 `template_version` 的行就是它的直接证据）；
    ///   ② 起算代及之后 + 代际未知(NULL)的行进 prompt，**早于起算代的行被排除**（两张表都要）；
    ///   ③ 被排除的条数**出现在注入文本里**（否则「0 条」会被模型读成「这只票没教训」，
    ///      而真因是「教训全在起算代之前」—— 两种缺席含义相反，不许静默合并）；
    ///   ④ 负控：把起算代降到旧代之下 ⇒ 旧代行**必须在**，且不得声称筛过。
    ///      没有这半条，②只要数据没插进去也能"绿"。
    #[tokio::test]
    async fn lessons_are_isolated_per_generation() {
        use axagent_entities::{reflection_lessons, stock_reflections};
        use sea_orm::{ActiveValue, Set};

        let db = axagent_dao::db::create_test_pool().await.expect("测试库应可创建").conn;
        let code = "600276";
        // ⚠ 反思行的 `created_at` 必须是**墙钟近期**：`fetch_same_ticker_completed` 带
        // 「近 90 天」窗，而规则化教训那侧没有窗 —— 夹具不贴窗，反思行会被整体滤掉。
        let now = chrono::Utc::now().timestamp_millis();
        // 夹具**相对起算代构造**：新旧两代由 floor 现算 ⇒「骑在界两侧」由构造保证，
        // 不需要再写 `assert!(常量 > 常量)`（那会被 clippy::assertions_on_constants 判红，
        // 而加 allow 只是把判据藏起来）。
        let floor = axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR;
        let old_gen = floor - 4; // 起算代之前

        // ── 规则化教训：起算代之后 / 之前 / 代际未知（档一律给 long，排除档维度干扰）──
        let mk_lesson = |id: &str, g: Option<i32>| reflection_lessons::ActiveModel {
            id: Set(id.to_string()),
            lesson_summary: Set(format!("代际教训文本 {id}，长度足够通过门槛")),
            rule_pattern: ActiveValue::NotSet,
            source_reflection_id: Set(Some("r-gen".to_string())),
            stock_code: Set(Some(code.to_string())),
            horizon: Set(Some("long".to_string())),
            template_version: Set(g),
            applicable_scenarios: ActiveValue::NotSet,
            times_applied: Set(0),
            success_count: Set(0),
            confidence: Set(0.9),
            status: Set("active".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        for (id, g) in [("g-new", Some(floor + 4)), ("g-old", Some(old_gen)), ("g-unknown", None)] {
            mk_lesson(id, g).insert(&db).await.expect("插入 lesson 应成功（含新列）");
        }

        // ── 反思行：起算代之后 / 之前（status 必须 completed 才会被注入）──
        // ⚠ 前置夹具：注入侧按「被复盘的分析有没有留逐节点依据（blackboard_snapshot）」筛除
        //（2026-10-14 §五十一-② 依据过滤，dao `partition_by_analysis_evidence`）。反思行指向的
        // 分析行不存在 ⇒ 与「快照为 NULL」同处置，整条被筛掉，本门测的「按代际隔离」根本走不到。
        // 因此两条反思各配一条**有快照**的分析行；快照内容本门不解析，非 NULL 即算有依据。
        let mk_analysis = |id: &str| axagent_entities::stock_analyses::ActiveModel {
            id: Set(id.to_string()),
            stock_code: Set(code.to_string()),
            stock_name: Set("恒瑞医药".to_string()),
            analysis_date: Set("2026-09-01".to_string()),
            provider_id: Set("test".to_string()),
            conversation_id: Set("test-conv".to_string()),
            status: Set("completed".to_string()),
            blackboard_snapshot: Set(Some("{}".to_string())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        for id in ["a-r-new", "a-r-old"] {
            mk_analysis(id).insert(&db).await.expect("插入被复盘的分析行应成功（带黑板快照）");
        }
        let mk_row = |id: &str, g: Option<i32>| stock_reflections::ActiveModel {
            id: Set(id.to_string()),
            stock_code: Set(code.to_string()),
            stock_name: Set("恒瑞医药".to_string()),
            original_analysis_id: Set(format!("a-{id}")),
            as_of_date: Set("2026-09-01".to_string()),
            hindsight_date: Set("2026-09-29".to_string()),
            min_confidence_threshold: Set(0),
            reflection_depth: Set("light".to_string()),
            horizon: Set(Some("long".to_string())),
            actual_outcome: Set("上涨".to_string()),
            lesson_summary: Set(Some(format!("反思教训 {id}，长度足够可注入"))),
            template_version: Set(g),
            source: Set(REFLECTION_SOURCE_DUE.to_string()),
            miss_layer: Set(None),
            failure_modes: Set(None),
            status: Set("completed".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        for (id, g) in [("r-new", Some(floor + 1)), ("r-old", Some(old_gen))] {
            mk_row(id, g).insert(&db).await.expect("插入反思行应成功（含新列）");
        }

        // ── ① 按起算代筛样 ──
        let (text, _ids) = super::super::core::fetch_stock_lessons(code, &db, Some("long")).await;
        let text = text.expect("起算代筛样下仍应有内容（新代行与未知代行都在）");
        assert!(text.contains("代际教训文本 g-new"), "起算代之后的规则应被注入: {text}");
        assert!(text.contains("代际教训文本 g-unknown"), "代际未知(NULL)规则应被注入: {text}");
        assert!(text.contains("反思教训 r-new"), "起算代之后的反思应被注入: {text}");
        assert!(!text.contains("g-old"), "早于起算代的规则不得进 prompt: {text}");
        assert!(!text.contains("r-old"), "早于起算代的反思不得进 prompt: {text}");
        // 两张表各有 1 条旧代 ⇒ 两个组各声明一次（条数必须写出来，否则「为什么没有」不可见）
        assert_eq!(
            text.matches("另有 1 条早于起算代（第 125 代），未注入").count(),
            2,
            "规则组与反思组都要声明被筛掉的条数: {text}"
        );

        // ── ② 前提的可读见证：夹具骑在起算代两侧（由上面的构造保证）────────────
        // 起算代自 §七十七 起由 `fetch_stock_lessons` **内部**解析 ⇒ 本测试不能再把 floor
        // 压低当负控。取而代之的是把三类样本都在场的**事实**打在日志里（夹具是相对 floor
        // 现构的：`old_gen = floor-4`、`floor+1`、`floor+4`、NULL），并断言 NULL 那条确实进来了
        // —— 它证明「不含旧代」不是把整表滤空，而是**只**滤掉界下的那一条。
        assert!(
            text.contains("g-unknown"),
            "代际未知的行必须仍在（它证明过滤只剔界下那条，不是整表滤空）: {text}"
        );
    }
}

// ── 一行 pending = 一个复盘档（PLAN-reflection-per-horizon-row 门）──
#[cfg(test)]
mod pending_rows_per_horizon_tests {
    use super::*;
    use sea_orm::ActiveValue;

    type Am = axagent_entities::stock_reflections::ActiveModel;

    fn seed<'a>(
        hd: Option<&'a str>,
        primary: Option<&'a str>,
        days: Option<i64>,
    ) -> PendingReflectionSeed<'a> {
        PendingReflectionSeed {
            analysis_id: "a-1",
            stock_code: "600519",
            stock_name: "贵州茅台",
            analysis_date: "2026-08-03",
            horizon_decisions: hd,
            primary_horizon: primary,
            primary_holding_days: days,
            min_confidence_threshold: DEFAULT_REFLECTION_MIN_CONFIDENCE,
            reflection_depth: DEFAULT_REFLECTION_DEPTH,
            analysis_template_version: None,
            source: REFLECTION_SOURCE_DUE,
            now_ms: 1_754_200_000_000,
        }
    }

    fn horizon_of(am: &Am) -> Option<String> {
        match &am.horizon {
            ActiveValue::Set(v) => v.clone(),
            _ => panic!("horizon 必须在建点即 Set（不再留 NULL 等收尾盖章）"),
        }
    }

    fn hindsight_of(am: &Am) -> String {
        match &am.hindsight_date {
            ActiveValue::Set(v) => v.clone(),
            _ => panic!("hindsight_date 未设"),
        }
    }

    /// 裁定 Q1：四档里只有**决策有方向**的档建行；观望/持有档不产空洞教训。
    #[test]
    fn only_directional_horizons_get_a_row() {
        let hd = r#"{"ultra_short":{"action":"买入"},"short":{"action":"观望"},
                     "mid":{"action":"持有"},"long":{"action":"减持"}}"#;
        let rows = build_pending_reflection_rows(&seed(Some(hd), Some("mid"), Some(28)));
        let got: Vec<String> = rows.iter().map(|r| horizon_of(r).unwrap()).collect();
        assert_eq!(
            got,
            vec!["ultra_short".to_string(), "long".to_string()],
            "档序须按 Period::ALL"
        );
    }

    /// 2026-10-16：`decisionsByHorizon` 的**生产键是 camelCase**（全库 100/100 为
    /// `ultraShort`；snake 键只存在于本 mod 的旧夹具里）—— 逐档建点必须两族键都认：
    /// 旧实现只取 snake ⇒ ultra 档在真实链路上系统性 miss（现网 ultra 逐档反思行
    /// 仅 6 条、且全部来自「主档恰为 ultra」的兜底路径，逐档路径一条都没有）。
    #[test]
    fn camel_case_ultra_short_key_is_recognized() {
        let hd = r#"{"ultraShort":{"action":"买入"},"short":{"action":"观望"},
                     "mid":{"action":"持有"},"long":{"action":"减持"}}"#;
        let rows = build_pending_reflection_rows(&seed(Some(hd), Some("mid"), Some(28)));
        let got: Vec<String> = rows.iter().map(|r| horizon_of(r).unwrap()).collect();
        assert!(
            got.contains(&"ultra_short".to_string()),
            "camel 键 ultraShort 必须被识别并建行（生产形态）: {got:?}"
        );
        assert!(got.contains(&"long".to_string()), "方向档 long 必须建行: {got:?}");
    }

    /// 每档的 hindsight_date 必须按**该档**天数外推，不能四档共用主档一个窗口。
    #[test]
    fn each_row_carries_its_own_horizon_hindsight_date() {
        let hd = r#"{"ultra_short":{"action":"买入"},"short":{"action":"增持"},
                     "mid":{"action":"减持"},"long":{"action":"卖出"}}"#;
        let rows = build_pending_reflection_rows(&seed(Some(hd), Some("mid"), Some(28)));
        assert_eq!(rows.len(), 4);
        let by: std::collections::BTreeMap<String, String> =
            rows.iter().map(|r| (horizon_of(r).unwrap(), hindsight_of(r))).collect();
        assert!(
            by["ultra_short"] < by["short"] && by["short"] < by["mid"] && by["mid"] < by["long"],
            "hindsight 必须随档位天数单调后移：{by:?}"
        );
    }

    /// 缺陷 ④ 的旧形态（一分析一行、档留 NULL）不得复发：无逐档决策时也只建**一行**，
    /// 且该行档位名必须 ∈ Period::ALL。
    #[test]
    fn missing_horizon_decisions_yield_single_primary_row() {
        let rows = build_pending_reflection_rows(&seed(None, Some("long"), Some(90)));
        assert_eq!(rows.len(), 1);
        assert_eq!(horizon_of(&rows[0]).as_deref(), Some("long"));

        // 负控：主档也缺失但声明了期望天数 ⇒ 按最近邻归一（不采信 LLM 措辞）
        let rows2 = build_pending_reflection_rows(&seed(None, None, Some(6)));
        assert_eq!(horizon_of(&rows2[0]).as_deref(), Some("short"));

        // 负控（P1-9 抽样裁定）：四档全为中性 ⇒ **只给抽中的档建行**，一个没抽中就是空表。
        // ⚠ 不得退回「无条件给主档建一行」：那等于让 20% 抽样全部作弊命中（本函数上方
        //   `if !parsed.is_empty() { return out; }` 的注释与 bin 的空名单文案同口径）。
        // 期望集按**同一条 `sampled_in` 判据**现推，而不是写死条数 —— 写死就是把这格的
        // 成败交给 token 哈希的运气（夹具 token 固定，抽中哪几档是可算的，不是巧合）。
        let all_neutral = r#"{"ultra_short":{"action":"观望"},"short":{"action":"持有"},
                              "mid":{"action":"观望"},"long":{"action":"持有"}}"#;
        let rows3 = build_pending_reflection_rows(&seed(Some(all_neutral), Some("mid"), Some(28)));
        let expected: Vec<String> = Period::ALL
            .iter()
            .filter(|p| sampled_in("a-1", p.as_str()))
            .map(|p| p.as_str().to_string())
            .collect();
        let got3: Vec<String> = rows3.iter().map(|r| horizon_of(r).unwrap()).collect();
        assert_eq!(got3, expected, "全中性档的建点集合必须恰为被抽中的那几档: {got3:?}");
        assert!(
            expected.len() < Period::ALL.len(),
            "20% 抽样下不可能四档全命中（全命中=伪复盘四行），实得 {expected:?}"
        );
    }

    /// `build_pending_reflection_rows_for` 只补给定档集（bin 的「缺哪档补哪档」）。
    #[test]
    fn rows_for_subset_builds_only_that_horizon() {
        // 种子本身有 4 个**有方向**的档，而这里只传 [long] ⇒ 必须恰建 1 行。
        // 负控：若 `_for` 忽略 horizons 参数、自己重算 ⇒ 会建 4 行，本用例当场红
        // （第一版就是这么写的，靠这条才发现参数被吞掉）。
        let hd = r#"{"ultra_short":{"action":"买入"},"short":{"action":"增持"},
                     "mid":{"action":"减持"},"long":{"action":"卖出"}}"#;
        let rows = build_pending_reflection_rows_for(
            &seed(Some(hd), Some("mid"), Some(28)),
            vec![("long".to_string(), 90)],
        );
        assert_eq!(rows.len(), 1, "_for 必须只建传入的档集，不得自行重算四档");
        assert_eq!(horizon_of(&rows[0]).as_deref(), Some("long"));
    }

    /// §七十一 按版归属：pending 行的代际由建点即盖，且**逐档各行同代**。
    ///
    /// 为什么单独一条：`template_version` 与 `horizon` 是两条正交的归属轴，档对而代错
    /// （或反之）都会让错题本沉淀出跨代样本，而按档那组门对此完全无感。
    /// 负控（`None` 那半）锁的是「不许把未知冒充成某一代」—— 这是 chat 通道与手动反思的
    /// 真实形态（成因不同但结论相同：NULL 就写 NULL）。
    #[test]
    fn pending_rows_carry_the_reflected_analyses_generation() {
        let hd = r#"{"ultra_short":{"action":"买入"},"short":{"action":"增持"},
                     "mid":{"action":"减持"},"long":{"action":"卖出"}}"#;
        let generation_of = |am: &Am| match &am.template_version {
            ActiveValue::Set(v) => *v,
            _ => panic!("template_version 必须在建点即 Set，不得留 NotSet 等收尾盖章"),
        };

        let mut s = seed(Some(hd), Some("mid"), Some(28));
        s.analysis_template_version = Some(129);
        let rows = build_pending_reflection_rows(&s);
        assert_eq!(rows.len(), 4, "夹具四档都有方向");
        for r in &rows {
            assert_eq!(
                generation_of(r),
                Some(129),
                "同一笔决策的四档反思必须同代（复盘的是同一条分析）"
            );
        }

        s.analysis_template_version = None;
        for r in &build_pending_reflection_rows(&s) {
            assert_eq!(generation_of(r), None, "代际未知必须如实落 NULL");
        }
    }
}

// ── P3 妖股漏检回放：专项上下文构造 ──
#[cfg(test)]
mod mover_recall_context_tests {
    use super::*;
    use axagent_entities::mover_recall_events::Model as MreModel;

    /// 只盖测试关心的标量字段，其余给类型默认值（上下文函数只读这 7 个字段）。
    fn mk_event(miss_layer: &str) -> MreModel {
        MreModel {
            id: "mre-1".into(),
            run_id: "run-1".into(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            period: "short".into(),
            anchor_date: "2026-08-29".into(),
            window_start: "2026-08-22".into(),
            window_end: "2026-08-29".into(),
            window_days: 5,
            cum_gain_pct: 23.5,
            max_daily_pct: 9.9,
            threshold_pct: 20.0,
            market_type: "main".into(),
            miss_layer: miss_layer.into(),
            picked_any_period: 0,
            scored_out_styles_json: None,
            batch_id: None,
            analysis_id: Some("a-1".into()),
            reflection_id: None,
            status: "analyzed".into(),
            error_detail: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn none_event_yields_placeholder_that_says_skip() {
        let text = mover_recall_context_text(None);
        assert!(text.contains("常规到期反思"), "占位句须自报常规反思: {text}");
        assert!(text.contains("忽略漏检专项三问"), "占位句须指示忽略三问: {text}");
    }

    #[test]
    fn event_block_carries_facts_layer_and_three_questions() {
        let text = mover_recall_context_text(Some(&mk_event("l3_scored_out")));
        // 窗口硬数字
        assert!(text.contains("2026-08-22"), "窗口起点: {text}");
        assert!(text.contains("2026-08-29"), "窗口右端: {text}");
        assert!(text.contains("5 个交易日"), "窗口天数: {text}");
        assert!(text.contains("+23.5%"), "累计涨幅: {text}");
        assert!(text.contains("+9.9%"), "最大单日: {text}");
        assert!(text.contains("short"), "档位: {text}");
        // 权威层码 + 中文语义
        assert!(text.contains("l3_scored_out"), "层码必须原样出现: {text}");
        assert!(text.contains("L3 评分阶段被截断"), "层码中文语义: {text}");
        // 固定三问锚点
        assert!(text.contains("missed_signals"), "一问字段名: {text}");
        assert!(text.contains("what_went_wrong"), "二问字段名: {text}");
        assert!(text.contains("fix_for_future"), "三问字段名: {text}");
        assert!(text.contains("已经可见"), "一问须限定窗口起点可见: {text}");
        assert!(text.contains("可执行的判据修正"), "三问须要求可执行: {text}");
    }

    #[test]
    fn every_known_layer_has_chinese_semantic() {
        for layer in ["not_in_pool", "l3_scored_out", "other_period", "unexplained"] {
            let text = mover_recall_context_text(Some(&mk_event(layer)));
            assert!(text.contains(&format!("{layer} ——")), "{layer} 层码行缺失: {text}");
            // 不得对已知层码退化为裸码（other 分支只会对未知码触发）
            assert!(!text.contains(&format!("：{layer}\n")), "层码缺中文语义: {text}");
        }
    }
}

/// 反思记忆（`sub-analysis`）上下文预算裁剪 —— 2026-10-14 反思输入 72.7 万
/// token 超模型 524k 上限事故的回归锁。
#[cfg(test)]
mod reflection_memory_budget_tests {
    use super::*;
    use serde_json::{Map, json};

    #[test]
    fn cap_string_keeps_short_and_marks_long() {
        let mut short = "短结论不动".to_string();
        cap_reflection_string(&mut short, 10);
        assert_eq!(short, "短结论不动");

        let mut long = "甲".repeat(100);
        cap_reflection_string(&mut long, 10);
        // 按 Unicode 字符保留前 10 个，不切碎汉字
        assert_eq!(long.chars().filter(|c| *c == '甲').count(), 10);
        assert!(long.contains("反思上下文裁剪"), "截断标记缺失: {long}");
        assert!(long.contains("仅保留前 10 字符"), "保留长度须记入标记: {long}");
    }

    #[test]
    fn cap_strings_recurses_objects_and_arrays() {
        let mut v = json!({
            "result": "乙".repeat(50_000),
            "rows": [{"text": "丙".repeat(20_000)}, "短", [{"deep": "丁".repeat(12_000)}]]
        });
        cap_reflection_strings(&mut v, 8_000);
        // 按字符（非 UTF-8 字节）断言：中文每字 3 字节
        assert!(v["result"].as_str().unwrap().chars().count() < 8_100);
        assert!(v["rows"][0]["text"].as_str().unwrap().chars().count() < 8_100);
        assert_eq!(v["rows"][1], json!("短"));
        assert!(v["rows"][2][0]["deep"].as_str().unwrap().chars().count() < 8_100);
    }

    #[test]
    fn small_snapshot_passes_through_with_content_parsed() {
        let snapshot = json!({
            "_raw.trader": {"content": json!({"action": "买入", "conviction": 0.8}).to_string()},
            "_raw.debate-bull-bear": {"content": "多方占优，风险可控。"}
        })
        .to_string();
        let v = build_sub_analysis_from_snapshot(Some(&snapshot), "600519");
        // content JSON 字符串被解析为对象，点路径可下钻
        assert_eq!(v["trader"]["content"]["action"], json!("买入"));
        assert_eq!(v["trader"]["content"]["conviction"], json!(0.8));
        assert_eq!(v["debate-bull-bear"]["content"], json!("多方占优，风险可控。"));
    }

    #[test]
    fn empty_node_id_key_is_skipped() {
        // 键字面 `_raw.`（空 node_id）实测是 build_blackboard_snapshot 的畸形重复写入
        let snapshot = json!({
            "_raw.": {"content": "畸形重复载荷"},
            "_raw.trader": {"content": "正常"}
        })
        .to_string();
        let v = build_sub_analysis_from_snapshot(Some(&snapshot), "600519");
        let obj = v.as_object().expect("应返回 JSON 对象");
        assert!(obj.get("").is_none(), "空 node_id 键必须被跳过");
        assert!(obj.contains_key("trader"), "正常节点不得受牵连");
    }

    #[test]
    fn oversized_raw_market_data_leaf_is_capped_end_to_end() {
        // 复现事故现场：raw-data.result 整块 8~12 万字符原始行情
        let snapshot = json!({
            "_raw.raw-data": {"result": "乙".repeat(100_000)}
        })
        .to_string();
        let v = build_sub_analysis_from_snapshot(Some(&snapshot), "000001");
        let result = v["raw-data"]["result"].as_str().expect("result 应保留为字符串");
        assert!(
            result.chars().count() <= REFLECTION_MEMORY_LEAF_CHAR_BUDGET + 64,
            "单叶超限未裁: {} 字符",
            result.chars().count()
        );
        assert!(result.contains("反思上下文裁剪"));
    }

    #[test]
    fn total_budget_evicts_fattest_nodes_but_keeps_conclusion() {
        // 6 个多叶大数据节点：每节点 25 个 3 万字符叶子，逐档压缩后仍远超节点预算，
        // 合计远超总预算 ⇒ 必须整体淘汰最肥者（全部非保护 ⇒ 走第一趟）。
        let mut root = Map::new();
        for i in 0..6 {
            let rows: Vec<serde_json::Value> =
                (0..25).map(|_| json!("丁".repeat(30_000))).collect();
            root.insert(format!("_raw.raw-data-{i}"), json!({"rows": rows}));
        }
        // 结论节点体积小，必须完整保留
        root.insert(
            "_raw.consensus".to_string(),
            json!({"content": "维持买入评级，目标价 1850，风险收益比合适。"}),
        );
        let snapshot = serde_json::Value::Object(root).to_string();

        let v = build_sub_analysis_from_snapshot(Some(&snapshot), "000001");
        let obj = v.as_object().expect("应返回 JSON 对象");

        let total: usize = obj.values().map(reflection_json_chars).sum();
        assert!(
            total <= REFLECTION_MEMORY_TOTAL_CHAR_BUDGET,
            "裁剪后总量 {total} 仍超总预算 {REFLECTION_MEMORY_TOTAL_CHAR_BUDGET}"
        );

        let evicted = obj
            .values()
            .filter(|node| node.get("_truncated").and_then(|x| x.as_str()).is_some())
            .count();
        assert!(evicted >= 1, "总预算超限时必须整体淘汰最肥节点");

        // 小体积的结论节点不在淘汰序列里
        assert_eq!(
            obj.get("consensus").and_then(|n| n.get("content")).and_then(|c| c.as_str()),
            Some("维持买入评级，目标价 1850，风险收益比合适。")
        );
    }

    #[test]
    fn legacy_snapshot_without_raw_returns_empty_object() {
        let snapshot = json!({"report.trader": "旧版报告文本"}).to_string();
        let v = build_sub_analysis_from_snapshot(Some(&snapshot), "600519");
        assert_eq!(v, json!({}));
    }

    #[test]
    fn protected_nodes_are_not_evicted_while_budget_can_meet_by_unprotected() {
        // P1-2：两个超肥非保护数据块（各自压缩后仍 ~4.4 万字符）+ 一个小的保护节点。
        // 预算 3.6 万：两数据块被整体淘汰即达标 ⇒ 保护节点必须原样完整保留。
        let fat = |leaf_char: &str| -> serde_json::Value {
            json!({"rows": (0..100).map(|_| json!(leaf_char.repeat(3_000))).collect::<Vec<_>>()})
        };
        let snapshot = json!({
            "_raw.raw-data-a": fat("甲"),
            "_raw.raw-data-b": fat("乙"),
            "_raw.data-quality": {"__untrusted": false, "note": "结论节点体积小"}
        })
        .to_string();
        let v = build_sub_analysis_from_snapshot(Some(&snapshot), "000001");
        let obj = v.as_object().expect("应返回 JSON 对象");

        assert!(obj["raw-data-a"].get("_truncated").is_some(), "非保护数据块必须先被整体淘汰");
        assert!(obj["raw-data-b"].get("_truncated").is_some(), "非保护数据块必须先被整体淘汰");
        assert!(
            obj["data-quality"].get("_truncated").is_none(),
            "预算可由非保护节点满足时，保护节点不得被淘汰"
        );
        assert_eq!(obj["data-quality"]["note"], json!("结论节点体积小"));

        let total: usize = obj.values().map(reflection_json_chars).sum();
        assert!(total <= REFLECTION_MEMORY_TOTAL_CHAR_BUDGET, "裁剪后总量 {total} 超预算");
    }

    #[test]
    fn evicted_protected_node_keeps_untrusted_sentinel() {
        // 保护节点自身超预算且无非保护者可垫背 ⇒ 兜底趟淘汰它，但 `__untrusted`
        // 哨兵必须留在占位里（comparator 的 u_* 映射在节点省略后仍要可判定）。
        let snapshot = json!({
            "_raw.data-quality": {
                "__untrusted": true,
                "rows": (0..100).map(|_| json!("丙".repeat(3_000))).collect::<Vec<_>>()
            }
        })
        .to_string();
        let v = build_sub_analysis_from_snapshot(Some(&snapshot), "000001");
        let obj = v.as_object().expect("应返回 JSON 对象");

        let placeholder = &obj["data-quality"];
        assert!(placeholder.get("_truncated").is_some(), "唯一超预算节点（保护）应被兜底淘汰");
        assert_eq!(
            placeholder["__untrusted"],
            json!(true),
            "占位必须保留原节点的 __untrusted 哨兵"
        );
        assert!(placeholder.get("rows").is_none(), "原始载荷必须被省略");
    }
}

#[cfg(test)]
mod failure_modes_column_tests {
    use super::*;

    #[test]
    fn column_is_null_when_template_omits_the_field() {
        assert_eq!(failure_modes_column_of(None), (None, Vec::new()));
    }

    #[test]
    fn column_holds_empty_array_when_every_token_is_outside_vocabulary() {
        let (col, rejected) = failure_modes_column_of(Some(r#"["sentiment_over_signal"]"#));
        assert_eq!(col.as_deref(), Some("[]"), "产出过字段就不能落 NULL，否则与未产出混态");
        assert_eq!(rejected, vec!["sentiment_over_signal".to_string()]);
    }

    #[test]
    fn column_accepts_all_three_observed_output_shapes() {
        for raw in [r#"["chase_big_up_bar"]"#, "chase_big_up_bar", r#""chase_big_up_bar""#] {
            let (col, rejected) = failure_modes_column_of(Some(raw));
            assert_eq!(col.as_deref(), Some(r#"["chase_big_up_bar"]"#), "形态未归一：{raw}");
            assert!(rejected.is_empty(), "不该有丢弃：{raw} -> {rejected:?}");
        }
    }

    #[test]
    fn column_keeps_legal_tokens_and_drops_only_illegal_ones() {
        let (col, rejected) =
            failure_modes_column_of(Some(r#"["other","invented_mode","chase_big_up_bar"]"#));
        assert_eq!(
            col.as_deref(),
            Some(r#"["chase_big_up_bar","other"]"#),
            "落库顺序按词表序，非法值不得进列"
        );
        assert_eq!(rejected, vec!["invented_mode".to_string()]);
    }
}
