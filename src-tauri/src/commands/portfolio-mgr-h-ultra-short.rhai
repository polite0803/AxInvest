// portfolio-mgr-h-ultra-short.rhai — **超短（2 交易日）** 决策分支（R-11 / PLAN §十一、§二十五）
//
// 与其它三档的关系不是「同一算法换参数」：本分支的参与腿、门形态、置信口径、出场口径
// 全部来自注入的 `branch_json`（权威源 `analysis-engine::evidence_weight::horizon_branch_specs`），
// 融合与判据写在本文件里 ⇒ 四档各有一份**真的不同**的算法。
//
// ⚠ 本脚本禁止出现（门 a 会红）：`leg_mult` / `horizon_leg_multipliers` / `pm_snr_confidence` /
//   `horizon_decision` —— 那三个正是被 R-11 退役的「四套参数冒充四套分支」的形态。
//   尤其 `pm_snr_confidence(heff, days, 28)`：√(h/28) 是**档间唯一的差异来源**时，
//   四档其实是同一个数；本分支改为「2 日窗内不做任何跨期缩放」（超短的持有期短于
//   该变换的假设区间，信噪比折算在此无意义 —— 这是退役的理由而不是偏好）。
//
// 超短的三条专属判据（`branch_json.entryGate = "breadth_required"`）：
// 1. **情绪广度是必要条件不是权重**：封板率 < 0.5（触板的票一半以上封不住）时，
//    个股形态再好也不出「买入/增持」—— 涨停家数与封板率是市场级量，把它当加权腿
//    会允许「广度很差但个股很强」合成出买入，那正是超短最贵的错。
// 2. 方向证据只允许三腿（动量 / 广度 / 催化剂），基本面与估值**按构造不适用**
//    （兑现周期 > 持有期，见 PLAN §十二）⇒ 不得为了让分母好看而补腿。
// 3. 出场 = `time_stop`：持有到期即出，止损只是尾部保护（1 个日波动），
//    不设「目标价止盈」—— 2 日窗内的目标价没有分布含义。

// ── 输入（由节点 input_mapping 注入；缺席一律 present() 守卫，不补默认值）──
// 「已接」= 种子里真有这条映射；「未接」= 本脚本读得到但值为 unit ⇒ 走缺席分支并在 dataGaps 留痕，
// 这两类必须分开写：把未接的写成已接，就是「present 守卫 ⇒ 恒假 ⇒ 该腿静默不激活」的开端
// （判据见 `scripts/audit-inject-coverage.mjs` ①）。
// branch_json        : 已接 —— `horizon_branch_json.ultra_short`（权威源 evidence_weight::horizon_branch_specs）
//                      档位标签不在入参里：本脚本硬编码 `h`，与 `branch_json["tier"]` 对拍 ⇒ 接线错配即点名
// tier_score         : 已接 —— t-scoring-hour(60 分钟) 的 totalScore
// macd_dif/macd_dea : 已接 —— 本档尺度指标（momentumSignal 的原料）
// rsi_value        : v136 起接本档尺度算出的那个周期的 RSI（scaleMomentum.value）。
//                      旧键 rsi_14 指的是日线命名槽 rsi14：月/季档的周期不落进 {6,12,14,24} 时
//                      它停在初值 50.0，恰好命中调制区间 = 给每一档凭空 +0.2（PLAN §一○二）。
// scoring_windows    : v138 起接 —— 本档评分节点**实际用过**的指标窗口回显（`indicators.windows`，
//                      裁定 3「让用户看出各档实际几根」，PLAN §一○六）。产端是回显不是「想要的配置」，
//                      所以面板印出来的那组就是这一档算过的那组。
// scoring_scale      : v138 起接 —— 本档尺度名（hourly/weekly/monthly/quarterly），与窗口并列成句；
//                      短/中/长三档的根数按公式本就是同一组，差别只在这里才可读
// seal_rate          : 已接 —— 涨停池 breadth.sealRate（breadthState 腿 + breadth_required 门的必要条件）
// pool_break_count   : 已接 —— 涨停池 breadth.breakCount（microstructure 条件腿）
// pool_in_pool       : 未接且本脚本**不读** —— 语义是「本代码 ∈ entries[]」，点号路径表达不了
//                      数组包含；下一片改成注入池子串 + 脚本内 parse 后，这个键才随映射一起回来
// catalyst_level     : 未接且本脚本**不读** —— PLAN §十一 把催化剂列为超短三腿之一，但权威表
//                      （`harness::holding_period::verdict_spec` 的 UltraShort.participating）
//                      里没有 eventCatalyst ⇒ 接了也没有腿消费；先按缺席，等契约补齐再接回
//                      ⚠ 这条作废现在**机器可见**（见下方 gaps.push），不再只活在本注释块里：
//                        只写注释 = 呈现层读不到，用户看到的就是「超短没有催化剂结论」而无理由。
// flow_persistence   : 未接且本脚本**不读** —— 不是本档腿，且 a-hot-money 的 verdict 还没这个字段
// kline_bars         : 已接 —— **日线** t-scoring 的 kline_json（要的量是 σ_daily，见下方 ⑥）
// overall_risk       : 已接 —— cls-risk-level.result.category（与主链同一四标签词表）
// stop_vol_mult      : 已接 —— 面板可调乘数，恒等映射；不接则脚本默认值会冒充面板值
// horizon_prior_json : 已接 —— Rust 侧按档算方向命中率后经验贝叶斯收缩；缺本档那行 ⇒ 拒绝出结论

// 与 `portfolio-mgr.rhai` 同规：`present` 是脚本自带的 1 行守卫（宿主引擎并未注册它，
// 靠宿主注册会让脚本依赖「注册面在不在」，本仓有过 pm_* 静默未注册的前例）。
fn present(x) { type_of(x) != "()" }

let h = "ultra_short";
let gaps = [];
gaps.push("超短分支：基本面/估值/研报/政策/解禁按构造不适用（兑现周期 > 2 日），缺腿是声明不是漏项");
// `absentLegs` 只说「这条腿没数」，分不清「市场今天没有涨停」与「成员判定根本没接线」⇒ 后者要单列，
// 否则缺席会被读成市场事实（本仓「结构性缺口不得在呈现层造成歧义」的同一条裁定）。
gaps.push("microstructure 腿本轮不可评估：涨停池成员判定（本代码 ∈ entries[]）尚未接进本分支 ⇒ 缺席是接线状态，不是盘面事实");
// 事件催化剂：契约层（`verdict_spec::UltraShort.participating`）没有 eventCatalyst 这条腿 ⇒
// 本档**显式作废**，与「今天没数据」区分开；主链 f3 仍在用 catalyst_level 是另一档口径的事，
// 见 #22（腿集与计权的一致性另有一门，未立）。
gaps.push("催化剂腿本档按构造不作废评估：权威分支表（verdict_spec.UltraShort）不含 eventCatalyst ⇒ 本档不出催化剂结论，不是漏数");

// ── ① 分支表与档位自检：表错了必须点名，不能拿错表的配比继续算 ──
let spec_ok = type_of(branch_json) == "map" && !branch_json.is_empty();
if !spec_ok {
    gaps.push("逐档分支表缺失 ⇒ 本档无腿可融合，拒绝输出结论（不退等权、不退共用参数）");
}
if spec_ok && branch_json["tier"] != h {
    gaps.push(`分支表档位是 ${branch_json["tier"]} 而本脚本是 ${h} ⇒ 子模板接线错配，结论不可用`);
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


// ── ② 逐腿取信号（信号口径的唯一实现 = pm_leg_signal，四档共用同一份）──
// ⚠ `raw` 里**只能出现本节点真映射过的名字**：`code_executor.rs:184` 只把 `input_mapping`
//   的 key 推进 scope（解析不到 = `Dynamic::UNIT`），脚本读一个没映射的名字不是「缺席」而是
//   **运行期 Variable not found ⇒ 整节点失败**（本仓 `Variable not found: horizon_prior_json`
//   的同型事故，见 `stock_workflow/rhai_registry.rs:826`）。测试里不会暴露这一条 ——
//   运行门按生产形态把 `present()` 的名字全铺 unit 再注入真值，**恰好把它糊过去了**。
//   所以这里刻意没有 `catalystLevel` / `flowPersistence` / `inLimitUpPool`：
//   催化剂不在本档腿集（PLAN §十一 与权威表的分歧，见下方 gaps）、资金持续性不是本档腿、
//   涨停池成员判定要的是「本代码 ∈ entries[]」= 数组归约，点号路径表达不了（下一片改成注入
//   池子串 + 脚本内 parse，届时这三个键才随各自的映射一起回来）。
let raw = #{
    totalScore: if present(tier_score) { tier_score } else { () },
    macdDif: if present(macd_dif) { macd_dif } else { () },
    macdDea: if present(macd_dea) { macd_dea } else { () },
    rsi: if present(rsi_value) { rsi_value } else { () },
    sealRate: if present(seal_rate) { seal_rate } else { () },
    breakCount: if present(pool_break_count) { pool_break_count } else { () },
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

// ── ③ 融合：只算 role=="direction" 且拿得到数的腿；配比按在场腿归一 ──
// 缺席腿**不参与归一化**（不参与 ≠ 信号为 0），否则等于把「没数据」算成「中性证据」
// 并摊薄在场腿 —— 主链「退出语义」章节记载的正是这个坑。
let tw = 0.0;
let ws = 0.0;
for row in present_legs {
    if row["role"] != "direction" { continue; }
    tw += row["weight"];
    ws += row["weight"] * row["signal"];
}
let avg = if tw > 0.0 { ws / tw } else { 0.0 };

// ── ④ 逐档先验（与其余三档同一注入源，但取的是本档那一行）──
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

// ── ⑤ 证据完整度：分子=在场方向腿配比合计，分母=本档全部方向腿配比合计 ──
let dir_total = 0.0;
for row in legs {
    if row["role"] == "direction" { dir_total += row["weight"]; }
}
let escale = if dir_total > 0.0 { pm_evidence_scale(tw, dir_total) } else { 0.0 };
let posterior = if prior_ok { clamp(prior_val + avg * escale, 0.0, 1.0) } else { 0.0 };

// ── ⑥ 置信：超短**不做跨期缩放**（退役 √(h/28) 的正面替代）──
// 出场口径 `time_stop` 与置信口径必须同族：既然 2 日到期就走，胜率就是
// 「本档证据的合成方向强度」本身，不随持有期开方调整。
let confidence = posterior;

// ── ⑦ 入场广度门（必要条件）：封板率 < 0.5 ⇒ 封顶「持有」 ──
// 0.5 是内禀锚（触板里一半封住 = 无方向信息），不是调参阈值。
let gate_pass = present(seal_rate) && seal_rate >= 0.5;
let gate_note = if !present(seal_rate) {
    "breadth_required 未判定：涨停池广度缺失（缺席 ≠ 通过，按不通过处理）"
} else if gate_pass {
    "breadth_required 通过：封板率 >= 0.5"
} else {
    `breadth_required 未过：封板率 ${seal_rate} < 0.5 ⇒ 本档不出买入/增持`
};
if !gate_pass {
    gaps.push(gate_note);
}

// ── ⑧ 价带与仓位：止损按 k·σ·√days（尾部保护），不设目标价止盈 ──
// 日波动与到期位移都走同一个宿主函数 `pm_vol_move_pct`（口径唯一，本仓共享引擎没有
// Math/sqrt ⇒ 开方只能在 Rust 侧）。`volLookbackDays` 来自分支表而不是脚本自带常量 ——
// 四个分支脚本各抄一份 60，就是下一个「多处一致地错」。
let bars_ok = present(kline_bars) && type_of(kline_bars) == "array";
let days = if spec_ok { branch_json["days"] } else { 2 };
// `band_days` / `band_lookback` 来自注入的分支表，已在上方经 `spec_int` 归一成 i64
// ⇒ 宿主函数（形参 i64）不会再因类型漏判而拿不到值（该缺陷曾让四档价带恒 0，见 `spec_int` 的口径注）。
let exit_move =
    if bars_ok && band_days > 0 && band_lookback > 0 {
    pm_vol_move_pct(kline_bars, band_lookback, band_days)
} else {
    0.0
};
let k_stop = if present(stop_vol_mult) { stop_vol_mult } else { 1.2 };
let stop_pct = exit_move * k_stop;
// 超短没有「止盈目标」这一档语义（出场口径 = `time_stop`）⇒ 参照取**到期位移本身**，
// 不再乘一个止盈倍数：2 日窗内「目标价」没有分布含义，乘倍数等于把止盈变成第二个调参入口。
let tp_pct = exit_move;
let odds = if stop_pct > 0.0 { tp_pct / stop_pct } else { 0.0 };
let kelly = if confidence >= 0.50 && odds > 0.001 {
    pm_kelly_position(confidence, odds, 0.0015, overall_risk)
} else {
    0.0
};
let position = kelly * (if spec_ok { branch_json["positionMultiplier"] } else { 0.6 });

// ── ⑨ 动作阶梯（本档自己的常量集，门 e：不得由他档常量 ± 偏移派生）──
let action = if !gate_pass && posterior >= 0.62 { "持有" }
    else if confidence >= 0.72 { "买入" }
    else if confidence >= 0.62 { "增持" }
    else if confidence >= 0.52 { "持有" }
    else if confidence >= 0.42 { "观望" }
    else if confidence >= 0.32 { "减持" }
    else { "卖出" };
if tw <= 0.0 || !prior_ok {
    action = "观望";
    position = 0.0;
    gaps.push("本档没有任何在场方向腿或先验不可得 ⇒ 动作退为观望（decision_degraded 哨兵），不产出仓位");
}

// ── ⑩ 输出：与主链 decisionsByHorizon 行同键（仲裁节点与面板按同一形状读）──
#{
    action: action,
    verdict: `决策=${action} 置信=${(confidence*1000.0).round()/10.0}% 档=${h} 出场=time_stop`,
    positionPct: (position*10.0).round()/10.0,
    confidence: (confidence*1000.0).round()/10.0,
    posterior: (posterior*100.0).round()/10.0,
    expectedHoldingDays: days,
    stopLossPct: (stop_pct*100.0).round()/100.0,
    takeProfitPct: (tp_pct*100.0).round()/100.0,
    odds: (odds*100.0).round()/100.0,
    evidenceScale: (escale*1000.0).round()/10.0,
    scoreSource: if present(tier_score) { "tier_native" } else { "daily_fallback" },
    // v138（裁定 3「让用户看出各档实际几根」）：本档**实际用的**指标窗口与所在尺度。
    // 产端是评分节点回显（`indicators.windows`），不是「想要的配置」⇒ 面板印出来的就是算过的那组。
    // 缺席语义：这两个键没有 = 该行产自早于 v138 的模板代 ⇒ 面板不渲染该行，不得补 0 或空表。
    scoringWindows: if present(scoring_windows) { scoring_windows } else { () },
    scoringScale: if present(scoring_scale) { scoring_scale } else { () },
    priorSource: prior_src,
    priorSamples: prior_samples,
    stopSource: if exit_move > 0.0 { "vol_band" } else { "fallback_pct" },
    positionSource: "kelly_x_position_multiplier",
    // 本档分支的自证字段（审计与门禁读这里，不再从文案反解）
    horizon: h,
    confidenceMethod: if spec_ok { branch_json["confidenceMethod"] } else { "missing_branch_table" },
    exitRule: if spec_ok { branch_json["exitRule"] } else { "missing_branch_table" },
    entryGate: if spec_ok { branch_json["entryGate"] } else { "missing_branch_table" },
    entryGatePassed: gate_pass,
    legs: present_legs,
    absentLegs: absent_legs,
    dataGaps: gaps,
}
