// portfolio-risk-gate.rhai — P1-E13 组合风控门
//
// 在 portfolio-mgr 之后、rule-check 之前运行。
// 职责：
//   1. 单股仓位上限检查（PositionLimits::max_single_stock_pct）
//   2. 行业暴露上限检查（max_sector_exposure_pct）
//   3. 持仓数量上限检查（max_total_positions）
//   4. 风险档位否决（RiskTier::forbid_new_position）
//   5. 空头目标价强制卖出（BEARISH_TARGET_PRICE_RATIO=0.85）
//   6. 组合总仓位归一化（normalize_position_weights）
//
// 设计原则：
//   - 保留 portfolio-mgr 的所有输出字段，仅覆盖被调整的 action/positionPct/riskLevel
//   - 追加 risk_gate 元数据（adjusted/reasons/concentrationWarning 等）
//   - 保守降级：任何异常都不阻塞决策，透传 portfolio-mgr 原始结果
//
// 输入映射（input_mapping）：
//   pm_result       ← portfolio-mgr.result           （整个 portfolio-mgr 输出对象）
//   current_price   ← t-scoring.result.currentPrice   （当前价）
//   target_price    ← trader.content.targetPrice       （目标价，用于空头检测）
//   stock_code      ← stock_code                       （工作流变量，core.rs 注入）
//   stock_sector    ← stock_sector                     （工作流变量，core.rs 注入）
//   holdings_json   ← holdings_json                    （工作流变量，core.rs 注入）
//   portfolio_cash  ← portfolio_cash                   （工作流变量，core.rs 注入）

// ── Rhai 全局辅助函数（必须位于全局作用域，Rhai 不允许在 try/块内定义函数）──
fn present(x) { type_of(x) != "()" }

fn safe_str(x) {
    if !present(x) { return ""; }
    if type_of(x) == "string" { x } else { "" }
}

fn safe_f64(x, fallback) {
    if !present(x) { return fallback; }
    if type_of(x) == "f64" { x } else { fallback }
}

fn safe_opt_f64(x) {
    if !present(x) { return (); }
    if type_of(x) == "f64" { x } else { () }
}

// P1-E13 修复: 返回 Option<&str> 而非 &str
// 避免 safe_str 把未设置的 stock_sector 转为 ""，
// 导致 Rust 侧 Option<&str> 收到 Some("") 误判为有效行业。
// 未设置或空字符串 → () (即 Rust None)
fn safe_opt_str(x) {
    if !present(x) { return (); }
    if type_of(x) != "string" { return (); }
    if x == "" { return (); }
    x
}

fn safe_bool(x, fallback) {
    if !present(x) { return fallback; }
    if type_of(x) == "bool" { x } else { fallback }
}

// 将 reasons 数组拼接为分号分隔的字符串
fn join_reasons(arr) {
    if type_of(arr) != "array" { return ""; }
    let text = "";
    for i in 0..arr.len() {
        let r = arr[i];
        if type_of(r) == "string" {
            if text == "" {
                text = r;
            } else {
                text = `${text}; ${r}`;
            }
        }
    }
    text
}

// 是否为可识别的操作档位名（6 档 + 两个中文哨兵）。
//
// 写成显式循环而不是 `Array::contains` / 正则：共享 Engine 未注册 `regex` 包，
// 且各脚本对标准库特性的可见性不一致（`data-quality.rhai` 就踩过
// `replace` 未注册的坑）。本文件也不引入新的标准库依赖。
fn is_known_action_label(s) {
    for k in ["买入", "增持", "持有", "观望", "减持", "卖出", "不确定", "数据缺失"] {
        if s == k { return true; }
    }
    false
}

// 把 `reasoning` 开头的 `决策=<档名>` 对齐到**展示档**。
//
// 背景（2026-09-21）：本门覆盖 `action` / `positionPct` 后，`reasoning` 首段的
//   结论名与 `positionState` 仍是 portfolio-mgr 的原值 ⇒ 同一条落库记录内部
//   三处互相矛盾。前端 `alignReasoningDecisionLabel` 只兜「中性档 + 空仓」那种
//   派生差异，兜不住「本门把 买入/持有 改成 观望」这类**跨档覆盖**，
//   何况消费端对齐只是补丁：落库文本与任何直读 DB 的消费端仍拿到错值。
//   必须在源头自洽。
//
// ⚠️ 只改开头这**一个** token。同段文本里 `| [风控门] R-200 …，持有→观望`
//    是**档位迁移留痕**，用原始档名才表达得对，一律不得改写。
// ⚠️ 前缀不是 `决策=`、结构不可解析、或档名识别不出 ⇒ **原样返回**，
//    不臆造结论（识别失败时改写等于把「看不懂」当成「已确认」）。
// 2026-09-22: 形参由 `display_action` 改名 `target_action` —— 展示档派生已废除，
//   本函数现在的目标档就是**方向档** `final_action`（见调用点注释）。
fn align_decision_label(reasoning, target_action) {
    if type_of(reasoning) != "string" || reasoning == "" { return reasoning; }
    // 锚点**全部**取自 `index_of`，不使用任何字面量偏移量：
    //   `sub_string` 与 `index_of` 必然同坐标系（无论按字符还是按字节），
    //   而字面量偏移在二者不一致时会静默切错位置 —— 中文 UTF-8 下
    //   「决策=」是 3 个字符但 9 个字节，写死任一个都会在另一坐标系下错位。
    let eq = reasoning.index_of("=");
    if eq < 0 { return reasoning; }
    if reasoning.sub_string(0, eq) != "决策" { return reasoning; }
    // 结论头与后续内容的分隔空白（portfolio-mgr 的模板是 `决策=X 置信=…`）
    let sep = reasoning.index_of(" ");
    if sep <= eq { return reasoning; }
    let old_label = reasoning.sub_string(eq + 1, sep - eq - 1);
    if !is_known_action_label(old_label) { return reasoning; }
    if old_label == target_action { return reasoning; }
    // 长度参数只需「足够大到取完剩余字符」，故 `len()` 无论返回字节数还是
    //   字符数都安全（`sub_string` 对越界长度做截断）。
    let tail = reasoning.sub_string(sep, reasoning.len());
    `决策=${target_action}${tail}`
}

// ── 防御性兜底：任何 Rhai 运行时错误都返回可解析的保守决策 ──
let gate_result = #{
    "action": "观望",
    "positionPct": 0.0,
    "confidence": 50.0,
    "riskLevel": "中风险",
    "reasoning": "portfolio-risk-gate 兜底：未执行",
};

try {

// 1. 读取 portfolio-mgr 结果
if !present(pm_result) || type_of(pm_result) != "map" {
    // portfolio-mgr 无输出 — 保守降级
    gate_result = #{
        "action": "观望",
        "positionPct": 0.0,
        "confidence": 50.0,
        "riskLevel": "中风险",
        "reasoning": "portfolio-risk-gate: portfolio-mgr 无输出，保守降级",
        "risk_gate": #{
            "adjusted": true,
            "originalAction": "",
            "originalPositionPct": 0.0,
            "reasons": ["portfolio-mgr 无输出"],
            "concentrationWarning": "",
            "topHoldingPct": 0.0,
            "maxSectorPct": 0.0,
        },
    };
} else {

let orig_action = safe_str(pm_result["action"]);
let orig_position_pct = safe_f64(pm_result["positionPct"], 0.0);
let orig_risk_level = safe_str(pm_result["riskLevel"]);

// 2. 调用 pm_portfolio_risk_gate 纯函数（注册在共享 Rhai Engine 中）
let gate_json = pm_portfolio_risk_gate(
    orig_action,
    orig_position_pct,
    orig_risk_level,
    safe_f64(current_price, 0.0),
    safe_opt_f64(target_price),
    safe_str(stock_code),
    safe_opt_str(stock_sector),
    safe_str(holdings_json),
    safe_f64(portfolio_cash, 0.0)
);

let gate = json_parse(gate_json);

if type_of(gate) != "map" {
    // 解析失败 — 透传 portfolio-mgr 结果，追加错误标记
    gate_result = pm_result;
    gate_result["risk_gate"] = #{
        "adjusted": false,
        "originalAction": orig_action,
        "originalPositionPct": orig_position_pct,
        "reasons": ["风控门 JSON 解析失败，未做调整"],
        "concentrationWarning": "",
        "topHoldingPct": 0.0,
        "maxSectorPct": 0.0,
    };
} else {
    let final_action = safe_str(gate["action"]);
    let final_position_pct = safe_f64(gate["positionPct"], orig_position_pct);
    let adjusted = safe_bool(gate["adjusted"], false);
    let concentration_warning = safe_str(gate["concentrationWarning"]);
    let top_holding_pct = safe_f64(gate["topHoldingPct"], 0.0);
    let max_sector_pct = safe_f64(gate["maxSectorPct"], 0.0);
    let reasons = gate["reasons"];

    // 3. 输出：保留 portfolio-mgr 的所有字段，覆盖被调整的字段，追加 risk_gate 元数据
    //    Rhai 的 map 赋值是值拷贝，修改 gate_result 不影响 pm_result
    //
    // ── 2026-09-21: 覆盖 action / positionPct 后，与它们**同源**的字段必须同步重算 ──
    // 本门此前只改 `action` 与 `positionPct`，另**三个下游面**留在 pm 原值上，
    //   ⇒ 同一条落库记录内部互相矛盾：
    //     ① `positionState`：R-200 把仓位砍到 0（或空头强制卖出）后，状态轴仍是
    //        `HOLDING` ⇒ 展示轴自身与仓位矛盾（2026-09-22 前还会经前端派生把
    //        已改成「观望」的档翻回「持有」，该派生已废除）。
    //     ② `reasoning` 首段 `决策=<档名>`：仍是 pm 的原档。
    //     ③ `stopLossPct` / `takeProfitPct`：pm 的判据是 `position_pct > 0.0`，
    //        仓位归零后应为 0，否则会给「无仓位」的决策挂一个止损/止盈位。
    // 判据与 `portfolio-mgr.rhai` 逐字一致 —— 改这里请同步该文件与前端
    //   `src/lib/stock-analysis-utils.ts::resolveDisplayAction`。
    // ⚠️ 状态用**未 round** 的 `final_position_pct` 判定，与 portfolio-mgr 同口径
    //   （它也用未 round 的 pct 判 state，只有输出才 round）。
    let final_position_pct_rounded = (final_position_pct * 10.0).round() / 10.0;
    let gate_position_state = if final_position_pct <= 0.0 {
        "EMPTY"
    } else if final_action == "买入" || final_action == "增持" {
        "OPENING"
    } else if final_action == "减持" || final_action == "卖出" {
        "TRIMMING"
    } else {
        "HOLDING"
    };
    // 2026-09-22: 原此处还有一份 `gate_display_action` 派生（中性档 + EMPTY ⇒ 观望、
    //   否则 ⇒ 持有），与 `portfolio-mgr.rhai` 里那份是**同源副本**，本次一并废除 ——
    //   用**本次决策算出的建议仓位**反推**本次展示名**属**循环判据**（见
    //   `AUDIT-300642-run-variance-2026-09-22.md`）。结论名统一 = 方向档 `final_action`。
    //   ⚠️ `positionState` **仍按新仓位重算**（上方保留）：它是独立展示轴，
    //      不重算会让「仓位已归零」的记录仍显示 `HOLDING`。
    //   ⚠️ 前端 `resolveDisplayAction` 与 `portfolio-mgr.rhai` 已同步为「方向档恒等」，
    //      三处判据必须同进退（本文件此前就因漏改而产生过同源矛盾）。

    gate_result = pm_result;
    gate_result["action"] = final_action;
    gate_result["positionPct"] = final_position_pct_rounded;
    gate_result["positionPctNormalized"] = final_position_pct_rounded;
    gate_result["positionState"] = gate_position_state;
    if final_position_pct <= 0.0 {
        gate_result["stopLossPct"] = 0.0;
        gate_result["takeProfitPct"] = 0.0;
    }

    // 如果风控门调整了决策，追加 reasoning 说明
    if adjusted {
        let reasons_text = join_reasons(reasons);
        let orig_reasoning = safe_str(pm_result["reasoning"]);
        if reasons_text != "" {
            gate_result["reasoning"] = `${orig_reasoning} | [风控门] ${reasons_text}`;
        }
    }

    // ⚠️ 结论名对齐放在**所有 reasoning 拼接之后**：`join_reasons` 产出的
    //   `R-200 极高风险档位禁止持仓，持有→观望` 属档位迁移留痕，用原始档名才
    //   表达得对；对齐函数只看开头那一个 token，故顺序不改正确性，放末尾只为了让
    //   「先拼后改」的因果对读者可见。
    // 2026-09-22: 目标档由已废除的 `gate_display_action` 改为**方向档** `final_action`。
    gate_result["reasoning"] = align_decision_label(
        safe_str(gate_result["reasoning"]),
        final_action
    );

    gate_result["risk_gate"] = #{
        "adjusted": adjusted,
        "originalAction": safe_str(gate["originalAction"]),
        "originalPositionPct": safe_f64(gate["originalPositionPct"], 0.0),
        "reasons": reasons,
        "concentrationWarning": concentration_warning,
        "topHoldingPct": top_holding_pct,
        "maxSectorPct": max_sector_pct,
    };
}

} // end if pm_result present

}
catch (err) {
    // 异常兜底：如果 pm_result 存在则透传，否则返回保守决策
    if present(pm_result) && type_of(pm_result) == "map" {
        gate_result = pm_result;
        gate_result["risk_gate"] = #{
            "adjusted": false,
            "originalAction": "",
            "originalPositionPct": 0.0,
            "reasons": ["风控门异常: " + err],
            "concentrationWarning": "",
            "topHoldingPct": 0.0,
            "maxSectorPct": 0.0,
        };
    } else {
        gate_result = #{
            "action": "观望",
            "positionPct": 0.0,
            "confidence": 50.0,
            "riskLevel": "中风险",
            "reasoning": "portfolio-risk-gate 异常且 portfolio-mgr 无输出: " + err,
            "risk_gate": #{
                "adjusted": false,
                "originalAction": "",
                "originalPositionPct": 0.0,
                "reasons": ["风控门异常: " + err],
                "concentrationWarning": "",
                "topHoldingPct": 0.0,
                "maxSectorPct": 0.0,
            },
        };
    }
}

gate_result
