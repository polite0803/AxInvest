// candidate-assembler.rhai — 快速趋势智选 Phase E：候选组装（替代原链的 candidate-mapper Agent）
//
// 归属：`serenity-screening-fast` 模板新增资产（原链由 `stock-candidate-mapper` Agent 承担）。
// 节点 id 沿用 `a-candidate-mapper`（`serenity.rs` 按 id 硬读），类型由 Agent 改为 Code(Rhai)。
//
// 输入：
//   scanner_raw     — a-trend-scanner.content（合并生成腿的 tool_json 文本）
//   strategy_trend1..5 — c-scorer-trend{1..5}.result（strategy-scorer.rhai 的确定评分）
//   consistency     — c-consistency-check.result（consistency-check.rhai 的输出）
//   strategy_type   — j-strategy-type.category（LlmClassifier 六选一）
// 输出：{candidates:[...], summary}
//   candidates 每项字段严格对齐 data-verifier.rhai 的 bottleneck 分支（12 字段子集），
//   ⚠️ 禁止写 strategy_type：data-verifier 的 bottleneck 分支按固定字段名逐个复制，
//   多写的键会被丢弃（写了等于没写，且误导后来者）。

// ── 通用防御（形态与 strategy-scorer.rhai 一致） ──
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

fn to_f64(v, fallback) {
    if type_of(v) == "f64" { v }
    else if type_of(v) == "i64" { v.to_float() }
    else if type_of(v) == "string" {
        let p = json_parse(v.to_string());
        if type_of(p) == "f64" { p } else if type_of(p) == "i64" { p.to_float() } else { fallback }
    } else { fallback }
}

fn field_str(m, key) {
    if type_of(m) != "map" { return ""; }
    let v = m[key];
    if v == () { return ""; }
    if type_of(v) == "string" { return v; }
    v.to_string()
}

fn field_f64(m, key, fallback) {
    if type_of(m) != "map" { return fallback; }
    let v = m[key];
    if v == () { return fallback; }
    to_f64(v, fallback)
}

fn clamp100(v) { if v < 0.0 { 0.0 } else if v > 100.0 { 100.0 } else { v } }

/// 保形态：map / array 原样透传，其它 → 空 map。
/// ⚠️ exit_signals 的契约是**对象**（technology_disruption_risk / capacity_oversupply_risk /
/// new_entrant_risk / demand_slowdown_risk / overall_exit_urgency），不能用 to_array
/// （to_array(map) 只在含 content/data/items 键时才取值，会把对象静默压成空数组）。
fn as_map_or_array(v) {
    if type_of(v) == "map" { v }
    else if type_of(v) == "array" { v }
    else { #{} }
}

/// 剥 ```tool_json / ```json 围栏（拆包失败时原样返回）
fn strip_fence(s) {
    let body = if s.contains("```") {
        let nl = s.index_of("\n");
        let rest = if nl >= 0 { s.sub_string(nl + 1, s.len() - nl - 1) } else { s };
        let fence_end = rest.index_of("```");
        if fence_end >= 0 { rest.sub_string(0, fence_end) } else { rest }
    } else { s };
    body
}

/// 从可能的包装中提取 computed_nodes 数组（与 consistency-check.rhai 同口径）
fn extract_computed_nodes(data) {
    if data == () { return []; }
    if type_of(data) == "array" { return data; }
    if type_of(data) == "map" {
        let nodes = data["computed_nodes"];
        if nodes != () && type_of(nodes) == "array" { return nodes; }
        let r = data["result"];
        if r != () && type_of(r) == "map" && r["computed_nodes"] != () && type_of(r["computed_nodes"]) == "array" {
            return r["computed_nodes"];
        }
    }
    []
}

// ═══════════════════════════════════════════════
// 1. 归一化 scanner_raw（三层形态兜底）
//    a) 裸 JSON 文本（agent_executor 拆包 tool_json 后的 arguments 文本）
//    b) ```tool_json 围栏包裹
//    c) {"report": "<json 文本>"} 双重编码
// ═══════════════════════════════════════════════
let raw_map = #{};
if type_of(scanner_raw) == "map" {
    raw_map = scanner_raw;
} else if type_of(scanner_raw) == "string" {
    let parsed = json_parse(strip_fence(scanner_raw.to_string()));
    if type_of(parsed) == "map" { raw_map = parsed; }
};
// b/c 兜底：report 字符串内层再解析（可能再带围栏）
if raw_map["trends"] == () && raw_map["report"] != () && type_of(raw_map["report"]) == "string" {
    let inner = json_parse(strip_fence(raw_map["report"].to_string()));
    if type_of(inner) == "map" { raw_map = inner; }
};
// arguments 包装形态：{"name":"submit_trends","arguments":{...}}
if raw_map["trends"] == () && raw_map["arguments"] != () && type_of(raw_map["arguments"]) == "map" {
    let args = raw_map["arguments"];
    if args["trends"] != () { raw_map = args; }
};

let trends = to_array(raw_map["trends"]);

// ═══════════════════════════════════════════════
// 2. 评分索引：c-scorer-trend{1..5} 的 computed_nodes → 按代码 / 名称建表
//    （顶层读取注入变量；fn 是纯函数看不到注入 scope）
// ═══════════════════════════════════════════════
let strat_vars = [
    strategy_trend1, strategy_trend2, strategy_trend3,
    strategy_trend4, strategy_trend5,
];
let score_by_code = #{};
let score_by_name = #{};
for sv in strat_vars {
    let nodes = extract_computed_nodes(sv);
    for nd in nodes {
        let comp = field_f64(nd, "bottleneck_composite", -1.0);
        if comp < 0.0 { continue; }
        let code = field_str(nd, "stock_code");
        if code != "" && !(code in score_by_code) { score_by_code[code] = comp; }
        let nm = field_str(nd, "node_name");
        if nm != "" && !(nm in score_by_name) { score_by_name[nm] = comp; }
        let snm = field_str(nd, "stock_name");
        if snm != "" && !(snm in score_by_name) { score_by_name[snm] = comp; }
    }
}

// 一致性置信度（<0 表示未检查；原链 consistency-check.rhai 无数据时返回 -1）
let consistency_score = field_f64(consistency, "consistency_score", -1.0);

// ═══════════════════════════════════════════════
// 3. 扁平 trends[].candidates[] → 去重 + 打分回填 + 置信度封顶
// ═══════════════════════════════════════════════
let candidates = [];
let seen = [];
let trend_names_used = [];

for t in trends {
    let tname = field_str(t, "trend_name");
    let ttype = field_str(t, "strategy_type");
    let tconf = field_f64(t, "confidence", 0.0);
    let cs = to_array(t["candidates"]);
    if cs.len() > 0 && tname != "" { trend_names_used.push(tname); }
    for c in cs {
        let code = field_str(c, "stock_code");
        if code == "" { continue; }
        if seen.contains(code) { continue; }
        seen.push(code);

        let name = field_str(c, "stock_name");
        // serenity_score：优先取确定评分（按代码 → 按名称），其次候选自带，最后 50
        let score = if code in score_by_code {
            score_by_code[code]
        } else if name != "" && (name in score_by_name) {
            score_by_name[name]
        } else {
            to_f64(c["serenity_score"], 50.0)
        };
        // 没有确定评分时，用趋势自身 confidence 作为弱代理（不覆盖真实评分）
        if !(code in score_by_code) && !(name != "" && (name in score_by_name)) && tconf > 0.0 {
            score = tconf;
        }

        let conf = field_f64(c, "confidence", 50.0);
        if consistency_score > 0.0 && consistency_score < 100.0 && consistency_score < conf {
            conf = consistency_score;
        }

        let att = c["attention_metrics"];
        let fin = c["financial_snapshot"];
        candidates.push(#{
            "stock_code": code,
            "stock_name": name,
            "relevance": if field_str(c, "relevance") != "" { field_str(c, "relevance") } else { "core" },
            "serenity_score": clamp100(score),
            "confidence": clamp100(conf),
            "bottleneck_product": field_str(c, "bottleneck_product"),
            "primary_risk": field_str(c, "primary_risk"),
            "catalysts": to_array(c["catalysts"]),
            "exit_signals": as_map_or_array(c["exit_signals"]),
            "attention_metrics": if type_of(att) == "map" { att } else { #{} },
            "financial_snapshot": if type_of(fin) == "map" { fin } else { () },
        });
    }
}

// ═══════════════════════════════════════════════
// 4. summary（空候选也必须给出可读原因 —— serenity.rs 的 emptyReason 取自此处）
// ═══════════════════════════════════════════════
let st = if strategy_type != () { strategy_type.to_string() } else { "bottleneck" };
let summary = "";
if candidates.len() > 0 {
    summary = "快速链组装 " + candidates.len().to_string() + " 只候选（策略=" + st
        + "，趋势槽位 " + trend_names_used.len().to_string() + " 个）";
} else if trends.len() == 0 {
    summary = "快速链无候选：合并生成腿未产出 trends（scanner_raw 解析为空或市场数据不可用）";
} else {
    summary = "快速链无候选：" + trends.len().to_string()
        + " 个趋势均未给出可解析的 candidates 数组（策略=" + st + "）";
};

#{ "candidates": candidates, "summary": summary }