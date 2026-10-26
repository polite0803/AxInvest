// portfolio-mgr-h-long.rhai — **长线（90 交易日）** 决策分支（R-11 / PLAN §十一、§二十五）
//
// 本档的算法分支在**三处**与其余三档不同，且每处都是换了判据而不是换了倍数：
// 1. 腿集合：方向只由「盈利质量 / 预期修正 / 宏观状态」三条方向腿决定。
//    估值腿 `valuationBand` 自 P0-3（PLAN-stock-decision-reliability，2026-10-16）起
//    **role=filter** —— 90 日窗内估值无方向预测力（实证 10-09 紫金 DCF 放行长档买入），
//    只作严重高估否决，便宜不再是买入理由（权重恒 0 由分支表保证，不靠这里自觉）。
//    技术腿 `trendStrength` 同为 filter（R-11）。行业轮动是 `partial`
//    （live 有源、回放结构性不可得 ⇒ 只作背景）。
// 2. 门：`positive_margin_of_safety`（门名保留，语义 P0-3 反转为**单向高估否决**）——
//    旧口径「DCF 上行 ≥20% 或 PE≤25 分位才放行」把便宜当买入必要条件，会在周期顶部被
//    乐观 DCF 放行；新口径：DCF upside ≤ -35%（内在价值较现价低 35%+）或 PE ≥ 自身
//    5 年 80 分位 ⇒ 否决开仓，其余可判情形放行，两口径都缺席按「未判定 ≠ 通过」不加仓。
//    basis 取值：not_overvalued / dcf_overvalued / pe_overvalued / unjudged。
//    周期组（P0-4）另走上限口径：中性化 DCF 上行 ≥ 面板 `value_safety_margin`（安全边际
//    最低折扣，默认 20）与周期下限 35% 的**较大者** —— 基准仍读面板那一个来源（宿主函数
//    `pm_required_margin_pct()`），周期下限是脚本自己的、更严的那一格。
// 3. 出场：`target_and_falsified` —— 止盈 = 估值目标价；止损 = **论点被证伪**
//    （估值严重高估、上行依据耗尽 ⇒ `margin_basis_exhausted`，与门同尺 -35%；
//    或盈利质量/预期修正两腿同时转负 ⇒ `thesis_contradicted`）。价格波动本身不构成
//    长线止损理由，这是本档与其余三档最根本的差别：短档的止损买的是「时间窗内的位移
//    控制」，长档买的是「论点还成立」。
//
// ⚠ 禁止出现（门 a）：`leg_mult` / `pm_snr_confidence` / `horizon_decision`。

// 与 `portfolio-mgr.rhai` 同规：`present` 是脚本自带的 1 行守卫（宿主引擎并未注册它，
// 靠宿主注册会让脚本依赖「注册面在不在」，本仓有过 pm_* 静默未注册的前例）。
fn present(x) { type_of(x) != "()" }

// P0-3 严重高估阈值（门与 falsified 共用一把尺）。
// ⚠ 必须用无参函数而非顶层 `const`：rhai 只把**编译 scope**（`compile_with_scope`）里的常量
//   传播进函数体；本脚本由生产执行器 `engine.compile(code)` 无 scope 编译（见
//   `rhai_ast_cache.rs`），顶层 const 对同脚本函数不可见 —— 函数内引用会在运行期
//   `Variable not found`（2026-10-16 实测，call_fn 与 eval_ast 两种入口同报）。
//   无参函数走 AST 函数库解析，函数内外同读一份，阈值仍是脚本内单点权威。
fn overval_dcf_pct() { -35.0 }
fn overval_pe_pctile() { 80.0 }
// P0-4③：周期股安全边际门阈值（20% → 35%）。周期股估值中枢随商品价格大幅摆动，
// 且本口径的 DCF 已用近 5 年 FCF 中位数中性化、PE 分位已在工具侧置空 ——
// 中性化后仍要求 ≥35% 上行才允许开仓（旧门的「便宜放行」语义只在周期组保留）。
// ⚠ `35` 是周期组**自己的下限**，不是面板那一格的抄写：面板 `value_safety_margin`（安全
//   边际最低折扣，默认 20、缺失/越界回落 30）是全局下限，宿主函数 `pm_required_margin_pct()`
//   原样转发它 ⇒ 两者取**较大者**。面板调到 40 时周期门跟着升到 40，「面板那格谁也不读」
//   的老形态就此不复现；面板缺席（回落 30）时周期门仍钉在 35，不随回落值漂移。
//   B 批跨载体门禁 `check-panel-var-landing.mjs` 的 P2/P3 钉的正是这一环：`.rhai` 里必须能
//   寻到 `pm_required_margin_pct()`，且不许回潮成 `moS_pct >= 30.0` 那种第二权威字面量。
fn cycle_margin_floor_pct() { 35.0 }
fn cycle_margin_min_pct() {
    if pm_required_margin_pct() > cycle_margin_floor_pct() {
        pm_required_margin_pct()
    } else {
        cycle_margin_floor_pct()
    }
}

// P0-3 单向高估否决（门的纯判定，抽出以便 call_fn 钉死，防阈值被人改回「便宜放行」）。
// 入参：mos_pct —— DCF 上行百分数，DCF 不适用/缺失时传 ()；
//       pe_pct  —— PE 自身 5 年分位（0-100），缺失时传 ()；
//       cyclical —— P0-4：是否命中周期组（读 dcf.assumptions.cyclical_normalized）。
// 返回 `[pass, basis]`：
//   · 普通股（P0-3，单向否决）：
//     [false, "dcf_overvalued"]：DCF 内在价值较现价低 35%+（upside ≤ -35）
//     [false, "pe_overvalued"]：PE ≥ 80 分位（valuation_band overvalued=75 再留半档缓冲）
//     [true,  "not_overvalued"]：至少一条口径可判且未严重高估（便宜不再放行，也不拦下）
//     [false, "unjudged"]：两条口径都缺席（缺席 ≠ 通过）
//   · 周期股（P0-4，PE 已在工具侧置空，只剩中性化 DCF 单口径）：
//     [true,  "cyclical_margin_pass"]：中性化 DCF upside ≥ `cycle_margin_min_pct()`
//       （= max(面板 `value_safety_margin`, 周期下限 35)，面板默认 ⇒ 35）
//     [false, "cyclical_margin_insufficient"]：upside 存在但低于该阈值（含轻微高估）
//     [false, "unjudged"]：DCF 不适用/缺席
fn pm_overvalued_veto(mos_pct, pe_pct, cyclical) {
    if cyclical {
        if type_of(mos_pct) == "()" {
            return [false, "unjudged"];
        }
        if mos_pct >= cycle_margin_min_pct() {
            return [true, "cyclical_margin_pass"];
        }
        return [false, "cyclical_margin_insufficient"];
    }
    if type_of(mos_pct) != "()" && mos_pct <= overval_dcf_pct() {
        return [false, "dcf_overvalued"];
    }
    if type_of(pe_pct) != "()" && pe_pct >= overval_pe_pctile() {
        return [false, "pe_overvalued"];
    }
    if type_of(mos_pct) != "()" || type_of(pe_pct) != "()" {
        return [true, "not_overvalued"];
    }
    return [false, "unjudged"];
}

let h = "long";
let gaps = [];
// ── 输入面（与节点 `input_mapping` 一一对应；读了没映射的名字 = 运行期 Variable not found）──
// 已接：branch_json / horizon_prior_json / tier_score / macd_dif / macd_dea / rsi_value /
//       pe_percentile / f_score / consensus_eps / consensus_estimated /
//       latest_eps / latest_eps_basis / pmi /
//       valuation_dcf_upside / valuation_dcf_applicable / valuation_dcf_cyclical /
//       kline_bars / overall_risk / stop_vol_mult / scoring_windows / scoring_scale / scale_trend / scale_momentum
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
// ── P1-8（`PLAN-stock-decision-reliability.md` R10）：宏观块时效门 ──
// 超滞（或发布日缺失）⇒ macroRegime 腿**按缺席处理**（不进 raw 的 pmi ⇒ 腿自然缺席、
// 不占权重、进 absent_legs）并如实进 gaps；旧实现把「GDP 停在 Q1、CPI/PPI 是 8 月值」的
// 残缺块按约 18% 名义权重喂给长档（10-09 紫金实证）。
let macro_stale = pm_macro_stale_reason(
    if present(macro_snapshot_date) { macro_snapshot_date } else { () },
    if present(gdp_release_date) { gdp_release_date } else { () },
    if present(cpi_release_date) { cpi_release_date } else { () },
    if present(ppi_release_date) { ppi_release_date } else { () },
    if present(pmi_release_date) { pmi_release_date } else { () },
);
let macro_fresh = if type_of(macro_stale) == "string" && macro_stale != "" {
    gaps.push(`宏观数据时效门：${macro_stale}`);
    false
} else {
    true
};
let raw = #{
    totalScore: if present(tier_score) { tier_score } else { () },
    macdDif: if present(macd_dif) { macd_dif } else { () },
    macdDea: if present(macd_dea) { macd_dea } else { () },
    rsi: if present(rsi_value) { rsi_value } else { () },
    pePercentile: if present(pe_percentile) { pe_percentile } else { () },
    fScore: if present(f_score) { f_score } else { () },
    consensusEps: if present(consensus_eps) && !cons_is_estimated { consensus_eps } else { () },
    latestEps: if present(latest_eps) { latest_eps } else { () },
    pmi: if present(pmi) && macro_fresh { pmi } else { () },
};
let present_legs = [];
let absent_legs = [];
for row in legs {
    let s = pm_leg_signal(row["factor"], raw);
    if type_of(s) == "()" {
        absent_legs.push(row["factor"]);
    } else {
        // 裁决层（leg_weight_overlay）的标记必须随腿落库：否则审计分不清
        // 「测出无预测力、主动置零」与「没接线/没数」。
        let out = #{ factor: row["factor"], role: row["role"], weight: row["weight"], signal: s, constraint: row["constraint"] };
        if type_of(row["icStatus"]) != "()" { out.icStatus = row["icStatus"]; }
        if row["icZeroed"] == true { out.icZeroed = true; }
        present_legs.push(out);
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

// ── ④ 估值高估否决门（P0-3，PLAN-stock-decision-reliability）──
// 2026-10-16 语义反转：旧门 `positive_margin_of_safety` 把「DCF 上行 ≥20% 或 PE≤25 分位」
// 当作买入**必要条件**（便宜才放行）。实证（10-09 紫金矿业：DCF 上行 25.7% 放行长档买入，
// 90 日内跌破决策价）表明估值在 90 日窗内没有方向预测力，周期顶部的乐观 DCF 反成开仓依据。
// 新口径是**单向否决**：便宜不再是买入理由（方向 posterior 已只由
// 盈利质量 / 预期修正 / 宏观三条方向腿决定，估值腿在分支表中已是 filter、权重 0），
// 只有**严重高估**才否决开仓：
//   · DCF 内在价值较现价低 35%+（upside ≤ -35%，dcf_overvalued）；
//   · 或 PE 处自身 5 年 80 分位以上（pe_overvalued；valuation_band 的 overvalued=75 再留半档缓冲）。
// 至少一条口径可判且未严重高估 ⇒ not_overvalued（放行，动作由 posterior 决定）；
// 两条口径都缺席 ⇒ unjudged，沿用「缺席 ≠ 通过」，不出加仓（与缺数不加仓语义一致）。
let dcf_ok = present(valuation_dcf_upside) && present(valuation_dcf_applicable) && valuation_dcf_applicable == true;
let moS_pct = if dcf_ok { valuation_dcf_upside } else { () };
// P0-4：周期开关标记由估值工具在 DCF 锚被近 5 年 FCF 中位数盖写时置 true
// （dcf.assumptions.cyclical_normalized）。缺失/false 都按普通股单向否决门走。
let dcf_cyclical = present(valuation_dcf_cyclical) && valuation_dcf_cyclical == true;
let veto = pm_overvalued_veto(
    moS_pct,
    if present(pe_percentile) { pe_percentile } else { () },
    dcf_cyclical
);
let gate_pass = veto[0];
let gate_basis = veto[1];
// ── 估值门开关（2026-10-26 用户裁决）────────────────────────────────
// 门只**单向否决**（拦买入/增持、不抬信号）；放开 ⇒ 本档不再因严重高估被否决。
// ⚠ 放开后必须**留痕**：否则「估值门未过」与「用户选择放开该门」在界面上完全同形。
// ⚠「unjudged」（DCF 不适用且 PE 分位缺失 ⇒ 缺席 ≠ 通过）**不是高估否决**，
//   它是**数据缺席**，放开高估门不应把它变成「通过」⇒ 下面只对「已判定为高估」的两档生效。
let valuation_gate_open = pm_gate_enabled("gate.valuation_enabled");
let valuation_overvalued_rejected = !gate_pass && gate_basis != "unjudged";
if gate_basis == "unjudged" && posterior >= 0.62 {
    gaps.push(if dcf_cyclical {
        "周期股估值门未判定：周期中性化 DCF 不适用/缺席 ⇒ 不给出加仓动作（缺席 ≠ 通过）"
    } else {
        "估值否决门未判定：DCF 不适用且 PE 分位缺失 ⇒ 不给出加仓动作（缺席 ≠ 通过）"
    });
} else if valuation_overvalued_rejected && posterior >= 0.62 {
    if valuation_gate_open {
        gaps.push(`估值门已放开（原判据：${gate_basis}）⇒ 本档不因严重高估被否决`);
    } else if gate_basis == "dcf_overvalued" {
        gaps.push(`估值高估否决：DCF 内在价值较现价低 35%+（upside=${(moS_pct*10.0).round()/10.0}%）⇒ 本档不出买入/增持（便宜不是买入理由，严重高估才拦下）`)
    } else if gate_basis == "pe_overvalued" {
        gaps.push(`估值高估否决：PE 处自身 5 年 ${overval_pe_pctile()} 分位以上（${(pe_percentile*10.0).round()/10.0}）⇒ 本档不出买入/增持`)
    } else {
        // cyclical_margin_insufficient
        gaps.push(`周期股估值门（P0-4）：近5年FCF中位数中性化 DCF 上行 ${(moS_pct*10.0).round()/10.0}% < 安全边际要求 ${cycle_margin_min_pct()}%（PE 分位已因周期扭曲置空）⇒ 本档不出买入/增持`)
    }
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
// P0-3：与估值高估否决门同尺（upside ≤ -35% 才算估值依据耗尽）。
// 旧界 moS≤0 在「便宜不是理由」的新口径下过紧 —— 轻微高估不应直接判论点证伪。
if type_of(moS_pct) != "()" && moS_pct <= overval_dcf_pct() {
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
// v156（终审发现 ①）：档位乘数后**再过一次风险封顶** —— `pm_kelly_position` 内部已按风险档
// 封顶，若乘数（长档 1.2）后置生效而不复封，"极高风险上限 10%"会被放大成 12%。
// `pm_risk_cap` 与 kelly 内部用同一个上限表（`portfolio_formula::apply_risk_cap`）。
let position = pm_risk_cap(kelly * (if spec_ok { branch_json["positionMultiplier"] } else { 1.2 }), overall_risk);

// ── ⑦ 动作阶梯（本档独立常量集；加仓必须先过安全边际门，再不被时机过滤降档）──
let action = if confidence >= 0.72 { "买入" }
    else if confidence >= 0.62 { "增持" }
    else if confidence >= 0.52 { "持有" }
    else if confidence >= 0.42 { "观望" }
    else if confidence >= 0.32 { "减持" }
    else { "卖出" };
if !gate_pass && !valuation_gate_open && (action == "买入" || action == "增持") {
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

// ── ⑦′ 档内风控否决（2026-10-10 B 裁定）──
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
    verdict: `决策=${action} 置信=${(confidence*1000.0).round()/10.0}% 档=${h} 出场=target_and_falsified 估值门=${gate_basis}${tier_veto_note}`,
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
    stopSource: if exit_move > 0.0 { "thesis_falsified_half_band" } else { "fallback_pct" },
    takeProfitSource: take_profit_source,
    positionSource: "kelly_x_position_multiplier",
    horizon: h,
    confidenceMethod: if spec_ok { branch_json["confidenceMethod"] } else { "missing_branch_table" },
    exitRule: if spec_ok { branch_json["exitRule"] } else { "missing_branch_table" },
    entryGate: if spec_ok { branch_json["entryGate"] } else { "missing_branch_table" },
    // ⚠ 门放开后 `entryGatePassed` 必须报 true（门已放行 ⇒ 入场门**已通过**），
    //   否则呈现层显示「入场门未过」而实际是「用户选择放开该门」⇒ 同屏误导。
    //   ⚠「unjudged」（数据缺席）**不因放开而变成通过** —— 缺席 ≠ 通过是数据契约。
    entryGatePassed: gate_pass || (valuation_gate_open && gate_basis != "unjudged"),
    gateBasis: gate_basis,
    // 估值门是否已被产品决策放开（2026-10-26）—— 呈现层据此显示「门已放开」而非「门未过」
    valuationGateOpen: valuation_gate_open,
    timingFiltered: timing_ok,
    falsifiedBy: falsified,
    filterLegs: filter_legs,
    backgroundLegs: background_legs,
    legs: present_legs,
    absentLegs: absent_legs,
    dataGaps: gaps,
}
