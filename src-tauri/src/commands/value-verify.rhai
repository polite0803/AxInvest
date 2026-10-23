// value-verify.rhai — 价值投资（巴菲特框架）输出的**确定性校验 + 原地覆写**
//
// 归属：`stock-analysis` 模板节点 `value-verify`（CodeNode + Rhai），置于 value-investor 之后。
//
// ## 为什么需要本节点
//
// 提示词层已三次收口（v89/v90 + `value-investor.md` 两层门禁），但**实证仍失效**：
//   · 601399（运行 `de6b0594`）：`dcf.assumptions.applicable=false`（模型前提不成立），
//     节点仍把 `intrinsic_value_range` 填成「0.69-0.94元（…仅供参考）」；
//   · 601166：算法给 +143% 上行空间，同一条输出里 LLM 给「观望 / 0% 仓位」；
//   · 300285：`dcf.mid` 从 88 改到 89，`intrinsic_value_range` 却从 8.37 跳到 17.35。
// ⇒ 结论：「引用算法字段」这条约束**不能靠提示词保证**，必须在结构上强制。
//
// ## 覆写机制（关键）
//
// 本节点的 `output_var` 与 value-investor **同名**（`"value-investor"`）。引擎按
// `output_var` 写 `workflow.results`（`rt-workflow/.../engine/mod.rs`），因此下游全部自动
// 读到修正值：research-mgr 的 `context_sources`、blackboard 快照（`build_blackboard_snapshot`
// 对本节点的 `result` 分支会序列化成 JSON 字符串）、落库快照、报告命令。
//
// ## 契约（只覆写**数值**字段；`verdict` / `report` 等定性字段一律不碰）
//
// ⚠ v175（2026-10-23）新增**形态归一**（`normalize_verdict_shape`）：本节点是估值链的
// 唯一确定性出口，`verdict` 的**形态**也属于本节点的契约 —— 见 "verdict 形态归一" 段。
// 归一不改变任何数值/措辞，只决定「业务字段嵌在 `verdict` 里还是顶层」。
//
//   · `intrinsic_value_range` ≡ `t-valuation.result.dcf.{low}-{high}`
//   · `margin_of_safety`      ≡ `t-valuation.result.dcf.upsidePct`
//   · `ideal_buy_price`       ≡ `t-valuation.result.dcf.idealBuyPrice`
//   · **硬不可用**（`dcf.available=false` / `upsidePct` 缺失 / `assumptions.applicable=false`）
//     ⇒ 区间与安全边际落 `null`、理想买入价落固定文案（见 `hard_unavail_ideal_text()`）。
//     含义：估值不可用 ≠ 估值为 0，把 null 当 0 是数据语义污染。
//   · **软衰减**（`assumptions.is_fallback_anchor=true`）⇒ 代理锚仍是数值，**可以引用**，
//     不在本节点处置范围（置信度衰减由提示词层承担，口径见 `mcp_tools.rs` 的 `applicable` 注释）。
//
// ## 合规判定（为什么不是「一律覆写」）
//
// 算法值的**文本形态**出现在 LLM 字段里即视为合规 —— 保留 LLM 更丰富的措辞与免责声明
// （如「0.69-0.94元（历史代理锚…）」），只有不命中才覆写。
// ⚠ 已知局限：用子串匹配代替数值解析（Rhai 沙箱无 regex）。整数形态的算法值（如 `10`）
// 可能被 `110-120` 这类文本误判为合规，方向是**少覆写**（保守），不会产生幻觉数字。
//
// ## 解析失败
//
// `value-investor.content` 不是合法 JSON 时（OutputMode::Json 下罕见）：**不硬失败、不覆写**，
// 降级为 `{report: <原文>, verdict: {}, valuation_audit: [skipped]}`，保住正文可见性。
//
// ## verdict 形态归一（v175 / 2026-10-23 实证）
//
// **两种形态曾经并存，且都没有归一**：
//   · `value-investor.md`「输出格式」要求**扁平 JSON**（`verdict` 是方向枚举**字符串**、
//     `bull_score`/`bear_score` 在**顶层**）；
//   · 其余分析师（`a-*`）、`trader`、`risk-convergence` 的产出是 **`{report, verdict: {…}}`
//     嵌套**，前端也早按嵌套读（`AnalystReportCard` 738-746 提升、`AnalystReportGrid` 529、
//     `stock-analysis/utils.ts:36`）。
//
// 三处实证后果（都发生在归一之前）：
//   ① `build_analyst_input_mapping`（`stock_analysis_setup/mod.rs`）对 value-investor 拼
//      `{node}.result.verdict.{bull,bear}_score` ⇒ 扁平形态下该跳恒 `None`，
//      `risk-agg` / `risk-con` / `risk-neu` 各 4 条 `input_mapping` **全部静默失效**
//      （日志 `Agent 节点 input_mapping 取不到值`）；而这三个节点的 `context_sources`
//      **故意不含** value-investor ⇒ 价值维度对风险评估是**零信息**。
//   ② `reconstruct_verdict_text`（Rust，`analysis-engine/blackboard.rs`）与
//      `reconstructVerdictTag`（TS，`lib/agentOutput.ts`）把字符串 verdict 直接拼进
//      `<!-- VERDICT: … -->`：Rust 侧产出 JSON **标量**（`"看多"`），TS 侧产出**非法 JSON**
//      （`看多`）⇒ 前端 `tryParseVerdictFormat` 解析失败后退化成关键词扫描，
//      同一份报告在结构化通道与关键词通道可能得出相反结论。
//   ③ 快照/实时两条路径对同一份产出的读法不同（②两处实现各读各的形态）。
//
// 处置 = **镜像归一**（不是迁移）：`verdict` 一律写回 **map**，九个业务字段
// （`verdict` / `bull_score` / `bear_score` / `confidence` / `bull_points` / `bear_points`
//  + 本节点覆写的 `intrinsic_value_range` / `margin_of_safety` / `ideal_buy_price`）
// 在 map 内补齐；**顶层业务字段一个不删**（`ValueAssessmentPanel.flattenVerdictReport`、
// `stock-analysis-export` 仍按顶层读）⇒ 旧读法全部保持可用。
// map 内**已有值不覆盖**：LLM 若真的输出了嵌套形态，尊重它的值。
// 留痕：`verdict_shape`（nested / nested(empty) / flat->nested）+ `verdict_shape_mirrored`
// + `valuation_audit` 一条记录 ⇒ 事后可区分「本来就是嵌套」与「本次归一而成」。
//
// ## 维护
//
// 本脚本经 `include_str!` 嵌入 `seed_stock_analysis.rs` 的节点 `code` 字段 ⇒ 改动必须
// bump `TEMPLATE_VERSION`（否则版本门 `existing.version >= TEMPLATE_VERSION ⇒ skip`
// 让存量库永不重播种）。

/// 变量是否存在（Rhai 的「空」是 unit，不是 null）。
fn present(x) { type_of(x) != "()" }

/// 剥 ```json / ```tool_json 围栏（无围栏时原样返回）。
/// `json_parse` 不剥围栏，LLM 偶尔带围栏输出会整体解析失败。
fn strip_fence(s) {
    if s.contains("```") {
        let nl = s.index_of("\n");
        let rest = if nl >= 0 { s.sub_string(nl + 1, s.len() - nl - 1) } else { s };
        let fence_end = rest.index_of("```");
        if fence_end >= 0 { rest.sub_string(0, fence_end) } else { rest }
    } else {
        s
    }
}

/// 宽容解析：map 原样透传，字符串剥围栏后 `json_parse`，失败返回 unit。
/// 形态与 `data-quality.rhai` / `candidate-assembler.rhai` 一致。
fn safe_parse(raw) {
    if !present(raw) { return (); }
    if type_of(raw) == "map" { return raw; }
    if type_of(raw) != "string" || raw == "" { return (); }
    try { json_parse(strip_fence(raw)) } catch (err) { () }
}

/// 任意值 → 可读文本（审计留痕用）；unit → ""。
fn as_text(v) {
    if !present(v) { return ""; }
    if type_of(v) == "string" { return v; }
    v.to_string()
}

/// 算法数值 → 用于文本比对的字符串；非数值返回 ""。
fn fmt_num(v) {
    if type_of(v) == "f64" { return v.to_string(); }
    if type_of(v) == "i64" { return v.to_string(); }
    if type_of(v) == "string" { return v; }
    ""
}

/// 硬不可用时的理想买入价固定文案（与 `feature/value-investor.md` 层① 的两档措辞对齐）。
fn hard_unavail_ideal_text() {
    "无算法估值锚（需清算价值/重置成本等替代方法）"
}

/// 一条审计记录：字段名 / LLM 原值 / 算法值 / 处置（ok | overwritten | skipped）。
fn audit_entry(field, llm_text, algo_text, action) {
    #{
        "field": field,
        "llm": llm_text,
        "algorithmic": algo_text,
        "action": action,
    }
}

/// 校验并原地修正 root（同层写回：`verdict` 嵌套时改 `verdict` 内的字段）。
///
/// ⚠️ Rhai 的 map 索引取值是**拷贝**，故须把 `fields` 写回 `root["verdict"]`，
/// 否则修正只落在临时副本上（表现为「脚本跑了但结果没变」）。
fn verify(root, dcf_low, dcf_high, dcf_upside, dcf_ideal, dcf_available, dcf_applicable) {
    let nested = type_of(root["verdict"]) == "map";
    let fields = if nested { root["verdict"] } else { root };

    // 硬不可用三态（与 `seed_stock_analysis.rs` / `value-investor.md` 的【硬不可用】同源）：
    //   ① dcf.available == false
    //   ② dcf.upsidePct 缺失（不可用的重灾区：算法侧把 low/mid/high 一并遮蔽为 null）
    //   ③ dcf.assumptions.applicable == false（模型前提不成立：负债率 >80% /
    //      真实现金流与盈利量级脱钩或符号相反）
    let hard_unavailable = (present(dcf_available) && dcf_available == false)
        || !present(dcf_upside)
        || (present(dcf_applicable) && dcf_applicable == false);

    let audit = [];

    if hard_unavailable {
        // ① 内在价值区间 → null
        let cur = fields["intrinsic_value_range"];
        if present(cur) {
            audit.push(audit_entry("intrinsic_value_range", as_text(cur), "null", "overwritten"));
            fields["intrinsic_value_range"] = ();
        } else {
            audit.push(audit_entry("intrinsic_value_range", "null", "null", "ok"));
        }

        // ② 安全边际 → null
        let cur2 = fields["margin_of_safety"];
        if present(cur2) {
            audit.push(audit_entry("margin_of_safety", as_text(cur2), "null", "overwritten"));
            fields["margin_of_safety"] = ();
        } else {
            audit.push(audit_entry("margin_of_safety", "null", "null", "ok"));
        }

        // ③ 理想买入价 → 固定文案
        let fixed = hard_unavail_ideal_text();
        let cur3 = as_text(fields["ideal_buy_price"]);
        if cur3 == fixed {
            audit.push(audit_entry("ideal_buy_price", cur3, fixed, "ok"));
        } else {
            audit.push(audit_entry("ideal_buy_price", cur3, fixed, "overwritten"));
            fields["ideal_buy_price"] = fixed;
        }
    } else {
        // ① 内在价值区间 ≡ dcf.{low}-{high}
        let low_s = fmt_num(dcf_low);
        let high_s = fmt_num(dcf_high);
        let cur = fields["intrinsic_value_range"];
        if low_s == "" || high_s == "" {
            // 可用态却缺三档（数据不全）⇒ 不猜、不写，只留痕
            audit.push(audit_entry("intrinsic_value_range", as_text(cur), "", "skipped"));
        } else {
            let range_text = low_s + "-" + high_s + "元";
            let ok = type_of(cur) == "string" && cur.contains(low_s) && cur.contains(high_s);
            if ok {
                audit.push(audit_entry("intrinsic_value_range", as_text(cur), range_text, "ok"));
            } else {
                audit.push(
                    audit_entry("intrinsic_value_range", as_text(cur), range_text, "overwritten"),
                );
                fields["intrinsic_value_range"] = range_text;
            }
        }

        // ② 安全边际 ≡ dcf.upsidePct（算法侧与 margin_of_safety.pct 同源同值）
        let up_s = fmt_num(dcf_upside);
        let cur2 = as_text(fields["margin_of_safety"]);
        if up_s == "" {
            audit.push(audit_entry("margin_of_safety", cur2, "", "skipped"));
        } else {
            let margin_text = up_s + "%";
            if cur2 != "" && cur2.contains(up_s) {
                audit.push(audit_entry("margin_of_safety", cur2, margin_text, "ok"));
            } else {
                audit.push(audit_entry("margin_of_safety", cur2, margin_text, "overwritten"));
                fields["margin_of_safety"] = margin_text;
            }
        }

        // ③ 理想买入价 ≡ dcf.idealBuyPrice（= DCF 中性档 × 0.70）
        let ideal_s = fmt_num(dcf_ideal);
        let cur3 = as_text(fields["ideal_buy_price"]);
        if ideal_s == "" {
            audit.push(audit_entry("ideal_buy_price", cur3, "", "skipped"));
        } else {
            let ideal_text = ideal_s + "元以下";
            if cur3 != "" && cur3.contains(ideal_s) {
                audit.push(audit_entry("ideal_buy_price", cur3, ideal_text, "ok"));
            } else {
                audit.push(audit_entry("ideal_buy_price", cur3, ideal_text, "overwritten"));
                fields["ideal_buy_price"] = ideal_text;
            }
        }
    }

    // 同层写回（见函数头注释：Rhai map 索引取值是拷贝）
    if nested { root["verdict"] = fields; }
    root["valuation_audit"] = audit;
    root
}

/// `verdict` 形态归一（v175 / 2026-10-23）—— 把扁平产出的核心六键**镜像**进 `verdict` map。
///
/// 为什么是镜像而不是迁移：顶层业务字段是**既有**读法（`ValueAssessmentPanel` 的
/// `flattenVerdictReport` 只提升「顶层没有的键」、`stock-analysis-export` 直接读顶层
/// `bull_score`），删掉顶层会打破它们；而 `verdict` 是**新**契约（`input_mapping` 的
/// `.verdict.*` 与前端嵌套读法）。两侧都要成立 ⇒ 双写，由本函数作为唯一写入点。
///
/// 为什么 map 内已有值不覆盖：LLM 若原本就输出了嵌套形态，那份值才是它自己的口径；
/// 用顶层同名字段覆盖会把「LLM 的两处结论不一致」变成「被脚本悄悄抹平」—— 那属于
/// 隐藏证据，不是归一。此处只在**缺键**时补，并在 `valuation_audit` 里记下补了哪些。
fn normalize_verdict_shape(root) {
    // 镜像键集合 = 「a-* 分析师放进 verdict 的那些业务字段」的 value 版：
    //   方向/打分（计票 + `input_mapping` 消费）、论据（卡片渲染）、
    //   估值三字段（本节点刚覆写的权威值 —— 扁平形态下 `verify()` 把它们写在顶层，
    //   归一后 `.verdict.*` 也读得到，与 `a-*` 的嵌套读法一致）。
    // ⚠ `null` 无法镜像：Rhai 里 null 与「键不存在」同为 `()`，`present()` 分不开
    //   （硬不可用时 `margin_of_safety` 落 null ⇒ map 内缺该键，顶层仍是 null）。
    //   缺键与 null 在 `resolve_var_path` / 前端 `??` 两种读法下等价，故不额外补救。
    let keys = [
        "verdict",
        "bull_score",
        "bear_score",
        "confidence",
        "bull_points",
        "bear_points",
        "intrinsic_value_range",
        "margin_of_safety",
        "ideal_buy_price",
    ];
    let raw = root["verdict"];
    let was_map = type_of(raw) == "map";
    let v = if was_map { raw } else { #{} };
    let mirrored = [];
    for k in keys {
        // `verdict` 本身：只有「顶层是方向字符串」才搬进容器；顶层已是 map 时
        //   `root["verdict"]` **就是容器本身**，再搬一次会铸出 `{verdict: {verdict: {}}}`
        //   这种自嵌套（2026-10-23 首轮实测被 `value_verify_unparsable_…` 抓到）。
        if k == "verdict" && was_map { continue; }
        if present(v[k]) { continue; }
        let top = root[k];
        if !present(top) { continue; }
        v[k] = top;
        mirrored.push(k);
    }
    // ⚠ Rhai 传参是值语义（同 `verify()` 的写法）：修改须写回 root 并由调用方接返回值。
    root["verdict"] = v;
    let shape = if !was_map {
        "flat->nested"
    } else if v.len() == 0 {
        "nested(empty)"
    } else {
        "nested"
    };
    root["verdict_shape"] = shape;
    root["verdict_shape_mirrored"] = mirrored;

    let audit = root["valuation_audit"];
    if type_of(audit) == "array" {
        audit.push(audit_entry(
            "verdict",
            if was_map { "" } else { as_text(raw) },
            shape,
            if was_map { "ok" } else { "normalized" },
        ));
        root["valuation_audit"] = audit;
    }
    root
}

/// 算法估值结论（2026-09-28）—— 原样封装 `t-valuation.result.content.conclusion`
/// 与 `relative` / `reverseDcf` 的骨架字段。
///
/// 为什么在本节点**重新封装**而不是直接透传：本节点是估值链的**唯一确定性出口**，
/// 下游（research-mgr 的 context_sources、blackboard 快照、落库、报告命令）读的都是
/// 本节点的 `output_var`。把结论放在这里 ⇒ 它不依赖 LLM 是否照抄，也不依赖
/// `value-investor.content` 是否能解析成功。
fn algo_conclusion(action, headline, method, rel_verdict, rel_primary, rev_feasibility) {
    #{
        "action": as_text(action),
        "headline": as_text(headline),
        "primaryMethod": as_text(method),
        "relativeVerdict": as_text(rel_verdict),
        "relativePrimary": as_text(rel_primary),
        "reverseFeasibility": as_text(rev_feasibility),
    }
}

// ── 入口 ────────────────────────────────────────────────────────────────
// ⚠️ 顶层不用 `return`（Rhai 顶层 return 语义依版本而异），改用 if/else 表达式。
let payload = safe_parse(vi_content);

// LLM 侧的 conclusion 原值（审计用）—— 必须在 `verify()` 改写之前取。
// 取法与 `verify()` 内一致：有 `verdict` 时看 `verdict`，否则看顶层。
let llm_conclusion_before = if type_of(payload) != "map" {
    ""
} else {
    let nested = type_of(payload["verdict"]) == "map";
    as_text((if nested { payload["verdict"] } else { payload })["valuation_conclusion"])
};

let output = if type_of(payload) != "map" {
    // 解析失败：不覆写（保 LLM 原文作为 report），只留痕
    #{
        "report": as_text(vi_content),
        "verdict": #{},
        "valuation_audit": [
            audit_entry("*", "", "", "skipped"),
            #{
                "field": "*",
                "llm": "",
                "algorithmic": "",
                "action": "reason",
                "detail": "value-investor.content 不是合法 JSON，未做任何覆写",
            },
        ],
    }
} else {
    verify(payload, dcf_low, dcf_high, dcf_upside, dcf_ideal, dcf_available, dcf_applicable)
};

// ── v175（2026-10-23）：verdict 形态归一（**必须先于**结论注入）─────────────
//
// 顺序理由：`valuation_conclusion` 是**顶层**键，与 verdict 嵌套无关；放在归一之后
// 只是为了「同一份 output 只写一次」——若反过来（先注结论再归一），归一函数会把
// 已含 conclusion 的 root 再整体写回，多一次无意义的深拷贝（Rhai map 克隆）。
// 覆盖面：解析失败分支（`verdict: #{}`）同样经过本函数 ⇒ `verdict_shape` 恒存在。
output = normalize_verdict_shape(output);

// ── 2026-09-28：算法估值结论**无条件**注入（顶层，权威值）──────────────
//
// 为什么放在这里、而不是在 `verify()` 内：`verify()` 在解析失败时**不被调用**，
//   而「结论必须存在」是用户的硬约束 ⇒ 注入点必须覆盖**全部**分支。
// 为什么放顶层而不是 verdict 内：解析失败分支没有 `verdict` map（只有 `#{}`）。
//   顶层恒定存在，且 `build_blackboard_snapshot` 对「无顶层 verdict」的形态走
//   「提取 result 序列化」分支 ⇒ 顶层键同样进入快照，不丢信息。
//
// ⚠️ 本注入与 LLM 文本**无关**：headline 为空（上游未注入，如旧模板）时
//   `valuation_conclusion.headline` 为空串、审计记 `skipped`，不伪造结论。
let conclusion = algo_conclusion(
    valuation_conclusion_action,
    valuation_conclusion_headline,
    valuation_conclusion_primary_method,
    valuation_relative_verdict,
    valuation_relative_primary,
    valuation_reverse_feasibility,
);
output["valuation_conclusion"] = conclusion;

let audit_out = output["valuation_audit"];
if type_of(audit_out) == "array" {
    let algo_text = conclusion["action"] + "：" + conclusion["headline"];
    let act = if conclusion["headline"] == "" { "skipped" }
        else if llm_conclusion_before == "" { "injected" }
        else { "overwritten" };
    audit_out.push(audit_entry("valuation_conclusion", llm_conclusion_before, algo_text, act));
    output["valuation_audit"] = audit_out;
}

output
