//! Backward-compat shim — delegates to `migrations::run_migrations`.
//!
//! 历史背景：v001 之前 dao crate 启动时跑这个函数 DROP `seaql_migrations` +
//! 全量 DDL，每次启动都重建。Phase 2 Task 2.1 引入 versioned migration
//! 框架（`migrations` 模块），不再有"全量重建"语义。
//!
//! 为避免破坏所有现有 call sites（`db::create_pool` / 测试 / 第三方
//! 集成），这里保留同名函数但只做转发。内部 DROP `seaql_migrations`
//! 行为已删除（无 seaql 依赖）；所有 schema 演进都走 `migrations`。
//!
//! 注：原签名 `&impl ConnectionTrait` 已变更为 `&DatabaseConnection`，
//! 因为 `migrations::run_migrations` 的内部 `up()` 函数需要
//! `&DatabaseConnection`（不可能把 `&impl ConnectionTrait` upcast 成
//! `&DatabaseConnection`——sea_orm 没有提供 back-reference）。所有 call
//! sites 传的都是 `&DatabaseConnection`，所以这个变化不破坏兼容性。

use sea_orm::ConnectionTrait;
use sea_orm::DatabaseConnection;
use sea_orm::DbErr;

/// 旧 API：执行所有数据表 DDL（幂等，适用新/旧数据库）。
///
/// 实际行为已变更为：补跑所有未应用的 schema 迁移。多次调用幂等，
/// 重启安全。之后运行 AxInvest 股票分析专用 DDL。
pub async fn run_initialization(db: &DatabaseConnection) -> Result<(), DbErr> {
    crate::migrations::run_migrations(db).await?;

    // ========================================================================
    // SECTION J: AxInvest — Stock Analysis tables
    // ========================================================================

    for sql in &[
        "CREATE TABLE IF NOT EXISTS stock_analyses (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            analysis_date TEXT NOT NULL, provider_id TEXT NOT NULL, conversation_id TEXT NOT NULL, \
            status TEXT NOT NULL, decision_action TEXT, decision_position_pct REAL, \
            decision_reasoning TEXT, decision_json TEXT, blackboard_snapshot TEXT, \
            config_id TEXT, \
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS watchlist_items (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            notes TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS portfolio_holdings (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            shares REAL NOT NULL DEFAULT 0, cost_price REAL NOT NULL DEFAULT 0, \
            current_price REAL, market_value REAL, profit_loss REAL, profit_loss_pct REAL, \
            notes TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS analysis_schedules (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            cron_expression TEXT NOT NULL, provider_id TEXT NOT NULL, \
            is_enabled INTEGER NOT NULL DEFAULT 1, last_run_at INTEGER, next_run_at INTEGER, \
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS price_alerts (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            condition TEXT NOT NULL, target_price REAL NOT NULL, \
            is_triggered INTEGER NOT NULL DEFAULT 0, triggered_at INTEGER, \
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS trades (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            direction TEXT NOT NULL, price REAL NOT NULL, quantity INTEGER NOT NULL, \
            trade_date TEXT NOT NULL, trade_time TEXT NOT NULL, \
            fee REAL, realized_pnl REAL, notes TEXT, \
            strategy TEXT, \
            created_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS reco_picks (\
            id TEXT NOT NULL PRIMARY KEY, generated_at TEXT NOT NULL, \
            period TEXT NOT NULL, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            style TEXT NOT NULL, confidence INTEGER NOT NULL DEFAULT 0, \
            synthetic INTEGER NOT NULL DEFAULT 0, \
            seed_pool_json TEXT, created_at TEXT NOT NULL)",
        // ========================================================================
        // SECTION L: AxInvest — Quant (量化交易 + 量化回测)
        // ========================================================================
        "CREATE TABLE IF NOT EXISTS quant_strategies (\
            id TEXT NOT NULL PRIMARY KEY, name TEXT NOT NULL, version TEXT NOT NULL DEFAULT '1.0.0', \
            strategy_type TEXT NOT NULL DEFAULT 'builtin', description TEXT, \
            script_source TEXT, params_json TEXT, \
            walk_forward_enabled INTEGER NOT NULL DEFAULT 1, \
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
            UNIQUE(name, version))",
        "CREATE TABLE IF NOT EXISTS quant_runs (\
            id TEXT NOT NULL PRIMARY KEY, strategy_id TEXT NOT NULL, name TEXT, \
            start_date TEXT NOT NULL, end_date TEXT NOT NULL, \
            initial_cash REAL NOT NULL DEFAULT 1000000.0, \
            config_json TEXT NOT NULL DEFAULT '{}', \
            status TEXT NOT NULL DEFAULT 'pending', \
            result_json TEXT, \
            walk_forward_enabled INTEGER NOT NULL DEFAULT 0, \
            walk_forward_folds INTEGER, \
            walk_forward_overfit_warning INTEGER, \
            walk_forward_stability_score REAL, \
            started_at INTEGER NOT NULL, finished_at INTEGER, \
            error_message TEXT, \
            FOREIGN KEY (strategy_id) REFERENCES quant_strategies(id) ON DELETE CASCADE)",
        "CREATE TABLE IF NOT EXISTS quant_signals (\
            id TEXT NOT NULL PRIMARY KEY, run_id TEXT NOT NULL, \
            code TEXT NOT NULL, action TEXT NOT NULL, strength REAL NOT NULL DEFAULT 0.5, \
            reason TEXT, close_reason TEXT, \
            timestamp TEXT NOT NULL, created_at INTEGER NOT NULL, \
            FOREIGN KEY (run_id) REFERENCES quant_runs(id) ON DELETE CASCADE)",
        "CREATE TABLE IF NOT EXISTS quant_paper_trades (\
            id TEXT NOT NULL PRIMARY KEY, run_id TEXT NOT NULL, \
            code TEXT NOT NULL, side TEXT NOT NULL, \
            quantity INTEGER NOT NULL, price REAL NOT NULL, amount REAL NOT NULL, \
            commission REAL NOT NULL DEFAULT 0, stamp_tax REAL NOT NULL DEFAULT 0, \
            slippage REAL NOT NULL DEFAULT 0, \
            timestamp TEXT NOT NULL, reason TEXT, realized_pnl REAL NOT NULL DEFAULT 0, \
            FOREIGN KEY (run_id) REFERENCES quant_runs(id) ON DELETE CASCADE)",
    ] {
        db.execute_unprepared(sql).await?;
    }

    // --- R1: 复盘→进化闭环（strategy_performance + strategy_weight_history） ---
    for sql in &[
        "CREATE TABLE IF NOT EXISTS strategy_performance (\
            id TEXT NOT NULL PRIMARY KEY, strategy_id TEXT NOT NULL, period TEXT NOT NULL, \
            stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            decision_at INTEGER NOT NULL, exit_at INTEGER NOT NULL, holding_days INTEGER NOT NULL, \
            return_pct REAL NOT NULL, was_correct INTEGER NOT NULL, \
            decision_confidence INTEGER NOT NULL, horizon_pnl_json TEXT, \
            created_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS strategy_weight_history (\
            id TEXT NOT NULL PRIMARY KEY, strategy_id TEXT NOT NULL, period TEXT NOT NULL, \
            old_weight REAL NOT NULL, new_weight REAL NOT NULL, delta_pct REAL NOT NULL, \
            trigger TEXT NOT NULL, source_reflection_id TEXT, sample_size INTEGER NOT NULL, \
            win_rate REAL NOT NULL, rationale TEXT, applied_at INTEGER NOT NULL)",
    ] {
        db.execute_unprepared(sql).await?;
    }

    // --- R2: 组合监控（portfolio_metrics_daily + portfolio_correlation_snapshot） ---
    for sql in &[
        "CREATE TABLE IF NOT EXISTS portfolio_metrics_daily (\
            id TEXT NOT NULL PRIMARY KEY, snapshot_date TEXT NOT NULL, \
            total_market_value REAL NOT NULL, cash_pct REAL NOT NULL, \
            total_pnl REAL NOT NULL, total_pnl_pct REAL NOT NULL, \
            max_drawdown_pct REAL NOT NULL, \
            beta REAL, sharpe_30d REAL, correlation_avg REAL, \
            top_concentration_pct REAL NOT NULL, \
            sector_exposure_json TEXT NOT NULL, stress_test_json TEXT, \
            created_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS portfolio_correlation_snapshot (\
            id TEXT NOT NULL PRIMARY KEY, snapshot_date TEXT NOT NULL, \
            lookback_days INTEGER NOT NULL, \
            code_a TEXT NOT NULL, code_b TEXT NOT NULL, \
            correlation REAL NOT NULL, created_at INTEGER NOT NULL)",
    ] {
        db.execute_unprepared(sql).await?;
    }

    // --- R3: 数据层（financial_snapshots + earnings_events） ---
    for sql in &[
        "CREATE TABLE IF NOT EXISTS financial_snapshots (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, \
            snapshot_date TEXT NOT NULL, \
            pe_ttm REAL, pb REAL, ps_ttm REAL, pcf REAL, ev_ebitda REAL, \
            roe REAL, gross_margin REAL, debt_ratio REAL, \
            revenue_yoy REAL, profit_yoy REAL, \
            source TEXT, created_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS earnings_events (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            event_date TEXT NOT NULL, event_type TEXT NOT NULL, \
            period TEXT, detail TEXT, source TEXT, created_at INTEGER NOT NULL)",
    ] {
        db.execute_unprepared(sql).await?;
    }

    // --- schema_migrations 迁移追踪表（已应用迁移版本） ---
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS schema_migrations (\
            id TEXT NOT NULL PRIMARY KEY, description TEXT NOT NULL, \
            applied_at INTEGER NOT NULL)",
    )
    .await?;
    let migration_ts = chrono::Local::now().timestamp();
    for (id, desc) in &[
        ("p0_init", "P0 主框架初始化（所有基础表）"),
        (
            "r1_2026_06_10",
            "R1 复盘→进化闭环（strategy_performance/strategy_weight_history）",
        ),
        (
            "r2_2026_06_10",
            "R2 组合监控（portfolio_metrics_daily/portfolio_correlation_snapshot）",
        ),
        ("r3_2026_06_10", "R3 数据层（financial_snapshots/earnings_events）"),
        (
            "m1_2026_06_11",
            "M1 量化交易+量化回测（quant_strategies/runs/signals/paper_trades）",
        ),
    ] {
        db.execute_unprepared(&format!(
            "INSERT OR IGNORE INTO schema_migrations(id, description, applied_at) \
             VALUES('{id}', '{desc}', {migration_ts})",
        ))
        .await?;
    }

    // --- Time-travel mode: stock_analyses 扩展字段（幂等） ---
    for sql in &[
        "ALTER TABLE stock_analyses ADD COLUMN analysis_kind TEXT NOT NULL DEFAULT 'live'",
        "ALTER TABLE stock_analyses ADD COLUMN as_of_date TEXT",
        "ALTER TABLE stock_analyses ADD COLUMN model_version TEXT",
        "ALTER TABLE stock_analyses ADD COLUMN data_snapshot_id TEXT",
        "ALTER TABLE stock_analyses ADD COLUMN outcome TEXT DEFAULT 'pending'",
        "ALTER TABLE stock_analyses ADD COLUMN decision_time_horizon TEXT",
        "ALTER TABLE stock_analyses ADD COLUMN decision_expected_holding_days INTEGER",
        "ALTER TABLE reco_picks ADD COLUMN strategy_weights_json TEXT",
        "ALTER TABLE trades ADD COLUMN strategy TEXT",
    ] {
        let _ = db.execute_unprepared(sql).await;
    }

    // --- 反思复盘表 ---
    let _ = db
        .execute_unprepared(
            "CREATE TABLE IF NOT EXISTS stock_reflections (\
            id TEXT NOT NULL PRIMARY KEY, stock_code TEXT NOT NULL, stock_name TEXT NOT NULL, \
            original_analysis_id TEXT NOT NULL, \
            as_of_date TEXT NOT NULL, hindsight_date TEXT NOT NULL, \
            min_confidence_threshold INTEGER NOT NULL DEFAULT 0, \
            reflection_depth TEXT NOT NULL DEFAULT 'light', \
            actual_outcome TEXT NOT NULL, \
            what_went_wrong TEXT, missed_signals TEXT, fix_for_future TEXT, \
            decision_json TEXT, blackboard_snapshot TEXT, \
            model_version TEXT, status TEXT NOT NULL DEFAULT 'completed', \
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .await;
    let _ = db
        .execute_unprepared(
            "ALTER TABLE stock_reflections ADD COLUMN parameter_suggestions_json TEXT",
        )
        .await;

    // --- Time-travel mode: market_data_history L2 cache 表 ---
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS market_data_history (\
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT, \
            vendor TEXT NOT NULL, method TEXT NOT NULL, stock_code TEXT NOT NULL DEFAULT '', \
            as_of_date TEXT NOT NULL, data_window_start TEXT, data_window_end TEXT, \
            payload_json TEXT NOT NULL, payload_hash TEXT NOT NULL, \
            fetched_at INTEGER NOT NULL, last_accessed_at INTEGER NOT NULL, \
            access_count INTEGER NOT NULL DEFAULT 0, expires_at INTEGER)",
    )
    .await?;
    db.execute_unprepared(
        "CREATE INDEX IF NOT EXISTS idx_mdh_lookup ON market_data_history(\
             vendor, method, stock_code, as_of_date, data_window_end)",
    )
    .await?;
    db.execute_unprepared(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_mdh_unique ON market_data_history(\
             vendor, method, stock_code, as_of_date, payload_hash)",
    )
    .await?;

    // --- Time-travel mode: replay_runs 元数据表 ---
    db.execute_unprepared(
        "CREATE TABLE IF NOT EXISTS replay_runs (\
            id TEXT NOT NULL PRIMARY KEY, name TEXT, \
            stock_codes TEXT NOT NULL, as_of_dates TEXT NOT NULL, \
            config_id TEXT, created_at INTEGER NOT NULL, completed_at INTEGER, \
            summary_json TEXT)",
    )
    .await?;

    // --- AxInvest indexes ---
    for sql in &[
        "CREATE INDEX IF NOT EXISTS idx_stock_analyses_code ON stock_analyses(stock_code)",
        "CREATE INDEX IF NOT EXISTS idx_stock_analyses_status ON stock_analyses(status)",
        "CREATE INDEX IF NOT EXISTS idx_stock_analyses_kind ON stock_analyses(analysis_kind, as_of_date)",
        "CREATE INDEX IF NOT EXISTS idx_stock_analyses_as_of ON stock_analyses(as_of_date)",
        "CREATE INDEX IF NOT EXISTS idx_mdh_accessed ON market_data_history(last_accessed_at)",
        "CREATE INDEX IF NOT EXISTS idx_mdh_expires ON market_data_history(expires_at)",
        "CREATE INDEX IF NOT EXISTS idx_replay_runs_created ON replay_runs(created_at)",
        "CREATE INDEX IF NOT EXISTS idx_strategy_perf_strategy ON strategy_performance(strategy_id, period, exit_at)",
        "CREATE INDEX IF NOT EXISTS idx_strategy_perf_code ON strategy_performance(stock_code, exit_at)",
        "CREATE INDEX IF NOT EXISTS idx_strategy_weight_hist_applied ON strategy_weight_history(strategy_id, period, applied_at)",
        "CREATE INDEX IF NOT EXISTS idx_portfolio_metrics_date ON portfolio_metrics_daily(snapshot_date)",
        "CREATE INDEX IF NOT EXISTS idx_portfolio_corr_date ON portfolio_correlation_snapshot(snapshot_date, lookback_days)",
        "CREATE INDEX IF NOT EXISTS idx_financial_snapshots_code_date ON financial_snapshots(stock_code, snapshot_date)",
        "CREATE INDEX IF NOT EXISTS idx_earnings_events_code_date ON earnings_events(stock_code, event_date)",
        "CREATE INDEX IF NOT EXISTS idx_earnings_events_date ON earnings_events(event_date)",
        "CREATE INDEX IF NOT EXISTS idx_quant_strategies_name ON quant_strategies(name)",
        "CREATE INDEX IF NOT EXISTS idx_quant_strategies_type ON quant_strategies(strategy_type)",
        "CREATE INDEX IF NOT EXISTS idx_quant_runs_strategy ON quant_runs(strategy_id, started_at DESC)",
        "CREATE INDEX IF NOT EXISTS idx_quant_runs_status ON quant_runs(status, started_at DESC)",
        "CREATE INDEX IF NOT EXISTS idx_quant_signals_run ON quant_signals(run_id, timestamp)",
        "CREATE INDEX IF NOT EXISTS idx_quant_signals_code_action ON quant_signals(code, action, timestamp)",
        "CREATE INDEX IF NOT EXISTS idx_quant_paper_trades_run ON quant_paper_trades(run_id, timestamp)",
        "CREATE INDEX IF NOT EXISTS idx_quant_paper_trades_code ON quant_paper_trades(code, timestamp)",
    ] {
        db.execute_unprepared(sql).await?;
    }

    Ok(())
}
