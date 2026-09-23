// SPDX-License-Identifier: AGPL-3.0-only

//! 产业链瓶颈三力评分（supply_rigidity / demand_elasticity / irreplaceability）的
//! **单一权威 Rust 实现**（`bottleneck_node_score`），注册进共享 Rhai Engine。
//!
//! 背景：`bottleneck-calc.rhai`（节点型）与 `strategy-scorer.rhai`（多策略）各写了一份
//! 三力评分，存在真实口径分歧（strategy-scorer 泄露 `"产能利用率提升"→60`、把
//! `"合同负债增长"/"合同负债"` 误映射为 60，而权威方 bottleneck-calc 映射为 75）。
//! 本函数收敛二者为**权威语义 = bottleneck-calc.rhai 原行为**（strategy-scorer 切换后
//! 走派生口径会「被修补」为权威值，属预期行为变更）。
//!
//! 健壮性：纯函数、无 DB、无副作用，唯一外部 I/O 是遇到未知证据类型时打印一条
//! `eprintln!` 告警（与原 `.rhai` 里 `print("WARNING: ...")` 的行为对齐）。
//! 纯函数不跨 await，无需任何锁，符合 clippy 铁律。

use rhai::{Dynamic, Engine, Map};

// ═══════════════════════════════════════════════
// 共享小工具
// ═══════════════════════════════════════════════

/// 把 Rhai Dynamic 数值转成 `Option<f64>`：接受 f64 / i64；unit 或其它 → None。
fn dyn_f64(v: &Dynamic) -> Option<f64> {
    if v.is_unit() {
        return None;
    }
    v.clone().try_cast::<f64>().or_else(|| v.clone().try_cast::<i64>().map(|x| x as f64))
}

/// 从 map 取数值字段，缺失/不可转 → None。
fn map_num(m: &Map, key: &str) -> Option<f64> {
    m.get(key).and_then(dyn_f64)
}

/// 从 map 取字符串字段，缺失/不可转 → 默认值。
fn map_str(m: &Map, key: &str, fallback: &str) -> String {
    m.get(key).and_then(|v| v.clone().into_string().ok()).unwrap_or_else(|| fallback.to_string())
}

/// 分档工具：技术壁垒「高/中/低」→ 分数。
fn barrier_to_score(level: &str) -> f64 {
    if level == "high" {
        85.0
    } else if level == "medium" {
        60.0
    } else {
        35.0
    }
}

/// 分档工具：需求确定性「高/中/低」→ 分数。
fn certainty_to_score(level: &str) -> f64 {
    if level == "high" {
        80.0
    } else if level == "medium" {
        55.0
    } else {
        30.0
    }
}

/// 证据类型 → 分数（**权威全映射**，逐字仿 bottleneck-calc.rhai 原实现）。
/// 包含 `"产能利用率提升"→60` 与 `"合同负债增长"/"合同负债"→75`，
/// 修复 strategy-scorer 的泄露 / 误映射。
fn evidence_to_score(typ: &str) -> f64 {
    if typ == "有已公开长协/订单" {
        90.0
    } else if typ == "已签长协" || typ == "长协订单" || typ == "合同负债增长" || typ == "合同负债"
    {
        75.0
    } else if typ == "产能预订"
        || typ == "产能满载"
        || typ == "产能利用率提升"
        || typ == "订单饱满"
        || typ == "订单增长"
    {
        60.0
    } else if typ == "客户扩产" || typ == "下游扩产" || typ == "政策强制" || typ == "政策驱动"
    {
        55.0
    } else if typ == "无公开证据" || typ == "LLM推测" || typ == "合理推断" || typ.is_empty()
    {
        30.0
    } else {
        // 未知类型按中位数 50 分并告警（对齐原脚本 `print` 行为）
        eprintln!("WARNING: evidence_to_score 遇到未知证据类型 '{}'，按 50.0 评分", typ);
        50.0
    }
}

/// 财务分项 → 分数（各分档），0..N 逐字仿 bottleneck-calc.rhai :60-65。
fn fin_score_gm(v: f64) -> f64 {
    if v >= 50.0 {
        90.0
    } else if v >= 30.0 {
        65.0
    } else if v >= 0.0 {
        40.0
    } else {
        -1.0
    }
}
fn fin_score_rg(v: f64) -> f64 {
    if v >= 20.0 {
        85.0
    } else if v >= 10.0 {
        60.0
    } else if v >= 0.0 {
        35.0
    } else {
        -1.0
    }
}
fn fin_score_dr(v: f64) -> f64 {
    if (0.0..40.0).contains(&v) {
        80.0
    } else if v < 60.0 {
        55.0
    } else if v >= 0.0 {
        30.0
    } else {
        -1.0
    }
}
fn fin_score_roe(v: f64) -> f64 {
    if v >= 15.0 {
        85.0
    } else if v >= 8.0 {
        60.0
    } else if v >= 0.0 {
        35.0
    } else {
        -1.0
    }
}
fn fin_score_rnd(v: f64) -> f64 {
    if v >= 10.0 {
        85.0
    } else if v >= 5.0 {
        60.0
    } else if v >= 0.0 {
        35.0
    } else {
        -1.0
    }
}
fn fin_score_cdr(v: f64) -> f64 {
    if v >= 2.0 {
        80.0
    } else if v >= 1.0 {
        55.0
    } else if v >= 0.0 {
        30.0
    } else {
        -1.0
    }
}

/// 财务 comps 评分（逐字仿 bottleneck-calc.rhai :52-84 的 `calc_financial_comps_score`）。
///
/// 返回与脚本版一致的字段全集：
/// available / num_metrics / gross_margin_score / revenue_growth_score /
/// debt_ratio_score / roe_score / rnd_score / capex_score / financial_health / raw_values。
fn calc_financial_comps_score(financial_data: Option<&Map>) -> Map {
    let present = financial_data.is_some();
    let empty = Map::new();
    let fin = financial_data.unwrap_or(&empty);

    let gm = map_num(fin, "gross_margin").unwrap_or(-1.0);
    let rg = map_num(fin, "revenue_growth_yoy").unwrap_or(-1.0);
    let dr = map_num(fin, "debt_ratio").unwrap_or(-1.0);
    let roe = map_num(fin, "roe").unwrap_or(-1.0);
    let rnd = map_num(fin, "rnd_ratio").unwrap_or(-1.0);
    let cdr = map_num(fin, "capex_dep_ratio").unwrap_or(-1.0);

    let gm_score = fin_score_gm(gm);
    let rg_score = fin_score_rg(rg);
    let dr_score = fin_score_dr(dr);
    let roe_score = fin_score_roe(roe);
    let rnd_score = fin_score_rnd(rnd);
    let cdr_score = fin_score_cdr(cdr);

    let mut total = 0.0;
    let mut cnt = 0.0;
    for s in [gm_score, rg_score, dr_score, roe_score, rnd_score, cdr_score] {
        if s >= 0.0 {
            total += s;
            cnt += 1.0;
        }
    }
    let financial_health = if cnt > 0.0 {
        (total / cnt).clamp(0.0, 100.0)
    } else {
        -1.0
    };

    let mut out = Map::new();
    out.insert("available".into(), Dynamic::from(present));
    out.insert("num_metrics".into(), Dynamic::from(cnt));

    // 原始值（嵌套）
    let mut raw = Map::new();
    raw.insert("gross_margin".into(), Dynamic::from(gm));
    raw.insert("revenue_growth".into(), Dynamic::from(rg));
    raw.insert("debt_ratio".into(), Dynamic::from(dr));
    raw.insert("roe".into(), Dynamic::from(roe));
    raw.insert("rnd_ratio".into(), Dynamic::from(rnd));
    raw.insert("capex_dep_ratio".into(), Dynamic::from(cdr));

    out.insert("gross_margin_score".into(), Dynamic::from(gm_score));
    out.insert("revenue_growth_score".into(), Dynamic::from(rg_score));
    out.insert("debt_ratio_score".into(), Dynamic::from(dr_score));
    out.insert("roe_score".into(), Dynamic::from(roe_score));
    out.insert("rnd_score".into(), Dynamic::from(rnd_score));
    out.insert("capex_score".into(), Dynamic::from(cdr_score));
    out.insert("financial_health".into(), Dynamic::from(financial_health));
    out.insert("raw_values".into(), Dynamic::from_map(raw));
    out
}

// ═══════════════════════════════════════════════
// 注册入口
// ═══════════════════════════════════════════════

/// 把产业链瓶颈三力评分的权威函数注册到指定 Engine。
///
/// 纯函数，无 DB / 无副作用。严格的实现：
/// - `node` = chain_node 的 map（`global_supplier_count` / `top3_market_share` /
///   `expansion_cycle_months` / `tech_barrier` / `demand_validation`（子字段
///   `demand_certainty` / `order_visibility`）/ `financial_data`），字段均可选，
///   缺失按 `-1.0` / `"low"` / `""` 处理。
/// - `icp` = 行业涨幅、`iif` = 行业主力净流入（脚本侧匹配后传入）。
/// - `ws` / `wd` / `wi` = 三力权重（脚本侧已从模板变量读出，默认 0.35/0.35/0.30）。
/// - `matched_ind` = 匹配到的行业名（`""` 表示无）。
pub fn register_bottleneck_functions(engine: &mut Engine) {
    engine.register_fn(
        "bottleneck_node_score",
        |node: Dynamic,
         icp: f64,
         iif: f64,
         ws: Dynamic,
         wd: Dynamic,
         wi: Dynamic,
         matched_ind: &str|
         -> Map { bottleneck_node_score(&node, icp, iif, ws, wd, wi, matched_ind) },
    );
}

/// 核心：单个产业链环节的三力评分（权威实现，见模块头文档）。
pub fn bottleneck_node_score(
    node: &Dynamic,
    icp: f64,
    iif: f64,
    ws: rhai::Dynamic,
    wd: rhai::Dynamic,
    wi: rhai::Dynamic,
    matched_ind: &str,
) -> Map {
    // ── 节点字段提取（均可选，缺失给默认值）─────────────────────────
    let node_map: Map = node.clone().try_cast().unwrap_or_default();

    // 权重去类型化：模板注入的 w_supply/w_demand/w_irreplace 可能是 f64/i64/字符串
    // （strategy-scorer v3 实锤过，见脚本注释），统一经 dyn_f64 兜底默认 0.35/0.35/0.30，
    // 避免强类型 f64 形参在 i64/字符串注入时报类型错误（bottleneck-calc 系原始注入）。
    let ws = dyn_f64(&ws).unwrap_or(0.35);
    let wd = dyn_f64(&wd).unwrap_or(0.35);
    let wi = dyn_f64(&wi).unwrap_or(0.30);
    let raw_supplier = map_num(&node_map, "global_supplier_count").unwrap_or(-1.0);
    let raw_top3 = map_num(&node_map, "top3_market_share").unwrap_or(-1.0);
    let raw_exp = map_num(&node_map, "expansion_cycle_months").unwrap_or(-1.0);
    let raw_barrier = map_str(&node_map, "tech_barrier", "low");

    let demand_validation: Map = node_map
        .get("demand_validation")
        .and_then(|v| v.clone().try_cast::<Map>())
        .unwrap_or_default();
    let demand_certainty = map_str(&demand_validation, "demand_certainty", "low");
    let evidence_type = map_str(&demand_validation, "order_visibility", "");

    let financial_data: Option<Map> =
        node_map.get("financial_data").and_then(|v| v.clone().try_cast::<Map>());

    // ── 财务 comps（权威 `calc_financial_comps_score`）────────────────
    let comps = calc_financial_comps_score(financial_data.as_ref());
    let has_financial =
        comps.get("available").and_then(|v| v.clone().try_cast::<bool>()).unwrap_or(false);

    // ── 子分 ──────────────────────────────────────────────────────────
    let concentration_score = if raw_top3 >= 70.0 {
        85.0
    } else if raw_top3 >= 50.0 {
        65.0
    } else if raw_top3 >= 0.0 {
        40.0
    } else {
        50.0
    };
    let barrier_score = barrier_to_score(&raw_barrier);
    let cycle_score = if raw_exp >= 24.0 {
        80.0
    } else if raw_exp >= 12.0 {
        60.0
    } else if raw_exp >= 0.0 {
        40.0
    } else {
        50.0
    };
    let financial_capex_score = if has_financial {
        map_num(&comps, "capex_score").unwrap_or(-1.0)
    } else {
        -1.0
    };
    let adjusted_cycle_score = if financial_capex_score >= 0.0 {
        (cycle_score + financial_capex_score) / 2.0
    } else {
        cycle_score
    };
    let supply_rigidity_score =
        concentration_score * 0.30 + barrier_score * 0.40 + adjusted_cycle_score * 0.30;

    let evidence_score = evidence_to_score(&evidence_type);
    let certainty_score = certainty_to_score(&demand_certainty);
    let industry_momentum_score = if icp > 5.0 && iif > 0.0 {
        75.0
    } else if icp > 2.0 && iif > 0.0 {
        60.0
    } else if icp > 0.0 {
        45.0
    } else {
        30.0
    };
    let financial_revenue_score = if has_financial {
        map_num(&comps, "revenue_growth_score").unwrap_or(-1.0)
    } else {
        -1.0
    };
    let adjusted_evidence = if industry_momentum_score > 0.0 {
        let base = if financial_revenue_score >= 0.0 {
            (evidence_score + financial_revenue_score) / 2.0
        } else {
            evidence_score
        };
        if icp > 0.0 {
            (base + industry_momentum_score) / 2.0
        } else if base > 50.0 {
            base - 10.0
        } else {
            base
        }
    } else if financial_revenue_score >= 0.0 {
        (evidence_score + financial_revenue_score) / 2.0
    } else {
        evidence_score
    };
    let demand_elasticity_score = adjusted_evidence * 0.60 + certainty_score * 0.40;

    let supplier_score = if (0.0..=2.0).contains(&raw_supplier) {
        90.0
    } else if raw_supplier <= 5.0 {
        70.0
    } else if raw_supplier <= 10.0 {
        50.0
    } else if raw_supplier > 10.0 {
        30.0
    } else {
        50.0
    };
    let financial_rnd_score = if has_financial {
        map_num(&comps, "rnd_score").unwrap_or(-1.0)
    } else {
        -1.0
    };
    let financial_roe_score = if has_financial {
        map_num(&comps, "roe_score").unwrap_or(-1.0)
    } else {
        -1.0
    };
    let tech_moat_score = if financial_rnd_score >= 0.0 && financial_roe_score >= 0.0 {
        financial_rnd_score * 0.5 + financial_roe_score * 0.5
    } else {
        barrier_score
    };
    let irreplaceability_score =
        supplier_score * 0.30 + tech_moat_score * 0.40 + barrier_score * 0.30;

    // ── composite 与可靠度 ────────────────────────────────────────────
    let composite =
        (supply_rigidity_score * ws + demand_elasticity_score * wd + irreplaceability_score * wi)
            .clamp(0.0, 100.0);
    let data_reliability = if has_financial {
        "partially_verified".to_string()
    } else if !matched_ind.is_empty() {
        "industry_inferred".to_string()
    } else if raw_supplier >= 0.0 || raw_top3 >= 0.0 {
        "llm_estimated".to_string()
    } else {
        "insufficient".to_string()
    };

    // 子分与脚本输出保持一致，也 clamp 到 0..100（原 .rhai 输出位单独 clamp）。
    let clamp100 = |v: f64| v.clamp(0.0, 100.0);

    let mut out = Map::new();
    out.insert("supply_rigidity_score".into(), Dynamic::from(clamp100(supply_rigidity_score)));
    out.insert("demand_elasticity_score".into(), Dynamic::from(clamp100(demand_elasticity_score)));
    out.insert("irreplaceability_score".into(), Dynamic::from(clamp100(irreplaceability_score)));
    out.insert("bottleneck_composite".into(), Dynamic::from(composite));
    out.insert("data_reliability".into(), Dynamic::from(data_reliability));
    out.insert("has_financial_data".into(), Dynamic::from(has_financial));
    out.insert("financial_comps".into(), Dynamic::from_map(comps));
    out.insert("industry_momentum_score".into(), Dynamic::from(industry_momentum_score));
    out
}
