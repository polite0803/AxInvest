// SPDX-License-Identifier: AGPL-3.0-only

//! portfolio 系列 Rhai 脚本依赖的 `pm_*` Rust 函数注册（单一权威源）。
//!
//! 消费方有两处，必须注册同一套函数：
//! 1. `init/services.rs::start_background_services` — 通过
//!    `register_shared_engine_initializer` 注入 rt-workflow 的
//!    `code_executor::shared_rhai_engine()`（DAG 主路径：portfolio-mgr /
//!    portfolio-risk-gate / data-quality 等 CodeNode 脚本）。
//! 2. `stock_workflow/decision.rs`（Rerun Decision 路径）— 自建本地 Engine 后调用本函数。
//!
//! 历史 bug（2026-09-09 实证）：fork 清理「AxAgent 残留」时把
//! `register_portfolio_mgr_rhai_functions` 的调用整个移除，只留下 doc 注释，
//! 导致共享 Engine 上没有任何 `pm_*` 函数，data-quality 节点在
//! `pm_compute_factor_completeness` 处报 `Function not found` → VALIDATION_FAILED。

use axagent_analysis_engine::portfolio_formula;
use rhai::Engine;

/// 把 portfolio 系列脚本依赖的全部 `pm_*` 函数注册到指定 Engine。
///
/// 纯数学/纯函数，无 DB / 无副作用；调用方负责沙箱限制（set_max_*）。
pub fn register_pm_functions(engine: &mut Engine) {
    engine.register_fn("pm_evidence_scale", |total_weight: f64, max_weight: f64| -> f64 {
        portfolio_formula::compute_evidence_scale(total_weight, max_weight)
    });
    engine.register_fn(
        "pm_kelly_position",
        |posterior: f64, odds: f64, cost_pct: f64, risk_level: &str| -> f64 {
            portfolio_formula::compute_kelly_position(posterior, odds, cost_pct, risk_level)
        },
    );
    engine.register_fn(
        "pm_classify_risk",
        |vol: rhai::Dynamic,
         sharpe: rhai::Dynamic,
         dd: rhai::Dynamic,
         roe: rhai::Dynamic,
         debt: rhai::Dynamic,
         growth: rhai::Dynamic|
         -> String {
            // P0 修复(2026-08-09): 原 6 个 Option<f64> 参数注册后不可调用（Rhai 1.25
            // 多 Option 参数闭包 Function not found），改为 Dynamic 参数内部转换。
            let f = |v: &rhai::Dynamic| -> Option<f64> {
                v.clone()
                    .try_cast::<f64>()
                    .or_else(|| v.clone().try_cast::<i64>().map(|x| x as f64))
            };
            portfolio_formula::classify_risk(
                f(&vol),
                f(&sharpe),
                f(&dd),
                f(&roe),
                f(&debt),
                f(&growth),
            )
        },
    );
    engine.register_fn("pm_risk_bias", |risk_level: &str| -> f64 {
        portfolio_formula::compute_risk_bias(risk_level)
    });
    engine.register_fn("pm_risk_veto", |action: &str, risk_level: &str| -> String {
        let (new_action, _, _) = portfolio_formula::apply_risk_veto(action, risk_level);
        new_action
    });
    engine.register_fn(
        "pm_covariance_decay",
        |f1_w: f64, f3_w: f64, f9_w: f64, f11_w: f64, decay_target: &str| -> f64 {
            let (f9, f11) = portfolio_formula::apply_covariance_decay(f1_w, f3_w, f9_w, f11_w);
            match decay_target {
                "f9" => f9,
                "f11" => f11,
                _ => 0.0,
            }
        },
    );
    // P0: 贝叶斯因子置信度（基于 prior→posterior 的证据强度）
    engine.register_fn("pm_compute_bayes_confidence", |prior: f64, posterior: f64| -> f64 {
        portfolio_formula::compute_bayes_confidence(prior, posterior)
    });
    // 因子数据完整度：供 data-quality.rhai 评估因子层数据完整度
    // P0 修复(2026-08-09): Rhai 1.25 的 register_fn 对含多个 Option<T> 参数的闭包
    // 注册后无法调用（全 Some/全 None/混合均报 Function not found，已实测确认），
    // 改为 10 个 Dynamic 参数（万能类型，接受 f64/i64/&str/unit），闭包内转 Option。
    engine.register_fn(
        "pm_compute_factor_completeness",
        |total_score: rhai::Dynamic,
         consensus_score: rhai::Dynamic,
         catalyst_level: rhai::Dynamic,
         risk_volatility: rhai::Dynamic,
         valuation_dcf_upside: rhai::Dynamic,
         trader_direction: rhai::Dynamic,
         money_flow_main_net_inflow: rhai::Dynamic,
         lockup_shareholder_trades_len: rhai::Dynamic,
         announcements_len: rhai::Dynamic,
         pace_signal: rhai::Dynamic|
         -> f64 {
            // Rhai Dynamic 数值提取：f64/i64 均接受，unit/其他 → None
            let f = |v: &rhai::Dynamic| -> Option<f64> {
                v.clone()
                    .try_cast::<f64>()
                    .or_else(|| v.clone().try_cast::<i64>().map(|x| x as f64))
            };
            // into_string: ImmutableString/String → String，unit 报错 → None
            let s = |v: &rhai::Dynamic| v.clone().into_string().ok();
            let i = |v: &rhai::Dynamic| v.clone().try_cast::<i64>();
            portfolio_formula::compute_factor_completeness(
                f(&total_score),
                f(&consensus_score),
                s(&catalyst_level).as_deref(),
                f(&risk_volatility),
                f(&valuation_dcf_upside),
                s(&trader_direction).as_deref(),
                f(&money_flow_main_net_inflow),
                i(&lockup_shareholder_trades_len),
                i(&announcements_len),
                f(&pace_signal),
            )
        },
    );
    // P1-E13: 组合风控门（portfolio-risk-gate.rhai 调用）。
    // 同样遵循 Dynamic 万能参数模式：target_price / stock_sector 在 Rhai 侧
    // 可能是 unit（safe_opt_* 兜底），闭包内转 Option。
    engine.register_fn(
        "pm_portfolio_risk_gate",
        |pm_action: &str,
         pm_position_pct: f64,
         pm_risk_level: &str,
         current_price: f64,
         target_price: rhai::Dynamic,
         stock_code: &str,
         stock_sector: rhai::Dynamic,
         holdings_json: &str,
         portfolio_cash: f64|
         -> String {
            let target = target_price
                .clone()
                .try_cast::<f64>()
                .or_else(|| target_price.clone().try_cast::<i64>().map(|x| x as f64));
            let sector =
                stock_sector.clone().try_cast::<rhai::ImmutableString>().map(|s| s.to_string());
            portfolio_formula::portfolio_risk_gate(
                pm_action,
                pm_position_pct,
                pm_risk_level,
                current_price,
                target,
                stock_code,
                sector.as_deref(),
                holdings_json,
                portfolio_cash,
            )
        },
    );
    // V66 修复(2026-07-29): 当前 portfolio-mgr.rhai 虽用本地词典未实际调用这两个函数，
    // 但保持注册对称可避免未来启用调用时 panic。
    engine.register_fn("pm_compute_news_sentiment", |title: &str, summary: &str| -> f64 {
        axagent_astock_data::sentiment::compute_news_sentiment(title, summary).unwrap_or(0.0)
    });
    engine.register_fn("pm_compute_text_sentiment", |text: &str| -> f64 {
        axagent_astock_data::sentiment::compute_text_sentiment(text).unwrap_or(0.0)
    });
}
