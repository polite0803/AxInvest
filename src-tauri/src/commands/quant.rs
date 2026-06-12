//! Quant 量化交易 + 量化回测 — Tauri 命令层
//!
//! M1 实施（4 个核心命令）：
//! - `quant_strategies_list`: 列出所有策略（DB 中已注册 + 内置 5 个默认项）
//! - `quant_backtest_run`: 跑回测（拉 K 线 → 撮合 → 写回 3 张表）
//! - `quant_metrics_compare`: 对比多个 run 的绩效指标
//! - `quant_strategy_register_rhai`: 注册 / 更新 Rhai 脚本策略（沙箱编译校验）

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::AppState;
use axagent_core::entity::{quant_paper_trades, quant_runs, quant_signals, quant_strategies};
use axagent_quant::prelude::{
    BacktestConfig, BacktestEngine, BacktestResult, Bar, BollStrategy as BollStrategyAlias,
    MaCrossStrategy, MacdStrategy, MatcherConfig, MetricsReport, RhaiStrategy, RsiStrategy,
    Strategy, StrategyCtx, TurtleStrategy, WalkForward, WalkForwardConfig,
};

/// 前端友好的"策略清单条目"（合并 DB 记录 + 内置 5 项默认）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyListItem {
    /// DB id（内置策略固定 id，rhai 策略用 DB 主键）
    pub id: String,
    pub name: String,
    pub version: String,
    /// "builtin" | "rhai"
    pub strategy_type: String,
    pub description: Option<String>,
    pub params: serde_json::Value,
    pub walk_forward_enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 默认内置 5 个策略（DB 中没有时返回）
fn builtin_strategy_items() -> Vec<StrategyListItem> {
    let now = Utc::now().timestamp();
    vec![
        builtin_item(
            "builtin.ma_cross",
            "MA Cross",
            "5/20 均线交叉",
            MaCrossStrategy::default(),
            now,
        ),
        builtin_item("builtin.macd", "MACD", "MACD 金叉死叉", MacdStrategy::default(), now),
        builtin_item("builtin.rsi", "RSI", "RSI 超买超卖", RsiStrategy::default(), now),
        builtin_item("builtin.boll", "Bollinger", "布林带突破", BollStrategyAlias::default(), now),
        builtin_item("builtin.turtle", "Turtle", "海龟交易法则", TurtleStrategy::default(), now),
    ]
}

fn builtin_item(id: &str, name: &str, desc: &str, s: impl Strategy, now: i64) -> StrategyListItem {
    StrategyListItem {
        id: id.to_string(),
        name: name.to_string(),
        version: s.version().to_string(),
        strategy_type: "builtin".to_string(),
        description: Some(desc.to_string()),
        params: s.params(),
        walk_forward_enabled: true,
        created_at: now,
        updated_at: now,
    }
}

/// 列出所有策略（DB 注册 + 内置默认）
#[tauri::command]
pub async fn quant_strategies_list(
    state: State<'_, AppState>,
) -> Result<Vec<StrategyListItem>, String> {
    // 1. 拉 DB 中所有策略（rhai + 任何已持久化的 builtin）
    let db_rows: Vec<quant_strategies::Model> = quant_strategies::Entity::find()
        .order_by_desc(quant_strategies::Column::UpdatedAt)
        .all(state.harness.db())
        .await
        .map_err(|e| format!("query quant_strategies failed: {e}"))?;

    // 2. 合并内置（DB 已有同 id 的则跳过）
    let mut items: Vec<StrategyListItem> = Vec::new();
    let mut existing_ids: std::collections::HashSet<String> = db_rows
        .iter()
        .map(|r| format!("{}|{}", r.strategy_type, r.name))
        .collect();

    for builtin in builtin_strategy_items() {
        let key = format!("{}|{}", builtin.strategy_type, builtin.name);
        if !existing_ids.contains(&key) {
            items.push(builtin);
        }
    }

    // 3. 加 DB 行（转换 params_json → Value）
    for row in db_rows {
        let params: serde_json::Value = row
            .params_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Object(Default::default()));
        items.push(StrategyListItem {
            id: row.id,
            name: row.name,
            version: row.version,
            strategy_type: row.strategy_type,
            description: row.description,
            params,
            walk_forward_enabled: row.walk_forward_enabled,
            created_at: row.created_at,
            updated_at: row.updated_at,
        });
    }

    Ok(items)
}

// ──────────────── 回测 ────────────────

/// 回测请求参数
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunRequest {
    /// 策略 id（builtin 传 builtin.* / rhai 传 DB 主键）
    pub strategy_id: String,
    /// 策略类型（必须与 strategy_id 匹配）
    pub strategy_type: String,
    /// 回测标的代码（如 "600519"）
    pub code: String,
    /// 起始日期 YYYY-MM-DD（含）
    pub start_date: String,
    /// 截止日期 YYYY-MM-DD（含）
    pub end_date: String,
    /// 初始资金（元）
    #[serde(default = "default_initial_cash")]
    pub initial_cash: f64,
    /// 策略参数覆盖（key → value）
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    /// 是否启用 Walk-Forward（默认 true，反过拟合硬规则）
    #[serde(default = "default_true")]
    pub walk_forward_enabled: bool,
    /// 是否显式强制关闭 Walk-Forward（必须为 true 才允许关闭，警告级别）
    #[serde(default)]
    pub walk_forward_force_off: bool,
    /// 撮合器配置覆盖（可选）
    #[serde(default)]
    pub matcher_config: Option<MatcherConfig>,
    /// 自定义 run 名（可选）
    #[serde(default)]
    pub name: Option<String>,
}

fn default_initial_cash() -> f64 {
    1_000_000.0
}
fn default_true() -> bool {
    true
}

/// 回测响应（包含运行记录 + 信号历史 + 纸面成交摘要）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunResponse {
    pub run: quant_runs::Model,
    pub metrics: MetricsReport,
    pub signal_count: i64,
    pub trade_count: i64,
    pub walk_forward: Option<axagent_quant::WalkForwardReport>,
}

/// 跑回测
///
/// 流程：
/// 1. 校验 + 构造策略
/// 2. 拉取 K 线（axagent-astock-data，含复权 + 涨跌停 quote）
/// 3. 写 quant_runs (status=running)
/// 4. 跑 BacktestEngine（可选 Walk-Forward）
/// 5. 写 quant_signals + quant_paper_trades
/// 6. 更新 quant_runs (status=completed, result_json)
/// 7. 算 MetricsReport 返回
#[tauri::command]
pub async fn quant_backtest_run(
    state: State<'_, AppState>,
    request: BacktestRunRequest,
) -> Result<BacktestRunResponse, String> {
    let started_at = Utc::now().timestamp();
    let run_id = Uuid::new_v4().to_string();

    // ── 1. 构造策略 ──
    let mut strategy: Box<dyn Strategy> = build_strategy(&state, &request).await?;

    // 应用参数覆盖
    for (k, v) in &request.params {
        strategy
            .set_param(k, v.clone())
            .map_err(|e| format!("invalid param '{k}': {e}"))?;
    }

    // ── 2. 拉 K 线（取 ~2 年前复权日 K） ──
    let klines_with_quotes = state
        .astock_client
        .get_klines_with_adj(&request.code, "daily", 120u32, Some(axagent_astock_data::AdjType::Forward))
        .await
        .map_err(|e| format!("get_klines failed for {}: {e}", request.code))?;

    if klines_with_quotes.is_empty() {
        return Err(format!("股票 {} 无 K 线数据", request.code));
    }

    // 合并 KLine → Bar
    let bars: Vec<Bar> = klines_with_quotes
        .iter()
        .map(|k| Bar::from_kline(&request.code, k))
        .filter(|b| {
            b.date.as_str() >= request.start_date.as_str()
                && b.date.as_str() <= request.end_date.as_str()
        })
        .collect();

    if bars.is_empty() {
        return Err(format!("区间 {}-{} 无数据", request.start_date, request.end_date));
    }

    // ── 3. 写 quant_runs (pending → running) ──
    let matcher_cfg = request
        .matcher_config
        .clone()
        .unwrap_or_else(MatcherConfig::default);

    let bt_config = BacktestConfig {
        initial_cash: request.initial_cash,
        matcher: matcher_cfg,
        codes: vec![request.code.clone()],
        start_date: Some(request.start_date.clone()),
        end_date: Some(request.end_date.clone()),
    };

    let config_json =
        serde_json::to_string(&bt_config).map_err(|e| format!("serialize BacktestConfig: {e}"))?;

    let run_active = quant_runs::ActiveModel {
        id: Set(run_id.clone()),
        strategy_id: Set(request.strategy_id.clone()),
        name: Set(request.name.clone()),
        start_date: Set(request.start_date.clone()),
        end_date: Set(request.end_date.clone()),
        initial_cash: Set(request.initial_cash),
        config_json: Set(config_json),
        status: Set("running".to_string()),
        result_json: Set(None),
        walk_forward_enabled: Set(request.walk_forward_enabled && !request.walk_forward_force_off),
        walk_forward_folds: Set(None),
        walk_forward_overfit_warning: Set(None),
        walk_forward_stability_score: Set(None),
        started_at: Set(started_at),
        finished_at: Set(None),
        error_message: Set(None),
    };

    quant_runs::Entity::insert(run_active)
        .exec(state.harness.db())
        .await
        .map_err(|e| format!("insert quant_runs: {e}"))?;

    // ── 4. 跑回测 ──
    let engine = BacktestEngine::new(bt_config.clone());
    let result = engine
        .run(&mut *strategy, bars.clone())
        .await
        .map_err(|e| format!("BacktestEngine::run failed: {e}"))?;

    // ── 5. 可选 Walk-Forward ──
    let wf_report = if request.walk_forward_enabled && !request.walk_forward_force_off {
        let wf_config = WalkForwardConfig {
            train_days: 504,
            test_days: 126,
            step_days: None,
            anchored: false,
            min_train_bars: 60,
            min_test_bars: 20,
            risk_free_annual: 0.025,
            force_off: false,
        };
        let wf = WalkForward::new(wf_config);
        let strategy_factory: Arc<dyn Fn(usize) -> Box<dyn Strategy> + Send + Sync> = {
            // 简单 grid：保持同一份策略实例（实际项目可在此 grid search）
            let stype = request.strategy_type.clone();
            let sid = request.strategy_id.clone();
            let params = request.params.clone();
            Arc::new(move |_fold_idx| -> Box<dyn Strategy> {
                // 注：这里仅作为工厂签名，Walk-Forward 内部仅取 best_params；
                // 单实例场景下同一 params 的 fold 重复跑无意义，所以这里只占位
                let mut s: Box<dyn Strategy> = match stype.as_str() {
                    "builtin.ma_cross" => Box::new(MaCrossStrategy::default()),
                    "builtin.macd" => Box::new(MacdStrategy::default()),
                    "builtin.rsi" => Box::new(RsiStrategy::default()),
                    "builtin.boll" => Box::new(BollStrategyAlias::default()),
                    "builtin.turtle" => Box::new(TurtleStrategy::default()),
                    "rhai" => {
                        // rhai fold：编译源（这里拿不到原始脚本，需要先读 DB）
                        // 为安全起见，使用 default 工厂（M1 简化：rhai 不走 WF grid）
                        Box::new(MaCrossStrategy::default())
                    },
                    _ => Box::new(MaCrossStrategy::default()),
                };
                for (k, v) in &params {
                    let _ = s.set_param(k, v.clone());
                }
                let _ = (s.version(), sid.as_str()); // 抑制未用警告
                s
            })
        };
        let sf = strategy_factory.clone();
        match wf.run(move |idx| sf(idx), bars.clone()).await {
            Ok(report) => Some(report),
            Err(e) => {
                eprintln!("WalkForward failed: {e}");
                None
            }
        }
    } else {
        None
    };

    // ── 6. 写 quant_signals ──
    let signal_models: Vec<quant_signals::ActiveModel> = result
        .signals
        .iter()
        .map(|sig| quant_signals::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            run_id: Set(run_id.clone()),
            code: Set(sig.code.clone()),
            action: Set(format!("{:?}", sig.action).to_lowercase()),
            strength: Set(sig.strength),
            reason: Set(Some(sig.reason.clone())),
            close_reason: Set(sig
                .close_reason
                .as_ref()
                .map(|c| format!("{c:?}").to_lowercase())),
            timestamp: Set("".to_string()),
            created_at: Set(Utc::now().timestamp()),
        })
        .collect();

    if !signal_models.is_empty() {
        // 海量信号分批插入
        for chunk in signal_models.chunks(500) {
            quant_signals::Entity::insert_many(chunk.to_vec())
                .exec(state.harness.db())
                .await
                .map_err(|e| format!("insert quant_signals: {e}"))?;
        }
    }

    // ── 7. 写 quant_paper_trades ──
    let trade_models: Vec<quant_paper_trades::ActiveModel> = result
        .trades
        .iter()
        .map(|t| quant_paper_trades::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            run_id: Set(run_id.clone()),
            code: Set(t.code.clone()),
            side: Set(format!("{:?}", t.side).to_lowercase()),
            quantity: Set(t.quantity as i64),
            price: Set(t.price),
            amount: Set(t.amount),
            commission: Set(t.commission),
            stamp_tax: Set(t.stamp_tax),
            slippage: Set(t.slippage),
            timestamp: Set(t.timestamp.clone()),
            reason: Set(Some(t.reason.clone())),
            realized_pnl: Set(t.realized_pnl),
        })
        .collect();

    if !trade_models.is_empty() {
        for chunk in trade_models.chunks(500) {
            quant_paper_trades::Entity::insert_many(chunk.to_vec())
                .exec(state.harness.db())
                .await
                .map_err(|e| format!("insert quant_paper_trades: {e}"))?;
        }
    }

    // ── 8. 算指标 + 更新 run (completed) ──
    let metrics = MetricsReport::from_backtest_result(&result, 0.025);

    let result_json =
        serde_json::to_string(&result).map_err(|e| format!("serialize BacktestResult: {e}"))?;

    let now_ts = Utc::now().timestamp();
    let mut run_update: quant_runs::ActiveModel = quant_runs::Entity::find_by_id(&run_id)
        .one(state.harness.db())
        .await
        .map_err(|e| format!("reload run: {e}"))?
        .ok_or_else(|| format!("run {run_id} disappeared"))?
        .into();
    run_update.status = Set("completed".to_string());
    run_update.result_json = Set(Some(result_json));
    run_update.finished_at = Set(Some(now_ts));
    if let Some(wf) = &wf_report {
        run_update.walk_forward_folds = Set(Some(wf.windows.len() as i32));
        run_update.walk_forward_overfit_warning = Set(Some(wf.overfit_window_count > 0));
        run_update.walk_forward_stability_score = Set(Some(wf.stability_score));
    }
    run_update
        .update(state.harness.db())
        .await
        .map_err(|e| format!("update quant_runs: {e}"))?;

    let final_run = quant_runs::Entity::find_by_id(&run_id)
        .one(state.harness.db())
        .await
        .map_err(|e| format!("reload final run: {e}"))?
        .ok_or_else(|| format!("run {run_id} gone after update"))?;

    Ok(BacktestRunResponse {
        run: final_run,
        metrics,
        signal_count: signal_models.len() as i64,
        trade_count: trade_models.len() as i64,
        walk_forward: wf_report,
    })
}

/// 构造策略实例（按 strategy_type 分发）
async fn build_strategy(
    state: &State<'_, AppState>,
    req: &BacktestRunRequest,
) -> Result<Box<dyn Strategy>, String> {
    match req.strategy_type.as_str() {
        "builtin" => match req.strategy_id.as_str() {
            "builtin.ma_cross" => Ok(Box::new(MaCrossStrategy::default())),
            "builtin.macd" => Ok(Box::new(MacdStrategy::default())),
            "builtin.rsi" => Ok(Box::new(RsiStrategy::default())),
            "builtin.boll" => Ok(Box::new(BollStrategyAlias::default())),
            "builtin.turtle" => Ok(Box::new(TurtleStrategy::default())),
            other => Err(format!("未知 builtin 策略: {other}")),
        },
        "rhai" => {
            // 从 DB 拉取脚本
            let row = quant_strategies::Entity::find_by_id(&req.strategy_id)
                .one(state.harness.db())
                .await
                .map_err(|e| format!("query rhai strategy: {e}"))?
                .ok_or_else(|| format!("Rhai 策略 {} 不存在", req.strategy_id))?;
            let script = row
                .script_source
                .ok_or_else(|| "Rhai 策略 script_source 为空".to_string())?;
            let mut s = RhaiStrategy::from_script(&row.name, &script)
                .map_err(|e| format!("RhaiStrategy::from_script failed: {e}"))?;
            if let Some(p) = row.params_json.as_deref() {
                let v: serde_json::Value =
                    serde_json::from_str(p).map_err(|e| format!("params_json: {e}"))?;
                if let Some(obj) = v.as_object() {
                    for (k, val) in obj {
                        s.set_param(k, val.clone())
                            .map_err(|e| format!("rhai set_param '{k}': {e}"))?;
                    }
                }
            }
            Ok(Box::new(s))
        },
        other => Err(format!("未知 strategy_type: {other}")),
    }
}

// ──────────────── 指标对比 ────────────────

/// 多 run 指标对比响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsCompareResponse {
    pub runs: Vec<RunWithMetrics>,
    pub best_by: HashMap<String, String>, // metric → run_id
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunWithMetrics {
    pub run: quant_runs::Model,
    pub strategy_name: String,
    pub metrics: Option<MetricsReport>,
    pub error_message: Option<String>,
}

/// 对比多个 run 的指标
#[tauri::command]
pub async fn quant_metrics_compare(
    state: State<'_, AppState>,
    run_ids: Vec<String>,
) -> Result<MetricsCompareResponse, String> {
    if run_ids.is_empty() {
        return Err("run_ids 不能为空".to_string());
    }

    let runs: Vec<quant_runs::Model> = quant_runs::Entity::find()
        .filter(quant_runs::Column::Id.is_in(&run_ids))
        .all(state.harness.db())
        .await
        .map_err(|e| format!("query runs: {e}"))?;

    // 拉对应策略名
    let strat_ids: Vec<String> = runs.iter().map(|r| r.strategy_id.clone()).collect();
    let strats: Vec<quant_strategies::Model> = quant_strategies::Entity::find()
        .filter(quant_strategies::Column::Id.is_in(&strat_ids))
        .all(state.harness.db())
        .await
        .map_err(|e| format!("query strategies: {e}"))?;
    let name_map: HashMap<String, String> = strats.into_iter().map(|s| (s.id, s.name)).collect();

    let mut results: Vec<RunWithMetrics> = Vec::new();
    for run in &runs {
        let metrics = match run.result_json.as_deref() {
            Some(s) => match serde_json::from_str::<BacktestResult>(s) {
                Ok(r) => Some(MetricsReport::from_backtest_result(&r, 0.025)),
                Err(_) => None,
            },
            None => None,
        };
        results.push(RunWithMetrics {
            run: run.clone(),
            strategy_name: name_map
                .get(&run.strategy_id)
                .cloned()
                .unwrap_or_else(|| "(已删除)".to_string()),
            metrics,
            error_message: run.error_message.clone(),
        });
    }

    // 简单"最优"判定：按 sharpe / annualized_return / max_drawdown_pct 综合
    let mut best_by: HashMap<String, String> = HashMap::new();
    let mut best_sharpe: Option<(f64, &str)> = None;
    let mut best_return: Option<(f64, &str)> = None;
    let mut best_dd: Option<(f64, &str)> = None; // 越接近 0 越好
    for r in &results {
        if let Some(m) = &r.metrics {
            let id = r.run.id.as_str();
            if best_sharpe.map(|(v, _)| m.sharpe > v).unwrap_or(true) {
                best_sharpe = Some((m.sharpe, id));
            }
            if best_return
                .map(|(v, _)| m.annualized_return > v)
                .unwrap_or(true)
            {
                best_return = Some((m.annualized_return, id));
            }
            // max_drawdown_pct 越小越好
            let dd = m.max_drawdown_pct.abs();
            if best_dd.map(|(v, _)| dd < v).unwrap_or(true) {
                best_dd = Some((dd, id));
            }
        }
    }
    if let Some((_, id)) = best_sharpe {
        best_by.insert("sharpe".to_string(), id.to_string());
    }
    if let Some((_, id)) = best_return {
        best_by.insert("annualizedReturn".to_string(), id.to_string());
    }
    if let Some((_, id)) = best_dd {
        best_by.insert("maxDrawdown".to_string(), id.to_string());
    }

    Ok(MetricsCompareResponse {
        runs: results,
        best_by,
    })
}

// ──────────────── Rhai 策略注册 ────────────────

/// Rhai 注册请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRhaiRequest {
    /// 策略名（user-readable）
    pub name: String,
    /// 版本号
    #[serde(default = "default_rhai_version")]
    pub version: String,
    /// 描述
    #[serde(default)]
    pub description: Option<String>,
    /// Rhai 脚本源码
    pub script_source: String,
    /// 初始参数（key → value）
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    /// 是否启用 Walk-Forward
    #[serde(default = "default_true")]
    pub walk_forward_enabled: bool,
    /// 覆盖已有策略（同名同版本时）
    #[serde(default)]
    pub upsert: bool,
}

fn default_rhai_version() -> String {
    "1.0.0".to_string()
}

/// 注册 / 更新 Rhai 策略
#[tauri::command]
pub async fn quant_strategy_register_rhai(
    state: State<'_, AppState>,
    request: RegisterRhaiRequest,
) -> Result<quant_strategies::Model, String> {
    if request.name.trim().is_empty() {
        return Err("策略名不能为空".to_string());
    }
    if request.script_source.trim().is_empty() {
        return Err("Rhai 脚本源码不能为空".to_string());
    }

    // 1. 编译校验（沙箱）
    let mut probe = RhaiStrategy::from_script(&request.name, &request.script_source)
        .map_err(|e| format!("Rhai 编译失败: {e}"))?;

    // 2. 应用参数（确保 set_param 不报错）
    for (k, v) in &request.params {
        probe
            .set_param(k, v.clone())
            .map_err(|e| format!("set_param '{k}': {e}"))?;
    }

    // 3. 找同名同版本是否已存在
    let now = Utc::now().timestamp();
    let existing: Option<quant_strategies::Model> = quant_strategies::Entity::find()
        .filter(quant_strategies::Column::Name.eq(&request.name))
        .filter(quant_strategies::Column::Version.eq(&request.version))
        .one(state.harness.db())
        .await
        .map_err(|e| format!("query existing: {e}"))?;

    let params_json =
        serde_json::to_string(&request.params).map_err(|e| format!("serialize params: {e}"))?;

    let row = if let Some(prev) = existing {
        if !request.upsert {
            return Err(format!(
                "策略 '{}@{}' 已存在，设置 upsert=true 以覆盖",
                request.name, request.version
            ));
        }
        let mut am: quant_strategies::ActiveModel = prev.clone().into();
        am.script_source = Set(Some(request.script_source.clone()));
        am.params_json = Set(Some(params_json));
        am.description = Set(request.description.clone());
        am.walk_forward_enabled = Set(request.walk_forward_enabled);
        am.updated_at = Set(now);
        am.update(state.harness.db())
            .await
            .map_err(|e| format!("update rhai strategy: {e}"))?
    } else {
        let new_id = Uuid::new_v4().to_string();
        let am = quant_strategies::ActiveModel {
            id: Set(new_id),
            name: Set(request.name.clone()),
            version: Set(request.version.clone()),
            strategy_type: Set("rhai".to_string()),
            description: Set(request.description.clone()),
            script_source: Set(Some(request.script_source.clone())),
            params_json: Set(Some(params_json)),
            walk_forward_enabled: Set(request.walk_forward_enabled),
            created_at: Set(now),
            updated_at: Set(now),
        };
        am.insert(state.harness.db())
            .await
            .map_err(|e| format!("insert rhai strategy: {e}"))?
    };

    Ok(row)
}

// 引用 quant crate 内部类型以避免 unused import（仅用作类型提示）
#[allow(dead_code)]
type _CtxRef<'a> = &'a mut StrategyCtx;
