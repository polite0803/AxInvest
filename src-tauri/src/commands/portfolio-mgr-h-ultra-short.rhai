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
// 2. 方向腿 = 短窗动量 + 事件催化剂（v156 起接回，#22 另一半；催化剂是个股级证据，
//    2 日窗的事件驱动性正来自它）；广度是**市场级量**只作门（第 1 条），基本面与估值
//    **按构造不适用**（兑现周期 > 持有期，见 PLAN §十二）⇒ 不得为了让分母好看而补腿。
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
// scoring_scale      : v138 起接 —— 本档尺度名（hourly/daily/weekly/monthly，本档 = hourly；
//                      档→尺度的定阶规则 d ≤ h/3 见 `Period::scale_key`），与窗口并列成句；
//                      根数按公式由持有期换算，差别只在这里才可读
// scale_trend        : v139 起接 —— 本档两带趋势事实（快/慢带 bar 数与数值、diffPct、fastSlope）；
//                      产端是 `indicators.scaleTrend`，`diffPct`/`fastSlope` 可为 null ⇒ 面板印「—」不印 0
// scale_momentum     : v139 起接 —— 本档动量的「周期 + 值」对（与 `rsi_value` 同一个数，成对给出才自证口径）
// excess_return_pct  : 阶段 2 起接 —— `relative_strength.ultra_short`（本档窗口相对中证全指
//                      的累计超额，百分点）。**只作入场门的个股前提并列分支**，不是方向腿
//                      （未进 `verdict_spec` 的 participating，与 `breadthState` 只作门同源）
// seal_rate          : 已接 —— 涨停池 breadth.sealRate（breadthState 腿 + breadth_required 门的必要条件）
// pool_break_count   : 已接 —— 涨停池 **breadth**.breakCount = **全市场炸板家数**（市场级量）
//                      ⚠ **禁止**用于个股 `microstructure` 腿：同名不同义的第二个载体是
//                      `LimitUpPoolEntry.break_count`（**个股**炸板次数，见下一条）。
//                      本键现在只作披露（输出 `marketBreakCount`），`leg_signal` 不读它。
// pool_entries       : 已接（v194）—— 涨停池 **entries[]** 整份数组。脚本内按 `stockCode`
//                      定位本代码那一行，取**个股** `breakCount` 并判定 `inLimitUpPool`。
//                      为什么只能在脚本内做（映射层的表达力上限，写在这里供后续同类任务复用）：
//                      `input_mapping` 的取值器只做「按 `.` 拆段、逐段 `get`」
//                      （`subworkflow_executor.rs` 的 `resolve_var_path`，其全部能力 = 定值取值
//                      + 数组下标/字段下钻），**不能**表达「按某字段值过滤数组」这类归约；
//                      而 `json_value_to_dynamic` 会把整份数组原样转成 Rhai Array
//                      ⇒ 归约必须写在脚本里。凡「数组按字段挑一行」的接线都照此形态。
//                      （旧注 `pool_in_pool` 已随本条接线退役：那正是本条判定出来的布尔）
// catalyst_level     : 已接（v156，P1 催化剂腿 · #22 另一半）—— `verdict_spec.UltraShort`
//                      participating 已含 eventCatalyst；路径与主链 f3 的读法逐字一致
// flow_persistence   : 未接且本脚本**不读** —— 不是本档腿，且 a-hot-money 的 verdict 还没这个字段
// kline_bars         : 已接 —— **日线** t-scoring 的 kline_json（要的量是 σ_daily，见下方 ⑥）
// overall_risk       : 已接 —— cls-risk-level.result.category（与主链同一四标签词表）
// stop_vol_mult      : 已接 —— 面板可调乘数，恒等映射；不接则脚本默认值会冒充面板值
// horizon_prior_json : 已接 —— Rust 侧按档算方向命中率后经验贝叶斯收缩；缺本档那行 ⇒ 拒绝出结论
// stock_code         : 已接（P0-5）—— 供 `pm_recent_strength` 判板块涨跌幅（10/20/30cm），
//                      不接则宿主按未知板块（10cm）兜底，20cm 标的的涨停会漏判 ⇒ 必须显式接线

// 与 `portfolio-mgr.rhai` 同规：`present` 是脚本自带的 1 行守卫（宿主引擎并未注册它，
// 靠宿主注册会让脚本依赖「注册面在不在」，本仓有过 pm_* 静默未注册的前例）。
fn present(x) { type_of(x) != "()" }

// ── P0-5（`PLAN-stock-decision-reliability.md`）：breadth 门的个股前提 + 降杠杆封顶 ──
// 近 5 日「强势股」的累计涨幅门槛（百分数）：没有涨停但 5 日涨 20%（≈ 两个涨停的量级）
// 也算强势。内禀锚不是调参值——它要回答的是「这只票自身是否处在被资金追逐的状态」，
// 5%/10% 这种小涨不构成超短接力的个股证据。
// ⚠ 用无参函数而非顶层 `const`：本脚本由生产执行器 `engine.compile(code)` 无 scope 编译
//   （见 `rhai_ast_cache.rs`），rhai 只把编译 scope 里的常量传播进函数体，顶层 const
//   对同脚本函数不可见（2026-10-16 实测运行期 Variable not found）。
fn strong_cum_gain_pct() { 20.0 }

// 入场门纯判据（call_fn 测试直接钉死返回对，防阈值被悄悄改回去）：
// ① 封板率缺席 ⇒ 未判定（缺席 ≠ 通过）；② 封板率 < 0.5 ⇒ 市场情绪否决；
// ③ 个股强势特征缺席（K 线不足/宿主返回 unit）⇒ 个股前提未判定，同样不放行；
// ④ 近 5 日有涨停 ⇒ 个股前提通过；⑤ 无涨停但累计涨幅 ≥ 20% ⇒ 通过；
// ⑤b 无涨停、涨幅未达标，但**同期相对中证全指为正超额** ⇒ 以「抗跌蓄势」放行入门
//     （阶段 2，2026-10-19）。立项实证：002696 as-of 2026-09-28 判 `stock_not_strong`，
//     而那天大盘 −4.53%、它只 −2.41%（5 日窗超额 +2.25pt）——「还没启动」被当成「没有证据」。
// ⚠ 本分支**只放宽入场门这个必要条件**，不新增方向腿、不改融合权重 ⇒ 放行 ≠ 看多：
//    本档方向腿仍是 短窗动量 + 事件催化剂，且 microstructure 未接线期间
//    `pm_cap_unwired_action` 仍把买入/增持压回持有（见下方封顶）。
// ⑥ 否则 ⇒ 个股不强势（大盘情绪再好也不为这只票背书）。
// 返回 [pass, basis]：basis 同时进 dataGaps 与输出的 gateBasis，呈现层不得靠文案反解。
fn pm_ultra_entry_gate(seal_rate, lim_up_count, cum_gain_pct, excess_return_pct) {
    if !present(seal_rate) { return [false, "breadth_missing"]; }
    if seal_rate < 0.5 { return [false, "breadth_weak"]; }
    if !present(lim_up_count) { return [false, "stock_unjudged"]; }
    if lim_up_count >= 1 { return [true, "limit_up_recent"]; }
    if present(cum_gain_pct) && cum_gain_pct >= strong_cum_gain_pct() {
        return [true, "strong_momentum"];
    }
    if present(excess_return_pct) && excess_return_pct > 0.0 {
        return [true, "relative_strength_emerging"];
    }
    [false, "stock_not_strong"]
}

// 涨停池 entries[] 归约（v194）：从整份池子里挑出**本代码那一行**。
//
// 为什么必须有这个函数（映射层做不到）：`input_mapping` 只能按点号路径定值取值，
// 表达不了「按 stockCode 过滤数组」的归约 ⇒ 归约只能在脚本内做。
//
// 返回 map（**不用数组**）：三个字段类型不同（bool / 数值或 unit / 数值或 unit），
// 而 Rhai 数组元素类型不受约束、`arr[1]` 取 unit 位是合法但易被后续读者误当「有值」的形态；
// 用具名键能把「缺席」写成显式的 `hasBreakCount`，与 `absentLegs`/`dataGaps` 的既有纪律同族。
//   · inPool      ：本代码是否在 entries[] 里（**确定布尔**，接口给了池子就一定可判）
//   · breakCount  ：**个股**炸板次数；未进池或接口未给 ⇒ ()
//   · limitUpStreak：**个股**连板数；同上 ⇒ ()
// **缺席 ≠ 0**：「没进涨停池」与「进了但炸板数接口没给」是两个不同事实；
// 混成 0 会让 `leg_signal` 把「接口没说」当「零次炸板」⇒ 白送满强度 +1.0
// （`astock-data/src/types.rs` 对 `open_num` 约 2/3 行为 null 的实证就是这条的代价）。
//
// 代码比对方式：**trim 后逐字相等**，不做「去后缀/抽数字」的归一。
// 依据是本分支已有的同一条契约假设 —— `pm_recent_strength(kline_bars, stock_code, 5)`
// 正是拿 `stock_code` 判 10/20/30cm 板幅，它同样要求 `stock_code` 是纯 6 位；
// 池子侧 `LimitUpPoolEntry.stock_code` 由同花顺接口直接给纯 6 位。
// 两侧同契约 ⇒ 在这里再造一套归一逻辑，只会引入「归一规则与宿主模块判定不一致」的第二真相源。
fn pm_pool_row_for(entries, code) {
    let miss = #{ inPool: false, breakCount: (), limitUpStreak: () };
    if type_of(entries) != "array" { return miss; }
    if !present(code) || type_of(code) != "string" { return miss; }
    let want = code.trim();
    if want == "" { return miss; }
    for row in entries {
        if type_of(row) != "map" { continue; }
        let got = row["stockCode"];
        if !present(got) || type_of(got) != "string" { continue; }
        if got.trim() != want { continue; }
        return #{
            inPool: true,
            // 个股炸板次数（`LimitUpPoolEntry.break_count` ⇐ `open_num`，实测多为 null）
            // 与连板数（`limit_up_streak`，同样可为 null）分别取 —— 不互相兜底。
            breakCount: num_of(row["breakCount"]),
            limitUpStreak: num_of(row["limitUpStreak"]),
        };
    }
    miss
}

// 降杠杆封顶（P0-5）：microstructure 个股腿（本代码 ∈ 涨停池 entries[]，封单/砸单/
// 池内成员）未接线期间，超短档**最高只出「持有」**。门通过只说明「情绪 + 个股代理」
// 不否决，并不等于拿到了个股微观结构的开仓证据 ⇒ 买入/增持一律压回持有，
// 其余动作原样（观望/减持/卖出本就比持有更弱，不许被「封顶」反向抬升）。
//
// ⚠ v194 现状（**勿误读成已解除**）：个股腿的**数据**已接线（`pool_entries` ⇒
//   `inLimitUpPool` / 个股 `breakCount`），但「进涨停池 = 有开仓证据」这个**产品口径
//   尚未拍板**（见上方 P0-5 注释的原始理由）⇒ **封顶行为一字未改**。
//   本轮只把「腿到底在不在场」透出为 `microLegPresent` / `microBasis`，
//   让该产品口径的裁决有数据可依，且不改变任何现网动作。
fn pm_cap_unwired_action(action) {
    if action == "买入" || action == "增持" { "持有" } else { action }
}

let h = "ultra_short";
let gaps = [];
gaps.push("超短分支：基本面/估值/研报/政策/解禁按构造不适用（兑现周期 > 2 日），缺腿是声明不是漏项");
// v194：原「microstructure 腿尚未接进本分支」那条固定 gap 已退役 —— 个股腿的数据面
// （`pool_entries` ⇒ `inLimitUpPool` / 个股 `breakCount`）本版已接线。
// ⇒ gap 改为**按实况分通道**（见下方 raw 段之后的 `microBasis` 分支）：
//   「没进池」是盘面事实、「炸板数接口没给」是取数缺席、「映射没到」才是接线缺陷 ——
//   三者混成一句就回到「结构性缺口在呈现层造成歧义」的老形态（本仓已立法禁止）。
// v156：催化剂腿已接入（#22 另一半）⇒ 原先那条「按构造不作废评估」的 gap 退役。
// 缺席语义回到通用通道：催化剂为「无」/空/表外标签 ⇒ `leg_signal` 判缺席 ⇒ 出现在
// `absentLegs` 里（那是「今天没有可计权的催化剂」，与接线状态无关 —— 该腿的接线由
// `tier_fanout_inputs_are_complete` 与 `audit-inject-coverage.mjs` 两道门在 CI 上守）。

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
//   ⇒ 新增任何读名之前，先在 `horizon_tier_template.rs` 的 `input_mapping` 里加映射。
//   （v194：`pool_entries` 已按此加映射 ⇒ `inLimitUpPool` / 个股 `breakCount` 接线完成；
//     `flowPersistence` 仍刻意不接 —— 它不是本档腿，且 a-hot-money 的 verdict 还没该字段。）
//
// ── v194：个股微观结构腿的原料（**修「同名不同义」**）─────────────────────────
// `pool_break_count` 是 `LimitUpBreadth.breakCount` = **全市场炸板家数**（市场级量，
// `astock-data/src/types.rs` 的 `LimitUpBreadth`）。而 `leg_signal` 的 `microstructure`
// 要的是 **个股**炸板次数（`LimitUpPoolEntry.break_count` ⇐ `open_num`）。
// 同一个键名 `breakCount` 在两处指两个量纲 —— 此前因 `inLimitUpPool` 恒缺席，
// 该腿从未进融合，错配一直没暴露；一旦接线 inLimitUpPool 而仍喂市场级数，
// 就是拿「全市场炸了 N 家」当「这只票炸了 N 次」算个股信号。
// ⇒ 现在 raw.breakCount 一律取**个股级**（下方 `micro_breaks`）；
//    `pool_break_count` 不再进 `raw`，仅作披露输出 `marketBreakCount`（用途见输出段）。
let pool_row = pm_pool_row_for(pool_entries, stock_code);
// ⚠ `pm_pool_row_for` 返回的是 **map**（`#{ inPool / breakCount / limitUpStreak }`），
// 必须按**键名**取值。曾误写成数组下标 `pool_row[0]`/`[1]`/`[2]` ⇒ Rhai 把 `0` 当 map 键、
// 要求键为 string，运行时报 `Data type incorrect: i64 (expecting string)`。
// （实测由 `branch_node_scope_is_exactly_the_seed_mapping` 在 ultra_short 档抓住 —— 见 v194）
let in_limit_up_pool = pool_row.inPool == true;
let micro_breaks = if in_limit_up_pool { pool_row.breakCount } else { () };
let micro_streak = if in_limit_up_pool { pool_row.limitUpStreak } else { () };
// 腿「在不在场」的可观测信号（v194）。判据 = 池内成员判定拿到了确定的布尔 + 炸板数可读。
// ⚠ 这不是「有开仓证据」的判据 —— 那个产品口径未拍板（见 `pm_cap_unwired_action` 的注）。
let micro_leg_present = in_limit_up_pool && present(micro_breaks);
let micro_basis = if !present(stock_code) {
    "stock_code_missing"
} else if type_of(pool_entries) != "array" {
    "pool_entries_missing"
} else if !in_limit_up_pool {
    "not_in_pool"
} else if !present(micro_breaks) {
    "break_count_absent"
} else {
    "in_pool"
};

let raw = #{
    totalScore: if present(tier_score) { tier_score } else { () },
    macdDif: if present(macd_dif) { macd_dif } else { () },
    macdDea: if present(macd_dea) { macd_dea } else { () },
    rsi: if present(rsi_value) { rsi_value } else { () },
    sealRate: if present(seal_rate) { seal_rate } else { () },
    // v194：**个股**炸板次数（不是 `pool_break_count` 的市场级炸板家数，见上）。
    // 未进池 ⇒ 缺席（`leg_signal` 会先看 inLimitUpPool 再读本键，两处同拒）。
    breakCount: micro_breaks,
    // v194：池内成员判定 —— `leg_signal` 的 microstructure 分支以它为前置必要条件。
    inLimitUpPool: in_limit_up_pool,
    // v156（#22 另一半）：催化剂腿 —— `leg_signal` 的 eventCatalyst 走受控六级标签词表
    // （L-3~L3），「无」/空/表外标签 ⇒ 缺席（不按 0 参与）。
    catalystLevel: if present(catalyst_level) { catalyst_level } else { () },
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

// v194：微观结构腿的缺席按 `microBasis` **分通道留痕**（不写一句「不可评估」了事）。
// 四态语义刻意不同，呈现层与审计据此分辨「盘面事实 / 取数缺席 / 接线缺陷」：
//   · in_pool              ⇒ 腿在场，不进 gap（在场就不是缺口）
//   · not_in_pool          ⇒ **盘面事实**：本代码今天不在涨停池里（这不是缺陷）
//   · break_count_absent  ⇒ **取数缺席**：进了池但接口没给 `open_num`（实测常见）
//   · stock_code_missing / pool_entries_missing ⇒ **接线缺陷**，必须显式告警
if micro_basis == "not_in_pool" {
    gaps.push("microstructure 腿本轮缺席：本代码不在涨停池 entries[]（盘面事实，不是接线缺陷）");
} else if micro_basis == "break_count_absent" {
    gaps.push("microstructure 腿本轮缺席：已进涨停池但接口未给个股炸板次数（取数缺席，非零次炸板）");
} else if micro_basis == "stock_code_missing" || micro_basis == "pool_entries_missing" {
    gaps.push(`microstructure 腿本轮不可评估：${micro_basis} ⇒ 这是接线/注入缺陷，不是盘面事实`);
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

// ── ⑦ 入场门（必要条件，P0-5 起双前提）：封板率 ≥ 0.5 **且** 个股近 5 日强势 ──
// 0.5 是内禀锚（触板里一半封住 = 无方向信息），不是调参阈值。
// P0-5 前本门只验封板率 —— 那是全市场共享的一个数，大盘情绪火热时任何票都能过门
// （紫金 2026-10-09：84% 封板率 + 个股毫无涨停特征，照样出买入）。个股前提走
// `pm_recent_strength`（近 5 日涨停次数 / 累计涨幅），微观结构腿（池内成员）接线后
// 再升级成第三前提。
let stock_strength = if present(kline_bars) && type_of(kline_bars) == "array" {
    pm_recent_strength(kline_bars, stock_code, 5)
} else {
    ()
};
let lim_up_count = if type_of(stock_strength) == "map" { stock_strength["limitUpCount"] } else { () };
let cum_gain_pct = if type_of(stock_strength) == "map" { stock_strength["cumGainPct"] } else { () };
// 阶段 2 第 4 参：本档窗口的相对强度（wiring 层按 `Period::ALL` 逐档算好注入，
// 口径单点在 `harness::indicators::excess_return_pct`；缺席 = null ⇒ 本分支不参与，
// 不折成 0 —— 0 会被读成「与基准同步」，那是另一个事实）
let gate_pair = pm_ultra_entry_gate(seal_rate, lim_up_count, cum_gain_pct, excess_return_pct);
let gate_pass = gate_pair[0];
let gate_basis = gate_pair[1];
// ── 情绪门开关（2026-10-26 用户裁决）────────────────────────────────
// 门只**单向否决**（拦买入、不抬信号）；放开 ⇒ 本档不再因市场情绪而不出买入/增持。
// ⚠ 放开后必须**留痕**（push 一条 gap），否则「未过情绪门」与「情绪门已放开」在
//   界面上完全同形 ⇒ 事后无法区分「市场情绪不支持」与「用户选择放开该门」。
let sentiment_gate_open = pm_gate_enabled("gate.sentiment_enabled");
let gate_note = if gate_basis == "breadth_missing" {
    "breadth_required 未判定：涨停池广度缺失（缺席 ≠ 通过，按不通过处理）"
} else if gate_basis == "breadth_weak" {
    `breadth_required 未过：封板率 ${seal_rate} < 0.5 ⇒ 市场情绪否决，本档不出买入/增持`
} else if gate_basis == "stock_unjudged" {
    `breadth_required 未判定：封板率达标（${seal_rate}）但个股近 5 日强势特征算不出（K 线不足）⇒ 缺席 ≠ 通过`
} else if gate_basis == "stock_not_strong" {
    `breadth_required 未过：封板率 ${seal_rate} 达标，但个股近 5 日无涨停且涨幅 < ${strong_cum_gain_pct()}% ⇒ 市场级情绪不为非强势个股背书`
} else {
    `breadth_required 通过：封板率 ${seal_rate} >= 0.5 且个股前提=${gate_basis}`
};
if !gate_pass {
    if sentiment_gate_open {
        // 门已放开 ⇒ 记「放开 + 原本会被拦」而不是记「未通过」，
        // 两者含义不同：前者是用户主动选择，后者是市场事实。
        gaps.push(`市场情绪门已放开（breadth_required 原判据：${gate_basis}）⇒ 本档不因市场情绪被否决`);
    } else {
        gaps.push(gate_note);
    }
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
// v156（终审发现 ①）：档位乘数后**再过一次风险封顶** —— `pm_kelly_position` 内部已按风险档
// 封顶，若乘数（长档 1.2）后置生效而不复封，"极高风险上限 10%"会被放大成 12%。
// `pm_risk_cap` 与 kelly 内部用同一个上限表（`portfolio_formula::apply_risk_cap`）。
let position = pm_risk_cap(kelly * (if spec_ok { branch_json["positionMultiplier"] } else { 0.6 }), overall_risk);

// ── ⑨ 动作阶梯（本档自己的常量集，门 e：不得由他档常量 ± 偏移派生）──
// ⚠ 情绪门放开 ⇒ `gate_pass` 不再参与动作阶梯的否决（门已放开时它只是「原判据」留痕）。
//   注意：**P0-5 降杠杆封顶（下一段）是独立的第四道限制**，不受情绪门开关影响 ——
//   微观结构腿未接线前，超短档开仓动作仍会被压回「持有」。
let action = if !gate_pass && !sentiment_gate_open && posterior >= 0.62 { "持有" }
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

// ── ⑨″ P0-5 降杠杆封顶：microstructure 个股腿未接线期间，最高只出「持有」 ──
// 封板率是市场级量、近 5 日强势只是粗代理；真正的个股微观结构证据（本代码 ∈ 涨停池
// entries[]、封单/砸单强度）接线前，本档不给开仓动作。封顶点必须在动作阶梯之后、
// 档内风控否决之前：退化哨兵（观望）比持有更弱，保持不动；风控否决只会更严，随后再压。
let pre_cap_action = action;
action = pm_cap_unwired_action(action);
let p05_cap_applied = action != pre_cap_action;
if p05_cap_applied {
    gaps.push(`P0-5 降杠杆：microstructure 个股腿数据已接线（v194，basis=${micro_basis}）但「进池=开仓证据」产品口径未拍板 ⇒ 开仓动作仍封顶 ${pre_cap_action}→持有（breadth 门依据=${gate_basis}）；该口径裁决后放开`);
}

// ── ⑨′ 档内风控否决（2026-10-10 B 裁定）──
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

// ── ⑩ 输出：与主链 decisionsByHorizon 行同键（仲裁节点与面板按同一形状读）──
#{
    action: action,
    // 档内否决前的方向值：读侧「何以降级」与冲突判定读它，不得从 verdict 文案反解。
    originalAction: original_action,
    tierVetoed: tier_vetoed,
    verdict: `决策=${action} 置信=${(confidence*1000.0).round()/10.0}% 档=${h} 出场=time_stop${tier_veto_note}`,
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
    // 本档分支的自证字段（审计与门禁读这里，不再从文案反解）
    horizon: h,
    confidenceMethod: if spec_ok { branch_json["confidenceMethod"] } else { "missing_branch_table" },
    exitRule: if spec_ok { branch_json["exitRule"] } else { "missing_branch_table" },
    entryGate: if spec_ok { branch_json["entryGate"] } else { "missing_branch_table" },
    // ⚠ 门放开后 `entryGatePassed` 必须报 true（门已放行 ⇒ 入场门**已通过**），
    //   否则呈现层会显示「入场门未过」而实际是「用户选择放开该门」⇒ 同屏误导。
    //   「原判据是什么」由 `gateBasis` + dataGaps 留痕承担，不靠这个布尔反解。
    entryGatePassed: gate_pass || sentiment_gate_open,
    // P0-5：门判据 basis 与个股强势原始量（审计/面板按字段读，不从 dataGaps 文案反解）
    gateBasis: gate_basis,
    // 情绪门是否已被产品决策放开（2026-10-26）—— 呈现层据此显示「门已放开」而非「门未过」
    sentimentGateOpen: sentiment_gate_open,
    stockStrength: stock_strength,
    // P0-5 降杠杆封顶是否触发（true = 原动作是买入/增持、被压成持有）
    p05CapApplied: p05_cap_applied,
    // ── v194：微观结构腿的**在场状态**（产品口径裁决的数据面）───────────────
    // ⚠ 这三个键**不改变任何动作**：封顶仍无条件生效（`pm_cap_unwired_action` 一字未改）。
    //   它们的作用是让「进涨停池算不算开仓证据」这个未拍板的产品口径**有数据可依** ——
    //   在此之前，封顶触发与否只表现为动作被压回，看不出是「腿不在场」还是「腿在场但仍封顶」。
    //   呈现层若把 `microLegPresent==true && p05CapApplied==true` 同屏显示，
    //   就是在提示「腿已有数据、缺的是产品裁决」。
    microLegPresent: micro_leg_present,
    // 四态：in_pool / not_in_pool / break_count_absent / stock_code_missing / pool_entries_missing
    // （判据见 raw 段上方的 `micro_basis`；呈现层按此分通道措辞，不解析 dataGaps 中文）。
    microBasis: micro_basis,
    // 个股连板数（`limit_up_streak`）：仅在池内且接口给值时存在，缺席不补 0（`types.rs` 同纪律）。
    microLimitUpStreak: micro_streak,
    // `pool_break_count` 的**真实语义**披露：它是 `LimitUpBreadth.breakCount` = **全市场炸板家数**
    // （市场级量），与个股 `microstructure` 腿无关。改名属独立改造（牵动映射键/夹具/变量表），
    // 本轮按保守方案保留原名，只在此显式披露 —— 防下一个读者再把它当个股炸板次数用。
    marketBreakCount: if present(pool_break_count) { pool_break_count } else { () },
    // P2a（2026-10-18）：封顶**前置动作**的结构化自证 —— 主链摘要「动作=封顶结果(买入被
    //   压回…)」读这里，不从 dataGaps 文案反解。未封顶时恒等于 action（消费方以
    //   p05CapApplied==true 为准，本键只供展示封顶前是什么）。
    actionBeforeCap: pre_cap_action,
    legs: present_legs,
    absentLegs: absent_legs,
    dataGaps: gaps,
}
