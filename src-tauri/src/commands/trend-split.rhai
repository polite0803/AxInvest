// trend-split.rhai — 快速趋势智选：把合并生成腿的 trends[] 切成具名槽位
//
// 归属：`serenity-screening-fast` 模板新增资产（原链无对应节点）。
//
// 存在理由（引擎限制的**历史**背景，2026-10-24 起已部分解除）—— rt-workflow 的
// resolve_var_path 自 2026-10-24 起支持 `name[idx]` 数组下标（见 executors/mod.rs 的
// get_path_segment），因此 `a-trend-scanner.content.trends[0].strategy_type` 现在能解析。
// 本节点仍保留，原因不再是“完全取不到”，而是：
//   ① 下游需要**恒存在**的具名槽位（缺失槽位填 {trend_name:"", chain_nodes:[]}），
//      直接用下标会在趋势数 <5 时让部分评分节点拿到 unit（策略类型回落 bottleneck）；
//   ② 五个槽位的路径只在此处定义一次，不在 5 个 input_mapping 里各抄一遍。
//
// 输入：scanner_raw — a-trend-scanner.content（合并生成腿的 tool_json 文本）
// 输出：#{trend1..trend5}
//   每项为 trend 对象（含 chain_nodes[]）；缺失槽位填 {trend_name:"", chain_nodes:[]}
//   ⚠️ 不输出 strategy_type：策略类型由 j-strategy-type 单独判定并注入各 c-scorer，
//      组装端（candidate-assembler.rhai）同样不写该字段（见 data-verifier 契约）。

// 任意值 → 数组（与 strategy-scorer.rhai / candidate-assembler.rhai 同口径）
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

/// 取第 idx 个趋势；越界或非对象时返回占位槽（恒返回 map，保证下游导航不断链）
fn slot(trends, idx) {
    if idx < trends.len() {
        let t = trends[idx];
        if type_of(t) == "map" { return t; }
    }
    #{ "trend_name": "", "chain_nodes": [] }
}

// ═══════════════════════════════════════════════
// 归一化 scanner_raw（三层形态兜底，与 candidate-assembler.rhai 同口径）
//    a) 裸 JSON 文本（agent_executor 拆包 tool_json 后的 arguments 文本）
//    b) ```tool_json 围栏包裹
//    c) {"report": "<json 文本>"} 双重编码
//    d) {"name": "submit_trends", "arguments": {...}} submit 协议包装
// ═══════════════════════════════════════════════
let raw_map = #{};
if type_of(scanner_raw) == "map" {
    raw_map = scanner_raw;
} else if type_of(scanner_raw) == "string" {
    let parsed = json_parse(strip_fence(scanner_raw.to_string()));
    if type_of(parsed) == "map" { raw_map = parsed; }
};
if raw_map["trends"] == () && raw_map["report"] != () && type_of(raw_map["report"]) == "string" {
    let inner = json_parse(strip_fence(raw_map["report"].to_string()));
    if type_of(inner) == "map" { raw_map = inner; }
};
if raw_map["trends"] == () && raw_map["arguments"] != () && type_of(raw_map["arguments"]) == "map" {
    let args = raw_map["arguments"];
    if args["trends"] != () { raw_map = args; }
};

let trends = to_array(raw_map["trends"]);

#{
    "trend1": slot(trends, 0),
    "trend2": slot(trends, 1),
    "trend3": slot(trends, 2),
    "trend4": slot(trends, 3),
    "trend5": slot(trends, 4),
}