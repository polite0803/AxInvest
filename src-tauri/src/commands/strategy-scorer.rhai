// strategy-scorer.rhai v2 — 统一多策略评分
// 根据 trend_strategy 变量路由到对应策略的评分逻辑
// 所有策略输出统一格式的 computed_nodes，供 candidate-mapper 消费
// Rhai v1.25 不支持 let mut，全部 let
//
// v2 修复（2026-07-31）：访问 input_mapping 注入变量的函数全部改为「闭包」。
// Rhai 的函数是纯函数，无法访问 eval_ast_with_scope 注入的 scope 变量
// （已验证：push/push_constant/顶层 let 提升在 fn 内全部 Variable not found），
// 而闭包（let f = |...| {...}）可以捕获环境变量。调用统一用 f.call(...)。
// 纯函数（clamp_score / readiness_signal）保持 fn 定义。

// ═══════════════════════════════════════════════
// 通用函数（所有策略共享）
// ═══════════════════════════════════════════════

fn clamp_score(v) { if v < 0.0 { 0.0 } else if v > 100.0 { 100.0 } else { v } }

// 任意值 → 数组（v4 防御，2026-07-31）
// 工具节点（ToolNode）输出经 ToolResult.content（String）注入 scope 后是 JSON 文本字符串，
// 直接 for 会报 "For loop expects iterable type"。统一解析：
// - 数组原样返回
// - 字符串 → json_parse 解析为数组
// - map → 尝试提取 content/data/industries/items 字段（ToolNode 包装格式）
// - 其它（含 ()/对象错误包）→ []
fn to_array(v) {
    if type_of(v) == "array" { v }
    else if type_of(v) == "string" {
        let p = json_parse(v.to_string());
        if type_of(p) == "array" { p } else { [] }
    } else if type_of(v) == "map" {
        // ToolNode 输出常被包装为 {"content": "JSON字符串"}
        if v["content"] != () && type_of(v["content"]) == "string" {
            let p = json_parse(v["content"].to_string());
            if type_of(p) == "array" { p }
            else if type_of(p) == "map" && p["data"] != () && type_of(p["data"]) == "array" { p["data"] }
            else if type_of(p) == "map" && p["industries"] != () && type_of(p["industries"]) == "array" { p["industries"] }
            else { [] }
        } else if v["data"] != () && type_of(v["data"]) == "array" { v["data"] }
        else if v["industries"] != () && type_of(v["industries"]) == "array" { v["industries"] }
        else if v["items"] != () && type_of(v["items"]) == "array" { v["items"] }
        else { [] }
    } else { [] }
}

// 任意值 → 浮点数（权重等数值字段防御：模板变量可能是 i64/字符串数字）
fn to_f64(v, fallback) {
    if type_of(v) == "f64" { v }
    else if type_of(v) == "i64" { v.to_float() }
    else if type_of(v) == "string" {
        let p = json_parse(v.to_string());
        if type_of(p) == "f64" { p } else if type_of(p) == "i64" { p.to_float() } else { fallback }
    } else { fallback }
}

// 从模板变量读取权重（兜底默认值）
// 注意：参数名不能用 default，它是 Rhai 保留关键字
let read_weight = |name, fallback| {
    if name == "w_supply" { if w_supply != () { w_supply } else { fallback } }
    else if name == "w_demand" { if w_demand != () { w_demand } else { fallback } }
    else if name == "w_irreplace" { if w_irreplace != () { w_irreplace } else { fallback } }
    else { fallback }
};

// fn readiness_signal(score) 产出档名与本体档名有 `_signal` vs `_bottleneck` 的命名分歧
// （见 OPEN_DIVERGENCES D3）。按 §7-a b1：阈值一律来自本体 band_for_score，
// 仅在映射回本脚本对外契约的 `_signal` 档名，6 处产出位（:250/:296/:342/:388/:446/:494）不改。
fn readiness_signal(score) {
    let band = band_for_score(score);
    if band == "strong_bottleneck" { "strong_signal" }
    else if band == "potential_bottleneck" { "potential_signal" }
    else { band } // "weak_signal" / "no_signal" 两侧档名一致，直接透传
}

// 归一化 chain_analysis（v18）：字符串（```tool_json/```json 围栏包裹的文本，拆包失败时保留）
// → 剥围栏 + json_parse 提取对象；解析失败（json_parse 返回 ()）置空 map。
// 否则闭包内 ca["chain_nodes"] 对字符串做索引会触发
// "Data type incorrect: string (expecting i64)"。
// 注意：Rhai 无内置 find/rfind，字符串查找用 contains()/index_of()；
// json_parse 由 harness 注册，失败静默返回 ()，无需 try/catch。
let ca = chain_analysis;
if type_of(ca) == "string" {
    let s = ca.to_string();
    let body = if s.contains("```") {
        // 剥围栏：跳过首行（```tool_json 等），取到闭合 ``` 之前
        let nl = s.index_of("\n");
        let rest = if nl >= 0 { s.sub_string(nl + 1, s.len() - nl - 1) } else { s };
        let fence_end = rest.index_of("```");
        if fence_end >= 0 { rest.sub_string(0, fence_end) } else { rest }
    } else { s };
    let parsed = json_parse(body); // serde_json 容忍首尾空白，Rhai 无可用 trim
    if type_of(parsed) == "map" { ca = parsed; }
    else { ca = #{}; }
} else if type_of(ca) == "map" && ca["report"] != () {
    // VERDICT 重构兜底形态：{"report":"```tool_json...```","verdict":{...}}
    // （agent_executor 拆包失败时 strict_mode 会把 tool_json 文本包进 report 字段）
    // 剥 report 文本的围栏再解析；解析失败保留原 map（走 no_data 兜底）
    let rs = ca["report"].to_string();
    let body = if rs.contains("```") {
        let nl = rs.index_of("\n");
        let rest = if nl >= 0 { rs.sub_string(nl + 1, rs.len() - nl - 1) } else { rs };
        let fence_end = rest.index_of("```");
        if fence_end >= 0 { rest.sub_string(0, fence_end) } else { rest }
    } else { rs };
    let parsed = json_parse(body);
    if type_of(parsed) == "map" {
        // 若 report 内含 submit_chain 协议（arguments 字段），取 arguments；否则用解析对象本身
        ca = if parsed["arguments"] != () && parsed["name"] != () { parsed["arguments"] }
             else { parsed };
    }
}

// ═══════════════════════════════════════════════
// bottleneck 策略评分（复用 bottleneck-calc 精简版）
// ═══════════════════════════════════════════════

let score_bottleneck = || {
    // 解包 chain_analysis（ca 为顶层归一化后的对象）
    // v4 简化：统一用 to_array 处理所有形态（直接数组 / 嵌套 arguments / JSON 字符串）
    let chain_nodes = [];
    if ca["chain_nodes"] != () {
        chain_nodes = to_array(ca["chain_nodes"]);
    } else if ca["arguments"] != () {
        let args = ca["arguments"];
        if args["chain_nodes"] != () {
            chain_nodes = to_array(args["chain_nodes"]);
        } else if args["content"] != () {
            let parsed = json_parse(args["content"].to_string());
            if parsed != () && parsed["chain_nodes"] != () {
                chain_nodes = to_array(parsed["chain_nodes"]);
            }
        }
    }
    if chain_nodes.len() == 0 {
        return #{"status": "no_data", "computed_nodes": [], "summary": "无产业链数据"};
    }

    // 行业排名数据处理
    // v4 修复（2026-07-31 22:30）：industry_ranking 是 ToolNode 输出，
    // 注入 scope 后可能是 字符串/数组/map（含 content 包装层）。
    // 统一用 to_array 解析，彻底覆盖所有形态。
    let ind_map = #{};
    if industry_ranking != () {
        let ir = to_array(industry_ranking);
        if ir.len() > 0 {
            for ind in ir {
                let name = if ind["industryName"] != () { ind["industryName"].to_string() } else { "" };
                if name != "" {
                    ind_map[name] = #{
                        "changePct": if ind["changePct"] != () { ind["changePct"] } else { 0.0 },
                        "mainInflow": if ind["mainInflow"] != () { ind["mainInflow"] } else { 0.0 },
                    };
                }
            }
        }
    }

    let computed = [];
    let max_score = 0.0; let top_name = "";
    let has_fin = false; let has_ind = false;

    for node in chain_nodes {
        let nn = if node["node_name"] != () { node["node_name"].to_string() } else { "未知" };
        // 节点原始字段（sc/t3/exp/bar/dc/ev/fd）的提取与全部三力/财务比分已收敛进
        // Rust 权威函数 bottleneck_node_score，脚本侧不再内联复算。

        // 行业动量
        let icp = 0.0; let iif = 0.0; let min = "";
        for ik in ind_map.keys() {
            let kw = ik.to_string();
            if nn.contains(kw) || kw.contains(nn) {
                min = kw;
                if ind_map[kw] != () {
                    icp = if ind_map[kw]["changePct"] != () { ind_map[kw]["changePct"] } else { 0.0 };
                    iif = if ind_map[kw]["mainInflow"] != () { ind_map[kw]["mainInflow"] } else { 0.0 };
                };
                break;
            }
        }
        if min != "" { has_ind = true; };

        // 三力评分（v3：权重统一 to_f64，防模板变量为 i64/字符串数字）
        // 评分本体已收敛到共享 Engine 的 Rust 权威函数 bottleneck_node_score
        // （口径 = 权威方 bottleneck-calc；strategy-scorer 原内联口径随此被修补为权威值）。
        let ws = if w_supply != () { to_f64(w_supply, 0.35) } else { 0.35 };
        let wd = if w_demand != () { to_f64(w_demand, 0.35) } else { 0.35 };
        let wi = if w_irreplace != () { to_f64(w_irreplace, 0.30) } else { 0.30 };
        let s = bottleneck_node_score(node, icp, iif, ws, wd, wi, min);

        let srs = s["supply_rigidity_score"];
        let des = s["demand_elasticity_score"];
        let irs = s["irreplaceability_score"];
        let composite = s["bottleneck_composite"];
        let rel = s["data_reliability"];
        let fin_ok = s["has_financial_data"];
        let ims = s["industry_momentum_score"];
        if fin_ok { has_fin = true; };
        if composite > max_score { max_score = composite; top_name = nn; };
        computed.push(#{
            "node_name": nn,
            "strategy_type": "bottleneck",
            "stock_code": if node["representative_stocks"] != () && node["representative_stocks"].len() > 0 { node["representative_stocks"][0]["code"].to_string() } else { "" },
            "stock_name": if node["representative_stocks"] != () && node["representative_stocks"].len() > 0 { node["representative_stocks"][0]["name"].to_string() } else { "" },
            "bottleneck_composite": composite,
            "supply_rigidity_score": clamp_score(srs),
            "demand_elasticity_score": clamp_score(des),
            "irreplaceability_score": clamp_score(irs),
            "data_reliability": rel,
            "readiness_signal": readiness_signal(composite),
            "industry_momentum_score": ims,
            "matched_industry": min,
        });
    }
    let rel_summary = if has_fin { "有真实个股财务数据支撑" } else if has_ind { "有行业排名数据交叉验证" } else { "纯 LLM 估算" };
    let overall = if has_fin { "mixed_with_financial" } else if has_ind { "industry_corroborated" } else { "all_llm_estimated" };
    let summary = "共分析 " + computed.len().to_string() + " 个环节, 最高分: " + top_name + " = " + max_score.to_string() + ", 可信度: " + rel_summary;
    #{"status": "ok", "num_nodes": computed.len(), "computed_nodes": computed,
      "top": #{ "name": top_name, "score": max_score },
      "overall_reliability": overall, "summary": summary}
};

// ═══════════════════════════════════════════════
// policy 策略评分
// ═══════════════════════════════════════════════

let score_policy = || {
    // note: weights are hardcoded per-strategy; future: inject via template variables
    let analysis = ca;  // Agent 输出的分析结果
    let beneficiaries = to_array(analysis["beneficiaries"]);
    if beneficiaries.len() == 0 {
        return #{"status": "no_data", "computed_nodes": [], "summary": "无政策受益数据"};
    }
    let policy_impact = if analysis["policy_impact_score"] != () { analysis["policy_impact_score"] } else { 50.0 };
    let computed = [];
    for sector in beneficiaries {
        let sector_name = if sector["sector"] != () { sector["sector"].to_string() } else { "未知行业" };
        let stocks = to_array(sector["stocks"]);
        for stk in stocks {
            let code = if stk["code"] != () { stk["code"].to_string() } else { "" };
            let name = if stk["name"] != () { stk["name"].to_string() } else { "" };
            let rationale = if stk["rationale"] != () { stk["rationale"].to_string() } else { "" };
            // policy 评分：政策力度 × 行业匹配度 × 龙头溢价
            let sector_match = 70.0; // 默认中高匹配
            let composite = clamp_score(policy_impact * 0.40 + sector_match * 0.35);
            computed.push(#{
                "node_name": name,
                "strategy_type": "policy",
                "stock_code": code,
                "stock_name": name,
                "sector": sector_name,
                "bottleneck_composite": composite,
                "policy_impact": policy_impact,
                "rationale": rationale,
                "data_reliability": "llm_estimated",
                "readiness_signal": readiness_signal(composite),
            });
        }
    }
    let summary = "政策分析: " + beneficiaries.len().to_string() + " 个受益行业, " + computed.len().to_string() + " 只候选";
    #{"status": "ok", "num_nodes": computed.len(), "computed_nodes": computed, "summary": summary,
      "overall_reliability": "llm_estimated"}
};

// ═══════════════════════════════════════════════
// earnings 策略评分
// ═══════════════════════════════════════════════

let score_earnings = || {
    let analysis = ca;
    let candidates = to_array(analysis["candidates"]);
    if candidates.len() == 0 {
        return #{"status": "no_data", "computed_nodes": [], "summary": "无业绩驱动数据"};
    }
    let computed = [];
    for c in candidates {
        let code = if c["code"] != () { c["code"].to_string() } else { "" };
        let name = if c["name"] != () { c["name"].to_string() } else { "" };
        let beat = if c["expected_beat_pct"] != () { c["expected_beat_pct"] } else { 0.0 };
        let reason = if c["reason"] != () { c["reason"].to_string() } else { "" };
        // earnings 评分：预期差幅度(40%) + 过往准确性(30%) + 催化剂时效(30%)
        let beat_score = if beat >= 30.0 { 90.0 } else if beat >= 15.0 { 70.0 } else if beat >= 5.0 { 50.0 } else { 25.0 };
        // 过往准确性：基于 reason 中的明确度估算（Agent 不输出时默认 50）
        let track_record = if reason.contains("数据") || reason.contains("调研") || reason.contains("动销") { 70.0 } else { 50.0 };
        // 催化剂时效：短中期利好加分
        let timeliness = if c["timeframe"] != () {
            let tf = c["timeframe"].to_string();
            if tf.contains("short") { 80.0 } else if tf.contains("mid") { 60.0 } else { 40.0 }
        } else { 55.0 };
        let composite = clamp_score(beat_score * 0.40 + track_record * 0.30 + timeliness * 0.30);
        computed.push(#{
            "node_name": name,
            "strategy_type": "earnings",
            "stock_code": code,
            "stock_name": name,
            "bottleneck_composite": composite,
            "expected_beat_pct": beat,
            "track_record_score": track_record,
            "timeliness_score": timeliness,
            "rationale": reason,
            "data_reliability": "partially_verified",
            "readiness_signal": readiness_signal(composite),
        });
    }
    let summary = "业绩分析: " + computed.len().to_string() + " 只候选";
    #{"status": "ok", "num_nodes": computed.len(), "computed_nodes": computed, "summary": summary,
      "overall_reliability": "partially_verified"}
};

// ═══════════════════════════════════════════════
// capital 策略评分
// ═══════════════════════════════════════════════

let score_capital = || {
    let analysis = ca;
    let sectors_analysis = to_array(analysis["sectors"]);
    if sectors_analysis.len() == 0 {
        return #{"status": "no_data", "computed_nodes": [], "summary": "无资金面数据"};
    }
    let computed = [];
    for sec in sectors_analysis {
        let sec_name = if sec["name"] != () { sec["name"].to_string() } else { "未知板块" };
        let inflow = if sec["net_inflow"] != () { sec["net_inflow"] } else { 0.0 };
        let sustainability = if sec["days"] != () { sec["days"] } else { 1.0 };
        let stocks = to_array(sec["stocks"]);
        for stk in stocks {
            let code = if stk["code"] != () { stk["code"].to_string() } else { "" };
            let name = if stk["name"] != () { stk["name"].to_string() } else { "" };
            // capital 评分：流入强度(40%) + 持续性(35%) + 成交量配合(25%)
            let inflow_score = if inflow > 20.0 { 85.0 } else if inflow > 10.0 { 65.0 } else if inflow > 5.0 { 45.0 } else { 25.0 };
            let sustain_score = if sustainability >= 5.0 { 85.0 } else if sustainability >= 3.0 { 65.0 } else { 40.0 };
            let volume_score = if sec["volume_surge"] != () {
                let vs = sec["volume_surge"];
                if vs > 1.5 { 80.0 } else if vs > 1.2 { 60.0 } else { 40.0 }
            } else { 50.0 };
            let composite = clamp_score(inflow_score * 0.40 + sustain_score * 0.35 + volume_score * 0.25);
            computed.push(#{
                "node_name": name,
                "strategy_type": "capital",
                "stock_code": code,
                "stock_name": name,
                "bottleneck_composite": composite,
                "net_inflow": inflow,
                "sustainability_days": sustainability,
                "volume_surge": if sec["volume_surge"] != () { sec["volume_surge"] } else { () },
                "sector": sec_name,
                "data_reliability": "industry_inferred",
                "readiness_signal": readiness_signal(composite),
            });
        }
    }
    let summary = "资金面分析: " + computed.len().to_string() + " 只候选";
    #{"status": "ok", "num_nodes": computed.len(), "computed_nodes": computed, "summary": summary,
      "overall_reliability": "industry_inferred"}
};

// ═══════════════════════════════════════════════
// 主函数：按策略类型路由
// ═══════════════════════════════════════════════

// ═══════════════════════════════════════════════
// event 策略评分（事件驱动）
// ═══════════════════════════════════════════════

let score_event = || {
    let analysis = ca;
    let events = to_array(analysis["events"]);
    if events.len() == 0 {
        return #{"status": "no_data", "computed_nodes": [], "summary": "无事件驱动数据"};
    }
    let computed = [];
    for ev in events {
        let code = if ev["code"] != () { ev["code"].to_string() } else { "" };
        let name = if ev["name"] != () { ev["name"].to_string() } else { "" };
        let ev_type = if ev["event_type"] != () { ev["event_type"].to_string() } else { "" };
        let desc = if ev["description"] != () { ev["description"].to_string() } else { "" };
        // 事件评分：事件确定性(40%) + 时效性(30%) + 价格位置(30%)
        let certainty = if ev_type == "contract" || ev_type == "订单" { 85.0 }
            else if ev_type == "policy" || ev_type == "政策" { 70.0 }
            else if ev_type == "earnings" || ev_type == "财报" { 60.0 }
            else if ev_type == "supply_shock" || ev_type == "供给冲击" { 75.0 }
            else if ev_type == "capacity_release" || ev_type == "产能释放" { 65.0 }
            else if ev_type == "rumor" || ev_type == "传闻" { 30.0 }
            else { 50.0 };
        let timeliness = if ev["timeframe"] != () {
            let tf = ev["timeframe"].to_string();
            if tf.contains("short") { 80.0 } else if tf.contains("mid") { 60.0 } else { 40.0 }
        } else { 50.0 };
        let price_position = if ev["price_position"] != () {
            let pp = ev["price_position"].to_string();
            if pp == "not_run" { 80.0 } else if pp == "slight_rise" { 65.0 } else if pp == "significant_rise" { 40.0 } else { 25.0 }
        } else { 50.0 };
        let composite = clamp_score(certainty * 0.40 + timeliness * 0.30 + price_position * 0.30);
        computed.push(#{
            "node_name": name,
            "strategy_type": "event",
            "stock_code": code,
            "stock_name": name,
            "bottleneck_composite": composite,
            "event_type": ev_type,
            "event_certainty": certainty,
            "event_timeliness": timeliness,
            "price_position_score": price_position,
            "description": desc,
            "data_reliability": "partially_verified",
            "readiness_signal": readiness_signal(composite),
        });
    }
    let summary = "事件分析: " + computed.len().to_string() + " 只候选";
    #{"status": "ok", "num_nodes": computed.len(), "computed_nodes": computed, "summary": summary,
      "overall_reliability": "partially_verified"}
};

// ═══════════════════════════════════════════════
// technical 策略评分（技术面驱动）
// ═══════════════════════════════════════════════

let score_technical = || {
    let analysis = ca;
    let signals = to_array(analysis["signals"]);
    if signals.len() == 0 {
        return #{"status": "no_data", "computed_nodes": [], "summary": "无技术面信号"};
    }
    let computed = [];
    for sig in signals {
        let code = if sig["code"] != () { sig["code"].to_string() } else { "" };
        let name = if sig["name"] != () { sig["name"].to_string() } else { "" };
        let pattern = if sig["pattern"] != () { sig["pattern"].to_string() } else { "" };
        let reason = if sig["reason"] != () { sig["reason"].to_string() } else { "" };
        // 技术评分：形态强度(35%) + 成交量确认(35%) + 板块共振(30%)
        let pattern_score = if pattern.contains("突破") || pattern.contains("breakout") { 85.0 }
            else if pattern.contains("金叉") || pattern.contains("golden") { 75.0 }
            else if pattern.contains("底背离") || pattern.contains("divergence") { 65.0 }
            else if pattern.contains("支撑") || pattern.contains("反弹") { 55.0 }
            else { 45.0 };
        let volume_confirm = if sig["volume_surge"] != () {
            let vs = sig["volume_surge"];
            if vs > 1.5 { 85.0 } else if vs > 1.2 { 65.0 } else { 45.0 }
        } else { 50.0 };
        let sector_alignment = if sig["sector_aligned"] != () && sig["sector_aligned"] { 80.0 } else { 50.0 };
        let composite = clamp_score(pattern_score * 0.35 + volume_confirm * 0.35 + sector_alignment * 0.30);
        computed.push(#{
            "node_name": name,
            "strategy_type": "technical",
            "stock_code": code,
            "stock_name": name,
            "bottleneck_composite": composite,
            "pattern": pattern,
            "pattern_score": pattern_score,
            "volume_confirm_score": volume_confirm,
            "sector_alignment_score": sector_alignment,
            "rationale": reason,
            "data_reliability": "industry_inferred",
            "readiness_signal": readiness_signal(composite),
        });
    }
    let summary = "技术分析: " + computed.len().to_string() + " 只候选";
    #{"status": "ok", "num_nodes": computed.len(), "computed_nodes": computed, "summary": summary,
      "overall_reliability": "industry_inferred"}
};

// ── 顶层路由（原 fn main → 顶层顺序执行） ──
// main() 的 fn 包装会导致 input_mapping 变量（trend_strategy）不可见，
// 移到顶层后 scope 变量可直接访问。
let strategy = if trend_strategy != () { trend_strategy.to_string() } else { "bottleneck" };
let result = if strategy == "bottleneck" { score_bottleneck.call() }
else if strategy == "policy" { score_policy.call() }
else if strategy == "earnings" { score_earnings.call() }
else if strategy == "capital" { score_capital.call() }
else if strategy == "event" { score_event.call() }
else if strategy == "technical" { score_technical.call() }
else { #{"status": "unknown_strategy", "computed_nodes": [], "summary": "未知策略类型: " + strategy} };
result
