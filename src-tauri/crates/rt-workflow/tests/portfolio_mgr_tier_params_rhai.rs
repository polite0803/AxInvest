//! 行为测试（v98 收口）：`portfolio-mgr.rhai` 的逐档止损/止盈**只能有一条通路** ——
//! `sl_pct_for` / `tp_pct_for`，且四档独立决策 `decisionsByHorizon` 必须经它们取值。
//!
//! 为什么锁这条（2026-09-29 自查发现，本轮我自己造成）：v97 把八项档位做成变量后，
//! 只接了**主决策**与 `horizonPriceMap`，而 `decisionsByHorizon` 仍另抄一份字面量表
//! （5/10/5、8/18/28、12/30/90 + 超短线 3/5）。后果不是「少个功能」而是**同屏两个读数**：
//! 用户在设置面板调档、反思建议改参 ⇒ 四档 Tab 纹丝不动，价位映射却已变 ⇒
//! 同一份 JSON 里 `stopLossPct` 与该档 `stopLoss` 反解互相打架。
//! 纯 `engine.compile` 门（`all_rhai_scripts_compile`）对此**完全无感** —— 字面量照样合法。
//!
//! ⚠ 本文件抽取脚本片段的**逐字副本**（同 `portfolio_mgr_veto_rhai.rs` 纪律），
//!   并在测试中断言源文件仍包含该副本 ⇒ 脚本漂移时本测试会红，而不是默默失效。
use rhai::Engine;

/// 与脚本第 22 行一致：`present` 只操作自身参数（`fn` 体读不到注入变量，见脚本头注释）。
const PRESENT_FN: &str = r#"
fn present(x) { type_of(x) != "()" }
"#;

/// 与 `portfolio-mgr.rhai`「逐周期止损/止盈档位」段逐字一致。
const TIER_PARAMS_SRC: &str = r#"
let sl_pct_ultra_short_v = if present(sl_pct_ultra_short) { sl_pct_ultra_short } else { 3.0 };
let sl_pct_short_v = if present(sl_pct_short) { sl_pct_short } else { 5.0 };
let sl_pct_mid_v = if present(sl_pct_mid) { sl_pct_mid } else { 8.0 };
let sl_pct_long_v = if present(sl_pct_long) { sl_pct_long } else { 12.0 };
let tp_pct_ultra_short_v = if present(tp_pct_ultra_short) { tp_pct_ultra_short } else { 5.0 };
let tp_pct_short_v = if present(tp_pct_short) { tp_pct_short } else { 10.0 };
let tp_pct_mid_v = if present(tp_pct_mid) { tp_pct_mid } else { 18.0 };
let tp_pct_long_v = if present(tp_pct_long) { tp_pct_long } else { 30.0 };
let sl_pct_for = |h| switch h {
    "ultra_short" => sl_pct_ultra_short_v,
    "short" => sl_pct_short_v,
    "mid" => sl_pct_mid_v,
    "long" => sl_pct_long_v,
    _ => 5.0,
};
let tp_pct_for = |h| switch h {
    "ultra_short" => tp_pct_ultra_short_v,
    "short" => tp_pct_short_v,
    "mid" => tp_pct_mid_v,
    "long" => tp_pct_long_v,
    _ => 10.0,
};
"#;

/// 输出四档各自的档位%，供断言。
const OUT: &str = r#"
#{
    "ultra_short": #{ "sl": sl_pct_for.call("ultra_short"), "tp": tp_pct_for.call("ultra_short") },
    "short": #{ "sl": sl_pct_for.call("short"), "tp": tp_pct_for.call("short") },
    "mid": #{ "sl": sl_pct_for.call("mid"), "tp": tp_pct_for.call("mid") },
    "long": #{ "sl": sl_pct_for.call("long"), "tp": tp_pct_for.call("long") },
}
"#;

/// 一档的读数：`(周期, 止损%, 止盈%)`。
type Tier = (String, f64, f64);

/// 与生产同构（`code_executor.rs`）：input_mapping → `push_constant` → `compile` → `eval_ast_with_scope`。
/// `tuned` 为空 = 八项全部注入 `unit`（V57 对 `present(x)` 名字的缺省填充），走脚本内默认值。
fn tiers(tuned: &[(&str, f64)]) -> Vec<Tier> {
    let engine = Engine::new();
    let script = format!("{PRESENT_FN}{TIER_PARAMS_SRC}{OUT}");
    let mut scope = rhai::Scope::new();
    for name in [
        "sl_pct_ultra_short",
        "sl_pct_short",
        "sl_pct_mid",
        "sl_pct_long",
        "tp_pct_ultra_short",
        "tp_pct_short",
        "tp_pct_mid",
        "tp_pct_long",
    ] {
        let v = tuned
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, x)| rhai::Dynamic::from(*x))
            .unwrap_or(rhai::Dynamic::UNIT);
        scope.push_constant(name, v);
    }
    let ast = engine.compile(&script).expect("档位段应可编译");
    let r = engine.eval_ast_with_scope::<rhai::Map>(&mut scope, &ast).expect("档位段应可求值");
    let mut out: Vec<Tier> = Vec::new();
    for k in ["ultra_short", "short", "mid", "long"] {
        let row = r
            .get(k)
            .unwrap_or_else(|| panic!("输出缺档位 {k}"))
            .clone()
            .try_cast::<rhai::Map>()
            .expect("档位应为 map");
        let g = |f: &str| {
            row.get(f)
                .and_then(|x| x.clone().try_cast::<f64>())
                .unwrap_or_else(|| panic!("档位 {k} 缺字段 {f}"))
        };
        out.push((k.to_string(), g("sl"), g("tp")));
    }
    out
}

/// 防漂移：脚本里的档位段与本文件副本必须逐字相同。
#[test]
fn tier_params_block_matches_source_verbatim() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    assert!(
        pm.contains(TIER_PARAMS_SRC.trim()),
        "portfolio-mgr.rhai 的逐档止损/止盈段与本测试副本已漂移，请同步"
    );
    assert!(pm.contains(PRESENT_FN.trim()), "present 定义已变，请同步本副本");
}

/// 缺省值必须仍是 3/5、5/10、8/18、12/30（与设置面板展示的默认值同源）。
#[test]
fn defaults_match_the_eight_seed_variables() {
    let got = tiers(&[]);
    let want =
        [("ultra_short", 3.0, 5.0), ("short", 5.0, 10.0), ("mid", 8.0, 18.0), ("long", 12.0, 30.0)];
    for (i, (t, sl, tp)) in got.iter().enumerate() {
        assert_eq!((t.as_str(), *sl, *tp), want[i], "档位默认值错档");
    }
}

/// 调档必须真的进档位函数（反思建议回写同一条通路）：只改中线一项，其余三档不受影响。
#[test]
fn injected_tier_params_take_effect_per_tier() {
    let got = tiers(&[("sl_pct_mid", 9.5), ("tp_pct_long", 25.0)]);
    let find = |k: &str| got.iter().find(|(t, _, _)| t.as_str() == k).expect("缺档");
    assert_eq!((find("mid").1, find("mid").2), (9.5, 18.0), "中线止损未随注入值变");
    assert_eq!((find("long").1, find("long").2), (12.0, 25.0), "长线止盈未随注入值变");
    assert_eq!((find("short").1, find("short").2), (5.0, 10.0), "未调档位被误联动");
    assert_eq!((find("ultra_short").1, find("ultra_short").2), (3.0, 5.0), "未调档位被误联动");
}

/// 核心锁：四档独立决策必须经 `sl_pct_for` / `tp_pct_for` / `days_for` 取值，
/// 且脚本里**不得再出现任何档位字面量调用形态**（v97 缺陷的复发方向就是抄回字面量）。
#[test]
fn decisions_by_horizon_reads_the_single_tier_source() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    let block_start = pm.find("let decisions_by_horizon = #{").expect("四档独立决策段应存在");
    let block = &pm[block_start..block_start + 900];
    for h in ["short", "mid", "long"] {
        for f in ["sl_pct_for", "tp_pct_for", "days_for"] {
            assert!(
                block.contains(&format!("{f}.call(\"{h}\")")),
                "decisionsByHorizon 的 {h} 档未经 {f} 取值 ⇒ 又抄回字面量表了"
            );
        }
    }
    // 反向锁：v97 之前的字面量形态不得回来。
    for lit in [
        "totalScore_short, 5.0, 10.0, 5",
        "totalScore_mid, 8.0, 18.0, 28",
        "totalScore_long, 12.0, 30.0, 90",
    ] {
        assert!(!pm.contains(lit), "脚本残留档位字面量抄本: {lit}");
    }
    // 超短线档同理走变量（其块内原先硬写 3.0/5.0）。
    assert!(
        pm.contains("\"stopLossPct\": if us_pos>0.0 { us_sl } else { 0.0 }")
            && pm.contains("\"takeProfitPct\": if us_pos>0.0 { us_tp } else { 0.0 }"),
        "超短线档应经 us_sl/us_tp（= sl_pct_for/tp_pct_for 的 ultra_short 取值）"
    );
    assert!(!pm.contains("\"stopLossPct\": if us_pos>0.0 { 3.0 }"), "超短线档不得再硬写 3.0");
}
