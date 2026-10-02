// SPDX-License-Identifier: AGPL-3.0-only

//! stock-analysis 工作流生命周期钩子（业务侧实现，运行时注册进 WorkEngine）。
//!
//! 上游通用层只认协议（`axagent_harness::WorkflowLifecycleHook`）与模板
//! `hooks_config` 声明，零业务名硬编码；本模块提供 stock-analysis 模板的
//! 三个业务钩子实现：
//!
//! - `stock-analysis-precheck`（pre_exec）：数据质量预检。业务封装路径
//!   （工作区/批量，`input` 带 `analysis_id` 标记）已在同步阶段完成预检，
//!   钩子直接放行；对话直执行路径（input 为纯文本）执行预检，
//!   `Insufficient` → 返回 Err 阻断执行。顺带把 `stock_name`（quote.name）
//!   写回变量，供 enhance 钩子免二次行情请求。
//! - `stock-analysis-enhance`（pre_exec）：变量增强 —— 调用
//!   [`build_stock_analysis_variables`] 注入市场状态/模拟指标/持仓/行业/
//!   regime 偏向/历史教训/相似案例等全部业务变量。所有执行路径统一走此钩子
//!   （工作区路径原 spawn 内增强块已迁移至此）。
//! - `stock-analysis-persist`（post_exec）：结果持久化。业务封装路径已
//!   自行持久化（input 带 `analysis_id` 标记），钩子跳过；对话直执行路径
//!   由钩子创建 `stock_analyses` 记录（analysis_kind="chat"）+ 反思 pending 占位。
//!
//! 注册：启动期 `register_stock_analysis_hooks`（init/state.rs）。

use crate::commands::stock_workflow::core::{
    fetch_similar_cases, fetch_stock_lessons, record_lesson_applications,
};
use crate::commands::stock_workflow::decision::{
    QualityPrecheckResult, data_quality_precheck, extract_decision_fields,
    extract_decision_from_results_map, extract_decisions_by_horizon, extract_horizon_price_map,
    extract_horizon_source, extract_position_state, normalize_action_for_storage,
};
use axagent_astock_data::AStockClient;
use axagent_entities::stock_analyses;
use axagent_harness::workflow_types::Variable;
use axagent_harness::{HookExecContext, HookOutcome, Period, WorkflowLifecycleHook};
use axagent_rt_workflow::work_engine::WorkEngine;
use sea_orm::DatabaseConnection;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde_json::json;
use std::sync::Arc;

// ── 公共辅助 ──────────────────────────────────────────────────────

// ── 跨系统互证（智选推荐 vs 工作流决策）────────────────────────────

/// 查询某股票近 `max_age_days` 天内的智选推荐（趋势智选面板同源的
/// serenity / bottleneck 风格记录），返回融合先验 JSON（camelCase）。
///
/// 两处消费：
/// 1. [`build_stock_analysis_variables`] 注入 `reco_prior` 变量（黑板可见，
///    仅供 LLM 上下文先验 —— report B2 已核：全仓 rhai 无 `reco_prior` 字面量消费）
/// 2. 决策持久化时构建 `crossCheck` 字段（跨系统互证，core.rs / decision.rs 独立调用）
///
/// ⚠ 本函数**只**服务上述两处「展示 / 互证」用途，**不得**被用来给决策链喂因子 ——
/// 理由见 `build_stock_analysis_variables` 里 `serenity_context` 段的注释（去重裁定）。
pub(crate) async fn fetch_reco_prior(
    db: &DatabaseConnection,
    stock_code: &str,
    max_age_days: i64,
) -> Option<serde_json::Value> {
    use axagent_entities::reco_picks;
    use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

    // created_at 是 ISO 8601 字符串列（"%Y-%m-%dT%H:%M:%S%.3f"），字典序即时间序
    let cutoff = (chrono::Local::now() - chrono::Duration::days(max_age_days))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let pick = match reco_picks::Entity::find()
        .filter(reco_picks::Column::StockCode.eq(stock_code))
        .filter(reco_picks::Column::Style.is_in(["serenity", "bottleneck"]))
        .filter(reco_picks::Column::CreatedAt.gte(cutoff))
        .order_by_desc(reco_picks::Column::CreatedAt)
        .one(db)
        .await
    {
        Ok(p) => p?,
        Err(e) => {
            tracing::warn!("[reco_prior] 查询智选推荐失败 ({}): {e}", stock_code);
            return None;
        },
    };
    let pick_data: serde_json::Value = pick
        .pick_data
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let seed: serde_json::Value = pick
        .seed_pool_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let catalysts: Vec<serde_json::Value> = seed["catalysts"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .take(3)
                .map(|c| {
                    json!({
                        "description": c["description"].as_str().unwrap_or(""),
                        "timeframe": c["expected_timeframe"].as_str().unwrap_or(""),
                        "confidence": c["confidence"].as_f64().unwrap_or(0.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(json!({
        "recoConfidence": pick.confidence,
        "recoStyle": pick.style,
        "recoStrategyType": pick_data["strategy_type"].as_str().unwrap_or("bottleneck"),
        "recoPeriod": pick.period,
        "recoPositionPct": pick_data["positionPct"].as_f64().unwrap_or(0.0),
        "recoHoldingDays": pick_data["holdingDays"].as_i64().unwrap_or(20),
        "recoPrice": pick_data["price"].as_f64().unwrap_or(0.0),
        "recoGeneratedAt": pick.generated_at,
        "attentionHeat": seed["attention_metrics"]["search_heat"].as_str().unwrap_or(""),
        "catalysts": catalysts,
    }))
}

/// 分歧归因：把「工作流为什么给不出利好」拆成**可点名的判据**。
///
/// 为什么需要它（2026-10-02）：分歧报告此前只并排两列数字（智选 78 / 分析 观望 0%），
/// 用户读到的是「两个系统打架」而不是「谁把结论压下去的」。实测 002812：原始后验
/// 49.3% 落在持有带，风险门槛压成生效后验 41.3% ⇒ 跨过持有线落到观望 —— 这条因果
/// 全程躺在决策 JSON 里，报告却没说。
///
/// 纪律与 [`inject_reco_crosscheck`] 一致：**只读**决策链已算出的字段，不另起一套计算、
/// 不硬编码阈值（阈值取本次决策实际生效的 `effective_params`）；字段缺失 ⇒ 不产该条，
/// 宁缺毋滥。产出一律结构化码 + 数值，叙事交前端 i18n。
fn divergence_attribution(decision: &serde_json::Value) -> serde_json::Value {
    let mut drivers: Vec<&'static str> = Vec::new();
    let mut out = serde_json::Map::new();

    // ── 1) 贡献最负的至多三条证据腿（|sigma × weight| 降序，sigma 与权重都是决策实际用的那份）──
    let legs: Vec<serde_json::Value> = decision
        .pointer("/evidence/factors")
        .and_then(serde_json::Value::as_object)
        .map(|factors| {
            let mut neg: Vec<(f64, serde_json::Value)> = factors
                .values()
                .filter_map(|f| {
                    let sigma = f.get("sigma")?.as_f64()?;
                    let weight = f.get("weight")?.as_f64()?;
                    if sigma < 0.0 && weight > 0.0 {
                        Some((
                            sigma * weight,
                            json!({ "name": f.get("name")?, "sigma": sigma, "weight": weight }),
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            neg.sort_by(|a, b| a.0.total_cmp(&b.0));
            neg.into_iter().take(3).map(|(_, v)| v).collect()
        })
        .unwrap_or_default();
    if !legs.is_empty() {
        drivers.push("negative_legs");
        out.insert("legs".into(), serde_json::Value::Array(legs));
    }

    // ── 2) 风险门槛是否**改变了档位** ──
    // `posteriorRaw` = 原始后验，`posterior` = 生效后验（含 risk_bias），两者同为 ×100。
    let thr = |k: &str| {
        decision
            .pointer(&format!("/effective_params/action_{k}_threshold"))
            .and_then(serde_json::Value::as_f64)
    };
    let (buy, increase, hold, watch, reduce) =
        (thr("buy"), thr("increase"), thr("hold"), thr("watch"), thr("reduce"));
    if let (Some((raw, eff)), (Some(buy), Some(inc), Some(hold), Some(watch), Some(reduce))) = (
        decision
            .get("posteriorRaw")
            .and_then(serde_json::Value::as_f64)
            .zip(decision.get("posterior").and_then(serde_json::Value::as_f64)),
        (buy, increase, hold, watch, reduce),
    ) {
        // 越高越看多：0 买入 … 5 卖出（与本文件既有 action 阶梯同序）
        let tier_of = |p: f64| -> u8 {
            if p >= buy {
                0
            } else if p >= inc {
                1
            } else if p >= hold {
                2
            } else if p >= watch {
                3
            } else if p >= reduce {
                4
            } else {
                5
            }
        };
        let (raw_t, eff_t, hold_x100) = (tier_of(raw / 100.0), tier_of(eff / 100.0), hold * 100.0);
        out.insert("posteriorRaw".into(), json!(raw));
        out.insert("posteriorEffective".into(), json!(eff));
        out.insert("holdThreshold".into(), json!(hold_x100));
        // 门槛确实压低后验 **且** 压到换了档 ⇒ 才归因给风险门槛；同档内的下调不构成分歧主因
        if raw > eff && raw_t < eff_t {
            drivers.push("risk_gate_downgrade");
        } else if eff_t >= 3 {
            // 已经在观望/减持/卖出带：报「低于持有线」，这是「为什么不是利好」的直接答
            drivers.push("below_hold_threshold");
        }
    }

    // ── 3) 估值腿被整条剔除（DCF 不适用）⇒ 方向档少一条腿的证据 ──
    if decision
        .pointer("/valuationApplicability/dcfApplicable")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        drivers.push("dcf_leg_excluded");
    }

    // ── 4) 数据缺口（证据不完整，与「判据为负」是两件事，必须分列）──
    if decision
        .pointer("/data_gaps")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|v| !v.is_empty())
    {
        drivers.push("data_gap");
    }

    out.insert(
        "drivers".into(),
        serde_json::Value::Array(drivers.iter().map(|d| json!(*d)).collect()),
    );
    serde_json::Value::Object(out)
}

/// 将智选推荐先验与工作流决策做跨系统互证，把 `crossCheck` 就地写入决策 JSON。
///
/// 分歧判定：智选 confidence≥60 且建议仓位>0，而工作流 action=观望/卖出 或仓位≤0
/// ——此时前端展示「智选推荐 vs 工作流否决」分歧报告。
/// 本函数纯结构化注入、不做叙述文本（叙事由前端 i18n 渲染）。
pub(crate) fn inject_reco_crosscheck(
    decision_value: &mut serde_json::Value,
    reco_prior: &serde_json::Value,
) {
    // 归因必须在取 `as_object_mut` 之前算：它只读，读的是决策链刚写完的那份字段。
    let divergence = divergence_attribution(decision_value);
    let Some(obj) = decision_value.as_object_mut() else {
        return;
    };
    let decision_action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let decision_pos = obj.get("positionPct").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // V76(2026-09-14): 持仓状态轴一并写入 crossCheck 快照 —— 展示层据
    // (action, positionState) 两轴派生「持有/观望」，不再依赖 pct 反推。
    let decision_state = obj.get("positionState").and_then(|v| v.as_str()).map(str::to_string);
    let reco_conf = reco_prior["recoConfidence"].as_f64().unwrap_or(0.0);
    let reco_pos = reco_prior["recoPositionPct"].as_f64().unwrap_or(0.0);
    // P1-6(2026-09-14): 否决档判定改走统一归一化。
    // 原实现硬编码中文 `matches!(decision_action, "观望" | "卖出")`：action 一旦是
    // 英文 `WAIT`/`SELL`（或 dashboard 值域的「强烈卖出」），此处判不出否决，
    // 分歧报告静默不生成。判定集合与旧实现严格等价 —— 只含「观望 / 卖出」两档，
    // 不含「减持」（是否纳入属产品语义变更，不在本次收敛范围）。
    use axagent_analysis_engine::decision_action::{ActionKind, normalize_action};
    let workflow_vetoed =
        matches!(normalize_action(&decision_action), Some(ActionKind::Wait | ActionKind::Sell));
    let divergent = reco_conf >= 60.0 && reco_pos > 0.0 && (decision_pos <= 0.0 || workflow_vetoed);
    obj.insert(
        "crossCheck".into(),
        json!({
            "recoConfidence": reco_prior["recoConfidence"],
            "recoStyle": reco_prior["recoStyle"],
            "recoStrategyType": reco_prior["recoStrategyType"],
            "recoPeriod": reco_prior["recoPeriod"],
            "recoPositionPct": reco_prior["recoPositionPct"],
            "recoHoldingDays": reco_prior["recoHoldingDays"],
            "recoPrice": reco_prior["recoPrice"],
            "recoGeneratedAt": reco_prior["recoGeneratedAt"],
            "decisionGeneratedAt": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            "attentionHeat": reco_prior["attentionHeat"],
            "catalysts": reco_prior["catalysts"],
            "decisionAction": decision_action,
            "decisionPositionState": decision_state,
            "decisionPositionPct": decision_pos,
            "divergent": divergent,
            "divergence": divergence,
        }),
    );
}

/// `input` 是否带业务封装路径标记（Object 且含 `analysis_id` 字段）。
/// 工作区 / 批量 / 重跑入口在 `opts.input` 中写入该字段；对话直执行路径
/// 的 input 是纯文本字符串，无此标记。
fn input_has_analysis_id(input: &Option<serde_json::Value>) -> bool {
    matches!(input, Some(serde_json::Value::Object(map)) if map.contains_key("analysis_id"))
}

/// 从 `input` Object 中取字符串字段（非 Object 或缺字段返回 None）。
fn input_str<'a>(input: &'a Option<serde_json::Value>, key: &str) -> Option<&'a str> {
    input.as_ref().and_then(|v| v.as_object()).and_then(|m| m.get(key)).and_then(|v| v.as_str())
}

/// 从变量列表中取字符串变量值。
fn var_str<'a>(vars: &'a [Variable], name: &str) -> Option<&'a str> {
    vars.iter().find(|v| v.name == name).and_then(|v| v.value.as_str())
}

// ── 共享：变量增强（工作区路径与 enhance 钩子共用） ────────────────

/// 构建 stock-analysis 工作流的完整业务变量集。
///
/// 工作区路径（`run_stock_workflow_inner`）与 `stock-analysis-enhance` 钩子
/// 共用同一实现，保证「判据用哪套、执行就用哪套」，两条路径变量零漂移。
///
/// `base_vars`：已有变量（工作区路径传模板变量；钩子传 ctx.variables）。
/// `analysis_id`：Some 时写 `lesson_applications`（业务路径）；
/// 对话直执行路径传 None（记录尚未创建，post_exec 才建）。
/// 取该股**最近一条已产出主档**的分析的 `decision_time_horizon`，作为对话直执行路径
/// 注入反思教训时的「本轮周期」代理。
///
/// 返回 `None` 的两种情形（都按「不按档过滤」处理并 WARN，不冒充某档）：
///   ① 该股从未有过带主档的分析（首次分析）；
///   ② DB 查询失败（基础设施问题，不该升级为决策错误）。
///
/// 为什么不能直接用公式主档：v2 的主档由 `portfolio-mgr.rhai` 按后验阈值定档，
/// 而本函数所在的增强钩子在 `portfolio-mgr` **之前**执行 —— 那一刻还没有后验。
async fn latest_known_horizon(stock_code: &str, db: &DatabaseConnection) -> Option<String> {
    // 查询经 dao 下沉（`axagent_dao::repo::stock_lesson_queries`），命令层不得直连
    //（分层门禁规则 1）。`Err`（DB 故障）与 `Ok(None)`（无既有主档）都按「不按档过滤」
    // 处理并 WARN，不冒充某档 —— 与下沉前逐位一致。
    let h = axagent_dao::repo::stock_lesson_queries::latest_analysis_horizon(db, stock_code)
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    if h.is_none() {
        tracing::warn!(
            "[stock-analysis] {stock_code} 无既有主档可代理 ⇒ 本轮教训注入不按档过滤（跨档混合，非事实陈述）"
        );
    }
    h
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_stock_analysis_variables(
    db: &DatabaseConnection,
    client: &AStockClient,
    stock_code: &str,
    stock_name: &str,
    base_vars: Vec<Variable>,
    screening_source: Option<&str>,
    as_of_date: Option<&str>,
    analysis_id: Option<&str>,
) -> Vec<Variable> {
    let mut merged_vars = base_vars;
    // 基础变量覆盖写（stock_code/stock_name 以本次执行为准）
    for (name, value, desc) in [
        ("stock_code", json!(stock_code), "当前分析的股票代码"),
        ("stock_name", json!(stock_name), "当前分析的股票名称"),
    ] {
        if let Some(existing) = merged_vars.iter_mut().find(|v| v.name == name) {
            existing.value = value;
        } else {
            merged_vars.push(Variable {
                name: name.into(),
                var_type: "string".into(),
                value,
                description: Some(desc.into()),
                is_secret: false,
            });
        }
    }
    // ── 周期常量表注入（horizon_consts_json：每档 {days, mult}）──
    // `portfolio-mgr.rhai` 的 `horizon_const` / `days_for` / 周期仓位乘数自 2026-09-28 起
    // 消费本变量，脚本侧不再手抄天数或乘数（唯一权威源
    // `axagent_harness::holding_period::Period::decision_consts_map`）。
    // ⚠ 必须**无条件恒注入**（同 `stock_lessons` 的先例）：缺失会让脚本直接 throw，
    //   这是有意选择 —— 未知周期静默兜 5 天会让中/长线记录带着错档的期望持有期入库。
    {
        let consts = Period::decision_consts_map();
        if let Some(existing) = merged_vars.iter_mut().find(|v| v.name == "horizon_consts_json") {
            existing.value = consts;
        } else {
            merged_vars.push(Variable {
                name: "horizon_consts_json".into(),
                var_type: "object".into(),
                value: consts,
                description: Some(
                    "周期常量表（每档 {days, mult}，权威源 Period::decision_consts_map）".into(),
                ),
                is_secret: false,
            });
        }
    }
    // ── 逐档 × 逐腿证据乘数表注入（horizon_leg_weights_json）──
    // 四周期科学化 Phase A：四档要「同一批证据、逐档重新加权」，故把按分析师索引的
    // 周期权重表（`evidence_weight::get_horizon_base_weights`）经桥表
    // `DECISION_LEG_ANALYST` 投影到决策脚本的因子腿上，脚本侧禁止再手抄任何倍数。
    // 恒注入：缺失 ⇒ 脚本按「该档乘数全 1.0」退化并显式标注（证据腿不再逐档不同是可观测的
    // 降级，不是错档），故不像 horizon_consts_json 那样 throw。
    {
        let leg_weights = axagent_analysis_engine::evidence_weight::horizon_leg_multipliers();
        if let Some(existing) =
            merged_vars.iter_mut().find(|v| v.name == "horizon_leg_weights_json")
        {
            existing.value = leg_weights;
        } else {
            merged_vars.push(Variable {
                name: "horizon_leg_weights_json".into(),
                var_type: "object".into(),
                value: leg_weights,
                description: Some(
                    "逐档×逐腿证据乘数表（派生自 evidence_weight 周期权重表，权威源 DECISION_LEG_ANALYST 桥）"
                        .into(),
                ),
                is_secret: false,
            });
        }
    }
    // ── 逐档先验注入（horizon_prior_json，四周期科学化 Phase C）──
    // 四档不再共用同一个 `prior`：先由反思统计算出**每档自己的方向命中率**，再与全档
    // 合并基准做经验贝叶斯收缩 `prior_h = (n·p_h + κ·p_pool)/(n+κ)`。
    // κ = 模板变量 `horizon_prior_kappa`（进设置面板、可被反思建议覆盖）。
    // 统计取数与 `reflection_stats` 命令**共用** `build_hitrate_stats`（禁区 12：不重复实现）。
    // 取数失败 ⇒ 注入 null，脚本按主链共用 prior 退化并在每档 `priorSource` 标注（不静默）。
    {
        use axagent_analysis_engine::horizon_prior::DEFAULT_KAPPA;
        let kappa = merged_vars
            .iter()
            .find(|v| v.name == "horizon_prior_kappa")
            .and_then(|v| v.value.as_f64())
            .unwrap_or(DEFAULT_KAPPA);
        let prior_map = match axagent_analysis_engine::reflection_stats::build_hitrate_stats(db)
            .await
        {
            Ok(stats) => axagent_analysis_engine::horizon_prior::horizon_prior_map(&stats, kappa),
            Err(e) => {
                tracing::warn!("[stock_workflow] 逐档先验统计取数失败 ⇒ 四档退回共用 prior: {e}");
                serde_json::Value::Null
            },
        };
        if let Some(existing) = merged_vars.iter_mut().find(|v| v.name == "horizon_prior_json") {
            existing.value = prior_map;
        } else {
            merged_vars.push(Variable {
                name: "horizon_prior_json".into(),
                var_type: "object".into(),
                value: prior_map,
                description: Some(
                    "逐档收缩先验 {档:{prior,samples,source}}（权威源 reflection_stats 按档命中率 + κ 收缩）"
                        .into(),
                ),
                is_secret: false,
            });
        }
    }
    if let Some(d) = as_of_date {
        merged_vars.push(Variable {
            name: "as_of_date".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(d.to_string()),
            description: Some("时间旅行模式截止日 (YYYY-MM-DD)；live 模式为空".into()),
            is_secret: false,
        });
    }

    // V53: 调用方指定 screening_source 时覆盖模板默认值
    // 使瓶颈掘金→股票分析的上下文可传递到 portfolio-mgr
    if let Some(source) = screening_source {
        if !source.is_empty() {
            if let Some(existing) = merged_vars.iter_mut().find(|mv| mv.name == "screening_source")
            {
                existing.value = serde_json::Value::String(source.to_string());
            } else {
                merged_vars.push(Variable {
                    name: "screening_source".into(),
                    var_type: "string".into(),
                    value: serde_json::Value::String(source.to_string()),
                    description: Some("筛选来源标记".into()),
                    is_secret: false,
                });
            }
        }
    }
    // ── f13 瓶颈因子的输入：**刻意不注入**（2026-10-01 去重裁定）──
    //
    // `serenity_context` 是 `portfolio-mgr.rhai` 的 f13 因子唯一输入。它**不再注入**，
    // 理由是同一要素不得重复产生决策影响：
    //
    //   · `DECISION_LEG_ANALYST` 把 f13 明确映射到 **`a-sector`**
    //     （`crates/analysis-engine/src/evidence_weight.rs`）；
    //   · 而 `a-sector`（"行业景气度与轮动分析"，`seed_stock_analysis.rs` 的 analysts
    //     数组第 9 项）**本来就在股票分析链里跑**，其观点经「多空辩论 →
    //     debate-convergence.consensus_score → f2（权重 0.25）」进入决策；
    //   · 趋势智选引以为据的政策 / 消息 / 产业链瓶颈 / 催化剂叙事，在分析链里
    //     分别由 `a-policy` / `a-news` / `a-sector` / `a-catalyst` 覆盖。
    //   ⇒ 再把趋势智选的 `serenity_score` 接成 f13，等于把同一维度**第二次**计入后验，
    //     正是本仓 P1-1「因子协方差衰减」要消除的重复计数形态
    //     （同 f3↔f11 公告重叠降权 35%、f1↔f9 趋势-资金共振降权 25%）。
    //
    // 反面实证（为何曾有「断链」一说）：f13 权重实测恒为 0（近 20 条分析、10 只股票
    // 100% 复现）—— 但那**不是**缺陷，而是隔离起效。曾以「数据到了门口没人开门」
    // 为由把它接上（v52 / `AUDIT-codebase-review-roadmap-2026-09-19.md` A1），
    // 该处置与去重纪律冲突，2026-10-01 用户裁定以**去重**为准。
    //
    // ⚠ 因此：**不要**为了「让荐股理由进入决策」而重新注入本变量。两个子系统的观点差异
    //   应当通过修**错的那一方**来收敛（实证：趋势智选侧 `consensus_gap` 用单篇研报
    //   目标价冒充共识 ⇒ 误判「明显低估」，已在 `astock-data` 的 `compute_attention_score`
    //   中修正为「近 90 天中位数 + 最小样本数」），而不是把结论搬进对方的算式里强行对齐。
    //
    // `screening_source` 变量本身仍在上面注入（保留显式来源声明的语义）。

    // ── 跨系统互证：注入近 14 天智选推荐先验（reco_prior）──
    // 无论 screening_source 是什么，只要该股近期被趋势智选命中过就注入。
    // 决策持久化时据此构建 crossCheck（互证字段，core.rs / decision.rs 独立调用
    // fetch_reco_prior 消费 —— 见 report 主线 B2）。
    // 注入黑板的 reco_prior 键当前**仅供 LLM 上下文先验**，无 rhai 因子消费
    // （report B2 已核：全仓 rhai 对 `reco_prior` 无字面量读取）；保留作动态扩展
    // 入口，勿删（删它会砍掉跨系统互证能力的输入路径）。
    if let Some(prior) = fetch_reco_prior(db, stock_code, 14).await {
        tracing::info!(
            "[stock-analysis] 注入 reco_prior: code={} conf={} strategy={}",
            stock_code,
            prior["recoConfidence"].as_f64().unwrap_or(0.0),
            prior["recoStrategyType"].as_str().unwrap_or(""),
        );
        merged_vars.push(Variable {
            name: "reco_prior".into(),
            var_type: "object".into(),
            value: prior,
            description: Some(
                "近 14 天智选推荐先验（confidence/strategyType/catalysts 等）；\n\
                 - 决策持久化 → crossCheck（跨系统互证，真实消费方）；\n\
                 - 黑板键 → 仅作 LLM 上下文先验，无 rhai 因子引用（report B2）"
                    .into(),
            ),
            is_secret: false,
        });
    }

    // 注入相似历史决策案例（失败案例优先，最多 5 条）
    if let Some(cases) = fetch_similar_cases(stock_code, db).await {
        merged_vars.push(Variable {
            name: "similar_cases".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(cases),
            description: Some("相似历史决策（失败案例，供避免重复错误）".into()),
            is_secret: false,
        });
    }

    // 注入市场状态（沪深300判断牛/熊/震荡）
    // （原工作区路径在 spawn 前计算，现统一在执行前此处计算）
    let regime_value = client
        .get_klines("000300", "daily", 60)
        .await
        .ok()
        .and_then(|klines| {
            if klines.is_empty() {
                return None;
            }
            let r = axagent_analysis_engine::market_regime::classify_regime(&klines);
            Some(json!({
                "regime": r.regime,
                "confidence": r.confidence,
                "volatility": r.volatility,
                "description": r.description,
            }))
        })
        .unwrap_or_else(|| {
            json!({
                "regime": "unknown",
                "confidence": null,
                "volatility": null,
                "description": "⚠️ 市场状态数据暂不可用（沪深300 K线拉取失败），请勿据此做多空判断，基于个股自身数据完成分析"
            })
        });
    merged_vars.push(Variable {
        name: "market_regime".into(),
        var_type: "object".into(),
        value: regime_value.clone(),
        description: Some("当前市场状态(bull/bear/sideways)+波动率+描述".into()),
        is_secret: false,
    });

    // 注入市场模拟指标（DES 轻量版，从个股 K 线估算，无需额外 API 调用）
    let sim_metrics = client
        .get_klines(stock_code, "daily", 30)
        .await
        .ok()
        .and_then(|klines| {
            if klines.len() < 5 {
                return None;
            }
            // 计算日收益率序列 → 年化波动率 → sim_stability
            let mut returns = Vec::with_capacity(klines.len() - 1);
            let mut total_volume: f64 = 0.0;
            for pair in klines.windows(2) {
                let prev_close = pair[0].close;
                let cur = &pair[1];
                if prev_close > 0.0 {
                    returns.push((cur.close - prev_close) / prev_close);
                }
                total_volume += cur.volume;
            }
            let avg_price = klines.last()?.close;
            let n = returns.len() as f64;
            if n < 3.0 {
                return None;
            }
            let mean = returns.iter().sum::<f64>() / n;
            let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
            let annual_vol = variance.sqrt() * (252.0_f64).sqrt();
            // ── P1-E 修复（2026-09-11）──
            // ① sim_stability 括号错位（死门）：
            //    原式 `1.0 / (1.0 + annual_vol * 3.0).clamp(0.3, 1.0)`
            //    方法调用优先级高于除法，实际是对内层 `1.0 + av*3.0` 做 clamp。
            //    av > 0 时 1+3av > 1.0 必被夹到上界 1.0 → 倒数恒 = 1.0。
            //    实测 12 条新模板运行 sim_stability 全部 = 1，
            //    S-501（sim_stability < 0.3 → 强制观望）为数学上不可能触发的死门。
            //    修正：钳位作用在「结果」上，并放宽下界到 0.05 保留动态范围
            //    （av=0.4→0.45 / av=0.8→0.29 / av=1.5→0.18 / av=2.5→0.12）。
            //    配套：rhai 侧 S-501 阈值同步 0.3 → 0.15（对应年化波动 >210%）。
            let sim_stability = (1.0 / (1.0 + annual_vol * 3.0)).clamp(0.05, 1.0);
            let avg_daily_volume = total_volume / (klines.len() as f64);
            let daily_volume_val = avg_daily_volume * avg_price;
            let sim_liquidity = (daily_volume_val / 100_000_000.0 * 0.7 + 0.1).clamp(0.1, 0.95);
            // ② sim_impact 量级错误导致饱和（S-503 退化为无条件减仓）：
            //    原式 `(annual_vol*100*(1-sim_liquidity)*50+5).clamp(1,200)`
            //    中 sim_liquidity 由上式得，日成交额 ≥1.21 亿即顶格 0.95，
            //    故 (1-liq) 恒 0.05，叠加常数 50 → av ≥ 0.4 就直接触顶 200。
            //    实测：sim_impact = 162.9 / 200（顶格），S-503（>150bps 仓位减半）
            //    触发率 92.5%，等价于「出仓位就砍半」的死规则。
            //    修正：流动性折价改用「日成交额绝对水平」（不经过已饱和的 liq 通道），
            //    量级重标定到 A 股真实区间（日成交额 5 亿→折价 0.1，2 亿→0.6，
            //    1 亿→0.8，≤0.25 亿→0.95）：
            //      av=0.35, 成交 5 亿 → 8.5bps ｜ av=0.90, 成交 5 亿 → 14bps
            //      av=0.60, 成交 1 亿 → 53bps ｜ av=0.90, 成交 0.5 亿 → 86bps
            //    配套：rhai 侧 S-503 阈值同步 150 → 80bps。标定原则：阈值须落在
            //    「高波动但流动性充足」（应放行，约 14bps）与「高波动+流动性不足」
            //    （该拦截，约 86bps）之间。使该规则从「出仓位就砍半」回归
            //    「惩罚真正的流动性风险」。
            //    注：本式为「轻量代理」，未含下单量/ADV 参与率，绝对水平仍属定性标定，
            //    需后续用真实成交数据回测校准。
            let turnover_yi = daily_volume_val / 100_000_000.0; // 日成交额（亿元）
            let liq_discount = (1.0 - (turnover_yi / 5.0).clamp(0.05, 0.9)).clamp(0.1, 0.95);
            let sim_impact = (annual_vol * 100.0 * liq_discount + 5.0).clamp(1.0, 500.0);
            Some(json!({
                "sim_stability": (sim_stability * 100.0).round() / 100.0,
                "sim_liquidity": (sim_liquidity * 100.0).round() / 10.0 / 10.0,
                "sim_impact": (sim_impact * 10.0).round() / 10.0,
                "sim_regime": if annual_vol > 0.4 { "high_vol" }
                    else if annual_vol > 0.2 { "normal" } else { "low_vol" },
            }))
        })
        .unwrap_or_else(|| {
            json!({
                "sim_stability": serde_json::Value::Null,
                "sim_liquidity": serde_json::Value::Null,
                "sim_impact": serde_json::Value::Null,
                "sim_regime": serde_json::Value::Null,
            })
        });
    if let Some(stab) = sim_metrics["sim_stability"].as_f64() {
        merged_vars.push(Variable {
            name: "sim_stability".into(),
            var_type: "number".into(),
            value: json!(stab),
            description: Some("市场模拟：价格稳定性(0~1, 越高越稳定)".into()),
            is_secret: false,
        });
    }
    if let Some(liq) = sim_metrics["sim_liquidity"].as_f64() {
        merged_vars.push(Variable {
            name: "sim_liquidity".into(),
            var_type: "number".into(),
            value: json!(liq),
            description: Some("市场模拟：流动性深度(0~1, 越高流动性越好)".into()),
            is_secret: false,
        });
    }
    if let Some(impact) = sim_metrics["sim_impact"].as_f64() {
        merged_vars.push(Variable {
            name: "sim_impact".into(),
            var_type: "number".into(),
            value: json!(impact),
            description: Some("市场模拟：大单冲击成本(bps)".into()),
            is_secret: false,
        });
    }

    // ── P1-E13: 注入组合风控门所需的持仓/现金/行业变量 ──
    // portfolio-risk-gate CodeNode 读取这些变量做组合层约束检查
    {
        use axagent_entities::portfolio_holdings;
        // 查询当前持仓，用 avg_cost 估值 market_value（避免逐个调 get_quote 造成延迟）
        let holdings = portfolio_holdings::Entity::find().all(db).await.unwrap_or_default();
        let holdings_json: Vec<serde_json::Value> = holdings
            .iter()
            .map(|h| {
                let mv = h.shares * h.avg_cost;
                json!({
                    "stockCode": h.stock_code,
                    "stockName": h.stock_name,
                    "totalShares": h.shares as i32,
                    "avgCost": h.avg_cost,
                    "currentPrice": h.avg_cost,
                    "marketValue": mv,
                    "unrealizedPnl": 0.0,
                    "unrealizedPnlPct": 0.0,
                    "totalRealizedPnl": 0.0,
                    "sectorName": null,
                })
            })
            .collect();
        let holdings_json_str =
            serde_json::to_string(&holdings_json).unwrap_or_else(|_| "[]".into());
        merged_vars.push(Variable {
            name: "holdings_json".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(holdings_json_str),
            description: Some("当前持仓 JSON 数组（供组合风控门检查仓位/行业暴露）".into()),
            is_secret: false,
        });
        // portfolio_cash（暂时注入 0.0，后续可扩展为从账户设置读取）
        merged_vars.push(Variable {
            name: "portfolio_cash".into(),
            var_type: "number".into(),
            value: json!(0.0),
            description: Some("可用现金（供组合风控门计算组合总价值）".into()),
            is_secret: false,
        });
        // stock_sector（当前股票的申万一级行业，供行业暴露检查）
        let stock_sector = client
            .get_sector_info(stock_code)
            .await
            .ok()
            .flatten()
            .map(|s| s.sector_name)
            .unwrap_or_default();
        if !stock_sector.is_empty() {
            merged_vars.push(Variable {
                name: "stock_sector".into(),
                var_type: "string".into(),
                value: serde_json::Value::String(stock_sector.clone()),
                description: Some("当前股票的申万一级行业（供组合风控门检查行业暴露）".into()),
                is_secret: false,
            });
        }
        tracing::info!(
            "[stock-analysis] P1-E13 注入: holdings={}条, sector={}",
            holdings.len(),
            if stock_sector.is_empty() {
                "(空)"
            } else {
                &stock_sector
            }
        );
    }

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
    merged_vars.push(Variable {
        name: "regime_prompt_bias".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(regime_prompt_bias.to_string()),
        description: Some("按当前市场状态(regime)匹配的分析偏向指令".into()),
        is_secret: false,
    });
    merged_vars.push(Variable {
        name: "regime_triggered_rules".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(regime_triggered_rules.to_string()),
        description: Some("当前市场状态触发的分析规则清单".into()),
        is_secret: false,
    });

    // 注入历史反思教训（stock_reflections 表最近的结构化反思结果）
    // 必须始终注入，即使为空，否则 value-investor/research-mgr/trader 等节点
    // 的 input_mapping 引用 {{stock_lessons}} 会报 VARIABLE_NOT_FOUND。
    // 〇-B v2 第 4 条 / PLAN 断点⑤：教训**按档隔离**后再注入。
    // 对话直执行路径的增强钩子跑在 `portfolio-mgr` **之前**，本轮主档（公式定档）
    // 此刻还不存在 ⇒ 用「该股最近一条已产出主档」作同档代理；取不到则不过滤并 WARN，
    // 不静默冒充某档（缺数 ≠ 默认）。
    let lesson_horizon = latest_known_horizon(stock_code, db).await;
    let (lessons_str, applied_lesson_ids) =
        fetch_stock_lessons(stock_code, db, lesson_horizon.as_deref()).await;
    let default_lessons = "（暂无历史反思）".to_string();
    let lessons_val = lessons_str.unwrap_or_else(|| default_lessons.clone());
    merged_vars.push(Variable {
        name: "stock_lessons".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(lessons_val.clone()),
        description: Some("该股历史反思教训（错因/被忽视信号/改进建议）".into()),
        is_secret: false,
    });
    // P2-F15: 批量写入 lesson_applications（失败不阻塞主流程）。
    // 仅业务封装路径（有真实 analysis_id）写入；对话直执行路径记录尚未创建。
    if let Some(aid) = analysis_id {
        if !applied_lesson_ids.is_empty() {
            record_lesson_applications(db, &applied_lesson_ids, aid, stock_code).await;
        }
    }
    // P1: 注入 per-role 经验和教训到辩论角色 prompt
    merged_vars.push(Variable {
        name: "bull_lessons".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(format!(
            "你作为多方研究员的过往经验教训：{}",
            lessons_val
        )),
        description: Some("该股多方视角的历史反思教训".into()),
        is_secret: false,
    });
    merged_vars.push(Variable {
        name: "bear_lessons".into(),
        var_type: "string".into(),
        value: serde_json::Value::String(format!(
            "你作为空方研究员的过往经验教训：{}",
            lessons_val
        )),
        description: Some("该股空方视角的历史反思教训".into()),
        is_secret: false,
    });

    merged_vars
}

// ── 钩子 1：数据质量预检（pre_exec） ──────────────────────────────

pub(crate) struct StockAnalysisPrecheckHook {
    client: Arc<AStockClient>,
}

impl StockAnalysisPrecheckHook {
    pub(crate) fn new(client: Arc<AStockClient>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl WorkflowLifecycleHook for StockAnalysisPrecheckHook {
    fn name(&self) -> &str {
        "stock-analysis-precheck"
    }

    async fn pre_exec(&self, ctx: HookExecContext) -> Result<Vec<Variable>, String> {
        // 业务封装路径已在同步阶段完成预检（且承担结构化 skip 报告），
        // input 带 analysis_id 标记 → 直接放行，避免双重数据请求。
        if input_has_analysis_id(&ctx.input) {
            return Ok(ctx.variables);
        }
        // 对话直执行路径：从变量取 stock_code（workflow_execute 的
        // extract_params_from_text 已注入），缺失则阻断并说明。
        let Some(stock_code) =
            var_str(&ctx.variables, "stock_code").map(str::to_string).or_else(|| {
                ctx.input.as_ref().and_then(|v| v.as_str()).and_then(|text| {
                    // 纯文本输入兜底：提取首个 6 位数字代码
                    text.split(|c: char| !c.is_ascii_digit())
                        .find(|s| s.len() == 6)
                        .map(str::to_string)
                })
            })
        else {
            return Err("stock_code 变量缺失，无法执行数据质量预检".into());
        };
        let quote =
            self.client.get_quote(&stock_code).await.map_err(|e| format!("行情获取失败: {e}"))?;
        match data_quality_precheck(&self.client, &stock_code, &quote).await {
            QualityPrecheckResult::Insufficient { summary, .. } => {
                Err(format!("数据不足，跳过分析: {summary}"))
            },
            QualityPrecheckResult::Pass | QualityPrecheckResult::Partial(_) => {
                let mut vars = ctx.variables;
                // 把 quote.name 写回 stock_name，供 enhance 钩子免二次行情请求
                if !vars.iter().any(|v| {
                    v.name == "stock_name" && !v.value.as_str().unwrap_or_default().is_empty()
                }) {
                    vars.push(Variable {
                        name: "stock_name".into(),
                        var_type: "string".into(),
                        value: json!(quote.name),
                        description: Some("当前分析的股票名称".into()),
                        is_secret: false,
                    });
                }
                Ok(vars)
            },
        }
    }

    async fn post_exec(&self, _ctx: HookExecContext, _outcome: &HookOutcome) -> Result<(), String> {
        Ok(())
    }
}

// ── 钩子 2：变量增强（pre_exec） ──────────────────────────────────

pub(crate) struct StockAnalysisEnhanceHook {
    db: DatabaseConnection,
    client: Arc<AStockClient>,
}

impl StockAnalysisEnhanceHook {
    pub(crate) fn new(db: DatabaseConnection, client: Arc<AStockClient>) -> Self {
        Self { db, client }
    }
}

#[async_trait::async_trait]
impl WorkflowLifecycleHook for StockAnalysisEnhanceHook {
    fn name(&self) -> &str {
        "stock-analysis-enhance"
    }

    async fn pre_exec(&self, ctx: HookExecContext) -> Result<Vec<Variable>, String> {
        // stock_code：优先 input（业务路径显式传入），回退变量（对话路径注入）
        let stock_code = input_str(&ctx.input, "stock_code")
            .map(str::to_string)
            .or_else(|| var_str(&ctx.variables, "stock_code").map(str::to_string))
            .ok_or_else(|| "stock_code 变量缺失，无法增强业务变量".to_string())?;
        // stock_name：优先 input（工作区路径带 quote.name），回退变量
        // （precheck 钩子已写入），最后降级空串（不阻断执行）
        let stock_name = input_str(&ctx.input, "stock_name")
            .map(str::to_string)
            .or_else(|| {
                var_str(&ctx.variables, "stock_name").map(str::to_string).filter(|s| !s.is_empty())
            })
            .unwrap_or_default();
        let screening_source = input_str(&ctx.input, "screening_source").map(str::to_string);
        let as_of_date = input_str(&ctx.input, "as_of_date").map(str::to_string);
        let analysis_id = input_str(&ctx.input, "analysis_id").map(str::to_string);

        Ok(build_stock_analysis_variables(
            &self.db,
            &self.client,
            &stock_code,
            &stock_name,
            ctx.variables,
            screening_source.as_deref(),
            as_of_date.as_deref(),
            analysis_id.as_deref(),
        )
        .await)
    }

    async fn post_exec(&self, _ctx: HookExecContext, _outcome: &HookOutcome) -> Result<(), String> {
        Ok(())
    }
}

// ── 钩子 3：结果持久化（post_exec） ──────────────────────────────

pub(crate) struct StockAnalysisPersistHook {
    db: DatabaseConnection,
}

impl StockAnalysisPersistHook {
    pub(crate) fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl WorkflowLifecycleHook for StockAnalysisPersistHook {
    fn name(&self) -> &str {
        "stock-analysis-persist"
    }

    async fn pre_exec(&self, ctx: HookExecContext) -> Result<Vec<Variable>, String> {
        Ok(ctx.variables)
    }

    async fn post_exec(&self, ctx: HookExecContext, outcome: &HookOutcome) -> Result<(), String> {
        // 业务封装路径已自行持久化（含 dashboard/自适应闭环/告警等完整后处理），
        // input 带 analysis_id 标记 → 跳过，避免重复写入。
        if input_has_analysis_id(&ctx.input) {
            return Ok(());
        }
        // 对话直执行路径：创建 stock_analyses 记录 + 反思 pending 占位
        let Some(stock_code) = input_str(&ctx.input, "stock_code")
            .map(str::to_string)
            .or_else(|| var_str(&ctx.variables, "stock_code").map(str::to_string))
        else {
            tracing::warn!("[stock-analysis-persist] stock_code 缺失，跳过持久化");
            return Ok(());
        };
        let stock_name = var_str(&ctx.variables, "stock_name").unwrap_or_default().to_string();

        let status = match outcome.status.as_str() {
            "completed" | "partially_completed" => "completed",
            "cancelled" => "cancelled",
            _ => "failed",
        };

        // 决策取值口径统一在 `decision::extract_decision_from_results_map`
        // （三级链尾优先：quality-fallback > portfolio-risk-gate > portfolio-mgr），
        // 与业务封装路径（`core.rs` → `decision::extract_decision_json`）共用同一实现
        // （铁律 41：同一语义不得被两条链按不同口径消费）。
        // 修复前此处**只认 portfolio-mgr** ⇒ 风控门的仓位修正（R-206 等）在对话直执行
        // 路径下被整体丢弃（V71 实证 601166：落库 8.85% vs 报告「已按 R-206 下调至 0%」）。
        let decision_json_str = extract_decision_from_results_map(&outcome.results);
        let (action, position_pct, reasoning, time_horizon, expected_holding_days) =
            extract_decision_fields(&decision_json_str);
        let horizon_source = extract_horizon_source(&decision_json_str);
        // 阶段1：抽四周期价位映射，与 core.rs 落库点共用同一提取实现
        let horizon_price_map = extract_horizon_price_map(&decision_json_str);
        // 阶段2：抽四周期独立决策，与 core.rs 落库点共用同一提取实现
        let horizon_decisions = extract_decisions_by_horizon(&decision_json_str);

        let now_ms = chrono::Utc::now().timestamp_millis();
        let analysis_id = uuid::Uuid::new_v4().to_string();
        let persist_result = stock_analyses::ActiveModel {
            id: Set(analysis_id.clone()),
            stock_code: Set(stock_code.clone()),
            stock_name: Set(stock_name.clone()),
            analysis_date: Set(chrono::Utc::now().format("%Y-%m-%d").to_string()),
            provider_id: Set("cognitive".into()),
            conversation_id: Set(uuid::Uuid::new_v4().to_string()),
            status: Set(status.into()),
            // P1-4(2026-09-14): 与 core.rs 的落库点共用同一归一化实现
            // （`decision_action` 的两个写入口值域必须一致）。
            decision_action: Set(normalize_action_for_storage(action.as_deref())),
            // P1-2: 与 action 正交的持仓状态轴（缺失保持 NULL）
            decision_position_state: Set(extract_position_state(&decision_json_str)),
            decision_position_pct: Set(position_pct),
            decision_reasoning: Set(reasoning.clone()),
            decision_json: Set(decision_json_str.clone()),
            horizon_price_map: Set(horizon_price_map),
            horizon_decisions: Set(horizon_decisions.clone()),
            llm_decision_json: Set(None),
            blackboard_snapshot: Set(Some(
                serde_json::to_string(&outcome.results).unwrap_or_else(|_| "{}".to_string()),
            )),
            config_id: Set(None),
            analysis_kind: Set("chat".into()),
            as_of_date: Set(Some(chrono::Utc::now().format("%Y-%m-%d").to_string())),
            model_version: Set(None),
            // A4：chat 通道（cognitive）决策落库 —— 不经过 `workflow_templates`，
            // 故无「公式版本」可言。⚠ 注意本行**是**有 blackboard_snapshot 的（见上），
            // 即「有快照但无版本」是这类记录的正常形态，复算器必须能区分
            // 「版本未知」与「版本不匹配」两种结论，不可混为一谈。
            template_version: Set(None),
            // 2026-09-24：本通道同样**不经过** `workflow_templates`，无模板 id 可言
            // ⇒ 显式 NULL（语义 = 链路未知）。读取侧的「排除快速链」过滤会放行它，
            // 这是对的：对话直执行的虽然是完整链，但不该因缺列而被当成快速链排除。
            template_id: Set(None),
            data_snapshot_id: Set(None),
            outcome: Set(None),
            // Phase 1：周期来源（由 portfolio-mgr.rhai 判定，见 decision.rs 的唯一读入口）
            decision_horizon_source: Set(horizon_source),
            decision_time_horizon: Set(time_horizon.clone()),
            decision_expected_holding_days: Set(expected_holding_days.map(|v| v as i64)),
            parent_analysis_id: Set(None),
            trade_intent_status: Set("pending".into()),
            trade_intent_source: Set(None),
            trade_intent_source_ref_id: Set(None),
            trade_intent_reviewed_at: Set(None),
            trade_intent_reviewed_by: Set(None),
            trade_intent_review_notes: Set(None),
            trade_intent_actual_trade_id: Set(None),
            created_at: Set(now_ms),
            updated_at: Set(now_ms),
        }
        .insert(&self.db)
        .await;
        if let Err(e) = persist_result {
            tracing::error!("[stock-analysis-persist] stock_analyses 写入失败: {e}");
            return Ok(()); // 结果已产生，持久化失败不阻断（post_exec 语义）
        }
        tracing::info!(
            "[stock-analysis-persist] 对话路径分析已落盘: {stock_code} ({stock_name}) status={status} analysis_id={analysis_id}"
        );

        // ── [B1 借鉴] 两阶段协议：决策成功时写 stock_reflections pending row ──
        // ⚠ 锚点用 `Utc::now()` 在本路径是**正确的**，不要照抄 `core.rs` 的 as-of 修法：
        // 本 hook 是**对话直执行通道**（`analysis_kind = "chat"`，见上方 `:828`），
        // 由用户当下提问触发，**不存在重放/as-of 语义** ⇒ today 就是分析锚点。
        // 而 `core.rs` 的 `run_stock_workflow_inner` 支持 `as_of_date` 重跑，
        // 那里若用 `now()` 会让 `hindsight_date` 落到未来、令反思以未来为 AS_OF 锚点
        // ⇒ `AsOfContext` 拒未来日期（`harness/src/as_of.rs:70-76`）⇒ 反思硬失败。
        // 判据：**先问这条链路有没有 as-of 入口**，再决定锚点用 now 还是 as-of。
        if status == "completed" {
            let today_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
            // 〇-B v2 第 4 条 / `PLAN-reflection-per-horizon-row`：**一行 pending = 一个复盘档**。
            //   本通道已落 `horizon_decisions`（四档独立决策）⇒ 逐档方向可判，按有方向的档各建一行。
            let pending_rows = super::reflection::build_pending_reflection_rows(
                &super::reflection::PendingReflectionSeed {
                    analysis_id: &analysis_id,
                    stock_code: &stock_code,
                    stock_name: &stock_name,
                    analysis_date: &today_str,
                    horizon_decisions: horizon_decisions.as_deref(),
                    primary_horizon: time_horizon.as_deref(),
                    primary_holding_days: expected_holding_days.map(|v| v as i64),
                    min_confidence_threshold: super::reflection::DEFAULT_REFLECTION_MIN_CONFIDENCE,
                    reflection_depth: super::reflection::DEFAULT_REFLECTION_DEPTH,
                    now_ms,
                },
            );
            let pending_count = pending_rows.len();
            for row in pending_rows {
                // 建行失败必须留痕：少一行 = 少一个档的复盘样本
                if let Err(e) = row.insert(&self.db).await {
                    tracing::warn!(
                        "[stock-analysis-persist] {stock_code} 建 pending 反思行失败: {e}"
                    );
                }
            }
            tracing::info!(
                "[stock-analysis-persist] {stock_code} 已落盘 {pending_count} 行 pending reflection（每档一行）"
            );
        }
        Ok(())
    }
}

// ── 注册 ─────────────────────────────────────────────────────────

/// 启动期把三个 stock-analysis 生命周期钩子注册进 WorkEngine。
///
/// 调用方：`init/state.rs`（astock_client 创建之后）。
pub(crate) async fn register_stock_analysis_hooks(
    engine: &WorkEngine,
    db: DatabaseConnection,
    client: Arc<AStockClient>,
) {
    engine
        .register_lifecycle_hook(Arc::new(StockAnalysisPrecheckHook::new(Arc::clone(&client))))
        .await;
    engine
        .register_lifecycle_hook(Arc::new(StockAnalysisEnhanceHook::new(
            db.clone(),
            Arc::clone(&client),
        )))
        .await;
    engine.register_lifecycle_hook(Arc::new(StockAnalysisPersistHook::new(db))).await;
    tracing::info!("[stock_workflow] 生命周期钩子已注册: precheck / enhance / persist");
}

#[cfg(test)]
mod divergence_attribution_tests {
    use super::*;

    /// 夹具 = 库里读出的真实形态（2026-10-02，`stock_analyses.id=75aac757…`，002812）。
    /// 证据后验 49.3% 本落在持有带，风险门槛压成生效后验 41.3% ⇒ 跨线到观望；
    /// 同票智选给 78 分 / 4.85% 试探仓。这正是用户问的「智选推荐、分析否决」。
    fn real_case_002812() -> serde_json::Value {
        json!({
            "action": "观望",
            "posterior": 41.3,
            "posteriorRaw": 49.3,
            "positionPct": 0.0,
            "evidence": { "factors": {
                "f1": {"name":"trend","sigma":0.26,"weight":0.105},
                "f2": {"name":"consensus","sigma":0.2,"weight":0.25},
                "f4": {"name":"risk","sigma":-0.351,"weight":0.15},
                "f5": {"name":"valuation","sigma":0.093,"weight":0.21},
                "f6": {"name":"data_quality","sigma":0.0,"weight":0.0},
                "f7": {"name":"trade_signal","sigma":0.0,"weight":0.0},
                "f10": {"name":"chip","sigma":-0.076,"weight":0.104},
                "f12": {"name":"momentum","sigma":-0.7,"weight":0.075},
                "f13": {"name":"bottleneck","sigma":0.0,"weight":0.0}
            }},
            "valuationApplicability": {"dcfApplicable": false},
            "data_gaps": [],
            "effective_params": {
                "action_buy_threshold": 0.63,
                "action_increase_threshold": 0.53,
                "action_hold_threshold": 0.48,
                "action_watch_threshold": 0.38,
                "action_reduce_threshold": 0.3
            }
        })
    }

    /// 夹具 = 真实形态（`id=94044f37…`，688498）：原始后验 == 生效后验 38.0，
    /// 风险门槛没动手 ⇒ 观望由证据本身给出（估值 / 一致预期 / 交易信号三条腿为负）。
    fn real_case_688498() -> serde_json::Value {
        json!({
            "action": "观望",
            "posterior": 38.0,
            "posteriorRaw": 38.0,
            "positionPct": 0.0,
            "evidence": { "factors": {
                "f1": {"name":"trend","sigma":0.32,"weight":0.105},
                "f2": {"name":"consensus","sigma":-0.2,"weight":0.25},
                "f3": {"name":"catalyst","sigma":-0.06,"weight":0.1},
                "f4": {"name":"risk","sigma":-0.107,"weight":0.15},
                "f5": {"name":"valuation","sigma":-0.485,"weight":0.21},
                "f6": {"name":"data_quality","sigma":0.0,"weight":0.0},
                "f7": {"name":"trade_signal","sigma":-0.362,"weight":0.1},
                "f9": {"name":"money_flow","sigma":-0.005,"weight":0.063},
                "f10": {"name":"chip","sigma":-0.076,"weight":0.104},
                "f12": {"name":"momentum","sigma":0.3,"weight":0.075}
            }},
            "valuationApplicability": {"dcfApplicable": false},
            "data_gaps": ["催化剂评估(a-catalyst)"],
            "effective_params": {
                "action_buy_threshold": 0.63,
                "action_increase_threshold": 0.53,
                "action_hold_threshold": 0.48,
                "action_watch_threshold": 0.38,
                "action_reduce_threshold": 0.3
            }
        })
    }

    fn drivers_of(d: &serde_json::Value) -> Vec<String> {
        d["drivers"]
            .as_array()
            .expect("drivers 必须是数组")
            .iter()
            .map(|v| v.as_str().expect("驱动项必须是字符串").to_string())
            .collect()
    }

    fn legs_of(d: &serde_json::Value) -> Vec<String> {
        d["legs"]
            .as_array()
            .expect("legs 必须是数组")
            .iter()
            .map(|l| l["name"].as_str().expect("腿名必须是字符串").to_string())
            .collect()
    }

    /// 跨了线的风险门槛必须被点名，且不重复报「低于持有线」（同一件事两个说法）。
    #[test]
    fn risk_gate_is_named_when_it_changed_the_tier() {
        let d = divergence_attribution(&real_case_002812());
        let drivers = drivers_of(&d);
        assert!(
            drivers.contains(&"risk_gate_downgrade".to_string()),
            "改变档位的风险门槛必须是首名归因，实得 {drivers:?}"
        );
        assert!(
            !drivers.contains(&"below_hold_threshold".to_string()),
            "已由 risk_gate 解释 ⇒ 不再重复报低于持有线，实得 {drivers:?}"
        );
        assert_eq!(d["posteriorRaw"], json!(49.3));
        assert_eq!(d["posteriorEffective"], json!(41.3));
        assert_eq!(d["holdThreshold"], json!(48.0));
        // 负贡献按 sigma×weight 由最负起排：risk −0.0527 < momentum −0.0525 < chip −0.0079
        assert_eq!(legs_of(&d), vec!["risk", "momentum", "chip"]);
    }

    /// 门槛没动手时，归因落到「证据本身」：观望带 ⇒ 低于持有线 + 最负的三条腿。
    #[test]
    fn watch_band_is_attributed_to_evidence_when_no_gate_fired() {
        let d = divergence_attribution(&real_case_688498());
        let drivers = drivers_of(&d);
        assert!(
            !drivers.contains(&"risk_gate_downgrade".to_string()),
            "原始=生效后验，门槛没改档位，不得谎报风险归因，实得 {drivers:?}"
        );
        assert!(drivers.contains(&"below_hold_threshold".to_string()));
        assert!(drivers.contains(&"dcf_leg_excluded".to_string()));
        assert!(drivers.contains(&"data_gap".to_string()));
        assert_eq!(legs_of(&d), vec!["valuation", "consensus", "trade_signal"]);
    }

    /// 权重为 0 的腿（该档被降权/退出）与 sigma≥0 的腿都不得进负贡献清单。
    #[test]
    fn zero_weight_and_positive_legs_are_excluded() {
        let d = divergence_attribution(&real_case_688498());
        assert!(
            !legs_of(&d).contains(&"data_quality".to_string()),
            "f6 权重 0 ⇒ 未参与决策，不得列为否决来源，实得 {:?}",
            legs_of(&d)
        );
        assert!(d["legs"].as_array().expect("legs").len() <= 3, "至多三条，避免长串刷屏");
    }

    /// 字段缺失 ⇒ 一条归因都不产（不得用默认阈值/借用别票后验把空白填成结论）。
    #[test]
    fn missing_fields_produce_no_claims() {
        let d = divergence_attribution(&json!({}));
        assert!(drivers_of(&d).is_empty(), "空决策不得产任何归因");
        assert!(d.get("posteriorRaw").is_none(), "无 effective_params ⇒ 不报档位归因也不回显后验");
        assert!(d.get("legs").is_none(), "无 factors ⇒ 不写 legs 键");
    }

    /// 接线自证：归因必须随 `crossCheck` 一起落进决策 JSON（改前此处无 divergence 键 ⇒ 红）。
    #[test]
    fn crosscheck_carries_the_attribution() {
        let mut decision = real_case_002812();
        inject_reco_crosscheck(
            &mut decision,
            &json!({ "recoConfidence": 78.0, "recoPositionPct": 4.85, "recoPeriod": "mid" }),
        );
        let cc = &decision["crossCheck"];
        assert_eq!(cc["divergent"], json!(true), "智选有仓 + 工作流观望 ⇒ 分歧成立");
        assert!(
            drivers_of(&cc["divergence"]).contains(&"risk_gate_downgrade".to_string()),
            "分歧报告必须带上归因，实得 {}",
            cc["divergence"]
        );
    }
}
