// trend-split.rhai — 快速趋势智选：把合并生成腿的 trends[] 切成具名槽位
//
// 归属：`serenity-screening-fast` 模板新增资产（原链无对应节点）。
//
// 存在理由（引擎限制，非设计偏好）—— rt-workflow 的 resolve_var_path
// （crates/rt-workflow/src/work_engine/executors/mod.rs:113-171）按点号导航、
// 中间段只穿透 JSON 字符串，**不支持数组下标**：`a-trend-scanner.content.trends[0]`
// 恒 resolve 为 None（`current.get("trends[0]")` 对 Map/Array 必然失败，
// 回落整条路径直查同样为 None）⇒ input_mapping 注入 unit。
// 原链未暴露该缺陷是因为它的 chain_analysis 走 `a-chain-{tn}.content`（Agent 输出
// 的字符串，脚本侧自己剥围栏解析）；而若照抄 PLAN 的 `trends[{i}]` 形态，
// c-scorer 会拿到 unit，strategy-scorer.rhai:120 的 `ca["chain_nodes"]`
// 对 unit 索引将抛 Rhai 运行时错误（节点失败 → 工作流 PartiallyCompleted）。
// ⇒ 用确定性脚本（非 LLM）把 trends[] 切成 trend1..trend5 五个**恒存在**的具名键，
// 下游一律走纯对象导航 `c-trend-split.result.trend{i}`，必然 resolve。
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