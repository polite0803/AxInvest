// portfolio-mgr-h-long.rhai — **长线（90 交易日）** 决策分支（R-11 / PLAN §十一、§二十五）
//
// 本档的算法分支在**三处**与其余三档不同，且每处都是换了判据而不是换了倍数：
// 1. 腿集合：方向只由「价值四腿」（估值带 / 盈利质量 / 预期修正 / 宏观状态）决定；
//    技术腿 `trendStrength` 在本档 **role=filter** —— 只作入场时机过滤，不进方向加权
//    （R-11 明文；权重恒 0 由分支表保证，不靠这里自觉）。行业轮动是 `partial`
//    （live 有源、回放结构性不可得 ⇒ 只作背景）。
// 2. 门：`positive_margin_of_safety` —— 没有安全边际就不出加仓动作。锚取**外部标准**：
//    DCF 上行空间 ≥ 30%（Graham 的 conventional margin of safety，不是本仓调出来的阈值）。
//    DCF 对本标的**不适用**（`applicable=false`：银行/保险/券商/地产/重资产周期，
//    由 `compute_dcf` 按数据形态判）时，换用**第二条显式口径**：PE 落在自身 5 年
//    下四分位（≤25 分位）。两条都拿不到 ⇒ 门「未判定」，按不通过处理。
//    ⚠ 这不是「拿近似数据顶替」：两条口径各自成立、各自点名，输出里带 `gateBasis`。
// 3. 出场：`target_and_falsified` —— 止盈 = 估值目标价；止损 = **论点被证伪**
//    （上行空间被走完 ⇒ `margin_basis_exhausted`；或盈利质量/预期修正两腿同时转负
//    ⇒ `thesis_contradicted`）。价格波动本身不构成长线止损理由，这是本档与其余三档
//    最根本的差别：短档的止损买的是「时间窗内的位移控制」，长档买的是「论点还成立」。
//
// ⚠ 禁止出现（门 a）：`leg_mult` / `pm_snr_confidence` / `horizon_decision`。

// 与 `portfolio-mgr.rhai` 同规：`present` 是脚本自带的 1 行守卫（宿主引擎并未注册它，
// 靠宿主注册会让脚本依赖「注册面在不在」，本仓有过 pm_* 静默未注册的前例）。
fn present(x) { type_of(x) != "()" }

let h = "long";
let gaps = [];
// ── 输入面（与节点 `input_mapping` 一一对应；读了没映射的名字 = 运行期 Variable not found）──
// 已接：branch_json / horizon_prior_json / tier_score / macd_dif / macd_dea / rsi_14 /
//       pe_percentile / f_score / consensus_eps / consensus_estimated /
//       latest_eps / latest_eps_basis / pmi /
//       valuation_dcf_upside / valuation_dcf_applicable / kline_bars / overall_risk / stop_vol_mult
// 刻意不读（有名字但没有数据面）：
//   · 催化剂（本档腿集无 `eventCatalyst`，任务 #22）
//   · 行业分位：行业排名的 as-of 通道仍有结构性缺口 ⇒ 不用个股涨幅近似（用户裁定③）
gaps.push("long 档 sectorRotation 只作背景（role=partial、权重恒 0），且行业分位的 as-of 通道未闭合 ⇒ 不用近似值顶替");

let spec_ok = type_of(branch_json) == "map" && !branch_json.is_empty();
if !spec_ok {
    gaps.push("逐档分支表缺失 ⇒ 本档无腿可融合，拒绝输出结论（不退等权、不退共用参数）");
}
if spec_ok && branch_json["tier"] != h {
    gaps.push(`分支表档位是 ${branch_json["tier"]} 而本脚本是 ${h} ⇒ 子模板接线错配`);
}
let legs = if spec_ok { branch_json["legs"] } else { [] };
// 价带两个入参只能来自分支表；缺键或类型不对 ⇒ 一律按「不可得」（band_* = 0 ⇒ 不宿主调用），
// 绝不对 unit 调 to_int() —— 那是运行时 Function not found，崩得比缺席难看。
// ⚠ 数值判据必须**同时**收 i64 与 f64。两个载体口径不同（实证于 2026-10-04，000710 四档
//   止损/止盈/赔率/仓位全 0）：`code_executor` 只把**顶层** JSON 数字统一转 f64，而嵌在 map 里的
//   数字走 `harness::rhai_engine::json_value_to_dynamic` —— 它**保整数**（避免精度丢失），
//   故 `branch_json["days"]` 的 `type_of` 是 "i64" 而不是 "f64"（该口径注见 `data-quality.rhai`
//   开头）。只认 "f64" ⇒ band_days/band_lookback 恒 0 ⇒ 宿主调用被跳过，而 `stopSource`
//   仍谎报 `fallback_pct`（固定百分比兜底其实从未参与）。
let spec_int = |v| if type_of(v) == "f64" { v.to_int() } else if type_of(v) == "i64" || type_of(v) == "int" { v } else { 0 };
let band_days = if spec_ok { spec_int.call(branch_json["days"]) } else { 0 };
let band_lookback = if spec_ok { spec_int.call(branch_json["volLookbackDays"]) } else { 0 };


// ── ① 逐腿取信号（口径唯一实现 = pm_leg_signal）──
// 一致预期的板块常数估算形态同 mid 档处理（`types.rs:310` 明文禁止拿它做超预期判定）。
let cons_is_estimated = present(consensus_estimated) && consensus_estimated == true;
if cons_is_estimated {
    gaps.push("一致预期是板块常数估算值（is_estimated=true）=> 本档预期修正腿按缺席处理，不用估算值做超预期判定");
}
// #24（v131）：分母 = 最近一个**已披露年报**的 EPS（`act` 在 `(cons-act)/act` 的分母位）。
// 缺席分两种、文案不同 —— 合成一句就分不出「这票没有年报」与「估值节点没跑成」：
//   · `basis` 在而 `value` 不在 ⇒ 截止日前没有已披露年报（标的属性；**不做年化外推**）
//   · 两者都不在 ⇒ `t-valuation` 节点整体缺席（它还要行情数据 ⇒ 接线/取数问题）
if !present(latest_eps) {
    if present(latest_eps_basis) {
        gaps.push(`long 档 expectationRevision 缺分母：截止日前没有已披露年报（basis=${latest_eps_basis}）⇒ 本腿按缺席处理，不拿季报累计值年化顶替`);
    } else {
        gaps.push("long 档 expectationRevision 缺分母：t-valuation 节点未产出 latestEps ⇒ 接线状态，不是「预期没变化」");
    }
}
let raw = #{
    totalScore: if present(tier_score) { tier_score } else { () },
    macdDif: if present(macd_dif) { macd_dif } else { () },
    macdDea: if present(macd_dea) { macd_dea } else { () },
    rsi14: if present(rsi_14) { rsi_14 } else { () },
    pePercentile: if present(pe_percentile) { pe_percentile } else { () },
    fScore: if present(f_score) { f_score } else { () },
    consensusEps: if present(consensus_eps) && !cons_is_estimated { consensus_eps } else { () },
    latestEps: if present(latest_eps) { latest_eps } else { () },
    pmi: if present(pmi) { pmi } else { () },
};
let present_legs = [];
let absent_legs = [];
for row in legs {
    let s = pm_leg_signal(row["factor"], raw);
    if type_of(s) == "()" {
        absent_legs.push(row["factor"]);
    } else {
        present_legs.push(#{ factor: row["factor"], role: row["role"], weight: row["weight"], signal: s, constraint: row["constraint"] });
    }
}

// ── ② 融合：只有 role=="direction" 进方向和归一 ──
let tw = 0.0;
let ws = 0.0;
let filter_legs = [];
let background_legs = [];
for row in present_legs {
    if row["role"] == "direction" {
        tw += row["weight"];
        ws += row["weight"] * row["signal"];
    } else if row["role"] == "filter" {
        filter_legs.push(row["factor"]);
    } else {
        background_legs.push(row["factor"]);
    }
}
let avg = if tw > 0.0 { ws / tw } else { 0.0 };
let dir_total = 0.0;
for row in legs {
    if row["role"] == "direction" { dir_total += row["weight"]; }
}

// ── ③ 逐档先验 ──
// 先验**只认注入的逐档先验表**（Rust 侧按档算方向命中率再做经验贝叶斯收缩，`hooks.rs` 注入）。
// 明确不做的三件事：① 不裸读 `prior`（那是主链脚本内由市场状态推出的量，不是工作流变量，
// 接枝时按裸读注入 => `Variable not found`）；② 不在分支里重推一份主链推导
// （等于造第二份口径，PLAN §二十四 那批缺陷的成因）；③ 拿不到就**拒绝出结论**，
// 不塞 0.5 这种"看着中性"的猜测值（那是把缺席伪装成中性，裁定③禁止的形态）。
let hp_row = if type_of(horizon_prior_json) == "map" && !horizon_prior_json.is_empty() {
    horizon_prior_json[h]
} else {
    ()
};
let prior_ok = type_of(hp_row) == "map"
    && type_of(hp_row["prior"]) != "()"
    && type_of(hp_row["source"]) != "()"
    && type_of(hp_row["samples"]) != "()";
let prior_val = if prior_ok { hp_row["prior"] } else { () };
let prior_src = if prior_ok { hp_row["source"] } else { "unavailable" };
let prior_samples = if prior_ok { hp_row["samples"] } else { 0 };
if !prior_ok {
    gaps.push("本档先验不可得（`horizon_prior_json` 缺失或缺本档那一行）=> 拒绝出结论：观望且不产出仓位");
}
let escale = if dir_total > 0.0 { pm_evidence_scale(tw, dir_total) } else { 0.0 };
let posterior = if prior_ok { clamp(prior_val + avg * escale, 0.0, 1.0) } else { 0.0 };
let confidence = posterior;

// ── ④ 安全边际门（两条显式口径，各自点名）──
let dcf_ok = present(valuation_dcf_upside) && present(valuation_dcf_applicable) && valuation_dcf_applicable == true;
let moS_pct = if dcf_ok { valuation_dcf_upside } else { () };
let gate_pass = false;
let gate_basis = "unjudged";
if type_of(moS_pct) != "()" && moS_pct >= 30.0 {
    gate_pass = true;
    gate_basis = "dcf_margin_30pct";
} else if present(pe_percentile) && pe_percentile <= 25.0 {
    gate_pass = true;
    gate_basis = "pe_bottom_quartile";
} else if type_of(moS_pct) != "()" || present(pe_percentile) {
    gate_basis = "both_bells_failed";
}
if (gate_basis == "unjudged" || gate_basis == "both_bells_failed") && posterior >= 0.62 {
    gaps.push(if gate_basis == "unjudged" {
        "positive_margin_of_safety 未判定：DCF 不适用且 PE 分位缺失 ⇒ 不给出加仓动作（缺席 ≠ 通过）"
    } else {
        "安全边际两条口径都不满足（DCF 上行 < 30% 且 PE 不在下四分位）⇒ 本档不出买入"
    });
}

// ── ⑤ 技术腿只作入场时机过滤：带外向上 ⇒ 把「买入/增持」降为「持有」，但**不改方向分**──
let bars_ok = present(kline_bars) && type_of(kline_bars) == "array";
let lookback = if spec_ok { branch_json["volLookbackDays"] } else { 20 };
let days = if spec_ok { branch_json["days"] } else { 90 };
// 宿主函数形参是 i64 ⇒ 入参一律先经上方 `spec_int` 归一（分支表里的数字实际是 i64，不是 f64）。
let z = if bars_ok && band_lookback > 0 {
    pm_band_z(kline_bars, band_lookback)
} else {
    ()
};
let timing_ok = type_of(z) == "()" || z <= 2.0;

// ── ⑥ 出场：目标价止盈 + 论点证伪止损 ──
// 目标价**只**来自估值结论（DCF 上行空间）。DCF 对本标的不适用时**不做 PE 分位代理**：
// 用「回到中位所需涨幅」冒充目标价，就是把近似数据当结论（用户裁定③明文禁止），
// 而本仓对 DCF 不适用那一批（银行/保险/券商/地产/重资产）本来就有更诚实的出口 ——
// 没有目标价 ⇒ 赔率无定义 ⇒ 不出仓位，并在 `takeProfitSource` 点名 `no_target`。
let tp_pct = if type_of(moS_pct) != "()" { moS_pct } else { 0.0 };
let take_profit_source = if type_of(moS_pct) != "()" { "dcf_target" } else { "no_target" };
// 止损 = 论点被证伪（下面三条机械可检条件），而非价格波动；但证伪判定有滞后 ⇒
// 仍留一条**尾部硬止损**，倍数沿用面板已有的 `stop_vol_mult`（与其余三档同一参数入口，
// 不新增第四个调参面）。
let falsified = [];
if present(f_score) && f_score <= 2.0 {
    falsified.push("earnings_quality_collapse");
}
if type_of(pm_leg_signal("expectationRevision", raw)) != "()" {
    let er = pm_leg_signal("expectationRevision", raw);
    if er <= -0.5 {
        falsified.push("expectation_reversed");
    }
}
if type_of(moS_pct) != "()" && moS_pct <= 0.0 {
    falsified.push("margin_basis_exhausted");
}
let exit_move =
    if bars_ok && band_days > 0 && band_lookback > 0 {
    pm_vol_move_pct(kline_bars, band_lookback, band_days)
} else {
    0.0
};
let k_stop = if present(stop_vol_mult) { stop_vol_mult } else { 1.2 };
let stop_pct = exit_move * k_stop;
let odds = if stop_pct > 0.0 { tp_pct / stop_pct } else { 0.0 };
let kelly = if confidence >= 0.50 && odds > 0.001 {
    pm_kelly_position(confidence, odds, 0.0015, overall_risk)
} else {
    0.0
};
let position = kelly * (if spec_ok { branch_json["positionMultiplier"] } else { 1.2 });

// ── ⑦ 动作阶梯（本档独立常量集；加仓必须先过安全边际门，再不被时机过滤降档）──
let action = if confidence >= 0.72 { "买入" }
    else if confidence >= 0.62 { "增持" }
    else if confidence >= 0.52 { "持有" }
    else if confidence >= 0.42 { "观望" }
    else if confidence >= 0.32 { "减持" }
    else { "卖出" };
if !gate_pass && (action == "买入" || action == "增持") {
    action = "持有";
}
if !timing_ok && (action == "买入" || action == "增持") {
    action = "持有";
}
if !falsified.is_empty() && confidence >= 0.42 {
    action = "减持";
    position = 0.0;
    gaps.push("论点已被机械判据证伪 ⇒ 动作降为减持且不出仓位（出场口径 target_and_falsified 的 falsified 侧）");
}
if tw <= 0.0 || !prior_ok {
    action = "观望";
    position = 0.0;
    gaps.push("本档无在场方向腿或先验不可得 ⇒ 动作退为观望，不产出仓位");
}

#{
    action: action,
    verdict: `决策=${action} 置信=${(confidence*1000.0).round()/10.0}% 档=${h} 出场=target_and_falsified 安全边际=${gate_basis}`,
    positionPct: (position*10.0).round()/10.0,
    confidence: (confidence*1000.0).round()/10.0,
    posterior: (posterior*100.0).round()/10.0,
    expectedHoldingDays: days,
    stopLossPct: (stop_pct*100.0).round()/100.0,
    takeProfitPct: (tp_pct*100.0).round()/100.0,
    odds: (odds*100.0).round()/100.0,
    evidenceScale: (escale*1000.0).round()/10.0,
    scoreSource: if present(tier_score) { "tier_native" } else { "daily_fallback" },
    priorSource: prior_src,
    priorSamples: prior_samples,
    stopSource: if exit_move > 0.0 { "thesis_falsified_half_band" } else { "fallback_pct" },
    takeProfitSource: take_profit_source,
    positionSource: "kelly_x_position_multiplier",
    horizon: h,
    confidenceMethod: if spec_ok { branch_json["confidenceMethod"] } else { "missing_branch_table" },
    exitRule: if spec_ok { branch_json["exitRule"] } else { "missing_branch_table" },
    entryGate: if spec_ok { branch_json["entryGate"] } else { "missing_branch_table" },
    entryGatePassed: gate_pass,
    gateBasis: gate_basis,
    timingFiltered: timing_ok,
    falsifiedBy: falsified,
    filterLegs: filter_legs,
    backgroundLegs: background_legs,
    legs: present_legs,
    absentLegs: absent_legs,
    dataGaps: gaps,
}
