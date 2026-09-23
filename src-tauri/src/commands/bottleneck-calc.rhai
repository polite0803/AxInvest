// bottleneck-calc.rhai v12 — 扩展证据类型 + 评分权重改为变量注入
// 不再依赖 7 个预定义行业基线，仅使用 chain-decomposer 产业链数据 + 行业排名
// 权重变量由 seed_serenity.rs 模板注入，默认 w_supply=0.35 w_demand=0.35 w_irreplace=0.30
// Rhai v1.25 不支持 let mut，全部 let

// ═══════════════════════════════════════════════
// 第一部分：函数定义
// ═══════════════════════════════════════════════

// 任意值 → 数组（防御 ToolNode 输出的多种形态）
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
            else if type_of(p) == "map" && p["industries"] != () && type_of(p["industries"]) == "array" { p["industries"] }
            else { [] }
        } else if v["data"] != () && type_of(v["data"]) == "array" { v["data"] }
        else if v["industries"] != () && type_of(v["industries"]) == "array" { v["industries"] }
        else if v["items"] != () && type_of(v["items"]) == "array" { v["items"] }
        else { [] }
    } else { [] }
}

// ═══════════════════════════════════════════════
// 第二部分：主函数
// ═══════════════════════════════════════════════

fn main() {
    // 解包 chain_analysis（来自链拆解 Agent 的 JSON 输出）
    let chain_nodes = [];
    if chain_analysis["chain_nodes"] != () {
        chain_nodes = to_array(chain_analysis["chain_nodes"]);
    } else if chain_analysis["arguments"] != () {
        let args = chain_analysis["arguments"];
        if args["chain_nodes"] != () {
            chain_nodes = to_array(args["chain_nodes"]);
        } else if args["content"] != () {
            let content_str = args["content"].to_string();
            let parsed = json_parse(content_str);
            if parsed != () && parsed["chain_nodes"] != () {
                chain_nodes = to_array(parsed["chain_nodes"]);
            }
        }
    }
    let num_nodes = chain_nodes.len();
    if num_nodes == 0 {
        return #{
            "status": "no_data", "computed_nodes": [],
            "summary": "无产业链数据输入",
            "overall_reliability": "insufficient_data"
        };
    }

    // 处理行业排名数据（用于行业动量评分，不影响瓶颈评分本身）
    let industry_map = #{};
    if industry_ranking != () {
        let ir = to_array(industry_ranking);
        if ir.len() > 0 {
            for ind in ir {
                let name = if ind["industryName"] != () { ind["industryName"].to_string() } else { "" };
                if name != "" {
                    industry_map[name] = #{
                        "changePct": if ind["changePct"] != () { ind["changePct"] } else { 0.0 },
                        "mainInflow": if ind["mainInflow"] != () { ind["mainInflow"] } else { 0.0 },
                        "turnover": if ind["turnover"] != () { ind["turnover"] } else { 0.0 },
                        "leaderName": if ind["leaderName"] != () { ind["leaderName"].to_string() } else { "" }
                    };
                }
            }
        }
    }

    // 遍历每个产业链环节，计算三力评分
    let computed_nodes = [];
    let max_composite = 0.0;
    let top_node_name = "";
    let any_has_financial = false;
    let any_has_industry = false;

    for node in chain_nodes {
        let node_name = if node["node_name"] != () { node["node_name"].to_string() } else { "未知环节" };
        let raw_supplier_count = if node["global_supplier_count"] != () { node["global_supplier_count"] } else { -1.0 };
        let raw_top3_share = if node["top3_market_share"] != () { node["top3_market_share"] } else { -1.0 };
        let raw_expansion = if node["expansion_cycle_months"] != () { node["expansion_cycle_months"] } else { -1.0 };
        let raw_barrier = if node["tech_barrier"] != () { node["tech_barrier"].to_string() } else { "low" };

        // 行业动量：遍历 industry_ranking 尝试匹配（不强制，匹配不上则默认）
        // （demand_certainty / evidence_type / financial_data 的提取与全部三力比分已
        //   收敛进 Rust 权威函数 bottleneck_node_score，见下方调用）
        let ind_change_pct = 0.0;
        let ind_main_inflow = 0.0;
        let matched_ind_name = "";
        for ind_key in industry_map.keys() {
            let kw = ind_key.to_string();
            if node_name.find(kw) != () || kw.find(node_name) != () {
                matched_ind_name = kw;
                if industry_map[kw] != () {
                    ind_change_pct = if industry_map[kw]["changePct"] != () { industry_map[kw]["changePct"] } else { 0.0 };
                    ind_main_inflow = if industry_map[kw]["mainInflow"] != () { industry_map[kw]["mainInflow"] } else { 0.0 };
                };
                break;
            }
        }
        if matched_ind_name != "" { any_has_industry = true; };

        // 三力权重（由模板变量注入，默认等权重：0.35/0.35/0.30）
        // 用户可在前端 Serenity 设置Tab中调整 w_supply/w_demand/w_irreplace
        let ws = if w_supply != () { w_supply } else { 0.35 };
        let wd = if w_demand != () { w_demand } else { 0.35 };
        let wi = if w_irreplace != () { w_irreplace } else { 0.30 };

        // 三力评分（含财务 comps / 行业动量 / 可靠度标签）：统一调用共享 Engine 的
        // Rust 权威函数 bottleneck_node_score（与 strategy-scorer 收敛为单一权威口径，
        // 权威语义 = 本脚本原行为）。s 的键均为 snake_case，脚本侧用面名直接取。
        let s = bottleneck_node_score(node, ind_change_pct, ind_main_inflow, ws, wd, wi, matched_ind_name);
        let supply_rigidity_score = s["supply_rigidity_score"];
        let demand_elasticity_score = s["demand_elasticity_score"];
        let irreplaceability_score = s["irreplaceability_score"];
        let bottleneck_composite = s["bottleneck_composite"];
        let final_composite = clamp(bottleneck_composite, 0.0, 100.0);
        let data_reliability = s["data_reliability"];
        let has_financial = s["has_financial_data"];
        let comps = s["financial_comps"];
        let industry_momentum_score = s["industry_momentum_score"];
        if has_financial { any_has_financial = true; };
        if final_composite > max_composite { max_composite = final_composite; top_node_name = node_name; };

        computed_nodes.push(#{
            "node_name": node_name,
            "bottleneck_potential": if node["bottleneck_potential"] != () { node["bottleneck_potential"].to_string() } else { "unknown" },
            "bottleneck_type": if node["bottleneck_type"] != () { node["bottleneck_type"].to_string() } else { "unknown" },
            "supply_rigidity_score": clamp(supply_rigidity_score, 0.0, 100.0),
            "demand_elasticity_score": clamp(demand_elasticity_score, 0.0, 100.0),
            "irreplaceability_score": clamp(irreplaceability_score, 0.0, 100.0),
            "bottleneck_composite": final_composite,
            "data_reliability": data_reliability,
            "has_financial_data": has_financial,
            "matched_industry": matched_ind_name,
            "industry_momentum_score": industry_momentum_score,
            "industry_change_pct": ind_change_pct,
            "financial_comps": comps,
            "readiness_signal": band_for_score(final_composite),
            "raw_inputs": #{
                "global_supplier_count": raw_supplier_count,
                "top3_market_share": raw_top3_share,
                "expansion_cycle_months": raw_expansion,
                "tech_barrier": raw_barrier
            }
        });
    }

    // 汇总
    let reliability_summary = if any_has_financial { "有真实个股财务数据支撑" } else if any_has_industry { "有行业排名数据交叉验证" } else { "纯 LLM 估算，无任何真实数据支撑" };
    let overall_reliability = if any_has_financial { "mixed_with_financial" } else if any_has_industry { "industry_corroborated" } else { "all_llm_estimated" };
    let summary = "共分析" + num_nodes.to_string() + "个环节, 最高瓶颈综合分: " + top_node_name + " = " + max_composite.to_string() + ", 可信度: " + reliability_summary;
    return #{
        "status": "ok",
        "num_nodes": num_nodes,
        "computed_nodes": computed_nodes,
        "has_any_financial_data": any_has_financial,
        "has_any_industry_match": any_has_industry,
        "top_bottleneck": #{ "node_name": top_node_name, "composite_score": max_composite },
        "overall_reliability": overall_reliability,
        "summary": summary,
        "industry_data_available": industry_ranking != ()
    };
}

// ═══════════════════════════════════════════════
// 第三部分：执行入口
// ═══════════════════════════════════════════════

main()
