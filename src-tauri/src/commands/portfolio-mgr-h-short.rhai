// portfolio-mgr-h-short.rhai — **短线（5 交易日）** 决策分支（R-11 / PLAN §十一、§二十五）
//
// 与超短分支的差别不是参数，是三处判据：
// 1. 参与腿不同（本档有 4 条方向腿：动量 / 广度 / 资金持续性 / 趋势 / 供给冲击）；
// 2. 门形态不同：超短是「市场级广度不够 ⇒ 不许买」的**外部门**，本档是
//    `trend_and_flow_same_sign` —— 趋势与资金持续性必须**同号**才允许加仓。
//    这是联合条件而不是加权平均：两者背离时（价涨量撤 / 量增价滞）任何线性合成
//    都会给出一个看似中性的正分，而 5 日窗里背离本身就是最常见的顶部形态。
// 3. 出场口径不同：`time_stop+fixed_stop` —— 到期出 + 固定比例硬止损（超短没有第二条）。
//
// ⚠ 禁止出现（门 a）：`leg_mult` / `pm_snr_confidence` / `horizon_decision`。
//    置信同样**不做跨期缩放**：持有期已经进了价带（`pm_vol_move_pct(...,days)`），
//    再乘一次 √days 是给同一个时间假设计两次利息。

// 与 `portfolio-mgr.rhai` 同规：`present` 是脚本自带的 1 行守卫（宿主引擎并未注册它，
// 靠宿主注册会让脚本依赖「注册面在不在」，本仓有过 pm_* 静默未注册的前例）。
fn present(x) { type_of(x) != "()" }

let h = "short";
let gaps = [];
// ── 输入面（与节点 `input_mapping` 一一对应；读了没映射的名字 = 运行期 Variable not found，
//    不是缺席。判据与来历见 PLAN §三十五）──
// 已接：branch_json / horizon_prior_json / tier_score / macd_dif / macd_dea / rsi_value /
//       lockup_float_ratio / lockup_supply_reason /     seal_rate / flow_persistence / kline_bars / overall_risk / stop_vol_mult / take_profit_vol_mult / scoring_windows / scoring_scale / scale_trend / scale_momentum
// 刻意不读（读了也没数据源 ⇒ 越界读取会打死节点）：
//   · 催化剂 —— 本档腿集里没有 `eventCatalyst`（PLAN §十一 与权威表的分歧，任务 #22 收编）
//   · 解禁供给 —— **v132 已接**（取数层按本档交易日窗归约的小数占比），缺席走下面条件式
if !present(lockup_float_ratio) {
    if present(lockup_supply_reason) {
        gaps.push(`short 档 supplyShock 缺分母：取数层拿不到行情或流通市值（reason=${lockup_supply_reason}）⇒ 本腿按缺席处理，不得当成「无解禁压力」`);
    } else {
        gaps.push("short 档 supplyShock 缺席：t-lockup-data 没有产出 supply_shock 块 ⇒ 接线状态，不是「没有解禁」");
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
// ── v133（B2-2）：本档分析师子集（权威来源 `branch_json["analysts"]`，Rust 侧由
//    `Period::analyst_subset()` 推导）──
// 用途：verdict 类输入（本档 `flow_persistence` ← `a-hot-money--short`）与「本档子集」
// 应互为必要条件 —— **子集里没有属主却有值**只可能是接线串档（读了他档实例），
// 必须留痕且不采信（「缺席不伪装」的反面：不该来的值也不能默默地来）。
let tier_analysts = if spec_ok && type_of(branch_json["analysts"]) == "array" {
    branch_json["analysts"]
} else {
    []
};
let hm_owner_ok = !spec_ok || ("a-hot-money" in tier_analysts);
if present(flow_persistence) && !hm_owner_ok {
    gaps.push("flow_persistence 有值但 a-hot-money 不在本档子集 ⇒ 接线串档（应指向本档实例），该值不予采信");
}
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
let raw = #{
    totalScore: if present(tier_score) { tier_score } else { () },
    macdDif: if present(macd_dif) { macd_dif } else { () },
    macdDea: if present(macd_dea) { macd_dea } else { () },
    rsi: if present(rsi_value) { rsi_value } else { () },
    flowPersistence: if present(flow_persistence) && hm_owner_ok { flow_persistence } else { () },
    sealRate: if present(seal_rate) { seal_rate } else { () },
    // #23（v132）：本档解禁供给占比（小数，取数层已按该档交易日窗归约）
    lockupFloatRatio: if present(lockup_float_ratio) { lockup_float_ratio } else { () },
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

// ── ② 融合（缺席腿不参与归一：缺席 ≠ 中性 0）──
let tw = 0.0;
let ws = 0.0;
for row in present_legs {
    if row["role"] != "direction" { continue; }
    tw += row["weight"];
    ws += row["weight"] * row["signal"];
}
let avg = if tw > 0.0 { ws / tw } else { 0.0 };
let dir_count = 0;
let dir_total = 0.0;
for row in legs {
    if row["role"] == "direction" { dir_total += row["weight"]; dir_count += 1; }
}
let live_dir = 0;
for row in present_legs {
    if row["role"] == "direction" { live_dir += 1; }
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
// P2-c（PLAN-stock-decision-reliability）：本档**原始方向命中率**（未收缩口径）。
// 缺席 ⇒ 前端不渲染命中率段 —— 先验此时借自全档合并基准，不得拿 pooled 值冒充本档命中率。
let prior_hit_rate = if prior_ok { num_of(hp_row["hitRate"]) } else { () };
// P1-3（R3/R7）：本档校准可得性。判据在 Rust `reflection_stats::CALIBRATION_MIN_SAMPLES`，
// 脚本只读结论（同 DQI 熔断纪律：阈值有两份载体就早晚不一致）。先验不可得 ⇒ false ⇒
// 主链按「未校准」封顶 —— fail-closed，缺席不得当成「已校准」。
let prior_cal_ready = prior_ok && hp_row["calibrationReady"] == true;
if !prior_ok {
    gaps.push("本档先验不可得（`horizon_prior_json` 缺失或缺本档那一行）=> 拒绝出结论：观望且不产出仓位");
}
let escale = if dir_total > 0.0 { pm_evidence_scale(tw, dir_total) } else { 0.0 };
let posterior = if prior_ok { clamp(prior_val + avg * escale, 0.0, 1.0) } else { 0.0 };
let confidence = posterior;

// ── ④ 联合门：趋势 × 资金同号 ──
// 只在两者**都有数**时判定；任一缺席 ⇒ 记为「未判定」并按不通过处理（缺席不等于通过）。
let trend_sig = pm_leg_signal("trendStrength", raw);
let flow_sig = pm_leg_signal("flowPersistence", raw);
let gate_state = if type_of(trend_sig) == "()" || type_of(flow_sig) == "()" {
    "unjudged"
} else if trend_sig * flow_sig > 0.0 {
    "pass"
} else {
    "conflict"
};
if gate_state != "pass" {
    gaps.push(if gate_state == "unjudged" {
        "trend_and_flow_same_sign 未判定：趋势或资金持续性缺数 ⇒ 不给出加仓动作（缺席 ≠ 通过）"
    } else {
        "trend_and_flow_same_sign 冲突：趋势与资金反向 ⇒ 5 日窗内背离本身即顶部形态，封顶持有"
    });
}
// 本档的第二条结构条件：方向腿**至少两条在场**。5 日窗不足以让单因子赌注兑现，
// 一条腿出买入与抛硬币同分布 —— 这是判据，不是保守偏好。
let min_legs_ok = live_dir >= 2;
if !min_legs_ok {
    gaps.push(`在场方向腿仅 ${live_dir} 条（本档要求 >= 2）⇒ 证据太薄，不出方向`);
}

// ── ⑤ 价带：到期位移 × 倍数，止损与止盈都按带出（出场口径含固定止损）──
let bars_ok = present(kline_bars) && type_of(kline_bars) == "array";
let lookback = if spec_ok { branch_json["volLookbackDays"] } else { 20 };
let days = if spec_ok { branch_json["days"] } else { 5 };
// 宿主函数形参是 i64 ⇒ 入参一律先经上方 `spec_int` 归一（分支表里的数字实际是 i64，不是 f64）。
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
// v156（终审发现 ①）：档位乘数后**再过一次风险封顶** —— `pm_kelly_position` 内部已按风险档
// 封顶，若乘数（长档 1.2）后置生效而不复封，"极高风险上限 10%"会被放大成 12%。
// `pm_risk_cap` 与 kelly 内部用同一个上限表（`portfolio_formula::apply_risk_cap`）。
let position = pm_risk_cap(kelly * (if spec_ok { branch_json["positionMultiplier"] } else { 0.8 }), overall_risk);

// ── ⑥ 动作阶梯（本档独立常量集，门 e 锁「不得由他档常量 ± 偏移派生」）──
let capped = gate_state != "pass" || !min_legs_ok;
let action = if !min_legs_ok { "观望" }
    else if capped && posterior >= 0.62 { "持有" }
    else if confidence >= 0.72 { "买入" }
    else if confidence >= 0.62 { "增持" }
    else if confidence >= 0.52 { "持有" }
    else if confidence >= 0.42 { "观望" }
    else if confidence >= 0.32 { "减持" }
    else { "卖出" };
if tw <= 0.0 || !prior_ok {
    action = "观望";
    position = 0.0;
    gaps.push("本档无在场方向腿或先验不可得 ⇒ 动作退为观望，不产出仓位");
}

// ── ⑥′ 档内风控否决（2026-10-10 B 裁定）──
// 否决必须落在**本档结论**上，否则呈现层会出现「主档观望 / 本档 tag 买入」的同屏分裂
//   （实证兆易创新 603986：中期档 tag 买入、主档被主链否决为观望，看着像两套结论）。
// 本档 `overall_risk` 来自本档风险节点（`cls-risk-level-<档>`），与主档拿的风险档同源。
// `original_action` 保留否决前的方向值：下游冲突判定（`decision.rs`）与呈现层「何以降级」
//   都读它 —— 否决是政策叠加，不是方向改判，方向信息不许被政策吞掉。
// 幂等：主链仍会对被选档再否决一次（算法档可能比本档更严 ⇒ 仍要收紧），而
//   `pm_risk_veto` 对已否决值不变（极高→观望 / 高→持有 再否决仍是自身）⇒ 主链那次天然 no-op。
let original_action = action;
let tier_vetoed = false;
if present(overall_risk) && type_of(overall_risk) == "string" {
    action = pm_risk_veto(action, overall_risk);
    tier_vetoed = original_action != action;
}
let tier_veto_note = "";
if tier_vetoed {
    gaps.push(`本档风控否决:${original_action}→${action}(本档风险档=${overall_risk})`);
    tier_veto_note = ` 风控=档内否决(${original_action}→${action})`;
}

// v156 一致性修复：仓位只属于「持有及以上」的动作 ——「观望/减持/卖出」= 目标仓位 0。
// 修两条产出「观望 + 正仓位」的路径（现网实证 002241/002080 与 002384/300045）：
//   ① kelly 门槛（conf ≥ 0.50）低于「持有」阶梯（0.52）⇒ [0.50, 0.52) 算得出正仓位却落「观望」；
//   ② 档内风控否决把动作折到「观望」时未清零仓位。
if action != "买入" && action != "增持" && action != "持有" {
    position = 0.0;
}

#{
    action: action,
    // 档内否决前的方向值：读侧「何以降级」与冲突判定读它，不得从 verdict 文案反解。
    originalAction: original_action,
    tierVetoed: tier_vetoed,
    verdict: `决策=${action} 置信=${(confidence*1000.0).round()/10.0}% 档=${h} 出场=time_stop+fixed_stop 联合门=${gate_state}${tier_veto_note}`,
    positionPct: (position*10.0).round()/10.0,
    confidence: (confidence*1000.0).round()/10.0,
    posterior: (posterior*100.0).round()/10.0,
    expectedHoldingDays: days,
    stopLossPct: (stop_pct*100.0).round()/100.0,
    takeProfitPct: (tp_pct*100.0).round()/100.0,
    odds: (odds*100.0).round()/100.0,
    evidenceScale: (escale*1000.0).round()/10.0,
    scoreSource: if present(tier_score) { "tier_native" } else { "daily_fallback" },
    // v138 裁定 3：本档实际用的指标窗口与尺度（产端 = 评分节点回显；缺席 = 早于 v138 的代 ⇒ 面板不渲染）
    scoringWindows: if present(scoring_windows) { scoring_windows } else { () },
    scoringScale: if present(scoring_scale) { scoring_scale } else { () },
    scoringTrend: if present(scale_trend) { scale_trend } else { () },
    scoringMomentum: if present(scale_momentum) { scale_momentum } else { () },
    // v140：本档风险档随分支行一起带回父池 —— 风险节点搬进子模板后，主链 `overall_risk_<档>`
    //   读的就是这里（`pm-h-<档>.result.riskCategory`）。漏这一行 = 按档风险收紧整条静默退役。
    //   缺席只有一种可能：本档风险节点自己失败 ⇒ 整档子执行已经失败 ⇒ 这行不会单独缺。
    riskCategory: if present(overall_risk) { overall_risk } else { () },
    priorSource: prior_src,
    priorSamples: prior_samples,
    calibrationReady: prior_cal_ready,
    priorHitRate: prior_hit_rate,
    stopSource: if exit_move > 0.0 { "vol_band" } else { "fallback_pct" },
    positionSource: "kelly_x_position_multiplier",
    horizon: h,
    confidenceMethod: if spec_ok { branch_json["confidenceMethod"] } else { "missing_branch_table" },
    exitRule: if spec_ok { branch_json["exitRule"] } else { "missing_branch_table" },
    entryGate: if spec_ok { branch_json["entryGate"] } else { "missing_branch_table" },
    entryGatePassed: gate_state == "pass",
    liveDirectionLegs: live_dir,
    requiredDirectionLegs: 2,
    legs: present_legs,
    absentLegs: absent_legs,
    dataGaps: gaps,
}
