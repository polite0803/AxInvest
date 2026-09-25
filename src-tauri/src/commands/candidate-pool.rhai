// candidate-pool.rhai — 快速趋势智选 Phase D：候选代码池（确定性清洗）
//
// 归属：`serenity-screening-fast` 模板新增资产（原链无对应脚本）。
//
// 输入：
//   search_results — t-candidate-search.result（search_stock，JSON 文本 [{code,name,market}]）
//   scanner_brief  — c-scanner-brief.result（Phase B 的 map 输出）
// 输出：
//   {stocks:[{code,name,source}], labels:["300285 国瓷材料", ...], pool_text, count, sources}
//
// 口径：清洗（去 ST / 去非 6 位代码 / 去重 / 截断）全部在脚本内完成，不调用 LLM。
//   来源两路：search_stock 命中（智能导演按 brief.keyword 检索）
//             + brief.hot_industries 的领涨股（行业排名的 leaderCode/leaderName）
//   labels 供 j-candidate-pick（LlmClassifier 的 categories_var）做「六选一」判定。
//   ⚠️ labels 不带序号：LlmClassifierExecutor 会对每个类别前缀 "N. "（llm_classifier_executor.rs:76-81），
//      脚本再带序号会变成 "1. 1. 300285 国瓷材料" 双重编号。序号只出现在人读的 pool_text 里。

fn to_array(v) {
    if type_of(v) == "array" { v }
    else if type_of(v) == "string" {
        let p = json_parse(v.to_string());
        if type_of(p) == "array" { p } else { [] }
    } else if type_of(v) == "map" {
        if v["content"] != () && type_of(v["content"]) == "string" {
            let p = json_parse(v["content"].to_string());
            if type_of(p) == "array" { p }
            else if type_of(p) == "map" && p["data"] != () && type_of(p["data"]) == "array" { p["data"] }
            else { [] }
        } else if v["data"] != () && type_of(v["data"]) == "array" { v["data"] }
        else if v["items"] != () && type_of(v["items"]) == "array" { v["items"] }
        else { [] }
    } else { [] }
}

fn field_str(m, key) {
    if type_of(m) != "map" { return ""; }
    let v = m[key];
    if v == () { return ""; }
    if type_of(v) == "string" { return v; }
    v.to_string()
}

/// 6 位纯数字代码判据
fn is_code6(s) {
    if type_of(s) != "string" { return false; }
    if s.len() != 6 { return false; }
    let digits = "0123456789";
    for i in range(0, 6) {
        if !digits.contains(s.sub_string(i, 1)) { return false; }
    }
    true
}

/// 可入池判据：6 位代码 + 非 ST / 退市 / 空名
fn acceptable(code, name) {
    if !is_code6(code) { return false; }
    if name == "" { return false; }
    let upper = name.to_upper();
    if upper.contains("ST") { return false; }
    if name.contains("退") { return false; }
    true
}

let pool_limit = 20;

let stocks = [];
let codes = [];
let sources = [];

// ── 来源 1：search_stock 命中 ──
let hits = to_array(search_results);
for h in hits {
    let code = field_str(h, "code");
    let name = field_str(h, "name");
    if !acceptable(code, name) { continue; }
    if codes.contains(code) { continue; }
    codes.push(code);
    stocks.push(#{ "code": code, "name": name, "source": "search_stock" });
};

// ── 来源 2：brief 里的行业领涨股 ──
if type_of(scanner_brief) == "map" {
    let hot = to_array(scanner_brief["hot_industries"]);
    for hr in hot {
        let code = field_str(hr, "leader_code");
        let name = field_str(hr, "leader_name");
        if !acceptable(code, name) { continue; }
        if codes.contains(code) { continue; }
        codes.push(code);
        stocks.push(#{ "code": code, "name": name, "source": "industry_leader" });
    };
    if hot.len() > 0 { sources.push("industry_leader"); }
};
if hits.len() > 0 { sources.push("search_stock"); }

// ── 截断 + 生成 labels / pool_text ──
let capped = [];
let labels = [];
let limit = if stocks.len() > pool_limit { pool_limit } else { stocks.len() };
for i in range(0, limit) {
    let s = stocks[i];
    capped.push(s);
    labels.push(field_str(s, "code") + " " + field_str(s, "name"));
};

let pool_text = "";
let seq = 0;
for l in labels {
    seq += 1;
    if pool_text != "" { pool_text += "\n"; }
    pool_text += seq.to_string() + ". " + l;
};
if pool_text == "" {
    pool_text = "（候选池为空：search_stock 无命中且行业排名无领涨股）";
};

#{
    "stocks": capped,
    "labels": labels,
    "pool_text": pool_text,
    "count": capped.len(),
    "sources": sources
}