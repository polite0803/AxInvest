// portfolio-mgr.rhai — 贝叶斯因子融合 + 凯利仓位
//
// 理论依据：
//   贝叶斯更新: P(上涨|证据) ∝ P(证据|上涨) × P(上涨)
//   凯利公式:   f* = (p × odds - q) / odds
//     p = P(上涨|证据)   q = 1-p
//     odds = 预期收益% / 预期亏损%
//
// 数据驱动（P1/P2）：
//   - 先验 P(上涨) = t-scoring 回测的市场状态（牛市0.55/熊市0.45/中性0.50）
//   - 因子权重 = t-scoring 回测的信息系数(IC)归一化
//
// 缺失因子不参与融合（不编造数据）。
//
// P2-B4: 统一新闻情感分词典已在 `axagent_astock_data::sentiment` 落地,
// 并通过 `pm_compute_news_sentiment(title, summary)` / `pm_compute_text_sentiment(text)`
// 注册到共享 Rhai Engine。下方公告关键词检测逻辑(risk_kw/pos_kw)保留独立实现,
// 因为它输出 [-0.3, 0.3] 修正信号(语义不同于 sentiment_score [-1.0, 1.0])。
// 未来重构可考虑调用 pm_compute_news_sentiment 替代本地词典。

// ── Rhai 全局函数（必须位于全局作用域，Rhai 不允许在 try/块内定义函数）──
fn present(x) { type_of(x) != "()" }
fn safe_parse(raw) {
    if !present(raw) { return (); }
    if type_of(raw) != "string" || raw == "" { return (); }
    try {
        json_parse(raw)
    } catch (err) {
        ()
    }
}
fn get_weight(weights, factor, fallback) {
    if type_of(weights) == "map" {
        let f = weights[factor];
        if type_of(f) != "()" && type_of(f) == "map" {
            let w = f["weight"];
            if type_of(w) == "f64" { w } else { fallback }
        } else { fallback }
    } else { fallback }
}
fn risk_rank(label) {
    switch label {
        "低风险"|"低" => 0,
        "中风险"|"中" => 1,
        "高风险"|"高" => 2,
        "极高风险"|"极高" => 3,
        _ => 1
    }
}
// has_negation: 检查关键词前 N 字符是否含否定词（避免"没有退市风险"被误判为风险信号）
// 必须位于全局作用域（Rhai 不允许在块内定义函数）
fn has_negation(text, kw, negs) {
    let idx = text.index_of(kw);
    if idx == -1 { return false; }
    // 取关键词前 20 个字符（覆盖常见的否定短语长度）
    let before = if idx >= 20 { text.sub_string(idx - 20, 20) } else { text.sub_string(0, idx) };
    for n in negs {
        if before.contains(n) { return true; }
    }
    false
}
// num_of: 统一数值读取 —— JSON 整数解析为 i64、小数解析为 f64，
// 旧代码只判 type_of(x)=="f64" 会漏读 i64 字段（静默归零）。
// 缺失或非数值返回 ()（unit），由调用点决定兜底语义。
// 必须位于全局作用域（Rhai 不允许在块内定义函数）
fn num_of(x) {
    if type_of(x) == "f64" { x }
    else if type_of(x) == "i64" || type_of(x) == "int" { x.to_float() }
    else { () }
}
// pm_avg_turnover: 从 kline_json（日线数组）估算近 N 日平均成交额（元）。
// 上游 amount 字段常为 0.0（未填充），故以 volume × close 兜底估算。
// 无有效样本时返回 ()，调用点需自行降级。
fn pm_avg_turnover(kline_raw, days) {
    if !present(kline_raw) { return (); }
    // 兼容两种上游形态：已解析的数组 / 双重编码的 JSON 字符串
    let arr = if type_of(kline_raw) == "array" {
        kline_raw
    } else if type_of(kline_raw) == "string" && kline_raw != "" {
        safe_parse(kline_raw)
    } else { () };
    if type_of(arr) != "array" || arr.len() == 0 { return (); }
    let n = arr.len();
    let take = if n < days { n } else { days };
    let sum = 0.0;
    let cnt = 0;
    for i in (n - take)..n {
        let bar = arr[i];
        if type_of(bar) != "map" { continue; }
        let amt = num_of(bar["amount"]);
        let effective = if type_of(amt) == "f64" && amt > 0.0 {
            amt
        } else {
            let vol = num_of(bar["volume"]);
            let close = num_of(bar["close"]);
            if type_of(vol) == "f64" && type_of(close) == "f64" { vol * close } else { 0.0 }
        };
        if effective > 0.0 {
            sum = sum + effective;
            cnt = cnt + 1;
        }
    }
    if cnt == 0 { () } else { sum / (cnt.to_float()) }
}
// pm_saturate: 有理饱和映射 r/(|r|+k) ∈ (-1, 1)，r=±k → ±0.5。
// 相比线性 clamp，避免「超过阈值即满值」造成的强度信息丢失。
fn pm_saturate(r, k) {
    if k <= 0.0 { return 0.0; }
    let denom = r.abs() + k;
    if denom <= 0.0 { 0.0 } else { clamp(r / denom, -1.0, 1.0) }
}

// ── 防御性兜底：任何 Rhai 运行时错误都返回可解析的保守决策，避免节点无输出 ──
let pm_result = ();

try {
// ── 可配置参数（修复 D7/D8：通过 input_mapping 覆盖，默认值为 V56 校准 A 股参数）──
// action 阈值（effective_posterior）
let ACTION_BUY_THRESHOLD = if present(action_buy_threshold) { action_buy_threshold } else { 0.63 };
let ACTION_INCREASE_THRESHOLD = if present(action_increase_threshold) { action_increase_threshold } else { 0.53 };
let ACTION_HOLD_THRESHOLD = if present(action_hold_threshold) { action_hold_threshold } else { 0.48 };
let ACTION_WATCH_THRESHOLD = if present(action_watch_threshold) { action_watch_threshold } else { 0.38 };
let ACTION_REDUCE_THRESHOLD = if present(action_reduce_threshold) { action_reduce_threshold } else { 0.30 };
// 仓位阈值（%）
let POS_BUY_MIN = if present(pos_buy_min) { pos_buy_min } else { 15.0 };
let POS_INCREASE_MIN = if present(pos_increase_min) { pos_increase_min } else { 10.0 };
// 风险分类阈值
let RISK_DEBT_EXTREME = if present(risk_debt_extreme) { risk_debt_extreme } else { 85.0 };
let RISK_VOL_EXTREME = if present(risk_vol_extreme) { risk_vol_extreme } else { 60.0 };
let RISK_SHARPE_EXTREME = if present(risk_sharpe_extreme) { risk_sharpe_extreme } else { -1.5 };
let RISK_VOL_HIGH = if present(risk_vol_high) { risk_vol_high } else { 40.0 };
let RISK_DD_HIGH = if present(risk_dd_high) { risk_dd_high } else { 45.0 };
let RISK_ROE_HIGH = if present(risk_roe_high) { risk_roe_high } else { 5.0 };
let RISK_DEBT_HIGH = if present(risk_debt_high) { risk_debt_high } else { 65.0 };
let RISK_VOL_LOW = if present(risk_vol_low) { risk_vol_low } else { 25.0 };
let RISK_SHARPE_LOW = if present(risk_sharpe_low) { risk_sharpe_low } else { 0.5 };
let RISK_DD_LOW = if present(risk_dd_low) { risk_dd_low } else { 30.0 };
let RISK_ROE_LOW = if present(risk_roe_low) { risk_roe_low } else { 8.0 };
let RISK_DEBT_LOW = if present(risk_debt_low) { risk_debt_low } else { 55.0 };
let RISK_GROWTH_LOW = if present(risk_growth_low) { risk_growth_low } else { 3.0 };
// 仓位限制（%）
let POS_CAP_EXTREME = if present(pos_cap_extreme) { pos_cap_extreme } else { 10.0 };
let POS_CAP_HIGH = if present(pos_cap_high) { pos_cap_high } else { 35.0 };
let POS_CAP_MID = if present(pos_cap_mid) { pos_cap_mid } else { 50.0 };
// 交易成本率（A 股默认 0.3%）
let COST_PCT = if present(cost_pct) { cost_pct } else { 0.003 };

// 安全解析 JSON：解析失败（非法 JSON / 非字符串 / 空）返回 ()（unit），
// 避免上游脏数据（如 LLM 返回带 markdown 包裹的 JSON）导致整个工作流崩溃。
// 调用点已有 type_of 校验，() 会自然跳过，等价于"无数据"。
// ── V55: 累加上游 strict_mode 兜底哨兵 ──
// 任意上游 AgentNode 走 strict_mode 降级（LLM 输出无法解析为合法 JSON）时，
// 会通过 NodeOutput 顶层注入 __untrusted=true。本脚本读 input_mapping
// 把这些哨兵折算为 untrusted_count（0-6），用于触发 weights_collapsed 兜底。
// 这样 LLM 失败的"50/50 中性兜底"会被显式识别为不可信信号。
let untrusted_count = 0;
if present(untrusted_trader) && untrusted_trader == true { untrusted_count += 1; }
if present(untrusted_research_mgr) && untrusted_research_mgr == true { untrusted_count += 1; }
if present(untrusted_catalyst) && untrusted_catalyst == true { untrusted_count += 1; }
if present(untrusted_debate_conv) && untrusted_debate_conv == true { untrusted_count += 1; }
if present(untrusted_data_quality) && untrusted_data_quality == true { untrusted_count += 1; }
if present(untrusted_risk_conv) && untrusted_risk_conv == true { untrusted_count += 1; }

// ── 先验：由市场状态方向派生（P1-A 修复，2026-09-11）──
// 修复背景（DB 实证，12 条新模板运行）：
//   旧实现 `prior = market_regime_prior`，而该变量映射自
//   `market_regime.confidence` —— classify_regime 的「分类置信度」。
//   致命点：bull 与 bear 分支的 confidence 公式**完全相同**
//     `(|price_above_ma60| × 2.0).clamp(0.5, 0.95)`
//   即它只反映「价格偏离 MA60 的幅度」，**完全不含方向信息**；且偏离落在
//   3%~25% 区间时 ×2 的结果 < 0.5 会被夹到下界，于是常态下恒等于 0.5。
//   实测：bear 市况（沪深300 空头排列）下 prior 恒 = 0.5，
//   相当于把市场状态信息整体屏蔽，posterior 只剩个股因子在动。
//   更荒谬的是：牛市/熊市偏离幅度越大 confidence 越高 → 先验越看多。
// 正确语义：prior 是「无个股证据时对上涨的基础概率判断」，必须由 regime **方向**派生。
// 可配置：三个先验值可经 input_mapping 注入覆盖（settings 可调），默认如下。
//   依据：A 股趋势市的一年期胜率经验值 bull 55% / sideways 50% / bear 45%；
//   偏离幅度控制在 ±5pt 以内，避免先验本身主导 posterior。
let REGIME_PRIOR_BULL = if present(regime_prior_bull) { regime_prior_bull } else { 0.55 };
let REGIME_PRIOR_SIDEWAYS = if present(regime_prior_sideways) { regime_prior_sideways } else { 0.50 };
let REGIME_PRIOR_BEAR = if present(regime_prior_bear) { regime_prior_bear } else { 0.45 };
let prior = if present(market_regime_state) && type_of(market_regime_state) == "string" {
    switch market_regime_state {
        "bull" => REGIME_PRIOR_BULL,
        "bear" => REGIME_PRIOR_BEAR,
        "sideways"|"neutral"|"unknown" => REGIME_PRIOR_SIDEWAYS,
        _ => REGIME_PRIOR_SIDEWAYS
    }
} else if present(market_regime_prior) && type_of(market_regime_prior) == "f64" {
    // 向后兼容：旧输入（可能仍是 confidence）夹到方向先验的合理区间，
    // 避免把 0.5~0.95 的分类置信度当成「95% 看多先验」。
    clamp(market_regime_prior, 0.35, 0.65)
} else { REGIME_PRIOR_SIDEWAYS };

// ── 因子权重：regime-weights（市场状态调节）优先，IC 归一化权重 fallback ──
// P1-1 修复: regime_factor_weights 来自 regime-weights.rhai（熊/牛/高波动自适应），
//             fallback 到 t-scoring 的 factor_backtest.factors（纯回测 IC 归一化）。
// regime_factor_weights 结构: {trend:{weight,regime_mod}, consensus:{...}, catalyst:{...}, ...}
// factor_weights 结构: {trend:{weight,win_rate,ic}, macd:{...}, ...}
// 优先用 regime_factor_weights（含市场状态感知），缺失时回退到回测权重。
let f_weights = if present(regime_factor_weights) && type_of(regime_factor_weights) == "map" {
    regime_factor_weights
} else if present(factor_weights) && type_of(factor_weights) == "map" {
    factor_weights
} else { #{}};  // 空 map（t-scoring 的 factor_backtest 是占位空对象，见 astock-data/src/mcp_tools.rs:1254）⇒ get_weight 对每个因子回落各自的 default；若 map 非空但缺该键，同样回落 default（get_weight 无「等权」逻辑）

// ── P1-F 权重政策契约（2026-09-11；v61 2026-09-20 增列 f13）──
// 背景：regime-weights.rhai 原先输出 9 个因子的市况调节权重，但本脚本实际只对
//   5 个因子调用 get_weight（trend / consensus / valuation / money_flow / chip，
//   另 pace / momentum 走 IC 回测权重）。其余四个因子 —— catalyst / risk /
//   data_quality / trade_signal —— 的调节权重合计 0.685（占调整后总权重 1.41 的
//   48.6%）产出后无人消费，是「死计算」：既不生效，又让人误以为权重已随市况自适应。
// 处理：regime-weights.rhai 已改为分桶输出（factor_weights 仅含被消费因子，
//   unconsumed_suggestions 保留未消费建议供审计）。本侧权重政策明确如下：
//
//   ┌ 因子 ──────────┬ 权重来源 ────────────────────────────┬ 说明 ─────────────────┐
//   │ f1  trend      │ regime_factor_weights.trend          │ 市况自适应（牛升熊降）│
//   │ f2  consensus  │ regime_factor_weights.consensus      │ 市况自适应            │
//   │ f3  catalyst   │ 硬编码 0.20                          │ 刻意不消费 regime 建议 │
//   │ f4  risk       │ 硬编码 0.15                          │ 刻意不消费 regime 建议 │
//   │ f5  valuation  │ regime_factor_weights.valuation      │ 市况自适应（熊市升权）│
//   │ f6  data_quality│ 硬编码 0.15                         │ 刻意不消费 regime 建议 │
//   │ f7  trade_signal│ 硬编码 0.10                         │ V65 降权为定性参考     │
//   │ f9  money_flow │ regime_factor_weights.money_flow     │ P1-D 重估 base 0.12    │
//   │ f10 chip       │ regime_factor_weights.chip           │ 市况自适应            │
//   │ f11 pace       │ 回测 IC 权重（fallback 0.08）        │ regime 未提供          │
//   │ f12 momentum   │ 回测 IC 权重（fallback 0.10）        │ regime 未提供          │
//   │ f13 bottleneck │ regime_factor_weights.bottleneck     │ v61 改为市况自适应     │
//   └────────────────┴──────────────────────────────────────┴───────────────────────┘
//   ⚠️ f13 于 v61 从「硬编码 0.10（刻意不消费）」改为「走 get_weight」——原理由
//      （「瓶颈是行业结构属性故市况不敏感」）混淆了「信号源取值不随市况变」与
//      「该因子的判别价值不随市况变」，详见 f13 段落的变更说明。
//
//   ⚠️ f12 的 fallback 值订正（2026-09-20）—— 本行原写「fallback 0.08」，是**声明错误**：
//      2026-09-11 建本表时按 f11(pace) 那一行**成批抄写**（两行措辞原本逐字相同），
//      未把 f12 的真实值改写。判定为错误的依据（三处独立证据 + 一条机制论证）：
//        · 代码 `:1037` `let f12_default = 0.10;`
//        · `max_weight`(`:1357`) 第 11 项亦为 `0.10`（**硬编码字面量，不引用变量**），
//          与 `:1339` 注释「v19: + f12=0.10 → 1.49」自洽（复算 11 项和 = 1.49 ✓）
//        · 机制：`momentum` 不在 regime-weights 的 `factor_names` 白名单内，
//          且 `factor_weights` 的来源 `factor_backtest.factors` 是**占位空 map**
//          （`astock-data/src/mcp_tools.rs:1254`「因子回测引擎未实现」）
//          ⇒ `get_weight` 在**两条路径下都**回落 `f12_default`
//          ⇒ **0.10 是 f12 权重的唯一实际取值来源**（历史全部运行皆用 0.10）。
//      ⇒ 本次仅订正**声明**（零运行时影响）。⚠ 切勿据此把 `:1037` 改成 0.08：
//        那会动参数标定，且 `max_weight` 是手抄字面量（不引用变量）⇒ 只改一处会让
//        分子/分母失配（比例 0.9866，系统性偏低 1.34%），正是 `:1344-1350` 警告的
//        `weights_collapsed` 边界误触发形态（同 V66）。
//      ⇒ 随本项删除门禁 `rhai_syntax_check.rs` 的 `SEMANTICS_EXEMPT` 豁免（裁决后须删）。
//
// 为何 f4(risk) **刻意**不消费 regime 建议的 0.225：
//   反事实实证（2026-09-11，9 样本复现模型）：把 f4 权重从 0.15 换成本脚本建议的
//   bear 市况 0.225 后，8/9 样本 posterior 更低（均值 -1.0pt），且与 P1-B 的成长
//   豁免叠加会造成「成长股既被豁免风险惩罚、又被放大风险权重」的双重矛盾。
//   风险维度的市况自适应已通过 risk_bias（极高 -0.15 / 高 -0.08）与 risk_cap
//   （极高 10% / 高 35%）在操作层实现，方向层保持固定权重避免超额压制。
// f3/f6 同理：它们的市况敏感度已由上游节点（催化剂等级、数据质量分）自身的
//   取值变化体现，无需再叠加一层权重调节。

// 尝试从 factor_weights 读取权重；找不到则用等权 fallback

// ── 因子 1: 技术面 ──
let f1_default = 0.15;  // V49 修复: 从 0.25 下调 — 技术面(趋势跟踪)必然滞后,
                          // 股价涨完才触发"多头排列", 不应占最高权重
let f1_weight = if present(totalScore) { get_weight(f_weights, "trend", f1_default) } else { 0.0 };
let f1_signal = if present(totalScore) { (totalScore - 50.0) / 50.0 } else { 0.0 };

// ── 因子 2: 共识 ──
let f2_default = 0.25;
// V29 修复: 共识因子应使用独立的回测权重键 "consensus"，而非复用 "trend"
// factor_weights 结构为 {trend:{...}, macd:{...}, rsi:{...}, consensus:{...}, ...}
// 若回测数据无 "consensus" 键，get_weight 会 fallback 到 f2_default
let f2_weight = if present(consensusScore) { get_weight(f_weights, "consensus", f2_default) } else { 0.0 };
let f2_signal = if present(consensusScore) { (consensusScore - 50.0) / 50.0 } else { 0.0 };

// ── 公告关键词风险检测（P2 新增，f3 催化剂修正）──
// 数据源: t-catalyst-data → get_stock_announcements
// 检测公告标题中的风险/利好关键词，输出 [-0.3, 0.3] 修正信号
// 与 LLM catalyst_level（f3）正交，提供额外的公告级预警
// 注: 若 ann_type 含"风险提示"、title 含"可能下跌"等为高频预警信号
let announcement_modifier = if present(announcements) && type_of(announcements) == "string" && announcements != "" {
    let ann_list = safe_parse(announcements);
    if type_of(ann_list) == "array" && ann_list.len() > 0 {
        let risk_count = 0;
        let positive_count = 0;
        let high_risk_count = 0;
        // 风险关键词（匹配公告标题/类型）
        // 修复 D9: 补充更多政策/监管/诉讼/ST 相关关键词
        let risk_kw = ["风险提示", "异常波动", "可能下跌", "可能快速下跌", "风险警示",
                       "退市", "立案调查", "监管函", "警示函", "减持", "业绩预亏",
                       "亏损", "ST", "*ST",
                       "股东减持", "被动减持", "强制平仓", "违约",
                       "暂停上市", "终止上市", "面值退市", "重大违法",
                       "责令改正", "通报批评", "公开谴责", "行政处罚",
                       "诉讼", "仲裁", "冻结", "查封",
                       "商誉减值", "资产减值", "信用减值", "坏账",
                       "st", "*st", "SST", "S*ST"];
        // 利好关键词
        let pos_kw = ["业绩预增", "扭亏为盈", "中标", "重大合同", "增持",
                      "回购", "股权激励", "分红", "送转",
                      "业绩预告向上", "业绩超预期"];
        for i in 0..ann_list.len() {
            let item = ann_list[i];
            if type_of(item) == "map" {
                let title = if type_of(item["title"]) == "string" { item["title"] } else { "" };
                let ann_type = if type_of(item["ann_type"]) == "string" { item["ann_type"] } else { "" };
                let combined = `${title} ${ann_type}`;
                // ── P2-2: 否定词前缀检测 ──
                // 关键词匹配无法处理否定语义，如"没有退市风险"会被误判为风险信号。
                // A 股公告常见否定句式: "不存在退市风险" "未发现违规行为" "已消除影响"
                // 在匹配关键词前，检查关键词前 N 个字符是否有否定词。
                // 否定词词典（覆盖中文公告中最常见的否定表达）:
                let neg_prefixes = ["没有", "未发现", "不存在", "不存在", "已消除",
                                    "已解决", "已排除", "并非", "非", "尚未",
                                    "未触及", "未达到", "未发生"];
                // has_negation 函数已移至全局作用域（Rhai 不允许在块内定义函数）
                // 高风险关键词检测（含否定词过滤）
                for kw in risk_kw {
                    if combined.contains(kw) || combined.contains(kw.to_upper()) {
                        // P2-2: 否定词检测 — 跳过被否定的风险关键词
                        if has_negation(combined, kw, neg_prefixes) {
                            // 被否定的风险关键词（如"不存在退市风险"），不计数但记录
                            continue;
                        }
                        if kw == "退市" || kw == "立案调查" || kw == "可能快速下跌" || kw == "风险警示" {
                            high_risk_count = high_risk_count + 1;
                        } else {
                            risk_count = risk_count + 1;
                        }
                        break;
                    }
                }
                // 利好关键词检测（含否定词过滤）
                for kw in pos_kw {
                    if combined.contains(kw) {
                        if has_negation(combined, kw, neg_prefixes) {
                            continue;
                        }
                        positive_count = positive_count + 1;
                        break;
                    }
                }
            }
        }
        // 合成修正信号：[-0.3, 0.3]
        // 每有一个高风险公告 → -0.15
        // 每有一个普通风险公告 → -0.05
        // 每有一个利好公告 → +0.05
        let raw_mod = high_risk_count * (-0.15) + risk_count * (-0.05) + positive_count * 0.05;
        clamp(raw_mod, -0.3, 0.3)
    } else { 0.0 }
} else { 0.0 };

// 将公告修改器应用到 f3 催化剂信号（仅当公告信号不为零时）
let f3_has_announcement_signal = announcement_modifier != 0.0;

// ── 因子 3: 催化剂 ──
// V37 修复: 催化剂信号来自 LLM（a-catalyst.content.catalyst_level），与技术面回测因子
// （volume=成交量）语义无关，不调用 get_weight 避免权重被无关回测数据绑架。
// 仅在 catalyst_level 非空且非"无"时激活默认权重；"无"表示 LLM 明确判断无催化剂。
// V38 修复: 扩展催化剂信号为双向（利好+利空），利空催化剂（退市预警/商誉爆雷等）
// 输出负信号拉低 posterior，避免对"有催化剂但实质利空"的股票给出偏多建议。
let f3_default = 0.20;
let f3_has_catalyst = present(catalyst_level) && catalyst_level != "" && catalyst_level != "无";
// P2 修复: 当有公告信号但无 LLM 催化剂时，仍以半权重激活 f3，使公告信号参与融合
let f3_weight = if f3_has_catalyst {
    f3_default
} else if f3_has_announcement_signal {
    f3_default * 0.5
} else { 0.0 };
let f3_signal = if f3_weight > 0.0 {
    // V40 修复: 用 contains 而非精确 switch 匹配，保证与 prompt 字段值对齐
    // prompt 输出格式: "L3估值体系级" / "L2业绩拐点级" / "L1普通消息"
    //                   "L-3退市/造假级" / "L-2业绩暴雷级" / "L-1普通利空"
    // V67 修复(2026-07-29): 用 f3_has_catalyst 守卫 catalyst_level 访问。
    //   原问题: 当 f3_has_catalyst=false（catalyst_level 为 unit/空/"无"）但
    //   f3_has_announcement_signal=true 时, f3_weight = f3_default * 0.5 > 0,
    //   进入本块后 catalyst_level.contains() 在 unit 上调用会抛 "Method not found",
    //   触发 catch 块导致整个 portfolio-mgr 降级为保守决策。
    //   修复: llm_signal 只在 f3_has_catalyst=true 时计算,否则为 0.0。
    let llm_signal = if f3_has_catalyst {
        if catalyst_level.contains("L-3") || catalyst_level.contains("退市") || catalyst_level.contains("造假") { -0.8 }
        else if catalyst_level.contains("L-2") || catalyst_level.contains("业绩暴雷") { -0.5 }
        else if catalyst_level.contains("L-1") || catalyst_level.contains("普通利空") { -0.2 }
        else if catalyst_level.contains("L3") || catalyst_level.contains("估值体系级") { 0.8 }
        else if catalyst_level.contains("L2") || catalyst_level.contains("业绩拐点级") { 0.5 }
        else if catalyst_level.contains("L1") || catalyst_level.contains("普通消息") { 0.2 }
        else { 0.0 }
    } else { 0.0 };
    // P2 修复: 叠加公告关键词风险信号，让公告标题层面的风险/利好修正催化剂强度
    // 公告信号权重 0.3，催化剂 LLM 信号权重 0.7，二者加权
    // 例: LLM=0.5(L2利好) + 公告=-0.15(有风险提示) → 加权=0.5*0.7+(-0.15)*0.3=0.305
    // 例: LLM=0(无催化剂) + 公告=0.2(业绩预增) → 加权=0*0.7+0.2*0.3=0.06(仍偏正)
    // 例: LLM=0(无催化剂) + 公告=-0.3(高风险公告) → 加权=-0.09(偏负，提示风险)
    if f3_has_announcement_signal {
        clamp(llm_signal * 0.7 + announcement_modifier * 0.3, -1.0, 1.0)
    } else {
        llm_signal
    }
} else if f3_has_announcement_signal {
    // 无 LLM 催化剂但有公告信号 → 直接用公告信号（权重减半避免过度影响）
    announcement_modifier * 0.5
} else { 0.0 };

// ── P1-7 修复: 确定性风险分类（优先算法，另一个口径仅作 fallback）──
// 基于 t-risk.stockRiskProfile 的量化指标做规则分类，结果覆盖 overall_risk，
// 仅当 t-risk 数据完全缺失时才 fallback 到 cls-risk-level 的分类结果。
// ⚠️ v55(2026-09-20): cls-risk-level 已由 LlmClassifierNode 改为 Rhai CodeNode，
//   其输出被包在 `result` 里 ⇒ **节点侧** input_mapping 的源路径已改为
//   `cls-risk-level.result.category`。**本文件侧零改**（键名 overall_risk_llm 与
//   下方全部消费逻辑均未动）。
//   ⚠️ 遗留：「LLM回退」这个 risk_source 字面量已名不副实（兜底者不再是 LLM）。
//      未随手改 —— 它会展示到 UI，属产品文案决策；且该分支实测命中率极低
//      （真库 25 条 completed 中仅 1 条），登记为待统一项。
// 修复(2026-07-21): overall_risk 必须先 let 声明，否则后续 `overall_risk = ...`
// 赋值会报 "Variable not found: overall_risk"（Rhai 不允许未声明变量直接赋值）
let overall_risk = "中风险";  // 默认中风险，被算法/LLM/无数据分支覆盖
let risk_source = "算法";  // "算法"/"LLM回退"/"无数据"
let risk_classified = false;

// 定义排序整数，便于比较风险等级高低

if present(risk_volatility) && risk_volatility > 0.0 {
    // 基于波动率/回撤/夏普的确定性分类（A 股校准 V54）
    let vol = risk_volatility;
    let dd = if present(risk_drawdown) { risk_drawdown } else { 0.0 };
    let sharpe = if present(risk_sharpe) { risk_sharpe } else { 0.0 };
    let roe = if present(risk_roe) { risk_roe } else { 100.0 };  // 无数据时不触发 ROE 惩罚
    let debt = if present(risk_debt_ratio) { risk_debt_ratio } else { 0.0 };
    let growth = if present(risk_revenue_growth) { risk_revenue_growth } else { 100.0 };

    // v40(2026-09-13) 极高风险判据改造（**只按数据形态，不看行业标签**）：
    //   C 「真困境」第三条件：该硬规则（负债率超高 且 营收负增长）须附加**真实的
    //     困境证据**（ROE<0 亏损，或营收增速 <−10%），防单一指标顶格。
    //   ⚠️ v78(2026-09-14) 删除原 A「金融业行业白名单豁免」：
    //      A 曾用 8 个行业关键词（金融/银行/保险/证券/信托/多元金融/非银金融/金融服务）
    //      算出 sector_is_financial，从而让**整个金融门类**永久免于该判据。
    //      这是**按行业归属下判据**，两个硬伤：
    //      ① 它等于宣称「金融企业不可能陷入财务困境」—— 一家 ROE<0 且在收缩的银行
    //         恰恰是最该顶格的标的，白名单会把真困境一起放掉；
    //      ② 行业分类口径一变（门类级 / 申万一级 / 自定义）判据即整体失效 —— 事实上
    //         它已经失效过一次：实测 stock_sector 取值域是**门类级**（"金融" 而非
    //         "银行"），只列「银行/保险/证券」时全部落空。
    //      改由 C 独立承担：C 观测的是「前提是否成立」这一**事实**（真在亏损 / 真在
    //      萎缩），与行业名无关 ⇒ 银行、保险、券商、地产、重资产周期**自动全覆盖**，
    //      无需枚举任何行业名。
    //   ⚠️ 对 601166 无影响（已正算复算）：growth=-0.3 不满足 <−10、roe=4.8 不满足 <0
    //      ⇒ C 本身为假 ⇒ 白名单删与不删都得到「中风险 / 持有」。

    // 极高风险: ST/*ST 标的或财务困境
    let is_extreme = false;
    // 财务困境: 负债率>RISK_DEBT_EXTREME 且 营收负增长 且 真困境(C)
    if debt > RISK_DEBT_EXTREME && growth < 0.0 && (roe < 0.0 || growth < -10.0) {
        is_extreme = true;
    }
    // 波动率极端 + 夏普极差
    if vol > RISK_VOL_EXTREME && sharpe < RISK_SHARPE_EXTREME { is_extreme = true; }

    // 高风险: 量化指标高波动 + 基本面无亮点
    let is_high = false;
    if (vol > RISK_VOL_HIGH || sharpe < 0.0 || dd > RISK_DD_HIGH) && (roe < RISK_ROE_HIGH || debt > RISK_DEBT_HIGH) { is_high = true; }
    if vol > 35.0 && sharpe < 0.3 && roe < 8.0 && growth < 5.0 { is_high = true; }

    // 低风险: 量化健康 + 基本面稳健
    let is_low = false;
    if vol < RISK_VOL_LOW && sharpe > RISK_SHARPE_LOW && dd < RISK_DD_LOW && roe > RISK_ROE_LOW && debt < RISK_DEBT_LOW && growth > RISK_GROWTH_LOW { is_low = true; }

    if is_extreme {
        overall_risk = "极高风险";
    } else if is_high {
        overall_risk = "高风险";
    } else if is_low {
        overall_risk = "低风险";
    } else {
        overall_risk = "中风险";
    }
    risk_source = "算法";
    risk_classified = true;
}

// LLM 回退: 仅当算法无法分类时使用
if !risk_classified && present(overall_risk_llm) {
    overall_risk = overall_risk_llm;
    risk_source = "LLM回退";
    risk_classified = true;
} else if !risk_classified {
    overall_risk = "中风险";
    risk_source = "无数据";
}

// A 股校准（V54 放宽）后，用风险等级的 rank 校准 f4 惩罚强度
let risk_rank_val = risk_rank(overall_risk);

// ── 因子 4: 风险 ──
// V37 修复: 风险信号来自 LlmClassifier（risk-level.category），与技术面回测因子
// （rsi=RSI超买超卖）语义无关，直接用默认权重，避免被 RSI 回测数据绑架。
//
// V50 修复: 优化 f4_signal 映射
//   旧: 低→+0.5 中→0.0 高→-0.4 极高→-0.7
//   问题①: 中→0.0 意味着"中风险=无风险信号"，逻辑矛盾
//   问题②: 低→中跳变 0.5 但高→极高只跳 0.3，不对称
//   新: 基于风险等级在 0-100 上的位置线性映射再反转
//      低(20)→+0.40 中(50)→-0.10 高(80)→-0.40 极高(95)→-0.65
//   原则: 中风险≠无风险，应有轻微负信号；跳变更均匀
let f4_default = 0.15;
let f4_weight = f4_default;   // overall_risk 始终存在（算法分类确保兜底）
// P0-2 修复: f4_signal 只读 LLM 分类（与 overall_risk 的算法分类解耦）
// overall_risk 已基于量化指标计算，f4_quant_adjust 又用同样的指标做修正
// 为避免双重惩罚，f4_signal 改为只读 LLM 的独立风险判断
// 当 LLM 分类缺失（算法兜底场景）时 f4_signal=0.0，由 f4_quant_adjust 独立承担
let f4_signal = if present(overall_risk_llm) && type_of(overall_risk_llm) == "string" {
    // ── 2026-09-23: `中风险` 由 `-0.05` 订正为 **`0.0`**（映射零点订正）──
    //
    // 依据（可判定，非标定）：本 switch 是「风险等级 → 证据信号」的映射，其**零点**必须是
    // 「中性风险」—— 而本模型已用 `prior = 0.50`（`regime_prior_sideways`）显式定义
    // 「无个股信息时的中性先验」。若「中风险」给出 `-0.05`，等于宣称**市场中位风险水平
    // 自带负证据**，与 `prior` 的定义自相矛盾。
    //
    // 实证（DB 全库 27 条含 portfolio-mgr 的样本）：`f4_adjusted` 中位 **-0.42**、
    // p75 = -0.28 —— 即 3/4 的样本 f4 ≤ -0.28。权重 0.15 下它单独贡献约 -0.063 的
    // `avg_signal`，而全库 `avg_signal` 中位仅 **-0.07** ⇒ **f4 一个因子的偏置就吃掉了
    // 整个融合信号的负偏**。这是「中风险」这个常态档被赋负值的直接后果。
    //
    // ⚠️ 只改零点，**不改幅度与不对称**：`低 +0.30 / 高 -0.25 / 极高 -0.50` 的负侧幅度
    // 大于正侧，是**有意**的（风险的负向影响大于正向，且高风险的处置由 `pm_risk_veto`
    // 与 `POS_CAP_*` 独立承担）。本次不动它们，避免把「零点订正」扩大成「重新标定」。
    switch overall_risk_llm {
        "低风险"|"低" => 0.30,
        "中风险"|"中" => 0.0,
        "高风险"|"高" => -0.25,
        "极高风险"|"极高" => -0.50,
        _ => 0.0
    }
} else { 0.0 };

// ── P0-2 补: 量化风险指标修正 f4_signal ──
// t-risk ToolNode 输出 stockRiskProfile 含波动率/回撤/夏普等真实风险指标
// 修复前: 这些指标通过 input_mapping 注入但未被 rhai 消费（数据流断裂）
// 修复后: 基于实际数值对 f4_signal 做合理修正
let f4_quant_adjust = 0.0;  // 累计修正量（风格相关项已按成长豁免打折）
let f4_quant_notes = [];
// ── P1-B: 成长风格豁免（成长股风险因子去偏，2026-09-11）──
// 实证（DB，12 条新模板运行）：risk_revenue_growth = 17.2 / 49.1 / 72.9（高成长标的），
//   同时 risk_volatility = 76.7 / 86.6 / 97.3（全部落 >60% 的「极高」档），
//   f4_quant_adjust 三重惩罚叠加（波动 -0.12 + 回撤 -0.10 + 负夏普 -0.08）
//   → f4_signal 实测 -0.42 ~ -0.78，是仅次于 f5 的第二大 posterior 拖累。
// 问题：高增速标的的波动率天然高于价值股（成长溢价必然伴随高波动），
//   用全市场统一的波动/回撤阈值惩罚 = 系统性歧视成长风格，
//   与「瓶颈掘金」（挖未起飞的 pre-takeoff 公司）的选股逻辑直接冲突
//   —— 越是高成长的标的越会被风控因子压死。
// 方案：**仅对风格相关惩罚打折**（波动率 / 最大回撤 / PE 偏高），
//   按营收增速分档（≥40% → 0.35 折｜≥25% → 0.55 折｜≥15% → 0.75 折）。
// 刻意**不打折**的三项（保持风控实质）：
//   ① 夏普比率 —— 真实风险调整后收益，与风格无关；
//   ② PE ≤ 0 / 缺失（-0.08）—— 财务质量信号（亏损企业），非风格问题；
//   ③ overall_risk 风险分类本身 —— 影响 risk_bias 与仓位 cap，属组合层约束，保持独立。
let growth_exempt = if present(risk_revenue_growth) && type_of(risk_revenue_growth) == "f64" {
    if risk_revenue_growth >= 40.0 { 0.35 }
    else if risk_revenue_growth >= 25.0 { 0.55 }
    else if risk_revenue_growth >= 15.0 { 0.75 }
    else { 1.0 }
} else { 1.0 };
if growth_exempt < 1.0 {
    f4_quant_notes.push(`成长豁免×${growth_exempt}（营收增速${(risk_revenue_growth*10.0).round()/10.0}%）`);
}
if present(risk_volatility) && risk_volatility > 0.0 {
    // 年化波动率: >60% 极高风险 | >40% 高风险 | <20% 低风险
    if risk_volatility > 60.0 {
        f4_quant_adjust -= 0.12 * growth_exempt;
        f4_quant_notes.push(`年化波动${(risk_volatility*10.0).round()/10.0}%（极高）`);
    } else if risk_volatility > 40.0 {
        f4_quant_adjust -= 0.06 * growth_exempt;
        f4_quant_notes.push(`年化波动${(risk_volatility*10.0).round()/10.0}%（偏高）`);
    } else if risk_volatility < 20.0 {
        f4_quant_adjust += 0.05;
    }
}
if present(risk_drawdown) && risk_drawdown > 0.0 {
    // 最大回撤: >50% 触发 VaR 预警 | >30% 注意
    // P1-B: 回撤同属风格相关项（高成长股回撤天然更深），按成长豁免打折
    if risk_drawdown > 50.0 {
        f4_quant_adjust -= 0.10 * growth_exempt;
        f4_quant_notes.push(`最大回撤${(risk_drawdown*10.0).round()/10.0}%（VaR预警）`);
    } else if risk_drawdown > 30.0 {
        f4_quant_adjust -= 0.05 * growth_exempt;
        f4_quant_notes.push(`最大回撤${(risk_drawdown*10.0).round()/10.0}%（偏高）`);
    }
}
if present(risk_sharpe) {
    // 夏普比率: <0 无效 | <0.5 差 | >1.0 好 | >2.0 优秀
    if risk_sharpe < 0.0 {
        f4_quant_adjust -= 0.08;
        f4_quant_notes.push(`夏普${(risk_sharpe*100.0).round()/100.0}（负值）`);
    } else if risk_sharpe < 0.5 && risk_sharpe > 0.0 {
        f4_quant_adjust -= 0.03;
    } else if risk_sharpe > 2.0 {
        f4_quant_adjust += 0.05;
    } else if risk_sharpe > 1.0 {
        f4_quant_adjust += 0.02;
    }
}
// PE 处理: 区分 0 占位（无数据）、亏损（负值）、偏高(>100)、极端高(>200)
// 2026-09-21 订正: 上游三家 vendor（xueqiu/eastmoney/browser_eastmoney）不再把
//   负 PE 过滤成 None（负值 = 亏损，是**有效信息**而非缺失）⇒ 本段必须自己分流：
//   负值 = 亏损企业；0 = vendor 缺字段时的占位值（与缺失同义）。
// P1-B: 「偏高」两项按成长豁免打折（高成长股 PE 天然高，用统一阈值惩罚=歧视成长风格）；
//       亏损保持原样不打折 —— 那是财务质量而非风格。
if present(risk_pe) && type_of(risk_pe) == "f64" {
    if risk_pe > 200.0 {
        f4_quant_adjust -= 0.05 * growth_exempt;
        f4_quant_notes.push(`PE=${(risk_pe*10.0).round()/10.0}（极端高）`);
    } else if risk_pe > 100.0 {
        f4_quant_adjust -= 0.03 * growth_exempt;
        f4_quant_notes.push(`PE=${(risk_pe*10.0).round()/10.0}（偏高）`);
    } else if risk_pe < 0.0 {
        // 负数 = 亏损企业（EPS<0 ⇒ PE 无市盈率含义）。这是**已知事实**，
        // 不是「数据缺失」——不得写进 data_gaps（见下方缺失判据）。
        f4_quant_adjust -= 0.08;
        f4_quant_notes.push(`PE=${(risk_pe*10.0).round()/10.0}（亏损企业，市盈率无含义）`);
    } else if risk_pe == 0.0 {
        // 0 = vendor 缺字段时的占位值，与「缺失」同义（data_gaps 亦按缺口登记）。
        f4_quant_adjust -= 0.08;
        f4_quant_notes.push("PE数据缺失（上游返回 0 占位）");
    }
}
let f4_adjusted = clamp(f4_signal + f4_quant_adjust, -1.0, 1.0);

// ── 因子 5: 估值风险 ──
// V51 新增: 估值风险信号来自 t-valuation（DCF/格雷厄姆上行空间）
//   估值过高 = 风险（负信号），估值低估 = 机会（正信号）
//   与 f4_signal（技术面/基本面风险）正交，独立衡量估值维度。
let f5_default = 0.15;
// V56: 估值因子消费 IC 代理权重（key="valuation"，由 compute_factor_backtest_stats 输出）
// 激活条件: t-valuation 已执行且输出有效（dcf_upside 或 graham_upside 存在）
//
// ── V77(2026-09-14): DCF **模型适用性门**（比 fallback 衰减更强的一档处置）──
//
// `valuation_dcf_applicable` ← `dcf.assumptions.applicable`，由 `compute_dcf` 按
// **数据形态**判定（不看行业标签，故银行/保险/券商/地产/重资产周期自动全覆盖）：
//   ① 资产负债率 > 80% —— 净利由权益乘数驱动，企业自由现金流口径不成立
//   ② 净利为正而当期真实 FCF ≤ 0（符号相反），或 `FCF/净利 < 0.3`
//   ③ 预测期负增长且终值现值占比 > 70% —— 结论由永续假设独裁
//
// 601166 实证（2026-09-14）：命中 ①②，DCF 给 40.17–49.26 元（`upsidePct = 143.1`，
//   现价 18.15），而 LLM 层给「观望 + 0% 仓位」——同一份输出里两个结论互相打脸。
//   根因：拿「近 5 年净利均值 × 0.90 = 730.5 亿」当**自由现金流**给银行折现，
//   而银行没有「企业自由现金流」概念（存款/贷款是经营原料，不是可分配现金）。
//   这个 44.13 元**没有经济含义**，把它读成「低估 143%」是类别错误。
//
// 处置 = **整腿剔除**，不是衰减：
//   衰减（如 `dcf_anchor_decay` 的 ×0.5）的语义是「这个数还有部分信息量，只是可信度低」；
//   而 `applicable == false` 的语义是「这个数不构成估值证据」⇒ 只能剔除。
//   剔除后由 graham 腿独立承担估值维度（格雷厄姆公式基于 EPS，对银行不失效）；
//   两腿都不可用时 `f5_weight = 0`，该因子对整个 posterior 贡献 0（不是中性 0.15，
//   也不是负分 —— 「无法判断」既不该加分也不该扣分）。
let dcf_leg_usable = present(valuation_dcf_upside)
    && !(present(valuation_dcf_applicable) && valuation_dcf_applicable == false);
let graham_leg_usable = present(valuation_graham_upside);
let f5_has_valuation = dcf_leg_usable || graham_leg_usable;
let f5_weight = if f5_has_valuation {
    get_weight(f_weights, "valuation", f5_default)
} else { 0.0 };
let f5_signal = if f5_weight > 0.0 {
    // 估值上行空间 → 信号映射：低估→正，过高→负。
    // dcf_upside / graham_upside 均为百分数（如 -40.8 表示下跌空间 40.8%）。
    // P3 修复(2026-09-11): 原先各自裸 clamp(x/30, -1, 1)。实测 12 条生产样本中
    //   dcf_upside 中位 −100、graham_upside 中位 −93.0 → 两边恒被 clamp 到 −1.0，
    //   强度信息全丢（DCF 显示 50% 低估也照样判高估）。改用 pm_saturate(x, 40)：
    //   r=±40% → ±0.5，r=±120% → ±0.75，单调且不饱和，与 f7/f9 的既有惯例对齐。
    let dcf_sig = if dcf_leg_usable {
        pm_saturate(valuation_dcf_upside, 40.0)
    } else { 0.0 };
    // ── V78(2026-09-21): 格雷厄姆腿的**对称护栏**（增长率封顶 ⇒ 软衰减）──
    //
    // `valuation_graham_growth_clamped` ← `graham.assumptions.growthClampedUpper`：
    //   true 表示 `revenue_yoy` 超过 `MAX_GROWTH(30%)` 被封顶 ⇒ 代入公式的增长率
    //   **低于**实际增速 ⇒ 内在价值系统性偏低 ⇒ `upsidePct` 偏负（**看空被夸大**）。
    //   与 DCF 侧 `is_fallback_anchor`（代理锚偏低）**同方向**，故取同族处置：软衰减。
    //
    // 为什么需要它 —— V77 的一个未覆盖分支：
    //   V77 让 `applicable=false` 时**整腿剔除 DCF**，由 graham 腿独立承担估值维度。
    //   该设计对金融/高杠杆标的成立（格雷厄姆走 EPS，不受 FCF 口径影响；601166 实证
    //   graham 给 +60.7% 而 DCF 给 +143% 是错的）。但对**高增长**标的：
    //   剔除 DCF 后 `fusion_wsum` 只剩 0.3 ⇒ graham 腿**独占 100%**（相对原 30%
    //   份额放大 3.33 倍），而它的增长率假设恰好也被顶死 —— 300308 实测
    //   2025 年报营收增速 182.5% 被封顶到 30%，`graham.intrinsicValue = EPS × 68.5`，
    //   `σ = pm_saturate(+36.6, 40) = +0.478`（用落库 round 值 `iv=168.08` 反解则得
    //   `+0.463` —— 同源、仅 round 精度不同，非分歧）。
    //   ⚠️ 2026-09-22 量纲修复前该腿为 `EPS × 9.1` / `σ = −0.672` —— **符号已翻转**，
    //   本段旧版记的绝对量级（0.672 / −0.339）随之作废，勿沿用。
    //   但下述论点与 σ 的**符号无关**，只取决于「权重占比」这一结构量：
    //   剔除 DCF 后 graham 腿独占 100%（相对原 30% 份额放大 3.33 倍），
    //   其 σ 不再被另一腿稀释 ⇒ 正是本护栏（`×0.5` 软衰减）针对的场景。
    //
    // ⚠️ 与 `applicable=false` 的分工（**不可混用**）：
    //   `applicable=false`     数值**没有经济含义**            ⇒ 整腿剔除；
    //   `growth_clamped=true`  数值**仍有信息量**、假设被压缩  ⇒ 软衰减。
    // 缺该字段（旧模板未注入）时取 1.0，行为与改动前完全一致。
    let graham_leg_decay = if present(valuation_graham_growth_clamped)
        && valuation_graham_growth_clamped == true { 0.5 } else { 1.0 };
    let graham_sig = if graham_leg_usable {
        pm_saturate(valuation_graham_upside, 40.0) * graham_leg_decay
    } else { 0.0 };
    // F-Score 加成: score >= 7（财务强劲）→ 估值信号可信度更高，信号放大 1.3x
    let fscore_mult = if present(valuation_fscore) && valuation_fscore >= 7 { 1.3 } else { 1.0 };
    // ⚠️ 护城河乘子（`moat_mult`）**已下移**到融合之后 —— 它需要 `fused_sig` 判定
    //   信号方向（V79 方向对称），而 Rhai 是顺序求值，前向引用不可用。
    //   历史沿革与取值域对齐的论证一并随块下移。
    // P3 修复(2026-09-11): 原「取 |绝对值较大者」，在 A 股高 PB 成长/周期样本上
    //   等价于恒选 graham（同上 12 样本：graham 中位 −93.0，dcf 中位 −100，
    //   但 graham 的 |x| 在绝大多数个股上更大），使 DCF 维度完全失效。
    //   改为加权融合 dcf 0.7 / graham 0.3：
    //     - dcf 是「情景三档 + 净利归一化」锚定，对成长股更适配，作主导项；
    //     - graham 保留 30% 作低 PB 校正（银行/地产等 PB<1 样本下可给正信号，
    //       本轮 12 样本全为高 PB，证据不足以直接剔除）。
    //   权重按实际可用项归一（fusion_wsum），避免单边缺失时信号按比例缩水。
    // ── V79(2026-09-21): PE 历史分位 = 第三条锚腿 ──
    // `valuation_pe_percentile` ← `t-valuation-band.result.content.metricPe.currentPercentile`
    //   （0 = 近 5 年最便宜，100 = 最贵），来自 `astock-data::valuation_band`，
    //   数据源是 `financial_snapshots` 的**日频 5 年**序列（DB 实况 15597 行 / 13 只，
    //   `pe_ttm`/`pb`/`ps_ttm` 填充率 100%）。
    //
    // 为什么加它 —— 现有两腿都有**共同的失效面**：
    //   ① DCF 腿：依赖 FCF 口径。`applicable=false`（杠杆/FCF 背离/负增长独裁）时整腿剔除，
    //      而命中的恰是银行/保险/券商/地产/重资产周期 ⇒ 这批标的**只剩 graham 一条腿**，
    //      而 graham 是纯 PE 恒等式（EPS 约掉，见 §二），近乎不含公司特定信息。
    //   ② graham 腿：高增长标的 `revenue_yoy` 被封顶到 30% ⇒ 内在价值系统性偏低。
    //   ⇒ 需要一个**既不依赖 FCF、也不依赖增长率假设**的锚。PE 分位只问
    //   「当前 PE 在它自己的 5 年分布里处于什么位置」，两者都不依赖。
    //
    // ⚠️ 它与 graham 的信息**不重复**：graham 给的是「当前 PE 相对 8.5+2g / 债券收益率
    //   基准的偏离」（绝对口径），本腿给的是「当前 PE 相对**自身历史**的位置」（相对口径）。
    //   同一只股票完全可能「graham 说高估、历史分位说便宜」（处于估值中枢下移期）。
    //
    // 信号映射用**线性**而非 pm_saturate：分位本身已是 [0,100] 的有界量，
    //   50 是天然中性点 ⇒ `(50 − pct)/50` 恰好给出 [−1, +1]，且 pct=25 → +0.5、
    //   pct=75 → −0.5，与 `valuation_band` 自带的 25/75 阈值口径逐字一致。
    //   若改用 pm_saturate 需再引入一个标定常数（且极值只到 ±0.5），徒增一个可调旋钮。
    //
    // ⚠️ 有效性门必须**按数据形态**判（不用 `present` 就够）：`currentPercentile`
    //   在「PE ≤ 0（亏损）」与「有效样本 < 20」两种情形下为 null/缺；即便有值，
    //   `compute_valuation_band` 的 verdict 也可能因样本不足为 `insufficient`。
    //   故此处只接「值落在 [0,100]」的分位；越界或缺失 ⇒ 该腿不可用、按可用腿归一化。
    // ── 2026-09-23：把上方的**声明**变成**实现**（补质量门，读 `verdict`）──
    //
    // 上方原注释写「即便有值，`compute_valuation_band` 的 verdict 也可能因样本不足为
    // `insufficient` ⇒ 故此处只接「值落在 [0,100]」的分位」—— 这句「故」在逻辑上是断的：
    // 承认了 verdict 可能不可靠，处置却是**只查值域**，等于**从未读过 verdict**。
    // 实测（2026-09-23）：`valuation_band_verdict` 在 Rhai 侧、在整个 `_raw` 输入映射里
    // **零注入**（只有 `valuation_pe_percentile` 一条）⇒ 该门此后形同虚设。
    //
    // 为什么这构成实质缺陷而不是「少一道保险」：
    //   `valuation_band.rs` 对样本不足的处置是 `verdict = "insufficient"`
    //   （`pe.sample_size < 20 || pb.sample_size < 20`），但 `current_percentile`
    //   **在该状态下仍可能有值**（只要 `current > 0` 且历史序列非空）
    //   ⇒ 该腿会以**满强度**（线性映射到 ±1，是本组量程最大的腿）进入融合，
    //   拿一个「仅供参考」的分位去和 DCF/graham 平均。
    //   与 DCF 腿的 `applicable`、graham 腿的 `growthClampedUpper` 相比，
    //   本腿是**唯一没有任何质量声明可消费**的一条 —— 根因是声明没被接线，不是没有声明。
    //
    // 向后兼容：旧模板未注入 `valuation_band_verdict` ⇒ `present()` 为 false ⇒
    //   `band_insufficient` 恒 false ⇒ 行为与改动前**逐位一致**。
    //   ⚠️ 本改动**只**消费 verdict 的「质量」语义（`insufficient`），**不用**它的方向
    //   （`overvalued` / `deep_value` 等）—— 方向已由 `currentPercentile` 线性表达，
    //   若同时用 verdict 定方向会造成同一信息二次计入。
    let band_insufficient = present(valuation_band_verdict)
        && type_of(valuation_band_verdict) == "string"
        && valuation_band_verdict == "insufficient";
    let band_leg_usable = !band_insufficient && present(valuation_pe_percentile)
        && valuation_pe_percentile >= 0.0
        && valuation_pe_percentile <= 100.0;
    let band_sig = if band_leg_usable {
        clamp((50.0 - valuation_pe_percentile) / 50.0, -1.0, 1.0)
    } else { 0.0 };
    // 权重 0.56 / 0.24 / 0.20 —— 前两腿**保持 7:3 内部比例**（0.56:0.24 = 7:3），且三者同缩
    //   到 0.8 的基数上，于是 band 腿缺位时归一化结果**逐位等于改动前的 0.7/0.3**：
    //   `wsum = 0.56 + 0.24 = 0.8`，dcf 份额 `0.56/0.8 = 0.7`、graham `0.24/0.8 = 0.3`。
    //   这条恒等式是向后兼容的**唯一保证**：旧样本（未跑分位带节点）行为完全不变，
    //   只有取到分位的新样本才引入第三条腿。
    //   ⚠️ 若误写成 0.7/0.3/0.2（三者和 1.2），band 缺位态**也**退化成 0.7/0.3（同样正确），
    //   两者的差别只在「band 可用时 dcf 份额」：0.56 vs 0.583。选 0.56 的理由是
    //   三者和恰为 1.0 ⇒ **band 腿份额恒为 0.20**（后者为 0.2/1.2 = 0.167），
    //   「新腿占两成」这句话才在两种状态下都成立、可直接断言。
    let fusion_wsum = (if dcf_leg_usable { 0.56 } else { 0.0 })
        + (if graham_leg_usable { 0.24 } else { 0.0 })
        + (if band_leg_usable { 0.20 } else { 0.0 });
    // V79: P0-I 的 fallback 衰减从「作用于融合后总值」改为**只作用于 DCF 腿自身**。
    //   定义提前到此处（原在下文 clamp 前），因为分腿衰减必须在**融合之内**生效。
    //   完整论证见下方 P0-I 注释段。
    let dcf_anchor_decay = if dcf_leg_usable
        && present(valuation_dcf_anchor_is_fallback)
        && valuation_dcf_anchor_is_fallback == true { 0.5 } else { 1.0 };
    let fused_sig = if fusion_wsum <= 0.0 {
        0.0
    } else {
        ((if dcf_leg_usable { dcf_sig * dcf_anchor_decay * 0.56 } else { 0.0 })
            + (if graham_leg_usable { graham_sig * 0.24 } else { 0.0 })
            + (if band_leg_usable { band_sig * 0.20 } else { 0.0 })) / fusion_wsum
    };
    // ── 护城河乘子（P2 新增；2026-09-21 取值域对齐 + V79 方向对称）──
    // 护城河是估值溢价的核心驱动因素：
    //   宽阔 → 看多 ×1.15 / 看空 ×0.85
    //   狭窄 → 看多 ×1.05 / 看空 ×0.95
    //   无/未知 → ×1.0（不提估值）
    //
    // ① 取值域对齐（2026-09-21，**bug 修复，不是调参**）：
    //   原比对值是 `"宽护城河"` / `"wide"` / `"窄护城河"` / `"narrow"` —— 四个**全都
    //   不可能命中**：本变量的真实来源是 `input_mapping` 的
    //   `t-valuation.result.content.moat.label`，而生产端 `compute_moat_score`
    //   的取值域是 **`"宽阔"` / `"狭窄"` / `"无"`**（三档）
    //   ⇒ 该乘子**恒为 1.0**，是从未生效过的死分支。
    //   为什么原作者会写成「宽护城河」：value-investor 节点输出的
    //   `verdict.moat_rating` 确实写 `"宽护城河"`（LLM 把算法的「宽阔」转写成了
    //   行业惯用语）—— 那是**另一套词汇表**，与 `input_mapping` 接入的算法字段
    //   不是同一个东西。DB 实证（38 条样本）：`moat.label` 取值
    //   宽阔 2 / 狭窄 3 / 无 16 / 缺失 17，按 `"宽护城河"` 比对命中 **0/21**。
    //   ⚠️ 取值域两侧分别手抄在 Rust（`compute_moat_score`）与 Rhai（此处），
    //   由 `seed_consistency_tests::moat_level_vocabulary_matches_rhai_consumer` 钉住。
    //
    // ② 方向对称（V79，2026-09-21 用户裁决）：改动前对**负信号**走的是 `|σ| × 乘子`
    //   ⇒ 宽阔护城河反而把看空信号放大 1.15 倍。这与本块的自身声明
    //   （「估值应享受溢价」）**方向相反**：护城河越宽，等于说越该看空。
    //   ⇒ 改为按信号方向对称作用：看多时给溢价，看空时给折价（`1 − premium`）。
    //
    //   为什么是 `1 − premium`（0.85/0.95）而不是 `1/(1+premium)`（0.870/0.952）：
    //   两种「对称」定义都自洽，但量化结论（`docs/audits/AUDIT-dcf-cashflow-anchor-2026-09-21.md`
    //   §6.10.3 的逐样本表与 Δposterior ≈ +0.008）是按**加减对称**算的，
    //   故落地取 0.85/0.95。两者对 action 的影响均不足以翻转（阈值带宽 0.10）。
    //
    //   ⚠️ 已量化：38 条样本里 `moat ∈ {宽阔,狭窄}` 仅 **5 条**（4 只去重），
    //   且 5/5 的乘子前 σ 全为负 ⇒ 候选「只在正信号段生效」对这 5 条等价于完全不生效。
    //   方向对称后 300308 Δσ = +0.1016、均值 −0.3234 → −0.2937，
    //   Δposterior ≈ **+0.008** ⇒ **不可能翻转任何 action**。
    //   ⇒ 本项定位是**一致性修正**（消除「声明与实现相反」），不是结果修正。
    //
    // 判定基准用 `fused_sig`（含分腿衰减的融合值）：`dcf_anchor_decay` 是正值常数,
    //   不改变符号 ⇒ 与用未衰减值判向结果**恒等**,故无需另存一份未衰减副本。
    let moat_mult = if present(valuation_moat) && type_of(valuation_moat) == "string" {
        if valuation_moat == "宽阔" { if fused_sig >= 0.0 { 1.15 } else { 0.85 } }
        else if valuation_moat == "狭窄" { if fused_sig >= 0.0 { 1.05 } else { 0.95 } }
        else { 1.0 }
    } else { 1.0 };
    // ── P0-I(2026-09-12): fallback 锚定下的 f5 置信度衰减（shrinkage）──
    // `valuation_dcf_anchor_is_fallback` 来自 t-valuation 的
    // `dcf.assumptions.is_fallback_anchor`：true 表示 FCF 锚定**不是当期真实 FCF**，
    // 而是「当期 FCF≤0 ⇒ 近 5 年报正净利均值 × 0.90」的历史代理
    // （`dcf.assumptions.basis` 给出具体口径；2026-09-21 起该字段**分三态**：
    //  「当期FCF≤0」= 标的当期现金流真为负；「现金流量表数据缺失」= 我方未取到数。
    //  两态处置相同 —— 锚都是历史代理，故都应为 true —— 但诊断文案必须区分。
    //  ⚠️ 不要在 Rhai 侧按 `basis` 文案判分支：那正是 P0-I 当初改用布尔量的原因）。
    //
    // 为什么要压：该锚定**回溯且偏低**，用它给成长/转型标的定价会系统性低估 ——
    //   603353 实证（模板 v37，样本 997cdf50）：basis 命中该 fallback，
    //   `dcf.upsidePct = −90.8` ⇒ `f5 σ = −0.696`，权重 0.21 为全因子最大
    //   （再乘 regime_mod 1.4）⇒ 该因子单独贡献 −0.1462，占**净负贡献**
    //   (Σ(σ·w) = −0.1688) 的 87%，即「负向证据几乎全来自这个退化锚定」。
    //   模型自己已在 basis 里声明这是退化锚定，却仍与「当期真实 FCF」
    //   拿同等的信号强度 —— 属自相矛盾的权重配置。
    //
    // ⚠️ 但**不要**把这个衰减说成「能把 action 抬起来」——已实测反证：
    //   用 DB 存档 σ/w 正算复现 v37（posterior 残差 0.035pt）后扫描：
    //     k=1.0 → eff 0.3027（减持）  k=0.5 → 0.3318（减持，本处取值）
    //     k=0.25 → 0.3464（减持）     k=0.0 → 0.3610（减持，σ 完全中性化）
    //   即**即使把 f5 完全中性化也仍是「减持」**。action 的真实约束是
    //   `prior=0.45(bear) + risk_bias=−0.08(高风险)` 且正向因子合计仅 0.157：
    //   把所有 σ<0 的因子全部归零，上限也只有 eff=0.4327（观望），
    //   距 ACTION_HOLD_THRESHOLD(0.48) 仍差 0.047。本衰减的定位是
    //   **置信度纠正（阻止退化锚定冒充证据）**，不是 action 翻转器。
    //
    // 为什么只压 σ、不动权重：σ 是**信号置信度**，权重是**信息配比**。
    //   此处要压的是「这个估值结论有多可信」，不是「估值维度该占多少信息」；
    //   动权重会连带改 total_weight / evidence 口径，进而污染 R-203 权重坍缩判定。
    //
    // 系数 0.5 = 标准 shrinkage 量级（把该因子贡献腰斩）。
    //   若将来要按标的分层标定，应上提为面板可调参数（走「参数四道门」），
    //   而不是继续改这个常数。
    //   `present()` 包裹保证旧模板（未注入该变量）下安全取到 () —— rt-workflow
    //   V57 会自动为 `present(x)` 里的未注入变量补 unit 默认值。
    //
    // V77(2026-09-14) 补门：本衰减只描述「**DCF 腿**的 FCF 锚定是历史代理」，
    //   故必须与 `dcf_leg_usable` 联立 —— 否则在 DCF 已被判不适用、只剩 graham 腿
    //   承担估值维度时，会用「DCF 的锚定缺陷」去惩罚一个**与 FCF 无关**的格雷厄姆信号。
    //
    // ── V79(2026-09-21) 分腿：把 V77 的**声明**变成**完全实现** ──
    //   V77 的措辞是「本衰减只描述 DCF 腿的锚定」，但实现是 `fused × 0.5` ——
    //   即乘在**融合后的总值**上，等于**同时**把 graham 腿也减半。
    //   两者之差恒为 `+0.15 × graham_sig`（`0.7·(dcf×0.5) + 0.3·g` vs
    //   `0.5·(0.7·dcf + 0.3·g)`），即 graham 腿**恢复满强度** ⇒ σ 按 graham 自己的
    //   符号更极端（**不是**更看多）。
    //
    //   量化（§6.10.4 逐样本，`output/tmp-continue-analyze.mjs`）：影响面 =
    //   `is_fallback_anchor=true` 且两腿齐全 = **8 / 38**（21%）。
    //   均值 σ −0.1284 → −0.1862（更看空），Δ 均值 −0.0578；但 601166 反而
    //   −0.3640 → −0.4543（其 graham 为 +60.7%，符号自洽）⇒ 8 条里 6 条 graham<0。
    //   Δposterior ≈ 0.005 ⇒ 同样**不改变任何 action**。
    //   故本项与 `moat_mult` 方向对称同属**一致性修正**，非结果修正。
    //
    //   实现：定义已上移到融合之前（`dcf_anchor_decay`，仍带 `dcf_leg_usable &&` 门），
    //   在融合表达式内**只乘 DCF 腿**。最终 clamp **不再**乘它 —— 否则会二次衰减。
    //   `fused_sig` 此时已是分腿衰减后的值。
    //
    // ⚠️ 改本文件即改 CodeNode 内容 ⇒ 必须升 `TEMPLATE_VERSION`（不升则不落库）。
    clamp(fused_sig * fscore_mult * moat_mult, -1.0, 1.0)
} else { 0.0 };

// ── 因子 6: 数据质量 ──
// V37 修复: 数据质量信号来自 AgentNode（data-quality.content.score），与技术面回测因子
// （bias=乖离率）语义无关，直接用默认权重，避免被乖离率回测数据绑架。
// V66 修复(2026-07-29): F 级（dqi_score<25）时 f6_signal 设为 0（中性），避免自指双重惩罚。
//   原问题: F 级时 f6_signal=(dqi-50)/50=-1.0（强负信号）拉低 posterior 到 0.3 以下,
//     导致 base_action="卖出"。但 F 级语义是"数据不足无法判断"（应观望）,
//     而非"看空"（卖出）。dqi_collapsed 已强制 position_pct=0,
//     再让 f6_signal 拉低 posterior 到"卖出"区间是双重惩罚且语义矛盾。
//   修复: F 级时 f6_signal=0,让 posterior 由其他因子自然决定,
//     再由 position_pct=0 的 action 降级逻辑（L1148）统一收敛为"观望"。
let f6_default = 0.15;
// 2026-09-23 退出语义：`f6_weight` 的定义**下移到 `f6_signal` 之后**（见下方）——
//   它必须由「本因子是否真的产生方向性信号」决定，而不是「`dqi_score` 字段是否存在」。
//   完整论证见下方 `f6_weight` 定义处。
// V75 修复(2026-09-13): f6 只做「不可信惩罚」，不再产生正向信号。
//   原实现 `(dqi_score-50)/50` 把「数据完整性」当成了方向信号：C 级 61.3 分
//   → +0.226 的正贡献（权重 0.15），语义等于「数据齐全 ⇒ 看多」。这是类别错误 ——
//   数据质量度量的是**证据可靠度**，不是标的涨跌方向。实证 601166（2026-09-12）：
//   f6 单独贡献 +0.034，直接抬高 posterior（avgSignal=+0.0638）。
//   修复后：dqi≥50 → 0（数据够用即中性，不再加分）；dqi<50 → 线性负分（越缺越扣）。
//   ⚠️ 该改动会**普遍小幅下移** posterior（凡 dqi>50 的标的），属口径纠正；
//   若需恢复"高质量数据略微加分"的建模意图，应改为调制 confidence 而非 posterior 方向。
let f6_signal = if present(dqi_score) {
    if dqi_score < 25.0 { 0.0 }  // V66: F 级中性化，避免自指双重惩罚
    else if dqi_score < 50.0 { (dqi_score - 50.0) / 50.0 }
    else { 0.0 }
} else { 0.0 };
// ── 2026-09-23「退出语义」：σ = 0 的因子必须**退出权重**，不得以 0 值占权重 ──
//
// 病根（**不是** f6 独有，是融合层的一处系统缺口）：
//   本因子的 σ = 0 有两种来源，**语义都等于「本因子没有信息」**：
//     · `dqi_score >= 50` → V75 明确定义「数据够用即中性，不再加分」（f6 只做不可信惩罚）；
//     · `dqi_score < 25`  → V66 明确定义「F 级语义是数据不足无法判断（应观望），
//                            而非看空」⇒ 中性化。
//   但旧实现让这个「无信息」的因子在 `total_weight`（分母）里**照占 0.15** ⇒
//   `avg_signal = Σ(σ·w) / Σw` 被一个零分子稀释，其余真正携带证据的因子被系统性摊薄。
//
// 为什么这是缺陷而不是「保守设计」：
//   σ = 0 在贝叶斯式加权平均里**不是中性证据，而是缺测**。把缺测按「观测到中性」
//   计入分母，等于宣称「我查了数据质量，它是中性的」—— 而实际上我们什么都没测到。
//   当多个因子同时如此（本轮样本 f6/f7 双双为 0，合计 0.25 权重）时，`avg_signal`
//   被压向 0 ⇒ 落「观望」档 —— 结论既不反映证据，也不反映「无证据」。
//
// ⚠️ 本修复**不是新设计**：同一文件 f11 的降级分支早已确立该范式 ——
//   「`pace_degraded` ⇒ f11 权重归零，**避免误用 `pace_signal=0` 作为中性信号**」。
//   f6/f7 是**同一形态未推广**（f13 的「条件计入」亦同族）。故此处与它们对齐，
//   而不是新增一套判据。
//
// 量化（2026-09-23，近 24 条含 f6 的分析样本）：f6 σ = 0 占 **9 条**（37.5%）、
//   f7 σ = 0 占 **8 条**（33.3%）⇒ 平均约 1/3 的样本里有 0.15~0.25 权重长期空转。
//
// ⚠️ 与 `max_weight` 定义处的关系：该行**刻意不退**本项（其注释说明它是「活跃因子的
//   **默认**权重之和」，按 default 而非实际计入）。故本改动会让 `evidence_scale =
//   pm_evidence_scale(total_weight, max_weight)` 随 total_weight 下降而**同步下降** ——
//   这是良性耦合：证据量减少 ⇒ 后验收缩（`raw_val = prior + avg_signal * evidence_scale`），
//   与「退出权重」的语义一致，不构成二次惩罚。
let f6_weight = if f6_signal != 0.0 { f6_default } else { 0.0 };

// ── 因子 7: 交易方案方向与质量 ──
// V65 架构调整: LLM trader 进一步降权至 10%（原 15%/30%）。
// 原因：trader 现在输出与 portfolio-mgr 同维度的完整决策（action/positionPct/riskLevel），
// 不再是"提案"，而是"独立 LLM 视角"。f7 因子如果消费 trader 的 action/positionPct，
// 会导致公式决策与 LLM 决策产生自指污染（compute_decision_agreement 的 f7_weight_pct 标记）。
// 因此 V65 将 f7 降权至 0.10，仅作为"定性方向参考"，定量决策（仓位/止损/止盈）
// 完全由公式独立计算，避免双视角对比虚高。
//
// f7_signal 综合考虑:
//   ① trader.verdict 方向（基础信号，权重 60%）
//   ② trader.confidence 调制（0.5 为中性，>0.5 加强 / <0.5 减弱）
//   ③ trader.evidence_cited 数量（V65 新增，引用 ≥3 条加强信号，<3 条减弱）
//   ④ trader.riskLevel 风险等级（V65 新增，极高风险/高风险减弱信号幅度）
// 不消费 trader.positionPct（避免仓位自指）、不消费 trader.action（避免 action 自指）。
// 2026-09-23 退出语义：`f7_weight` 的定义**下移到 `f7_signal` 之后**（见该处注释）——
//   由「本因子是否真的产生方向信号」决定，而不是无条件 0.10。
let f7_signal = if present(trader_direction) && type_of(trader_direction) == "string" {
    // V65 新路径: 定性方向 + 置信度 + 证据密度 + 风险调制
    let dir = trader_direction.to_lower();
    // ── V75 修复(2026-09-12): trader_confidence 单位不一致 ⇒ f7 退化为「伪二值」──
    //   本段按 0~1 设计（见下方 ②「0.5 为中性，>0.5 加强 / <0.5 减弱」），
    //   但输入映射是 trader_confidence ← trader.content.verdict.confidence，
    //   即 LLM 的 0~100 口径（DB 实证：70 / 92 / 85）。
    //   于是 conf*0.6*combined_mod 恒 ≥ 4，远超上限 0.6，被 clamp 死死顶在边界 ⇒
    //   f7 ∈ {+0.6, -0.6, 0} 三值，**幅度信息全部丢失**：
    //   conf 大小、evidence_mod(引用密度)、risk_mod(风险等级) 三重调制同时失效，
    //   代码注释里设计的「≥5 条引用加强 15%」「高风险减弱 30%」从未生效。
    //   与 V70 的 f9（恒定 ±0.67）同类缺陷，且后果更重 —— f7 单因子贡献 ±0.06：
    //   本轮样本 Σ(σ·w) = -0.0595，f7 恰为唯一的负项，直接决定 avg_signal 的符号。
    //   DB 实证：近 3 次含 f7 的分析 σ 全部恰好为 -0.6，而 trader 置信度 70/92/85 各不相同。
    //   归一：>1 视为 0~100 口径除以 100；同时兼容 i64 注入。
    let conf_raw = if present(trader_confidence) {
        if type_of(trader_confidence) == "f64" {
            trader_confidence
        } else if type_of(trader_confidence) == "i64" || type_of(trader_confidence) == "int" {
            trader_confidence.to_float()
        } else { 0.5 }
    } else { 0.5 };
    let conf = if conf_raw > 1.0 { conf_raw / 100.0 } else { conf_raw };

    // V65: 证据密度调制 — 引用上游论据 ≥3 条加强信号，<3 条减弱
    // trader_evidence_count 来自 trader.content.evidence_cited（数组长度由 LLM 输出决定）
    let evidence_count = if present(trader_evidence_count) && type_of(trader_evidence_count) == "array" {
        trader_evidence_count.len()
    } else { 0 };
    let evidence_mod = if evidence_count >= 5 {
        1.15  // 引用充分，信号加强 15%
    } else if evidence_count >= 3 {
        1.0   // 引用达标，信号不变
    } else if evidence_count >= 1 {
        0.7   // 引用不足，信号减弱 30%
    } else {
        0.4   // 无引用，信号减弱 60%（接近兜底）
    };

    // V65: 风险等级调制 — 极高风险/高风险减弱信号幅度（保守化）
    let risk_mod = if present(trader_risk_level) && type_of(trader_risk_level) == "string" {
        let rl = trader_risk_level.to_lower();
        if rl.contains("极高") { 0.5 }
        else if rl.contains("高") && !rl.contains("极高") { 0.7 }
        else if rl.contains("中") { 1.0 }
        else if rl.contains("低") { 1.1 }
        else { 1.0 }
    } else { 1.0 };

    // 综合调制因子（clamped 防止过度放大）
    let combined_mod = clamp(evidence_mod * risk_mod, 0.2, 1.3);

    if dir.contains("buy") || dir.contains("bull") || dir.contains("看多") || dir.contains("买入") || dir.contains("long") {
        clamp(conf * 0.6 * combined_mod, 0.05, 0.6)
    } else if dir.contains("sell") || dir.contains("bear") || dir.contains("看空") || dir.contains("卖出") || dir.contains("short") {
        clamp(-conf * 0.6 * combined_mod, -0.6, -0.05)
    } else { 0.0 }
} else if present(trader_target_price) && present(trader_stop_loss) && trader_stop_loss > 0.0 {
    // 旧兼容路径: 仍有 targetPrice/stopLoss 但无 verdict（极少见，仅作兜底）
    if present(current_price) && current_price > 0.0 {
        let upside = (trader_target_price - current_price) / current_price;
        if upside > 0.05 {
            let ratio = trader_target_price / trader_stop_loss;
            let base = clamp(upside * 1.5, 0.05, 0.8);
            if ratio > 1.15 { clamp(base + 0.1, 0.1, 1.0) } else { base }
        } else if upside < -0.05 {
            clamp(upside * 1.3, -1.0, -0.15)
        } else { 0.0 }
    } else {
        let ratio = trader_target_price / trader_stop_loss;
        if ratio > 1.15 { (ratio - 1.0) / 1.0 } else if ratio < 0.85 { -0.5 } else if ratio > 1.0 { 0.1 } else { 0.0 }
    }
} else { 0.0 };
// ── 2026-09-23「退出语义」：σ = 0 的因子必须**退出权重**（与 f6 / f11 / f13 同范式）──
//
// 本因子的 σ = 0 有两种来源，语义都是「**trader 没有给出方向**」：
//   · `dir` 落在「中性」档（生产样本里的常态 —— 见上方 V65 注释引用的实证：
//     「样本 trader_direction 多为「中性」」）；
//   · `trader_direction` 缺失 / 非字符串（trader 节点未产出可用结论）。
// 旧实现无条件占 0.10 权重 ⇒ 沿用「无信息却按中性证据计入分母」的稀释缺陷（见 f6 段）。
//
// ⚠️ 为什么必须下移到此处才定义：σ 由 `dir` 决定，而 `dir` 只能在 `f7_signal` 内部求得。
//   Rhai 顺序求值 ⇒ 前向引用不可用（同 `moat_mult` 下移的先例）。
//
// ⚠️ 已核对三个下游消费点，行为**不变或更正确**：
//   ① `posterior_cap` 的最小权重门（下方 `f7_weight >= TRADER_CAP_MIN_WEIGHT`）：
//      该门语义是「f7 在当前市况下是否可靠」，而封顶只在 `f7_signal < -0.35 / -0.6`
//      时生效 —— 那两个分支里 σ ≠ 0 ⇒ 权重仍是 0.10 ⇒ 门照常通过，**封顶不受影响**；
//      σ = 0 时门不通过，但封顶判据本就不成立（等价于无操作）。
//   ② `total_weight_no_f7` / `weighted_signal_no_f7`（去除 f7 的纯净对照）：
//      σ = 0 时两者退回与主路径**恒等** —— 这正是「trader 未给方向」的正确语义
//      （此时公式结论本就不含 trader 观点，对照才有意义；旧实现反而让一个零信号
//       占着 0.10 权重，使 no_f7 版与主版产生**虚假差异**）。
//   ③ `f7_weight_pct`（落库，供 compute_decision_agreement 的防自指标记）：
//      σ = 0 时为 0，含义是「本次公式未吸收 trader 观点」—— 比旧值的 0.10/total 更准确。
let f7_weight = if f7_signal != 0.0 { 0.10 } else { 0.0 };

// ── P1 新增: 因子 9 — 资金面（主力资金流向）──
// 数据源: t-hotmoney-data → get_stock_money_flow（单日净流入/流出，单位：元）
//         t-scoring.result.content.kline_json（日线 volume×close → 成交额）
// 核心逻辑: 主力净流入占「近 5 日平均成交额」的比例 → [-1, 1] 信号
// 正信号: 主力资金净买入（有明确资金推动）
// 负信号: 主力资金净卖出（资金出逃）
// 零信号: 资金流向平衡或无数据
//
// V70 修复(2026-09-11): 归一化分母错误 —— 旧公式
//     total_est = (|main_in| * 3.0).max(1.0); clamp(main_in / total_est * 2.0)
//   分母由被归一化量自身构成 → 结果在 |main_in| > 0.34 元 后恒为 ±0.67，
//   与成交额、流通市值完全无关，**等价于一个伪二值信号**：
//   当日主力净流入 → +0.67，净流出 → -0.67，幅度信息全部丢失。
//   DB 实证（近 30 天全部 39 次分析）：f9 取值只有 -0.67 / 0 / 0.67 三种，
//   且单日下跌的票恒吃 -0.67，形成系统性负偏置（权重 0.08 → 恒定 -0.054 拖累）。
//   本修复改为与「个股自身流动性」匹配的分母（近 5 日均成交额），
//   跨市值可比、且保留强度信息：
//     f9 = clamp(net / avg_amount, -1, 1) 经有理饱和 r/(|r|+K)
//   K = 0.06：净流入达日均成交额 6% → 信号 0.5；18% → 0.75；理论极限 ±1。
//   若 kline_json 缺失（数据缺口），退回绝对额饱和（K=1 亿元），
//   仍是连续值而非恒定 ±0.67。
let f9_default = 0.08;
let f9_signal = if present(money_flow) && type_of(money_flow) == "string" && money_flow != "" {
    let mf = safe_parse(money_flow);
    if type_of(mf) == "map" {
        // V60 字段名兼容(2026-09-10): get_stock_money_flow 实际输出 camelCase
        // (mainNetInflow/superLargeNet/largeNet/mediumNet)，同时兼容 snake_case。
        // num_of 统一处理 f64 / i64（JSON 整数会解析为 i64，旧代码只判 f64 会漏读）。
        let main_in = if present(mf["mainNetInflow"]) { num_of(mf["mainNetInflow"]) }
            else if present(mf["main_net_inflow"]) { num_of(mf["main_net_inflow"]) }
            else { () };
        let super_large_in = if present(mf["superLargeNet"]) { num_of(mf["superLargeNet"]) }
            else if present(mf["super_large_net"]) { num_of(mf["super_large_net"]) }
            else { () };
        let large_in = if present(mf["largeNet"]) { num_of(mf["largeNet"]) }
            else if present(mf["large_net"]) { num_of(mf["large_net"]) }
            else { () };
        let medium_in = if present(mf["mediumNet"]) { num_of(mf["mediumNet"]) }
            else if present(mf["medium_net"]) { num_of(mf["medium_net"]) }
            else { () };
        // 主力净额优先；缺失时用超大单+大单合成
        let net = if type_of(main_in) == "f64" { main_in }
            else if type_of(super_large_in) == "f64" && type_of(large_in) == "f64" { super_large_in + large_in }
            else { () };
        if type_of(net) == "f64" {
            let avg_amt = pm_avg_turnover(kline_json, 5);
            if type_of(avg_amt) == "f64" && avg_amt > 0.0 {
                // 与个股自身流动性匹配：净额 / 近 5 日均成交额，K=6%
                pm_saturate(net / avg_amt, 0.06)
            } else {
                // 兜底：无 K 线时按绝对额饱和（K=1 亿元），仍是连续值
                pm_saturate(net, 100000000.0)
            }
        } else if type_of(medium_in) == "f64" {
            // 只有中小单数据时反推：中单净流出=主力可能在买入（弱信号，封顶 ±0.4）
            clamp(-pm_saturate(medium_in, 100000000.0) * 0.4, -0.4, 0.4)
        } else { 0.0 }
    } else { 0.0 }
} else { 0.0 };
// V56: 资金流因子消费 IC 代理权重（key="money_flow"）
let f9_weight = if f9_signal != 0.0 { get_weight(f_weights, "money_flow", f9_default) } else { 0.0 };

// ── P1 新增: 因子 10 — 筹码面（股东增减持+解禁+大宗交易）──
// 数据源: t-lockup-data → get_stock_lockup_bundle
// 三信号融合:
//   ① 股东增减持方向（核心信号，权重最高）
//   ② 限售解禁压力（未来 30 天解禁市值占比）
//   ③ 大宗交易折溢价（折价高 = 出货意愿强 = 负信号）
let f10_default = 0.08;
let f10_signal = if present(lockup_bundle) && type_of(lockup_bundle) == "string" && lockup_bundle != "" {
    let lb = safe_parse(lockup_bundle);
    if type_of(lb) == "map" {
        let chip_sig = 0.0;
        // ① 股东增减持信号（权重 0.6）
        let trades = lb["shareholder_trades"];
        if type_of(trades) == "array" {
            let len = trades.len();
            if len > 0 {
                let net_direction = 0.0;
                let total_volume = 0.0;
                for i in 0..len {
                    let t = trades[i];
                    if type_of(t) == "map" {
                        let vol = if type_of(t["shares"]) == "f64" { t["shares"] } else { 0.0 };
                        let typ = if type_of(t["trade_type"]) == "string" { t["trade_type"] } else { "" };
                        if typ == "增持" || typ == "买入" || typ == "increase" {
                            net_direction = net_direction + vol;
                        } else if typ == "减持" || typ == "卖出" || typ == "decrease" {
                            net_direction = net_direction - vol;
                        }
                        total_volume = total_volume + vol;
                    }
                }
                if total_volume > 0.0 {
                    // net_direction/total_volume 范围[-1,1]，表示净方向占比
                    chip_sig = chip_sig + clamp(net_direction / total_volume, -1.0, 1.0) * 0.6;
                }
            }
        }
        // ② 限售解禁压力（权重 0.25）
        let lockup = lb["lockup_schedule"];
        if type_of(lockup) == "array" && lockup.len() > 0 {
            let upcoming_ratio = 0.0;
            for i in 0..lockup.len() {
                let l = lockup[i];
                if type_of(l) == "map" {
                    // unlockRatio 表示解禁股占总股本比例
                    let ratio = if type_of(l["unlockRatio"]) == "f64" { l["unlockRatio"] } else { 0.0 };
                    upcoming_ratio = upcoming_ratio + ratio;
                }
            }
            // 解禁比例>5% 产生显著负信号
            if upcoming_ratio > 5.0 {
                chip_sig = chip_sig + clamp(-upcoming_ratio / 20.0, -0.35, 0.0) * 0.25;
            } else if upcoming_ratio > 1.0 {
                chip_sig = chip_sig + (-0.05) * 0.25;
            }
            // 解禁比例<1% 忽略
        }
        // ③ 大宗交易折溢价（权重 0.15）
        let blocks = lb["block_trades"];
        if type_of(blocks) == "array" && blocks.len() > 0 {
            let avg_discount = 0.0;
            let count = 0;
            for i in 0..blocks.len() {
                let b = blocks[i];
                if type_of(b) == "map" {
                    // discount_pct 正值=折价（低于市价成交），负值=溢价；已由 vendor 计算
                    let disc = if type_of(b["discount_pct"]) == "f64" { b["discount_pct"] } else { 0.0 };
                    avg_discount = avg_discount + disc;
                    count = count + 1;
                }
            }
            if count > 0 {
                avg_discount = avg_discount / count;
                // 折价>5% 显著负信号 (出货意愿强)；溢价>0% 轻微正信号 (机构抢筹)
                if avg_discount > 5.0 {
                    chip_sig = chip_sig + clamp(-avg_discount / 20.0, -0.2, 0.0) * 0.15;
                } else if avg_discount < 0.0 {
                    chip_sig = chip_sig + clamp(-avg_discount / 10.0, 0.0, 0.15) * 0.15;
                }
            }
        }
        // ④ 龙虎榜机构信号（权重 0.15）（P2 新增）
        //   dragon_tiger 数据来自 t-dragon-tiger-data，含机构席位买卖动向
        //   核心逻辑: 机构席位净买入 → 正信号（机构认可价值）
        //             游资席位主导 → 中性（短炒博弈）
        //             无龙虎榜数据 → 无信号（不扣分）
        if present(dragon_tiger) && type_of(dragon_tiger) == "string" && dragon_tiger != "" {
            let dt = safe_parse(dragon_tiger);
            if type_of(dt) == "map" {
                let entries = dt["entries"];
                if type_of(entries) == "array" && entries.len() > 0 {
                    let inst_buy = 0.0;
                    let inst_sell = 0.0;
                    let total_buy = 0.0;
                    let total_sell = 0.0;
                    for i in 0..entries.len() {
                        let e = entries[i];
                        if type_of(e) == "map" {
                            let buy_amt = if type_of(e["buyAmount"]) == "f64" { e["buyAmount"] } else { 0.0 };
                            let sell_amt = if type_of(e["sellAmount"]) == "f64" { e["sellAmount"] } else { 0.0 };
                            let dept = if type_of(e["deptName"]) == "string" { e["deptName"] } else { "" };
                            // 机构专用席位买入/卖出
                            if dept.contains("机构") || dept.contains("机构专用") || dept.contains("special") || dept.contains("institution") {
                                inst_buy = inst_buy + buy_amt;
                                inst_sell = inst_sell + sell_amt;
                            }
                            total_buy = total_buy + buy_amt;
                            total_sell = total_sell + sell_amt;
                        }
                    }
                    let total_vol = (total_buy + total_sell).max(1.0);
                    // 机构净买卖比率: (inst_buy - inst_sell) / total_vol
                    let inst_net = (inst_buy - inst_sell) / total_vol;
                    // 如果机构净买入占比较大 → 正信号；净卖出 → 负信号
                    if inst_net.abs() > 0.01 {
                        chip_sig = chip_sig + clamp(inst_net * 2.0, -0.3, 0.3) * 0.15;
                    }
                }
            }
        }
        clamp(chip_sig / (0.6 + 0.25 + 0.15 + 0.15), -1.0, 1.0)
    } else { 0.0 }
} else { 0.0 };
// V56: 筹码因子消费 IC 代理权重（key="chip"）
let f10_weight = if f10_signal != 0.0 { get_weight(f_weights, "chip", f10_default) } else { 0.0 };

// ── f11: PACE 情绪因子 — 基于公告的四维情绪向量 ──
// pace-calc.rhai 基于 t-catalyst-data 公告数据计算 PACE 信号（Polarity/Actuality/Credibility/Expectation），
// 输出 pace_signal ∈ [-1, 1]，通过 input_mapping "pace-calc.pace_signal" 注入。
// f11 权重 0.08（与 f9/f10 同级补充因子），仅在 pace_signal 非零时激活。
// 修复 D6: 使用 get_weight 消费 IC 代理权重（key="pace"），与 f5/f9/f10 一致
// P2-2 修复: 检查 pace_degraded 标志，降级时 f11 权重归零，避免误用 pace_signal=0 作为中性信号
let f11_default = 0.08;
let pace_is_degraded = present(pace_degraded) && type_of(pace_degraded) == "bool" && pace_degraded;
let f11_signal = if pace_is_degraded {
    0.0  // pace 数据降级，不参与决策
} else if present(pace_signal) && type_of(pace_signal) == "f64" && pace_signal != 0.0 {
    clamp(pace_signal, -1.0, 1.0)
} else { 0.0 };
let f11_weight = if f11_signal != 0.0 { get_weight(f_weights, "pace", f11_default) } else { 0.0 };

// ── f12 动量因子（v19 新增 2026-09-10）──
// 热点股最强的正向证据是价格动量本身，但此前因子集无动量通道（90 天审计 + 601231 涨停板实测）：
//   f1 技术评分 totalScore 0-100 压缩了动量信息（涨停日 totalScore 常仅 55-65 → 信号仅 +0.1），
//   RSI/MACD 仅用于 R-40x 否决（纯负向门），涨幅/量比数据未注入 portfolio-mgr。
// v19 用已注入的 macd_dif/macd_dea/rsi_14 构建趋势状态因子（零数据管线改动）：
//   dif>0 且 dif>dea（零上多头）    → +0.7
//   dif>0 且 dif≤dea（多头回调）    → +0.3
//   dif≤0 且 dif>dea（零下金叉修复） → +0.3
//   dif≤0 且 dif≤dea（零下空头）    → -0.7
//   RSI 调制: 50~75 动量健康 +0.2; >75 过热 -0.2（>80 已被 R-401 否决）
// 数据缺失 → 信号 0（权重归零，不占证据覆盖）。
let f12_default = 0.10;
let f12_signal = 0.0;
if present(macd_dif) && type_of(macd_dif) == "f64" && present(macd_dea) && type_of(macd_dea) == "f64" {
    if macd_dif > 0.0 && macd_dif > macd_dea {
        f12_signal = 0.7;
    } else if macd_dif > 0.0 {
        f12_signal = 0.3;
    } else if macd_dif > macd_dea {
        f12_signal = 0.3;
    } else {
        f12_signal = -0.7;
    }
    if present(rsi_14) && type_of(rsi_14) == "f64" {
        if rsi_14 >= 50.0 && rsi_14 <= 75.0 {
            f12_signal = f12_signal + 0.2;
        } else if rsi_14 > 75.0 {
            f12_signal = f12_signal - 0.2;
        }
    }
    f12_signal = clamp(f12_signal, -1.0, 1.0);
}
let f12_weight = if f12_signal != 0.0 { get_weight(f_weights, "momentum", f12_default) } else { 0.0 };

// ── f13 瓶颈因子（2026-09-19 新增）──
// 背景：本项目核心方法论「瓶颈掘金」的评分**长期不参与确定性决策** ——
//   `serenity_context` 自 V53/X1 起就已由 core.rs 注入本节点的 input_mapping
//   （映射见 `seed_stock_analysis.rs` 的 ("serenity_context", "serenity_context")），
//   且 `hooks.rs` 的注入注释已写明意图是「增加因子：瓶颈置信度」，
//   但本脚本对 `serenity_context` **零引用** ⇒ 数据到了门口没人开门，
//   三力合成分只能停留在 LLM 上下文，导致核心方法论不可复现、不可回测、不可归因。
//   本因子即补齐那条断链（审计报告 `AUDIT-codebase-review-roadmap-2026-09-19.md` A1）。
//
// 映射口径：`serenity_score` ∈ [0,100]（实证：analysis-engine/recommender/
//   strategies/serenity.rs 的 `"瓶颈分析评分: {:.0}/100"`，及 serenity_hit_rate_backtest.rs
//   的分层 [0,40)/[40,60)/[60,80)/[80,100]）
//   ⇒ σ = (serenity_score − 50) / 50，与 f1/f2 的「50 分中位线」口径**同形态**，
//     保持「同一语义同一映射」（铁律：勿为单因子发明第二套尺度）。
//
// 权重政策（v61, 2026-09-20 变更）：**改走 `get_weight(f_weights, "bottleneck", 0.10)`**
//   —— 即与 f1/f2/f5/f9/f10 同一机制，由 `regime-weights.rhai` 的市况调节给出权重；
//   `regime-weights.rhai` 的白名单 `consumed_names` 已同步加入 `"bottleneck"`，
//   其条目缺失时 `get_weight` 回落 `f13_default = 0.10`（= 变更前的取值）。
//
//   ⚠ 变更前的政策是「**硬编码 0.10**，刻意不消费 regime 建议 —— 理由同 f3/f4/f6：
//     瓶颈是**行业结构属性**（供给刚性/需求弹性/不可替代性），市况敏感度已由
//     `serenity_score` 自身的取值变化体现，无需再叠加一层市况权重调节」。
//     该理由被产品裁决**推翻**（2026-09-20 待裁决项②：改走 get_weight 接 regime 白名单）：
//     结构属性 ≠ 市况不敏感 —— 它的**取值**不随市况变，但它的**判别价值**随市况变
//     （牛市里 β 淹没个股结构性差异，压力市况里结构是先于价格存活的维度）。
//     这一区分正是原政策混淆之处：把「"信号源本身不随市况变" 当成 "该因子权重不该随市况变"」。
//
//   regime 调节系数取值与理由（见 `regime-weights.rhai` 的 `factors` 表）：
//     bull 0.9 / neutral 1.0 / bear 1.2 / volatile 1.3
//     方向与 valuation(0.7/1.4/1.2) / chip(0.8/1.3/1.5) 一致（压力市况升权），
//     但幅度刻意更小 —— ① f13 绝对权重仅 0.10，摆幅过大会让单一结构因子主导后验；
//     ② 其信号源已含市况敏感度，叠加过强即重复计数（同 P1-1 协方差衰减要解决的形态）。
//     待真正的 IC 标定：应按「参数四道门」把 base 与四档系数上提为面板可调参数，
//     而不是继续改这两个文件里的常数（同 `dcf_anchor_decay` 的处置约定）。
//
// 激活条件：仅当 `serenity_context` 存在且 `serenity_score` 可解析为数值时激活；
//   其余场景权重归零（**不占证据覆盖**），与 f11/f12 的「数据缺失 → 权重 0」一致。
//   `screening_source != "serenity"` 时上游根本不注入该变量 ⇒ `present()` 守卫
//   保证旧模板 / 非瓶颈路径下安全跳过（rt-workflow V57 会为 present() 内未注入
//   变量补 unit 默认值）。
//   哨兵取 −1.0：`serenity_score` 恒 ≥ 0，故负值可无歧义地表示「缺失/非数值」。
let f13_default = 0.10;
let f13_signal = 0.0;
let f13_weight = 0.0;
if present(serenity_context) && type_of(serenity_context) == "map" {
    let ss = serenity_context["serenity_score"];
    let ss_f = if type_of(ss) == "f64" {
        ss
    } else if type_of(ss) == "i64" || type_of(ss) == "int" {
        ss.to_float()
    } else {
        -1.0
    };
    if ss_f >= 0.0 {
        f13_signal = clamp((ss_f - 50.0) / 50.0, -1.0, 1.0);
        f13_weight = get_weight(f_weights, "bottleneck", f13_default);
    }
}

// ── P1-1: 因子协方差衰减（消除高相关因子的信号重复计数）──
// 贝叶斯因子融合隐含朴素独立假设（Naïve Bayes: P(E1,E2|H) = P(E1|H)×P(E2|H)），
// 但现实中 f3(催化剂)与f11(PACE情绪)都依赖 t-catalyst-data 公告数据，约 70% 信息重叠；
// f1(技术面)与f9(资金面)在趋势行情中高度共振，资金流入往往只是趋势的附属信号。
// 策略：对后激活因子（权重更小的那个）做权重衰减，从 total_weight 和 avg_signal 中消去重复计数。
//
// f3 ↔ f11: 公告数据重叠 → 次要因子(f11)降权 35%
//   例: f3=0.20 + f11=0.08 → f11_effective = 0.08×0.65=0.052, 重叠计数 -0.028
//   根因: PACE 情绪分析从公告文本中提取，催化剂也从公告+LLM 判断，信息源高度重合。
if f3_weight > 0.0 && f11_weight > 0.0 {
    f11_weight = f11_weight * 0.65;  // 减掉 35% 的公告重复计数
}
// f1 ↔ f9: 趋势-资金共振 → 次要因子(f9)降权 25%
//   例: f1=0.15 + f9=0.08 → f9_effective = 0.08×0.75=0.060, 重叠计数 -0.020
//   根因: 主力资金流入是趋势上涨的伴随现象，并非独立证据源。趋势向上时主力必然流入。
// P1-D 补充（2026-09-11）：加最低权重门
//   实证：f9 的最终有效权重被三重削减后仅 0.042（占总量 1.438 的 2.9%）——
//     base 0.08 → bear regime_mod 0.7 = 0.056 → 本处共振衰减 ×0.75 = 0.042，
//   成为全因子最低权重，近乎无效（f9 修复后信号是连续值却推不动 posterior）。
//   当 f9 已因市况调节降到低位时，再削 25% 属于对资金面因子的二次惩罚，
//   共振衰减的本意（消除重复计数）被过度执行。
//   改为仅当有效权重 ≥0.06 时才施加衰减。
//   配套：regime-weights.rhai 的 money_flow base 同步 0.08 → 0.12，
//   重估后 bear 市况 = 0.12×0.7×0.75 = 0.063（较原 0.042 提升 50%），
//   仍显著低于 trend(0.105)，保留「资金面是趋势伴随证据」的保守假设。
if f1_weight > 0.0 && f9_weight >= 0.06 {
    f9_weight = f9_weight * 0.75;  // 减掉 25% 的趋势-资金共振重复
}
// f1 ↔ f12: 技术评分与动量因子同源（行情数据） → 次要因子(f12)降权 25%
if f1_weight > 0.0 && f12_weight > 0.0 {
    f12_weight = f12_weight * 0.75;
}

// ── P1 新增: 技术面硬约束否决（内联计算，从 t-scoring.indicators 注入 rsi_14/macd_dif/macd_dea）──
// 规则：
//   R-401: RSI>80 → 否决（追高风险）
//   R-402: RSI<20 → 否决（恐慌杀跌）
//   R-405: RSI>80 + MACD DIF<0 → 否决（双重风险）
let tech_vetoed = false;
let tech_veto_rule = "";
let tech_veto_reason = "";
let tech_veto_trail = [];

if present(rsi_14) && type_of(rsi_14) == "f64" {
    if rsi_14 > 80.0 {
        tech_vetoed = true;
        tech_veto_rule = "R-401";
        tech_veto_reason = `RSI(14)=${rsi_14}>80 超买区域，触发追高风险否决`;
    } else if rsi_14 < 20.0 {
        tech_vetoed = true;
        tech_veto_rule = "R-402";
        tech_veto_reason = `RSI(14)=${rsi_14}<20 超卖区域，触发恐慌杀跌否决`;
    }
    if rsi_14 > 80.0 && present(macd_dif) && macd_dif < 0.0 {
        tech_vetoed = true;
        tech_veto_rule = "R-405";
        tech_veto_reason = `RSI>80 且 MACD DIF<0，技术面双重风险否决`;
    }
    if tech_vetoed {
        tech_veto_trail.push(#{
            "rule_id": tech_veto_rule,
            "status": "VETOED",
            "detail": tech_veto_reason,
        });
    }
}

// ── 决策追溯链（decision_trail）──
// 记录所有影响决策的关键规则及其裁决结果
// 格式: [{rule_id, status: "PASS"|"VETOED"|"DOWNGRADED", detail, timestamp}]
let decision_trail = [];

// 从技术否决收集 trail
for t in tech_veto_trail {
    decision_trail.push(t);
}
// P0-1 修复: R-200/R-201/R-202/R-204 原在此处前向引用未声明变量（risk_downgraded/
// bearish_downgraded/trader_data_valid/position_pct/base_action），被 catch_unwind
// 吞掉后永不生效。已移至各变量定义之后，参照 C2.1 修复 R-203/R-205 的模式。

// ── 数据 sanity 预检（必须在 position_pct/final_action 之前定义）──
let trader_data_valid = true;
let sanity_reason = "";

// V74 修复(2026-09-12): 先判定 trader 方案方向，再做「价格字段倒置」预检。
//   旧实现无条件按**做多语义**校验 targetPrice > stopLoss，对看空方案必然误判：
//   看空/卖出方案的标准结构就是 targetPrice(下方目标) < currentPrice < stopLoss(上方止损)，
//   因此 targetPrice <= stopLoss 对看空方案是**正常**的。旧检查把这类输出一律打成
//   R-202「trader数据异常」，连带触发仓位封顶与前端「决策冲突」假警报。
//   实证(2026-09-12): targetPrice(1.8) <= stopLoss(2.34) 的看空样本被误判为数据异常，
//   而该样本的 action 本就是「卖出」，字段结构自洽。
//   方向判据：优先 trader_direction（LLM 定性方向），缺失时用 targetPrice vs currentPrice 推断。
//   注意：本豁免只放行「字段倒置」这一项，检查 2（偏离 >70%）/ 检查 3（价格 <1 元）
//   仍会在 trader_data_valid 为 true 时继续执行，垃圾数据防护不受影响。
let trader_dir_bearish = false;
let trader_dir_bullish = false;
if present(trader_direction) && type_of(trader_direction) == "string" {
    let d0 = trader_direction.to_lower();
    if d0.contains("sell") || d0.contains("bear") || d0.contains("看空") || d0.contains("卖出") || d0.contains("short") {
        trader_dir_bearish = true;
    } else if d0.contains("buy") || d0.contains("bull") || d0.contains("看多") || d0.contains("买入") || d0.contains("long") {
        trader_dir_bullish = true;
    }
}
if !trader_dir_bearish && !trader_dir_bullish
    && present(current_price) && present(trader_target_price) && current_price > 0.0 {
    if trader_target_price < current_price {
        trader_dir_bearish = true;
    } else if trader_target_price > current_price {
        trader_dir_bullish = true;
    }
}

// 检查 1: targetPrice <= stopLoss → 价格字段倒置（仅对「非看空方案」成立）
if !trader_dir_bearish
    && present(trader_target_price) && present(trader_stop_loss) && trader_stop_loss > 0.0 {
    if trader_target_price <= trader_stop_loss {
        trader_data_valid = false;
        sanity_reason = `targetPrice(${trader_target_price})<=stopLoss(${trader_stop_loss})`;
    }
}

// V47 新增检查 1.5: targetPrice 显著低于 currentPrice → 空头预测（做多无意义）
// 与检查2不同: 检查2只标记"偏离>70%"为数据异常（可能是LLM幻觉），
// 而此处标记"明确看空"为逻辑不一致——trader预测下跌但系统仍给出持仓建议。
// 阈值15%: 低于此值可能是短期波动，高于此值应触发空头处理。
let trader_bearish = false;
let bearish_magnitude = 0.0;
if trader_data_valid && present(current_price) && present(trader_target_price) && current_price > 0.0 {
    let upside = (trader_target_price - current_price) / current_price;
    if upside < -0.15 {
        trader_bearish = true;
        bearish_magnitude = upside;
    }
}

// 检查 2: 有 current_price 时，targetPrice 偏离超过 70% → 数据异常
if trader_data_valid && present(current_price) && present(trader_target_price) && current_price > 0.0 {
    let deviation = (trader_target_price - current_price) / current_price;
    if deviation.abs() > 0.70 {
        trader_data_valid = false;
        sanity_reason = `targetPrice(${trader_target_price})偏离currentPrice(${current_price})超过70%`;
    }
}

// 检查 3: stopLoss < 1 元或 targetPrice < 1 元 → 垃圾数据
if trader_data_valid {
    if present(trader_target_price) && trader_target_price < 1.0 {
        trader_data_valid = false;
        sanity_reason = `targetPrice(${trader_target_price})<1元`;
    }
    if present(trader_stop_loss) && trader_stop_loss < 1.0 {
        trader_data_valid = false;
        sanity_reason = `stopLoss(${trader_stop_loss})<1元`;
    }
}

// ── 检查 4 (2026-09-13, P0-12): targetPrice ≈ currentPrice → 价格信号零信息量 ──
// 实证（603466 风语筑 2026-09-12）：trader 在「持有/观望」档把 targetPrice 填成
// 与 currentPrice **完全相同**的值，其 decision_trail 原话为
// 「targetPrice=13.27（持有目标不追高）」。这不是数据异常（不触发 R-202），
// 但下方所有价格判据都写成**严格** `>` / `<`：
//   · L1096/1098 方向推断  · L1119 upside  · L1279 odds_from_trader
// 等值时两侧**全不成立** ⇒ 价格信号整体失效，却既不报错也不留痕，
// 归因字段还会宣称「波动率fallback」（听起来像「LLM 没给价格」，实际是「给了但无信息」）。
//
// 处置：只标记为「无效价格信号」并留痕，**不改 `trader_data_valid`** ——
//   后者会触发 R-202 降档 + 仓位封 10%，而「无信息」不等于「数据错误」，
//   把它升级成异常会凭空压掉实盘仓位（与 V59「单点失败只降级不全盘观望」同源）。
// 容差用相对 0.5%：LLM 直抄现价时通常逐位相等，但也可能四舍五入到角。
// 用**短路表达式**一次求值：`&&` 左侧的 `present(...)` 保证右侧才访问该变量
// （Rhai 变量可未定义，独立语句里直接读会抛运行时错误），
// 同时规避 Rhai 的 `let` 不可重赋值（`let mut` 在 v1.25 有已知解析问题）。
let trader_price_no_info = trader_data_valid && present(current_price)
    && present(trader_target_price) && current_price > 0.0
    && ((if trader_target_price >= current_price {
        trader_target_price - current_price
    } else {
        current_price - trader_target_price
    }) / current_price < 0.005);
if trader_price_no_info {
    decision_trail.push(#{
        "rule_id": "R-204",
        "status": "INFO",
        "detail": `trader价格信号无信息量:targetPrice(${trader_target_price})≈currentPrice(${current_price})，方向推断/赔率均不采信`,
        "timestamp": 0.0,
    });
}

// P0-1 修复: R-202 移至 trader_data_valid 定义之后（原位置前向引用为死代码）
// 记录数据异常
if !trader_data_valid {
    decision_trail.push(#{
        "rule_id": "R-202",
        "status": "DOWNGRADED",
        "detail": `trader数据异常:${sanity_reason}`,
        "timestamp": 0.0,
    });
}

// ── 贝叶斯融合 ──
// V51: 因子数 7（f1~f7），P1 扩展至 10 因子（新增 f9 资金面 + f10 筹码面）
// PACE 集成: 新增 f11 情绪因子 → 11 因子, total_weight 包含 f11
// v19(2026-09-10): 新增 f12 动量因子 → 12 因子
// v78(2026-09-19): 新增 f13 瓶颈因子 → 13 因子（核心方法论入决策，见上方 f13 段）
let total_weight = f1_weight + f2_weight + f3_weight + f4_weight + f5_weight + f6_weight + f7_weight + f9_weight + f10_weight + f11_weight + f12_weight + f13_weight;
let weighted_signal = f1_weight*f1_signal + f2_weight*f2_signal
                    + f3_weight*f3_signal + f4_weight*f4_adjusted
                    + f5_weight*f5_signal + f6_weight*f6_signal
                    + f7_weight*f7_signal + f9_weight*f9_signal + f10_weight*f10_signal
                    + f11_weight*f11_signal + f12_weight*f12_signal
                    + f13_weight*f13_signal;
let avg_signal = if total_weight > 0.0 { weighted_signal / total_weight } else { 0.0 };

// 理论最大权重和（活跃因子的默认权重之和）
// 用于 weights_collapsed 比例阈值和 evidence_pct 计算
// V66 修复(2026-07-29): f7 已在 V65 降权至 0.10，此处同步更新，避免
//   total_weight/max_weight 比例偏低约 3.5% 而在边界误触发 weights_collapsed。
//   f1=0.15 + f2=0.25 + f3=0.20 + f4=0.15 + f5=0.15 + f6=0.15 + f7=0.10 + f9=0.08 + f10=0.08 + f11=0.08 = 1.39
//   v19: + f12=0.10 → 1.49
// v78(2026-09-19): f13 采用**条件计入**，而**不是**无条件 +0.10。
//   定义回顾：本量 = 「**活跃**因子的默认权重之和」。f13 只在瓶颈路径激活
//   （`screening_source=="serenity"` ⇒ 上游注入了 serenity_context）；非瓶颈路径下
//   `f13_weight = 0`，f13 就**不是活跃因子**，不该计入分母。
//   ⚠️ 为什么不能写成固定 1.59：本量同时是 `evidence_scale` 的分母、以及
//   `weights_collapsed` / `evidence_pct` / `weightRatio` 三处的判别基准 ——
//   固定 1.59 会让**非瓶颈路径**的 total_weight/max_weight 系统性偏低 6.3%
//   （1.49/1.59），从而在阈值边界误触发 weights_collapsed。
//   这与 V66（「f7 降权后未同步 max_weight ⇒ 比例偏低 3.5% 而误触发」）
//   是**同一形态、幅度更大** ⇒ 必须按激活状态条件计入，
//   保证非瓶颈路径的口径**零回归**（分母仍为 1.49）。
//   v61 补充（f13 改走 get_weight 后仍**不**改本行）：
//   本量是「活跃因子的**默认**（基线）权重之和」，**不是**「市况调节后的权重之和」——
//   f1/f2/f5/f9/f10 走 get_weight 后同样用各自的 default 计入本量。若把 f13 换成
//   `f13_weight`，则熊市/高波动下分母随市况漂移，`evidence_scale` 就不再是稳定的
//   基线比值（同一份证据在不同市况下得到不同的 evidence_pct）。
//   ⇒ 此处继续用 `f13_default`，与其余市况自适应因子**同形态**。
let max_weight = 0.15 + 0.25 + 0.20 + 0.15 + 0.15 + 0.15 + 0.10 + 0.08 + 0.08 + 0.08 + 0.10
    + (if f13_weight > 0.0 { f13_default } else { 0.0 });
// ── P2-1+P3-1: 证据缩放 — 调用 Rust 函数（非线性 sqrt 曲线）──
let evidence_scale = pm_evidence_scale(total_weight, max_weight);
	// V49 修复: trader 否决权 — 技术面（滞后指标）不应覆盖 LLM trader 的实时判断
	// f1(技术评分) 是趋势跟踪指标，必然滞后。当 trader 根据最新信息判断看空时，
	// posterior 不应被技术面推高到"增持/买入"区。
	// 否决规则（仅在 trader 数据有效且 f7 信号强烈时生效）:
	//   f7_signal < -0.5（强烈看空）→ posterior 封顶 0.48（最高"持有"）
	//   f7_signal < -0.2（温和看空）→ posterior 封顶 0.53（最高"增持"，会被风险否决降级）
	// ── P1-C 收敛（2026-09-11）──
	// V65 已把 f7(trade_signal) 权重降到 10% 并明确定义为「定性参考」，
	// 但它仍保留 posterior 的一票否决权（f7 < -0.5 → 封顶 0.48 = 最高「持有」），
	// 与 10% 所表达的「参考级」置信度严重不匹配：一个占总证据约 7% 的因子
	// 能单独把 posterior 压掉 15pt 以上。
	// 收敛三点：
	//   ① 最小权重门（trader_cap_min_weight，默认 0.08）：f7 权重低于该值时，
	//      说明它已被 IC 权重/去重机制判定为当前市况下不可靠，
	//      不应再保留一票否决权（否则「降权」与「否决」自相矛盾）；
	//   ② 触发阈值下移（-0.5→-0.6 / -0.2→-0.35）：只在 LLM trader 强烈看空时生效。
	//      实证：样本 trader_direction 多为「中性」，旧阈值 -0.2 容易在
	//      温和看空区间命中封顶，造成与 f7 权重不匹配的过度影响；
	//   ③ 封顶值上调（0.48→0.50 / 0.53→0.55）：原 0.48 会把整个「持有」区间压掉，
	//      与「定性参考」的定位不符。
	let TRADER_CAP_MIN_WEIGHT = if present(trader_cap_min_weight) { trader_cap_min_weight } else { 0.08 };
	let posterior_cap = if trader_data_valid && f7_weight >= TRADER_CAP_MIN_WEIGHT && present(f7_signal) {
	    if f7_signal < -0.6 { 0.50 }
	    else if f7_signal < -0.35 { 0.55 }
	    else { 1.0 }
	} else { 1.0 };
	let raw_val = prior + avg_signal * evidence_scale;
	let posterior = clamp(if raw_val > posterior_cap { posterior_cap } else { raw_val }, 0.0, 1.0);

	// ── 无 f7 版本 posterior（消除自指悖论）──
	// P0 修复: portfolio-mgr 决策以 0.30 权重吸收 trader 观点后，又与 trader 比较
	// 一致性，导致 compute_decision_agreement 陷入逻辑循环。
	// 此版本排除 f7 影响，用于"纯净"双视角比较（无 f7 公式 vs LLM trader）。
	// 注意: 不加 trader 否决（f7 < -0.5 封顶），因为那是基于 f7 自身的自指约束。
	// P1: 无 f7 版本同时排除 f9/f10（它们是独立信号源，用于 pure 版对比时
	//     不应被排除——但保持向后兼容，保留旧名。实际上 no_f7 仍包含 f9/f10。）
	let total_weight_no_f7 = total_weight - f7_weight;
	let weighted_signal_no_f7 = weighted_signal - f7_weight * f7_signal;
	let avg_signal_no_f7 = if total_weight_no_f7 > 0.0 {
	    weighted_signal_no_f7 / total_weight_no_f7
	} else { 0.0 };
	// P3-1: no_f7 证据缩放 — 调用 Rust 函数
	let evidence_scale_no_f7 = pm_evidence_scale(total_weight_no_f7, max_weight);
	let raw_val_no_f7 = prior + avg_signal_no_f7 * evidence_scale_no_f7;
	// 不加 trader 否决（自指），不加权重坍缩兜底（那是 full 版本的事）
	let posterior_no_f7 = clamp(raw_val_no_f7, 0.0, 1.0);
	let f7_weight_pct = if total_weight > 0.0 {
	    (f7_weight / total_weight * 100.0 * 10.0).round() / 10.0
	} else { 0.0 };

	// ── V42+V44: 风险的多维度处理 ──
	// 风险有两个独立维度, 分别处理:
	//   维度1 (方向): 风险作为看空因子 → f4_signal 在贝叶斯里拉低 posterior (已做)
	//   维度2 (操作): risk_bias 左移 action 阈值 + V43 硬性否决 (见下方)
	//
	// 注意: confidence 不受风险 cap (V44 曾错误 cap, 已回滚)。
	//   权威定义 (Nature Neurosci 2016): confidence = P(决策正确 | 证据), 是后验概率。
	//   posterior 已通过 f4_signal 综合风险因子, confidence = posterior × 100 即可。
	//   "高风险 + 高置信度看多 + action=持有" 是自洽的风险管理, 不是矛盾。
	// P3-1: risk_bias — 调用 Rust 函数
	let risk_bias = pm_risk_bias(overall_risk);
	let effective_posterior = clamp(posterior + risk_bias, 0.0, 1.0);

	// ── 无 f7 版本 effective_posterior 与 action（消除自指）──
	let effective_posterior_no_f7 = clamp(posterior_no_f7 + risk_bias, 0.0, 1.0);
	// P1-5(2026-09-14): 阈值复用上方 ACTION_* 命名常量（支持 input_mapping 覆盖）。
	// 历史实现把 0.63/0.53/0.48/0.38/0.30 又硬编码了一遍：一旦工作流参数覆盖
	// action_*_threshold，base_action 阶梯走新值、f7_free_action 走旧值，
	// 两个口径立刻分叉且无告警（「同一语义两份拷贝」）。
	let f7_free_action = if effective_posterior_no_f7 >= ACTION_BUY_THRESHOLD { "买入" }
	    else if effective_posterior_no_f7 >= ACTION_INCREASE_THRESHOLD { "增持" }
	    else if effective_posterior_no_f7 >= ACTION_HOLD_THRESHOLD { "持有" }
	    else if effective_posterior_no_f7 >= ACTION_WATCH_THRESHOLD { "观望" }
	    else if effective_posterior_no_f7 >= ACTION_REDUCE_THRESHOLD { "减持" }
	    else { "卖出" };

	// ── V47 修复: 凯利赔率使用标准盈亏比 ──
	// 旧: odds = targetPrice / stopLoss（仅看两个绝对价格比值，忽略 currentPrice）
	//   例: currentPrice=20 targetPrice=8 stopLoss=7 → odds=1.14 → 算出正仓位
	//   但 targetPrice < currentPrice 意味着做多无盈利空间，赔率应为 0。
	// 新: odds = (targetPrice - currentPrice) / (currentPrice - stopLoss)
	//   即 潜在收益 / 潜在亏损，仅在 targetPrice > currentPrice 时 > 0。
	//   currentPrice=20 targetPrice=25 stopLoss=18 → odds=(25-20)/(20-18)=2.5 ✓
	//   currentPrice=20 targetPrice=8  stopLoss=7  → targetPrice<currentPrice → odds=0 ✓
	//   赔率上限 10（避免极端值主导 Kelly）
	//
	// ── P0-1: 无 trader 数据时的波动率赔率 fallback ──
	// 原设计: trader_target_price/stop_loss 缺失时 odds=0 → kelly=0 → position_pct=0
	//   后果: LLM trader 单点故障导致整个凯利仓位置零，决策覆盖率严重受限
	// 新: 基于**生效后验**强度估算保守赔率（无 targetPrice 时的经验安全值）:
	//   effPost≥0.70(强看多) → 2.5x (典型 A 股趋势突破后的盈亏比分布中位数)
	//   effPost≥0.60(看多)   → 2.0x (中性偏优, 对应大盘股震荡上行行情)
	//   effPost≥0.50(偏多)   → 1.5x (保守, 适合低波动/底部盘整标的)
	// V59(2026-09-10): 新增试探档 effPost∈[0.42,0.50) → 1.5x
	//   原设计该区间 odds=0 → 凯利与试探仓双双归零 → 全市场 100% 观望 0%
	//   (近 30 天 41 次分析 41 次 0%, DB 实证)。试探仓激活条件见下方 V59 块。
	//   effPost<0.42         → 0.0x (凯利不适用)
	// ⚠️ 本表的档位标签 2026-09-23 由 `posterior` 改为 `effPost`（= effective_posterior）
	//   —— 与下方实际比较的自变量保持一致（原先标签与实现分叉）。
	// 2026-09-23: 概率量统一为 `effective_posterior`（与 kelly_raw / kelly_position /
	//   试探仓窗口**同源**）。理由：本表是「证据强度 → 赔率」的分档，它与凯利是**同一次
	//   下注决策**的两个输入 —— 若一个读 policy 概率、一个读 raw 概率，会出现
	//   「odds=0（证据不足不下注）但 kelly 却算出了正仓位」这类自相矛盾。
	//   注意 `posterior<0.42 → 0.0` 这一档正是「凯利不适用」的触发点（见 :1729 的 V74 注）。
	let odds_fallback = if effective_posterior >= 0.70 { 2.5 }
	    else if effective_posterior >= 0.60 { 2.0 }
	    else if effective_posterior >= 0.50 { 1.5 }
	    else if effective_posterior >= 0.42 { 1.5 }
	    else { 0.0 };
	let odds = if present(trader_target_price) && present(trader_stop_loss) && present(current_price)
	    && trader_stop_loss > 0.0 && current_price > trader_stop_loss
	    && trader_target_price > current_price {
	    let profit = trader_target_price - current_price;
	    let loss = current_price - trader_stop_loss;
	    if loss > 0.0 { clamp(profit / loss, 0.0, 10.0) } else { 0.0 }
	} else { odds_fallback };
	// 记录赔率来源, 用于 decision_trail 追溯
	// V74 修复(2026-09-12): 来源标签必须与实际生效的 odds 分支**同条件**。
	//   旧实现只判「trader 两个价格字段存在且 > 0」就标 trader，而 odds 走的是哪个分支
	//   取决于「trader 是否呈看多结构」（current_price > stop_loss 且 target > current_price）。
	//   于是看空/异常样本会产出「赔率=0.0(trader)」这类自相矛盾的归因 ——
	//   那个 0.0 恰恰来自 odds_fallback 的 posterior<0.42 档，与 trader 无关。
	//   归因字段说谎会同时污染 decision_trail 追溯与用户对赔率来源的判断。
	let odds_from_trader = present(trader_target_price) && present(trader_stop_loss) && present(current_price)
	    && trader_stop_loss > 0.0 && current_price > trader_stop_loss
	    && trader_target_price > current_price;
	let odds_source = if odds_from_trader { "trader" } else { "波动率fallback" };
	// ── 2026-09-23: 仓位链的概率量由 `posterior` 统一为 **`effective_posterior`** ──
	//
	// 病根：同一条决策链上有**两个概率量**在流 —— `action` 阶梯读 `effective_posterior`
	//   （= posterior + risk_bias），而凯利仓位与试探仓读 `posterior`（纯证据）。
	//   后果不是「数字不准」，而是**风险政策对仓位完全失效 + 试探仓机制被绕过**：
	//   · `risk_bias`（极高 -0.15 / 高 -0.08）的设计意图是「高风险标的的证据要求上移」
	//     （见上式 `pm_risk_bias` 与 :255 的「风险维度的市况自适应已通过 risk_bias 与
	//     risk_cap 承担」）。但仓位读原概率 ⇒ risk_bias 对仓位**零影响** ⇒ 该意图实际
	//     只由 `POS_CAP_*` 的硬上限承担，与「高风险 ⇒ 更小下注」的两条腿设计不符。
	//   · 更硬的一处：`probe_position_pct` 的激活窗口是 `posterior ∈ [0.42, 0.50)`
	//     （见下方 V59 段）。V59 的本意是「观望升级为持有」的试探机制，而**观望档正是
	//     由 `effective_posterior` 判出的** ⇒ 用 `posterior` 做窗口时，一个 `effPost=0.434`
	//     （应激活试探仓）的样本会因 `posterior=0.514` 而**跳过激活** ⇒ 机制静默失效。
	//
	// 实证 300642（2026-09-22，DB `portfolio-mgr` 输出）：
	//   `evidence.posterior = 0.514`、`f7_free.effective_posterior = 0.434`、`riskLevel = 高风险`；
	//   输出 `action = 观望` + `positionPct = 9.434%` + `positionState = HOLDING`。
	//   9.434% = 凯利用 `posterior(0.514)` 算出的；而 `effPost(0.434) < 0.50` ⇒ 按凯利
	//   自身的适用条件（下方 `>= 0.50` 才下注）应当为 **0**。同一个输出里两个概率各算一半。
	//
	// 处置：**全链统一到 `effective_posterior`** —— 它是「政策调整后的决策概率」，
	//   门槛比较与下注比例本就该同源。改后 300642：`effPost = 0.434 < 0.50` ⇒ 凯利不适用
	//   ⇒ 仓位 0；同时落入试探窗口 [0.42, 0.50) 且 `!trader_bearish` ⇒ **试探仓 3% 激活**
	//   ⇒ 输出「观望 + 3%（试探）」—— 与 V59 的设计语义一致。
	let q = 1.0 - effective_posterior;
	let kelly_raw = if effective_posterior >= 0.50 && odds > 0.001 {
	    (effective_posterior * odds - q) / odds
	} else { 0.0 };
	let kelly_negative = kelly_raw <= 0.0001;
	// ── P3-1: 凯利仓位 — 调用 Rust 函数 ──
	// 内部处理: 半凯利 + 交易成本扣减 + 风险等级上限
	// 当概率 <0.50 或 odds≤0.001 时返回 0.0（凯利不适用）
	// 2026-09-23: 概率量统一为 `effective_posterior`（理由见上方 kelly_raw 段）
	let base_position_pct = pm_kelly_position(effective_posterior, odds, COST_PCT, overall_risk);
	// V59(2026-09-10): trader 数据异常从「清零」改为「降级」——仓位上限 10%。
	//   原代码 !trader_data_valid → position 0，单点 LLM 价格异常就全盘空仓，
	//   与 V56「单点失败不应全盘观望」的分级降级理念自相矛盾。
	//   odds 已走 fallback 路径（trader 异常时不满足看多条件 → odds_fallback），
	//   凯利值本身已经是无 trader 数据的保守估计，只需加护栏而非归零。
	let position_pct = if !trader_data_valid { clamp(base_position_pct, 0.0, 10.0) } else { base_position_pct };
	// ── P0-11 修复(2026-09-12): 「仓位充足度」与「仓位上限」解耦 ──
	// 旧实现: action 判定（下方 L1409/L1412）直接读 position_pct —— 而它是
	//   pm_kelly_position 的返回值，已在函数内部按风险等级封顶（极高10 / 高35 / 中50）。
	//   后果① 口径错配: pos_buy_min(15) > 极高风险上限(10) ⇒ 「买入」的第二个门
	//     在该风险档下算术上永不成立。这是「仓位上限」越权去否决「方向判断」，
	//     与 L1557「clamp 只压不抬、风控是上限不是强制值」的既有原则相矛盾。
	//   后果② 信息丢失: 「凯利想要 60%（被封到 10%）」与「凯利只想要 9%」在判定里
	//     无法区分 —— 门槛问的是「证据强度是否支撑大仓位」，读的却是被风控压扁后的值。
	// 新实现: 判定改读 kelly_pos_uncapped（半凯利 × 成本扣减，不施加风险封顶），
	//   输出仓位仍用 position_pct（封顶后）。两者职责分离：
	//     门槛回答「方向够不够强」；上限回答「实际能下多少」。
	// 与 Rust 侧 compute_kelly_position 的关系: 该函数 = clamp(半凯利×成本, 0, 95)
	//   之后再 apply_risk_cap；本变量即「apply_risk_cap 之前」的那一步，
	//   因此不会出现两套半凯利公式漂移。
	// 注意: 本修复只解除「算术不可达」，不会也不应人为抬高买入率 ——
	//   若下游证据（avg_signal）仍恒负，action 分布不会发生任何变化。
	let kelly_pos_uncapped = if kelly_raw > 0.0001 {
	    clamp(kelly_raw / 2.0 * 100.0 * (1.0 - COST_PCT), 0.0, 95.0)
	} else { 0.0 };
	// ── 置信度 = P(决策正确 | 证据) = posterior × 100 ──
	// 权威定义 (Nature Neurosci 2016, "Confidence and certainty: distinct probabilistic quantities"):
	//   confidence = P(decision correct | evidence), 是后验概率, 与特定决策绑定。
	//   certainty  = 对变量估计的概率分布宽度 (另一个概念, 这里不用)。
	//
	// 关键: 风险已经通过 f4_signal 进入贝叶斯融合拉低 posterior (方向维度)。
	//   posterior = P(上涨 | 证据) 已综合风险因子, confidence = posterior × 100
	//   在数学和语义上都正确, 不需要再按风险 cap。
	//
	// V44 曾错误地按风险 cap confidence (基于"高风险→低置信"的错误推理),
	//   混淆了 risk (输入证据/收益离散度) 和 confidence (对决策正确的后验概率)。
	//   "高风险 + 高置信度看多 + action=持有" 是自洽的风险管理:
	//   我70%确定会涨, 但风险大所以不加仓。V43 的 action 否决已足够。
	//
	// V50 修复: 接入 risk-convergence 的三方分歧度。
	//   当 risk_disagreement 存在且高时(>50), 三个风险评估师分歧大,
	//   说明风险评估本身不确定性高, 应适度降低最终置信度。
	//   这是对"风险评估的可靠性"的独立修正，与 f4_signal(风险方向)正交。
	//   分歧度 0~100, 修正因子 = (100 - disagreement) / 100, 范围 [0.7, 1.0]
	//   P0 修复(2026-07-25): 原下限 0.3 导致数据质量A级时 confidence 也被砍到19%。
	//     风险已通过 f4_signal 进入贝叶斯 posterior, disagreement_mod 是二次惩罚。
	//     0.3 下限意味着分歧度70时 confidence 打三折 → 63%→19%, 让用户觉得分析不可信。
	//     提高到 0.7: 分歧度70时 confidence 打七折 → 63%→44%, 仍反映不确定性但不过度。
// 2026-09-22 修复：缺失不再默认 1.0（=「无分歧」）。
//   实证 300642 同日两次：11:33 该值 =40 ⇒ 修正 0.70；11:49 缺失 ⇒ 修正 1.00 ——
//   **数据缺失的那次置信度反而更高**（60.9 > 43.3），语义上把「不知道分歧多少」
//   当成了「零分歧」。缺失的真实原因见 `AUDIT-300642-run-variance-2026-09-22.md`：
//   映射路径写死为 `risk-convergence.content.verdict.disagreement_score`（嵌套形态），
//   而该 LLM 节点有时**扁平**输出 `{disagreement_score: ...}` ⇒ 路径解析 None
//   （53 次落库中仅 9 次取到值）。在映射层修好之前，此处按「未知即保守」取下界 0.7。
let disagreement_mod = if present(risk_disagreement) {
    clamp((100.0 - risk_disagreement) / 100.0, 0.7, 1.0)
} else { 0.7 };
	// ── 置信度: 贝叶斯因子驱动 + 数据质量门控（P0 2026-07-25 重写 v3）──
	// 定义：置信度 = posterior 偏离 prior 的证据强度（贝叶斯因子）
	//   BF = posterior_odds / prior_odds (posterior≥prior时)
	//   BF = prior_odds / posterior_odds (posterior<prior时)
	//   BF ≥ 1,越大证据越强。BF=1 → 50%, BF=10 → 90.9%, BF=100+ → 99.5%+
	// 数据质量门控(P0 2026-07-25 新增): 上限与 dqi_grade 挂钩
	//   A → 99.9 / B → 85 / C → 75 / D、F → 60
	//
	// 2026-09-14 修复(阈值统一): 原实现用 dqi_score 数值 + 本文件私有阈值 90/75/60,
	//   与 data-quality.rhai 的 grade 阈值 85/65/45/25 不一致 —— 同一个分数被两套判定
	//   消费, 而 UI 上给用户看的是 grade。最尖锐的后果: grade="C"(45≤score<65) 的股票,
	//   分数落在 60~64 时此处按"D 级"给 60 的置信度上限, 用户看到的是「C 级却按 D 级限额」。
	//   现直接消费 data-quality.rhai 输出的 grade 字符串, 全项目阈值只此一份。
	//
	//   原 else 分支(dqi_score 缺失)返回 99.9 属 fail-open: data-quality 节点本身
	//   continue_on_fail=true 允许失败, 失败反而拿到满额置信度上限, 语义上等于
	//   "数据质量检查不可用 ⇒ 结论更可信"。现按最保守档处理。
	let dqi_grade_str = if present(dqi_grade) && type_of(dqi_grade) == "string" { dqi_grade } else { "" };
	let bayes_confidence = pm_compute_bayes_confidence(prior, effective_posterior);
	let confidence_quality_cap = if dqi_grade_str == "A" {
	    99.9
	} else if dqi_grade_str == "B" {
	    85.0
	} else if dqi_grade_str == "C" {
	    75.0
	} else {
	    60.0
	};
	// decision_confidence(方向确信度)保留供参考,但不是"置信度"
	let decision_confidence_raw = if effective_posterior >= 0.5 {
		effective_posterior
	} else {
		1.0 - effective_posterior
	} * 100.0;
	let signal_strength_raw = (effective_posterior - 0.5).abs() * 200.0;
	let final_confidence = clamp(bayes_confidence * disagreement_mod, 0.0, confidence_quality_cap);

	// ── M2 修复：LLM 失败级联兜底 ──
	// V55 扩展: 上游 AgentNode 走 strict_mode 兜底时（LLM 输出无法解析为合法 JSON），
	//   agent_executor 会注入 __untrusted=true 标记。下游若读到任一上游不可信，
	//   视为严重数据异常：强制 weights_collapsed（观望+空仓+confidence 对半），
	//   避免"50/50 中性兜底"被当成有效信号继续融合。
	// V56 优化 + V54 修复: 分级降级 — 单点失败不应全盘观望。
	//   level 2 全盘降级 (weights_collapsed): (total_weight/max_weight)<0.3 或 untrusted_count>=2
//     → 仅置信度×0.7，不强制空仓+观望（让 posterior 自然决定 action）
//   level 1 部分降级 (partial_degraded): untrusted_count==1
//     → 禁止买入/增持（降级为持有） + confidence×0.7，不强制空仓
//   理由: 6 个上游 Agent 中任一失败就全盘降级过于保守，
//     其余 5 个可信 Agent 的信号仍有价值，不应被单点失败完全否决。
//   P2 修复: weights_collapsed 改为比例阈值 (total_weight/max_weight<0.3)，
	//     避免在 regime_factor_weights 缩放权重后误判为坍缩。
// V57 修复: 数据质量为 F 级时（grade=="F"）视为决策基础不成立,
//   强制触发 weights_collapsed,即使 total_weight/max_weight≥0.3.
// 根因: 10 个分析师全缺失时,分析师层完全无信号,
//   但 f4(算法风险)/f5(估值)/f7(trader) 等"非分析师"因子仍有数据,
//   导致 total_weight 仍可达 0.45+,f6_signal=-1.0 的负信号被稀释,
//   posterior 仍可达到 0.5+,confidence 显示为"中等置信"(60%~70%),
//   与"数据质量F级"语义完全矛盾。
//
// 2026-09-14 修复(阈值统一): 以下三处判定原以 dqi_score 数值分档
//   (F<25 / A≥85 / A|B≥65), 与上方置信度上限的 90/75/60 构成
//   **同一分数两套字母阈值**。现统一改为消费 data-quality.rhai 输出的 grade 字符串 ——
//   二者在数值上等价(grade 由该 score 唯一决定), 但阈值来源收敛为一份,
//   避免日后单独改动某一处造成分档漂移。
// V58 修复(2026-07-28): 因子完整度纳入数据质量评估
//   A/B 级数据表示分析师输出可靠,因子缺失不应触发全盘降级
//   C/D/F 级数据时,因子缺失+分析师不可靠才触发坍缩
// V66 修复(2026-07-29): A 级数据完全豁免 weights_collapsed
//   依据: grade 由 data-quality.rhai 综合作出, 其 score 已包含 factor_completeness
//         维度(2026-09-14 校正: 实际权重 0.30; 原注释误记为 0.20 且引用了失效行号),
//         A 级意味着报告质量+工具可信度+因子完整度三方面均优秀,
//         因子缺失已通过 factor_completeness 体现在 grade 中,无需双重惩罚。
//   注意: 此豁免依赖 score 包含 factor_completeness 维度。若未来公式移除该维度,
//         则需重新评估 A 级豁免的合理性。
let dqi_collapsed = dqi_grade_str == "F";
// V67 修复(2026-07-29): A 级豁免增加 untrusted_count==0 前提条件。
//   原代码达 A 即完全豁免 weights_collapsed,但 grade 可能因
//   report_quality + tool_credibility 满分而达 A 级,此时若存在 untrusted 分析师,
//   豁免会导致不可信信号被当成有效信号继续融合。
//   新条件: grade=="A" 且 untrusted_count==0 才完全豁免。
//   若 grade=="A" 但 untrusted_count>=1,降级为 B 级处理(threshold=0.15)。
let dqi_a_level = dqi_grade_str == "A" && untrusted_count == 0;
let dqi_high_quality = dqi_grade_str == "A" || dqi_grade_str == "B";
// 因子权重坍缩阈值与数据质量联动:
//   A 级(grade=="A" 且 untrusted==0): 完全豁免（threshold=0.0 永不触发，且跳过 untrusted/dqi_collapsed）
//   B 级(grade=="B", 或 grade=="A" 但有 untrusted): 仅当 total_weight/max_weight<0.15 时触发
//   C/D/F 级: total_weight/max_weight<0.3 时触发(因子+分析师双重不可靠)
let weight_collapse_threshold = if dqi_a_level { 0.0 }
    else if dqi_high_quality { 0.15 }
    else { 0.3 };
let weights_collapsed = !dqi_a_level && (
    (total_weight / max_weight) < weight_collapse_threshold
    || untrusted_count >= 2
    || dqi_collapsed
);
	let partial_degraded = untrusted_count == 1;
	// 三级降级策略（C2.1 修复：原二级退化为单级，现恢复三分支）
	//   分支 A: weights_collapsed → 0% 仓位 + confidence × 0.5
	//   分支 B: untrusted_count==1 且 posterior≥0.70 → 仓位上限 30% + confidence × 0.85 + 保留买入/增持
	//   分支 C: untrusted_count==1 且 posterior<0.70 → 禁止加仓 + confidence × 0.70
	let partial_high_confidence = partial_degraded && posterior >= 0.70;
	let partial_low_confidence = partial_degraded && posterior < 0.70;
	if weights_collapsed {
	    // 分支 A: 权重坍缩或≥2 个上游不可信 → 强制空仓(仓位控制已够防御)
	    // 置信度来自假设检验,不受坍缩影响。
	    decision_confidence_raw = clamp(decision_confidence_raw * 0.5, 0.0, 100.0);
	    signal_strength_raw = clamp(signal_strength_raw * 0.5, 0.0, 100.0);
	    position_pct = 0.0;
	} else if partial_high_confidence {
	    // 分支 B: 单点不可信但后验高置信 → 保留买入/增持，仓位上限 30%（在仓位对齐段应用）
	    decision_confidence_raw = clamp(decision_confidence_raw * 0.85, 0.0, 100.0);
	    signal_strength_raw = clamp(signal_strength_raw * 0.85, 0.0, 100.0);
	} else if partial_low_confidence {
	    // 分支 C: 单点不可信且后验低置信 → 禁止加仓 + 方向确信度 × 0.70
	    decision_confidence_raw = clamp(decision_confidence_raw * 0.70, 0.0, 100.0);
	    signal_strength_raw = clamp(signal_strength_raw * 0.70, 0.0, 100.0);
	}
	// C2.1: 三级降级 decision_trail（原 696-713 行前向引用为死代码，移至此处方能生效）
	if weights_collapsed {
	    // V68 修复(2026-07-29): 用 weight_collapse_threshold 替代硬编码 0.3，
	    //   保持与上方 weights_collapsed 判定逻辑一致。
	    //   原问题: B 级数据 threshold=0.15，但此处硬编码 0.3 导致 dqi_collapsed 分支
	    //   条件错位——B 级数据 total_weight/max_weight=0.2 时,
	    //     weights_collapsed 判定为 true（0.2 < 0.15 ❌ 实际 false, 但若 < 0.15 则 true）
	    //     此处 !(0.2 < 0.3) = false → 走 else 分支「因子权重坍缩」措辞
	    //   对 B 级数据应使用 0.15 阈值,否则 collapse_reason 文案与实际触发条件不匹配。
	    let collapse_reason = if dqi_collapsed && !((total_weight / max_weight) < weight_collapse_threshold) && untrusted_count < 2 {
	        `数据质量F级(grade=${dqi_grade_str}), 10个分析师基本全缺失, 0%仓位+置信度×0.5`
	    } else {
	        `因子权重坍缩(${(total_weight*100.0).to_int()}%)或不可信上游≥2,0%仓位+置信度×0.5`
	    };
	    decision_trail.push(#{
	        "rule_id": if dqi_collapsed { "R-206" } else { "R-203" },
	        "status": "DOWNGRADED",
	        "detail": collapse_reason,
	        "timestamp": 0.0,
	    });
	} else if partial_high_confidence {
	    decision_trail.push(#{
	        "rule_id": "R-205",
	        "status": "DOWNGRADED",
	        "detail": `单点上游不可信(untrusted_count=1,posterior≥0.70),仓位≤30%+置信度×0.85`,
	        "timestamp": 0.0,
	    });
	} else if partial_low_confidence {
	    decision_trail.push(#{
	        "rule_id": "R-205",
	        "status": "DOWNGRADED",
	        "detail": `单点上游不可信(untrusted_count=1,posterior<0.70),禁止加仓+置信度×0.70`,
	        "timestamp": 0.0,
	    });
	}

	// ── V59(2026-09-10): 试探仓机制 ──
	// 背景: DB 实证近 30 天 41 次分析 100% 观望 0%。死锁链:
	//   prior(sideways)=0.528 + 信号微负 → posterior 0.40-0.46 恒 <0.50
	//   → 凯利硬门(posterior≥0.50)归零 → odds_fallback 也要求 ≥0.50 → 观望 0%。
	//   pre-takeoff 成长股(高PE/高波动/未突破)的 f4/f5 天然为负, 永远过不了 0.50。
	// 设计: posterior∈[0.42,0.50) 且赔率为正时给 3% 试探上限(与趋势智选 5% 试探仓
	//   语义对齐)——「不确定但值得跟踪验证」而非「一刀切拒绝」。
	// 护栏: 坍缩/单点不可信/空头预测/无赔率 均不激活; 极高风险仍被 R-200 风控否决
	//   (持有→观望→仓位清零), 高风险封顶不受影响。
	// ── V75 修复(2026-09-12): 试探仓的激活与「观望」行动档脱钩 ⇒ 产出「减持 + 3% 仓位」──
	//   实测(09-12 05:33 603353)：posterior=0.428 ⇒ 落入试探窗口 [0.42,0.50) ⇒ 给 3%；
	//   而 action 阶梯读的是 effective_posterior=prior+risk_bias=0.348（高风险 risk_bias=-0.08）
	//   ⇒ 落在「减持区」[0.30,0.38)。两者共用同一 posterior 但**口径不同**，
	//   于是窗口与减持区必然重叠（高风险档下 posterior∈[0.42,0.46) 全部命中）。
	//   结果：输出 action=减持 且 positionPct=3%，与 LLM trader 的「减持/0%」直接打架，
	//   而试探仓的设计语义本是「观望升级为持有」（见下方 V59 块注释）。
	//   修法分两步：① 赋值延后到 base_action 之后，只在「观望」档生效（结构性约束）；
	//   ② 空头判据改用方向语义 —— 原 `!trader_bearish` 是「targetPrice 低于现价 ≥15%」的
	//      **幅度代理**，本样本缺口仅 -4.46% 故未拦截，但 trader 已明确 verdict=看空 / 减持 / 0%。
	//      上一行护栏注释列的是「空头预测不激活」，故用 v35 新增的 trader_dir_bearish 对齐语义。
	// 2026-09-23: 窗口的概率量统一为 `effective_posterior` —— 本通道的**出口**是
	//   「观望 → 试探仓（3%）」，而「观望」档正是用 `effective_posterior` 判出的。
	//   入口读 `posterior`、出口读 `effective_posterior` ⇒ 同一概率量在链上分叉 ⇒
	//   机制静默失效（300642 实证见上方 kelly_raw 段：应激活却跳过）。
	let probe_position_pct = if !weights_collapsed && untrusted_count == 0
	    && !trader_bearish && !trader_dir_bearish
	    && effective_posterior >= 0.42 && effective_posterior < 0.50 && odds > 0.001 { 3.0 } else { 0.0 };
	// 注意: position_pct 的赋值已移至下方 base_action 判定之后（V75），勿在此处提前赋值。

	// ── V42 重构: action 阈值使用 effective_posterior(=posterior+risk_bias) ──
	// 实现是**互斥的 else-if 阶梯**（先命中先返回），不是区间覆盖：
	//   买入 ← ≥ ACTION_BUY_THRESHOLD      且 kelly_pos_uncapped ≥ POS_BUY_MIN
	//   增持 ← ≥ ACTION_INCREASE_THRESHOLD 且 kelly_pos_uncapped ≥ POS_INCREASE_MIN
	//   持有 ← ≥ ACTION_HOLD_THRESHOLD
	//   观望 ← ≥ ACTION_WATCH_THRESHOLD
	//   减持 ← ≥ ACTION_REDUCE_THRESHOLD
	//   卖出 ← 其余（< ACTION_REDUCE_THRESHOLD）
	// ⚠️ P2-1(2026-09-14): 原注释写成「持有 ← 0.48-0.53」「观望 ← 0.38-0.50」
	//   「减持 ← 0.30-0.40」，三处两两重叠（0.48-0.50、0.38-0.40）——
	//   与代码的互斥阶梯不符，照注释读会以为某些 posterior 同时落两档。
	//   重叠带实际归**靠前的档**（0.48-0.50 → 持有；0.38-0.40 → 观望）。
	// ── P0-11(2026-09-12): 仓位门改读 kelly_pos_uncapped ──
	//   原读 position_pct（已按风险等级封顶）⇒ 极高风险档 pos_buy_min(15) > cap(10)，
	//   算术上永不成立；且无法区分「凯利被封顶」与「凯利本来就小」。
	//   现改为读未封顶的凯利意愿仓位 —— 门槛语义 =「证据强度是否支撑大仓位」。
	//   注意: 即便过了此门，「极高/高风险」仍会被下方 pm_risk_veto 按政策降级，
	//   这是两条独立的约束（门槛管方向强度，veto 管风险政策），不是冗余。
	let base_action = "卖出";
	// 买入区（effective_posterior≥0.63 且凯利意愿仓位充足）
	if effective_posterior >= ACTION_BUY_THRESHOLD && kelly_pos_uncapped >= POS_BUY_MIN {
	    base_action = "买入";
	// 增持区（effective_posterior≥0.53 且凯利意愿仓位达标）
	} else if effective_posterior >= ACTION_INCREASE_THRESHOLD && kelly_pos_uncapped >= POS_INCREASE_MIN {
	    base_action = "增持";
	// 持有区（effective_posterior≥0.48，与"观望"的模糊边界）
	} else if effective_posterior >= ACTION_HOLD_THRESHOLD {
	    base_action = "持有";
	// 观望区（posterior 0.38-0.50，与"减持"的模糊边界）
	} else if effective_posterior >= ACTION_WATCH_THRESHOLD {
	    base_action = "观望";
	// 减持区（posterior 0.30-0.40，与"卖出"的模糊边界）
	} else if effective_posterior >= ACTION_REDUCE_THRESHOLD {
	    base_action = "减持";
	}

	// V59: 试探仓激活 → 观望升级为持有（仓位>0 必须与行动对齐，否则 V42 对齐段清零）
	// V75(2026-09-12): 仓位赋值收敛到此块，与「观望」档严格绑定 —— 试探仓是「观望→持有」的
	//   升级机制，若 action 阶梯已判「减持/卖出」（effective_posterior<0.38），说明证据净向负，
	//   此时再给 3% 多头试探仓即自相矛盾。此改动使「减持 + 3%」在结构上不可达。
	if probe_position_pct > 0.0 && base_action == "观望" {
		if position_pct <= 0.0 {
			position_pct = probe_position_pct;
		}
		// V76(2026-09-14): 移除「观望 → 持有」的 action 改写（互改的升级方向）。
		//   原先此处把 base_action 从「观望」改成「持有」，使 action 同时承载
		//   「方向强度」与「持仓状态」两维 ⇒ 落库后无法区分一条「持有」是
		//   posterior 判出的、还是试探仓升级来的。
		//   两轴拆开后：action 保留「观望」（方向判断保真），持仓状态由
		//   positionState=HOLDING（position_pct>0）显式表达，展示层
		//   resolveDisplayAction 据此渲染「持有」——展示效果不变，数据保真。
		decision_trail.push(#{
			"rule_id": "R-207",
			"status": "ACTIVATED",
			"detail": `试探仓激活:effective_posterior=${(effective_posterior*1000.0).to_int()/1000.0}∈[0.42,0.50),odds=${(odds*100.0).to_int()/100.0},仓位=${position_pct}%`,
			"timestamp": 0.0,
		});
	} else if probe_position_pct > 0.0 {
		// V75: 窗口命中但行动档为减持/卖出/买入区 ⇒ 不激活，落 trail 说明原因（可追溯）
		decision_trail.push(#{
			"rule_id": "R-207",
			"status": "SKIPPED",
			"detail": `试探仓窗口命中但未激活:posterior=${(posterior*1000.0).to_int()/1000.0}∈[0.42,0.50),但action档=${base_action}(effective_posterior=${(effective_posterior*1000.0).to_int()/1000.0}),仅「观望」档可升级为持有`,
			"timestamp": 0.0,
		});
	}

	// V76(2026-09-14): 原此处记录「零仓位修正」trail（rule_id=R-204）。
	//   两个问题：① 该 trail 描述的行为正是本次移除的互改；② rule_id 与
	//   L1206 的「价格信号无信息量」真实生产者**撞号**（审计 AUDIT-xingye-601166 已记录）。
	//   互改移除后此 trail 的前提（零仓位 ⇒ action 被改）不复存在，直接删除。

	// ── 最终 action：sanity 不通过则强制降级 ──
	// V54 修复: weights_collapsed 不再强制"观望"，让 posterior 自然决定 action
	// V59 重构: 零仓位对齐检查提前到首位（trader 异常分支同样适用）；
	//   trader 数据异常从「观望/卖出二选一」改为「激进动作封顶为持有」——
	//   原逻辑 posterior<0.45 强制卖出是过度反应（数据异常≠基本面恶化）。
	// V76(2026-09-14): 移除「零仓位 ⇒ 降级为观望」分支（互改的降级方向）。
	//   原分支把 买入/增持/持有 在 position_pct<=0 时统一改写为「观望」，使 action
	//   既表示方向强度、又被仓位反向决定 ⇒ 一条「观望」记录无法区分
	//   「posterior 判为中性」与「看多但目标仓位为 0（凯利/风控所致）」。
	//   两轴拆开后：action 保真，positionState=EMPTY 表达空仓，展示层
	//   resolveDisplayAction(action, EMPTY) 派生「观望」文案。
	let final_action = if !trader_data_valid {
	    // 数据异常时：激进动作封顶为持有，观望/减持/卖出按后验自然分档
	    if base_action == "买入" || base_action == "增持" { "持有" } else { base_action }
	} else if partial_low_confidence && (base_action == "买入" || base_action == "增持") {
	    // 分支 C: 单点不可信+低置信 — 禁止加仓，降级为"持有"（可继续持仓但不追加）
	    "持有"
	} else { base_action };

	// ── P3-1: 风控否决 — 调用 Rust 函数 ──
	// 高风险→禁止买入/增持(降级为持有), 极高风险→禁止买入/增持/持有(降级为观望)
	let original_action = final_action;
	final_action = pm_risk_veto(final_action, overall_risk);
	// 检测是否触发了风控降级，供 decision_trail 和 reasoning 使用
	let risk_downgraded = original_action != final_action;
	let risk_downgrade_note = if risk_downgraded {
	    if overall_risk == "极高风险" || overall_risk == "极高" {
	        ` | ⚠️极高风险风控否决:${original_action}→${final_action}(极高风险不应继续持仓)`
	    } else if overall_risk == "高风险" || overall_risk == "高" {
	        ` | ⚠️高风险风控否决:${original_action}→${final_action}(高风险禁止加仓)`
	    } else { "" }
	} else { "" };

	// P0-1 修复: R-200 移至 risk_downgraded 定义之后（原位置前向引用为死代码）
	// 记录风险否决
	if risk_downgraded {
	    decision_trail.push(#{
	        "rule_id": "R-200",
	        "status": "DOWNGRADED",
	        "detail": risk_downgrade_note,
	        "timestamp": 0.0,
	    });
	}

	// ── V47 新增: 空头 trader 预测否决 ──
	// 当 trader 预测 targetPrice 显著低于 currentPrice（≥15%）时，意味着系统自身
	// 的交易员认为股价将大幅下跌。此时给出"持有"或更积极的建议是逻辑矛盾。
	// 规则: 空头预测 + 持仓建议 → 强制卖出
	// 排除: 已经"卖出"或"减持"的不再降级；"观望"降为"卖出"（明确看空不应观望）
	let bearish_downgraded = false;
	let bearish_downgrade_note = "";
	// ── v78(2026-09-19) A3 补门：否决权的**输入**必须仍然可信 ──
	// 真缺口（审计报告 A3）：`trader_bearish` 在检查 1.5（L1177 区域）判定时
	//   `trader_data_valid` 尚为 true，而**随后**的检查 2 才把「|偏离|>70%」
	//   判为数据异常并置 `trader_data_valid = false` —— 但 `trader_bearish` 已置 true
	//   且**不会回滚**。于是本否决把它当唯一条件 ⇒ 一个被系统**自己判定为异常**
	//   的目标价，仍能把决策强制翻成「卖出」（最强动作），与「异常数据不可信」
	//   自相矛盾；且该翻转对持仓是减法（下方「卖出 ⇒ 仓位=0」）。
	// 修法：联立 `trader_data_valid` —— 异常输入不得行使否决权。
	// ⚠️ 这不是削弱 V47 否决（那是有意设计、已归因 R-200/R-201），
	//    而是给它补上「输入可信」这个前置条件。
	if trader_bearish && trader_data_valid {
	    if final_action == "买入" || final_action == "增持" || final_action == "持有" {
	        bearish_downgraded = true;
	        bearish_downgrade_note = ` | ⚠️空头预测否决:targetPrice(${trader_target_price})<现价(${current_price})${(bearish_magnitude*100.0).to_int()}%,${final_action}→卖出`;
	        final_action = "卖出";
	    } else if final_action == "观望" {
	        bearish_downgraded = true;
	        bearish_downgrade_note = ` | ⚠️空头预测否决:targetPrice(${trader_target_price})<现价(${current_price})${(bearish_magnitude*100.0).to_int()}%,${final_action}→卖出`;
	        final_action = "卖出";
	    }
	    // "减持"/"卖出"保持不动（已经是看空方向）
	} else if trader_bearish && !trader_data_valid {
	    // 留痕：被抑制的否决必须可见，否则「决策为何没翻转」事后无从解释
	    //（R-202 只记了「数据异常」，不记「异常输入本来要行使否决」）
	    decision_trail.push(#{
	        "rule_id": "R-201-SUPPRESSED",
	        "status": "INFO",
	        "detail": `空头预测否决被抑制:targetPrice(${trader_target_price})<现价(${current_price})，但trader数据已被判异常(${sanity_reason})⇒异常输入不得行使否决权`,
	        "timestamp": 0.0,
	    });
	}

	// P0-1 修复: R-201 移至 bearish_downgraded 定义之后（原位置前向引用为死代码）
	// 记录空头预测否决
	if bearish_downgraded {
	    decision_trail.push(#{
	        "rule_id": "R-201",
	        "status": "DOWNGRADED",
	        "detail": bearish_downgrade_note,
	        "timestamp": 0.0,
	    });
	}

	// ── 仓位与 action 对齐（V42）: action 决定了仓位性质 ──
	// 如果 action 是减持/卖出，仓位应趋于零或极低
	if final_action == "卖出" {
	    position_pct = 0.0;
	} else if final_action == "减持" && position_pct > 15.0 {
	    position_pct = 15.0;   // 减持通道，仓位不超15%
	// V76(2026-09-14): 移除「final_action==观望 ⇒ position_pct=0」。
	//   该分支是互改的**反向耦合**（仓位被 action 决定）：移除①后试探仓会
	//   在此被清零（action=观望 + position_pct=3%），功能静默丢失。
	//   两轴拆开后「观望 + 有仓位」是合法组合（= 持有中但不再加仓），
	//   由 positionState=HOLDING 表达。
	}
	// C2.1 分支 B: 单点不可信但高置信 — 仓位上限 30%
	if partial_high_confidence && position_pct > 30.0 {
	    position_pct = 30.0;
	}

	let contradiction_note = if !trader_data_valid {
	    // V74 修复(2026-09-12): base_action == final_action 时不再拼接「已从X修正为X」。
	    //   旧实现无条件拼接，产出「action已从卖出修正为卖出」这类零信息量文案，
	    //   让用户误以为决策被系统改写（2026-09-12 实证截图），实际什么都没改。
	    if base_action != final_action {
	        ` | ⚠️trader数据异常:${sanity_reason},action已从${base_action}修正为${final_action}`
	    } else {
	        ` | ⚠️trader数据异常:${sanity_reason}(action未变,仍为${final_action})`
	    }
	} else if !risk_downgraded && base_action != final_action {
	    ` | ⚠️action修正:${base_action}→${final_action}`
	} else if final_action == "观望" || final_action == "减持" || final_action == "卖出" {
	    ` | ⚠️仓位修正:action=${final_action}仓位调至${position_pct}%`
	} else { "" };

	// C2.1 三级降级标注
	// V69 修复(2026-07-29): 此处 0.3 同步替换为 weight_collapse_threshold，
	//   与 L1102 的 V68 修复保持一致，避免 B 级数据文案与触发条件错位。
let collapse_note = if weights_collapsed {
    if dqi_collapsed && !((total_weight / max_weight) < weight_collapse_threshold) && untrusted_count < 2 {
        ` | ⚠️数据质量F级(grade=${dqi_grade_str}),0%仓位+置信度×0.5`
    } else {
        ` | ⚠️因子权重坍缩(${(total_weight*100.0).to_int()}%)或不可信上游≥2,0%仓位+置信度×0.5`
    }
} else if partial_high_confidence {
    ` | ⚠️单点上游不可信(untrusted_count=1,posterior≥0.70),仓位≤30%+置信度×0.85`
} else if partial_low_confidence {
    ` | ⚠️单点上游不可信(untrusted_count=1,posterior<0.70),禁止加仓+置信度×0.70`
} else { "" };

	// V43 修复: position_pct 用 clamp(只压不抬), 避免 risk_note 把凯利低仓位
	//   (如 21%)强行抬高到风险上限(35%), 违背"高风险限制仓位上限"的语义。
	//   风控是上限不是强制值, 低仓位不应被抬高。
	// V43: 去掉"action阈值左移"描述, 因风险 action 约束已改为硬性否决(见上方 risk_downgrade)
	let risk_note = if overall_risk == "极高风险" {
	    if position_pct > POS_CAP_EXTREME { position_pct = POS_CAP_EXTREME; }
	    ` | ⚠️极高风险:仓位≤${POS_CAP_EXTREME}%`
	} else if overall_risk == "高风险" || overall_risk == "高" {
	    if position_pct > POS_CAP_HIGH { position_pct = POS_CAP_HIGH; }
	    ` | ⚠️高风险:仓位≤${POS_CAP_HIGH}%`
	} else if (overall_risk == "中风险" || overall_risk == "中") && position_pct > POS_CAP_MID {
	    position_pct = POS_CAP_MID;
	    ` | ⚠️中风险:仓位上限${POS_CAP_MID}%`
	} else { "" };

// ── 模拟门（S-501~503）：基于多 Agent 市场仿真的硬性约束 ──
// 在 final_action/position_pct 确定后执行，作为决策输出前的最后一道门。
// 需要上游 market-sim CodeNode 注入 sim_stability/sim_liquidity/sim_impact。
let sim_vetoed = false;
let sim_veto_rule = "";
let sim_veto_reason = "";
// 记录模拟门前的原始决策状态（供前端对比展示）
let pre_sim_action = final_action;
let pre_sim_position_pct = position_pct;

if present(sim_stability) && type_of(sim_stability) == "f64"
   && present(sim_liquidity) && type_of(sim_liquidity) == "f64" {
    // S-501: 价格极度不稳定 → 强制观望
    // P1-E 重估（2026-09-11）：阈值 0.3 → 0.15。
    //   背景：sim_stability 原计算的括号错位（见 hooks.rs）使其恒为 1.0，
    //   本规则在数学上从未可能触发（死门）。修复括号后 sim_stability 恢复为
    //   `1/(1+3×年化波动)`，取值范围：
    //     波动 40% → 0.45 ｜ 80% → 0.29 ｜ 150% → 0.18 ｜ 250% → 0.12
    //   而 A 股个股年化波动普遍 60%~100%（本样本 76.7 / 86.6 / 97.3），
    //   若沿用 0.3 阈值会把绝大多数高波动成长股一刀切为「强制观望」，
    //   与「瓶颈掘金」的选股逻辑直接冲突。
    //   0.15 对应年化波动约 210% —— 只捕捉真正的极端失稳标的。
    if sim_stability < 0.15 {
        sim_vetoed = true;
        sim_veto_rule = "S-501";
        sim_veto_reason = `市场模拟显示价格稳定性不足(${(sim_stability*100.0).round()}%), ${final_action}→观望`;
        final_action = "观望";
        position_pct = 0.0;
    }
    // S-502: 流动性极差 → 仓位上限减半
    if !sim_vetoed && sim_liquidity < 0.2 {
        sim_vetoed = true;
        sim_veto_rule = "S-502";
        let old_pos = position_pct;
        position_pct = clamp(position_pct * 0.5, 0.0, 50.0);
        sim_veto_reason = `市场模拟显示流动性不足(${(sim_liquidity*100.0).round()}%), 仓位从${old_pos}%降至${position_pct}%`;
    }
    // S-503: 大单冲击过大 → 仓位上限减半 + 提示
    // v19(2026-09-10): 阈值 50→150bps。
    // P1-E 重估（2026-09-11）：阈值 150 → 60bps，配套 hooks.rs 的 sim_impact 量级修正。
    //   根因：原 sim_impact 公式 `(annual_vol*100*(1-liq)*50+5).clamp(1,200)` 中
    //   (1-liq) 恒为 0.05（liq 由日成交额计算，≥1.21 亿即顶格 0.95），
    //   叠加常数 50 → 常态波动即触顶 200。实测 12 条运行 sim_impact = 162.9 / 200，
    //   150bps 阈值触发率 92.5%，等价于「只要有仓位就无条件砍半」，退化为死规则。
    //   修正后 sim_impact 恢复区分度（日成交额 5 亿→折价 0.1 / 2 亿→0.6 / 1 亿→0.8 / ≤0.25 亿→0.95）：
    //     波动 35% + 成交 5 亿 → 8.5bps ｜ 波动 90% + 成交 5 亿 → 14bps
    //     波动 60% + 成交 1 亿 → 53bps ｜ 波动 90% + 成交 0.5 亿 → 86bps
    //   标定原则：阈值须落在「高波动但流动性充足」（应放行，约 14bps）
    //   与「高波动 + 流动性不足」（该拦截，约 86bps）之间 → 取 80bps。
    //   这样 S-503 从「只要有仓位就无条件砍半」回归「惩罚真正的流动性风险」的本意。
    if !sim_vetoed && present(sim_impact) && type_of(sim_impact) == "f64" && sim_impact > 80.0 {
        sim_vetoed = true;
        sim_veto_rule = "S-503";
        let old_pos = position_pct;
        position_pct = clamp(position_pct * 0.5, 0.0, 50.0);
        sim_veto_reason = `市场模拟显示大单冲击成本过高(${sim_impact}bps), 仓位从${old_pos}%降至${position_pct}%`;
    }
    if sim_vetoed {
        decision_trail.push(#{
            "rule_id": sim_veto_rule,
            "status": "VETOED",
            "detail": sim_veto_reason,
        });
    }
}

// ── 缺失数据 ──
let data_gaps = [];
if !present(totalScore) { data_gaps.push("技术服务评分(t-scoring)"); }
if !present(consensusScore) { data_gaps.push("辩论共识分数(debate-convergence)"); }
if !present(catalyst_level) || catalyst_level == "" { data_gaps.push("催化剂评估(a-catalyst)"); }
// overall_risk 始终存在（算法分类确保兜底）
if !present(money_flow) { data_gaps.push("资金流向(t-hotmoney-data)"); }
if !present(lockup_bundle) { data_gaps.push("筹码面数据(t-lockup-data)"); }
if !present(dragon_tiger) { data_gaps.push("龙虎榜数据(t-dragon-tiger-data)"); }

// ── 2026-09-21：存在性之外的**内容级**缺口判据 ──
// 原判据只测 `present()`（节点/字段是否存在）⇒ 「工具返回了、但内容缺失」测不出来。
// 实证（AUDIT-analyst-low-confidence-2026-09-21.md §六）：本轮 300308 的 LLM 识别出
// 5 条真实缺口，而此处 data_gaps = []（空）—— 因为 t-dragon-tiger-data 确实返回了 JSON、
// t-fundamentals-data 也确实返回了 peers，只是龙虎榜席位恒为「买入0席位/卖出0席位」、
// 同侪 PE/PB 全为 null。**原文写「缺口信息在此丢失」不准确，实为口径只测存在性**。
// 先例：`risk_pe` 的判据已按同一口径补内容级检查（下方 2026-09-21 订正处：
//   `!present` 与 `== 0.0` 才算缺口，负值＝亏损属已知事实）。
// 该信号只登记缺口、不参与评分（评分口径见文件末的决策注释），与 trader_price_no_info 一致。
let dt_seat_blank = false;
if present(dragon_tiger) && type_of(dragon_tiger) == "string" && dragon_tiger != "" {
    let dt_v = safe_parse(dragon_tiger);
    if type_of(dt_v) == "map" {
        let dt_entries = dt_v["entries"];
        if type_of(dt_entries) == "array" && dt_entries.len() > 0 {
            let blank_n = 0;
            for i in 0..dt_entries.len() {
                let e = dt_entries[i];
                if type_of(e) == "map" {
                    let dept = if type_of(e["deptName"]) == "string" { e["deptName"] } else { "" };
                    // 失效形态：席位名为空，或旧实现的「买入N席位/卖出N席位」占位
                    // （真实营业部名与「机构专用」均不含「席位」二字）
                    if dept == "" || (dept.contains("席位") && !dept.contains("营业部")) {
                        blank_n += 1;
                    }
                }
            }
            if blank_n == dt_entries.len() { dt_seat_blank = true; }
        }
    }
}
if dt_seat_blank {
    data_gaps.push("龙虎榜席位明细无营业部名(仅汇总级数据)(t-dragon-tiger-data)");
}
if !present(announcements) { data_gaps.push("公告数据(t-catalyst-data)"); }
// 2026-09-21 订正：**亏损 ≠ 数据缺失**。
// 原判据 `!present(risk_pe) || risk_pe <= 0.0` 把两者合流，而上游 vendor 曾把
// 负 PE 一律抹成 None ⇒ 亏损企业被登记成「PE数据(t-risk) 缺失」。688114 实证：
//   库里 `pe_ttm = −144.08`（eastmoney 估值历史有值）、决策链却报缺口，
//   并连带把一致性评分的 data_gaps 维度打成 0 分、UI 渲染出
//   「本次「观望」为数据不足导致的被动降级，非看空判断」——方向被说反。
// 现口径三态：`!present` = 真缺失；`== 0.0` = vendor 缺字段占位（无经济含义）；
//   `< 0.0` = 亏损企业（已知事实，由上方 f4 段记 note，**不进缺口清单**）。
if !present(risk_pe) || risk_pe == 0.0 { data_gaps.push("PE数据(t-risk)"); }
if !present(pace_signal) { data_gaps.push("PACE情绪因子(pace-calc)"); }
// 2026-09-13 P0-12: trader 给了 targetPrice 但等于现价 ⇒ 有字段、零信息量。
// 归入「缺口」而非「异常」：下游的赔率/方向推断实质拿不到有效价格信号，
// 与「字段缺失」在决策后果上等价（都走 odds_fallback），只是成因不同。
if trader_price_no_info { data_gaps.push("trader目标价(等于现价、无方向信息)"); }
// evidence_pct = total_weight / max_weight * 100（max_weight 已在贝叶斯融合段计算）
let evidence_pct_raw = (total_weight / max_weight * 100.0).to_int();
let evidence_pct = if evidence_pct_raw > 100 { 100 } else { evidence_pct_raw };

// ── 时间 ──
let time_horizon = if present(trader_time_horizon) && present(trader_holding_days)
    && trader_holding_days > 0.0 { trader_time_horizon } else if posterior >= 0.65 { "long" } else if posterior >= 0.50 { "mid" } else if posterior >= 0.35 { "short" } else { "ultra_short" };
let expected_holding_days = if present(trader_holding_days) && trader_holding_days > 0.0 {
    trader_holding_days.to_int()
} else { switch time_horizon { "ultra_short"=>2, "short"=>5, "mid"=>28, "long"=>90, _=>5 } };
let target_timeframe = switch time_horizon {
    "ultra_short"=>"1d-2d", "short"=>"1w", "mid"=>"1m", "long"=>"3m", _=>"1m"
};

// ── 推理 ──
let prior_src = if present(market_regime_state) { `(${market_regime_state})` } else { "" };
let gap_note = if data_gaps.len() > 0 {
    ` | 缺口(${data_gaps.len()}) 证据=${evidence_pct}%`
} else { "" };
// v40(2026-09-13) 修复「posterior 一物三义」：
//   原文案只写 `后验=<posterior×100>`，而 `posterior` 是**未叠加 risk_bias 的原始
//   后验概率**（0.51），同一次决策的 `decision_json.posterior` 字段却是**生效后验**
//   （含 risk_bias −0.15 ⇒ 35.7）。同一份输出里两个数量共用一个名字，实证差 15pt，
//   前端与决策解释官都会误读（文案说"后验 0.51"、字段说 35.7）。
//   现同时标注两个量并明确口径。
//   ⚠️ 2026-09-13 二次修复：除数笔误 `*100/10`（0.54 → 5.4，与相邻的 `后验` 不同量纲）。
//   两个量必须同刻度：`*100` 取整再 `/100` ⇒ 0.54 / 0.36 这类小数，与 `后验` 可直接对比。
let bayes_note = ` | 先验=${prior}${prior_src} 后验=${(posterior*100.0).round()/100.0} 生效后验=${(effective_posterior*100.0).round()/100.0}`;
let kelly_note = ` | 赔率=${(odds*10.0).round()/10.0}(${odds_source}) 凯利=${(kelly_raw*1000.0).round()/1000.0} 成本=${(COST_PCT*100.0).to_int()}%` + if trader_price_no_info { " | ⚠️trader目标价=现价(价格信号无信息量)" } else { "" };
// V50: 分歧度修正标注（仅在 disagreement 显著时标记）
let disagreement_note = if present(risk_disagreement) && risk_disagreement > 50 {
    ` | ⚠️风险分歧(${risk_disagreement})置信修正×${(disagreement_mod*100.0).round().to_int()}%`
} else { "" };

// ── V77(2026-09-14): DCF 不适用的**显式告知** ──
// 601166 实证（2026-09-14）：估值面板写「内在价值 40.17–49.26 元（低估 143%）」，
//   决策卡写「观望 + 0% 仓位」—— 两条结论并列出现且**没有任何一句话解释为什么**，
//   用户直接质问「这是不是天方夜谭」。根因之一是剔除 DCF 后**没有告知**剔除动作。
// 原则（用户裁决原话）：「失败时下游不该继续，但应该告知错误」—— 静默降级 = 说谎。
// 因此只要 DCF 有值却被剔除，reasoning 必须带上原因，让「估值说低估、决策说不买」
//   这两个数字之间有可读的因果解释。
//
// 写法说明：原因后缀先单独拼好，再与主句用 `+` 连接 —— **刻意不在反引号模板里内嵌
//   `if` 表达式**（`${if ... { \`...\` } else { "" }}`）。嵌套反引号字符串里的 `${`
//   会让 Rhai 的插值匹配提前闭合，属本项目未验证过的语法形态；`+` 拼接零风险。
let valuation_reason_suffix = if present(valuation_dcf_inapplicable_reason)
    && valuation_dcf_inapplicable_reason != "" {
    "：" + valuation_dcf_inapplicable_reason
} else { "" };
let valuation_note = if !dcf_leg_usable && present(valuation_dcf_upside) {
    " | ⚠️估值(DCF)不适用已剔除" + valuation_reason_suffix
} else { "" };

// ── P1-2(2026-09-14): 持仓状态提升为**独立轴**显式输出 ──
// V76(2026-09-14): 「持有 ⇄ 观望」的**双向互改已移除** —— action 不再由仓位改写：
//   ① 试探仓块不再把「观望」改成「持有」（仅赋值 position_pct）
//   ② 「零仓位 ⇒ 观望」分支已删  ③ 「观望 ⇒ 清仓」分支已删
// 「持有 vs 观望」此前不是两个语义，而是同一中性档因仓位有无被互改的两个名字。
// 现把持仓状态显式输出为 positionState，使两轴正交可表达：
//   action        = 方向强度（买入/增持/持有/观望/减持/卖出）
//   positionState = 持仓状态（EMPTY/OPENING/HOLDING/TRIMMING）
// ⚠️ 两轴可同时给出「有方向 + 空仓」（如 买入 + EMPTY），这是**合法**组合：
//    表示「方向看多但目标仓位为 0（凯利/风控所致）」，展示层据此渲染。
let position_state = if position_pct <= 0.0 {
    "EMPTY"
} else if final_action == "买入" || final_action == "增持" {
    "OPENING"
} else if final_action == "减持" || final_action == "卖出" {
    "TRIMMING"
} else {
    "HOLDING"
};

// ── 2026-09-22: 「展示档」派生**已移除** —— 结论名 = 方向档 `final_action` ──
// 根因（实证 300642 同日两次，见 `AUDIT-300642-run-variance-2026-09-22.md`）：
//   2026-09-21 引入的派生判据是「中性档 + position_state==EMPTY ⇒ 观望；否则 ⇒ 持有」。
//   而 `position_state` 读的是**本次决策刚算出的建议仓位** `position_pct`，于是：
//     LLM trader 一个措辞变化（verdict=看空 → 中性）
//       ⇒ 试探仓守卫 `!trader_dir_bearish` 放行 ⇒ position_pct 0% → 3%
//       ⇒ position_state EMPTY → HOLDING ⇒ 展示档「观望」→「持有」。
//   三次跳转之后，**方向档 `final_action` 两次都是「观望」**（effective_posterior
//   0.382 / 0.391 同落 [0.38,0.48)），用户却看到两个不同结论。
//   ⇒ 用本次决策的输出反推本次决策的展示名是**循环判据**；且把 LLM 的非确定性
//     经「试探仓 → 仓位 → 展示档」三级旁路注入了结论名，破坏「同输入同结论」。
// 现语义（唯一真相源 = 方向档）：
//   action        = 方向强度，由 effective_posterior 的互斥阶梯判出（确定性）——结论名取它；
//   positionState = **本次建议后的**持仓状态（EMPTY/OPENING/HOLDING/TRIMMING），
//                   只用于展示「建议建多少仓」，**不得**再用于反推方向档名。
// ⚠️ 前端 `src/lib/stock-analysis-utils.ts::resolveDisplayAction` 已同步改为恒等
//   返回方向档（签名保留以兼容调用点）。改一侧必须同步另一侧。
// ⚠️ 用户**真实持仓**不在本判据内（position_state 描述的是建议，不是账本）——
//   若将来要按真实持仓区分「持有/观望」文案，须另接 `portfolio_holdings`，
//   不要复用本字段（复用即回到循环判据）。

// ── 交易档位百分比：**单一来源**（2026-09-23 提取为变量）──
// 提取动机：原实现把同两张 `switch time_horizon` 内联在 `pm_result` 的 map 里；
//   新增「绝对价格」换算需要**第三份**同一张档位表 ⇒ 会把档位表在文件内手抄成三份
//   （档位数值一旦改动，必然出现某一处漏改 ⇒ 同一 JSON 里 `stopLossPct` 与
//   `stopLoss` 反解不一致，属手抄多副本缺陷族）。故在此收敛为唯一定义。
// 语义：`position_pct <= 0` ⇒ 两档位均为 0.0（**无交易计划**，而非「0% 止损」）——
//   下游以 `sl_pct > 0.0` 作为「本交易计划是否存在」的判据（见 targetPrice/stopLoss）。
// 2026-09-23 阶段1（PROPOSAL-stock-decision-four-horizon.md）改造：
//   档位值收敛为三张逐一对应的「周期 → 值」转换函数（sl_pct_for / tp_pct_for /
//   days_for），**主档位** `sl_pct`/`tp_pct` 由 `time_horizon` 经函数取，**四周期
//   价位映射** `horizonPriceMap` 亦由同一批函数逐周期生成 ⇒ 档位数值在仓库里出现
//   且只出现一遍（此前已注释声明「档位表禁止在文件内出现三份」，本改进一步收敛）。
let sl_pct_for = |h| switch h { "ultra_short" => 3.0, "short" => 5.0, "mid" => 8.0, "long" => 12.0, _ => 5.0 };
let tp_pct_for = |h| switch h { "ultra_short" => 5.0, "short" => 10.0, "mid" => 18.0, "long" => 30.0, _ => 10.0 };
let days_for = |h| switch h { "ultra_short" => 2, "short" => 5, "mid" => 28, "long" => 90, _ => 5 };
let sl_pct = if position_pct > 0.0 { sl_pct_for(time_horizon) } else { 0.0 };
let tp_pct = if position_pct > 0.0 { tp_pct_for(time_horizon) } else { 0.0 };

// ── 四周期价位映射（阶段1 新增）──
// PROPOSAL 阶段1：不改变 action/仓位语义，仅把目标价/止损价扩展为四周期映射，
//   为阶段2「四周期独立决策」打数据与契约垫脚石。主档位 targetPrice/stopLoss
//   仍由 `time_horizon` 决定并保留（兼容既有消费方与列表摘要）；本映射按四周期
//   各出一组「档位% + 持有天数 + 绝对价」。
// 语义与主档位一致：空仓档（`position_pct <= 0`）整组输出 ()（JSON null）——
//   下游以 `stopLossPct > 0.0` 判断该周期是否有交易计划，不产出半截价位。
// 绝对价口径与主档位相同：`现价 × (1 ∓ 档位%)` —— 风控档位换算，非价格预测。
let price_ok = present(current_price) && current_price > 0.0;
let horizon_price_of = |h, sl, tp, days| if price_ok {
    #{
        "stopLossPct": sl, "takeProfitPct": tp,
        "expectedHoldingDays": days,
        "targetPrice": (current_price * (1.0 + tp / 100.0) * 100.0).round() / 100.0,
        "stopLoss": (current_price * (1.0 - sl / 100.0) * 100.0).round() / 100.0,
    }
} else {
    #{
        "stopLossPct": 0.0, "takeProfitPct": 0.0,
        "expectedHoldingDays": days,
        "targetPrice": (), "stopLoss": (),
    }
};
let horizon_price_map = #{
    "ultra_short": horizon_price_of("ultra_short", sl_pct_for("ultra_short"), tp_pct_for("ultra_short"), days_for("ultra_short")),
    "short":       horizon_price_of("short",       sl_pct_for("short"),       tp_pct_for("short"),       days_for("short")),
    "mid":         horizon_price_of("mid",         sl_pct_for("mid"),         tp_pct_for("mid"),         days_for("mid")),
    "long":        horizon_price_of("long",        sl_pct_for("long"),        tp_pct_for("long"),        days_for("long")),
};

// ── 阶段2（PROPOSAL-stock-decision-four-horizon.md）：四周期独立决策 ──
// 语义：主决策（pm_result）保持日线口径**不动**；本节为 short/mid/long 各出一档「用该周期
//   技术评分重新融合的独立决策」，ultra_short 走方案B（单后验 + 档位映射 + 置信下限）。
// 口径：非技术因子（f2~f13，基本面/情绪/资金/风险）在四周期共用主链同一组信号 —— 它们
//   本身依附日线证据，逐周期重算无落地来源；仅技术面（f1，滞后趋势）按周期**独立评分**后
//   把 f1 权重与 f1 信号替换、再融回原贝叶斯式 → 得到该周期自己的 posterior/effective。
// 数据不足（ts 未注入）时该周期退化用主链 f1_signal，verdict 附「该周期评分缺失」注。
let non_tech_total_weight = total_weight - f1_weight;
let non_tech_weighted_signal = weighted_signal - f1_weight * f1_signal;

// 单周期决策闭包。参数 ts = 该周期 totalScore（未注入时 input_mapping 已补 unit）。
// 用闭包而非 `fn`：rhai 的 `fn` 读不到本文件顶层的 `let` 常量，闭包可捕获外层作用域。
let horizon_decision = |h, ts, slh, tph, daysh| {
    let f1s = if present(ts) { (ts - 50.0) / 50.0 } else { f1_signal };
    let tw = f1_weight + non_tech_total_weight;
    let ws = f1_weight * f1s + non_tech_weighted_signal;
    let avg = if tw > 0.0 { ws / tw } else { 0.0 };
    let hpost = clamp(prior + avg * evidence_scale, 0.0, 1.0);
    let heff = clamp(hpost + risk_bias, 0.0, 1.0);
    // 纯阈值阶梯（与主链 action 同源常量；省略 kelly 门以降复杂度，见阶段2说明）
    let haction = if heff >= ACTION_BUY_THRESHOLD { "买入" }
        else if heff >= ACTION_INCREASE_THRESHOLD { "增持" }
        else if heff >= ACTION_HOLD_THRESHOLD { "持有" }
        else if heff >= ACTION_WATCH_THRESHOLD { "观望" }
        else if heff >= ACTION_REDUCE_THRESHOLD { "减持" }
        else { "卖出" };
    // 同主链 odds_fallback 表（证据强度 → 保守赔率），保证「下注概率」与「赔率」同源
    let hodds = if heff >= 0.70 { 2.5 } else if heff >= 0.60 { 2.0 }
        else if heff >= 0.50 { 1.5 } else if heff >= 0.42 { 1.5 } else { 0.0 };
    // 仓位复用 pm_kelly_position（半凯利 + 成本 + 风险上限），与主链同一函数
    let hpos = if heff >= 0.50 && hodds > 0.001 { pm_kelly_position(heff, hodds, COST_PCT, overall_risk) }
        else { 0.0 };
    let hgap = if present(ts) { "" } else { "[该周期评分缺失，按日线退化]" };
    let hverdict = `决策=${haction} 置信=${(heff*100.0*10.0).round()/10.0}${hgap}`;
    let htarget = if hpos > 0.0 && price_ok { (current_price*(1.0+tph/100.0)*100.0).round()/100.0 } else { () };
    let hstop = if hpos > 0.0 && price_ok { (current_price*(1.0-slh/100.0)*100.0).round()/100.0 } else { () };
    #{
        "action": haction,
        "verdict": hverdict,
        "positionPct": (hpos*10.0).round()/10.0,
        "confidence": ((heff*100.0)*10.0).round()/10.0,
        "posterior": ((hpost*100.0)*10.0).round()/10.0,
        "stopLossPct": if hpos>0.0 { slh } else { 0.0 },
        "takeProfitPct": if hpos>0.0 { tph } else { 0.0 },
        "expectedHoldingDays": daysh,
        "targetPrice": htarget,
        "stopLoss": hstop,
    }
};

// 超短线（方案B）：单后验 = 主链 effective_posterior（日线已是最短摄入周期，无更短评分源），
//   档位固定 3/5/2；因单后验信息不足，输出置信下限 0.35 + 文案，供前端提示「超短线为
//   低确信档」，不以独立评分强算虚假置信。
let us_action = if effective_posterior >= ACTION_BUY_THRESHOLD { "买入" }
    else if effective_posterior >= ACTION_INCREASE_THRESHOLD { "增持" }
    else if effective_posterior >= ACTION_HOLD_THRESHOLD { "持有" }
    else if effective_posterior >= ACTION_WATCH_THRESHOLD { "观望" }
    else if effective_posterior >= ACTION_REDUCE_THRESHOLD { "减持" }
    else { "卖出" };
let us_odds = if effective_posterior >= 0.70 { 2.5 } else if effective_posterior >= 0.60 { 2.0 }
    else if effective_posterior >= 0.50 { 1.5 } else if effective_posterior >= 0.42 { 1.5 } else { 0.0 };
let us_pos = if effective_posterior >= 0.50 && us_odds > 0.001 { pm_kelly_position(effective_posterior, us_odds, COST_PCT, overall_risk) }
    else { 0.0 };
let ultra_short_dec = #{
    "action": us_action,
    "verdict": `决策=${us_action} 置信=${(effective_posterior*100.0*10.0).round()/10.0}[超短线单后验，置信下限35%]`,
    "positionPct": (us_pos*10.0).round()/10.0,
    "confidence": ((effective_posterior*100.0)*10.0).round()/10.0,
    "posterior": ((effective_posterior*100.0)*10.0).round()/10.0,
    "stopLossPct": if us_pos>0.0 { 3.0 } else { 0.0 },
    "takeProfitPct": if us_pos>0.0 { 5.0 } else { 0.0 },
    "expectedHoldingDays": 2,
    "targetPrice": if us_pos>0.0 && price_ok { (current_price*(1.0+5.0/100.0)*100.0).round()/100.0 } else { () },
    "stopLoss": if us_pos>0.0 && price_ok { (current_price*(1.0-3.0/100.0)*100.0).round()/100.0 } else { () },
    "confLowerBound": 0.35,
};

// 四周期独立决策汇总（camelCase 键，供前端按周期 Tab 分组展示）
// 短/中/长分别注入对应周期评分（seed 新增 t-scoring-week / t-scoring-month 节点供给）
let decisions_by_horizon = #{
    "ultraShort": ultra_short_dec,
    "short": horizon_decision("short", totalScore_short, 5.0, 10.0, 5),
    "mid": horizon_decision("mid", totalScore_mid, 8.0, 18.0, 28),
    "long": horizon_decision("long", totalScore_long, 12.0, 30.0, 90),
};

// ── 输出 ──
	pm_result = #{
    "action": final_action,
    "positionState": position_state,
    "positionPct": (position_pct * 10.0).round() / 10.0,
    "positionPctNormalized": (position_pct * 10.0).round() / 10.0,  // P0-2: 组合归一化前的原始仓位权重
    "maxPositionPct": if overall_risk == "极高风险" { POS_CAP_EXTREME }
        else if overall_risk == "高风险" { POS_CAP_HIGH }
        else if overall_risk == "中风险" { POS_CAP_MID }
        else { 95.0 },  // P0-2: 本标的在当前风险等级下的单标的上限
    "portfolioExposureWarning": if position_pct > 33.3 {
        `⚠️仓位${position_pct}%超过33%——组合层面若持有3个以上同仓位标的，总敞口将超100%`
    } else { "" },  // P0-2: 组合敞口预警（33%=3标的中性仓位阈值）
    "confidence": (final_confidence * 10.0).round() / 10.0,
    // v40(2026-09-13) 口径澄清：本字段 = **生效后验** × 100（= posterior + risk_bias）。
    //   此前注释写成「原始上涨概率 posterior × 100」，与实现不符 —— 实证 601166：
    //   本字段 35.7，而未含风险偏置的原始后验为 50.7（见 evidence.posterior /
    //   posteriorRaw）。字段名保留以兼容既有消费方，语义以本注释为准。
    "posterior": ((effective_posterior * 100.0) * 10.0).round() / 10.0,
    // v40(2026-09-13) 新增：未叠加 risk_bias 的原始后验概率 × 100，供归因对照
    //   （「是因子判断错了，还是风险档惩罚压下去的」）。
    "posteriorRaw": ((posterior * 100.0) * 10.0).round() / 10.0,
    // V58: 决策方向置信度 — 无论买卖方向都体现"多确信"（解决看空决策 confidence 偏低被误读为"不确信"）
    "decisionConfidence": (decision_confidence_raw * 10.0).round() / 10.0,
    // V58: 信号强度 — 偏离中性的程度，0=完全中性，100=极端强信号
    "signalStrength": (signal_strength_raw * 10.0).round() / 10.0,
    // V66 修复(2026-07-29): weights_collapsed 状态结构化输出，供前端 Tag+Tooltip 可见化
    "weightsCollapsed": weights_collapsed,
    "collapseReason": if !weights_collapsed { "none" }
        else if dqi_collapsed && !((total_weight / max_weight) < weight_collapse_threshold) && untrusted_count < 2 {
            "dqi_collapsed"
        }
        else if untrusted_count >= 2 { "multi_untrusted" }
        else { "low_weight_ratio" },
    "weightRatio": ((total_weight / max_weight) * 100.0 * 10.0).round() / 10.0,
    "untrustedCount": untrusted_count,
    "riskLevel": overall_risk,
    "riskSource": risk_source,
    // 2026-09-23：档位百分比**提取为变量** `sl_pct`/`tp_pct`（定义见 `pm_result` 上方），
    //   本处只做引用。动机见上方定义处的说明（档位表禁止在文件内出现三份）。
    "stopLossPct": sl_pct,
    "takeProfitPct": tp_pct,
    // ── 2026-09-23：交易价位的**绝对价格**（公式侧首次产出）──
    // 根因（300642 实证，2026-09-23）：本 map 此前**只输出百分比**，绝对价格键从不产出 ⇒
    //   `dashboard_report` 读 `targetPrice`/`stopLoss` 恒 None ⇒ 展示层输出
    //   「交易目标价: —（未设）」，同时止损位被 **LLM 自填的单端值**（17.90，-14.8%）顶替
    //   ⇒ 用户读到「没有目标价，却有止损」这一自相矛盾的组合。
    //   而公式**明明已算出完整成对档位**（timeHorizon=mid ⇒ 止损 8% / 止盈 18%），
    //   只因无人把它换算成价格而整体丢失。原注释「本函数恒命中」正是该断链的自证。
    // 口径：`现价 × (1 ∓ 档位%)` —— 这是**风控档位**的换算，**不是价格预测**：
    //   它是确定性的、可复算的，且与同一 JSON 里的 `timeHorizon` 自洽；
    //   与 LLM 自由填值相比，前者可被下游校验，后者不可。
    // 优先级：本键一旦产出，`merge_price_fields_from_llm`（decision.rs）的
    //   `is_none_or(is_null)` 判据即判为「已存在」⇒ 不再覆盖为 LLM 值 ⇒ **公式优先**。
    // 空仓档（`sl_pct = 0.0`，即 `position_pct <= 0`）仍输出 `()`（JSON null）
    //   ⇒ 由展示层 `pair_trade_prices` 按成对性统一处理，不产出半截交易计划。
    "targetPrice": if sl_pct > 0.0 && present(current_price) && current_price > 0.0 {
        (current_price * (1.0 + tp_pct / 100.0) * 100.0).round() / 100.0
    } else { () },
    "stopLoss": if sl_pct > 0.0 && present(current_price) && current_price > 0.0 {
        (current_price * (1.0 - sl_pct / 100.0) * 100.0).round() / 100.0
    } else { () },
    // 2026-09-23 阶段1（PROPOSAL-stock-decision-four-horizon.md）：
    //   四周期价位映射。主档位 targetPrice/stopLoss 语义不变（仍取 `time_horizon` 档），
    //   本映射按 ultra_short/short/mid/long 各出一组「档位% + 持有天数 + 绝对价」，
    //   供展示层按周期 Tab 查看，且为阶段2「四周期独立决策」打数据契约垫脚石。
    //   消费方缺省时此键缺席不影响旧路径（decision.rs 解析做 snake/camel 双兼容）。
    "horizonPriceMap": horizon_price_map,
    // 阶段2（PROPOSAL-stock-decision-four-horizon.md）：四周期独立决策。
    // 供前端按周期 Tab 分组展示；主档位 action/仓位/价位语义不受影响。
    "decisionsByHorizon": decisions_by_horizon,
    // P0-I(2026-09-12): 置信 / 仓位必须格式化后再进用户可见文案。
    //   原实现直接插值 f64，把全精度打进决策卡（实证截图：`置信=45.74106680714534`）。
    //   沿用本文件既有写法 `${(x*10.0).round()/10.0}`（见 bearish_downgrade_note 的 to_int 惯例）。
    // 2026-09-22: 结论名回归 `final_action`（方向档）—— 展示档派生已移除，
    //   见上方说明：结论名必须由确定性后验阶梯唯一决定，不得掺入仓位/LLM 措辞。
    "reasoning": `决策=${final_action} 置信=${(final_confidence*10.0).round()/10.0} 仓位=${(position_pct*10.0).round()/10.0}%${gap_note}${bayes_note}${kelly_note}${valuation_note}${disagreement_note}${contradiction_note}${risk_downgrade_note}${bearish_downgrade_note}${risk_note}${collapse_note}`,
    "data_gaps": data_gaps,
    "timeHorizon": time_horizon,
    "expectedHoldingDays": expected_holding_days,
    // V74 修复(2026-09-12): 与 sanity 检查 1 同源缺陷。
    //   旧定义 `targetPrice <= stopLoss` 隐含做多语义，对看空方案
    //   （targetPrice < currentPrice < stopLoss）恒为 true ⇒ 方向自洽的「卖出」样本
    //   被 DecisionBanner 挂上「⚠ 决策矛盾」红标（2026-09-12 实证截图同一根因）。
    //   加 !trader_dir_bearish 豁免后，与 sanity 检查 1 的判据完全对齐。
    "isContradictory": present(trader_target_price) && present(trader_stop_loss) && trader_stop_loss > 0.0
        && trader_target_price <= trader_stop_loss && !trader_dir_bearish,
    "targetTimeframe": target_timeframe,
    "evidence": #{
        "prior": prior, "posterior": (posterior*1000.0).round()/1000.0,
        "avgSignal": (avg_signal*100.0).round()/100.0,
        "activeFactors": total_weight, "totalWeight": total_weight,
        "oddsSource": if trader_price_no_info { "trader价格信号无效" } else { odds_source },
        "tradingCostPct": (COST_PCT*100.0).to_int(),
        "f9_money_flow_signal": (f9_signal*100.0).round()/100.0,
        "f10_chip_signal": (f10_signal*100.0).round()/100.0,
        "f11_pace_signal": (f11_signal*100.0).round()/100.0,
        "f12_momentum_signal": (f12_signal*100.0).round()/100.0,
        "announcement_signal": (announcement_modifier*100.0).round()/100.0,
        // ── B3-c(2026-09-12): 11 组因子 σ/w 全量落库 ──
        // 原实现只落 f9~f12 四个信号（上方扁平键），**f1~f7 全缺** ⇒ 事后归因必须
        //   逆向重建：从 bb 的 `_raw.*` / `params.*` 节点快照取上游输入，再查
        //   `workflow_template_versions` 抽当日模板版本的内嵌 rhai 复算旧公式。
        //   09-12 实测该路径代价极高，且踩了「用现版本公式复算历史样本」的版本坑
        //   （12/13 样本跑在 v10~v26，无成长豁免；按 v30+ 复算系统性偏高 ~4.5pt）
        //   ⇒ 改为就地落全量，让 avg_signal 可**逐项对账**而无需逆向。
        // 口径：σ 用**实际进入 weighted_signal 的那个值**（f4 是 `f4_adjusted`，
        //   不是 `f4_signal`）；w 是**经 IC 权重 × regime_mod 之后的最终权重**
        //   （与 total_weight 同源，含协方差衰减后的值）。
        // 对账不变量：Σ(sigma × weight) / weightSum == avgSignal
        //   （残差仅来自下面的 3 位小数舍入）。
        // 加法式扩展：上方 4 个扁平信号键全部保留；已 grep 前端 + Rust，
        //   无消费方按键集校验 evidence（故新增键向后兼容）。
        "factors": #{
            "f1":  #{ "name": "trend",        "sigma": (f1_signal*1000.0).round()/1000.0,   "weight": (f1_weight*10000.0).round()/10000.0 },
            "f2":  #{ "name": "consensus",    "sigma": (f2_signal*1000.0).round()/1000.0,   "weight": (f2_weight*10000.0).round()/10000.0 },
            "f3":  #{ "name": "catalyst",     "sigma": (f3_signal*1000.0).round()/1000.0,   "weight": (f3_weight*10000.0).round()/10000.0 },
            "f4":  #{ "name": "risk",         "sigma": (f4_adjusted*1000.0).round()/1000.0, "weight": (f4_weight*10000.0).round()/10000.0 },
            "f5":  #{ "name": "valuation",    "sigma": (f5_signal*1000.0).round()/1000.0,   "weight": (f5_weight*10000.0).round()/10000.0 },
            "f6":  #{ "name": "data_quality", "sigma": (f6_signal*1000.0).round()/1000.0,   "weight": (f6_weight*10000.0).round()/10000.0 },
            "f7":  #{ "name": "trade_signal", "sigma": (f7_signal*1000.0).round()/1000.0,   "weight": (f7_weight*10000.0).round()/10000.0 },
            "f9":  #{ "name": "money_flow",   "sigma": (f9_signal*1000.0).round()/1000.0,   "weight": (f9_weight*10000.0).round()/10000.0 },
            "f10": #{ "name": "chip",         "sigma": (f10_signal*1000.0).round()/1000.0,  "weight": (f10_weight*10000.0).round()/10000.0 },
            "f11": #{ "name": "pace",         "sigma": (f11_signal*1000.0).round()/1000.0,  "weight": (f11_weight*10000.0).round()/10000.0 },
            "f12": #{ "name": "momentum",     "sigma": (f12_signal*1000.0).round()/1000.0,  "weight": (f12_weight*10000.0).round()/10000.0 },
            // v78(2026-09-19): f13 瓶颈因子 —— 落进本表使「核心方法论是否真被消费」
            //   可**逐项对账**，而不是只体现在最终 posterior 上。σ/w 口径与上方
            //   weighted_signal 同源，故对账不变量 Σ(sigma×weight)/weightSum == avgSignal
            //   自动涵盖 f13，无需额外基准。
            "f13": #{ "name": "bottleneck",   "sigma": (f13_signal*1000.0).round()/1000.0,  "weight": (f13_weight*10000.0).round()/10000.0 },
        },
        // 与 factors 配套的对账基准（原实现只落 totalWeight，缺分母上限 ⇒
        //   无法判断「证据是否被 regime 压缩」，而这正是 weights_collapsed 的判据）
        "weightSum": total_weight,
        "maxWeight": max_weight,
    },
    "risk_quant": #{
        "f4_base": (f4_signal*100.0).round()/100.0,
        "f4_quant_adjust": (f4_quant_adjust*100.0).round()/100.0,
        "f4_adjusted": (f4_adjusted*100.0).round()/100.0,
        "notes": f4_quant_notes,
    },
    // ── V77(2026-09-14): DCF 模型适用性（结构化输出）──
    // 存在的理由：DCF 被剔除这个**动作本身**必须可见，否则面板上的「低估 143%」
    //   与决策卡的「观望 0%」就是两个互相打脸且无法解释的数字。
    //   `risk_quant` 已开「子诊断对象」先例，此处沿用同一形态。
    //   `dcfApplicable` 缺省取 `true`（旧模板未注入该变量时，行为与改动前完全一致）。
    "valuationApplicability": #{
        "dcfApplicable": if present(valuation_dcf_applicable) { valuation_dcf_applicable } else { true },
        "dcfLegUsed": dcf_leg_usable,
        "grahamLegUsed": graham_leg_usable,
        "reason": if present(valuation_dcf_inapplicable_reason) { valuation_dcf_inapplicable_reason } else { "" },
        // ── 2026-09-21 新增：两条腿各自的「锚定退化」标记 ──
        // 为什么必须透出：`dcfApplicable` 只覆盖「模型前提不成立」这一档，
        //   而**更常见**的是「数值照常输出、但锚定是历史代理」（`is_fallback_anchor`）
        //   或「增长率假设被封顶」（`growthClampedUpper`）—— 这两种情形下面板上的
        //   区间看着像个正经估值，实际系统性偏低。不标注 ⇒ 用户永远不知道分母是什么。
        //   300308 实证：面板显示「内在价值 80.63–156.77 元」，而该区间由
        //   「近 5 年正净利均值 × 0.90」代理锚算出 ⇒ 与当期现金流无关。
        // 缺省 false（旧模板未注入时行为与改动前完全一致）。
        "anchorIsFallback": if present(valuation_dcf_anchor_is_fallback) { valuation_dcf_anchor_is_fallback } else { false },
        "grahamGrowthClamped": if present(valuation_graham_growth_clamped) { valuation_graham_growth_clamped } else { false },
    },
    "decision_trail": decision_trail,
    "pre_sim_action": pre_sim_action,
    "pre_sim_position_pct": pre_sim_position_pct,
    // ── P0-11(2026-09-12): 未封顶的凯利意愿仓位（%），供对账 ──
    // 用途：action 判定的仓位门读的是它（而非 positionPct），审计时可直接验证
    //   「门槛是否被风险封顶污染」。注意**不可放进 effective_params** ——
    //   那个 map 的不变量是「字段集合 == PORTFOLIO_MGR_TUNABLE_PARAMS（28 项）」，
    //   本变量是派生输出、不是可调参数，混入会破坏反思侧的参数尺子。
    "kelly_pos_uncapped": kelly_pos_uncapped,
    "technical_veto": #{
        "vetoed": tech_vetoed,
        "rule_id": tech_veto_rule,
        "reason": tech_veto_reason,
    },
    "simulation_gate": #{
        "vetoed": sim_vetoed,
        "rule_id": sim_veto_rule,
        "reason": sim_veto_reason,
        "pre_sim_action": pre_sim_action,
        "pre_sim_position_pct": pre_sim_position_pct,
        "sim_stability": if present(sim_stability) { (sim_stability*100.0).round()/100.0 } else { () },
        "sim_liquidity": if present(sim_liquidity) { (sim_liquidity*100.0).round()/100.0 } else { () },
        "sim_impact": if present(sim_impact) { sim_impact } else { () },
    },
    // ⚠️ 2026-09-13: `sig` 原写作 `(avg_signal*100.0).round()/10.0`（0.16 → 1.6，无意义刻度），
    //   与同一次输出里的 `evidence.avgSignal`（`*100/100` ⇒ 0.16）**同量不同刻度**（铁律 15）。
    //   决策解释官会读本串，两种刻度会让它误判信号强度。统一为 `/100.0`。
    "computation_logs": `prior=${prior}${prior_src} post=${(posterior*100.0).round()/100.0} sig=${(avg_signal*100.0).round()/100.0} w=${total_weight} odds=${(odds*10.0).round()/10.0} k=${(kelly_raw*1000.0).round()/1000.0} pos=${position_pct} f9=${(f9_signal*100.0).round()/100.0} f10=${(f10_signal*100.0).round()/100.0} f11=${(f11_signal*100.0).round()/100.0} f12=${(f12_signal*100.0).round()/100.0} ann=${(announcement_modifier*100.0).round()/100.0}`,
    "f7_free": #{
        "posterior": posterior_no_f7,
        "action": f7_free_action,
        "effective_posterior": effective_posterior_no_f7,
        "f7_weight": f7_weight,
        "total_weight": total_weight,
        "f7_weight_pct": f7_weight_pct,
    },
    // ── 本次实际生效的可配置参数快照（2026-09-11）──
    // 用途：让「反思 / 演进」能观测「做出该决策时用的是哪套参数」。
    //   缺少这个快照时，反思只能看到决策结果而看不到参数上下文，
    //   无法归因（例如无法判断「posterior 偏低」是因子弱还是先验被调低了）。
    // 覆盖层级：rhai 内置默认 < 模板变量(context.variables) < 反思建议应用值，
    //   此处输出的是**最终生效值**；字段名与模板变量名一一对应，可直接作为
    //   反思参数建议的 param 名（apply_param_suggestions 同时支持短名别名）。
    // ⚠️ 字段集合必须与 `seed_stock_analysis.rs::PORTFOLIO_MGR_TUNABLE_PARAMS`（28 项）
    //   完全一致 —— 权威源是「有哪些可调参数」的唯一定义，本 map 是「它们各自生效成什么」。
    //   两者集合不等意味着反思会看到一把不完整的尺子（历史上就漏过 trader_cap_min_weight）。
    // source 判定：能读到 action_buy_threshold 即说明 input_mapping 已注入模板值，
    //   否则说明模板变量缺失、走的是 .rhai 内置默认值。
    "effective_params": #{
        "source": if present(action_buy_threshold) && present(regime_prior_bull) { "template" } else { "rhai_default" },
        "regime_prior_bull": REGIME_PRIOR_BULL,
        "regime_prior_sideways": REGIME_PRIOR_SIDEWAYS,
        "regime_prior_bear": REGIME_PRIOR_BEAR,
        // 因子融合门（f7 权重低于此值时其看空信号不再封顶后验）—— 原漏登记，
        //   导致反思只能看到「f7_weight」却看不到判定它是否达标的那把尺子。
        "trader_cap_min_weight": TRADER_CAP_MIN_WEIGHT,
        "action_buy_threshold": ACTION_BUY_THRESHOLD,
        "action_increase_threshold": ACTION_INCREASE_THRESHOLD,
        "action_hold_threshold": ACTION_HOLD_THRESHOLD,
        "action_watch_threshold": ACTION_WATCH_THRESHOLD,
        "action_reduce_threshold": ACTION_REDUCE_THRESHOLD,
        "pos_buy_min": POS_BUY_MIN,
        "pos_increase_min": POS_INCREASE_MIN,
        "pos_cap_extreme": POS_CAP_EXTREME,
        "pos_cap_high": POS_CAP_HIGH,
        "pos_cap_mid": POS_CAP_MID,
        "risk_vol_extreme": RISK_VOL_EXTREME,
        "risk_vol_high": RISK_VOL_HIGH,
        "risk_vol_low": RISK_VOL_LOW,
        "risk_dd_high": RISK_DD_HIGH,
        "risk_dd_low": RISK_DD_LOW,
        "risk_sharpe_extreme": RISK_SHARPE_EXTREME,
        "risk_sharpe_low": RISK_SHARPE_LOW,
        "risk_debt_extreme": RISK_DEBT_EXTREME,
        "risk_debt_high": RISK_DEBT_HIGH,
        "risk_debt_low": RISK_DEBT_LOW,
        "risk_roe_high": RISK_ROE_HIGH,
        "risk_roe_low": RISK_ROE_LOW,
        "risk_growth_low": RISK_GROWTH_LOW,
        "cost_pct": COST_PCT,
    }
}

}

// ── 错误兜底：Rhai 运行时异常时返回保守决策，确保节点始终输出可解析结果 ──
catch (err) {
    pm_result = #{
        // P0-5(2026-09-14): Rhai 执行异常时不再输出 action="观望"。
        // 「脚本崩了」与「判断为观望」是两回事：把前者写成后者，会让 UI 与 DB
        // 出现一条从未被任何逻辑产出的观望结论（不可追溯、不可统计）。
        "action": "数据缺失",
        "positionState": "EMPTY",
        "positionPct": 0.0,
        "confidence": 50.0,
        "decisionConfidence": 50.0,
        "signalStrength": 0.0,
        "riskLevel": "中风险",
        "riskSource": "错误兜底",
        "stopLossPct": 0.0,
        "takeProfitPct": 0.0,
        // 2026-09-23：与成功路径**同 key 集**（沿用上方 B3-c 的原则）——本路径无交易计划，
        //   故显式输出 JSON null 而非**缺键**。缺键会让 `merge_price_fields_from_llm`
        //   把 LLM 自填价位灌进一条「Rhai 执行异常」的记录 ⇒ 一条从未被任何公式产出的
        //   价位出现在异常记录上，与本 catch 块 L2668「脚本崩了 ≠ 判断为观望」同源：
        //   **异常路径不得携带任何交易结论。**
        "targetPrice": (),
        "stopLoss": (),
        "reasoning": "portfolio-mgr Rhai 执行异常，已降级为保守决策: " + err,
        "data_gaps": [],
        "timeHorizon": "short",
        "expectedHoldingDays": 5,
        "isContradictory": false,
        "targetTimeframe": "1w",
        // B3-c(2026-09-12): 兜底路径同样输出 factors（空 map）+ 对账基准，
        //   保证成功/异常两条路径的 key 集一致 —— 审计脚本按「factors 是否存在」
        //   判定该次运行是否落了全量因子时会用到，缺键会把异常误判成「旧版本」。
        "evidence": #{ "prior": 0.5, "posterior": 0.5, "avgSignal": 0.0,
            "activeFactors": 0.0, "totalWeight": 0.0,
            "factors": #{}, "weightSum": 0.0, "maxWeight": 0.0 },
        "risk_quant": #{ "f4_base": 0.0, "f4_quant_adjust": 0.0, "f4_adjusted": 0.0, "notes": [] },
        "decision_trail": [],
        "simulation_gate": #{ "vetoed": false, "rule_id": "", "reason": "" },
        "computation_logs": "ERROR: " + err,
        "f7_free": #{ "posterior": 0.5, "action": "观望", "effective_posterior": 0.5, "f7_weight": 0.15, "total_weight": 0.0, "f7_weight_pct": 0.0 }
    };
}

pm_result
