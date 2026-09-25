// trader-proxy.rhai — 快速链（`stock-analysis-fast`）的「交易结论」组装节点
//
// ⚠ 本脚本是**快速链专属的新脚本**，不是任何既有脚本的副本或版本分支 ——
//   H2 禁止的是「把两链共用的脚本复制一份加 `-fast` 后缀、再让两边分叉」。
//   两链真正共用的 `portfolio-mgr.rhai` / `analyst-brief.rhai` / `data-quality.rhai`
//   仍然各只有一份、零内部分支；本脚本只负责产出「与原链 `trader` 变量**同形**」的
//   数据，供那份共用的 `portfolio-mgr.rhai` 原样消费。
//
// ── 为什么需要它 ──
// 原链的 `trader` 由 Agent 节点产出（LLM 输出 `{report, verdict:{...}}`）。
// 快速链不建那 23 个 Agent 节点（H1：原图一行不动；快速链本就是另一张新图）
// ⇒ 若不补这一步，`portfolio-mgr` 的 8 条 `trader_*` 映射全部解析为 Null，
//   f7 因子整体失效、`odds` 恒走 fallback 档。本节点以**确定性**方式重建同形数据：
//     方向 / 风险等级 / 置信度 ← Jev 分类节点（`j-*`）
//     目标价 / 止损价          ← 算法腿（现价 × DCF 上行空间；止损按年化波动率）
//     周期 / 持有天数          ← Jev 的强度档
//     证据清单                 ← 段 B 维度判定的非中性项 + 数据质量诊断（**必须是数组**，见下）
//
// ── 输入契约（由 CodeNodeConfig.input_mapping 注入）──
//   current_price  ← t-scoring.result.content.currentPrice
//   dcf_upside     ← t-valuation.result.content.dcf.upsidePct
//   volatility     ← t-risk.result.content.stockRiskProfile.annualizedVolatilityPct
//   dq_diagnostics ← data-quality.result.diagnostics
//   j_dimensions   ← jev_verdicts.result          （段 B 的 10 个维度判定数组）
//   j_direction    ← j-direction.category      （看多 / 中性 / 看空）
//   j_confidence   ← j-confidence.confidence   （Jev 数值 0-1）或 .category（高/中/低）
//   j_conviction   ← j-conviction.category     （强 / 中 / 弱）
//   j_risk_level   ← j-risk-level.category     （低 / 中 / 高 / 极高）
//   j_target       ← j-target.category         （"5%" / "10%" / "20%" / "30%"）
//   j_stop         ← j-stop.category           （同上）
//
// ⚠ 段 B/C/D 的 29 个 `j-*` 节点与两个聚合器由
//   `seed_stock_analysis.rs::apply_fast_chain_overrides` 的 ⑤ 步创建，并**显式补边**
//   到本节点（见 `FAST_JEV_TRADER_INPUTS` 与 `jev_verdicts`）。任一环节缺失（节点没建 /
//   边没补 / 名字被改）时这些路径都解析为 Null ⇒ 全部走 `present()` 降级分支，
//   **不会硬失败**（同 `risk-level.rhai` 的降级纪律）—— 这正是「静默失效」的高发点，
//   故由 `fast_chain_jev_nodes_are_wired` 单测逐条断言「每条声明都有对应的边」。
//
// ⚠ V57 防御（11 个输入变量**必须**逐个出现在 `present(...)` 调用里）──
//   `code_executor.rs::execute_rhai_directly` 会**文本扫描**脚本源码、把
//   `present(<变量>)` 的实参名抠出来（不剥注释、不看语义），再对「未被
//   input_mapping 注入、且不是 fn 形参 / let 局部变量」的名字自动补 `unit`。
//   写文档时一律用 `present(...)` / `present(<变量>)` 这两种形态。
//
// ── 输出契约（engine 会把本脚本返回值再包一层 `result`）──
//   下游读到的路径是 `trader.result.content.verdict.*`
//   —— 原链 Agent 输出是 `trader.content.verdict.*`，差一层正是本节点为 CodeNode
//      所致；`portfolio-mgr` 在快速链的 `input_mapping` 因此必须改前缀
//      （属 H2 明确允许的**配置层**差异，脚本零分叉）。
//   ⚠ `verdict.evidence_cited` **必须是数组**：`portfolio-mgr.rhai` 的 f7 在
//      `type_of(...) != "array"` 时把 `evidence_mod` 打到 0.4 下界（静默降权）。
//      故本脚本任何分支都返回数组（空数组也合法），绝不返回 null。
//   `content.degraded` / `content.warnings` 是决策留痕（诊断面板与复盘用），
//   下游不消费，勿删。
//
// ── 证据清单的两个来源 ──
//   ① `j_dimensions`（`jev_verdicts.result`）：段 B 的 10 个维度判定数组。**非中性**项
//      即「本次决策实际依赖的论据」，是快速链真实可测的证据来源（T7 已接通）。
//   ② `dq_diagnostics`（`data-quality.result.diagnostics`）：逐维度诊断里
//      `status == "normal"` 的维度名。该诊断的输入是 10 个分析师的 `*_verdict` /
//      `*_report`，而快速链不建这些 Agent ⇒ 本链该来源多为空；保留它是为了让两链
//      共用同一份脚本时不丢原链语义。
//   ⚠ 两个来源**都必须有边**：只加 `input_mapping` 不加边会读到尚未执行的 Null
//     （DAG 只保证「有边才等」），且全程零报错。

// ── 辅助函数（Rhai 要求函数位于全局作用域）──
fn present(x) { type_of(x) != "()" }

// has: 是否为可用数值。Rhai 整数宽度随构建而异（本项目为 "i64"，32-bit 为 "int"），都接。
// ⚠ 字符串不算可用数值：上游契约是 JSON Number（ToolNode 输出经 resolve_var_path 穿透
//   `.content` 时自动 from_str 解析，终值为 Number）⇒ 若哪天改成字符串透出，
//   本函数返回 false 走降级，而不是把 "35.2" 当数字参与比较。
fn has(x) {
    let t = type_of(x);
    t == "f64" || t == "i64" || t == "int"
}

fn num_of(x, dflt) {
    if type_of(x) == "f64" { return x; }
    if type_of(x) == "i64" || type_of(x) == "int" { return x.to_float(); }
    dflt
}

fn clampf(x, lo, hi) {
    if x < lo { return lo; }
    if x > hi { return hi; }
    x
}

// tier_pct: Jev 档位标签 → 百分比数值。
// 只认 PLAN §8 拍板的 4 档（5/10/20/30），其余（含自定义文案）返回 () ⇒ 不参与夹逼。
fn tier_pct(label) {
    if type_of(label) != "string" { return (); }
    if label == "5%" || label == "5" { return 5.0; }
    if label == "10%" || label == "10" { return 10.0; }
    if label == "20%" || label == "20" { return 20.0; }
    if label == "30%" || label == "30" { return 30.0; }
    ()
}

// conf_from_category: 档位标签 → 0-100 置信度（`j-confidence` 只给 category 时的兜底）
fn conf_from_category(label) {
    if type_of(label) != "string" { return (); }
    if label == "高" { return 85.0; }
    if label == "中" { return 65.0; }
    if label == "低" { return 45.0; }
    ()
}

// ── 止损系数（**行业惯例取值，不是从数据反推**；要调只改这两行）──
// 年化波动率 ÷ √252 ≈ 日波动率；止损距离取 k = 2 倍日波动。
// A 股短线止损惯例落在 3%~12%，故对结果做该区间夹逼（低于 3% 易被噪声扫掉、
// 高于 12% 已失去止损意义）。实证：年化 35% ⇒ 日波动 2.2% ⇒ 止损 4.4%。
// √252 ≈ 15.8745（A 股每年约 244~252 个交易日，取上界口径）。
let SIGMA_PER_DAY = 15.8745;
let STOP_K = 2.0;
let STOP_MIN_PCT = 3.0;
let STOP_MAX_PCT = 12.0;

let warnings = [];
let degraded = false;

// ── ① 方向（f7 的方向词表：看多 / 看空 命中，其余归 0 = 中性）──
let direction = "中性";
if present(j_direction) && type_of(j_direction) == "string" && j_direction != "" {
    direction = j_direction;
} else {
    degraded = true;
    warnings.push("缺少 j-direction 判定 —— 方向按中性处理（f7 的方向信号为 0）");
}

// ── ② 置信度（统一为 0-100 口径）──
// portfolio-mgr 的 f7 期望 0-100（脚本内 `conf > 1.0` 才自动 /100），而 Jev 的
// `confidence` 是 0-1 ⇒ 必须在此统一单位，否则 f7 幅度被压到几乎为 0。
let confidence = ();
if has(j_confidence) {
    let c = num_of(j_confidence, 0.0);
    confidence = if c <= 1.0 { c * 100.0 } else { c };
} else if present(j_confidence) && type_of(j_confidence) == "string" {
    confidence = conf_from_category(j_confidence);
} else if present(j_conviction) {
    // 无数值置信度时退到强度档（强 / 中 / 弱）
    if j_conviction == "强" { confidence = 80.0; }
    else if j_conviction == "中" { confidence = 60.0; }
    else if j_conviction == "弱" { confidence = 40.0; }
}
if !present(confidence) {
    degraded = true;
    warnings.push("缺少置信度（j-confidence 的数值/档位与 j-conviction 均不可用）");
}

// ── ③ 风险等级（f7 的 risk_mod 按 极高/高/中/低 关键词匹配）──
let risk_level = ();
if present(j_risk_level) && type_of(j_risk_level) == "string" && j_risk_level != "" {
    risk_level = j_risk_level;
} else {
    warnings.push("缺少 j-risk-level 判定 —— f7 的风险调节按中性处理");
}

// ── ④ 目标价 / 止损价 ──
// 哨兵 -999.0 **只参与判定，绝不进输出**（同 risk-level.rhai 的纪律：不把「缺失」
// 伪装成一个真实读数）。
let price_ok = has(current_price) && num_of(current_price, 0.0) > 0.0;
let cp = if price_ok { num_of(current_price, 0.0) } else { 0.0 };

// 目标价：现价 × (1 + DCF 上行空间%)。DCF 给出下行空间（≤0）或缺失时改用 Jev 的
// `j-target` 档位（PLAN §8 的 4 档）；两者都没有 ⇒ null（下游走 odds fallback）。
// `j-target` 档位同时作为**上限夹逼**：DCF 上行空间超过档位时按档位截断并留痕
// （档位是 Jev 对该标的上行空间的上界判断，DCF 的理论空间不应越过它）。
let target_price = ();
if price_ok {
    let tier_t = tier_pct(j_target);
    let upside = if has(dcf_upside) { num_of(dcf_upside, -999.0) } else { -999.0 };
    if upside <= 0.0 {
        if present(tier_t) {
            upside = tier_t;
            warnings.push("DCF 上行空间不可用或非正 —— 目标价改用 j-target 档位");
        }
    } else if present(tier_t) && upside > tier_t {
        upside = tier_t;
        warnings.push("DCF 上行空间超过 j-target 档位 —— 目标价按档位截断");
    }
    if upside > 0.0 {
        target_price = cp * (1.0 + upside / 100.0);
    }
} else {
    degraded = true;
    warnings.push("缺少 currentPrice —— 目标价与止损价均无法计算");
}

// 止损价：现价 × (1 − 止损距离%)；止损距离优先由年化波动率推导，
// 缺失时退到 `j-stop` 档位，`j-stop` 档位同时作为上限夹逼。
let stop_loss = ();
if price_ok {
    let tier_s = tier_pct(j_stop);
    let stop_pct = if has(volatility) {
        clampf(STOP_K * num_of(volatility, 0.0) / SIGMA_PER_DAY, STOP_MIN_PCT, STOP_MAX_PCT)
    } else {
        -999.0
    };
    if stop_pct <= 0.0 {
        if present(tier_s) {
            stop_pct = tier_s;
            warnings.push("缺少年化波动率 —— 止损距离改用 j-stop 档位");
        }
    } else if present(tier_s) && stop_pct > tier_s {
        stop_pct = tier_s;
        warnings.push("波动率推导的止损距离超过 j-stop 档位 —— 按档位截断");
    }
    if stop_pct > 0.0 {
        stop_loss = cp * (1.0 - stop_pct / 100.0);
    }
}

// ── ⑤ 周期 / 持有天数（原链由 trader 的 LLM 判定给出，快速链由 Jev 强度档派生）──
// `portfolio-mgr.rhai` 的取值条件是「time_horizon 与 holding_days 同时存在且
// holding_days > 0」才采信 trader 的值，否则按 posterior 分档兜底
// ⇒ 缺失是**安全**的（只是丢掉 Jev 的周期判断），故不强造默认值。
let time_horizon = ();
let holding_days = ();
if present(j_conviction) {
    if j_conviction == "强" {
        time_horizon = "long";
        holding_days = 20.0;
    } else if j_conviction == "中" {
        time_horizon = "mid";
        holding_days = 10.0;
    } else if j_conviction == "弱" {
        time_horizon = "short";
        holding_days = 5.0;
    }
}

// ── ⑥ 证据清单（**必须数组**）──
// 来源 ①：段 B 的 10 个维度判定 —— 非中性判定就是本次决策依赖的论据。
// 每项渲染为 `节点id=判定`（如 `j-market=支持`）；中性项**不入清单**（证据的作用是
// 解释「凭什么这么判」，把中性项也列进去只会稀释它）。
let evidence_cited = [];
if present(j_dimensions) && type_of(j_dimensions) == "array" {
    for d in j_dimensions {
        if type_of(d) == "map" {
            let cat = d["category"];
            if type_of(cat) == "string" && cat != "" && cat != "中性" {
                let nid = d["node_id"];
                if type_of(nid) == "string" && nid != "" {
                    evidence_cited.push(nid + "=" + cat);
                } else {
                    evidence_cited.push(cat);
                }
            }
        }
    }
}
// 来源 ②：`data-quality` 逐维度诊断里 `status == "normal"` 的维度名（原链语义，本链多为空）。
if present(dq_diagnostics) && type_of(dq_diagnostics) == "array" {
    for d in dq_diagnostics {
        if type_of(d) == "map" {
            let st = d["status"];
            let nm = d["name"];
            if type_of(st) == "string" && st == "normal" && type_of(nm) == "string" && nm != "" {
                evidence_cited.push(nm);
            }
        }
    }
}
if evidence_cited.len() == 0 {
    warnings.push("证据清单为空（段 B 判定全为中性且无可用诊断）—— f7 的证据修正按 0.4 下界处理");
}

// ── 输出 ──
#{
    content: #{
        verdict: #{
            verdict: direction,
            confidence: confidence,
            riskLevel: risk_level,
            targetPrice: target_price,
            stopLoss: stop_loss,
            timeHorizon: time_horizon,
            expectedHoldingDays: holding_days,
            evidence_cited: evidence_cited,
        },
        degraded: degraded,
        warnings: warnings,
    }
}
