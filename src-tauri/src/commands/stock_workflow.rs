//! 工作流驱动的股票分析 — 基于持久化 WorkflowTemplate + WorkEngine DAG 执行。
//!
//! 启动时种子化 stock-analysis 工作流模板到 workflow_templates 表，
//! 每次分析从模板加载 DAG 结构，注入实时行情数据，由 WorkEngine 并行执行。

use crate::AppState;
use axagent_core::entity::stock_analyses;
use axagent_core::workflow_types::{WorkflowEdge, WorkflowNode};
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Emitter, State};

// ── 算法工具：内联计算（独立于 orchestrator，供 Tool 节点调用）──

fn compute_scoring_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let klines: Vec<axagent_astock_data::KLine> = args
        .get("kline_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    if klines.is_empty() {
        return Ok(json!({"error": "K线数据为空，无法评分"}));
    }
    let price = klines.last().map(|k| k.close).unwrap_or(0.0);
    let sc = args
        .get("stock_code")
        .and_then(|v| v.as_str())
        .unwrap_or("000001");
    let indicators = axagent_astock_data::indicators::compute_indicators(sc, &klines);
    let score = axagent_stock_analysis::scoring::ScoringEngine::score(&indicators, price, None);
    Ok(serde_json::to_value(&score).unwrap_or_default())
}

fn compute_valuation_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let financials: Vec<axagent_astock_data::FinancialReport> = args
        .get("financials_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let quote_price = args
        .get("quote_price")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let metrics = axagent_stock_analysis::value::ValueEngine::assess(quote_price, &financials, 1.0);
    Ok(serde_json::to_value(&metrics).unwrap_or_default())
}

fn compute_portfolio_risk_inner(args: &serde_json::Value) -> serde_json::Value {
    let positions: Vec<serde_json::Value> = args
        .get("positions_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let total_mv: f64 = positions
        .iter()
        .filter_map(|p| p.get("marketValue").and_then(|v| v.as_f64()))
        .sum();
    let max_single = positions
        .iter()
        .filter_map(|p| p.get("marketValue").and_then(|v| v.as_f64()))
        .fold(0.0_f64, f64::max);
    let concentration = if total_mv > 0.0 {
        (max_single / total_mv) * 100.0
    } else {
        0.0
    };
    let risk_level = if concentration > 50.0 {
        "高风险"
    } else if concentration > 30.0 {
        "中高风险"
    } else if concentration > 20.0 {
        "中等风险"
    } else {
        "低风险"
    };
    json!({
        "total_market_value": total_mv,
        "position_count": positions.len(),
        "concentration_pct": (concentration * 10.0).round() / 10.0,
        "max_single_pct": if total_mv > 0.0 { (max_single / total_mv * 1000.0).round() / 10.0 } else { 0.0 },
        "risk_level": risk_level,
    })
}

fn run_quality_gate_inner(args: &serde_json::Value) -> serde_json::Value {
    let reports: HashMap<String, String> = args
        .get("reports_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let check = axagent_stock_analysis::quality::run_quality_gate(&reports);
    json!({
        "grade": format!("{:?}", check.grade),
        "summary": check.summary,
        "warnings": check.warnings,
    })
}

// ── 新增：12 个金融模型 tool handler 内联函数 ──

fn calc_max_drawdown_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let prices: Vec<f64> = args
        .get("prices_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let dd = axagent_stock_analysis::risk::max_drawdown(&prices);
    Ok(json!({"max_drawdown_pct": (dd * 10000.0).round() / 100.0}))
}

fn calc_sharpe_ratio_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let returns: Vec<f64> = args
        .get("returns_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let rf = args
        .get("risk_free")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.03);
    let r = axagent_stock_analysis::risk::sharpe_ratio(&returns, rf);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn calc_var_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let returns: Vec<f64> = args
        .get("returns_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let conf = args
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.95);
    let r = axagent_stock_analysis::risk::value_at_risk(&returns, conf);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn calc_pe_percentile_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let current_pe = args
        .get("current_pe")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let hist: Vec<f64> = args
        .get("historical_pes_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let r = axagent_stock_analysis::risk::pe_percentile(current_pe, &hist);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn calc_peg_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let pe = args.get("pe").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let growth = args
        .get("growth_rate")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let r = axagent_stock_analysis::risk::peg_ratio(pe, growth);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn detect_ma_cross_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let klines_json = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let fast = args
        .get("fast_period")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as usize;
    let slow = args
        .get("slow_period")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    let r = axagent_stock_analysis::signals::detect_ma_cross(klines_json, fast, slow);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn detect_breakout_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let klines_json = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let support = args.get("support").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let resistance = args
        .get("resistance")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let r = axagent_stock_analysis::signals::detect_breakout(klines_json, support, resistance);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn calc_kelly_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let win_rate = args.get("win_rate").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let avg_win = args.get("avg_win").and_then(|v| v.as_f64()).unwrap_or(0.05);
    let avg_loss = args
        .get("avg_loss")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.05);
    let r = axagent_stock_analysis::risk::kelly_criterion(win_rate, avg_win, avg_loss);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn calc_risk_parity_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let vols: Vec<f64> = args
        .get("volatilities_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let corr = args
        .get("correlations_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let r = axagent_stock_analysis::risk::risk_parity_weights(&vols, corr);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn clean_outliers_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let prices_json = args
        .get("prices_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let method = args
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("zscore");
    let threshold = args
        .get("threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(2.0);
    let r = axagent_stock_analysis::data_clean::remove_outliers(prices_json, method, threshold);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn clean_fill_missing_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let prices_json = args
        .get("prices_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let method = args
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("forward");
    let r = axagent_stock_analysis::data_clean::fill_missing(prices_json, method);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

fn adjust_prices_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let klines_json = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let dividends_json = args
        .get("dividends_json")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let r = axagent_stock_analysis::data_clean::adjust_prices(klines_json, dividends_json);
    Ok(serde_json::to_value(&r).unwrap_or_default())
}

// ── 新增：9 个数据 API tool handler（封装 AStockClient 已有方法）──

async fn get_research_reports_inner(
    client: &axagent_astock_data::AStockClient,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let code = args
        .get("stock_code")
        .and_then(|v| v.as_str())
        .unwrap_or("000001");
    match client.get_research_reports(code).await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

async fn get_consensus_eps_inner(
    client: &axagent_astock_data::AStockClient,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let code = args
        .get("stock_code")
        .and_then(|v| v.as_str())
        .unwrap_or("000001");
    match client.get_consensus_eps(code).await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

async fn get_concept_blocks_inner(
    client: &axagent_astock_data::AStockClient,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let code = args
        .get("stock_code")
        .and_then(|v| v.as_str())
        .unwrap_or("000001");
    match client.get_concept_blocks(code).await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

async fn get_announcements_inner(
    client: &axagent_astock_data::AStockClient,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let code = args
        .get("stock_code")
        .and_then(|v| v.as_str())
        .unwrap_or("000001");
    match client.get_announcements(code).await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

// ── 市场级数据工具（无需 stock_code）──

async fn get_market_dragon_tiger_inner(
    client: &axagent_astock_data::AStockClient,
) -> Result<serde_json::Value, String> {
    match client.get_market_dragon_tiger().await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

async fn get_hot_stocks_inner(
    client: &axagent_astock_data::AStockClient,
) -> Result<serde_json::Value, String> {
    match client.get_hot_stocks().await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

async fn get_industry_ranking_inner(
    client: &axagent_astock_data::AStockClient,
) -> Result<serde_json::Value, String> {
    match client.get_industry_ranking().await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

async fn get_cls_flash_inner(
    client: &axagent_astock_data::AStockClient,
) -> Result<serde_json::Value, String> {
    match client.get_cls_flash().await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

async fn get_north_bound_flow_inner(
    client: &axagent_astock_data::AStockClient,
    _args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    match client.get_north_bound_flow().await {
        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

// ── P1: 4 个技术指标补充 ──

fn compute_atr_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    #[derive(serde::Deserialize)]
    struct Raw { high: f64, low: f64, close: f64 }
    let klines: Vec<Raw> = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let period = args.get("period").and_then(|v| v.as_u64()).unwrap_or(14) as usize;
    let n = klines.len();
    if n < 2 || period == 0 {
        return Ok(json!({"atr": 0.0, "period": period}));
    }
    let mut trs = Vec::with_capacity(n - 1);
    for i in 1..n {
        let prev = &klines[i - 1];
        let cur = &klines[i];
        let tr = (cur.high - cur.low)
            .max((cur.high - prev.close).abs())
            .max((cur.low - prev.close).abs());
        trs.push(tr);
    }
    let atr = if trs.len() <= period {
        trs.iter().sum::<f64>() / trs.len() as f64
    } else {
        let mut atr_val = trs[..period].iter().sum::<f64>() / period as f64;
        for &tr in &trs[period..] {
            atr_val = (atr_val * (period - 1) as f64 + tr) / period as f64;
        }
        atr_val
    };
    let latest_price = klines.last().map(|k| k.close).unwrap_or(0.0);
    Ok(json!({"atr": (atr * 100.0).round() / 100.0, "atr_pct": (atr / latest_price * 10000.0).round() / 100.0, "period": period}))
}

fn compute_kdj_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    #[derive(serde::Deserialize)]
    struct Raw { high: f64, low: f64, close: f64 }
    let klines: Vec<Raw> = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(9) as usize;
    if klines.len() < n {
        return Ok(json!({"k": 50.0, "d": 50.0, "j": 50.0, "signal": "中性"}));
    }
    let mut k = 50.0_f64;
    let mut d = 50.0_f64;
    for i in (n - 1)..klines.len() {
        let window = &klines[i + 1 - n..=i];
        let low_min = window.iter().map(|x| x.low).fold(f64::MAX, f64::min);
        let high_max = window.iter().map(|x| x.high).fold(f64::MIN, f64::max);
        let close = window.last().unwrap().close;
        let rsv = if (high_max - low_min).abs() > 1e-10 {
            (close - low_min) / (high_max - low_min) * 100.0
        } else {
            50.0
        };
        k = 2.0 / 3.0 * k + 1.0 / 3.0 * rsv;
        d = 2.0 / 3.0 * d + 1.0 / 3.0 * k;
    }
    let j = 3.0 * k - 2.0 * d;
    let signal = if j > 100.0 { "严重超买" } else if j > 80.0 { "超买" } else if j < 0.0 { "严重超卖" } else if j < 20.0 { "超卖" } else if k > d { "多头" } else { "空头" };
    Ok(json!({"k": (k * 100.0).round() / 100.0, "d": (d * 100.0).round() / 100.0, "j": (j * 100.0).round() / 100.0, "signal": signal}))
}

fn compute_obv_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    #[derive(serde::Deserialize)]
    struct Raw { close: f64, volume: f64 }
    let klines: Vec<Raw> = args
        .get("klines_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    if klines.is_empty() {
        return Ok(json!({"obv": 0.0, "trend": "无数据"}));
    }
    let mut obv = 0.0_f64;
    for i in 1..klines.len() {
        if klines[i].close > klines[i - 1].close {
            obv += klines[i].volume;
        } else if klines[i].close < klines[i - 1].close {
            obv -= klines[i].volume;
        }
    }
    let obv_ma5 = if klines.len() >= 6 {
        let obvs: Vec<f64> = (1..klines.len()).scan(0.0, |acc, i| {
            if klines[i].close > klines[i - 1].close { *acc += klines[i].volume; }
            else if klines[i].close < klines[i - 1].close { *acc -= klines[i].volume; }
            Some(*acc)
        }).collect();
        let n = obvs.len();
        if n >= 5 { obvs[n - 5..].iter().sum::<f64>() / 5.0 } else { obv }
    } else { obv };
    let trend = if obv > obv_ma5 * 1.1 { "量价齐升" } else if obv < obv_ma5 * 0.9 { "量价背离" } else { "量价平稳" };
    Ok(json!({"obv": (obv / 1e8 * 100.0).round() / 100.0, "obv_ma5": (obv_ma5 / 1e8 * 100.0).round() / 100.0, "trend": trend, "unit": "亿"}))
}

fn calc_beta_inner(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let returns_stock: Vec<f64> = args
        .get("stock_returns_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let returns_market: Vec<f64> = args
        .get("market_returns_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let n = returns_stock.len().min(returns_market.len());
    if n < 2 {
        return Ok(json!({"beta": 1.0, "r_squared": 0.0, "interpretation": "数据不足"}));
    }
    let s = &returns_stock[..n];
    let m = &returns_market[..n];
    let mean_s: f64 = s.iter().sum::<f64>() / n as f64;
    let mean_m: f64 = m.iter().sum::<f64>() / n as f64;
    let cov: f64 = s.iter().zip(m.iter()).map(|(&si, &mi)| (si - mean_s) * (mi - mean_m)).sum::<f64>() / (n - 1) as f64;
    let var_m: f64 = m.iter().map(|&mi| (mi - mean_m).powi(2)).sum::<f64>() / (n - 1) as f64;
    let beta = if var_m > 1e-10 { cov / var_m } else { 1.0 };
    // R²
    let var_s: f64 = s.iter().map(|&si| (si - mean_s).powi(2)).sum::<f64>() / (n - 1) as f64;
    let r_sq = if var_s > 1e-10 && var_m > 1e-10 { (cov / (var_s.sqrt() * var_m.sqrt())).powi(2) } else { 0.0 };
    let interp = if beta > 1.5 { "高波动" } else if beta > 1.1 { "略高于市场" } else if beta > 0.9 { "与市场同步" } else if beta > 0.5 { "防御型" } else { "低波动" };
    Ok(json!({"beta": (beta * 1000.0).round() / 1000.0, "r_squared": (r_sq * 1000.0).round() / 1000.0, "interpretation": interp}))
}

/// 从 DB 加载工作流模板，仅注入 stock_code 到 Trigger 节点。
/// 专家 prompt 由 AgentExecutor 从 agent_profile 自动加载，
/// 行情数据通过 ToolNode 的 context_sources 由上游工具节点输出注入，
/// 运行时变量由 prompt_template 两阶段渲染处理。
async fn load_and_inject_template(
    db: &sea_orm::DatabaseConnection,
    stock_code: &str,
) -> Result<(Vec<WorkflowNode>, Vec<WorkflowEdge>), String> {
    use axagent_core::entity::workflow_template;

    let template = workflow_template::Entity::find_by_id("stock-analysis")
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
        .ok_or("股票分析工作流模板未种子化，请重启应用")?;

    let mut nodes: Vec<WorkflowNode> =
        serde_json::from_str(&template.nodes).map_err(|e| format!("解析模板节点失败: {e}"))?;
    let edges: Vec<WorkflowEdge> =
        serde_json::from_str(&template.edges).map_err(|e| format!("解析模板边失败: {e}"))?;

    // 仅注入 stock_code 到 trigger 节点，prompt 不再手动替换
    for node in &mut nodes {
        if let WorkflowNode::Trigger(tn) = node {
            if let Some(sc) = tn.config.config.get_mut("stock_code") {
                *sc = serde_json::Value::String(stock_code.to_string());
            }
        }
    }

    Ok((nodes, edges))
}

#[tauri::command]
pub async fn run_stock_workflow(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    stock_code: String,
) -> Result<serde_json::Value, String> {
    // ── 1. 获取行情基本信息（用于写入分析记录）──
    let quote = state
        .astock_client
        .get_quote(&stock_code)
        .await
        .map_err(|e| format!("行情获取失败: {e}"))?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let analysis_id = uuid::Uuid::new_v4().to_string();

    // 写入 stock_analyses 表
    stock_analyses::ActiveModel {
        id: Set(analysis_id.clone()),
        stock_code: Set(stock_code.clone()),
        stock_name: Set(quote.name.clone()),
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
        created_at: Set(now_ms),
        updated_at: Set(now_ms),
    }
    .insert(&state.sea_db)
    .await
    .map_err(|e| format!("DB 写入失败: {e}"))?;

    // ── 2. 从模板加载 DAG 并注入 stock_code ──
    let (nodes, edges) = load_and_inject_template(&state.sea_db, &stock_code).await?;

    // 加载模板的 schema 和 variables
    use axagent_core::entity::workflow_template;
    use axagent_core::workflow_types::JsonSchema;
    let template = workflow_template::Entity::find_by_id("stock-analysis")
        .one(&state.sea_db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
        .ok_or("股票分析工作流模板未种子化，请重启应用")?;
    let input_schema: Option<JsonSchema> = template
        .input_schema
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());
    let output_schema: Option<JsonSchema> = template
        .output_schema
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());

    // ── 3. 注册工具 handler（数据获取 + 算法计算）──
    let engine = Arc::clone(&state.work_engine);

    // 数据工具
    let tool_client = Arc::clone(&state.astock_client);
    let sc = stock_code.clone();
    engine
        .register_tool_handler(
            "search_stock",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                let code = sc.clone();
                Box::pin(async move {
                    let kw = args["keyword"].as_str().unwrap_or(&code);
                    match client.search_stock(kw).await {
                        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
                        Err(e) => Ok(json!({"error": e.to_string()})),
                    }
                })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    let sc = stock_code.clone();
    engine
        .register_tool_handler(
            "get_stock_quote",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                let code = sc.clone();
                Box::pin(async move {
                    let c = args["stock_code"].as_str().unwrap_or(&code);
                    match client.get_quote(c).await {
                        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
                        Err(e) => Ok(json!({"error": e.to_string()})),
                    }
                })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    let sc = stock_code.clone();
    engine
        .register_tool_handler(
            "get_stock_kline",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                let code = sc.clone();
                Box::pin(async move {
                    let c = args["stock_code"].as_str().unwrap_or(&code);
                    let period = args["period"].as_str().unwrap_or("daily");
                    let limit = args["limit"].as_u64().unwrap_or(120) as u32;
                    match client.get_klines(c, period, limit).await {
                        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
                        Err(e) => Ok(json!({"error": e.to_string()})),
                    }
                })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    let sc = stock_code.clone();
    engine
        .register_tool_handler(
            "get_stock_financials",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                let code = sc.clone();
                Box::pin(async move {
                    let c = args["stock_code"].as_str().unwrap_or(&code);
                    match client.get_financials(c).await {
                        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
                        Err(e) => Ok(json!({"error": e.to_string()})),
                    }
                })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    let sc = stock_code.clone();
    engine
        .register_tool_handler(
            "get_stock_news",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                let code = sc.clone();
                Box::pin(async move {
                    let c = args["stock_code"].as_str().unwrap_or(&code);
                    let limit = args["limit"].as_u64().unwrap_or(30) as u32;
                    match client.get_news(c, limit).await {
                        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
                        Err(e) => Ok(json!({"error": e.to_string()})),
                    }
                })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    let sc = stock_code.clone();
    engine
        .register_tool_handler(
            "get_stock_money_flow",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                let code = sc.clone();
                Box::pin(async move {
                    let c = args["stock_code"].as_str().unwrap_or(&code);
                    match client.get_money_flow(c).await {
                        Ok(v) => Ok(serde_json::to_value(v).unwrap_or_default()),
                        Err(e) => Ok(json!({"error": e.to_string()})),
                    }
                })
            }),
        )
        .await;

    // 新增 4 个技术指标工具 (P1)
    engine
        .register_tool_handler(
            "compute_atr",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { compute_atr_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "compute_kdj",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { compute_kdj_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "compute_obv",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { compute_obv_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "calc_beta",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { calc_beta_inner(&args) })
            }),
        )
        .await;

    // 新增 9 个数据 API 工具
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_research_reports",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_research_reports_inner(&client, &args).await })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_consensus_eps",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_consensus_eps_inner(&client, &args).await })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_concept_blocks",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_concept_blocks_inner(&client, &args).await })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_announcements",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_announcements_inner(&client, &args).await })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_north_bound_flow",
            Arc::new(move |_name: String, args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_north_bound_flow_inner(&client, &args).await })
            }),
        )
        .await;
    // 市场级工具（无需 stock_code）
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_market_dragon_tiger",
            Arc::new(move |_name: String, _args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_market_dragon_tiger_inner(&client).await })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_hot_stocks",
            Arc::new(move |_name: String, _args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_hot_stocks_inner(&client).await })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_industry_ranking",
            Arc::new(move |_name: String, _args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_industry_ranking_inner(&client).await })
            }),
        )
        .await;
    let tool_client = Arc::clone(&state.astock_client);
    engine
        .register_tool_handler(
            "get_cls_flash",
            Arc::new(move |_name: String, _args: serde_json::Value| {
                let client = Arc::clone(&tool_client);
                Box::pin(async move { get_cls_flash_inner(&client).await })
            }),
        )
        .await;

    // 算法工具
    engine
        .register_tool_handler(
            "compute_scoring",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { compute_scoring_inner(&args).map_err(|e| e.to_string()) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "compute_valuation",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { compute_valuation_inner(&args).map_err(|e| e.to_string()) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "compute_portfolio_risk",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { Ok(compute_portfolio_risk_inner(&args)) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "run_quality_gate",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { Ok(run_quality_gate_inner(&args)) })
            }),
        )
        .await;

    // 新增 12 个金融模型工具
    engine
        .register_tool_handler(
            "calc_max_drawdown",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { calc_max_drawdown_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "calc_sharpe_ratio",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { calc_sharpe_ratio_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "calc_var",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { calc_var_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "calc_pe_percentile",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { calc_pe_percentile_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "calc_peg",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { calc_peg_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "detect_ma_cross",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { detect_ma_cross_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "detect_breakout",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { detect_breakout_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "calc_kelly",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { calc_kelly_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "calc_risk_parity",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { calc_risk_parity_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "clean_outliers",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { clean_outliers_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "clean_fill_missing",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { clean_fill_missing_inner(&args) })
            }),
        )
        .await;
    engine
        .register_tool_handler(
            "adjust_prices",
            Arc::new(|_name: String, args: serde_json::Value| {
                Box::pin(async move { adjust_prices_inner(&args) })
            }),
        )
        .await;

    // ── 4. 创建并执行工作流 ──
    let wf_name = format!("stock-analysis-{stock_code}");
    let workflow = engine
        .create_workflow(&wf_name, nodes, edges)
        .await
        .map_err(|e| format!("创建工作流失败: {e}"))?;
    let wf_id = workflow.id.clone();
    let wf_id_ret = wf_id.clone();
    let app_h = app.clone();
    let db = state.sea_db.clone();
    let aid = analysis_id.clone();

    // 进度回调
    let progress_app = app.clone();
    let progress_wf_id = wf_id.clone();
    let progress_cb: ProgressCallback = Arc::new(move |event: StepProgressEvent| {
        let app = progress_app.clone();
        let wf_id = progress_wf_id.clone();
        Box::pin(async move {
            let _ = app.emit(
                "workflow-step-done",
                serde_json::json!({
                    "workflowId": wf_id,
                    "nodeId": event.node_id,
                    "status": event.status,
                    "totalNodes": event.total_nodes,
                    "completedNodes": event.completed_nodes,
                }),
            );
        })
    });

    let sc_for_ret = stock_code.clone();
    let sc_name = quote.name.clone();
    tokio::spawn(async move {
        let mut opts = RunOptions::default()
            .with_max_concurrent(9)
            .with_step_timeout(std::time::Duration::from_secs(300))
            .with_progress_callback(progress_cb)
            .with_input(json!({"stock_code": &stock_code}));
        if let Some(s) = input_schema {
            opts = opts.with_input_schema(s);
        }
        if let Some(s) = output_schema {
            opts = opts.with_output_schema(s);
        }

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
                        let _ = app_h.emit(
                            "workflow-error",
                            serde_json::json!({ "workflowId": wf_id, "error": "部分分析步骤失败" }),
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
                        // 优先用 output（EndNode 聚合 + output_schema 过滤），
                        // 回退到 results["portfolio-mgr"] 兼容旧模板
                        let decision_json = result
                            .output
                            .and_then(|v| serde_json::to_string(&v).ok())
                            .or_else(|| {
                                result
                                    .results
                                    .get("portfolio-mgr")
                                    .and_then(|v| serde_json::to_string(v).ok())
                            });
                        let _ = stock_analyses::Entity::update_many()
                            .col_expr(stock_analyses::Column::Status, Expr::value("completed"))
                            .col_expr(
                                stock_analyses::Column::DecisionJson,
                                Expr::value(decision_json),
                            )
                            .col_expr(
                                stock_analyses::Column::UpdatedAt,
                                Expr::value(chrono::Utc::now().timestamp_millis()),
                            )
                            .filter(stock_analyses::Column::Id.eq(&aid))
                            .exec(&db)
                            .await;
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
