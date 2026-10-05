// portfolio-mgr-h-mid.rhai — **中线（28 交易日）** 决策分支（R-11 / PLAN §十一、§二十五）
//
// 与短档的差别不是把天数从 5 改成 28，而是三件事都换了：
// 1. **腿集合**：趋势 / 资金持续性 / 估值带 / 预期修正进方向；盈利质量只作「兑现跨季」的
//    部分来源（constraint=qualityRealizationBeyondHorizon）；解禁供给在本档**降为风险提示**
//    （role=riskNote，权重恒 0）—— 28 日内它已被趋势吸收，再进方向就是重复计数。
// 2. **门形态**：`inside_sigma_band` —— 建仓要求价格仍在均线波动带内（|z| ≤ 2）。
//    短档的门是「两个证据同不同向」，本档的门是「还在不在带里」：中线做的是
//    均值回归意义上的位置，追高（带外向上）时最强的正向证据恰恰是最该回避的。
//    z 的口径在 Rust 侧（`harness::indicators::band_z`），四档共用一份实现。
// 3. **出场口径**：`k_sigma_band` —— 止盈/止损都由带出（不是固定百分比、也不是到期强出）。
//
// ⚠ 禁止出现（门 a）：`leg_mult` / `pm_snr_confidence` / `horizon_decision`。
//    置信同样**不做跨期开方**：持有期已经进了价带，再乘 √(h/28) 等于给同一个时间假设
//    计两次利息 —— 那正是被退役的「四套参数冒充四套分支」。

// 与 `portfolio-mgr.rhai` 同规：`present` 是脚本自带的 1 行守卫（宿主引擎并未注册它，
// 靠宿主注册会让脚本依赖「注册面在不在」，本仓有过 pm_* 静默未注册的前例）。
fn present(x) { type_of(x) != "()" }

let h = "mid";
let gaps = [];
// ── 输入面（与节点 `input_mapping` 一一对应；读了没映射的名字 = 运行期 Variable not found）──
// 已接：branch_json / horizon_prior_json / tier_score / macd_dif / macd_dea / rsi_14 /
//       pe_percentile / f_score / consensus_eps / consensus_estimated / lockup_float_ratio / lockup_supply_reason
//       latest_eps / latest_eps_basis / flow_persistence /
//       kline_bars / overall_risk / stop_vol_mult / take_profit_vol_mult
// 刻意不读（有名字但没有数据面 ⇒ 读它只会打死节点）：
//   · 催化剂（本档腿集无 `eventCatalyst`，任务 #22）
//   · 解禁占比（窗口归约 + 百分数→小数换算未闭合，任务 #23；本档它本来也只是风险提示）
gaps.push("mid 档 supplyShock 只作风险提示（role=riskNote、权重恒 0），不进方向");
// #23（v132）：占比由取数层按本档交易日窗归约后注入。缺席分两种，文案不同 ——
// 「该窗没有解禁」是真值 0（有值、信号为 0），而下面是**无数**，两者不许混写。
if !present(lockup_float_ratio) {
    if present(lockup_supply_reason) {
        gaps.push(`mid 档 supplyShock 缺分母：取数层拿不到行情或流通市值（reason=${lockup_supply_reason}）⇒ 本档连风险提示也无数`);
    } else {
        gaps.push("mid 档 supplyShock 无数据：t-lockup-data 没有产出 supply_shock 块 ⇒ 接线状态，不是「没有解禁」");
    }
}

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


// ── ① 逐腿取信号 ──
// 一致预期 EPS 有一个**板块常数估算**的兜底形态（`ConsensusEPS.is_estimated`，
// `astock-data/src/types.rs:310` 的文档明文禁止下游用它做「超预期/不及预期」判定）：
// 那个数完全可能是某只票的真实一致预期，**数值层面不可区分，只有产出端知道自己是不是编的**
// ⇒ 命中时按缺席处理，而不是拿它算修正幅度（这正是用户裁定③「不用近似数据顶替」的口径）。
let cons_is_estimated = present(consensus_estimated) && consensus_estimated == true;
if cons_is_estimated {
    gaps.push("一致预期是板块常数估算值（is_estimated=true）=> 本档预期修正腿按缺席处理，不用估算值做超预期判定");
}
// #24（v131）：分母 = 最近一个**已披露年报**的 EPS（年度口径，与上面的年度一致预期同族）。
// 缺席分两种、文案不同 —— 合成一句就分不出「这票没有年报」与「估值节点没跑成」：
//   · `basis` 在而 `value` 不在 ⇒ 截止日前没有已披露年报（标的属性；**不做年化外推**）
//   · 两者都不在 ⇒ `t-valuation` 节点整体缺席（它还要行情数据 ⇒ 接线/取数问题）
if !present(latest_eps) {
    if present(latest_eps_basis) {
        gaps.push(`mid 档 expectationRevision 缺分母：截止日前没有已披露年报（basis=${latest_eps_basis}）⇒ 本腿按缺席处理，不拿季报累计值年化顶替`);
    } else {
        gaps.push("mid 档 expectationRevision 缺分母：t-valuation 节点未产出 latestEps ⇒ 接线状态，不是「预期没变化」");
    }
}
let raw = #{
    totalScore: if present(tier_score) { tier_score } else { () },
    macdDif: if present(macd_dif) { macd_dif } else { () },
    macdDea: if present(macd_dea) { macd_dea } else { () },
    rsi14: if present(rsi_14) { rsi_14 } else { () },
    flowPersistence: if present(flow_persistence) { flow_persistence } else { () },
    pePercentile: if present(pe_percentile) { pe_percentile } else { () },
    fScore: if present(f_score) { f_score } else { () },
    // #23（v132）：本档解禁供给占比（小数）。mid 里这条腿是 **riskNote**（权重恒 0、不进方向），
    // 但「有数可提示」与「无数可提示」仍是两件事 —— 接通后 riskNotes 才真能给出该窗口的占比。
    lockupFloatRatio: if present(lockup_float_ratio) { lockup_float_ratio } else { () },
    consensusEps: if present(consensus_eps) && !cons_is_estimated { consensus_eps } else { () },
    latestEps: if present(latest_eps) { latest_eps } else { () },
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

// ── ② 融合：方向腿按在场配比归一；riskNote 腿只出提示，不进 ws/tw ──
let tw = 0.0;
let ws = 0.0;
let risk_notes = [];
for row in present_legs {
    if row["role"] == "direction" {
        tw += row["weight"];
        ws += row["weight"] * row["signal"];
    } else if row["role"] == "riskNote" {
        risk_notes.push(`${row["factor"]}=${row["signal"]}`);
    }
}
if !risk_notes.is_empty() {
    // 只登记「有几条风险提示」，不在文案里拼数组 —— 明细走下面输出的 `riskNotes` 字段，
    // 让面板与门禁读结构化数据，而不是从一句话里反解（本仓已多次为「文案当数据」付过代价）。
    gaps.push(`本档把供给冲击只当风险提示（不进方向），共 ${risk_notes.len()} 条，明细见 riskNotes`);
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

// ── ④ 带门：|z| ≤ 2 才算「仍在带内」（2σ 是带宽定义的行业惯例界，不是调参阈值）──
let bars_ok = present(kline_bars) && type_of(kline_bars) == "array";
let lookback = if spec_ok { branch_json["volLookbackDays"] } else { 20 };
let days = if spec_ok { branch_json["days"] } else { 28 };
// 宿主函数形参是 i64 ⇒ 入参一律先经上方 `spec_int` 归一（分支表里的数字实际是 i64，不是 f64）。
let z = if bars_ok && band_lookback > 0 {
    pm_band_z(kline_bars, band_lookback)
} else {
    ()
};
let gate_state = if type_of(z) == "()" {
    "unjudged"
} else if z > 2.0 {
    "above_band"
} else if z < -2.0 {
    "below_band"
} else {
    "inside"
};
if gate_state == "unjudged" {
    gaps.push("inside_sigma_band 未判定：K 线不足或窗口退化（停盘/一字板 σ=0）⇒ 不给出加仓动作，且不得把「算不出」当「正居中」");
}
if gate_state == "above_band" {
    gaps.push(`带外向上（z=${(z*100.0).round()/100.0} > 2）⇒ 本档封顶持有：中线的正向证据在追高时不构成加仓理由`);
}
// 带内 ⇒ 出场由带出：止盈取带顶方向（k_take 个到期位移），止损取 k_stop 个。
let exit_move =
    if bars_ok && band_days > 0 && band_lookback > 0 {
    pm_vol_move_pct(kline_bars, band_lookback, band_days)
} else {
    0.0
};
let k_stop = if present(stop_vol_mult) { stop_vol_mult } else { 1.2 };
let k_take = if present(take_profit_vol_mult) { take_profit_vol_mult } else { 2.0 };
let stop_pct = exit_move * k_stop;
let tp_pct = exit_move * k_take;
let odds = if stop_pct > 0.0 { tp_pct / stop_pct } else { 0.0 };
let kelly = if confidence >= 0.50 && odds > 0.001 {
    pm_kelly_position(confidence, odds, 0.0015, overall_risk)
} else {
    0.0
};
let position = kelly * (if spec_ok { branch_json["positionMultiplier"] } else { 1.0 });

// ── ⑤ 动作阶梯（本档独立常量集；门 e 要求不得由他档常量 ± 偏移派生）──
let allow_open = gate_state == "inside" || gate_state == "below_band";
let action = if confidence >= 0.72 && allow_open { "买入" }
    else if confidence >= 0.62 && allow_open { "增持" }
    else if confidence >= 0.52 { "持有" }
    else if confidence >= 0.42 { "观望" }
    else if confidence >= 0.32 { "减持" }
    else { "卖出" };
if !allow_open && confidence >= 0.62 {
    action = "持有";
}
if tw <= 0.0 || !prior_ok {
    action = "观望";
    position = 0.0;
    gaps.push("本档无在场方向腿或先验不可得 ⇒ 动作退为观望，不产出仓位");
}

#{
    action: action,
    verdict: `决策=${action} 置信=${(confidence*1000.0).round()/10.0}% 档=${h} 出场=k_sigma_band 带位=${gate_state}`,
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
    stopSource: if exit_move > 0.0 { "sigma_band" } else { "fallback_pct" },
    positionSource: "kelly_x_position_multiplier",
    horizon: h,
    confidenceMethod: if spec_ok { branch_json["confidenceMethod"] } else { "missing_branch_table" },
    exitRule: if spec_ok { branch_json["exitRule"] } else { "missing_branch_table" },
    entryGate: if spec_ok { branch_json["entryGate"] } else { "missing_branch_table" },
    entryGatePassed: allow_open,
    bandZ: if type_of(z) == "()" { () } else { (z*100.0).round()/100.0 },
    riskNotes: risk_notes,
    legs: present_legs,
    absentLegs: absent_legs,
    dataGaps: gaps,
}
