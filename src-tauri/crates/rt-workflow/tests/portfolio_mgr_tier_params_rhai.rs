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
"#;

/// Phase D 起固定百分比档退为「σ 不可得时的兜底」，纯「档 → 值」映射；
/// 与脚本 `sl_pct_fallback_for` / `tp_pct_fallback_for` 逐字一致。
const TIER_FALLBACK_SRC: &str = r#"
let sl_pct_fallback_for = |h| switch h {
    "ultra_short" => sl_pct_ultra_short_v,
    "short" => sl_pct_short_v,
    "mid" => sl_pct_mid_v,
    "long" => sl_pct_long_v,
    _ => 5.0,
};
let tp_pct_fallback_for = |h| switch h {
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
    "ultra_short": #{ "sl": sl_pct_fallback_for.call("ultra_short"), "tp": tp_pct_fallback_for.call("ultra_short") },
    "short": #{ "sl": sl_pct_fallback_for.call("short"), "tp": tp_pct_fallback_for.call("short") },
    "mid": #{ "sl": sl_pct_fallback_for.call("mid"), "tp": tp_pct_fallback_for.call("mid") },
    "long": #{ "sl": sl_pct_fallback_for.call("long"), "tp": tp_pct_fallback_for.call("long") },
}
"#;

/// 一档的读数：`(周期, 止损%, 止盈%)`。
type Tier = (String, f64, f64);

/// 与生产同构（`code_executor.rs`）：input_mapping → `push_constant` → `compile` → `eval_ast_with_scope`。
/// `tuned` 为空 = 八项全部注入 `unit`（V57 对 `present(x)` 名字的缺省填充），走脚本内默认值。
fn tiers(tuned: &[(&str, f64)]) -> Vec<Tier> {
    let engine = Engine::new();
    let script = format!("{PRESENT_FN}{TIER_PARAMS_SRC}{TIER_FALLBACK_SRC}{OUT}");
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
        "portfolio-mgr.rhai 的八项档位缺省段与本测试副本已漂移，请同步"
    );
    assert!(
        pm.contains(TIER_FALLBACK_SRC.trim()),
        "portfolio-mgr.rhai 的兜底档位映射（sl_pct_fallback_for / tp_pct_fallback_for）与副本已漂移"
    );
    assert!(pm.contains(PRESENT_FN.trim()), "present 定义已变，请同步本副本");
    assert!(
        pm.contains(LEG_MULT_FN.trim()),
        "portfolio-mgr.rhai 的 leg_mult 与本测试副本已漂移（逐档乘数取值器是 A1 的承重件）"
    );
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
    // 超短档（v99 起重做）：必须走**同一逐档融合**，不得再直接取主链后验。
    assert!(
        pm.contains("let ultra_short_dec = horizon_decision.call(\"ultra_short\""),
        "超短档应经 horizon_decision 逐档融合（旧「单后验方案B」与主档恒等，已被判为构造性错误）"
    );
    assert!(
        !pm.contains("let us_action = if effective_posterior"),
        "超短档不得退回直接取主链 effective_posterior 的形态"
    );
    assert!(!pm.contains("\"stopLossPct\": if us_pos>0.0 { 3.0 }"), "超短档不得再硬写 3.0");
}

/// 核心锁（四周期科学化 Phase A）：每档融合必须**逐腿乘该档权重**，
/// 且旧的「非技术腿整块共用主链」捷径不得复活 —— 那是 E1（一个预测贴四个标签）的实现形态。
#[test]
fn every_leg_is_reweighted_per_tier() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    assert!(
        pm.contains("leg.weight * leg_mult.call(h, leg.key)"),
        "逐档融合必须对每条证据腿乘该档乘数，而不是只替换 f1 信号"
    );
    assert!(
        pm.contains("let decision_legs = ["),
        "应有统一的证据腿表（腿名与 evidence_weight::DECISION_LEG_ANALYST 桥对应）"
    );
    for dead in ["non_tech_total_weight", "non_tech_weighted_signal"] {
        assert!(
            !pm.contains(dead),
            "{dead} 是非技术腿四档共用的捷径变量，Phase A 起必须不存在（留着它 = 回到 E1）"
        );
    }
    // 降级必须可检：乘数表缺失时要标注来源，不得静默当成逐档加权。
    assert!(
        pm.contains("\"weightsSource\": if horizon_leg_weights_ok"),
        "每档应输出 weightsSource（table | fallback_unity），让降级可见"
    );
}

/// 逐档乘数取值器 —— 与 `portfolio-mgr.rhai` 的 `leg_mult` 逐字一致（防漂移靠下方断言）。
const LEG_MULT_FN: &str = r#"
let leg_mult = |h, leg| {
    if !horizon_leg_weights_ok {
        1.0
    } else {
        let row = horizon_leg_weights_json[h];
        if type_of(row) != "map" {
            1.0
        } else {
            let m = row[leg];
            if type_of(m) == "()" { 1.0 } else { m }
        }
    }
};
"#;

/// 融合循环骨架 —— 与脚本 `horizon_decision` 内的逐腿加权段同构（腿表用固定夹具，
/// 目的是验「乘数真的进了融合」，不是验腿名）。
const FUSE: &str = r#"
let tw = 0.0;
let ws = 0.0;
for leg in legs {
    let w = leg.weight * leg_mult.call(H, leg.key);
    tw += w;
    ws += w * leg.signal;
}
#{ "avg": if tw > 0.0 { ws / tw } else { 0.0 } }
"#;

fn fuse(tier: &str, weights: rhai::Map) -> f64 {
    let engine = Engine::new();
    let mut scope = rhai::Scope::new();
    scope.push_constant("H", rhai::Dynamic::from(tier.to_string()));
    scope.push_constant("horizon_leg_weights_ok", rhai::Dynamic::from(true));
    scope.push_constant("horizon_leg_weights_json", rhai::Dynamic::from(weights));
    let legs = rhai::Array::from(vec![
        rhai::Dynamic::from({
            let mut m = rhai::Map::new();
            m.insert("key".into(), rhai::Dynamic::from("f1"));
            m.insert("weight".into(), rhai::Dynamic::from(0.15));
            m.insert("signal".into(), rhai::Dynamic::from(0.4));
            m
        }),
        rhai::Dynamic::from({
            let mut m = rhai::Map::new();
            m.insert("key".into(), rhai::Dynamic::from("f5"));
            m.insert("weight".into(), rhai::Dynamic::from(0.2));
            m.insert("signal".into(), rhai::Dynamic::from(-0.6));
            m
        }),
    ]);
    scope.push_constant("legs", rhai::Dynamic::from(legs));
    let ast = engine.compile(format!("{LEG_MULT_FN}{FUSE}")).expect("融合骨架应可编译");
    engine
        .eval_ast_with_scope::<rhai::Map>(&mut scope, &ast)
        .expect("融合骨架应可求值")
        .get("avg")
        .and_then(|v| v.clone().try_cast::<f64>())
        .expect("avg 应为浮点")
}

fn tier_row(pairs: &[(&str, f64)]) -> rhai::Map {
    let mut m = rhai::Map::new();
    for (k, v) in pairs {
        m.insert((*k).into(), rhai::Dynamic::from(*v));
    }
    m
}

/// 判别力：同一批腿、同一批信号，**只有乘数表不同** ⇒ 融合结果必须不同。
/// 这条锁的是「乘数真的进了融合」；若有人把融合改回「只换 f1 信号」，两档会算出同一个数。
#[test]
fn tier_multipliers_actually_change_the_fusion() {
    let mut weights = rhai::Map::new();
    weights.insert("mid".into(), rhai::Dynamic::from(tier_row(&[("f1", 1.0), ("f5", 1.0)])));
    weights.insert("long".into(), rhai::Dynamic::from(tier_row(&[("f1", 0.6), ("f5", 2.0)])));
    let mid = fuse("mid", weights.clone());
    let long = fuse("long", weights);
    assert!(
        (mid - long).abs() > 1e-6,
        "乘数表不同却算出同一个 avg（{mid}）⇒ 乘数没进融合，四档仍是共用权重"
    );
    // 负控：两档乘数完全相同 ⇒ avg 必须相同（证明差异只来自乘数，不是夹具里的随机性）
    let mut same = rhai::Map::new();
    same.insert("mid".into(), rhai::Dynamic::from(tier_row(&[("f1", 1.0), ("f5", 1.0)])));
    same.insert("long".into(), rhai::Dynamic::from(tier_row(&[("f1", 1.0), ("f5", 1.0)])));
    assert_eq!(fuse("mid", same.clone()), fuse("long", same));
}

/// 缺表 ⇒ 恒 1.0（可检降级），且此时各档必然同值 —— 正是 `weightsSource` 要暴露的形态。
#[test]
fn missing_table_falls_back_to_unity() {
    let engine = Engine::new();
    let mut scope = rhai::Scope::new();
    scope.push_constant("H", rhai::Dynamic::from("long"));
    scope.push_constant("horizon_leg_weights_ok", rhai::Dynamic::from(false));
    scope.push_constant("horizon_leg_weights_json", rhai::Dynamic::UNIT);
    let mut m = rhai::Map::new();
    m.insert("key".into(), rhai::Dynamic::from("f5"));
    m.insert("weight".into(), rhai::Dynamic::from(0.2));
    m.insert("signal".into(), rhai::Dynamic::from(-0.6));
    scope.push_constant(
        "legs",
        rhai::Dynamic::from(rhai::Array::from(vec![rhai::Dynamic::from(m)])),
    );
    let ast = engine.compile(format!("{LEG_MULT_FN}{FUSE}")).expect("应可编译");
    let got = engine
        .eval_ast_with_scope::<rhai::Map>(&mut scope, &ast)
        .expect("缺表时融合仍应可求值")
        .get("avg")
        .and_then(|v| v.clone().try_cast::<f64>())
        .expect("avg");
    assert!((got - (-0.6)).abs() < 1e-9, "缺表应退化为原始信号加权（乘数恒 1.0），实得 {got}");
}

/// 逐档先验取值器 —— 与 `portfolio-mgr.rhai` 的 `prior_for` 逐字一致（Phase C）。
const PRIOR_FOR_FN: &str = r#"
let prior_for = |h| {
    let row = if horizon_prior_ok { horizon_prior_json[h] } else { () };
    if type_of(row) == "map" && type_of(row["prior"]) != "()" {
        #{ "value": row["prior"], "source": row["source"], "samples": row["samples"] }
    } else {
        #{ "value": prior, "source": "shared_regime_prior", "samples": 0 }
    }
};
"#;

/// 探针：命中档 / 表里没有的档，各取什么值、标什么来源。
const PRIOR_PROBE: &str = r#"
#{
    "own_value": prior_for.call("mid")["value"],
    "own_source": prior_for.call("mid")["source"],
    "miss_source": prior_for.call("nope")["source"],
    "miss_value": prior_for.call("nope")["value"],
}
"#;

fn prior_probe(table: rhai::Map) -> (f64, String, String, f64) {
    let engine = Engine::new();
    let mut scope = rhai::Scope::new();
    scope.push_constant("prior", rhai::Dynamic::from(0.52_f64));
    scope.push_constant("horizon_prior_ok", rhai::Dynamic::from(true));
    scope.push_constant("horizon_prior_json", rhai::Dynamic::from(table));
    let ast = engine.compile(format!("{PRIOR_FOR_FN}{PRIOR_PROBE}")).expect("先验段应可编译");
    let r = engine.eval_ast_with_scope::<rhai::Map>(&mut scope, &ast).expect("先验段应可求值");
    let g = |k: &str| r.get(k).cloned().unwrap();
    (
        g("own_value").try_cast::<f64>().expect("own_value"),
        g("own_source").try_cast::<String>().expect("own_source"),
        g("miss_source").try_cast::<String>().expect("miss_source"),
        g("miss_value").try_cast::<f64>().expect("miss_value"),
    )
}

fn prior_row(prior: f64, source: &str, samples: i64) -> rhai::Map {
    let mut m = rhai::Map::new();
    m.insert("prior".into(), rhai::Dynamic::from(prior));
    m.insert("source".into(), rhai::Dynamic::from(source.to_string()));
    m.insert("samples".into(), rhai::Dynamic::from(samples));
    m
}

/// 逐档先验必须真的被取用：命中档取该档收缩值并带来源；表里没有的档退回共用 prior
/// 且**标成 `shared_regime_prior`**（不得伪装成本档统计）。
#[test]
fn prior_for_uses_the_tier_estimate_and_labels_the_fallback() {
    let mut table = rhai::Map::new();
    table.insert("mid".into(), rhai::Dynamic::from(prior_row(0.61, "shrunk", 88)));
    let (own_val, own_src, miss_src, miss_val) = prior_probe(table);
    assert_eq!((own_val, own_src.as_str()), (0.61, "shrunk"), "命中档应取该档收缩先验");
    assert_eq!(miss_src, "shared_regime_prior", "缺档必须标成共用先验");
    assert_eq!(miss_val, 0.52, "缺档退回值必须是主链 prior");
}

/// 防漂移 + 防「取而不用」：脚本必须逐字含 `prior_for`，且融合里后验用的是 `hp["value"]`
/// 而不是四档共用的裸 `prior`（后者正是 E1 的实现形态）。
#[test]
fn per_tier_prior_is_wired_into_the_fusion() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    assert!(
        pm.contains(PRIOR_FOR_FN.trim()),
        "prior_for 与本测试副本已漂移（逐档先验取值器是 Phase C 的承重件）"
    );
    assert!(
        pm.contains("clamp(hp[\"value\"] + avg * evidence_scale"),
        "逐档后验必须用该档先验 hp[\"value\"]；写回裸 `prior` 就是退回四档共用先验（E1）"
    );
    assert!(
        pm.contains("let hp = prior_for.call(h);"),
        "prior_for 是闭包，必须 .call（按名调用恒 Function not found）"
    );
    assert!(
        pm.contains("\"priorSource\": hp[\"source\"]"),
        "每档必须透出先验来源，否则「收缩自本档」与「退回共用」在产出里不可区分"
    );
}

/// 锁（Phase D）：止损/止盈必须由「σ_daily × √持有天数」推导，且降级路径与来源标注齐全。
/// 这条锁的判别力：若有人把 `sl_pct_for` 改回纯 switch（不看波动），
/// 前两断言当场红；若删掉降级标注，第三条红 —— 正是「同屏两个读数」被禁止的形态。
#[test]
fn stop_and_take_profit_are_volatility_derived_with_labeled_fallback() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    assert!(
        pm.contains("clamp(stop_vol_mult_v * mv, 0.5, STOP_CAP_PCT)")
            && pm.contains("clamp(tp_vol_mult_v * mv, 1.0, TP_CAP_PCT)"),
        "止损/止盈应由 k·σ·√h 推导（含语义上限截断），实得形态不符"
    );
    assert!(
        pm.contains("pm_vol_move_pct(kline_bars, VOL_LOOKBACK_DAYS, days_for.call(h))"),
        "该档位移必须按**该档持有天数**算（days_for），不能四档共用一个数"
    );
    assert!(
        pm.contains("if mv <= 0.0 {\n        sl_pct_fallback_for.call(h)"),
        "σ 不可得时必须退回可调百分比档（而不是算出 0% 止损）"
    );
    assert!(
        pm.contains("\"stopSource\": stop_source_for.call(h)")
            && pm.contains("\"stopSource\": stop_source_for.call(time_horizon)"),
        "每档与主档都要输出 stopSource，否则「波动率口径」与「退回固定档」在产出里无法区分"
    );
    assert!(
        pm.contains("let stop_vol_mult_v = if present(stop_vol_mult)"),
        "k1 必须经 present() 守卫读取（旧快照缺该变量时不得抛 Variable not found）"
    );
}
