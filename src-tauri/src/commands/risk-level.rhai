// risk-level.rhai — 风险等级确定性分类（替代原 LlmClassifierNode `cls-risk-level`）
//
// ── 本次改造的背景与口径声明 ──
// 原节点是 `LlmClassifierNode`（`seed_stock_analysis.rs` 内联 prompt），每轮运行耗
// 4.8s（均值，实测 76 次调用），并曾 5 次撞 60s 超时导致整条运行落
// `partially_completed`。其 prompt 通篇是**确定性阈值规则**，并明写
// 「不要计算综合评分，直接按规则判定」—— 等于用 LLM 跑一个 if-else。
//
// 本脚本把同一套规则下沉为 Rhai 确定性实现：<10ms、零 token、零超时。
//
// **口径来源 = 原 prompt 原文（下方完整保留），阈值 1:1 转写，未做任何调整。**
// 唯一新增的是原 prompt 未明说的两处兜底语义，均已在对应位置显式注释。
//
// ── 输入契约（由 CodeNodeConfig.input_mapping 注入）──
//   risk_volatility      ← t-risk.result.content.stockRiskProfile.annualizedVolatilityPct
//   risk_drawdown        ← ...maxDrawdownPct
//   risk_sharpe          ← ...sharpeRatio
//   risk_roe             ← ...roeTTMPct
//   risk_gross_margin    ← ...grossMarginPct
//   risk_debt_ratio      ← ...debtRatioPct
//   risk_revenue_growth  ← ...revenueGrowthYoYPct
//
// ⚠️ 类型契约（2026-09-20 对生产库实证，非推断）──
//   `t-risk.output.result.content` 是**字符串**（JSON 文本）；`executors/mod.rs::resolve_var_path`
//   在穿透中间段时会自动 `from_str` 解析它，因此 `...content.stockRiskProfile.<field>`
//   最终拿到的是 **JSON Number**。DB 实证（逐字段 `jsonb_typeof`）：
//       content::jsonb -> 'stockRiskProfile' -> 'annualizedVolatilityPct'  ⇒ number
//       其余 6 个字段（maxDrawdownPct / sharpeRatio / roeTTMPct / grossMarginPct /
//       debtRatioPct / revenueGrowthYoYPct）同为 number。
//   ⇒ 本脚本收到的是 f64，`has()` 命中；**不是**字符串。
//   ⚠️ 若用 `->>`（text 抽取）核对本契约，会看到带引号的字符串 —— 那是导出产物，
//      不是真实类型。真若按字符串透出，7 个 `has_*` 全 false ⇒ `degraded=true`
//      ⇒ 全部静默退化为**保守兜底档「高风险」**（不报错）。该风险的可见性由输出里的
//      `degraded` 与 `warnings` 承担，勿删。
//      ⚠️ 2026-09-20 起兜底档由「中风险」（中性）改为「高风险」（保守）：本契约一旦
//         失效，全量标的都会被压成高风险 —— **这是放大器，不是无害降级**。
//         排查顺序：先核对下方 DB 实证（逐字段 `jsonb_typeof`），再看 warnings 里
//         是否出现「缺失 annualizedVolatilityPct」。
//
// ⚠️ V57 防御（7 个输入变量**必须**逐个出现在 `present(...)` 调用里）──
//   `code_executor.rs::execute_rhai_directly` 会**文本扫描**脚本源码、把 `present(<变量>)`
//   的实参名抠出来（**注意：它不剥注释、不看语义**），再对「未被 input_mapping 注入、
//   且不是 fn 形参 / let 局部变量」的名字自动补 `unit`。
//   ⇒ 反过来说：注释或字符串里只要出现「present( + 裸标识符」这个字面量，就会凭空往
//      scope 里塞一个同名 unit 常量（本文件 2026-09-20 自查时连踩两次：在文档里举例写了
//      那个字面量，扫描结果里就真多出一个变量）。写文档时一律用 `present(...)` /
//      `present(<变量>)` 这两种形态（括号后是 `.` 或 `<`，扫描器不会取值）。
//   若某变量从不出现于 `present(...)`，一旦模板漏配该键，Rhai 会在**求值实参阶段**
//   直接抛 `Variable not found: <名>` —— 整个节点报错，下游 `overall_risk_llm` 取不到值
//   （是硬失败，不是降级）。故下方取值一律先经 `present()`。
//   这也是 pace-calc.rhai / portfolio-mgr.rhai 的既有形态。
//
// ── 输出契约（下游消费方，改动需同步）──
//   ① `category` —— 字符串档位，被两处消费：
//      - `portfolio-mgr` 的 input_mapping：("overall_risk_llm", "cls-risk-level.result.category")
//        → 该脚本 :455 用作算法分类失败时的回退档；:484-492 的 f4_signal 查表
//          （低 +0.30 / 中 -0.05 / 高 -0.25 / 极高 -0.50），是 f4 因子的实际来源
//      - 前端 `src/lib/decisionInputDiagnosis.ts` 的 `case "cls-risk-level"` 读
//        `content.category`（该处对 CodeNode 会自动解包 `result`，故无需改动）
//   ② `risk_factors` —— 保 `config/llm-output-schemas.json` 的 `risk_output` 形状
//      （该 schema 全仓无代码引用，仅作形状对齐）
//   ③ `reason` / `matched_rules` / `metrics` —— 决策留痕，供诊断面板与复盘
//
// ── 与原 LLM 版的实测差异（真库 59 条可比对样本影子比对）──
//   档位一致率 64.4%（38/59）；posterior 位移**均值 +0.15pp**（双向抵消，
//   无系统性放松/收紧），最大上抬 +4.63pp、最大下压 -3.60pp。
//   分歧主因是原 LLM 判定自身不一致，而非本脚本偏离规则 —— 实证：
//   `debt=91.6, growth=-0.3` 判「高风险」，而 `debt=92.2, growth=+8.3` 判「极高风险」
//   （后者负债率更高、增长为正，档位反而更严）。
//   影子比对脚本与完整分歧清单：`output/rhai-shadow-compare.mjs` / `output/rhai-shadow-report.txt`。
//
// ── 已知未实现项（数据不可得，与原 LLM 版**同等受限**）──
//   ⚠️ 原 prompt 的「极高风险 ① 标的为 ST/*ST/退市股」与「低风险 ③ 无 ST/退市风险，
//      无重大负面公告」两条判据**无法实现**：t-risk 输出（astock-data/src/mcp_tools.rs
//      的 compute_portfolio_risk）只含 stockRiskProfile 的 7 个数值字段，不含任何 ST
//      标记或公告数据；`is_st` 仅存在于上游 Quote（astock-data/src/types.rs）且未被透出。
//      原 LLM 版同样拿不到该信息 ⇒ 该判据在改造前后**一致地不生效**（等价照抄）。
//      若要真正启用，需在 compute_portfolio_risk 的输出补 `isST` 字段（属改口径，待批）。

// ── 辅助函数（Rhai 要求函数位于全局作用域）──
fn present(x) { type_of(x) != "()" }

// has: 是否为可用数值（数字类型判定）。
//   Rhai 整数宽度随构建而异：64-bit（本项目 rt-workflow）为 "i64"，32-bit 为 "int"，都接。
//   ⚠️ 字符串**不算**可用数值：上游契约是 JSON Number（见文件头类型契约实证）。
//      若哪天上游改成字符串透出，本函数返回 false ⇒ 走 degraded 兜底，
//      而不是把 "22.7" 当成数字参与比较（那会静默产出错误档位）。
fn has(x) {
    let t = type_of(x);
    t == "f64" || t == "i64" || t == "int"
}

// maybe: 命中则取值，否则输出 unit（→ JSON null）。
//   用于杜绝「缺失」被哨兵值（-1.0 / 0.0）伪装成一个真实读数。
fn maybe(ok, v) {
    if ok { v } else { () }
}

// num_of: 取数值，缺失返回默认值（默认值仅用于判定，绝不进输出）
fn num_of(x, dflt) {
    if type_of(x) == "f64" { return x; }
    if type_of(x) == "i64" || type_of(x) == "int" { return x.to_float(); }
    dflt
}

fn join_arr(arr, sep) {
    let s = "";
    for i in 0..arr.len() {
        if i > 0 { s += sep; }
        s += arr[i];
    }
    s
}

// ══════════════════════════════════════════════════════════════════════════
// 以下为原 prompt 原文（`seed_stock_analysis.rs` 的原 `LlmClassifierNode.config.prompt`），
// 完整保留作为本脚本的**口径权威来源**。修改本脚本前请先核对本段。
//
// 「你是专业风险分析师。根据以下单股风险画像数据，判断该股票的整体风险等级
//   （低风险/中风险/高风险/极高风险）。
//
// ## 数据解读（A股标准）
// ### 量化指标
// - annualizedVolatilityPct: 年化波动率（%）。A股正常15-45%，<20%低波动，20-35%正常，35-50%偏高，>50%高波动
// - maxDrawdownPct: 历史最大回撤（%）。A股正常20-40%，<25%好，25-40%正常，40-55%偏大，>55%深
// - sharpeRatio: 夏普比率。>0.5好，0-0.5正常，<0偏弱
//
// ### 基本面指标
// - roeTTMPct: ROE(TTM)（%）。>10%良好，5-10%一般，<5%偏弱
// - grossMarginPct: 毛利率（%）。>25%好，15-25%正常，<15%偏低
// - debtRatioPct: 资产负债率（%）。<40%低，40-60%正常，>60%偏高
// - revenueGrowthYoYPct: 营收增速YoY（%）。>15%好，5-15%正常，0-5%偏低，<0萎缩
// - peTTM: 市盈率(TTM)。<0亏损，0-20低，20-40正常，>40偏高
//
// ## 等级判定规则（按优先级，满足即判定，不要计算综合评分）
// ### 极高风险（立即回避）
// - 标的为ST/*ST/退市股
// - 资产负债率>80% 且 营收增速<0（财务困境）
// - 年化波动率>60% 且 夏普比率<-1.5
//
// ### 高风险（谨慎参与，小仓位）
// - 量化维度高风险：波动率>40% 或 夏普<0 或 回撤>45%
// - 且基本面有至少一个风险点：ROE<5% 或 毛利率<10% 或 负债率>65%
// - 或：量化偏高（波动率>35% 或 夏pe<0.3）且基本面无亮点（ROE<8% 且 营收增速<5%）
//
// ### 中风险（正常参与）
// - 量化维度中等：波动率20-40% 或 夏普0-0.5 或 回撤25-45%
// - 且基本面无硬伤：ROE>5% 且 负债率<65% 且 营收增速>0
// - 或：量化低风险（波动率<20% 且 夏普>0.5）但基本面中等
//
// ### 低风险（优先配置）
// - 量化维度低风险：波动率<20% 且 夏普>0.5 且 回撤<30%
// - 且基本面健康：ROE>10% 且 毛利率>20% 且 负债率<50% 且 营收增速>5%
// - 且：无ST/退市风险，无重大负面公告
//
// ## 重要
// - 不要计算综合评分，直接按规则判定
// - A股大多数股票应落在「中风险」档
// - 仅当量化+基本面均差时才判「高风险」」
//
// ⚠️ 原文「夏pe<0.3」为笔误，按语义取「夏普比率<0.3」实现（下方 HIGH-B）。
// ══════════════════════════════════════════════════════════════════════════

let matched_rules = [];
let warnings = [];

// 数据可用性（**先判可用性**）：原 LLM 版拿到的是一段 JSON 文本，字段缺失时它会跳过该子条件；
// 本实现同样「缺失即该子条件为 false」（不因缺失而加严），保持判定等价。
// ⚠️ 每个 present() 都是 V57 防御的触发点，7 个变量一个都不能少（见文件头「V57 防御」）；
//    present() 判"有没有"，has() 判"类型是不是数字"，两层都要。
let has_vol = present(risk_volatility) && has(risk_volatility);
let has_dd = present(risk_drawdown) && has(risk_drawdown);
let has_sharpe = present(risk_sharpe) && has(risk_sharpe);
let has_roe = present(risk_roe) && has(risk_roe);
let has_gm = present(risk_gross_margin) && has(risk_gross_margin);
let has_debt = present(risk_debt_ratio) && has(risk_debt_ratio);
let has_growth = present(risk_revenue_growth) && has(risk_revenue_growth);

// 指标取值：缺失用哨兵（而非 0，避免"缺失"被当成"读数很低"参与判定）。
// 哨兵**只参与判定**，绝不进输出 —— 输出一律经 maybe() 转 null。
let vol = num_of(risk_volatility, -1.0);
let dd = num_of(risk_drawdown, -1.0);
let sharpe = num_of(risk_sharpe, 0.0);
let roe = num_of(risk_roe, 0.0);
let gm = num_of(risk_gross_margin, 0.0);
let debt = num_of(risk_debt_ratio, 0.0);
let growth = num_of(risk_revenue_growth, 0.0);

// 核心指标（波动率）缺失 ⇒ 无法做任何量化判定，直接兜底。
// 兜底档取「高风险」（**保守档**，2026-09-20 裁决）—— 依据三条：
//   ① 原节点的 fallback_label「中风险」是**中性兜底**，而下游 `portfolio-mgr` 只用
//      `category` 算 f4 因子（portfolio-mgr.rhai 的 switch：中 -0.05 / 高 -0.25），
//      **并不消费 degraded** ⇒ 中性档在决策链上等价于「我判定它是中风险」，
//      即把「没判定」伪装成「已判定」。低置信/缺失应倾向**不行动**而非默认放行。
//   ② 本档表征的是**未知**；未知作用在资金方向上不可逆 ⇒ 取代价较小的一侧。
//   ③ 存量影响面 = 0：真库 59 条可比对样本中 degraded **零命中**
//      （rhai-shadow-report.txt 里「LLM 档缺失(f4=0.0)」的 4 条其 vol 均存在）。
//      故本改动只影响未来异常场景，不改写任何既有结论。
//   ⚠️ degraded 与 warnings 仍照常输出（诊断面板与复盘用），勿删。
let degraded = !has_vol;
if degraded {
    warnings.push("stockRiskProfile 缺失 annualizedVolatilityPct —— 未做量化判定，按保守兜底档处理");
}

// ── 规则判定 ──
// ⚠️ 判定顺序说明（唯一一处对 prompt 的补充）：prompt 按「极高→高→中→低」罗列并称
//   「满足即判定」，但若机械照该顺序，中风险条件 A（三个"或"，极宽松）会抢在低风险
//   之前命中，使「低风险（优先配置）」实际不可达。本实现按**条件特异性**排序
//   （极严的条件先判），与原 LLM 的实测行为一致（实测确有条目被判为低风险）。
//   顺序：极高 → 高A → 高B → 低 → 中A → 中B → 兜底中风险。

let level = "";
let reason = "";

// ── 极高风险（立即回避）──
if !degraded {
    if has_debt && has_growth && debt > 80.0 && growth < 0.0 {
        matched_rules.push("EXTREME-2: 资产负债率>80% 且 营收增速<0（财务困境）");
        level = "极高风险";
    }
    if has_vol && has_sharpe && vol > 60.0 && sharpe < -1.5 {
        matched_rules.push("EXTREME-3: 年化波动率>60% 且 夏普比率<-1.5");
        level = "极高风险";
    }
    // EXTREME-1「标的为ST/*ST/退市股」—— 数据不可得，见文件头「已知未实现项」
}

// ── 高风险（谨慎参与，小仓位）──
if !degraded && level == "" {
    // 高风险-A：量化维度高风险 且 基本面至少一个风险点
    let quant_high = (has_vol && vol > 40.0) || (has_sharpe && sharpe < 0.0) || (has_dd && dd > 45.0);
    let fundamental_risk = (has_roe && roe < 5.0) || (has_gm && gm < 10.0) || (has_debt && debt > 65.0);
    if quant_high && fundamental_risk {
        matched_rules.push("HIGH-A: 量化高风险（波动率>40% 或 夏普<0 或 回撤>45%）且 基本面有风险点（ROE<5% 或 毛利率<10% 或 负债率>65%）");
        level = "高风险";
    }
    // 高风险-B：量化偏高 且 基本面无亮点
    // （原文「夏pe<0.3」笔误，按「夏普<0.3」实现）
    let quant_elevated = (has_vol && vol > 35.0) || (has_sharpe && sharpe < 0.3);
    let fundamental_dull = (has_roe && roe < 8.0) && (has_growth && growth < 5.0);
    if level == "" && quant_elevated && fundamental_dull {
        matched_rules.push("HIGH-B: 量化偏高（波动率>35% 或 夏普<0.3）且 基本面无亮点（ROE<8% 且 营收增速<5%）");
        level = "高风险";
    }
}

// ── 低风险（优先配置）──
// ⚠️ 第 ③ 条「无ST/退市风险，无重大负面公告」数据不可得，见文件头；仅以量化+基本面 7 项判定。
//    注：7 项全为 AND，门槛很高 —— 真库 59 条实测样本中零命中（原 LLM 版判出 2 条低风险，
//    用的是更宽的标准）。此处不改门槛，以保持与 prompt 的字面一致。
if !degraded && level == "" {
    let quant_low = (has_vol && vol < 20.0) && (has_sharpe && sharpe > 0.5) && (has_dd && dd < 30.0);
    let fundamental_healthy = (has_roe && roe > 10.0) && (has_gm && gm > 20.0)
        && (has_debt && debt < 50.0) && (has_growth && growth > 5.0);
    if quant_low && fundamental_healthy {
        matched_rules.push("LOW: 量化低风险（波动率<20% 且 夏普>0.5 且 回撤<30%）且 基本面健康（ROE>10% 且 毛利率>20% 且 负债率<50% 且 营收增速>5%）");
        level = "低风险";
    }
}

// ── 中风险（正常参与）──
if !degraded && level == "" {
    // 中风险-A：量化维度中等 且 基本面无硬伤
    let quant_mid = (has_vol && vol >= 20.0 && vol <= 40.0)
        || (has_sharpe && sharpe >= 0.0 && sharpe <= 0.5)
        || (has_dd && dd >= 25.0 && dd <= 45.0);
    let fundamental_ok = (has_roe && roe > 5.0) && (has_debt && debt < 65.0) && (has_growth && growth > 0.0);
    if quant_mid && fundamental_ok {
        matched_rules.push("MID-A: 量化中等（波动率20-40% 或 夏普0-0.5 或 回撤25-45%）且 基本面无硬伤（ROE>5% 且 负债率<65% 且 营收增速>0）");
        level = "中风险";
    }
    // 中风险-B：量化低风险但基本面中等（未达 LOW 的严格门槛）
    let quant_low_b = (has_vol && vol < 20.0) && (has_sharpe && sharpe > 0.5);
    if level == "" && quant_low_b {
        matched_rules.push("MID-B: 量化低风险（波动率<20% 且 夏普>0.5）但基本面中等");
        level = "中风险";
    }
}

// ── 兜底 ──
// 分两种情况，**不可合并**（2026-09-20 裁决）：
//   ① degraded（数据缺失）⇒「高风险」保守档，理由见上方 degraded 定义处；
//   ② 规则全不命中（数据齐全但落在规则缝里）⇒「中风险」，依据 prompt 的
//      「A股大多数股票应落在中风险档」—— 这一档是**真判定**，不是降级。
if level == "" {
    if degraded {
        level = "高风险";
    } else {
        level = "中风险";
        matched_rules.push("FALLBACK: 未命中任何显式规则 → 中风险（prompt：A股大多数股票应落在中风险档）");
    }
}

// ── 判定依据说明（人类可读，供 research-mgr 上下文与决策留痕）──
if degraded {
    reason = "风险画像数据缺失，按保守兜底档「高风险」输出（未做量化判定，degraded=true）";
} else {
    reason = join_arr(matched_rules, "；");
}

// ── 输出 ──
#{
    // 下游唯一实质消费字段（portfolio-mgr 的 overall_risk_llm + 前端诊断面板）
    "category": level,
    // 保 llm-output-schemas.json 的 risk_output 形状（无事件级数据，留空数组）
    "risk_factors": [],
    // 决策留痕
    "matched_rules": matched_rules,
    "reason": reason,
    "degraded": degraded,
    "metrics": #{
        // ⚠️ 缺失 → null。哨兵值（-1.0 / 0.0）不得泄漏到输出：
        //    否则诊断面板会把 -1.0 读成"波动率就是 -1%"。
        "volatility_pct": maybe(has_vol, vol),
        "max_drawdown_pct": maybe(has_dd, dd),
        "sharpe": maybe(has_sharpe, sharpe),
        "roe_ttm_pct": maybe(has_roe, roe),
        "gross_margin_pct": maybe(has_gm, gm),
        "debt_ratio_pct": maybe(has_debt, debt),
        "revenue_growth_yoy_pct": maybe(has_growth, growth)
    },
    "warnings": warnings
}
