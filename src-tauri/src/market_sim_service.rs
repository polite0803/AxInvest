// SPDX-License-Identifier: AGPL-3.0-only
//! 市场仿真内核 —— **命令层与工作流侧共用的单一实现源**。
//!
//! ## 为什么住在 `src/` 顶层，而不是 `commands/` 下
//!
//! 本模块的两个入口有**两个命令层之外的消费者**：
//! 1. `commands/stock_workflow/sim_hook.rs` —— 决策落库后的自动仿真挂钩（默认路径）；
//! 2. `commands/stock_workflow/rhai_pm.rs` —— Rhai 宿主函数 `sim_run_mc` 的注册体。
//!
//! 它们原先各自写 `crate::commands::market_sim::run_mc_preset(…)`，于是
//! 「命令模块 A 横向调命令模块 B」—— 依赖图从树退化成网。分层门禁
//! `scripts/check-layer-discipline.mjs` 的 `commands-no-sibling-call` 拦的正是这个。
//! 把共享内核提到服务层后，依赖方向变成
//! `commands/* → market_sim_service`（垂向），横向边消失。
//!
//! ## 为什么错误码仍然来自 `commands::error_code`
//!
//! 错误码契约（`error.<code>` 的 11 语言对齐，见 `scripts/check-errorcode-alignment.mjs`）
//! 的真相源固定为 `commands/error_code.rs`，**不得搬移**。而 `error` / `error_code`
//! 已被门禁列为**共享基础设施模块**（`SHARED_INFRA_MODULES`）—— 即「被当公共库用」，
//! 不是命令业务模块。故此处 `use` 它们不构成反向依赖。
//!
//! ## 搬迁纪律
//!
//! 本文件内容自 `commands/market_sim.rs` **逐字搬迁**（非手抄），只有三处差异：
//! ① 补本文件头与 `use` 块；② 原文件里服务层不消费的 `use` 项未带过来；
//! ③ 原文件对它们的 `pub(crate)` 可见性保持不变。

use axagent_market_sim::{
    BEST_PARAMS, ExchangeAgent, MarketMakerAgent, MomentumAgent, NoiseAgent, SimConfig, ValueAgent,
    monte_carlo::{MonteCarloEngine, ScenarioConfig, ScenarioType},
};
use serde::{Deserialize, Serialize};

use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::stock_sim as sim_err;

/// 蒙特卡洛多场景模拟请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McSimRequest {
    pub stock_code: String,
    pub reference_price: i64,
    /// 最大模拟时间（纳秒），默认 50ms
    pub max_sim_time_ns: Option<u64>,
    /// 随机种子，默认 42
    pub seed: Option<u64>,
    /// 场景列表
    pub scenarios: Vec<McScenarioSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McScenarioSpec {
    pub scenario: String,
    pub paths: u32,
}

/// 蒙特卡洛模拟结果（前端展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McSimResult {
    pub stock_code: String,
    pub reference_price: i64,
    pub total_paths: usize,
    /// 跨场景上涨占比（终价高于参考价的场景数 / 有效场景数）。
    /// 由用户勾选的场景集合决定，不代表个股质地。
    pub survival_rate: f64,
    /// 场景一致性（变异系数 stddev / |mean|）。
    /// `null` = **不可判定** —— 各场景涨跌幅均值趋零时数学上无定义。
    /// 前端必须把 null 渲染为「无法判定」，不可当作 0（0 会被读成「高度一致」）。
    pub consistency_score: Option<f64>,
    pub best_scenario: String,
    pub worst_scenario: String,
    pub scenario_results: Vec<McScenarioResultItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McScenarioResultItem {
    pub scenario: String,
    pub label: String,
    pub paths: usize,
    pub avg_total_trades: f64,
    pub avg_final_mid_price: Option<f64>,
    pub price_change_pct: Option<f64>,
}

/// 蒙特卡洛多场景模拟的**核心实现**（单一权威源）。
///
/// 消费方三处，必须共用本函数 —— 否则「前端手动跑」与「工作流自动跑」会出现
/// 两套可能漂移的结论（历史教训：同一对比逻辑两套实现，数值随时间不再可比）：
/// 1. Tauri 命令 [`market_sim_run_mc`] —— 前端面板手动触发
/// 2. [`run_mc_preset`] —— 供「决策落库后自动仿真」挂钩（默认路径）与 Rhai
///    宿主函数 `sim_run_mc` 共用；前者见 `stock_workflow/sim_hook.rs`
pub(crate) fn run_mc_core(request: &McSimRequest) -> Result<McSimResult, String> {
    let ref_price = request.reference_price;
    let stock_code = request.stock_code.clone();
    let max_time_ns = request.max_sim_time_ns.unwrap_or(50_000_000);
    let seed = request.seed.unwrap_or(42);
    let scenarios = request.scenarios.clone();

    let default_agents = move |_seed: u64| -> Vec<Box<dyn axagent_market_sim::SimAgent>> {
        vec![
            Box::new(ExchangeAgent::with_tick_size("exchange", 1)),
            Box::new(MarketMakerAgent::new(
                "mm",
                BEST_PARAMS.mm_spread_bps,
                BEST_PARAMS.mm_quote_size,
                5000,
                0.1,
                200_000,
                ref_price,
            )),
            Box::new(MomentumAgent::new(
                "momentum",
                5,
                BEST_PARAMS.momentum_threshold,
                200,
                2000,
                500_000,
                ref_price as f64,
            )),
            Box::new(ValueAgent::new(
                "value",
                (ref_price as f64 * 1.02) as i64,
                30,
                300,
                3000,
                1_000_000,
            )),
            Box::new(NoiseAgent::new(
                "noise",
                300_000,
                BEST_PARAMS.noise_act_prob,
                50,
                BEST_PARAMS.noise_price_noise_bps,
                ref_price,
                _seed,
            )),
        ]
    };

    let config = SimConfig {
        max_time_ns,
        seed,
        stock_code: stock_code.clone(),
        reference_price: ref_price,
        tick_size: 1,
        ..Default::default()
    };

    let mut engine = MonteCarloEngine::new(config, default_agents);
    engine.scenarios = scenarios
        .iter()
        .map(|s| {
            let scenario_type = match s.scenario.as_str() {
                "bull" => ScenarioType::Bull,
                "bear" => ScenarioType::Bear,
                "flash_crash" => ScenarioType::FlashCrash,
                "high_vol" => ScenarioType::HighVolatility,
                _ => ScenarioType::Normal,
            };
            ScenarioConfig { scenario: scenario_type, paths: s.paths as usize }
        })
        .collect();

    let report = engine.run();

    Ok(McSimResult {
        stock_code: report.stock_code,
        reference_price: report.reference_price,
        total_paths: report.total_paths,
        survival_rate: (report.survival_rate * 1000.0).round() / 10.0,
        consistency_score: report.consistency_score,
        best_scenario: report.best_scenario,
        worst_scenario: report.worst_scenario,
        scenario_results: report
            .scenario_results
            .into_iter()
            .map(|sr| McScenarioResultItem {
                scenario: format!("{:?}", sr.scenario),
                label: sr.label,
                paths: sr.paths,
                avg_total_trades: (sr.avg_total_trades * 10.0).round() / 10.0,
                avg_final_mid_price: sr.avg_final_mid_price.map(|p| (p * 100.0).round() / 100.0),
                price_change_pct: sr.price_change_pct.map(|p| (p * 100.0).round() / 100.0),
            })
            .collect(),
    })
}

/// 把仿真失败原因序列化成 `ErrorResponse` 的 JSON 形状（`{code,category,detail?}`）。
///
/// 为什么复用 `ErrorResponse` 而不是自造 `{"error":"..."}`：
/// 失败值会写进 `blackboard_snapshot` 并直接喂给前端，而前端已有统一翻译层
/// （`src/lib/errorI18n.ts` 按 `t("error.${code}")` 查表）。产出**同一形状**
/// ⇒ 前端零特殊分支；自造形状则会让「仿真失败」成为第二套错误契约，
/// 且自由文本在非中文界面无法翻译（项目规范禁止，见 AGENTS.md「后端错误码 i18n 规范」）。
///
/// `detail` 只放**技术串**（变量名 + 值），不放面向用户的整句文案 ——
/// 用户看到的是 `error.${code}` 的翻译；`detail` 仅在前端缺译时兜底，
/// 以及用于日志排查。
fn sim_err_json(code: &str, category: ErrorCategory, detail: impl Into<String>) -> String {
    serde_json::to_string(&ErrorResponse::new(code).with_category(category).with_detail(detail))
        .unwrap_or_else(|_| format!(r#"{{"code":"{code}"}}"#))
}

/// 供 Rhai 宿主函数 `sim_run_mc` 调用的「按预设场景运行蒙特卡洛」便捷入口。
///
/// **返回 JSON 字符串而非结构体**：Rhai `register_fn` 对复杂/多泛型参数的跨层签名
/// 支持不稳（Rhai 1.25 已知问题：多 `Option<T>` 参数注册后不可调用，见
/// `rhai_pm.rs` 中 `pm_classify_risk` 的历史修复），而脚本侧已有现成的 `json_parse`。
/// 字符串是两侧的最小公共面。
///
/// `reference_price_yuan` 单位是**元**（与 `t-scoring` 的 `currentPrice` 口径一致），
/// 内部转「分」以匹配 `McSimRequest.reference_price`。
///
/// 失败不返回 Err（宿主函数签名受限），改返回 `ErrorResponse` 的 JSON 形状：
/// 让脚本能识别并优雅降级，使节点走 `continue_on_fail` 语义，而不是中断整条决策链。
/// 成功体含 `simOk` 字段，失败体含 `code` 字段 —— 这是两侧区分成功/失败的判据。
pub(crate) fn run_mc_preset(stock_code: &str, reference_price_yuan: f64, preset: &str) -> String {
    if stock_code.trim().is_empty() {
        return sim_err_json(sim_err::INPUT_MISSING, ErrorCategory::Validation, "stock_code 为空");
    }
    if !reference_price_yuan.is_finite() || reference_price_yuan <= 0.0 {
        return sim_err_json(
            sim_err::NO_REFERENCE_PRICE,
            ErrorCategory::Validation,
            format!("reference_price_yuan 无效: {reference_price_yuan}"),
        );
    }
    let ref_price_fen = (reference_price_yuan * 100.0).round() as i64;

    // 场景集合与前端面板默认档一致：normal/bull/bear 各 20 路径，闪崩/高波动各 15。
    let all_scenarios =
        [("normal", 20u32), ("bull", 20), ("bear", 20), ("flash_crash", 15), ("high_vol", 15)];
    // preset 控制纳入哪些场景：
    //   "stress"（默认）—— 全部场景，直接回答「最坏能坏到哪」，工作流自动运行用
    //   "base"          —— 仅 normal/bull/bear，省算力，供高频重跑
    let selected: Vec<McScenarioSpec> = all_scenarios
        .iter()
        .filter(|(k, _)| preset != "base" || !matches!(*k, "flash_crash" | "high_vol"))
        .map(|(k, p)| McScenarioSpec { scenario: (*k).to_string(), paths: *p })
        .collect();

    let request = McSimRequest {
        stock_code: stock_code.to_string(),
        reference_price: ref_price_fen,
        max_sim_time_ns: None,
        seed: None,
        scenarios: selected,
    };
    match run_mc_core(&request) {
        Ok(r) => serde_json::to_string(&r).unwrap_or_else(|e| {
            tracing::warn!("[market_sim] McSimResult 序列化失败: {e}");
            sim_err_json(
                sim_err::EXEC_FAILED,
                ErrorCategory::Unrecoverable,
                format!("结果序列化失败: {e}"),
            )
        }),
        Err(e) => {
            tracing::warn!("[market_sim] sim-verify 预设仿真失败: {e}");
            // detail 交给 serde_json 转义（不再手工 format! 拼 JSON）——
            // 手工拼接需要自己保证引号/换行转义，是结构被破坏的经典入口。
            sim_err_json(sim_err::EXEC_FAILED, ErrorCategory::Unrecoverable, e)
        },
    }
}
