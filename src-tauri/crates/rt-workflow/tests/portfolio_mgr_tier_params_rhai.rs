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
    // ⚠ 2026-10-04 R-11 退役：`leg_mult` / 乘数表整条链没了，原本的「LEG_MULT_FN 逐字一致」
    //   锁随之删除（对象不存在时，逐字锁会退化成恒假断言 + 一次改名即红的假门）。
    //   同一条「档间必须有真实差异」的判据换了产地：
    //   `seed_consistency_tests::horizon_branch_rhai_scripts_compile_and_are_genuinely_forked`
    //   （四份分支脚本两两不同 + 禁词 + 必须读注入分支表）与
    //   `rhai_registry::portfolio_mgr_runs_with_valuation_evidence_without_degrading` 的 ②/④。
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

/// 核心锁（R-11 换心脏后改了方向）：`decisionsByHorizon` 只**装配**四路分支输出，
/// 装配区里不得再出现任何逐档计算 —— 原先这条锁的是「四档必须经 `sl_pct_for`/`tp_pct_for`/`days_for`
/// 取值」（那时逐档决策由主链闭包算），而闭包本身已被判为「一个算法 + 四套参数」。
/// 现在同一格的期望形态反过来：主链若又去取逐档参数，就是代算。
#[test]
fn decisions_by_horizon_assembles_and_never_recomputes() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    let region_start =
        pm.find("// 8< assembly-begin").expect("装配段起始锚点应存在（换心脏的落点）");
    let region_end =
        pm.find("// 8< assembly-end").expect("装配段结束锚点应存在（锚点变了要同步本测试）");
    assert!(region_end > region_start, "装配区边界反了");
    let region = &pm[region_start..region_end];
    // 阶段2：四路输入清单**只有一份**（`branch_tiers`，定义在选档段），装配区必须迭代它。
    // 另立一份的坏处不是重复，而是「选档用一份、装配用另一份」⇒ 两处的有效性判定会漂移。
    assert!(region.contains("for b in branch_tiers"), "装配区不再复用选档段的行清单");
    for src in ["h_ultra_short", "h_short", "h_mid", "h_long"] {
        assert!(
            pm.contains(&format!("row: {src}")),
            "四路输入清单里没接 {src} ⇒ 该路分支输出根本没进主链"
        );
    }
    // 反向锁：主链不得在装配区里逐档取值/融合/折算。
    // ⚠ 每个禁词都**带调用标点**（`.call(` / `(`）：装配区上方留着三段退役说明注释，逐字提到
    //   `leg_mult` / `horizon_decision` / `pm_snr_confidence` —— 那是给后来者
    //   解释「这里为什么没有乘数」的。裸名判定会把注释也算进去 ⇒ 假红（2026-10-04 实测踩过）。
    //   判据本来就是「不得**调用**逐档函数」，带上括号既更准，也天然免疫注释。
    for dead in [
        "sl_pct_for.call(",
        "tp_pct_for.call(",
        "days_for.call(",
        "leg_mult(",
        "horizon_decision(",
        "pm_snr_confidence(",
        "evidence_max_for(",
        "prior_for(",
    ] {
        assert!(
            !region.contains(dead),
            "装配区又出现 `{dead}` ⇒ 主链开始代算逐档参数（R-11 已把这件事交回各分支脚本）"
        );
    }
    // 自证（禁词不是恒真）：把退役调用插进装配区，同一套锚点必须能把它抓出来。
    let bad = pm.replace(
        "let decisions_by_horizon = #{};",
        "let decisions_by_horizon = #{};\nlet __probe = pm_snr_confidence(1.0, 5.0, 28.0);",
    );
    assert_ne!(bad, pm, "负控变异点未命中 ⇒ 上面的禁词已失去区分力");
    let bad_start = bad.find("// 8< assembly-begin").expect("装配段起始锚点应存在");
    let bad_end = bad.find("// 8< assembly-end").expect("装配段结束锚点应存在");
    assert!(
        bad[bad_start..bad_end].contains("pm_snr_confidence("),
        "负控失效：装配区里真出现退役调用时禁词没报 ⇒ 本锁恒真"
    );
    // 缺席必须走「不出键 + 点名」，而不是补一行占位（已批口径 A，PLAN §四十一）。
    assert!(
        region.contains("data_gaps.push(`") && region.contains("分支未产出"),
        "缺席档没在 data_gaps 点名 ⇒ 结构性缺口会在呈现层变成歧义"
    );
    assert!(region.contains("continue;"), "缺席分支必须跳过该档（continue），而不是产出空行");
    // 逐档退化也要在**顶层** data_gaps 留痕（旧主链就有，换心脏时一度被装配段丢掉）。
    assert!(
        region.contains("row[\"scoreSource\"] == \"daily_fallback\""),
        "装配段不再把「该档技术腿按日线退化」推进顶层 data_gaps ⇒ 缺口横幅会少计，UI 只剩档内注脚"
    );
    // 顺序契约：`gap_note` 必须在逐档 push **之后**求值，否则文案写「缺口(3)」而实有 5 条。
    let gap_note_at = pm.find("let gap_note = if data_gaps.len()").expect("gap_note 应存在");
    assert!(
        gap_note_at > region_end,
        "gap_note 的定义跑到了装配段之前 ⇒ 逐档缺口不计入推理文案：note={gap_note_at} 装配段末={region_end}"
    );
    // 旧的字面量抄本与「超短直接取主链后验」两族形态继续锁住（它们与换心脏无关，仍是禁区）。
    for lit in [
        "totalScore_short, 5.0, 10.0, 5",
        "totalScore_mid, 8.0, 18.0, 28",
        "totalScore_long, 12.0, 30.0, 90",
    ] {
        assert!(!pm.contains(lit), "脚本残留档位字面量抄本: {lit}");
    }
    assert!(
        !pm.contains("let us_action = if effective_posterior"),
        "超短档不得退回直接取主链 effective_posterior 的形态"
    );
    assert!(!pm.contains("\"stopLossPct\": if us_pos>0.0 { 3.0 }"), "超短档不得再硬写 3.0");
    // ⚠ 退役字段（`weightsSource` / `sharesPosteriorWith` / `snrAnchorDays`）与退役算法的**全文级**
    //   禁词门不在本文件：主链脚本里留着三段退役说明注释逐字提到这些名字，裸文本判定必然假红，
    //   而「剥注释后再查代码域」需要的 `code_only()` 在主 crate（本 crate 拿不到，也不该复制一份）。
    //   ⇒ 那条门搬到 `seed_consistency_tests::main_script_keeps_retired_per_tier_algebra_out_of_code`。
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
    // 主档必须输出 stopSource，而且**标签说的是实际被采用的那条算法**
    // （2026-10-04 R-11 把逐档出场口径交给分支脚本；v125 批准 ① 又把主档数值改成取自
    //   所选档 ⇒ 来历标签必须跟着走。否则分支算出的止损会被署上主链的 `vol`/`fallback_pct`，
    //   那是给一条**没被采用的算法**署名 —— 与「兜底伪装成主口径」同一族缺陷）。
    assert!(
        pm.contains(
            "let main_stop_source = if type_of(b_sl) != \"()\" { picked_row[\"stopSource\"] } else { stop_source_for.call(time_horizon) };"
        ),
        "主档 stopSource 的来源不再是「被采用那条算法自己的标签」"
    );
    assert!(
        pm.contains("\"stopSource\": main_stop_source"),
        "主档没有 stopSource ⇒ 「波动率口径」与「退回固定档」在产出里无法区分"
    );
    const PER_TIER_STOP_SOURCE: &str = "\"stopSource\": stop_source_for.call(h)";
    assert!(
        !pm.contains(PER_TIER_STOP_SOURCE),
        "主链又逐档写 stopSource ⇒ 逐档出场口径回到主链代算（应在分支脚本里）"
    );
    assert!(
        pm.replace(
            "let decisions_by_horizon = #{};",
            &format!(
                "let probe = #{{ {} }};\nlet decisions_by_horizon = #{{}};",
                PER_TIER_STOP_SOURCE
            )
        )
        .contains(PER_TIER_STOP_SOURCE),
        "负控失效：逐档 stopSource 真回到主链时这条禁词抓不出来 ⇒ 上面的断言恒真"
    );
    // 逐档那一份必须由分支脚本产出（否则「翻向」= 把判据删了，两头都没锁）。
    for (tier, code) in [
        ("ultra_short", include_str!("../../../src/commands/portfolio-mgr-h-ultra-short.rhai")),
        ("short", include_str!("../../../src/commands/portfolio-mgr-h-short.rhai")),
        ("mid", include_str!("../../../src/commands/portfolio-mgr-h-mid.rhai")),
        ("long", include_str!("../../../src/commands/portfolio-mgr-h-long.rhai")),
    ] {
        assert!(
            code.contains("stopSource:") && code.contains("fallback_pct"),
            "分支脚本 {tier} 不再输出自己的止损口径 ⇒ 逐档 stopSource 这一族信息整体丢失"
        );
    }
    assert!(
        pm.contains("let stop_vol_mult_v = if present(stop_vol_mult)"),
        "k1 必须经 present() 守卫读取（旧快照缺该变量时不得抛 Variable not found）"
    );
}
