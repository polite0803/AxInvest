// data-quality.rhai — 数据质量确定性评分（替代原 LLM Agent，P1-4 修复）
//
// 输入：每个 stock-analyst 的 verdict map（来自 content.verdict）
// 输出：{grade: "A"|"B"|"C"|"D"|"F", score: 0-100, summary: String}
//
// 分析师缩写对照（当前 DAG 10 个分析师）:
//   mk   → a-market-analyst (market-analyst)
//   sent → a-sentiment (sentiment-analyst)
//   news → a-news (news-analyst)
//   fund → a-fundamentals (fundamentals-analyst)
//   pol  → a-policy (policy-analyst)
//   hm   → a-hot-money (hot-money-tracker)
//   lk   → a-lockup (lockup-watcher)
//   res  → a-research (research-analyst)
//   sec  → a-sector (sector-analyst)
//   cat  → a-catalyst (catalyst-analyst)
//
// 生命周期维护:
//   - DAG 中新增 stock-analyst → 在此添加一行 {abbr}_verdict
//     并同步更新下方的 n 值和分析师列表。
//
// P1 修复（2026-07-23）:
//   - input_mapping 从 {id}.content.verdict.confidence 改为 {id}.content.verdict
//   - 新增 extract_conf() 从 verdict map 中提取 confidence
//
// P1-B3 升级（2026-07-24）:
//   - 算法化 report_quality_score（60% 程序化，基于报告字数+关键词覆盖+占位符检测）
//   - 算法化 tool_credibility_score（基于已有 avg_conf + gap_count；good_count 惩罚已于
//     2026-09-24 移除，理由见 tool_credibility 上方注释）
//   - 综合 score = report_quality × 0.45 + tool_credibility × 0.35 + factor_completeness × 0.20
//     （V42 调整: 加入因子完整度维度，权重重新分配为 45/35/20）
//     ⚠ 该权重已被 V67 取代：现行公式为 0.35 / 0.35 / 0.30
//     （本文件下方 `let score = ...`，理由见其上方注释）。此处保留 V42 原值仅作历史记录。
//   - t-scoring/t-valuation/t-risk 的 credibility 字段待 P3 接入后再升级 tool_credibility

// ── 辅助函数（供因子完整度评估使用）──
// present: 检查变量是否存在且非空
fn present(x) { type_of(x) != "()" }

// safe_parse: 安全解析 JSON 字符串，失败返回 ()
fn safe_parse(raw) {
    if !present(raw) { return (); }
    if type_of(raw) != "string" || raw == "" { return (); }
    try {
        json_parse(raw)
    } catch (err) {
        ()
    }
}

// extract_conf: 从 verdict map 中提取 confidence 数值
// 支持四种格式：
//   扁平: {"verdict":"看多", "confidence":70, ...}
//   嵌套: {"report":"...", "verdict":{"confidence":85, ...}}
//   JSON 字符串: 上述任一格式的字符串形式（V68 修复(2026-09-10)：
//     resolve_var_path 终值不 auto-parse，若上游映射到 .content 终值，
//     rhai 收到的是 JSON 字符串而非 map，原实现恒返回 -1.0 导致
//     催化剂分析师被恒判 missing。现先 safe_parse 再走 map 分支）
//   判定器形态: {"category":"支持", "node_id":"j-market", ...}（2026-09-24 新增，
//     判据是 `node_id` 而非 `category` —— 见下方分支）
// Rhai 1.25 64-bit: type_of(i64) == "i64"，不是 "int"
fn extract_conf(v) {
    let m = if type_of(v) == "string" { safe_parse(v) } else { v };
    if type_of(m) == "map" {
        let c = m["confidence"];
        if type_of(c) == "f64" { return c; }
        if type_of(c) == "i64" || type_of(c) == "int" { return c.to_float(); }
        // 嵌套格式 fallback: m.verdict.confidence
        let inner = m["verdict"];
        if type_of(inner) == "map" {
            let ic = inner["confidence"];
            if type_of(ic) == "f64" { return ic; }
            if type_of(ic) == "i64" || type_of(ic) == "int" { return ic.to_float(); }
        }
        // ── 2026-09-24 增强（服务**两条链**，非快速链分支）──
        // 快速链的 10 个维度判定由 `llmClassifier`（`j-*` 节点）产出，形态是
        // `{category, confidence, model, provider, node_id}`：**没有 `verdict` 字段**，
        // 且未配 `confidence_threshold` 时 `confidence` 恒为 `null`
        // （`llm_classifier_executor.rs` 的 `resolved_confidence = None` 通路）。
        // 不做兼容 ⇒ 快速链 30 路分析师侧输入全判 missing：gap_count = 10
        // ⇒ `tool_credibility = clamp(30 - 40, 0, 100) = 0`、10 条诊断全 `missing`、
        // grade 被钉死在 F —— 这**不是**「快速链取数失败」，是本节点读不懂判定器形态。
        // 判据：存在**非空** `category` 字符串 ⇒ 有判定产出、无自评置信度 ⇒ 记 0.0
        // （「低置信」档，不是「缺失」档）。取 0.0 而非某个正的基准分，是为了不与
        // 原链「分析师自评置信度」的语义混淆：0.0 会让 `tool_credibility` 走
        // `avg_conf <= 0 ⇒ base = 30` 的低基准分支（不虚高、不虚低、不产生 gap 罚分）。
        // ⚠ 对原链**机械不可达**（H2 回归判据）：判定器形态由 `node_id` 唯一标识 ——
        //   `llm_classifier_executor.rs` 的成功与降级两条通路**都**写入 `node_id`
        //   （`node.base_id()`），而原链 10 路 `*_verdict` 是 AgentNode 的 verdict map，
        //   受 `config/llm-output-schemas.json` 约束（`verdict` / `bull_score` /
        //   `bear_score` / `confidence` …），**不含** `node_id`。故本分支只在判定器形态下命中；
        //   不用「有没有 category」单独做判据 —— 那是形态相似性，不是来源判据。
        let cat = m["category"];
        if type_of(m["node_id"]) == "string" && type_of(cat) == "string" && cat != "" {
            return 0.0;
        }
        -1.0
    } else { -1.0 }
}

// extract_dir: 从 verdict map 中提取方向分值
// 返回 -1（看空/偏空）/ 0（中性/未知）/ +1（看多/偏多）
// 支持三种格式（V68 同 extract_conf：JSON 字符串先 safe_parse）：
//   扁平: {"verdict":"看多", ...}
//   嵌套: {"report":"...", "verdict":{"verdict":"偏空"}} 或 {"direction":"多"}
fn extract_dir(v) {
    let m = if type_of(v) == "string" { safe_parse(v) } else { v };
    if type_of(m) != "map" { return 0; }
    let d = m["verdict"];
    if type_of(d) == "string" {
        if d.contains("看多") || d.contains("偏多") { return 1; }
        if d.contains("看空") || d.contains("偏空") { return -1; }
        return 0;
    }
    // 嵌套格式: verdict map 内再找 verdict/direction
    if type_of(d) == "map" {
        let inner = d["verdict"];
        if type_of(inner) == "string" {
            if inner.contains("看多") || inner.contains("偏多") { return 1; }
            if inner.contains("看空") || inner.contains("偏空") { return -1; }
        }
        let dir = d["direction"];
        if type_of(dir) == "string" {
            if dir.contains("多") { return 1; }
            if dir.contains("空") { return -1; }
        }
    }
    0
}

// extract_report: 从 *_report 字段提取字符串正文
// 输入可能是：string（直接字符串）、map（含 report 字段）、()（缺失）
// 返回 "" 表示无正文
fn extract_report(v) {
    if type_of(v) == "string" { return v; }
    if type_of(v) == "map" {
        let r = v["report"];
        if type_of(r) == "string" { return r; }
    }
    ""
}

// is_untrusted: 检测分析师 verdict 是否被标记为不可信
// strict_mode 降级时 agent_executor 在节点顶层注入 __untrusted=true,
// 但 verdict map 本身可能不含此字段（__untrusted 在 NodeOutput 顶层）。
// data-quality.rhai 的 input_mapping 只映射了 .content.verdict,未映射节点顶层的 __untrusted,
// 因此此处额外检查 verdict map 内的 __untrusted 字段（部分降级路径会内嵌）。
// 修复(2026-07-29): 原代码完全忽略 __untrusted 标记,导致 strict_mode 降级的分析师
//   被当成可信分析师参与评分,tool_credibility_score 虚高。
fn is_untrusted(v) {
    if type_of(v) == "map" {
        let flag = v["__untrusted"];
        if type_of(flag) == "bool" { return flag; }
    }
    false
}

// count_chars: 估算报告字数（中英文混合）
// P0 修复(2026-08-09): Rhai 默认 Engine（Engine::new() + register_common_functions，
// 未注册 StringPackage）下 String::replace 实际解析为返回 unit 的降级函数，
// 原实现 text.replace(" ", "") 链式调用 100% 报 "Function not found: replace"，
// 导致 count_chars 失败 → report_quality 失败 → 整个 data-quality 脚本执行失败
// → quality-gate 无 grade → 全部走 quality-fallback 保守路径（历史最大缺陷）。
// 现改为纯 len()（UTF-8 字节数，BasicPackage 内置）估算：
// 中文 3 字节/字符、英文 1 字节/字符，A 股报告以中文为主 → 平均按 2.0 字节/字符
// （比原 /2.5 更接近中英混合比例；纯英文从低估 60% 收窄到 50%，分段阈值
//  200/500 对 90% 中文报告依然准确）。不再依赖 replace/去空白。
fn count_chars(text) {
    if type_of(text) != "string" || text == "" { return 0; }
    let bytes = text.len();
    // bytes / 2.0（整数运算：×50/100，避免浮点转换）
    let approx = bytes * 50 / 100;
    if approx < 0 { 0 } else { approx }
}

// has_any: 检查 text 是否包含 keywords 数组中任一关键词
fn has_any(text, keywords) {
    if type_of(text) != "string" || text == "" { return false; }
    for i in 0..keywords.len() {
        if text.contains(keywords[i]) { return true; }
    }
    false
}

// count_keyword_groups: 统计报告覆盖的关键词组数（最多 5 组）
// 五组：趋势/估值/风险/建议/数据，每组命中任一关键词即 +1
fn count_keyword_groups(text) {
    if type_of(text) != "string" || text == "" { return 0; }
    let groups = 0;
    if has_any(text, ["趋势", "走势", "MACD", "RSI", "K线", "均线", "支撑", "阻力"]) { groups += 1; }
    if has_any(text, ["估值", "PE", "PB", "ROE", "市值", "NAV", "EPS"]) { groups += 1; }
    if has_any(text, ["风险", "下行", "回撤", "波动", "止损", "利空"]) { groups += 1; }
    if has_any(text, ["建议", "操作", "仓位", "买入", "卖出", "持有", "增持", "减持"]) { groups += 1; }
    if has_any(text, ["数据", "指标", "显示", "根据", "统计", "查询"]) { groups += 1; }
    groups
}

// placeholder_markers: 失败标记词表
// 2026-09-12 扩充：原表 12 个形态实测有 10.5% 假阴性（27/258 份报告含失败语但未识别，
// 占全部「真有工具缺口」报告的 25%），漏检形态按频次依次为：
//   为 null(9) / 返回空(8) / 数据不可用(4) / 无数据(3) / =null(2) /
//   均为空(2) / 获取失败(2) / 未能获取(2) / 空值(1)
// ⚠ 2026-09-21：本表**不再单独使用** —— 纯子串匹配存在 33% 假阳性（见下方
//   strip_verdict_blocks / soft_marker_suppression 的说明），命中判定统一走 placeholder_hits。
fn placeholder_markers() {
    return ["占位报告", "占位", "placeholder", "Placeholder", "PLACEHOLDER",
            "无法获取", "数据缺失", "未注入", "agentrunner 未注入",
            "DRY RUN", "TODO", "TBD",
            "为 null", "=null", "均为空", "返回空", "空值",
            "数据不可用", "无数据", "获取失败", "未能获取"];
}

// ── 2026-09-21 修复：词表判据的两类假阳性 ──
// 实证（AUDIT-analyst-low-confidence-2026-09-21.md §五，本轮 hm 节点）：
//   「本维度数据不可用，标注为技术停披而非无数据」——**一句话**同时命中
//   `数据不可用` + `无数据`，占 hm 全部 3 处命中里的 2 处。纯子串匹配不识别否定语气，
//   分析师越严谨地排除「这是数据缺口」，越容易被判成数据缺口 —— 判据方向反了。
// 两层修法（互补，缺一不可）：
//   ① strip_verdict_blocks —— 剥离报告尾部 `<!-- VERDICT ... -->` 注释块。
//      601698 轮 lk 的 `<!-- VERDICT: {"bear_points":[...,"质押数据缺失"]} -->` 是
//      **正常风险点罗列**却被计入命中，判据应只看正文。
//   ② soft_marker_suppression —— 对「易被澄清语气包裹」的软标记，若正文含其
//      **定向否定短语**则不计入。硬标记（无法获取/数据缺失/未注入/占位报告…）不设抑制：
//      本轮真缺口正是用「数据缺失」表达的，抑制它会误杀真缺口。
// ⚠ 不要对软标记做「整篇抑制」：本轮 hm 报告**同时**含真缺口（数据缺失，来自龙虎榜
//    席位）与澄清语，整篇抑制会连带抹掉真缺口 —— 抑制必须按 marker 定向。

// strip_verdict_blocks: 以首个 `<!-- VERDICT` 为界截断，只保留正文
fn strip_verdict_blocks(text) {
    if type_of(text) != "string" || text == "" { return ""; }
    if !text.contains("<!-- VERDICT") { return text; }
    let parts = text.split("<!-- VERDICT");
    if parts.len() == 0 { return text; }
    parts[0]
}

// soft_marker_suppression: 软标记 → 该标记的定向否定短语
// 命中短语即视为「分析师在澄清/排除该缺口」，该标记不计入失败标记数。
fn soft_marker_suppression() {
    #{
        "数据不可用": ["而非无数据", "非无数据", "而非数据不可用"],
        "无数据": ["而非无数据", "非无数据", "无数据源", "暂无数据", "该维度暂无"],
        "返回空": ["暂无数据", "无期权覆盖", "视为该维度暂无", "该维度暂无"],
        "空值": ["暂无数据", "无期权覆盖"],
        "均为空": ["暂无数据"],
    }
}

// placeholder_hits: 返回报告命中的失败标记清单（供逐节点归因 + 严重度计数）
// 折叠规则：命中「占位报告」等长词时不再重复计其子串「占位」，避免同一处双计。
// 2026-09-21：先剥离 VERDICT 块；软标记按定向否定短语抑制（见上方说明）。
fn placeholder_hits(text) {
    if type_of(text) != "string" || text == "" { return []; }
    let body = strip_verdict_blocks(text);
    let markers = placeholder_markers();
    let suppression = soft_marker_suppression();
    let hits = [];
    for i in 0..markers.len() {
        if !body.contains(markers[i]) { continue; }
        // 软标记：正文含该标记的定向否定短语 ⇒ 判为澄清，不计入
        let phrases = suppression[markers[i]];
        if type_of(phrases) == "array" && has_any(body, phrases) { continue; }
        hits.push(markers[i]);
    }
    let long_hit = false;
    for i in 0..hits.len() {
        if hits[i] == "占位报告" || hits[i] == "placeholder"
            || hits[i] == "Placeholder" || hits[i] == "PLACEHOLDER" {
            long_hit = true;
        }
    }
    if !long_hit { return hits; }
    let folded = [];
    for i in 0..hits.len() {
        if hits[i] != "占位" { folded.push(hits[i]); }
    }
    return folded;
}

// placeholder_occurrences: 失败标记在正文中的**实际出现次数**（同一词多次出现重复计）
// 2026-09-21 新增：`placeholder_hits` 按**词种**去重（同一 marker 出现几次都只计 1），
// 面板显示「N 处」与用户数原文的直觉不符（实证：hm 显示 3 处，原文实际出现 4+ 次）。
// 本函数给出出现次数的独立口径，供面板并列展示「词种数 / 出现次数」。
// 剥离 VERDICT 与软标记抑制规则与 placeholder_hits **完全一致** ——
// 否则两个口径会落在不同判定集上，越显示越矛盾。
fn placeholder_occurrences(text) {
    if type_of(text) != "string" || text == "" { return 0; }
    let body = strip_verdict_blocks(text);
    let markers = placeholder_markers();
    let suppression = soft_marker_suppression();
    // 折叠规则与 placeholder_hits 保持一致：长词命中 ⇒ 不再计其子串「占位」
    let long_hit = body.contains("占位报告") || body.contains("placeholder")
        || body.contains("Placeholder") || body.contains("PLACEHOLDER");
    let total = 0;
    for i in 0..markers.len() {
        let m = markers[i];
        if m == "占位" && long_hit { continue; }
        if !body.contains(m) { continue; }
        let phrases = suppression[m];
        if type_of(phrases) == "array" && has_any(body, phrases) { continue; }
        let n = body.split(m).len() - 1;
        if n > 0 { total += n; }
    }
    total
}

// has_placeholder: 检测报告是否含占位符/失败标记（布尔版，供 report_quality 扣分用）
fn has_placeholder(text) {
    placeholder_hits(text).len() > 0
}

// ── P2-3 新增(2026-08-09): Schema 驱动的深度字段校验 ──
// 从 verdict map 中提取 bull_score 数值
fn extract_bull_score(v) {
    if type_of(v) != "map" { return (); }
    let bs = v["bull_score"];
    if type_of(bs) == "f64" { return bs; }
    if type_of(bs) == "i64" || type_of(bs) == "int" { return bs.to_float(); }
    // 嵌套格式 fallback
    let inner = v["verdict"];
    if type_of(inner) == "map" {
        let ibs = inner["bull_score"];
        if type_of(ibs) == "f64" { return ibs; }
        if type_of(ibs) == "i64" || type_of(ibs) == "int" { return ibs.to_float(); }
    }
    ()
}

// 从 verdict map 中提取 bear_score 数值
fn extract_bear_score(v) {
    if type_of(v) != "map" { return (); }
    let bs = v["bear_score"];
    if type_of(bs) == "f64" { return bs; }
    if type_of(bs) == "i64" || type_of(bs) == "int" { return bs.to_float(); }
    // 嵌套格式 fallback
    let inner = v["verdict"];
    if type_of(inner) == "map" {
        let ibs = inner["bear_score"];
        if type_of(ibs) == "f64" { return ibs; }
        if type_of(ibs) == "i64" || type_of(ibs) == "int" { return ibs.to_float(); }
    }
    ()
}

// 从 verdict map 中提取 verdict 字符串（用于逻辑一致性检查）
fn extract_verdict_str(v) {
    if type_of(v) != "map" { return ""; }
    let d = v["verdict"];
    if type_of(d) == "string" { return d; }
    // 嵌套格式: verdict map 内再找 verdict
    if type_of(d) == "map" {
        let inner = d["verdict"];
        if type_of(inner) == "string" { return inner; }
    }
    ""
}

// validate_verdict_schema: 深度字段校验（纯检测，不改写上游数据）
// 检查：1) bull_score + bear_score 自洽性 2) verdict 与评分方向一致性 3) confidence 范围
// 返回问题列表（String 数组）
//
// 2026-09-14 修复（假修复）：原实现名为 validate_and_fix_verdict_schema，内部构造
//   fixed_data 做「自动归一化修复」并写回，但唯一调用方 validate_verdict_schema
//   只取 issues、fixed_data 被直接丢弃 —— 该「修复」从未生效，属
//   「声明的机制 ≠ 运行的机制」的出口不严型。且本节点输出为
//   grade/score/diagnostics，不含 verdict，修复结果本就无下游消费方。
//   归一化 bull/bear 属各分析师自身职责，检查员只报告、不改写。
fn validate_verdict_schema(verdict_data, analyst_name) {
    let issues = [];

    if type_of(verdict_data) != "map" {
        return issues;
    }

    // 1. bull_score + bear_score 自洽性检查
    let bs = extract_bull_score(verdict_data);
    let bes = extract_bear_score(verdict_data);
    
    if present(bs) && present(bes) {
        let sum = bs + bes;
        // 如果总和偏离 100 超过 10，触发告警
        if sum < 90.0 || sum > 110.0 {
            issues.push(`[${analyst_name}] 评分不自洽: bull_score + bear_score = ${sum} (应约为 100)`);
        } else if sum < 95.0 || sum > 105.0 {
            // 轻微偏离（5-10 之间），仅告警
            issues.push(`[${analyst_name}] 评分轻微偏离: bull_score + bear_score = ${sum} (建议约为 100)`);
        }
    }

    // 2. verdict 与评分方向一致性检查
    let v = extract_verdict_str(verdict_data);
    if v != "" && present(bs) && present(bes) {
        let is_bull = v.contains("看多") || v.contains("看涨") || v.contains("偏多");
        let is_bear = v.contains("看空") || v.contains("看跌") || v.contains("偏空");

        if is_bull && bs <= bes {
            issues.push(`[${analyst_name}] 逻辑矛盾: verdict='${v}' 但 bull_score(${bs}) <= bear_score(${bes})`);
        }
        if is_bear && bes <= bs {
            issues.push(`[${analyst_name}] 逻辑矛盾: verdict='${v}' 但 bear_score(${bes}) <= bull_score(${bs})`);
        }
    }

    // 3. confidence 范围检查 (0-100)
    let conf = extract_conf(verdict_data);
    if conf >= 0.0 {
        if conf > 100.0 {
            issues.push(`[${analyst_name}] 置信度越界: confidence=${conf} (超出 0-100)`);
        }
        // 极低置信度也告警
        if conf > 0.0 && conf < 10.0 {
            issues.push(`[${analyst_name}] 置信度极低: confidence=${conf}`);
        }
    }

    issues
}

// report_quality: 算法化报告质量分（0-100，满量程）
// 维度：字数(50) + 关键词覆盖(40) + 占位符扣分(-15) + 一致性奖励(10)
// 缺失报告（空字符串）返回 0
//
// 2026-09-14 修复（满量程）：原各维度上限合计仅 80（40+30+10），不是 100 ⇒
//   report_quality_avg ≤ 80 ⇒ score ≤ 80×0.35 + 100×0.35 + 100×0.30 = 93，
//   0.35 的权重被系统性稀释，A 级（≥85）只剩 8 分余量、近乎不可达。
//   更关键的是 V67 调整因子完整度权重时，论证用的是「报告满分 100」的错误前提
//   （100×0.45 + 100×0.35 + 50×0.20 = 90 判 A），据此得出的
//   「5/10 因子缺失仍可达 A 级」结论与实际不符。现补齐至满量程 100。
// S2(2026-09-26): 第 3 参 `suppress_ph` = 该维度处于 as-of **设计性降级**
//   （上游按当下语义约束返回空，非工具故障）⇒ 跳过占位符 -15 扣分。
//   判据来源见下方 asof_design 块与 PLAN-asof-replay-quality-attribution.md。
fn report_quality(text, conf, suppress_ph) {
    if type_of(text) != "string" || text == "" { return 0.0; }

    let chars = count_chars(text);
    // 字数评分（50 分）
    let length_score = if chars >= 500 { 50.0 }
        else if chars >= 200 { 30.0 }
        else if chars >= 50 { 10.0 }
        else { 0.0 };

    // 关键词覆盖评分（40 分，5 组 × 8 分）
    let kw_groups = count_keyword_groups(text);
    let kw_score = kw_groups.to_float() * 8.0;

    // 占位符扣分（-15）
    let placeholder_penalty = if !suppress_ph && has_placeholder(text) { -15.0 } else { 0.0 };

    // confidence 一致性奖励（10 分）
    // P3-2 修复(2026-08-09): extract_conf 缺失时返回 -1.0，原 conf < 30.0 分支会把
    // -1.0 误判为"低置信+短报告"给 +5 奖励（方向相反）。加 conf >= 0.0 前置条件。
    let consistency_bonus = 0.0;
    if type_of(conf) == "f64" && conf >= 0.0 {
        if conf >= 70.0 && chars >= 200 { consistency_bonus = 10.0; }
        else if conf < 30.0 && chars < 50 { consistency_bonus = 5.0; }
    }

    let raw = length_score + kw_score + placeholder_penalty + consistency_bonus;
    clamp(raw, 0.0, 100.0)
}

// tool_credibility: 算法化工具可信度分（0-100）
// 当前指标：avg_conf 基础 + gap 惩罚
// P3 升级方向：接入 t-scoring/t-valuation/t-risk 的 credibility 字段后替换此实现
//
// ⚠ 2026-09-24 决策：**移除 good_count 惩罚**（原 good_count == 0 → -20，< n/2 → -15）。
//   它与 avg_conf 同源，构成二次计分：
//   ① good_count 的判据是「自评 confidence >= 50 且报告无失败标记」——`conf >= 50`
//      这一半已被 avg_conf 对同一批自评值求均值计过一次；「报告含失败标记」这一半已被
//      report_quality 的占位符扣分（每份 -15）计过，且 report_quality_avg 另有 0.35 权重。
//   ② 实证后果（511abde4，D 级）：tc = avg_conf 44.2 − good 15 − severe 冲突 20 −
//      字段校验 9 = 0.2 ⇒ 惩罚项合计 44 分全部叠加在同一批自评值 / 报告文本上，而真正
//      与「上游取数」直接相关的只有 avg_conf 与 gap 两项。
//   ③ 评分标尺被打穿：全库 46 条最优 composite 79.3、tc 上限 ~69 ⇒ 在实测分布
//      （rq ≤ 75.6、fc 常 100）下 0.35×75.6 + 0.35×69 + 0.30×100 = 80.4 < 85，
//      A 级**结构性不可达**。移除本项后 tc 上限回到 avg_conf 量级，A 级重新可达 ——
//      这是修标尺而非放宽标准：avg_conf 仍按原始自评值线性入分，gap 阶梯一格未动。
//   若将来要让「多数分析师偏低」成为独立信号，正确做法是**替换** avg_conf 口径
//   （如改用中位数 / 最低 3 位均值），而不是与 avg_conf 叠加。
//
// ⚠ 2026-09-21 决策：degraded_count（报告含失败标记、但自评 confidence >= 50）**不进入**
//   惩罚项，这是有意为之，不要「顺手补上」——
//   ① 与 avg_conf 同源：两者都从分析师**自评 confidence** 派生，惩罚 degraded 等于对
//      同一信号二次计分；
//   ② 反事实反证（AUDIT-analyst-low-confidence-2026-09-21.md §八.2）：剔除本轮 4 个
//      degraded 节点后 avg_conf 反而**上升**（64.5 → 68.3）⇒ 两者负相关，按 avg_conf
//      扣过的部分再扣一轮即双重扣分；
//   ③ 语义不同：本函数衡量「上游工具可信度」，degraded 描述的是「分析师措辞/自评偏低」，
//      该信号已通过 report_quality 的占位符惩罚与 warnings 体现。
//   若未来要让它影响评分，正确做法是**替换** avg_conf 惩罚口径（二选一），而非叠加。
fn tool_credibility(avg_conf, gap_count) {
    let base = if avg_conf > 0.0 { avg_conf } else { 30.0 };
    let penalty = 0.0;
    // gap 惩罚（按 gap_count 阶梯扣分，对齐原 V1 严格性）
    if gap_count >= 4 { penalty += 40.0; }
    else if gap_count >= 3 { penalty += 30.0; }
    else if gap_count >= 2 { penalty += 20.0; }
    else if gap_count >= 1 { penalty += 10.0; }
    clamp(base - penalty, 0.0, 100.0)
}

// ── 分析师 confidence 解析 ──
let mk_c = extract_conf(mk_verdict);
let sent_c = extract_conf(sent_verdict);
let news_c = extract_conf(news_verdict);
let fund_c = extract_conf(fund_verdict);
let pol_c = extract_conf(pol_verdict);
let hm_c = extract_conf(hm_verdict);
let lk_c = extract_conf(lk_verdict);
let res_c = extract_conf(res_verdict);
let sec_c = extract_conf(sec_verdict);
let cat_c = extract_conf(cat_verdict);

// ── P1-B3: 分析师报告正文解析（用于算法化 report_quality_score）──
let mk_r = extract_report(mk_report);
let sent_r = extract_report(sent_report);
let news_r = extract_report(news_report);
let fund_r = extract_report(fund_report);
let pol_r = extract_report(pol_report);
let hm_r = extract_report(hm_report);
let lk_r = extract_report(lk_report);
let res_r = extract_report(res_report);
let sec_r = extract_report(sec_report);
let cat_r = extract_report(cat_report);

// ── 2026-09-12: 报告文本失败标记检测（路径 A，客观证据）──
// 与路径 B（diag_for 消费的 LLM 自评 confidence）独立，此前两者全程无交叉：
//   has_placeholder 只影响 report_quality 扣分 + 一句「有占位符」告警（10 节点 OR 成一个布尔），
//   diag_for 只看 confidence ⇒ 603353 实证：失败标记最多 5 处的 hm(cf=55) 显示「正常」，
//   而标记为 0、数据齐全的 sec(cf=45) 被判「低置信」。
// 现按节点分别计算命中数，注入 diag_for 作为 status 的第二证据源。
let mk_ph = placeholder_hits(mk_r);
let sent_ph = placeholder_hits(sent_r);
let news_ph = placeholder_hits(news_r);
let fund_ph = placeholder_hits(fund_r);
let pol_ph = placeholder_hits(pol_r);
let hm_ph = placeholder_hits(hm_r);
let lk_ph = placeholder_hits(lk_r);
let res_ph = placeholder_hits(res_r);
let sec_ph = placeholder_hits(sec_r);
let cat_ph = placeholder_hits(cat_r);
// 2026-09-21: 与 *_ph 同源的实际出现次数（*_ph 是词种数）—— 供面板并列展示
let mk_po = placeholder_occurrences(mk_r);
let sent_po = placeholder_occurrences(sent_r);
let news_po = placeholder_occurrences(news_r);
let fund_po = placeholder_occurrences(fund_r);
let pol_po = placeholder_occurrences(pol_r);
let hm_po = placeholder_occurrences(hm_r);
let lk_po = placeholder_occurrences(lk_r);
let res_po = placeholder_occurrences(res_r);
let sec_po = placeholder_occurrences(sec_r);
let cat_po = placeholder_occurrences(cat_r);

// ── S2(2026-09-26): as-of 回放「设计性降级」豁免（PLAN-asof-replay-quality-attribution.md）──
// 回放中 搜索/快讯/新闻 等返回空是**设计语义**（astock-data as_of 决策矩阵：
// 搜索是当下语义 ⇒ record_degradation 留痕并返回空），不是工具故障。
// 此前这些设计性缺口与真故障同样被失败标记词表扣分 ⇒ 实证（300642
// as_of=2026-09-22）：tool_credibility 27、综合 C 级 —— 回放评分既不可与
// live 横比，还以「工具不可信」的错误归因污染反思链。
// 豁免口径：宿主函数 `pm_asof_degraded_methods()` 返回本次回放已降级的
// vendor 方法清单，经下方 method→维度映射，命中的分析师维度其失败标记
// **不进扣分与失败清单**，改记入 `asof_designed_dims` 单独展示。
// ⚠ truncate_*（按截止日正常截断，数据仍在）刻意**不在映射表** —— 那不是缺口。
// ⚠ 映射表按 `astock-data/src/lib.rs` 的 record_degradation 调用点维护；
//   新增降级方法时若不同步此表，该维度只是**不豁免**（保守方向，不会错扣）。
fn asof_method_dims() {
    #{
        "get_news": ["news"],
        "search_news": ["news", "pol", "res", "sec", "cat"],
        "get_policy_news": ["pol"],
        "get_research_reports": ["res"],
        "get_consensus_eps": ["res", "fund"],
        "get_cls_flash": ["cat", "sent"],
        "get_social_sentiment": ["sent"],
        "get_margin_data": ["sent", "hm"],
        "get_north_bound_holding": ["sent", "fund"],
        "get_money_flow": ["fund", "hm"],
        "get_hot_stocks": ["hm"],
        "get_market_dragon_tiger": ["hm", "mk"],
        "get_industry_ranking": ["sec"],
        "get_sector_info": ["sec"],
        "get_concept_blocks": ["sec"],
        "search_concept_boards": ["sec"],
        "get_concept_board_members": ["sec"],
        "search_stock": ["cat"],
        "get_pledge_data": ["lk"],
        "get_peers": ["fund", "sec", "cat"],
        "get_option_pcr": ["sent"],
        "get_financials": ["fund"],
        "get_quote": ["mk"],
        "get_index_quotes": ["mk"],
    }
}
let asof_methods_raw = pm_asof_degraded_methods();
// 防御式解析：宿主缺失/替身返回 unit 时按「无降级」处理（live 语义）
let asof_parsed = if asof_methods_raw == "[]" || asof_methods_raw == "" { [] } else { json_parse(asof_methods_raw) };
let asof_methods = if type_of(asof_parsed) == "array" { asof_parsed } else { [] };
let asof_map = asof_method_dims();
let asof_design = #{};   // 维度缩写 → true 表示该维度处于设计性降级
for i in 0..asof_methods.len() {
    let dims = asof_map[asof_methods[i]];
    if type_of(dims) == "array" {
        for j in 0..dims.len() { asof_design[dims[j]] = true; }
    }
}
let asof_sup_mk = asof_design["mk"] == true;
let asof_sup_sent = asof_design["sent"] == true;
let asof_sup_news = asof_design["news"] == true;
let asof_sup_fund = asof_design["fund"] == true;
let asof_sup_pol = asof_design["pol"] == true;
let asof_sup_hm = asof_design["hm"] == true;
let asof_sup_lk = asof_design["lk"] == true;
let asof_sup_res = asof_design["res"] == true;
let asof_sup_sec = asof_design["sec"] == true;
let asof_sup_cat = asof_design["cat"] == true;
// 评分口径（_e = effective）：被豁免维度的失败标记不进任何扣分/清单
let mk_ph_e = if asof_sup_mk { [] } else { mk_ph };
let sent_ph_e = if asof_sup_sent { [] } else { sent_ph };
let news_ph_e = if asof_sup_news { [] } else { news_ph };
let fund_ph_e = if asof_sup_fund { [] } else { fund_ph };
let pol_ph_e = if asof_sup_pol { [] } else { pol_ph };
let hm_ph_e = if asof_sup_hm { [] } else { hm_ph };
let lk_ph_e = if asof_sup_lk { [] } else { lk_ph };
let res_ph_e = if asof_sup_res { [] } else { res_ph };
let sec_ph_e = if asof_sup_sec { [] } else { sec_ph };
let cat_ph_e = if asof_sup_cat { [] } else { cat_ph };
let mk_po_e = if asof_sup_mk { 0 } else { mk_po };
let sent_po_e = if asof_sup_sent { 0 } else { sent_po };
let news_po_e = if asof_sup_news { 0 } else { news_po };
let fund_po_e = if asof_sup_fund { 0 } else { fund_po };
let pol_po_e = if asof_sup_pol { 0 } else { pol_po };
let hm_po_e = if asof_sup_hm { 0 } else { hm_po };
let lk_po_e = if asof_sup_lk { 0 } else { lk_po };
let res_po_e = if asof_sup_res { 0 } else { res_po };
let sec_po_e = if asof_sup_sec { 0 } else { sec_po };
let cat_po_e = if asof_sup_cat { 0 } else { cat_po };

let placeholder_total_hits = mk_ph_e.len() + sent_ph_e.len() + news_ph_e.len() + fund_ph_e.len()
    + pol_ph_e.len() + hm_ph_e.len() + lk_ph_e.len() + res_ph_e.len() + sec_ph_e.len() + cat_ph_e.len();
// 2026-09-21: 与 placeholder_total_hits 同判定集的**实际出现次数**合计（词种数 vs 出现次数）
let placeholder_total_occurrences = mk_po_e + sent_po_e + news_po_e + fund_po_e + pol_po_e
    + hm_po_e + lk_po_e + res_po_e + sec_po_e + cat_po_e;

// 单个分析师报告质量分（缺失报告 → 0 分）
let mk_q = report_quality(mk_r, mk_c, asof_sup_mk);
let sent_q = report_quality(sent_r, sent_c, asof_sup_sent);
let news_q = report_quality(news_r, news_c, asof_sup_news);
let fund_q = report_quality(fund_r, fund_c, asof_sup_fund);
let pol_q = report_quality(pol_r, pol_c, asof_sup_pol);
let hm_q = report_quality(hm_r, hm_c, asof_sup_hm);
let lk_q = report_quality(lk_r, lk_c, asof_sup_lk);
let res_q = report_quality(res_r, res_c, asof_sup_res);
let sec_q = report_quality(sec_r, sec_c, asof_sup_sec);
let cat_q = report_quality(cat_r, cat_c, asof_sup_cat);

// gap 推断：confidence < 0（字段缺失/节点失败）才算真正的数据缺口
let mk_g = mk_c < 0.0;
let sent_g = sent_c < 0.0;
let news_g = news_c < 0.0;
let fund_g = fund_c < 0.0;
let pol_g = pol_c < 0.0;
let hm_g = hm_c < 0.0;
let lk_g = lk_c < 0.0;
let res_g = res_c < 0.0;
let sec_g = sec_c < 0.0;
let cat_g = cat_c < 0.0;

// ── V67 修复(2026-07-29): 检测分析师 __untrusted 标记 ──
// strict_mode 降级时 agent_executor 在 NodeOutput 顶层注入 __untrusted=true,
// data-quality.rhai 的 input_mapping 映射了 {id}.__untrusted → {id}_untrusted 字段。
// 同时保留 verdict map 内 __untrusted 检测作为兜底(部分降级路径会内嵌到 content)。
// 原代码完全忽略此标记,导致不可信分析师被当成可信分析师参与评分。
// 现单独统计 untrusted_analyst_count,用于:
//   1. 从 good_count 中排除不可信分析师(避免虚高)
//   2. 在 tool_credibility 中增加 untrusted 惩罚
let mk_u = is_untrusted(mk_verdict) || (present(mk_untrusted) && mk_untrusted == true);
let sent_u = is_untrusted(sent_verdict) || (present(sent_untrusted) && sent_untrusted == true);
let news_u = is_untrusted(news_verdict) || (present(news_untrusted) && news_untrusted == true);
let fund_u = is_untrusted(fund_verdict) || (present(fund_untrusted) && fund_untrusted == true);
let pol_u = is_untrusted(pol_verdict) || (present(pol_untrusted) && pol_untrusted == true);
let hm_u = is_untrusted(hm_verdict) || (present(hm_untrusted) && hm_untrusted == true);
let lk_u = is_untrusted(lk_verdict) || (present(lk_untrusted) && lk_untrusted == true);
let res_u = is_untrusted(res_verdict) || (present(res_untrusted) && res_untrusted == true);
let sec_u = is_untrusted(sec_verdict) || (present(sec_untrusted) && sec_untrusted == true);
let cat_u = is_untrusted(cat_verdict) || (present(cat_untrusted) && cat_untrusted == true);
let untrusted_analyst_count = 0;
if mk_u { untrusted_analyst_count += 1; }
if sent_u { untrusted_analyst_count += 1; }
if news_u { untrusted_analyst_count += 1; }
if fund_u { untrusted_analyst_count += 1; }
if pol_u { untrusted_analyst_count += 1; }
if hm_u { untrusted_analyst_count += 1; }
if lk_u { untrusted_analyst_count += 1; }
if res_u { untrusted_analyst_count += 1; }
if sec_u { untrusted_analyst_count += 1; }
if cat_u { untrusted_analyst_count += 1; }

// P2-3 修复(2026-08-09): 原 n=10 硬编码，DAG 增删分析师需手动同步（失配时
// report_quality_avg 分母、good_count<n/2 阈值、low_count 全部失真）。
// 改为从 verdict 输入动态推导（非 () 即视为该分析师有输出），自动适配 DAG 实际分析师数。
fn count_present(v) { if type_of(v) != "()" { 1 } else { 0 } }
let n_int = count_present(mk_verdict) + count_present(sent_verdict) + count_present(news_verdict)
    + count_present(fund_verdict) + count_present(pol_verdict) + count_present(hm_verdict)
    + count_present(lk_verdict) + count_present(res_verdict) + count_present(sec_verdict)
    + count_present(cat_verdict);
let n = n_int.to_float();

let good_count = 0;       // confidence >= 50 且 非 untrusted 且 报告无失败标记
let degraded_count = 0;    // 有输出、confidence >= 50，但报告文本含失败标记（置信度虚高）
let valid_count = 0;      // confidence >= 0 且 非 untrusted
let gap_count = 0;        // 数据完全缺失（confidence < 0）或 untrusted
let total_conf = 0.0;     // 累加有效 confidence（排除 untrusted）

// 修复(2026-07-29): untrusted 分析师不计入 valid/good_count,计入 gap_count
//   理由: strict_mode 降级的 confidence=50 是中性兜底,不代表真实数据质量
// 2026-09-12 修复: good_count 原判据只有「自评 confidence >= 50」，与报告文本证据（路径 A）
//   完全脱钩 ⇒ 置信度虚高的节点（hm cf=55 但报告 5 处「无法获取/返回空」）被计为 good，
//   虚高 tool_credibility_score。现改为 **A ∪ B**：报告含失败标记即降级（degraded_count），
//   不再计入 good。degraded 归入 low 语义（low_count = n_int - good - gap 天然包含）。
if mk_u || mk_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += mk_c; if mk_c >= 50.0 { if mk_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if sent_u || sent_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += sent_c; if sent_c >= 50.0 { if sent_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if news_u || news_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += news_c; if news_c >= 50.0 { if news_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if fund_u || fund_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += fund_c; if fund_c >= 50.0 { if fund_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if pol_u || pol_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += pol_c; if pol_c >= 50.0 { if pol_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if hm_u || hm_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += hm_c; if hm_c >= 50.0 { if hm_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if lk_u || lk_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += lk_c; if lk_c >= 50.0 { if lk_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if res_u || res_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += res_c; if res_c >= 50.0 { if res_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if sec_u || sec_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += sec_c; if sec_c >= 50.0 { if sec_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }
if cat_u || cat_c < 0.0 { gap_count += 1; } else { valid_count += 1; total_conf += cat_c; if cat_c >= 50.0 { if cat_ph_e.len() > 0 { degraded_count += 1; } else { good_count += 1; } } }

let avg_conf = if valid_count > 0 { total_conf / valid_count.to_float() } else { 0.0 };

// ── P1-B3: 算法化双维度评分（按设计文档公式）──
// report_quality_score: 10 个分析师报告质量均值（缺失报告按 0 分拉低均值）
// tool_credibility_score: 基于已有 avg_conf + gap_count 指标算法化
// score = report × 0.6 + tool × 0.4（替代旧版 valid/good_count 占比 + 阶梯惩罚）
let report_quality_sum = mk_q + sent_q + news_q + fund_q + pol_q + hm_q + lk_q + res_q + sec_q + cat_q;
// P2-3 修复(2026-08-09): n 动态化后需防除零（全部 verdict 缺失时 n==0）
let report_quality_avg = if n > 0.0 { report_quality_sum / n } else { 0.0 };
let tool_credibility_score = tool_credibility(avg_conf, gap_count);

// ── P2-2 修复(2026-08-09): 分析师方向矛盾检测（升级为加权模式） ──
// 该检测源自原 data-quality-inspector 专家方法论第 3 条（"分析师结论是否相互矛盾"），
// 但原 Rhai 版只用了 confidence，verdict 方向字段被完全丢弃。
// 2026-09-14: 上述专家已注销注册，其职责由本 CodeNode 全权承担（见
// stock_analysis_setup/mod.rs 的 RETIRED_EXPERT_IDS）；该 .md 文件作为方法论文档保留，
// 不再是工作流专家。
// 现解析 10 个分析师的方向分值（看多/偏多→+1，中性→0，看空/偏空→-1），
// untrusted 分析师方向不计（数据不可信）。
// V2 升级(2026-08-09): 从简单计数升级为加权评估（结合 confidence 权重），
// 只有当两边都有显著的高置信度观点时，才判定为严重冲突。
let mk_dir = if mk_u { 0 } else { extract_dir(mk_verdict) };
let sent_dir = if sent_u { 0 } else { extract_dir(sent_verdict) };
let news_dir = if news_u { 0 } else { extract_dir(news_verdict) };
let fund_dir = if fund_u { 0 } else { extract_dir(fund_verdict) };
let pol_dir = if pol_u { 0 } else { extract_dir(pol_verdict) };
let hm_dir = if hm_u { 0 } else { extract_dir(hm_verdict) };
let lk_dir = if lk_u { 0 } else { extract_dir(lk_verdict) };
let res_dir = if res_u { 0 } else { extract_dir(res_verdict) };
let sec_dir = if sec_u { 0 } else { extract_dir(sec_verdict) };
let cat_dir = if cat_u { 0 } else { extract_dir(cat_verdict) };

// 基础计数（保留用于展示）
let bull_dir_count = 0;
let bear_dir_count = 0;
if mk_dir > 0 { bull_dir_count += 1; } else if mk_dir < 0 { bear_dir_count += 1; }
if sent_dir > 0 { bull_dir_count += 1; } else if sent_dir < 0 { bear_dir_count += 1; }
if news_dir > 0 { bull_dir_count += 1; } else if news_dir < 0 { bear_dir_count += 1; }
if fund_dir > 0 { bull_dir_count += 1; } else if fund_dir < 0 { bear_dir_count += 1; }
if pol_dir > 0 { bull_dir_count += 1; } else if pol_dir < 0 { bear_dir_count += 1; }
if hm_dir > 0 { bull_dir_count += 1; } else if hm_dir < 0 { bear_dir_count += 1; }
if lk_dir > 0 { bull_dir_count += 1; } else if lk_dir < 0 { bear_dir_count += 1; }
if res_dir > 0 { bull_dir_count += 1; } else if res_dir < 0 { bear_dir_count += 1; }
if sec_dir > 0 { bull_dir_count += 1; } else if sec_dir < 0 { bear_dir_count += 1; }
if cat_dir > 0 { bull_dir_count += 1; } else if cat_dir < 0 { bear_dir_count += 1; }

// 加权方向冲突检测（结合 confidence 权重）
let weighted_bull_score = 0.0;
let weighted_bear_score = 0.0;
// mk
if !mk_u && mk_c > 0.0 {
    if mk_dir > 0 { weighted_bull_score += mk_c; }
    else if mk_dir < 0 { weighted_bear_score += mk_c; }
}
// sent
if !sent_u && sent_c > 0.0 {
    if sent_dir > 0 { weighted_bull_score += sent_c; }
    else if sent_dir < 0 { weighted_bear_score += sent_c; }
}
// news
if !news_u && news_c > 0.0 {
    if news_dir > 0 { weighted_bull_score += news_c; }
    else if news_dir < 0 { weighted_bear_score += news_c; }
}
// fund
if !fund_u && fund_c > 0.0 {
    if fund_dir > 0 { weighted_bull_score += fund_c; }
    else if fund_dir < 0 { weighted_bear_score += fund_c; }
}
// pol
if !pol_u && pol_c > 0.0 {
    if pol_dir > 0 { weighted_bull_score += pol_c; }
    else if pol_dir < 0 { weighted_bear_score += pol_c; }
}
// hm
if !hm_u && hm_c > 0.0 {
    if hm_dir > 0 { weighted_bull_score += hm_c; }
    else if hm_dir < 0 { weighted_bear_score += hm_c; }
}
// lk
if !lk_u && lk_c > 0.0 {
    if lk_dir > 0 { weighted_bull_score += lk_c; }
    else if lk_dir < 0 { weighted_bear_score += lk_c; }
}
// res
if !res_u && res_c > 0.0 {
    if res_dir > 0 { weighted_bull_score += res_c; }
    else if res_dir < 0 { weighted_bear_score += res_c; }
}
// sec
if !sec_u && sec_c > 0.0 {
    if sec_dir > 0 { weighted_bull_score += sec_c; }
    else if sec_dir < 0 { weighted_bear_score += sec_c; }
}
// cat
if !cat_u && cat_c > 0.0 {
    if cat_dir > 0 { weighted_bull_score += cat_c; }
    else if cat_dir < 0 { weighted_bear_score += cat_c; }
}

// 严重冲突判定：两边加权分都 >= 100
let severe_direction_conflict = weighted_bull_score >= 100.0 && weighted_bear_score >= 100.0;
// 一般冲突判定（保留基础计数逻辑作为兜底）
let basic_direction_conflict = bull_dir_count >= 2 && bear_dir_count >= 2;
let direction_conflict = severe_direction_conflict || basic_direction_conflict;
if direction_conflict {
    // 严重冲突扣更多分
    if severe_direction_conflict {
        tool_credibility_score = clamp(tool_credibility_score - 20.0, 0.0, 100.0);
    } else {
        tool_credibility_score = clamp(tool_credibility_score - 15.0, 0.0, 100.0);
    }
}

// ── P2-3 新增(2026-08-09): 深度字段校验（Schema Validation） ──
// 对每个分析师的 verdict 数据进行深度校验：
//   1. bull_score + bear_score 是否自洽（应约为 100）
//   2. verdict 字符串与评分方向是否逻辑一致
//   3. confidence 是否在合法范围内
// 所有问题收集到 field_validation_issues 数组，用于：
//   - 额外扣分（每个问题扣 3 分）
//   - 追加到 warnings 供用户查看
let field_validation_issues = [];
// 技术面
if !mk_u {
    let mk_issues = validate_verdict_schema(mk_verdict, "技术面");
    for i in 0..mk_issues.len() { field_validation_issues.push(mk_issues[i]); }
}
// 情绪面
if !sent_u {
    let sent_issues = validate_verdict_schema(sent_verdict, "情绪面");
    for i in 0..sent_issues.len() { field_validation_issues.push(sent_issues[i]); }
}
// 新闻面
if !news_u {
    let news_issues = validate_verdict_schema(news_verdict, "新闻面");
    for i in 0..news_issues.len() { field_validation_issues.push(news_issues[i]); }
}
// 基本面
if !fund_u {
    let fund_issues = validate_verdict_schema(fund_verdict, "基本面");
    for i in 0..fund_issues.len() { field_validation_issues.push(fund_issues[i]); }
}
// 政策面
if !pol_u {
    let pol_issues = validate_verdict_schema(pol_verdict, "政策面");
    for i in 0..pol_issues.len() { field_validation_issues.push(pol_issues[i]); }
}
// 资金面
if !hm_u {
    let hm_issues = validate_verdict_schema(hm_verdict, "资金面");
    for i in 0..hm_issues.len() { field_validation_issues.push(hm_issues[i]); }
}
// 解禁观察
if !lk_u {
    let lk_issues = validate_verdict_schema(lk_verdict, "解禁观察");
    for i in 0..lk_issues.len() { field_validation_issues.push(lk_issues[i]); }
}
// 研报
if !res_u {
    let res_issues = validate_verdict_schema(res_verdict, "研报");
    for i in 0..res_issues.len() { field_validation_issues.push(res_issues[i]); }
}
// 行业
if !sec_u {
    let sec_issues = validate_verdict_schema(sec_verdict, "行业");
    for i in 0..sec_issues.len() { field_validation_issues.push(sec_issues[i]); }
}
// 催化剂
if !cat_u {
    let cat_issues = validate_verdict_schema(cat_verdict, "催化剂");
    for i in 0..cat_issues.len() { field_validation_issues.push(cat_issues[i]); }
}

// 字段校验问题扣分（每个问题扣 3 分，最多扣 15 分）
let validation_penalty = field_validation_issues.len() * 3.0;
if validation_penalty > 15.0 { validation_penalty = 15.0; }
if validation_penalty > 0.0 {
    tool_credibility_score = clamp(tool_credibility_score - validation_penalty, 0.0, 100.0);
}

// ── 因子数据完整度（0.0 ~ 1.0）──
// 调用 Rust 函数 pm_compute_factor_completeness 评估 9 个因子的数据存在性。
// 因子缺失也是数据质量问题的一部分，纳入综合评分可避免「分析师报告写得好看但因子数据全缺」的虚高评分。
// P1-1 修复(2026-08-09): 注入的 money_flow/lockup_bundle/announcements 经 code_executor
// 的 resolve_var_path（auto_parse_value 解析 JSON 字符串为对象）+ json_value_to_dynamic
// 后是 **Rhai map**（ToolNode result 的 JSON 字符串已被解析为对象），
// 原 `type_of(x) == "string"` 前置判断恒为 false → 三个解析块永不执行
// → 主力净流入/股东增减持/公告数据三因子恒缺失（factor_completeness 上限 0.7、
// 综合评分系统性偏低约 9 分，且此前 camelCase 字段名修复改的是不可达分支）。
// 现改为 map/string 双兼容：map 直接提取，string 先 safe_parse。

// 提取主力净流入（camelCase 优先 + snake_case 兜底）
fn extract_main_net_inflow(raw) {
    if !present(raw) { return (); }
    let mf = if type_of(raw) == "string" { safe_parse(raw) } else { raw };
    if type_of(mf) == "map" {
        let v = mf["mainNetInflow"];
        if type_of(v) == "()" { v = mf["main_net_inflow"]; }
        if type_of(v) == "f64" { return v; }
        if type_of(v) == "i64" || type_of(v) == "int" { return v.to_float(); }
    }
    ()
}

// 提取股东增减持交易条数
fn extract_shareholder_trades_len(raw) {
    if !present(raw) { return (); }
    let lb = if type_of(raw) == "string" { safe_parse(raw) } else { raw };
    if type_of(lb) == "map" {
        let trades = lb["shareholder_trades"];
        if type_of(trades) == "array" { return trades.len(); }
    }
    ()
}

// 提取公告条数
fn extract_announcements_len(raw) {
    if !present(raw) { return (); }
    let ann = if type_of(raw) == "string" { safe_parse(raw) } else { raw };
    if type_of(ann) == "array" { return ann.len(); }
    ()
}

let mf_main_net_inflow = extract_main_net_inflow(money_flow);
let lb_shareholder_trades_len = extract_shareholder_trades_len(lockup_bundle);
let ann_count = extract_announcements_len(announcements);
// 2026-09-12: 移除 f7「交易方向」因子。根本原因：data-quality 是 trader 的**上游**
// （trader 的 dqi_score 由本节点提供），架构上不可能读到 trader 的输出。原实现的
// trader_direction input_mapping 因循环依赖被删边（见 seed_stock_analysis.rs 的
// 「修复循环依赖: 移除 trader → data-quality 边」注释）⇒ 该变量恒为 () ⇒ 因子恒缺失
// ⇒ 完整度上限被钉在 0.9、综合分恒定少 3 分，且 UI 恒显示「缺失因子：交易方向」
// 这条假告警。f7 的可用性由 portfolio-mgr（trader 下游）消费，不在本节点职责内。
// 现分母 10 → 9（与 portfolio_formula::compute_factor_completeness 同步）。
let factor_completeness = pm_compute_factor_completeness(
    total_score, consensus_score, catalyst_level,
    risk_volatility, valuation_dcf_upside,
    mf_main_net_inflow, lb_shareholder_trades_len,
    ann_count, pace_signal
);
let factor_completeness_pct = (factor_completeness * 100.0 * 10.0).round() / 10.0;

// ── 构建缺失因子列表（供前端 UI 展示）──
let missing_factors = [];
if !present(total_score) || total_score <= 0.0 { missing_factors.push("技术面评分"); }
if !present(consensus_score) || consensus_score <= 0.0 { missing_factors.push("共识评分"); }
if !present(catalyst_level) || catalyst_level == "" || catalyst_level == "无" { missing_factors.push("催化剂等级"); }
if !present(risk_volatility) || risk_volatility <= 0.0 { missing_factors.push("风险波动率"); }
if !present(valuation_dcf_upside) { missing_factors.push("估值上行空间"); }
// 2026-09-12: 「交易方向」判定项已删除（f7 因子整体移除，见上方说明）。
// 该变量在此处恒为 ()，保留只会让 UI 永久显示一条无法消除的假告警。
// P2-1(2026-08-09): 与 Rust compute_factor_completeness 口径对齐——主力净流入=0 视为中性无信号（缺失）
if !present(mf_main_net_inflow) || mf_main_net_inflow == 0.0 { missing_factors.push("主力净流入"); }
if !present(lb_shareholder_trades_len) || lb_shareholder_trades_len <= 0 { missing_factors.push("股东增减持"); }
if !present(ann_count) || ann_count <= 0 { missing_factors.push("公告数据"); }
// P2-1(2026-08-09): PACE 信号需在 [-1,1] 值域内才算有效
if !present(pace_signal) || pace_signal < -1.0 || pace_signal > 1.0 { missing_factors.push("PACE情绪"); }

// ── 2026-09-21: 上游取数缺口（**只告警，不扣分**）──
// 与 `missing_factors` 的**根本区别**（故不可合并成一张表）：
//   `missing_factors` 判的是**本节点直接消费的因子**是否存在，其长度与
//   `pm_compute_factor_completeness` 的分母口径绑死（Rust 侧 9 项 ↔ 本文件 9 项）；
//   本列表判的是**上游节点自己有没有取到数**，是「关于数据的元数据」，不是因子本身。
//   ⇒ 若并进 missing_factors，会出现「列表长度 ≠ 公式分母」的不一致（本仓有门禁守它）。
//
// 病灶（审计 §6.10.5 已精确到行）：原判据
//   `if !present(valuation_dcf_upside) { missing_factors.push("估值上行空间"); }`
//   判的是**派生值是否存在**，而非**底层数据是否取到**。`valuation_dcf_upside`
//   在「锚是历史代理（`is_fallback_anchor=true`）」时**照样存在** ⇒ 该因子记 ✔；
//   而 `operating_cash_flow` 恒为 `None` 这件事**全链路零告警** ——
//   这正是 38 条样本 dqi 从不报警的机制。
//
// 判据用 `valuation_dcf_fcf_data_missing`（Rust 侧布尔量
//   ← `dcf.assumptions.fcf_data_missing`），**不**按 `basis` 文案判：
//   文案是给人看的诊断文本，一改判据就静默失效（同 P0-I 改用布尔量的理由）。
// ⚠️ 只把**缺数**计入缺口，**不**计「当期FCF≤0」：
//   后者是标的真实经营状态，不是我方取数失败（同 F2 的两态区分纪律，不可合并）。
// ⚠️ 本列表**不参与** `score` 公式、不进 `factor_completeness` —— 本轮只做「给用户看见」，
//   是否扣分 / 扣多少 / 是否进 grade 属独立设计（审计 §6.10.5 的未决部分）。
let upstream_data_gaps = [];
if present(valuation_dcf_fcf_data_missing) && valuation_dcf_fcf_data_missing == true {
    upstream_data_gaps.push("现金流量表数据缺失（DCF 锚定退化为近5年净利均值代理）");
}

// 综合评分公式：报告质量 35% + 工具可信度 35% + 因子完整度 30%
// V67 修复(2026-07-29): 因子完整度权重从 0.20 提高到 0.30。
//   原权重 0.20 时,5/10 因子缺失(factor=0.5)仍可达 A 级(score≥85):
//     100×0.45 + 100×0.35 + 50×0.20 = 45+35+10 = 90 → A 级
//   提高到 0.30 后,5/10 因子缺失:
//     100×0.35 + 100×0.35 + 50×0.30 = 35+35+15 = 85 → A 级下限(临界)
//   6/10 因子缺失(factor=0.4):
//     100×0.35 + 100×0.35 + 40×0.30 = 35+35+12 = 82 → B 级 ✓
//   这样"因子缺失仍评 A 级"的问题被缓解(需 ≤5 个因子缺失且报告+工具满分才临界 A)。
// 同时降低 report_quality 权重 0.45→0.35,因为报告写得好不等于数据质量好。
let score = report_quality_avg * 0.35 + tool_credibility_score * 0.35 + factor_completeness * 100.0 * 0.30;
score = clamp(score, 0.0, 100.0);
let final_score = (score * 10.0).round() / 10.0;
let report_quality_score_final = (report_quality_avg * 10.0).round() / 10.0;
let tool_credibility_score_final = (tool_credibility_score * 10.0).round() / 10.0;

let grade = if final_score >= 85.0 { "A" }
    else if final_score >= 65.0 { "B" }
    else if final_score >= 45.0 { "C" }
    else if final_score >= 25.0 { "D" }
    else { "F" };

// ── 诊断条目生成 ──
// P3-4 修复(2026-08-09): strict_mode 降级的分析师 confidence=50（中性兜底）原显示
// "normal"，与 untrusted 语义矛盾。新增 untrusted 参数，降级分析师显示 "untrusted"。
// 2026-09-21：新增第 7 个参数 `rq` —— 本节点**自己的**报告质量分（`report_quality(text, conf, suppress_ph)`）。
//   此前该值只被累加进全局 `report_quality_avg`，单个节点的得分被平均掉 ⇒
//   面板顶部只能拿到全局值，10 个分析师弹窗显示同一组数字（用户据此质问「是不是造假」）。
//   ⚠ 这是**事实量**，不是等级：禁止据此派生 per-node 字母等级 ——
//     那会再造出「两套同名等级」，正是 2026-09-14 才修掉的坑。
// 2026-09-21(P2-1)：新增第 8 个参数 `attr_note` —— 归因核对结论
//   （`attribution_note(lk_report, lk_tool_calls)` 的返回值，`""` = 无异常）。
//   刻意**不复用** `gap_reason` 的内部计算，也不新增结构化字段：结论统一由
//   `attribution_note` 产出后并入 `gap_reason`，复用**既有**展示通道
//   （弹窗「缺口原因」行 + DecisionBanner 逐节点表的「缺口原因」列），
//   避免为此新增字段 + 前端行 + 11 语言 key。
fn diag_for(c, name, expected, untrusted, ph_n, po_n, rq, attr_note) {
    let has_ph = ph_n > 0;
    // 2026-09-12: status 判定从「只看 LLM 自评 confidence」升级为 **A ∪ B**
    //   A = 报告文本失败标记（客观，placeholder_hits）
    //   B = analyst 自评 confidence（主观）
    // 修复前实证反例（603353）：hm 报告有 5 处「无法获取/返回空」而 cf=55 → 显示「正常」；
    //   sec 报告数据齐全（排名 29/30、板块 -3.26%、净流出 4.08 亿）而 cf=45 → 显示「低置信」。
    // 现 A 优先：报告含失败标记即至少 low（无论自评多少）。
    let status = if untrusted { "untrusted" }
        else if c < 0.0 { "missing" }
        else if has_ph { "low" }
        else if c < 50.0 { "low" }
        else { "normal" };
    // gap_reason 从「三选一免责罗列」升级为确定归因（原文案：
    //   "可能原因：上游工具数据不完整 / strict_mode 降级兜底 / 分析师对数据覆盖度不满意"
    //   —— 系统有能力真归因，却把判断责任抛给用户）。
    let reason = if untrusted {
        "strict_mode 降级（LLM 不可用/超时，confidence 为中性兜底，数据不可信）"
    } else if c < 0.0 {
        "节点未输出 VERDICT 或 confidence 字段缺失（可能原因：LLM 超时/截断/解析失败/strict_mode 降级未注入 confidence）"
    } else if has_ph && c < 50.0 {
        `确定归因：上游工具数据不完整 —— 报告含 ${ph_n} 处失败标记，且置信度自评仅 ${c}`
    } else if has_ph {
        `确定归因：上游工具数据不完整 —— 报告含 ${ph_n} 处失败标记（置信度自评 ${c} 偏高，未反映该缺口）`
    } else if c < 50.0 {
        "非数据缺口：报告无失败标记、字段齐全 —— 置信度自评低反映的是分析师对自身结论的不确定，而非数据缺失（confidence 语义与系统按「数据充分度」消费的口径不一致）"
    } else { "" };
    // P2-1(2026-09-21): 归因核对结论并入 gap_reason。
    //   ⚠ 两段都要保留：`reason` 是「数据为什么缺」，`attr_note` 是「报告对缺失的
    //   归因是否可信」。只留其一会让面板重新变成「有结论但看不出可信度」。
    //   `attr_note` 非字符串（函数返回 unit）时按空处理，避免把 `()` 渲染进面板。
    let attr_ok = type_of(attr_note) == "string" && attr_note != "";
    let reason_final = if !attr_ok { reason }
        else if reason == "" { attr_note }
        else { `${reason} ｜ ${attr_note}` };
    #{
        "name": name,
        "expected_data": expected,
        "confidence": c,
        "status": status,
        "gap_reason": reason_final,
        // ph_n = 命中**词种数**（同一词多次出现只计 1）；po_n = **实际出现次数**。
        // 2026-09-21：两个口径一并返回，面板可区分「N 种标记」与「共 M 次出现」，
        // 避免用户按原文出现次数核对时误判为计数错误。
        "placeholder_hits": ph_n,
        "placeholder_occurrences": po_n,
        // 本节点报告质量分（0-100，与全局 report_quality_avg 同源同口径）。
        // 2026-09-21 新增：使面板能显示「这个分析师自己的报告写得好不好」，
        //   与顶部的全局 grade/score 区分开（此前两者都是全局值，无法区分）。
        "report_quality": rq
    }
}

fn join_arr(arr, sep) {
    let s = "";
    for i in 0..arr.len() {
        if i > 0 { s += sep; }
        s += arr[i];
    }
    s
}

// ─────────────────────────────────────────────────────────────────────────────
// P2-1(2026-09-21): 伪归因检出 —— 报告所说的「工具被拒绝」有没有本轮事实支撑
//
// 实证（`AUDIT-pledge-attribution-2026-09-21.md`）：a-lockup 的报告原文写着
//   「**质押风险**：质押数据获取失败（**工具调用被拒绝**），无法评估平仓线距离…」
// 而本轮该节点的 `tool_calls_made` 里：
//   · **没有任何质押工具调用**（`get_stock_pledge_data` 全库历史 0 次）；
//   · 唯一 `is_error=true` 的是 `get_stock_margin_data`，原因是**两个并行分析师
//     节点撞了 200ms 最小间隔**（限流），不是权限。
// ⇒ 报告把一次**限流**（可重试的时序冲突）写成「工具调用被拒绝」，且把影响归到一个
//   **从未被调用**的维度上，把使用者的排查方向整体带偏。
//
// 判据**锚在可观测事实**（`tool_calls_made` 的 `is_error` 与工具名）：
//   措辞只用来判断「报告是否在做工具归因」，**归因真假完全由调用记录裁决** ——
//   这正是本判据相对于纯词表（`placeholder_markers`）的增量：纯词表抓得到
//   「有缺口」，抓不到「归因是编造的」。
//
// 两级结论，严重度不同，**不要合并**：
//   · 编造   —— 报告说工具被拒，而本轮**一条失败记录都没有**；
//   · 含糊   —— 确有失败记录，但报告**一个被拒工具名都没提** ⇒ 无法确认被拒的是
//               哪个工具 / 影响哪个维度，也无法排除归错维度（本轮实证形态）。
// ─────────────────────────────────────────────────────────────────────────────

/// 归因性短语：报告把「数据缺失」归因到「工具被拒绝 / 未授权」时用的措辞。
///
/// ⚠ 只用于判定「报告是否在做工具归因」，**不用**于判定归因真假（那由调用记录裁决）。
/// 含「工具调用失败」是刻意的：真失败通常伴随真记录，缺记录时它就属于编造。
fn attribution_markers() {
    return ["工具调用被拒绝", "工具调用失败", "工具调用被拒", "调用被拒绝",
            "权限被拒绝", "权限不足", "无权限调用", "未授权调用", "接口未授权",
            "工具未授权", "工具不可用", "接口不可用", "工具被拒绝"];
}

/// 失败调用摘要 → `#{ count, names, named_tools }`。
///
/// 字段名是 `is_error`（由 `agent_executor` 写入），**不是** `success` ——
/// 写错字段名会静默得到 0 条失败（判据恒绿），故此处注明。
fn rejection_summary(tool_calls) {
    let names = [];
    if present(tool_calls) && type_of(tool_calls) == "array" {
        for tc in tool_calls {
            if tc.is_error == true {
                let n = tc.tool;
                if present(n) && type_of(n) == "string" { names.push(n); }
            }
        }
    }
    #{ "count": names.len(), "names": join_arr(names, "、"), "named_tools": names }
}

/// 产出归因核对结论；`""` = 无异常，或**无可核对的数据**。
fn attribution_note(report, tool_calls) {
    // ── 守卫①：没有调用记录就**不判** ──
    // 旧快照缺字段、或该节点本就无工具时，`tool_calls` 为 Null/空数组。
    // 此时判「编造」会把「测不出」当成「有问题」（判据假阳性），故直接放行。
    if !present(tool_calls) || type_of(tool_calls) != "array" || tool_calls.len() == 0 {
        return "";
    }
    let text = if type_of(report) == "string" { report } else { "" };
    // ── 守卫②：报告没有在做工具归因 ⇒ 本判据不适用 ──
    if !has_any(text, attribution_markers()) {
        return "";
    }

    let rej = rejection_summary(tool_calls);
    if rej.count == 0 {
        // ⚠ 本条与下面那条都是**用户可见**文本（渲染进弹窗「缺口原因」行 + DecisionBanner）。
        //   Rhai 的模板字符串 **不支持 `\` 行续接**（它不是 C 风格转义上下文）：
        //   写成跨行会让 `\` 与行首缩进**逐字进入界面**。
        //   实证（`output/tmp-151b-rhai-probe.rs` 真执行打印）：
        //     `…**无一条失败** \` + 换行 + 16 空格 + `⇒ 该归因无事实依据…`
        //   ⇒ 下方刻意保持**长单行**，不要为了可读性折行。
        //   守护：`data_quality_placeholder_gate.rs::attribution_note_text_is_single_line`。
        return `⚠ 疑似编造归因：报告称工具被拒绝，但本轮 ${tool_calls.len()} 次工具调用**无一条失败** ⇒ 该归因无事实依据，不要据此推断数据缺口的原因`;
    }

    // 有失败记录：看报告是否点名了被拒的那个工具。
    let named = false;
    for n in rej.named_tools {
        if text.contains(n) { named = true; }
    }
    if named {
        return "";
    }
    return `⚠ 归因含糊：本轮 ${rej.count} 次调用失败（${rej.names}），但报告未点名被拒工具 ⇒ 无法确认被拒的是哪个工具、影响哪个维度；也可能把 A 工具的失败归到了 B 维度（实证形态：真被拒的是 get_stock_margin_data，报告却写成「质押数据获取失败」）`;
}

// ⚠ P2-1(2026-09-21)：第 8 个实参是归因核对结论（`attribution_note`）。
//   10 个调用点**必须全部补齐** —— Rhai 的实参个数不匹配是**运行期**错误（不是编译期），
//   漏一处会让 data-quality 节点整体失败，故新增/删除本函数参数时必须同步这 10 处。
//   正控：`output/tmp-151-rhai-arity.mjs` 逐点核对本块的实参个数。
let diagnostics = #{
    "mk": diag_for(mk_c, "技术面分析师", "K线形态、MACD/RSI、支撑阻力位、成交量（t-market-data）", mk_u, mk_ph_e.len(), mk_po_e, mk_q,
        attribution_note(mk_report, mk_tool_calls)),
    // 2026-09-21 修正归因（原值把用户引向 t-sentiment-data，而真实缺口在
    // t-dragon-tiger-data ⇒ 归因指向错误会误导排查方向，实证见
    // AUDIT-analyst-low-confidence-2026-09-21.md §八.1）：
    //   情绪面「席位数据缺失」与资金面「龙虎榜席位与北向数据缺失」同源于
    //   t-dragon-tiger-data（龙虎榜营业部明细，独立于 t-sentiment-data 的节点）。
    "sent": diag_for(sent_c, "情绪面分析师", "市场情绪、散户/机构态度（t-sentiment-data）；龙虎榜席位明细（t-dragon-tiger-data）", sent_u, sent_ph_e.len(), sent_po_e, sent_q,
        attribution_note(sent_report, sent_tool_calls)),
    "news": diag_for(news_c, "新闻面分析师", "新闻公告影响评估（t-news-data: get_stock_news）", news_u, news_ph_e.len(), news_po_e, news_q,
        attribution_note(news_report, news_tool_calls)),
    // 基本面/催化剂/行业的「同侪 PE/PB 全为 null」同源于 t-fundamentals-data 的
    // get_stock_peers（2026-09-21 已补 PE_TTM/PB_MRQ/市值/涨跌幅；ROE 仍未接入）。
    "fund": diag_for(fund_c, "基本面分析师", "PE/PB/ROE、营收利润、资产负债（t-fundamentals-data）；同侪估值（t-fundamentals-data: get_stock_peers）", fund_u, fund_ph_e.len(), fund_po_e, fund_q,
        attribution_note(fund_report, fund_tool_calls)),
    "pol": diag_for(pol_c, "政策面分析师", "宏观政策与行业政策影响（t-policy-data）", pol_u, pol_ph_e.len(), pol_po_e, pol_q,
        attribution_note(pol_report, pol_tool_calls)),
    "hm": diag_for(hm_c, "资金面分析师", "游资动向、主力资金净流入（t-hotmoney-data: get_stock_money_flow）；龙虎榜席位明细（t-dragon-tiger-data）与北向字段", hm_u, hm_ph_e.len(), hm_po_e, hm_q,
        attribution_note(hm_report, hm_tool_calls)),
    // 2026-09-21(v72) 修正：质押此前**从未接线**（无节点、无白名单授权），
    //   面板却在 expected_data 里写着「质押风险（t-lockup-data）」——**指明了错误的
    //   取数入口**，把使用者的排查方向一起带偏（该工具是解禁+增减持+大宗交易**三方**，
    //   结构上不含质押字段）。现补 `t-pledge-data` 节点 + `get_stock_pledge_data` 授权，
    //   此处同步为**真实来源**，避免「期望值自证」（拿声明当事实）继续误导。
    // P2-1：本节点正是伪归因的**实证现场**（报告写「工具调用被拒绝」而归因错维度）。
    "lk": diag_for(lk_c, "解禁观察员", "解禁减持、大宗交易（t-lockup-data）；股权质押（t-pledge-data）", lk_u, lk_ph_e.len(), lk_po_e, lk_q,
        attribution_note(lk_report, lk_tool_calls)),
    "res": diag_for(res_c, "研报分析师", "券商研报观点汇总、目标价（t-research-data）", res_u, res_ph_e.len(), res_po_e, res_q,
        attribution_note(res_report, res_tool_calls)),
    "sec": diag_for(sec_c, "行业分析师", "行业景气度、轮动分析（t-sector-data）；同侪估值（t-fundamentals-data: get_stock_peers）", sec_u, sec_ph_e.len(), sec_po_e, sec_q,
        attribution_note(sec_report, sec_tool_calls)),
    "cat": diag_for(cat_c, "催化剂分析师", "催化剂与叙事完整度、公告关键词（t-catalyst-data）；同侪估值锚（t-fundamentals-data: get_stock_peers）", cat_u, cat_ph_e.len(), cat_po_e, cat_q,
        attribution_note(cat_report, cat_tool_calls))
};

let missing_analysts = [];
let low_confidence_analysts = [];
// 2026-09-12: 口径与 gap_count / diagnostics.status 对齐
//   missing = untrusted || c < 0（原实现漏了 untrusted，与 gap_count 不一致）
//   low     = c < 50 || 报告含失败标记（原实现只看 confidence）
if mk_u || mk_c < 0.0 { missing_analysts.push("技术面"); } else if mk_c < 50.0 || mk_ph_e.len() > 0 { low_confidence_analysts.push("技术面"); }
if sent_u || sent_c < 0.0 { missing_analysts.push("情绪面"); } else if sent_c < 50.0 || sent_ph_e.len() > 0 { low_confidence_analysts.push("情绪面"); }
if news_u || news_c < 0.0 { missing_analysts.push("新闻面"); } else if news_c < 50.0 || news_ph_e.len() > 0 { low_confidence_analysts.push("新闻面"); }
if fund_u || fund_c < 0.0 { missing_analysts.push("基本面"); } else if fund_c < 50.0 || fund_ph_e.len() > 0 { low_confidence_analysts.push("基本面"); }
if pol_u || pol_c < 0.0 { missing_analysts.push("政策面"); } else if pol_c < 50.0 || pol_ph_e.len() > 0 { low_confidence_analysts.push("政策面"); }
if hm_u || hm_c < 0.0 { missing_analysts.push("资金面"); } else if hm_c < 50.0 || hm_ph_e.len() > 0 { low_confidence_analysts.push("资金面"); }
if lk_u || lk_c < 0.0 { missing_analysts.push("解禁观察"); } else if lk_c < 50.0 || lk_ph_e.len() > 0 { low_confidence_analysts.push("解禁观察"); }
if res_u || res_c < 0.0 { missing_analysts.push("研报"); } else if res_c < 50.0 || res_ph_e.len() > 0 { low_confidence_analysts.push("研报"); }
if sec_u || sec_c < 0.0 { missing_analysts.push("行业"); } else if sec_c < 50.0 || sec_ph_e.len() > 0 { low_confidence_analysts.push("行业"); }
if cat_u || cat_c < 0.0 { missing_analysts.push("催化剂"); } else if cat_c < 50.0 || cat_ph_e.len() > 0 { low_confidence_analysts.push("催化剂"); }

// 2026-09-12: 报告文本含失败标记的节点逐项清单（原实现把 10 节点 OR 成一个布尔，
// 前端只能看到「检测到占位符」一句话，无法定位是哪个分析师/工具出问题）。
let placeholder_nodes = [];      // 中文名，供 UI 展示
let placeholder_node_ids = [];   // 节点缩写（mk/sent/...），供定位 agent 节点
if mk_ph_e.len() > 0 { placeholder_nodes.push("技术面"); placeholder_node_ids.push("mk"); }
if sent_ph_e.len() > 0 { placeholder_nodes.push("情绪面"); placeholder_node_ids.push("sent"); }
if news_ph_e.len() > 0 { placeholder_nodes.push("新闻面"); placeholder_node_ids.push("news"); }
if fund_ph_e.len() > 0 { placeholder_nodes.push("基本面"); placeholder_node_ids.push("fund"); }
if pol_ph_e.len() > 0 { placeholder_nodes.push("政策面"); placeholder_node_ids.push("pol"); }
if hm_ph_e.len() > 0 { placeholder_nodes.push("资金面"); placeholder_node_ids.push("hm"); }
if lk_ph_e.len() > 0 { placeholder_nodes.push("解禁观察"); placeholder_node_ids.push("lk"); }
if res_ph_e.len() > 0 { placeholder_nodes.push("研报"); placeholder_node_ids.push("res"); }
if sec_ph_e.len() > 0 { placeholder_nodes.push("行业"); placeholder_node_ids.push("sec"); }
if cat_ph_e.len() > 0 { placeholder_nodes.push("催化剂"); placeholder_node_ids.push("cat"); }

let low_count = n_int - good_count - gap_count;

// ── P1-B3: warnings 数组（基于报告质量检测的问题列表）──
let warnings = [];
if gap_count >= 1 { warnings.push(`${gap_count} 个分析师数据缺失`); }
// 2026-09-21 新增：degraded 此前只出现在 summary 文本里，用户看不到「它为什么不扣分」。
// 显式给出原因，避免后续维护者误判为漏实现（决策依据见 tool_credibility 上方注释）。
if degraded_count >= 1 { warnings.push(`${degraded_count} 个分析师报告含失败标记但自评置信度偏高（已计入低置信；未额外扣分——该信号与 avg_conf 同源自评，叠加会双重计数）`); }
if untrusted_analyst_count >= 1 { warnings.push(`${untrusted_analyst_count} 个分析师 strict_mode 降级(__untrusted)`); }
// 2026-09-21: 上游取数缺口 —— 显式写「不影响评分」，避免与上面几条扣分告警混淆。
if upstream_data_gaps.len() > 0 {
    warnings.push(`上游取数缺口 ${upstream_data_gaps.len()} 项（仅提示，不影响因子完整度与综合评分）：${join_arr(upstream_data_gaps, "；")}`);
}
if avg_conf < 30.0 && avg_conf > 0.0 { warnings.push(`平均置信度过低 (${avg_conf})`); }
// P2-2 修复(2026-08-09): 方向冲突告警（升级为加权模式）
if direction_conflict {
    if severe_direction_conflict {
        warnings.push(`严重多空分歧：加权看多 ${weighted_bull_score.round()} vs 看空 ${weighted_bear_score.round()}（两边均有高置信度支持，工具可信度已扣 20 分）`);
    } else {
        warnings.push(`分析师结论方向冲突：${bull_dir_count} 个看多 vs ${bear_dir_count} 个看空（数据互相矛盾，工具可信度已扣 15 分）`);
    }
}
// P2-3 新增(2026-08-09): 深度字段校验告警
if field_validation_issues.len() > 0 {
    warnings.push(`深度字段校验发现 ${field_validation_issues.len()} 个问题（已扣 ${validation_penalty} 分）：`);
    for i in 0..field_validation_issues.len() {
        warnings.push(`  - ${field_validation_issues[i]}`);
    }
}
// 字数过低的报告（< 200 字符）
if mk_r != "" && count_chars(mk_r) < 200 { warnings.push("技术面报告字数不足 200"); }
if sent_r != "" && count_chars(sent_r) < 200 { warnings.push("情绪面报告字数不足 200"); }
if news_r != "" && count_chars(news_r) < 200 { warnings.push("新闻面报告字数不足 200"); }
if fund_r != "" && count_chars(fund_r) < 200 { warnings.push("基本面报告字数不足 200"); }
if pol_r != "" && count_chars(pol_r) < 200 { warnings.push("政策面报告字数不足 200"); }
if hm_r != "" && count_chars(hm_r) < 200 { warnings.push("资金面报告字数不足 200"); }
if lk_r != "" && count_chars(lk_r) < 200 { warnings.push("解禁观察报告字数不足 200"); }
if res_r != "" && count_chars(res_r) < 200 { warnings.push("研报报告字数不足 200"); }
if sec_r != "" && count_chars(sec_r) < 200 { warnings.push("行业报告字数不足 200"); }
if cat_r != "" && count_chars(cat_r) < 200 { warnings.push("催化剂报告字数不足 200"); }
// 占位符/失败标记检测
// 2026-09-12: 逐节点清单替代原「10 节点 OR 成一个布尔」——
// 原实现只能报「有问题」，无法回答「是哪个分析师/哪个工具」。
if placeholder_nodes.len() > 0 {
    warnings.push(`检测到占位符/失败标记：${join_arr(placeholder_nodes, "、")}（${placeholder_total_hits} 种标记，共出现 ${placeholder_total_occurrences} 次；节点 ${join_arr(placeholder_node_ids, "/")}）`);
}
// S2(2026-09-26): 回放设计性降级 —— 显式声明「这些维度没进扣分」，
// 避免后续维护者把豁免误读为漏扣分（同 degraded_count 的处置先例）。
let asof_designed_dims_cn = [];
if asof_design["mk"] == true { asof_designed_dims_cn.push("技术面"); }
if asof_design["sent"] == true { asof_designed_dims_cn.push("情绪面"); }
if asof_design["news"] == true { asof_designed_dims_cn.push("新闻面"); }
if asof_design["fund"] == true { asof_designed_dims_cn.push("基本面"); }
if asof_design["pol"] == true { asof_designed_dims_cn.push("政策面"); }
if asof_design["hm"] == true { asof_designed_dims_cn.push("资金面"); }
if asof_design["lk"] == true { asof_designed_dims_cn.push("解禁观察"); }
if asof_design["res"] == true { asof_designed_dims_cn.push("研报"); }
if asof_design["sec"] == true { asof_designed_dims_cn.push("行业"); }
if asof_design["cat"] == true { asof_designed_dims_cn.push("催化剂"); }
if asof_designed_dims_cn.len() > 0 {
    warnings.push(`as-of 回放：${join_arr(asof_designed_dims_cn, "、")} 上游按设计降级（当下语义约束，非工具故障），其失败标记未计入扣分与失败清单`);
}

let replay_prefix = if asof_methods.len() > 0 { "【as-of 回放】" } else { "" };
let summary_text = if grade == "F" || grade == "D" {
    let base = `${replay_prefix}数据质量 ${grade} 级（综合 ${final_score} = 报告 ${report_quality_score_final} × 0.35 + 工具 ${tool_credibility_score_final} × 0.35 + 因子完整度 ${factor_completeness_pct} × 0.30）：${gap_count} 个缺失，${low_count} 个低置信（其中 ${degraded_count} 个为报告含失败标记但自评偏高），${good_count} 个高置信，${untrusted_analyst_count} 个降级。`;
    let miss_part = if missing_analysts.len() > 0 { `缺失：${join_arr(missing_analysts, "、")}。` } else { "" };
    let low_part = if low_confidence_analysts.len() > 0 { `低置信：${join_arr(low_confidence_analysts, "、")}。` } else { "" };
    let ph_part = if placeholder_nodes.len() > 0 { `报告含失败标记：${join_arr(placeholder_nodes, "、")}（${placeholder_total_hits} 种标记、共出现 ${placeholder_total_occurrences} 次）。` } else { "" };
    let val_part = if field_validation_issues.len() > 0 { `字段校验问题：${field_validation_issues.len()} 个。` } else { "" };
    `${base}${miss_part}${low_part}${ph_part}${val_part}平均置信度 ${avg_conf}。建议检查对应分析师的上游工具数据源。`
} else {
    let val_part = if field_validation_issues.len() > 0 { `字段校验问题：${field_validation_issues.len()} 个；` } else { "" };
    let ph_part = if placeholder_nodes.len() > 0 { `报告含失败标记：${join_arr(placeholder_nodes, "、")}（${placeholder_total_hits} 种标记、共出现 ${placeholder_total_occurrences} 次）；` } else { "" };
    `${replay_prefix}数据质量 ${grade} 级（综合 ${final_score} = 报告 ${report_quality_score_final} × 0.35 + 工具 ${tool_credibility_score_final} × 0.35 + 因子完整度 ${factor_completeness_pct} × 0.30）：${good_count}/${n_int} 高置信度，${untrusted_analyst_count} 个降级，${ph_part}${val_part}平均置信度 ${avg_conf}。`
};

#{
    "grade": grade,
    "score": final_score,
    "report_quality_score": report_quality_score_final,
    "tool_credibility_score": tool_credibility_score_final,
    "factor_completeness_pct": factor_completeness_pct,
    "missing_factors": missing_factors,
    // 2026-09-21: 上游**取数**缺口（与 missing_factors 是两张表，见其定义处注释）。
    //   只告警不扣分 ⇒ 本字段出现不会改变 score / grade。
    "upstream_data_gaps": upstream_data_gaps,
    "gap_count": gap_count,
    "good_count": good_count,
    // 2026-09-12: 报告含失败标记但自评 confidence >= 50 的节点数（置信度虚高）
    "degraded_count": degraded_count,
    // 2026-09-12: 报告文本含失败标记的逐节点清单（中文名 / 节点缩写 / 命中总数）
    "placeholder_nodes": placeholder_nodes,
    "placeholder_node_ids": placeholder_node_ids,
    // S2(2026-09-26): as-of 回放设计性降级留痕 —— 豁免了哪些维度、上游降级了哪些
    // vendor 方法。live 运行恒为 false / []。前端只消费不重算。
    "asof_replay": asof_methods.len() > 0,
    "asof_designed_dims": asof_designed_dims_cn,
    "asof_degraded_methods": asof_methods,
    "placeholder_total_hits": placeholder_total_hits,
    // 2026-09-21: 与 placeholder_total_hits 同一判定集的实际出现次数（词种数 vs 出现次数）
    "placeholder_total_occurrences": placeholder_total_occurrences,
    "avg_confidence": (avg_conf * 10.0).round() / 10.0,
    "total_analysts": n,
    // P2-2(2026-08-09): 方向冲突检测结果（升级为加权模式）
    "direction_conflict": direction_conflict,
    "severe_direction_conflict": severe_direction_conflict,
    "bull_dir_count": bull_dir_count,
    "bear_dir_count": bear_dir_count,
    "weighted_bull_score": (weighted_bull_score * 10.0).round() / 10.0,
    "weighted_bear_score": (weighted_bear_score * 10.0).round() / 10.0,
    // P2-3(2026-08-09): 深度字段校验结果
    "field_validation_issues": field_validation_issues,
    "field_validation_issue_count": field_validation_issues.len(),
    "field_validation_penalty": (validation_penalty * 10.0).round() / 10.0,
    "diagnostics": diagnostics,
    "missing_analysts": missing_analysts,
    "low_confidence_analysts": low_confidence_analysts,
    "warnings": warnings,
    "summary": summary_text
}
