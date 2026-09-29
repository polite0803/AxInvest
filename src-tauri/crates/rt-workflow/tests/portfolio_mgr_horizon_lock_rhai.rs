//! 行为测试（〇-B v2：四周期决策 100% 本地公式）：
//! `portfolio-mgr.rhai` 的「主周期定档 + 周期仓位乘数」必须是**纯确定性**的 ——
//! 既不接受用户入口锁档（v1 形态已撤除），也不采信 LLM 自报的 `timeHorizon`。
//!
//! 为什么锁这条：主档是落库列 `stock_analyses.decision_time_horizon` /
//! `decision_horizon_source` / `decision_expected_holding_days` 的来源，也是
//! **单周期反思**（〇-B 第 4 条）默认复盘的那一档。只要还留一条「采信模型自报」的
//! 旁路，同一份证据就能因模型措辞不同落到不同档，反思按天判成熟随之错档。
//!
//! 同时锁 Phase 3 的接线：定档段必须**在仓位之前**并产出 `horizon_position_mult`
//! —— 此前该乘数只存在于 `evidence_weight` 旁路（前端交叉验证），主决策从不消费。
//!
//! ⚠ 本文件抽取脚本片段的**逐字副本**（同 `portfolio_mgr_veto_rhai.rs` 纪律），
//!   并在测试中断言源文件仍包含该副本 ⇒ 脚本漂移时本测试会红，而不是默默失效。
use rhai::Engine;

/// 与 `portfolio-mgr.rhai` 定档段逐字一致（含天数闭包，乘数与天数同源于
/// `Period::decision_consts_map`）。
const PRIMARY_TIER_SRC: &str = r#"	let days_for = |h| horizon_const.call(h, "days");
	// ⚠ 这里**不做**四档 switch —— 四档各自的决策由下方 decisionsByHorizon 逐档产出，
	//   主档只是「本轮最值得看的那一档」的指针。
	let time_horizon = if effective_posterior >= 0.65 { "long" } else if effective_posterior >= 0.50 { "mid" } else if effective_posterior >= 0.35 { "short" } else { "ultra_short" };
	let horizon_source = "formula";
	// 周期仓位乘数：与 evidence_weight.rs 的 `compute_recommended_position` 消费**同一个**
	//   `Period::position_multiplier`（超短 0.6 / 短 0.8 / 中 1.0 / 长 1.2）。
	let horizon_position_mult = horizon_const.call(time_horizon, "mult");
	// 主档期望持有天数**只从权威表取**，不再采信 LLM 的自由数字 —— 否则同一决策里
	//   「哪个周期」是公式定的、「该周期多久」却是模型随口报的，反思按天判成熟即错档。
	let expected_holding_days = days_for.call(time_horizon);
"#;

/// 周期常量闭包（逐字副本，见 `portfolio_mgr_days_for_rhai.rs` 的同名副本）。
const HORIZON_CONST_CLOSURE: &str = r#"let horizon_const = |h, field| {
	    if type_of(horizon_consts_json) != "map" || horizon_consts_json.is_empty() {
	        throw "决策中止：缺少工作流变量 horizon_consts_json（周期常量表，权威源 crates/harness/src/holding_period.rs 的 Period::decision_consts_map，由 stock_workflow/hooks.rs 注入）";
	    }
	    let found = false;
	    for tier_key in horizon_consts_json.keys() {
	        if tier_key == h { found = true; }
	    }
	    if !found {
	        throw `决策中止：周期 '${h}' 不在周期常量表中（应为 ultra_short/short/mid/long 之一）`;
	    }
	    let row = horizon_consts_json[h];
	    if type_of(row) != "map" || type_of(row[field]) == "()" {
	        throw `决策中止：周期常量表缺字段 '${field}'（周期 ${h}）`;
	    }
	    row[field]
	};"#;

const OUT: &str = r#"
#{ "h": time_horizon, "s": horizon_source, "mult": horizon_position_mult, "days": expected_holding_days }
"#;

fn consts_map() -> rhai::Map {
    let mut m = rhai::Map::new();
    for (k, d, mult) in
        [("ultra_short", 2_i64, 0.6_f64), ("short", 5, 0.8), ("mid", 28, 1.0), ("long", 90, 1.2)]
    {
        let mut row = rhai::Map::new();
        row.insert("days".into(), rhai::Dynamic::from(d));
        row.insert("mult".into(), rhai::Dynamic::from(mult));
        m.insert(k.into(), rhai::Dynamic::from(row));
    }
    m
}

fn tier(effective_posterior: f64) -> (String, String, f64, i64) {
    let engine = Engine::new();
    // 顺序与生产一致：先建 const scope，再编译，再 eval_ast_with_scope。
    // ⚠ 模型自报值**故意也在 scope 里** —— 它必须影响不到结果（被撤除的旁路）。
    let mut scope = rhai::Scope::new();
    scope.push_constant("effective_posterior", rhai::Dynamic::from(effective_posterior));
    scope.push_constant("trader_time_horizon", rhai::Dynamic::from("short".to_string()));
    scope.push_constant("trader_holding_days", rhai::Dynamic::from(5.0_f64));
    scope.push_constant("horizon_consts_json", rhai::Dynamic::from(consts_map()));
    let script = format!("{HORIZON_CONST_CLOSURE}\n{PRIMARY_TIER_SRC}{OUT}");
    let ast = engine.compile_with_scope(&scope, &script).expect("定档段应可编译");
    let r = engine.eval_ast_with_scope::<rhai::Map>(&mut scope, &ast).expect("定档段应可求值");
    let g = |k: &str| {
        r.get(k)
            .and_then(|x| x.clone().try_cast::<String>())
            .unwrap_or_else(|| panic!("输出缺字段 {k}"))
    };
    let mult = r.get("mult").and_then(|x| x.clone().try_cast::<f64>()).expect("mult 应为浮点");
    let days = r.get("days").and_then(|x| x.clone().try_cast::<i64>()).expect("days 应为整数");
    (g("h"), g("s"), mult, days)
}

/// 防漂移：脚本里的定档段与本文件副本必须逐字相同。
#[test]
fn primary_tier_block_matches_source_verbatim() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    assert!(
        pm.contains(PRIMARY_TIER_SRC),
        "portfolio-mgr.rhai 的主周期定档段与本测试副本已漂移，请同步"
    );
    // 反向锁①：v1 的两条旁路不得回来
    assert!(
        !pm.contains("user_time_horizon") && !pm.contains("horizon_user_locked"),
        "v1 的用户锁档通路应已撤除，脚本里不得再出现 user_time_horizon / horizon_user_locked"
    );
    assert!(
        !pm.contains("} else if horizon_model_reported {"),
        "v1 的「采信 trader 自报定档」分支应已撤除"
    );
    // 反向锁②：定档必须在凯利仓位之前（乘数否则进不了主决策，Phase 3 就还是旁路）
    let tier_at = pm.find("let time_horizon = if effective_posterior").expect("定档段应存在");
    let kelly_at = pm.find("let base_position_pct = pm_kelly_position").expect("凯利段应存在");
    assert!(
        tier_at < kelly_at,
        "定档段必须在凯利仓位之前：周期乘数当前在 char {tier_at}，仓位在 {kelly_at}"
    );
    // 反向锁③：仓位必须真的消费周期量（而不是只算不用）。Phase D-2 后主口径是
    // **风险预算**，经验乘数只在 σ 不可得的降级分支存活 ⇒ 两条分支都逐字锁住，
    // 且锁 `position_source` 的两种取值：谁被用上了必须在输出里可反解。
    assert!(
        pm.contains(
            "\tlet position_pct = if type_of(main_risk_budget_pct) == \"()\" {\n\t\t\
             clamp(position_pct_raw * horizon_position_mult, 0.0, 95.0)\n\t\
             } else {\n\t\t\
             clamp(min(position_pct_raw, main_risk_budget_pct), 0.0, 95.0)\n\t};"
        ),
        "主决策仓位口径已漂移：risk_budget（min(凯利, 100·R/止损%)）与降级分支（凯利×周期乘数）必须逐字如上"
    );
    assert!(
        pm.contains("\"fallback_kelly_x_mult\"") && pm.contains("\"risk_budget\""),
        "仓位来源标注必须两值齐备 —— 缺一个就等于把降级伪装成主口径"
    );
    assert!(
        pm.contains("let hconf = pm_snr_confidence(heff, daysh, SNR_ANCHOR_DAYS);"),
        "「长线更值得」走判定侧 SNR √h，这行不在则收益侧周期优势又变回仓位乘数"
    );
}

/// 四档边界：阈值映射逐档命中，天数与乘数同时取自权威表，来源恒为 formula。
#[test]
fn four_tiers_map_deterministically() {
    for (p, want_h, want_mult, want_days) in [
        (0.70, "long", 1.2_f64, 90_i64),
        (0.65, "long", 1.2, 90),
        (0.55, "mid", 1.0, 28),
        (0.50, "mid", 1.0, 28),
        (0.40, "short", 0.8, 5),
        (0.35, "short", 0.8, 5),
        (0.20, "ultra_short", 0.6, 2),
    ] {
        let (h, s, mult, days) = tier(p);
        assert_eq!(
            (h.as_str(), s.as_str(), mult, days),
            (want_h, "formula", want_mult, want_days),
            "effective_posterior={p} 定档/乘数/天数错档"
        );
    }
}

/// 模型自报值在 scope 里也**不得**改变结果（v2「LLM 不参与定档」的正控）。
#[test]
fn llm_reported_horizon_cannot_move_the_tier() {
    // tier() 已注入 trader_time_horizon="short"、trader_holding_days=5；
    // 0.70 应落 long / 90 天，若被 LLM 值污染则会变成 short / 5 天。
    let (h, _, mult, days) = tier(0.70);
    assert_eq!((h.as_str(), mult, days), ("long", 1.2, 90), "模型自报值泄漏进了定档");
}

/// 同一后验两次求值必须同档（确定性；防止重新掺入 LLM 值或随机项）。
#[test]
fn same_posterior_same_tier() {
    assert_eq!(tier(0.52), tier(0.52));
}
