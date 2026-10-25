//! 行为测试：portfolio-mgr.rhai **f2 的方向口径**（v182）。
//!
//! 背景（2026-10-25 核查）：`consensus_score` 的契约是**双方达成共识的程度**
//! （`agency_experts/stock-analysis/debate-convergence.md` 工作流程第 5 步：起点 50、
//! 双方 strength 差距 <15 → +20、差距 >30 → 占优方 −10、remaining_disputes × −5），
//! 本身没有方向含义；旧实现 `pm_score_to_signal(consensus_score)` 却按「>50 即看多」消费它。
//! 现网读数（全库 283 条带聚合预测的记录）：direction 分布 bearish 165 / divided 46 /
//! neutral 46 / bullish 20 —— 「双方一致看空」是常态收敛形态，旧口径会给它**正**信号。
//! 实测一条 `direction=neutral` + `consensus_score=55` 的记录被投成 +0.1 看多。
//!
//! 本测试抽取 f2 段的最小复现（与 `portfolio_mgr_veto_rhai.rs` 同一「抽取关心片段」模式），
//! 同时算出旧口径作对照，断言：
//! - **正控①**：一致看空且收敛度高 ⇒ 新口径为负、旧口径为正（方向相反，这就是缺陷本体）
//! - **正控②**：neutral / divided ⇒ 信号 0 **且权重退出**（不占 0.25 权重挂 0 信号）
//! - **正控③**：direction 未注入（旧快照 / 未换代）⇒ 权重退出、缺席标记为真
//! - **正控④**：收敛度当强度 ⇒ 同一 bullish 下，收敛度低的信号必须更小（单调）
//!
//! ⚠ 若主链 f2 段漂移，本复现需同步；反向锁（不得再把 `pm_score_to_signal(consensusScore)`
//! 抄回来）住在 `analysis-engine::evidence_audit::score_normalization_is_not_inlined_anywhere`，
//! 本文件不重复那条判据。
use rhai::Engine;

const SCRIPT: &str = r#"
// ── 与主链同形的 helper（fn 体只操作自己的参数，见 portfolio-mgr.rhai:22 处注释）──
fn present(x) { type_of(x) != "()" }
fn get_weight(weights, factor, fallback) {
    if type_of(weights) == "map" {
        let f = weights[factor];
        if type_of(f) != "()" && type_of(f) == "map" {
            let w = f["weight"];
            if type_of(w) == "f64" { w } else { fallback }
        } else { fallback }
    } else { fallback }
}

// ── f2（v182）：方向取 direction 的符号，强度取 consensus_score/100 ──
let f2_default = 0.25;
let f2_direction_known = present(consensusDirection)
    && type_of(consensusDirection) == "string"
    && (consensusDirection == "bullish" || consensusDirection == "bearish"
        || consensusDirection == "neutral" || consensusDirection == "divided");
let f2_dir_sign = if consensusDirection == "bullish" {
    1.0
} else if consensusDirection == "bearish" {
    -1.0
} else {
    0.0
};
let f2_weight = if present(consensusScore) && f2_direction_known && f2_dir_sign != 0.0 {
    get_weight(f_weights, "consensus", f2_default)
} else { 0.0 };
let f2_signal = if f2_weight > 0.0 {
    f2_dir_sign * clamp(consensusScore.to_float() / 100.0, 0.0, 1.0)
} else { 0.0 };

// ── 旧口径（对照用，主链已不再这样写）──
let old_signal = if present(consensusScore) {
    (consensusScore.to_float() - 50.0) / 50.0
} else { 0.0 };

#{ "weight": f2_weight, "signal": f2_signal, "known": f2_direction_known, "old": old_signal }
"#;

struct F2 {
    weight: f64,
    signal: f64,
    direction_known: bool,
    old_signal: f64,
}

/// `direction` 传 `None` 模拟「变量存在但没有值」（旧快照 / 未换代时 What-If 的兜底形态）。
fn run(direction: Option<&str>, consensus_score: i64) -> F2 {
    let mut engine = Engine::new();
    // `clamp` 在生产由 `harness::rhai_engine::register_common_functions` 注册
    // （签名 f64/f64/f64）；裸 `Engine::new()` 没有它 ⇒ 报 `ErrorFunctionNotFound`。
    // 与 `data_quality_placeholder_gate.rs` 的替身注册保持同一签名。
    engine.register_fn("clamp", |v: f64, min: f64, max: f64| -> f64 { v.clamp(min, max) });
    let mut scope = rhai::Scope::new();
    match direction {
        Some(d) => {
            scope.push("consensusDirection", d.to_string());
        },
        None => {
            scope.push("consensusDirection", ());
        },
    }
    scope.push("consensusScore", consensus_score);
    // 主链的 f_weights 来自回测/市况权重表；这里给**空 map** ⇒ `get_weight` 查不到
    // "consensus" 键 ⇒ 回落 f2_default，正是本测试要验的那条默认权重路径。
    // （不 push 会 `ErrorVariableNotFound` —— 主链里这个变量恒由 input_mapping 注入。）
    scope.push("f_weights", rhai::Map::new());
    // 不给回测权重表 ⇒ get_weight 回落 f2_default，正是要验的那条默认权重路径
    let result: rhai::Map = engine.eval_with_scope(&mut scope, SCRIPT).expect("脚本执行失败");
    let num = |k: &str| -> f64 {
        result
            .get(k)
            .and_then(|d| d.clone().try_cast::<f64>())
            .unwrap_or_else(|| panic!("{k} 应为数字"))
    };
    let direction_known = result
        .get("known")
        .and_then(|d| d.clone().try_cast::<bool>())
        .unwrap_or_else(|| panic!("known 应为布尔"));
    F2 { weight: num("weight"), signal: num("signal"), direction_known, old_signal: num("old") }
}

/// 正控①：一致看空且收敛度高 —— 新口径给负信号，旧口径给**正**信号（缺陷本体）。
#[test]
fn consensus_of_a_bearish_debate_no_longer_reads_as_bullish() {
    let got = run(Some("bearish"), 80);
    assert!((got.signal + 0.8).abs() < 1e-9, "bearish + 收敛 80 应得 -0.8，实得 {}", got.signal);
    assert!(
        (got.old_signal - 0.6).abs() < 1e-9,
        "对照：旧口径把同一条读成 +0.6 看多，实得 {}",
        got.old_signal
    );
    assert!(got.signal < 0.0 && got.old_signal > 0.0, "两侧方向必须相反，否则本用例没测到缺陷");
    assert!((got.weight - 0.25).abs() < 1e-9, "有方向的腿应保留 f2_default 权重");
}

/// 正控②：neutral / divided 是「不给方向」，信号 0 且**权重退出**。
#[test]
fn undirected_debate_withdraws_the_leg_instead_of_holding_weight_at_zero() {
    for d in ["neutral", "divided"] {
        let got = run(Some(d), 70);
        assert!((got.signal).abs() < 1e-9, "{d} 不该出方向票，实得 {}", got.signal);
        assert!((got.weight).abs() < 1e-9, "{d} 必须让权重退出，否则 0 信号白占 0.25");
        // 「值域内但无方向」与「字段没吐」是两件事：前者 known=true
        assert!(got.direction_known, "{d} 是有效值域内的结论，不是缺席");
    }
}

/// 正控③：direction 缺席 ⇒ 标记为假（主链据此写 data_gaps），权重与信号都归零。
#[test]
fn missing_direction_is_flagged_as_absent_not_as_neutral() {
    let got = run(None, 65);
    assert!(!got.direction_known, "字段没吐必须报缺席，不能与 neutral/divided 混成同一种");
    assert!((got.weight).abs() < 1e-9);
    assert!((got.signal).abs() < 1e-9);
}

/// 正控④：收敛度只当强度 —— 同方向下收敛度越低，信号越小（单调，不再翻转方向）。
#[test]
fn consensus_degree_only_scales_strength_within_one_direction() {
    let strong = run(Some("bullish"), 80);
    let weak = run(Some("bullish"), 30);
    assert!(strong.signal > weak.signal, "收敛度高者信号必须更强");
    assert!((weak.signal - 0.3).abs() < 1e-9, "bullish + 收敛 30 应得 +0.3，实得 {}", weak.signal);
    assert!(weak.signal > 0.0, "旧口径会把收敛度 30 读成 -0.4 看空；新口径不得翻转方向");
    assert!((strong.weight - weak.weight).abs() < 1e-9, "两例权重同为 f2_default，差异只在信号");
}

/// 负控：脚本里那句「值域外」（模型吐出词表外的方向串）按缺席处理，不给方向票。
#[test]
fn out_of_vocabulary_direction_is_treated_as_absent() {
    let got = run(Some("强烈看好"), 90);
    assert!(!got.direction_known, "词表外的方向串不得被当成有效方向");
    assert!((got.weight).abs() < 1e-9);
}
