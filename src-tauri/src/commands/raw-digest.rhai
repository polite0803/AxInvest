// raw-digest.rhai — 快速链（stock-analysis-fast）专属「原始数据分维度摘要」脚本
//
// ⚠ 本脚本是快速链**专属的新脚本**，不是 `analyst-brief.rhai` 的副本、也不是它的
//   版本分支：原链的 `analyst-brief` 节点消费 10 份分析师 verdict map
//   （`bull_score` / `bear_score`），而快速链**没有分析师节点** —— Jev 判定链
//   直接读原始数据。两者的输入契约、输出契约、消费者完全不同，故独立实现；
//   原脚本逐字不动（H1：原链一行不改）。
//
// 挂载方式（见 `seed_stock_analysis.rs` 的 `apply_fast_chain_overrides`）：
//   派生图里 `analyst-brief` 这个 **node id / output_var 保持原名**，只把 `code`
//   换成本脚本、`input_mapping` 换成下方 20 条映射 ⇒ 段 B 的
//   `input_var = analyst-brief.result.<段>` 与段 F 的引用都无需改动。
//
// 输入契约（全部由 input_mapping 注入，形如 `<节点>.result.content`：
// ToolNode 的 content 是 JSON **字符串**，故统一先 `load()` 解析）：
//   14 个数据源：market_data / sentiment_data / news_data / fundamentals_data /
//     policy_data / hotmoney_data / lockup_data / research_data / sector_data /
//     catalyst_data / pledge_data / index_quotes / institutional_visits /
//     dragon_tiger_data
//   6 条算法腿：algo_scoring / algo_valuation / algo_valuation_band / algo_risk /
//     algo_scoring_week / algo_scoring_month
//
// 输出契约（map；engine 会把脚本返回值包进 `.result` ⇒ 下游读
// `analyst-brief.result.<键>`）：
//   market / sentiment / news / fundamentals / policy / hotmoney / lockup /
//   research / sector / catalyst / pledge / index_quotes / institutional_visits /
//   dragon_tiger / algo_scoring / algo_valuation / algo_valuation_band /
//   algo_risk / algo_scoring_week / algo_scoring_month
//   每个键都是一个**已裁剪的纯文本段**（不是 JSON），可直接作为 LLM 的上下文片段。
//
// 裁剪预算（单位=字符，逐段独立；合计约 16k，落在 32k 上下文预算内）：
//   原文类（news / policy / research / catalyst）各 1800，单条 300；
//   指标类 market 800 / sentiment 600 / fundamentals 1000 / hotmoney 700 /
//     lockup 700 / sector 700；
//   附加类 pledge / index_quotes / institutional_visits / dragon_tiger 各 400；
//   算法腿各 500。超预算按条目截断并在段内留下「…（其余条目已按预算裁剪）」标记。
//
// 降级纪律：任何一段缺数据（工具失败 / 未接线 / JSON 解析失败）只让该段显示
//   「（无数据）」或原始文本，**绝不影响其它段、绝不抛错**；节点的
//   `continue_on_fail = true` 进一步保证本节点不会阻断整条链。

fn present(x) { type_of(x) != "()" }

// trim 的 Rhai 语义（与既有先例 `demand-keywords.rhai:14` 同一坑，此处再记一次）：
// `ImmutableString::trim` 被注册为 **non-pure（`&mut self`）** ⇒ 它**原地修改接收者、
// 返回值恒为 `()`**。故 `let s = x.trim();` 拿到的是 `()`；若 x 是 scope 常量
// （code_executor 用 `push_constant` 注入输入），更会直接抛
// `Non-pure method 'trim' cannot be called on constant`。两种形态都会让后续
// `len()` / `json_parse()` 炸成 "Function not found"，或让整段静默降级成「（无数据）」。
// 正确形态：**先赋值到本地变量，再把 trim 当语句用**，之后读该变量本身。
fn trim_in_place(x) {
    let t = x;
    t.trim();
    t
}

// load: 统一入口 —— JSON 字符串 → map/array；已是 map/array 原样返回；
// 解析失败（如纯文本 content）返回原字符串本身，绝不返回 () 丢失数据。
// 幂等：可安全地对已解析值重复调用。
fn load(raw) {
    if !present(raw) { return (); }
    let t = type_of(raw);
    if t == "map" || t == "array" { return raw; }
    if t != "string" { return raw; }
    let s = trim_in_place(raw);
    if s == "" { return (); }
    // `json_parse` 解析失败是**静默返回 ()**，不是抛错 —— 故不能靠 try/catch 兜底，
    // 必须按返回值判定。
    let parsed = json_parse(s);
    if present(parsed) { parsed } else { s }
}

// clip: 按字符数裁剪（Rhai 的 len() 为字符数，非字节数）。
// 接收者的 trim 同样必须走 trim_in_place（实参可能是常量数组/字符串里的元素）。
fn clip(s, n) {
    if type_of(s) != "string" { return ""; }
    let t = trim_in_place(s);
    if t.len() <= n { return t; }
    t.sub_string(0, n) + "…"
}

// scalar_text: 只渲染标量；map/array 交给 render_nested 下钻，此处返回空串。
fn scalar_text(v) {
    if !present(v) { return ""; }
    let t = type_of(v);
    if t == "string" { return trim_in_place(v); }
    if t == "map" || t == "array" { return ""; }
    v.to_string()
}

// render_item: 渲染**数组里的一个元素**（一行）。优先按 priority 字段序，
// 命中为空时退化为「全部标量字段」。刻意不下钻嵌套容器 —— 保证单行长度可控。
fn render_item(item, priority, budget) {
    if !present(item) { return ""; }
    let t = type_of(item);
    if t == "string" { return clip(item, budget); }
    if t != "map" { return clip(scalar_text(item), budget); }
    let parts = [];
    for f in priority {
        let s = scalar_text(item[f]);
        if s != "" { parts.push(f + "=" + clip(s, 120)); }
    }
    if parts.is_empty() {
        // ⚠ Rhai 的 Map **不可迭代**（BasicIteratorPackage 只为 Array/Blob/String/range
        //   注册迭代器），`for (k, v) in map` 必抛 ErrorFor；按 keys() 遍历（同 pace-calc.rhai:117）。
        for k in item.keys() {
            let s = scalar_text(item[k]);
            if s != "" { parts.push(k + "=" + clip(s, 120)); }
        }
    }
    clip(join(parts, " | "), budget)
}

// render_nested: 渲染**对象里的一个字段值**，对 array / map 下钻一层（最多 6 个元素），
// 避免 `{lockup: [...], reduction: [...]}` 这类包裹对象把整段内容丢成空串。
fn render_nested(v, budget) {
    if !present(v) { return ""; }
    let t = type_of(v);
    if t == "array" {
        let parts = [];
        for item in v {
            if parts.len() >= 6 {
                parts.push("…");
                break;
            }
            parts.push(render_item(item, [], 150));
        }
        return clip(join(parts, " ; "), budget);
    }
    if t == "map" {
        let parts = [];
        for k in v.keys() {
            let s = render_nested(v[k], 150);
            if s != "" { parts.push(k + "=" + s); }
        }
        return clip(join(parts, " | "), budget);
    }
    clip(scalar_text(v), budget)
}

// render_list: 渲染列表类数据源（新闻 / 政策 / 研报 / 公告），每条一行，按预算截断。
fn render_list(raw, priority, budget) {
    let v = load(raw);
    if !present(v) { return "（无数据）"; }
    if type_of(v) == "string" { return clip(v, budget); }
    if type_of(v) != "array" { return render_map(v, priority, budget); }
    if v.is_empty() { return "（无数据）"; }
    let out = "";
    let cut = false;
    for item in v {
        if out.len() >= budget {
            cut = true;
            break;
        }
        let line = render_item(item, priority, 300);
        if line != "" { out += "- " + line + "\n"; }
    }
    if out == "" { return "（无数据）"; }
    if cut { out += "- …（其余条目已按预算裁剪）\n"; }
    clip(out, budget)
}

// render_map: 渲染对象类数据源（指标 / 算法腿输出）。优先按 priority 字段序，
// 全部未命中时退化为「逐字段 + 嵌套下钻」。
fn render_map(raw, priority, budget) {
    let v = load(raw);
    if !present(v) { return "（无数据）"; }
    let t = type_of(v);
    if t == "string" { return clip(v, budget); }
    if t == "array" { return render_list(v, priority, budget); }
    let parts = [];
    for f in priority {
        let s = render_nested(v[f], 200);
        if s != "" { parts.push(f + "=" + s); }
    }
    if parts.is_empty() {
        for k in v.keys() {
            let s = render_nested(v[k], 200);
            if s != "" { parts.push(k + "=" + s); }
        }
    }
    if parts.is_empty() { return "（无数据）"; }
    clip(join(parts, " | "), budget)
}

// ── 字段优先级表（按数据源 DTO 的语义字段名，camelCase）──
let P_NEWS = ["title", "summary", "publishTime", "source", "url"];
let P_REPORT = [
    "title",
    "institution",
    "rating",
    "targetPrice",
    "publishDate",
    "analyst",
    "summary",
];
let P_ANNOUNCE = ["title", "announceDate", "annType", "summary"];
let P_KLINE = ["date", "close", "pctChg", "open", "high", "low", "volume"];
let P_FLOW = ["date", "mainNetInflow", "largeNetInflow", "xlargeNetInflow", "history"];
let P_LOCKUP = ["date", "title", "ratio", "shares", "type", "announceDate"];
let P_SECTOR = ["name", "rank", "pctChg", "indexName", "total"];
let P_VISIT = ["date", "institution", "type", "title", "summary"];
let P_TIGER = ["date", "title", "reason", "netAmount", "buyAmount", "sellAmount", "seats"];
let P_PLEDGE = ["pledgeRatio", "title", "date", "holder", "shares"];

// ── 主逻辑：逐段渲染，任一段失败只影响该段 ──
let out = #{};

out["news"] = render_list(news_data, P_NEWS, 1800);
out["policy"] = render_list(policy_data, P_NEWS, 1800);
out["research"] = render_list(research_data, P_REPORT, 1800);
out["catalyst"] = render_list(catalyst_data, P_ANNOUNCE, 1800);

out["market"] = render_map(market_data, P_KLINE, 800);
out["sentiment"] = render_map(sentiment_data, P_NEWS, 600);
out["fundamentals"] = render_map(fundamentals_data, P_REPORT, 1000);
out["hotmoney"] = render_map(hotmoney_data, P_FLOW, 700);
out["lockup"] = render_map(lockup_data, P_LOCKUP, 700);
out["sector"] = render_map(sector_data, P_SECTOR, 700);

out["pledge"] = render_map(pledge_data, P_PLEDGE, 400);
out["index_quotes"] = render_map(index_quotes, [], 400);
out["institutional_visits"] = render_map(institutional_visits, P_VISIT, 400);
out["dragon_tiger"] = render_map(dragon_tiger_data, P_TIGER, 400);

out["algo_scoring"] = render_map(algo_scoring, [], 500);
out["algo_valuation"] = render_map(algo_valuation, [], 500);
out["algo_valuation_band"] = render_map(algo_valuation_band, [], 500);
out["algo_risk"] = render_map(algo_risk, [], 500);
out["algo_scoring_week"] = render_map(algo_scoring_week, [], 500);
out["algo_scoring_month"] = render_map(algo_scoring_month, [], 500);

out
