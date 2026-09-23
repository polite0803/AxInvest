// SPDX-License-Identifier: AGPL-3.0-only

//! portfolio 系列 Rhai 脚本依赖的 `pm_*` Rust 函数注册（函数体权威源）。
//!
//! ⚠ 本函数**不再被各入口直接调用**（2026-09-22 收敛）：唯一调用方是
//! `stock_workflow/rhai_registry.rs::register_axinvest_rhai_functions`
//! —— 那里是「AxInvest 需要哪些宿主函数」的唯一定义点，新增函数只改它。
//! 该单点入口再被两条路径消费：
//! 1. `init/services.rs::start_background_services` — 通过
//!    `register_shared_engine_initializer` 注入 rt-workflow 的
//!    `code_executor::shared_rhai_engine()`（DAG 主路径：portfolio-mgr /
//!    portfolio-risk-gate / data-quality 等 CodeNode 脚本）。
//! 2. `rhai_registry::build_stock_rhai_engine` — 供无法走共享 Engine 的入口
//!    （rerun 决策 `decision.rs`、What-If 回测 `commands/stock_analysis.rs`）
//!    构造独立 Engine。
//!
//! 历史 bug（2026-09-09 实证）：fork 清理「AxAgent 残留」时把
//! `register_portfolio_mgr_rhai_functions` 的调用整个移除，只留下 doc 注释，
//! 导致共享 Engine 上没有任何 `pm_*` 函数，data-quality 节点在
//! `pm_compute_factor_completeness` 处报 `Function not found` → VALIDATION_FAILED。
//!
//! 历史 bug 2（2026-09-22 实证）：恢复调用时写成**两处独立注册**（pm_* 与
//! bottleneck_* 各一次），而当时共享 Engine 的注册通道是单槽 `OnceLock` ⇒
//! 第二次被静默丢弃，`bottleneck_node_score` 从未生效。

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
    // ⚠️⚠️ 死映射警示（2026-09-13 实证）：本函数注册进引擎，但**全仓零调用** ——
    //   仓库内所有 `*.rhai` 均无 `pm_classify_risk` 出现，DB `workflow_templates`
    //   全表扫描（nodes/tool_defs）同样为 0（正对照 `pm_evidence_scale` 命中 1 条，
    //   证明检索本身有效）。生产运行时的风险分类走的是 `portfolio-mgr.rhai` 内联的
    //   同构镜像（阈值取面板参数 `RISK_*`，本函数则是硬编码 V54 值）。
    //   ⇒ **改这里不会改变任何一次运行的结论**。要改风险判据，改 `portfolio-mgr.rhai`。
    //   保留原因：它是风险判据唯一的语义单测载体（analysis-engine::portfolio_formula）。
    //   待裁决：删除（去重）还是把 rhai 改调本函数（统一真相源，但会失去面板调参）。
    //   v78(2026-09-14)：第 7 参 `sector` 已删除（原用于「金融业白名单豁免」），与
    //   `portfolio-mgr.rhai` 对齐 —— 风险判据不再读取任何行业标签。
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
    // 改为 9 个 Dynamic 参数（万能类型，接受 f64/i64/&str/unit），闭包内转 Option。
    // 2026-09-12: 由 10 参降为 9 参 —— 移除 f7「trader_direction」（data-quality 是
    // trader 的上游，该因子恒缺失，详见 portfolio_formula::compute_factor_completeness 文档）。
    engine.register_fn(
        "pm_compute_factor_completeness",
        |total_score: rhai::Dynamic,
         consensus_score: rhai::Dynamic,
         catalyst_level: rhai::Dynamic,
         risk_volatility: rhai::Dynamic,
         valuation_dcf_upside: rhai::Dynamic,
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

    // ── 仿真验证（工作流 `sim-verify` 节点）─────────────────────────────────
    // 供 `sim-verify.rhai` 调用：在**决策之后**自动跑蒙特卡洛压力测试。
    //
    // 为什么必须走宿主函数、不能在脚本里算：共享 Rhai 引擎设了
    // `set_max_operations(200_000)`（`rt-workflow/.../code_executor.rs`），
    // 而仿真要遍历「场景 × 路径 × 事件」，写成脚本必然超限。宿主函数内部是
    // Rust 循环，对 Rhai 只计 1 次操作。
    //
    // 形参一律用 `rhai::Dynamic` 而非具体类型，两个原因都不能省：
    //   ① `input_mapping` 注入的数字被统一转成 f64（`code_executor.rs` 的
    //      `Value::Number` 分支），若形参写 `i64` 会 Function not found；
    //   ② Rhai 1.25 对多 `Option<T>` / 泛型参数注册有已知缺陷（见上文
    //      `pm_classify_risk` 的历史修复）。类型转换放在函数体内是唯一稳的形态。
    engine.register_fn(
        "sim_run_mc",
        |stock_code: rhai::Dynamic,
         reference_price: rhai::Dynamic,
         preset: rhai::Dynamic|
         -> String {
            let code = stock_code.clone().into_string().unwrap_or_default();
            let price = reference_price
                .clone()
                .try_cast::<f64>()
                .or_else(|| reference_price.clone().try_cast::<i64>().map(|v| v as f64))
                .unwrap_or(0.0);
            let preset = preset.clone().into_string().unwrap_or_else(|_| "stress".to_string());
            crate::market_sim_service::run_mc_preset(&code, price, &preset)
        },
    );
}
