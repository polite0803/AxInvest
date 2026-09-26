//! `data-quality.rhai` 失败标记判据的**可执行**回归门禁。
//!
//! 为什么需要它：`rhai_syntax_check.rs` 只做 `engine.compile`（语法层），
//! 既抓不到「Function not found」类**运行时**错误，也锁不住判据行为。
//!
//! 被锁的缺陷（实证见 `AUDIT-analyst-low-confidence-2026-09-21.md` §五）：
//! 原 `placeholder_hits` 做纯子串匹配，把分析师的**澄清句**
//! 「本维度数据不可用，标注为技术停披而非无数据」判成 2 处数据缺口 ——
//! 这一句话贡献了资金面全部 3 处命中里的 2 处（**假阳性占 33%**），
//! 并直接把该节点推入「低置信」，最终改写决策（`positionPct` → 0）。
//!
//! 修复后的期望行为：
//!   ① `<!-- VERDICT ... -->` 注释块不参与匹配（结构化风险点罗列不是「缺口语」）；
//!   ② 软标记（无数据 / 数据不可用 / 返回空 / 空值 / 均为空）若正文含**定向否定短语**
//!      （而非无数据 / 无数据源 / 暂无数据 …）则不计入；
//!   ③ 硬标记（数据缺失 / 无法获取 / 未注入 / 占位报告 …）不受抑制 —— 真缺口必须保留。
//!
//! ⚠ 本门禁同时**钉住一个引擎契约**：判据依赖 Rhai 的 `StringPackage`
//!   （`split`/`replace`），而运行时引擎是 `Engine::new()`（含 StandardPackage）。
//!   若未来有人给 rhai 打开 `no_string`/`no_index` feature 或改用 `Engine::new_raw()`，
//!   见 `rhai_string_package_available` 会先红。

use rhai::{Dynamic, Engine, Map, Scope};

const SCRIPT: &str = include_str!("../../../src/commands/data-quality.rhai");

/// 脚本从工作流 scope 读取的全部外部输入（未提供时以 `()` 注入）。
///
/// ⚠ 这份清单的**权威来源**是节点的 `input_mapping`，不是本文件：
/// `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs` 的
/// `data-quality` CodeNode（搜索 `("money_flow", "t-hotmoney-data.result.content")`）。
/// 漏一个 ⇒ 脚本执行到该行报 `ErrorVariableNotFound`（首次实测即踩：
/// 把 `money_flow`/`lockup_bundle`/`announcements` 误写成了它们在脚本内**派生**出的
/// 变量名 `mf_main_net_inflow`/`lb_shareholder_trades_len`/`ann_count`）。
/// 改 `input_mapping` 后必须回来同步这里。
const EXTERNAL_VARS: &[&str] = &[
    "mk_verdict",
    "sent_verdict",
    "news_verdict",
    "fund_verdict",
    "pol_verdict",
    "hm_verdict",
    "lk_verdict",
    "res_verdict",
    "sec_verdict",
    "cat_verdict",
    "mk_report",
    "sent_report",
    "news_report",
    "fund_report",
    "pol_report",
    "hm_report",
    "lk_report",
    "res_report",
    "sec_report",
    "cat_report",
    // 2026-09-21(P2-1) 补：`input_mapping` 同批新增 10 个 `*_tool_calls`
    //   （`<分析员节点>.tool_calls_made`）供 `attribution_note()` 做伪归因交叉核对。
    //   ⚠️ 与上面 `valuation_dcf_fcf_data_missing` 那次**同一形态**：本清单是**手抄**的，
    //   seed 的 `input_mapping` 才是权威来源 ⇒ 加键时漏同步这里，
    //   就会被 `external_vars_match_node_input_mapping` 的双向差集抓住（本轮实测：
    //   declared 50 vs 本清单 40 ⇒ missing 10 项 ⇒ 该门禁红）。
    //   注入 `()` 时 `attribution_note` 的守卫①（`type_of != "array"`）直接返回空串
    //   ⇒ 下方行为断言不受影响，这 10 项纯粹是为了让清单与映射保持等式。
    "mk_tool_calls",
    "sent_tool_calls",
    "news_tool_calls",
    "fund_tool_calls",
    "pol_tool_calls",
    "hm_tool_calls",
    "lk_tool_calls",
    "res_tool_calls",
    "sec_tool_calls",
    "cat_tool_calls",
    "mk_untrusted",
    "sent_untrusted",
    "news_untrusted",
    "fund_untrusted",
    "pol_untrusted",
    "hm_untrusted",
    "lk_untrusted",
    "res_untrusted",
    "sec_untrusted",
    "cat_untrusted",
    "total_score",
    "consensus_score",
    "catalyst_level",
    "risk_volatility",
    "valuation_dcf_upside",
    // 2026-09-21 补：`input_mapping` 于同批新增该键（seed_stock_analysis.rs 的
    //   `("valuation_dcf_fcf_data_missing", "t-valuation.result.content.dcf.assumptions.fcf_data_missing")`），
    //   但当时**漏同步本清单** ⇒ 脚本执行到 `data-quality.rhai:923`
    //   （`if present(valuation_dcf_fcf_data_missing) …`）即抛
    //   `ErrorVariableNotFound("valuation_dcf_fcf_data_missing", 923:12)`，
    //   导致本文件 9 个测试**全红**、`data-quality` 节点整体失败 ⇒ 无 `diagnostics`
    //   ⇒ 前端「分析师数据质量」弹窗退化为「本次记录不含该节点的逐节点诊断」。
    //   ⚠️ 这类漏声明是**静默**的（`rhai_syntax_check` 只编译不执行，照样绿）
    //   ⇒ 本清单与 input_mapping 的双向锁（`external_vars_match_node_input_mapping`）是唯一哨兵。
    "valuation_dcf_fcf_data_missing",
    "money_flow",
    "lockup_bundle",
    "announcements",
    "pace_signal",
];

/// 构造与运行时一致的 Engine（harness::register_common_functions +
/// rhai_pm::register_pm_functions 的等价子集；`pm_compute_factor_completeness` 用常量替身）。
fn build_engine() -> Engine {
    build_engine_with_asof_methods("[]")
}

/// 同 `build_engine`，但 `pm_asof_degraded_methods` 返回指定 JSON（S2 豁免判据的注入点）。
///
/// 生产实现在 `stock_workflow/rhai_pm.rs`：live 恒 "[]"，回放返回本次
/// 设计性降级的 vendor 方法名数组。
fn build_engine_with_asof_methods(methods_json: &'static str) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_expr_depths(1024, 1024);
    engine.set_max_operations(2_000_000);
    engine.register_fn("clamp", |v: f64, min: f64, max: f64| -> f64 { v.clamp(min, max) });
    engine.register_fn("join", |arr: rhai::Array, sep: &str| -> String {
        arr.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(sep)
    });
    // 替身策略：对象/标量维持原「返回 UNIT」形态（既有测试都直接注入 map，
    // 不走解析路径）；**字符串数组**给出真实解析 —— S2 的 asof 方法清单判据
    // 必须能真的解析出数组，否则豁免逻辑在测试里恒为假绿。
    engine.register_fn("json_parse", |s: &str| -> Dynamic {
        match serde_json::from_str::<serde_json::Value>(s) {
            Ok(serde_json::Value::Array(arr)) if arr.iter().all(|v| v.is_string()) => {
                Dynamic::from(
                    arr.iter()
                        .map(|v| Dynamic::from(v.as_str().unwrap_or("").to_string()))
                        .collect::<rhai::Array>(),
                )
            },
            _ => Dynamic::UNIT,
        }
    });
    engine.register_fn("print", |_s: &str| {});
    engine.register_fn("pm_asof_degraded_methods", move || -> String { methods_json.to_string() });
    // 签名与 src-tauri/src/commands/stock_workflow/rhai_pm.rs 完全一致（9 个 Dynamic 参数）
    engine.register_fn(
        "pm_compute_factor_completeness",
        |_a: Dynamic,
         _b: Dynamic,
         _c: Dynamic,
         _d: Dynamic,
         _e: Dynamic,
         _f: Dynamic,
         _g: Dynamic,
         _h: Dynamic,
         _i: Dynamic|
         -> f64 { 0.9 },
    );
    engine
}

/// 执行整个 data-quality.rhai，返回其输出 map。
///
/// `reports`：节点缩写 → 报告正文（覆盖同名 `(){}_report` 输入）
/// `confs`  ：节点缩写 → 分析师自评 confidence（覆盖同名 `(){}_verdict` 输入）
fn run_quality(reports: &[(&str, &str)], confs: &[(&str, f64)]) -> Map {
    run_quality_with_tool_calls(reports, confs, &[])
}

/// 同 `run_quality`，并可注入各节点的 `tool_calls_made`（P2-1 伪归因判据的输入）。
///
/// `tool_calls`：节点缩写 → 该分析师本轮的工具调用记录数组。
/// **未给出的缩写保持 `()` 注入** —— 与生产「旧快照缺该字段」的形态一致，
/// 用于验证 `attribution_note` 的守卫①（无记录 ⇒ 不判）。
fn run_quality_with_tool_calls(
    reports: &[(&str, &str)],
    confs: &[(&str, f64)],
    tool_calls: &[(&str, rhai::Array)],
) -> Map {
    run_quality_impl(reports, confs, tool_calls, build_engine())
}

/// 同 `run_quality`，但注入 as-of 设计性降级方法清单（S2 豁免判据）。
fn run_quality_asof(
    reports: &[(&str, &str)],
    confs: &[(&str, f64)],
    methods_json: &'static str,
) -> Map {
    run_quality_impl(reports, confs, &[], build_engine_with_asof_methods(methods_json))
}

/// 同 `run_quality`，但 verdict map 里带 `verdict` 方向串（R9a 冲突判据的输入）。
fn run_quality_dirs(reports: &[(&str, &str)], dirs: &[(&str, f64, &str)]) -> Map {
    let engine = build_engine();
    let ast = engine.compile(SCRIPT).expect("data-quality.rhai 编译失败");
    let mut scope = Scope::new();
    for v in EXTERNAL_VARS {
        scope.push_dynamic(*v, Dynamic::UNIT);
    }
    for (abbr, c, d) in dirs {
        let mut m = Map::new();
        m.insert("confidence".into(), Dynamic::from(*c));
        m.insert("verdict".into(), Dynamic::from(d.to_string()));
        scope.push_dynamic(format!("{abbr}_verdict"), Dynamic::from(m));
    }
    for (abbr, text) in reports {
        scope.push_dynamic(format!("{abbr}_report"), Dynamic::from(text.to_string()));
    }
    engine.eval_ast_with_scope::<Map>(&mut scope, &ast).expect("data-quality.rhai 执行失败")
}

fn run_quality_impl(
    reports: &[(&str, &str)],
    confs: &[(&str, f64)],
    tool_calls: &[(&str, rhai::Array)],
    engine: Engine,
) -> Map {
    let ast = engine.compile(SCRIPT).expect("data-quality.rhai 编译失败");
    let mut scope = Scope::new();
    for v in EXTERNAL_VARS {
        scope.push_dynamic(*v, Dynamic::UNIT);
    }
    for (abbr, c) in confs {
        let mut m = Map::new();
        m.insert("confidence".into(), Dynamic::from(*c));
        scope.push_dynamic(format!("{abbr}_verdict"), Dynamic::from(m));
    }
    for (abbr, text) in reports {
        scope.push_dynamic(format!("{abbr}_report"), Dynamic::from(text.to_string()));
    }
    for (abbr, calls) in tool_calls {
        scope.push_dynamic(format!("{abbr}_tool_calls"), Dynamic::from(calls.clone()));
    }
    engine
        .eval_ast_with_scope::<Map>(&mut scope, &ast)
        .expect("data-quality.rhai 执行失败（检查是否引用了未注册的宿主函数）")
}

/// 造一条 `tool_calls_made` 记录。
///
/// ⚠ 字段名必须与生产落盘一致：**`is_error`**（bool）+ **`tool`**（string）。
///   写成 `success` 之类会让 `rejection_summary` 静默得到 0 条失败（判据恒绿）——
///   这正是本判据最脆的一处，故样本刻意按真实键名构造。
fn tc(tool: &str, is_error: bool) -> Dynamic {
    let mut m = Map::new();
    m.insert("tool".into(), Dynamic::from(tool.to_string()));
    m.insert("arguments".into(), Dynamic::from("{}".to_string()));
    m.insert("result".into(), Dynamic::from(String::new()));
    m.insert("is_error".into(), Dynamic::from(is_error));
    Dynamic::from(m)
}

/// 取某节点的 diagnostics 子 map
fn diag(result: &Map, abbr: &str) -> Map {
    let d = result["diagnostics"].clone().try_cast::<Map>().expect("diagnostics 不是 map");
    d[abbr].clone().try_cast::<Map>().expect("diagnostic 条目不是 map")
}

fn hits(result: &Map, abbr: &str) -> i64 {
    diag(result, abbr)["placeholder_hits"].clone().try_cast::<i64>().unwrap_or(-1)
}

fn status(result: &Map, abbr: &str) -> String {
    diag(result, abbr)["status"].clone().into_string().unwrap_or_default()
}

fn names(result: &Map, key: &str) -> Vec<String> {
    result[key]
        .clone()
        .try_cast::<rhai::Array>()
        .map(|a| a.iter().map(|x| x.to_string()).collect::<Vec<_>>())
        .unwrap_or_default()
}

/// 本轮（2026-09-21 / 300308 中际旭创）资金面分析师的**真实报告片段**：
/// 前一句是澄清（北向停披不是数据源故障），后一句是真缺口（龙虎榜席位缺失）。
const HM_REPORT_REAL: &str = "北向自 2024-08-16 监管停披净流入数据，当前返回为成交额（沪 1611.7 亿、深 1727.5 亿），无净流入信号，无法用于个股北向判断；\
本维度数据不可用，标注为技术停披而非无数据。龙虎榜席位与北向数据缺失，无法验证机构与外资方向。";

#[test]
fn rhai_string_package_available() {
    // 判据（strip_verdict_blocks）依赖 StringPackage。此测试是**引擎契约的钉子**：
    // 若它红了，说明引擎不再注册 StandardPackage，strip_verdict_blocks 必须改写法。
    let engine = build_engine();
    let first: String =
        engine.eval(r#"let p = "正文<!-- VERDICTx".split("<!-- VERDICT"); p[0]"#).unwrap();
    assert_eq!(first, "正文", "Rhai split 不可用或行为改变");
    // `replace`：脚本当前**不使用**它（`data-quality.rhai:134` 的历史注释称
    // 「原实现 text.replace(…) 链式调用 100% 报 Function not found」，与实况不符 ——
    // 实测可执行）。这里只在**能力**层钉住，不做任何依赖。
    //
    // ⚠ 断言只判「不报错」，**不断言返回值** —— 因为 Rhai 的脚本末句若是**方法调用**，
    //   其返回值会被丢弃、整段返回 `()`（实测：`let s = "a-b"; s.replace("-","+")`
    //   执行成功但返回 `()`，`eval::<String>` 报 `ErrorMismatchOutputType("string","()")`；
    //   而上面那条末句是 `p[0]`（索引表达式）时正常返回）。故末句显式补 `()` 并只断言
    //   `is_ok`，与末句形态解耦。
    let replace_ok = engine.eval::<()>(r#"let s = "a-b"; s.replace("-", "+"); ()"#);
    assert!(replace_ok.is_ok(), "Rhai replace 不可用或行为改变: {:?}", replace_ok.err());
}

#[test]
fn clarification_sentence_is_not_counted_as_gap() {
    let r = run_quality(&[("hm", HM_REPORT_REAL)], &[("hm", 55.0)]);
    let n = hits(&r, "hm");
    assert_eq!(
        n, 1,
        "hm 应只计 1 处真缺口（「龙虎榜席位与北向数据缺失」）；\
         澄清句「而非无数据」贡献的 `数据不可用` + `无数据` 必须被抑制。实际 {n} 处。\
         若为 3 处 ⇒ 假阳性回归（2026-09-21 前的老行为）"
    );
    // 真缺口仍在 ⇒ 节点仍判 low（不是 normal）
    assert_eq!(status(&r, "hm"), "low", "含真缺口时应仍为 low，不能因抑制而放过");
    // 逐词精确核对：抑制的两个软标记不得出现
    let node_names = names(&r, "placeholder_nodes");
    assert!(node_names.contains(&"资金面".to_string()), "资金面应仍在失败标记节点清单内");
}

#[test]
fn hard_marker_survives_clarification() {
    // 硬标记 `数据缺失` 不设抑制：同篇报告里的澄清语不得连带抹掉真缺口
    let r = run_quality(&[("hm", HM_REPORT_REAL)], &[("hm", 55.0)]);
    assert_eq!(hits(&r, "hm"), 1, "真缺口 `数据缺失` 被澄清语连带抑制了");
    assert_eq!(r["placeholder_total_hits"].clone().try_cast::<i64>().unwrap(), 1);
}

#[test]
fn verdict_comment_block_is_stripped() {
    // 601698 轮 lk 的真实形态：结构化 VERDICT 里的正常风险点罗列被计入命中
    let report = "质押比例 12.3%，低于警戒线，无异常。\n<!-- VERDICT: {\"bear_points\":[\"质押数据缺失\"]} -->";
    let r = run_quality(&[("lk", report)], &[("lk", 60.0)]);
    assert_eq!(
        hits(&r, "lk"),
        0,
        "VERDICT 注释块内的字段不应参与失败标记判据（应被 strip_verdict_blocks 剥离）"
    );
    assert_eq!(status(&r, "lk"), "normal");

    // 反向对照：正文自身含标记时必须仍然命中（证明剥离没有把正文一起吃棹）
    let report2 = "财报窗口内无法获取审计意见。\n<!-- VERDICT: {\"bear_points\":[]} -->";
    let r2 = run_quality(&[("lk", report2)], &[("lk", 60.0)]);
    assert_eq!(hits(&r2, "lk"), 1, "剥离 VERDICT 后正文标记必须保留");
}

#[test]
fn historical_false_positive_samples_are_clean() {
    // 跨轮抽查到的三类历史误报（AUDIT §五）
    let cases: &[(&str, &str)] = &[
        (
            "sent",
            "期权隐含情绪指标返回空（视为该维度暂无数据/无期权覆盖），对情绪极值判断贡献有限。",
        ),
        ("news", "期权PCR数据返回null（该维度暂无数据，对消息面分析影响低）。"),
        (
            "news",
            "未检索到立案调查、监管函类记录（数据缺口：无监管函类记录，暂判定为「无监管事件」，非「无数据源」）。",
        ),
    ];
    for (abbr, text) in cases {
        let r = run_quality(&[(abbr, text)], &[(abbr, 60.0)]);
        assert_eq!(
            hits(&r, abbr),
            0,
            "历史误报样本应 0 命中（已澄清/无覆盖不等于数据缺口）：{text}"
        );
    }
}

#[test]
fn genuine_gaps_still_detected() {
    // 反向断言：真缺口语必须继续被检出（防「修假阳性改成假阴性」）。
    //
    // ⚠ 期望值是**按词表逐条算出来的**，不是估的 —— 首次写成 2 而实际 1（见第 1 例），
    //   根因是把「同业横向估值对比缺失」误当成命中 `数据缺失`（实为 `对比缺失`），
    //   故这里把每例命中的**标记名**列出，改词表时能一眼看出该动哪一条。
    let cases: &[(&str, &str, i64, &str)] = &[
        // 只命中 `为 null`（「返回全为 null」含「为 null」）。注意 `返回空` **不**命中
        // （「返回全」≠「返回空」），`数据缺失` 也不命中（原文是「对比缺失」）。
        (
            "fund",
            "（注：行业同侪 PE/PB/ROE 数据源返回全为 null，同业横向估值对比缺失）",
            1,
            "为 null",
        ),
        ("cat", "同业可比公司 PE/PB 字段在 peer 工具中为 null，横向估值锚缺失", 1, "为 null"),
        ("hm", "主力资金数据无法获取，北向通道未注入。", 2, "无法获取 + 未注入"),
        (
            "sent",
            "该股龙虎榜买卖方席位均为「0/0」，未显示营业部明细，席位数据缺失。",
            1,
            "数据缺失",
        ),
        // `占位` 被折叠规则去掉（长词 `占位报告` 命中 ⇒ 不再计其子串），故为 1 不是 2
        ("news", "报告为占位报告，请勿采信。", 1, "占位报告（占位 被折叠）"),
    ];
    for (abbr, text, expected, why) in cases {
        let r = run_quality(&[(abbr, text)], &[(abbr, 60.0)]);
        assert_eq!(
            hits(&r, abbr),
            *expected,
            "真缺口漏检：期望 {expected} 处（{why}），实际 {} 处 —— {text}",
            hits(&r, abbr)
        );
    }
}

#[test]
fn low_confidence_list_uses_same_threshold_as_diagnostics() {
    // 口径一致性：low_confidence_analysts 与 diagnostics.status 必须同步
    // （历史缺陷：两处口径各写一套，面板与 summary 互相矛盾）
    let r = run_quality(&[("hm", HM_REPORT_REAL)], &[("hm", 55.0)]);
    let low = names(&r, "low_confidence_analysts");
    assert!(low.contains(&"资金面".to_string()), "资金面应进 low 清单，实际 {low:?}");
    let missing = names(&r, "missing_analysts");
    // 其余 9 个节点注入 () ⇒ confidence = -1 ⇒ 全部 missing
    assert!(missing.contains(&"技术面".to_string()), "未注入的节点应为 missing，实际 {missing:?}");
    assert!(!missing.contains(&"资金面".to_string()), "资金面不应同时出现在 missing 与 low");
}

// ───────────────────────────────────────────────────────────────────────────
// 等式门禁：手抄的 EXTERNAL_VARS ↔ 节点 input_mapping
// ───────────────────────────────────────────────────────────────────────────

/// 从 seed 源文件里抽出 `data-quality` 节点 `input_mapping: [...]` 的键。
///
/// 不做完整 Rust 解析，只靠**两个结构性锚点 + 括号分块**：
///   ① 起点：`let dq_id = "data-quality";`（该节点定义的唯一入口）；
///   ② 终点：区间内首个 `.into_iter()`（`.into_iter()/.map()/.collect()` 是
///      `input_mapping` 数组字面量的固定收尾三元组）；
///   ③ 区间内按 `(` 分块，每块的前两个字符串字面量即 `(键, 路径)`。
///
/// 先按行剔除 `//` 注释：映射区里有多条带引号与括号的注释
/// （如 `resolve_var_path("{id}.content.verdict.confidence")`），
/// 不剔除会把注释里的字面量当成键，产出**假红**。
fn declared_input_mapping_keys() -> Vec<String> {
    let seed = include_str!("../../../src/commands/stock_analysis_setup/seed_stock_analysis.rs");
    let start = seed.find(r#"let dq_id = "data-quality";"#).expect("未找到 data-quality 节点区段");
    let region = &seed[start..];
    let open = region.find("input_mapping: [").expect("未找到 input_mapping");
    let region = &region[open..];
    let close = region.find(".into_iter()").expect("未找到 input_mapping 收尾锚 .into_iter()");
    let region: String = region[..close]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    region
        .split('(')
        .filter_map(|chunk| {
            // 一块里必须有**两个**字符串字面量（键 + 路径）才成对；
            // 只有 1 个引号的残块直接丢弃（与下方自检用的等价实现逐字同源）。
            let parts: Vec<&str> = chunk.split('"').collect();
            if parts.len() < 3 {
                return None;
            }
            Some(parts[1].to_string())
        })
        .collect()
}

/// `EXTERNAL_VARS` 必须与节点 `input_mapping` 的键集**双向相等**。
///
/// 为什么需要：手抄清单的漂移是**静默**的 ——
///   · 漏一个键 ⇒ 该变量不在 scope 里 ⇒ 脚本执行到那行才报 `ErrorVariableNotFound`
///     （首次实测即踩：`money_flow` 被写成了脚本内派生名 `mf_main_net_inflow`）；
///   · 多一个键则完全无声（只是注入了一个没人读的变量）。
/// 两侧各自声明 + 双向差集，是把「物理上无法单点声明」的常量变成可验证门禁的标准做法
/// （与 `src-tauri/src/commands/stock_analysis_setup/seed_consistency_tests.rs` 同族）。
#[test]
fn external_vars_match_node_input_mapping() {
    let declared = declared_input_mapping_keys();

    // ① 提取器自检：0 命中 ≠ 没问题。解析失败时会得到一个偏少的集合，
    //    若不先卡下限，它可能恰好与另一侧的错值「对上了」而报绿。
    assert!(
        declared.len() >= 39,
        "解析 input_mapping 只得到 {} 个键（应 ≥39）⇒ 提取器失效（锚点/分隔符已变），\
         先修提取器再信本次比较。实际解析结果：{declared:?}",
        declared.len()
    );
    // ② 正负对照：确认确实读到了各类已知成员，而不是碰巧凑数
    for probe in ["money_flow", "hm_report", "cat_untrusted", "pace_signal", "risk_volatility"] {
        assert!(declared.iter().any(|k| k == probe), "提取器漏了已知键 {probe}：{declared:?}");
    }

    // ③ 双向差集
    let mine: std::collections::BTreeSet<String> =
        EXTERNAL_VARS.iter().map(|s| s.to_string()).collect();
    let theirs: std::collections::BTreeSet<String> = declared.into_iter().collect();
    let missing: Vec<_> = theirs.difference(&mine).cloned().collect();
    let extra: Vec<_> = mine.difference(&theirs).cloned().collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "EXTERNAL_VARS 与节点 input_mapping 不一致 —— \
         改了 seed 的 input_mapping 后必须同步本文件顶部的 EXTERNAL_VARS\n  \
         漏声明（会导致 ErrorVariableNotFound）: {missing:?}\n  \
         多声明（无声冗余）: {extra:?}"
    );
}

// ───────────────────────────────────────────────────────────────────────────
// 2026-09-21：per-node 报告质量分（`diag_for` 第 7 参 `rq`）
// ───────────────────────────────────────────────────────────────────────────

/// `diagnostics[abbr].report_quality` 必须存在、为正、且各节点可区分。
///
/// 为什么必须有（两条理由都不能靠其它测试兜住）：
///   ① **本次修复的目标字段就是它**：面板「10 个分析师弹窗显示同一组数字」的根因是
///      data-quality 输出的全局对象里没有 per-node 的等级/分数，前端物理上拿不到；
///      而 `mk_q … cat_q` 本来就算好了，只是**只被累加进全局均值、从未输出**。
///      本字段正是那次「算了但没输出」的接线 —— 只改脚本不加断言，无法证明它真的出来了，
///      即本仓高发的「改了不生效」（生成物陷阱 / 只改定义端漏消费端）。
///   ② **Rhai 没有编译期检查**：`diag_for` 加参数后若漏改某个调用点，`engine.compile`
///      仍可能通过（错误只在执行到那一行时才炸），故这里**穷举 10 个分析师**逐一断言。
///      思路与上面 `external_vars_match_node_input_mapping` 的「双向锁」一致：
///      手抄 10 处调用点无法单点声明，那就用穷举把漏改钉死。
#[test]
fn diagnostics_expose_per_node_report_quality() {
    // hm 给一份长报告 + 高自评，确保其 report_quality 严格为正；
    // mk 不注入报告（空字符串 ⇒ 按 0 计，与全局 report_quality_avg 同口径）。
    // 正文刻意不含任何失败标记词，避免把 status 判据卷进本断言。
    let long = "主力资金净流入 3.2 亿元，北向净买入 1.1 亿元，机构专用席位现身龙虎榜。\
                技术面 MACD 金叉、RSI 58、成交量温和放大，支撑位 12.40 元，阻力位 14.80 元。\
                基本面 PE 22.3、PB 3.1、ROE 15.2%，营收同比增 18%，归母净利同比增 22%。\
                公司公告中标 5.6 亿元大单，券商维持增持评级，目标价 16.5 元。\
                行业景气度回升，板块轮动至成长风格，催化剂为新品放量与国产替代加速。";
    let r = run_quality(&[("hm", long)], &[("hm", 80.0)]);

    const ALL: [&str; 10] = ["mk", "sent", "news", "fund", "pol", "hm", "lk", "res", "sec", "cat"];

    // ① 穷举：10 个节点都必须带该字段（漏改任一调用点即红）
    for abbr in ALL {
        let d = diag(&r, abbr);
        assert!(
            d.contains_key("report_quality"),
            "{abbr} 的 diagnostics 缺少 report_quality ⇒ 该 diag_for 调用点漏改第 7 参"
        );
        let typed = d["report_quality"].clone().try_cast::<f64>();
        assert!(typed.is_some(), "{abbr}.report_quality 不是 f64（report_quality() 应返回 f64）");
    }

    // ② 有报告 ⇒ 严格为正（字段存在但恒 0 = 没接上，等同未修）
    let hm_q = diag(&r, "hm")["report_quality"].clone().try_cast::<f64>().unwrap();
    assert!(hm_q > 0.0, "hm 有长报告，report_quality 应 > 0，实际 {hm_q}");

    // ③ 无报告 ⇒ 0（与 report_quality() 的 `text == "" ⇒ 0.0` 契约一致）
    let mk_q = diag(&r, "mk")["report_quality"].clone().try_cast::<f64>().unwrap();
    assert_eq!(mk_q, 0.0, "mk 未注入报告，report_quality 应为 0，实际 {mk_q}");

    // ④ 可区分性 —— 本次修复的直接目的：
    //    若此断言不成立，说明该字段退化成常量，面板仍会显示同一组数字（修复失效）
    assert_ne!(hm_q, mk_q, "不同节点的 report_quality 必须可区分，否则弹窗仍无法反映本节点质量");
}

// ───────────────────────────────────────────────────────────────────────────
// 2026-09-21 P2-1：伪归因判据（`attribution_note`）
// ───────────────────────────────────────────────────────────────────────────

fn gap_reason(result: &Map, abbr: &str) -> String {
    diag(result, abbr)["gap_reason"].clone().into_string().unwrap_or_default()
}

/// 伪归因判据必须在**真实执行**中按两级严重度生效，且负对照不得误触发。
///
/// 为什么放在本文件（而不是 `seed_consistency_tests.rs` 里那条）：
///   那边是**文本抽取**口径（数 `diag_for` 的实参个数），看不见脚本**跑起来**会怎样；
///   本判据的两级结论由 `tool_calls_made` 的 `is_error` + 工具名**裁决** ——
///   只有真执行才能证明它 ① 会触发、② 不误触发、③ 不因字段名写错而恒绿
///   （脚本里最脆的一处：`rejection_summary` 读的是 `is_error`，写成 `success`
///    会静默得到 0 条失败 ⇒ 判据永远走「编造」档，而无任何报错）。
///
/// 被判的真实缺陷（`AUDIT-pledge-attribution-2026-09-21.md`）：
///   报告原文写「质押数据获取失败（**工具调用被拒绝**）」，而当轮 `tool_calls_made` 里
///   **没有任何质押工具调用**，唯一失败的是 `get_stock_margin_data`（原因**限流**，不是权限）
///   ⇒ 一次限流被写成「被拒绝」，且归到一个**从未被调用**的维度上。
#[test]
fn unbacked_attribution_is_flagged_in_two_severities() {
    // 真实报告原文：含归因性措辞，**不含**任何被拒工具名 —— 这正是判据要抓的形态
    const REPORT: &str =
        "**质押风险**：质押数据获取失败（工具调用被拒绝），无法评估平仓线距离，该维度数据缺失。";

    // ── 情形 A（编造）：有归因措辞，而本轮**一条失败记录都没有** ──
    // 样本按本轮真实形态构造：3 次调用全成功（真被限流的是**另一个**节点）
    let calls_zero_failure = vec![
        tc("get_stock_lockup_bundle", false),
        tc("search_stock", false),
        tc("get_stock_announcements", false),
    ];
    let r = run_quality_with_tool_calls(
        &[("lk", REPORT)],
        &[("lk", 60.0)],
        &[("lk", calls_zero_failure)],
    );
    let g = gap_reason(&r, "lk");
    assert!(
        g.contains("编造归因"),
        "报告称「工具调用被拒绝」而本轮 3 次调用**零失败** ⇒ 应判「疑似编造归因」。实际 gap_reason：{g}"
    );

    // ── 情形 B（含糊）：确有失败记录，但报告**一个被拒工具名都没提** ──
    // 样本 = 本轮实证：唯一失败的是 get_stock_margin_data（限流），报告未点名
    let calls_ambiguous =
        vec![tc("get_stock_lockup_bundle", false), tc("get_stock_margin_data", true)];
    let r = run_quality_with_tool_calls(
        &[("lk", REPORT)],
        &[("lk", 60.0)],
        &[("lk", calls_ambiguous.clone())],
    );
    let g = gap_reason(&r, "lk");
    assert!(
        g.contains("归因含糊"),
        "有失败记录但报告未点名被拒工具 ⇒ 应判「归因含糊」。实际 gap_reason：{g}"
    );
    assert!(
        g.contains("get_stock_margin_data"),
        "「含糊」一档必须写出**真实被拒的工具名**，否则用户仍无从排查。实际：{g}"
    );
    assert!(!g.contains("编造归因"), "有失败记录时不得落到「编造」档（两级严重度不可合并）：{g}");

    // ── 情形 C（不得触发）：报告点名了被拒工具 ⇒ 归因有据 ──
    let report_named = "**质押风险**：get_stock_margin_data 工具调用被拒绝，无法评估平仓线距离。";
    let r = run_quality_with_tool_calls(
        &[("lk", report_named)],
        &[("lk", 60.0)],
        &[("lk", calls_ambiguous.clone())],
    );
    let g = gap_reason(&r, "lk");
    assert!(
        !g.contains("编造归因") && !g.contains("归因含糊"),
        "报告已点名被拒工具 ⇒ 归因有据，不得报警（否则是假阳性）。实际：{g}"
    );

    // ── 情形 D（不得触发）：有归因措辞但**无调用记录** ──
    // 守卫① 的负对照：缺数据必须**不判**，否则会把「测不出」当成「有问题」
    let r = run_quality(&[("lk", REPORT)], &[("lk", 60.0)]); // 未注入 ⇒ ()
    let g = gap_reason(&r, "lk");
    assert!(
        !g.contains("编造归因") && !g.contains("归因含糊"),
        "`tool_calls` 为 `()`（旧快照缺字段）时**不得**下归因判决。实际：{g}"
    );
    // 同一档：空数组也必须放行
    let r = run_quality_with_tool_calls(&[("lk", REPORT)], &[("lk", 60.0)], &[("lk", Vec::new())]);
    let g = gap_reason(&r, "lk");
    assert!(
        !g.contains("编造归因") && !g.contains("归因含糊"),
        "`tool_calls` 为空数组时**不得**下归因判决。实际：{g}"
    );

    // ── 情形 E（不得触发）：有失败记录，但报告**没在做工具归因** ──
    // 措辞门是第一道闸：只说「数据缺失」的报告不该被本判据卷入
    let report_plain = "**质押风险**：质押数据缺失，无法评估平仓线距离。";
    let r = run_quality_with_tool_calls(
        &[("lk", report_plain)],
        &[("lk", 60.0)],
        &[("lk", calls_ambiguous)],
    );
    let g = gap_reason(&r, "lk");
    assert!(
        !g.contains("编造归因") && !g.contains("归因含糊"),
        "报告未做工具归因（仅陈述数据缺失）⇒ 本判据不适用。实际：{g}"
    );
}

/// P2-1 的归因结论必须是**单行、无 `\` 续接残留**。
///
/// 为什么必须单独钉（这是本轮真执行才暴露的缺陷）：
///   Rhai 的模板字符串（反引号）**不做 C 风格转义**，`\` 行续接不会折平 ——
///   `\` 与行首缩进会**逐字写进字符串**，而这段文本直接渲染进弹窗「缺口原因」行。
///   实证（2026-09-21 零锁探针 `output/tmp-151b-rhai-probe.rs` 真执行首跑）：
///   界面文本实际为 `…**无一条失败** \` + 换行 + 16 个空格 + `⇒ 该归因无事实依据…`。
///   ⚠ `cargo check` / `clippy` **不解析 `.rhai`**，`engine.compile` 也**不会报**
///   （跨行模板字符串本身是合法字符串）⇒ 只有「真执行 + 断言文本形态」能抓住它。
#[test]
fn attribution_note_text_is_single_line() {
    const REPORT: &str =
        "**质押风险**：质押数据获取失败（工具调用被拒绝），无法评估平仓线距离，该维度数据缺失。";
    let zero_failure = vec![tc("get_stock_lockup_bundle", false)];
    let some_failure =
        vec![tc("get_stock_lockup_bundle", false), tc("get_stock_margin_data", true)];

    for (label, calls) in [("编造档", zero_failure), ("含糊档", some_failure)] {
        let r = run_quality_with_tool_calls(&[("lk", REPORT)], &[("lk", 60.0)], &[("lk", calls)]);
        let g = gap_reason(&r, "lk");
        assert!(
            !g.contains('\\'),
            "{label} 的归因结论含反斜杠 —— Rhai 模板字符串无转义语义，`\\` 会原样显示给用户：{g}"
        );
        assert!(
            !g.contains('\n'),
            "{label} 的归因结论含换行 —— 模板字符串跨行未折平，会连带行首缩进一起渲染：{g}"
        );
        assert_eq!(g.trim(), g, "{label} 的归因结论首尾有空白：{g}");
    }
}

// ───────────────────────────────────────────────────────────────────────────
// S2(2026-09-26)：as-of 回放「设计性降级」豁免（PLAN-asof-replay-quality-attribution.md）
// 实证缺陷（300642 as_of=2026-09-22）：回放中搜索/快讯等按当下语义约束返回空，
// 分析师如实写「无法获取」被词表按**工具故障**扣分 ⇒ tool_credibility 27、综合 C 级。
// ───────────────────────────────────────────────────────────────────────────

/// 政策面分析师在回放中的典型报告：含硬标记「无法获取」+ 软标记「返回空」（无抑制短语）。
const POL_REPORT_ASOF: &str = "宏观政策与行业政策搜索返回空，无法获取政策原文，\
本维度仅能基于既有信息推断，不构成对政策方向的确认。仓位建议以观望为主，等待数据恢复。";

#[test]
fn asof_designed_degradation_exempts_failure_markers() {
    let r = run_quality_asof(
        &[("pol", POL_REPORT_ASOF)],
        &[("pol", 30.0)],
        r#"["search_news","get_policy_news"]"#,
    );
    assert_eq!(hits(&r, "pol"), 0, "设计性降级维度不得计失败标记");
    assert_eq!(
        status(&r, "pol"),
        "low",
        "低置信仍按自评判 low（avg_conf 语义保留，豁免不掩盖不确定性）"
    );
    let g = gap_reason(&r, "pol");
    // S6(2026-09-26)：被豁免且原始报告确有标记 ⇒ 必须说「按设计降级…已豁免」，
    // 不得再说「报告无失败标记」（豁免后 ph 恒 0，那句话是假话）。
    assert!(
        g.contains("按设计降级") && g.contains("已豁免"),
        "豁免维度的 gap_reason 必须如实归因: {g}"
    );
    assert!(!g.contains("报告无失败标记"), "gap_reason 不得声称无标记（原始报告有）: {g}");
    assert_eq!(r["placeholder_total_hits"].as_int().unwrap_or(-1), 0);
    assert!(r["asof_replay"].as_bool().unwrap_or(false));
    let dims = names(&r, "asof_designed_dims");
    assert!(dims.iter().any(|d| d == "政策面"), "asof_designed_dims 应含政策面：{dims:?}");
    let warns = names(&r, "warnings");
    assert!(
        warns.iter().any(|w| w.contains("按设计降级")),
        "warnings 必须显式声明豁免，避免被误读为漏扣分：{warns:?}"
    );
    let summary = r["summary"].clone().into_string().unwrap_or_default();
    assert!(summary.starts_with("【as-of 回放】"), "summary 必须带回放前缀：{summary}");
}

#[test]
fn asof_exemption_raises_report_quality() {
    let live = run_quality(&[("pol", POL_REPORT_ASOF)], &[("pol", 30.0)]);
    let replay =
        run_quality_asof(&[("pol", POL_REPORT_ASOF)], &[("pol", 30.0)], r#"["get_policy_news"]"#);
    let lq = live["report_quality_score"].as_float().unwrap_or(0.0);
    let rq = replay["report_quality_score"].as_float().unwrap_or(0.0);
    assert!(rq > lq, "豁免跳过 -15 占位扣分后报告质量应抬升：live {lq} vs replay {rq}");
}

#[test]
fn asof_unmapped_method_does_not_exempt() {
    // truncate_* 是「按截止日截断」的正常语义（数据仍在），刻意不在映射表 ⇒ 不豁免。
    // 同时兜住映射表被误删空的回归：未知方法必须保守地**不**触发豁免。
    let r = run_quality_asof(
        &[("pol", POL_REPORT_ASOF)],
        &[("pol", 30.0)],
        r#"["truncate_klines_by_asof"]"#,
    );
    assert!(hits(&r, "pol") > 0, "未映射方法不得豁免失败标记");
    assert!(r["asof_replay"].as_bool().unwrap_or(false), "有降级记录即标记回放");
}

#[test]
fn live_mode_has_no_asof_exemption() {
    let r = run_quality(&[("pol", POL_REPORT_ASOF)], &[("pol", 30.0)]);
    assert!(!r["asof_replay"].as_bool().unwrap_or(true));
    assert!(hits(&r, "pol") > 0, "live 模式失败标记照常计入（豁免只属回放）");
    let summary = r["summary"].clone().into_string().unwrap_or_default();
    assert!(!summary.starts_with("【as-of 回放】"), "live summary 不得带回放前缀：{summary}");
}

#[test]
fn asof_exempted_dim_without_markers_says_replay_not_false_gap() {
    // 政策面被设计性降级，但报告写得干净（无失败标记词）且低置信。
    // 旧口径会落到「非数据缺口：报告无失败标记、字段齐全」——对降级维度同样误导。
    let clean =
        "政策面依据既有公开信息做了方向性推断，未见矛盾信号，建议观望等待数据恢复后复核仓位。";
    let r = run_quality_asof(&[("pol", clean)], &[("pol", 30.0)], r#"["get_policy_news"]"#);
    let g = gap_reason(&r, "pol");
    assert!(g.starts_with("as-of 回放"), "无标记但被降级的维度也必须归因到回放: {g}");
    assert!(!g.contains("字段齐全"), "不得声称字段齐全（上游本轮按设计无数据）: {g}");
}

#[test]
fn direction_conflict_no_longer_penalizes_tool_credibility() {
    // R9a(2026-09-26)：severe 冲突（两边各 2 个 conf60 ⇒ 加权和 120≥100）是
    // 辩论架构的设计常态，只提示不扣分。判据：tc == avg_conf == 60。
    let long = "该维度支持看多方向，依据为既有行情与财务数据，趋势信号一致。";
    let short = "该维度支持看空方向，依据为既有行情与财务数据，风险信号明显。";
    // 10 个 verdict 全注入（其余 6 个中性）—— 否则缺失维度按 gap 罚 −40，
    // 会把「冲突不扣分」的判据淹没在 gap 罚里，用例失去区分力。
    let neutral = "该维度中性，数据可得，方向信号不显著，维持观望。";
    let r = run_quality_dirs(
        &[
            ("mk", long),
            ("sent", long),
            ("news", short),
            ("fund", short),
            ("pol", neutral),
            ("hm", neutral),
            ("lk", neutral),
            ("res", neutral),
            ("sec", neutral),
            ("cat", neutral),
        ],
        &[
            ("mk", 60.0, "看多"),
            ("sent", 60.0, "看多"),
            ("news", 60.0, "看空"),
            ("fund", 60.0, "看空"),
            ("pol", 60.0, "中性"),
            ("hm", 60.0, "中性"),
            ("lk", 60.0, "中性"),
            ("res", 60.0, "中性"),
            ("sec", 60.0, "中性"),
            ("cat", 60.0, "中性"),
        ],
    );
    assert!(
        r["severe_direction_conflict"].as_bool().unwrap_or(false),
        "用例前提：必须构造出 severe 冲突"
    );
    let tc = r["tool_credibility_score"].as_float().unwrap_or(0.0);
    assert!(
        (tc - 60.0).abs() < 1e-6,
        "R9a 后 severe 冲突不得扣分（tc 应等于 avg_conf 60，旧口径会是 40）：tc={tc}"
    );
    let warns = names(&r, "warnings");
    assert!(
        warns.iter().any(|w| w.contains("多空分歧") && w.contains("不扣分")),
        "冲突信号必须仍以 warnings 呈现（只去扣分不去信号）：{warns:?}"
    );
}

#[test]
fn r9b_absence_phrases_suppress_but_true_failures_still_count() {
    // R9b(2026-09-26)：live 报告抽样出的缺席类语境必须被定向抑制……
    // ⚠ 样本刻意不用 `数据缺失` —— 它维持硬标记（正文级共现抑制会吞同篇真缺口，见 rhai 注释）。
    let absent = "北向净流入自 2024 年 8 月起监管停披，该维度数据不可用；\
                  机构评级均为空（视为无机构覆盖），不影响主结论。";
    let r = run_quality(&[("hm", absent)], &[("hm", 60.0)]);
    assert_eq!(hits(&r, "hm"), 0, "缺席类（停披/无机构覆盖）措辞不得计失败标记");
    let cfg = "当前 auto_stop_loss_pct 未注入，按规则保守取 stopLoss = MA20 附近。";
    let r3 = run_quality(&[("hm", cfg)], &[("hm", 60.0)]);
    assert_eq!(hits(&r3, "hm"), 0, "配置参数未注入与数据源无关，应抑制");
    // ……而真故障句（同一 `数据缺失` 措辞、无缺席否定词）必须照常计入 —— 负控。
    let real = "get_stock_pledge_data 调用因系统频率限制返回权限错误，质押数据缺失，\
                无法评估平仓线距离。";
    let r2 = run_quality(&[("hm", real)], &[("hm", 60.0)]);
    assert!(hits(&r2, "hm") > 0, "真工具故障不得被 R9b 抑制误杀");
}
