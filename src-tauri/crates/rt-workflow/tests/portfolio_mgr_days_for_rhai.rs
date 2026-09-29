//! 行为测试（Phase 0 → 〇-B v2）：`portfolio-mgr.rhai` 的周期常量读取
//! 必须**只来自注入的 `horizon_consts_json`**，缺表 / 缺档 / 缺字段一律显式失败。
//!
//! 背景：`days_for` 原先在脚本里手抄 `switch h { … _ => 5 }`（同一文件两处），
//! 未知周期静默落到短线 5 天 ⇒ 中/长线记录带着错档的期望持有期入库，反思据此
//! 判成熟即错档。现权威源 = `axagent_harness::holding_period::Period::decision_consts_map`，
//! 由 `stock_workflow/hooks.rs` 注入。
//!
//! ⚠ 两处纪律由本测试锁住：
//!   ① `horizon_const` **必须是闭包**（`let f = |…| {…}`）而非 `fn` —— 本仓 AST 由
//!     `engine.compile`（**无 scope**）生成，`fn` 体读不到工作流变量，而纯编译门
//!     `all_rhai_scripts_compile` **不会暴露**这一点。
//!     2026-09-28 同台实验（本文件历史版本曾内联此探针）：
//!       `fn f(){v.a}` + 裸 compile + eval_ast_with_scope ⇒ Variable not found
//!       `fn f(){v.a}` + eval_with_scope（不编译）                      ⇒ Ok
//!       `let g=||v.a` + const scope + eval_ast_with_scope              ⇒ Ok
//!       `fn f(){v.a}` + compile_with_scope + eval_ast_with_scope       ⇒ Ok
//!     ⇒ 结论：**只有闭包在任何编译形态下都稳**，故脚本侧统一用闭包。
//!   ② 副本与源文件逐字一致（同 `portfolio_mgr_veto_rhai.rs` 纪律）。
use rhai::Engine;

/// 与 `portfolio-mgr.rhai` 的周期常量闭包逐字一致。
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

const CALL: &str = r#"let days_for = |h| horizon_const.call(h, "days");
#{ "days": days_for.call(TIER), "mult": horizon_const.call(TIER, "mult") }"#;

fn script() -> String {
    format!("{HORIZON_CONST_CLOSURE}\n{CALL}")
}

/// 与 `hooks.rs` 注入形态一致：`Period::decision_consts_map()` → object → Rhai map of map。
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

/// const scope + AST + `eval_ast_with_scope`（与生产同一条执行三段式）。
fn read(tier: &str, inject: bool) -> Result<(i64, f64), String> {
    let engine = Engine::new();
    let mut scope = rhai::Scope::new();
    scope.push_constant("TIER", rhai::Dynamic::from(tier.to_string()));
    if inject {
        scope.push_constant("horizon_consts_json", rhai::Dynamic::from(consts_map()));
    }
    let ast = engine.compile_with_scope(&scope, script()).map_err(|e| e.to_string())?;
    let r = engine.eval_ast_with_scope::<rhai::Map>(&mut scope, &ast).map_err(|e| e.to_string())?;
    let days = r.get("days").and_then(|d| d.clone().try_cast::<i64>()).expect("days 应为整数");
    let mult = r.get("mult").and_then(|d| d.clone().try_cast::<f64>()).expect("mult 应为浮点");
    Ok((days, mult))
}

/// 防漂移 + 语言约束：闭包副本逐字一致，且不得退化为 `fn`。
#[test]
fn horizon_const_is_closure_and_matches_source_verbatim() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    assert!(
        pm.contains(HORIZON_CONST_CLOSURE),
        "portfolio-mgr.rhai 的 horizon_const 与本测试副本已漂移，请同步"
    );
    assert!(
        !pm.contains("fn horizon_const"),
        "horizon_const 必须是闭包：本仓 AST 无 scope 编译，`fn` 体读不到 horizon_consts_json"
    );
}

#[test]
fn four_tiers_read_days_and_multiplier() {
    for (t, d, m) in
        [("ultra_short", 2_i64, 0.6_f64), ("short", 5, 0.8), ("mid", 28, 1.0), ("long", 90, 1.2)]
    {
        let (gd, gm) = read(t, true).unwrap();
        assert_eq!((gd, gm), (d, m), "周期 {t} 的常量读取错档");
    }
}

/// 缺注入 ⇒ 必须报错，不得退回任何默认天数。
#[test]
fn missing_injection_fails_loudly() {
    let err = read("mid", false).expect_err("缺 horizon_consts_json 应当失败");
    assert!(err.contains("horizon_consts_json"), "诊断串应点名缺失变量: {err}");
}

/// 未知档位（含空串/脏值）⇒ 必须报错，不得静默落短线 5 天。
#[test]
fn unknown_tier_fails_instead_of_silent_5_days() {
    for t in ["", "medium", "ULTRA_SHORT_", "weekly"] {
        let err = read(t, true).expect_err("未知周期应失败");
        assert!(err.contains("不在周期常量表"), "诊断应点名错档周期 {t}: {err}");
    }
}

/// 缺字段 ⇒ 也要报错（防止注入面向但结构不完整，如只注 days 不注 mult）。
#[test]
fn missing_field_fails() {
    let engine = Engine::new();
    let mut scope = rhai::Scope::new();
    scope.push_constant("TIER", rhai::Dynamic::from("mid".to_string()));
    let mut row = rhai::Map::new();
    row.insert("days".into(), rhai::Dynamic::from(28_i64));
    let mut m = rhai::Map::new();
    m.insert("mid".into(), rhai::Dynamic::from(row));
    scope.push_constant("horizon_consts_json", rhai::Dynamic::from(m));
    let ast = engine.compile_with_scope(&scope, script()).expect("脚本应可编译");
    let err = engine
        .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &ast)
        .expect_err("缺 mult 字段应失败");
    assert!(err.to_string().contains("缺字段"), "诊断应点名缺失字段: {err}");
}
