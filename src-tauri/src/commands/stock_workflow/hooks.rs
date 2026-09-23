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
    extract_decisions_by_horizon, extract_horizon_price_map, extract_position_state,
    normalize_action_for_storage,
};
use axagent_astock_data::AStockClient;
use axagent_entities::stock_analyses;
use axagent_entities::stock_reflections;
use axagent_harness::workflow_types::Variable;
use axagent_harness::{HookExecContext, HookOutcome, WorkflowLifecycleHook};
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

/// 将智选推荐先验与工作流决策做跨系统互证，把 `crossCheck` 就地写入决策 JSON。
///
/// 分歧判定：智选 confidence≥60 且建议仓位>0，而工作流 action=观望/卖出 或仓位≤0
/// ——此时前端展示「智选推荐 vs 工作流否决」分歧报告。
/// 本函数纯结构化注入、不做叙述文本（叙事由前端 i18n 渲染）。
pub(crate) fn inject_reco_crosscheck(
    decision_value: &mut serde_json::Value,
    reco_prior: &serde_json::Value,
) {
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
    // X1 修复: 当 screening_source = serenity 时，从候选缓存注入瓶颈分析数据
    // 使 portfolio-mgr.rhai 能感知 Serenity 瓶颈分析结果，增加因子 6: 瓶颈置信度
    if screening_source == Some("serenity") {
        if let Some(detail) =
            axagent_analysis_engine::recommender::get_serenity_candidate_detail(stock_code)
        {
            merged_vars.push(Variable {
                name: "serenity_context".into(),
                var_type: "object".into(),
                value: detail.clone(),
                description: Some(
                    "Serenity 瓶颈分析上下文（serenity_score / bottleneck_product / catalysts 等）"
                        .into(),
                ),
                is_secret: false,
            });
            tracing::info!(
                "[stock-analysis] 注入 serenity_context: score={}, bottleneck={}",
                detail["serenity_score"].as_f64().unwrap_or(0.0),
                detail["bottleneck_product"].as_str().unwrap_or("")
            );
        } else {
            tracing::warn!(
                "[stock-analysis] screening_source=serenity 但候选缓存为空: {}",
                stock_code
            );
        }
    }

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
    let (lessons_str, applied_lesson_ids) = fetch_stock_lessons(stock_code, db).await;
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

/// 从节点结果 map（Value 形式）提取 portfolio-mgr 决策 JSON 字符串。
///
/// 与 `decision::extract_decision_json`（接收 &Workflow）逻辑同构：
/// 优先 `.result`（CodeNode Rhai 输出包装），回退 `.output`（V63 格式），
/// 最后整个 portfolio-mgr 值。
fn extract_decision_from_results(results: &serde_json::Value) -> Option<String> {
    let pm = results.get("portfolio-mgr")?;
    let actual = match pm {
        serde_json::Value::Object(obj) => {
            obj.get("result").or_else(|| obj.get("output")).cloned().unwrap_or_else(|| pm.clone())
        },
        _ => pm.clone(),
    };
    serde_json::to_string(&actual).ok()
}

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

        let decision_json_str = extract_decision_from_results(&outcome.results);
        let (action, position_pct, reasoning, time_horizon, expected_holding_days) =
            extract_decision_fields(&decision_json_str);
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
            horizon_decisions: Set(horizon_decisions),
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
            data_snapshot_id: Set(None),
            outcome: Set(None),
            decision_time_horizon: Set(time_horizon),
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
            let hold_days = expected_holding_days.unwrap_or(28) as i64;
            let hindsight_date_str = {
                let h = chrono::NaiveDate::parse_from_str(&today_str, "%Y-%m-%d")
                    .map(|d| d + chrono::Duration::days(hold_days))
                    .unwrap_or_else(|_| {
                        chrono::Local::now().date_naive() + chrono::Duration::days(hold_days)
                    });
                h.format("%Y-%m-%d").to_string()
            };
            let pending_id = uuid::Uuid::new_v4().to_string();
            let _ = stock_reflections::ActiveModel {
                id: Set(pending_id.clone()),
                stock_code: Set(stock_code.clone()),
                stock_name: Set(stock_name.clone()),
                original_analysis_id: Set(analysis_id.clone()),
                as_of_date: Set(today_str.clone()),
                hindsight_date: Set(hindsight_date_str),
                min_confidence_threshold: Set(70),
                reflection_depth: Set("light".to_string()),
                actual_outcome: Set(String::new()),
                raw_return: Set(None),
                alpha_return: Set(None),
                holding_days: Set(None),
                benchmark_name: Set(None),
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
                created_at: Set(now_ms),
                updated_at: Set(now_ms),
            }
            .insert(&self.db)
            .await;
            tracing::info!(
                "[stock-analysis-persist] {stock_code} 已落盘 pending reflection {pending_id}"
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
