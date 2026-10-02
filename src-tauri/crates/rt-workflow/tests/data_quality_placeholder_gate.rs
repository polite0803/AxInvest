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
    // 2026-10-02 补：DCF **不可用机读原因码**（第四类缺席的判据输入）。
    //   与上面那次漏同步同形 —— 加 `input_mapping` 键必须同时加进本清单，
    //   否则脚本首跑即 `ErrorVariableNotFound`、本文件全红。
    "valuation_dcf_unavailable_reason",
    //   2026-10-02 同批：`dcf.available` 是第四类缺席的**伞形**判据（遮蔽态无码）。
    "valuation_dcf_available",
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
    // ⚠ 与生产 CodeNode 引擎**逐项对齐**（`code_executor.rs:57` `set_max_operations(200_000)`）。
    //   此处曾放宽到 2_000_000 ⇒ D1 逐字符断句实现烧穿生产限额、data-quality 节点
    //   整体失败（2026-09-27 16:54 运行 2bb5bb9d），而门禁全绿 —— 测试引擎的
    //   资源上限本身就是被测契约的一部分，不得比生产宽松。
    engine.set_max_operations(200_000);
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

/// D1 回归（2026-09-27，301302 C5 重跑 6 标记逐句实证为合法缺席）：
/// 抑制粒度降到句级后，用**真实报告语句**钉死「6 → 2」：
/// fund/hm/sec 的缺席句（语境与命中同句）清零；lk 结论段裸「质押数据缺失」
/// 与 sec 的「=null」枚举**维持计数**（语境在异句/异段，句级不连坐）。
#[test]
fn d1_sentence_level_suppression_on_real_reports() {
    // fund 两句（真实原文节选）
    let fund = "数据缺口影响评估：无机构 EPS 覆盖、连续亏损年数缺失、商誉/质押数据缺失，使估值锚定和退市风险评估置信度下降约 15-20 分。\n\
                无商誉/质押/审计非标信息返回，该维度数据获取失败。";
    let r = run_quality(&[("fund", fund)], &[("fund", 55.0)]);
    assert_eq!(hits(&r, "fund"), 0, "fund 两句均含同句缺席语境（无机构/无商誉…信息返回），应清零");

    // hm 句（真实原文节选）：「需明确标注」与命中同句
    let hm = "无法判断北向对华如科技的买卖方向，该维度数据缺失，需明确标注。";
    let r = run_quality(&[("hm", hm)], &[("hm", 35.0)]);
    assert_eq!(hits(&r, "hm"), 0, "北向停披缺席语境与命中同句，应清零");

    // lk 结论段（真实原文节选）：语境（不可得）在另一段 ⇒ 维持计数
    let lk = "质押风险：pledge_ratio_unavailable（质押数据不可得），无法评估平仓线与纾困敞口。\n\
              结论：筹码面中期供给偏重；质押数据缺失削弱结论确定性，整体偏空。";
    let r = run_quality(&[("lk", lk)], &[("lk", 42.0)]);
    assert_eq!(hits(&r, "lk"), 1, "结论段裸「数据缺失」与语境异句，不得连坐抑制");

    // sec 段（真实原文节选）：「仅取到」同句豁免，「=null」枚举维持
    let sec = "机构覆盖数=null；历史价格区间/均线/量价结构数据缺失，仅取到当日快照。";
    let r = run_quality(&[("sec", sec)], &[("sec", 45.0)]);
    assert_eq!(hits(&r, "sec"), 1, "=null 枚举维持计数；「数据缺失，仅取到…」同句豁免");
}

/// F3（2026-09-27，001313 粤海饲料运行 `c6399466`）：两融「设计上没有」语境进句级抑制表。
/// 病灶不是通道（`RPTA_WEB_RZRQ_GGMX` 对茅台 3989 页明细、对非标的的粤海如实回 9201 空），
/// 而是 live 裸 `null` 让分析师只能写「数据缺失」。F1 输出结构化 `available:false`、
/// F2 prompt 引导「设计性缺席」措辞后，同句语境在此豁免；真故障句不受影响。
#[test]
fn f3_margin_designified_absence_suppressed() {
    // ① 同句语境 ⇒ 豁免（F2 引导措辞即便仍残留失败动词也不误伤）
    let a = "融资余额数据缺失（该券非融资融券标的，属设计性缺席），散户仓位规则不计分。";
    let r = run_quality(&[("sent", a)], &[("sent", 60.0)]);
    assert_eq!(hits(&r, "sent"), 0, "「非融资融券标的/设计性缺席」与命中同句，应清零");

    // ② 反向锁：真故障句（无该语境）两词种照计
    let b = "两融明细数据缺失，取数通道超时无响应，该维度数据获取失败。";
    let r = run_quality(&[("hm", b)], &[("hm", 55.0)]);
    assert_eq!(hits(&r, "hm"), 2, "数据缺失+获取失败两词种均无缺席语境，不得豁免");

    // ③ 连坐锁：语境与命中异句 ⇒ 不抑制（句级粒度对 F3 新短语同样成立）
    let c = "该券非融资融券标的，两融维度属设计性缺席。\n融资融券明细数据缺失，杠杆情绪无法评估。";
    let r = run_quality(&[("sent", c)], &[("sent", 50.0)]);
    assert_eq!(hits(&r, "sent"), 1, "语境在第一句、命中在第二句 ⇒ 维持计数");
}

/// G1（2026-10-01，600406 运行 `a7e590a4` / 002371 运行 `97d79306`）：
/// **缺席语境与标记词解耦** —— 同一句里出现两个软标记时，不得一个被豁免、一个被计缺口。
///
/// 被判的真实缺陷（不对称）：软标记的抑制短语是**按标记词各配一份**的，而
/// `设计性缺席`/`非两融标的`（F3 新增）当时只配给了 `数据缺失`/`获取失败`。
/// 002371 **情绪面**分析师原文（`diagnostics.sent`）：
///   「获取的社交情绪数据源为空（**设计性缺席**或**无数据**），无法直接量化散户/舆情极端度」
/// ⇒ 「设计性缺席」那半个词被豁免，`无数据` 照计 —— 同一事实、同一句，判成 1 处真缺口。
/// 另 600406 情绪面：「近一年**未上龙虎榜**，拉萨天团、封单比两项规则**无数据**可用，
/// 属该券低波动白马属性导致」⇒ 事件型榜单「没上榜」被当成取数失败（+1 处）。
///
/// 修法：缺席语境独立成 `absence_context_phrases()`，**对全部软标记生效**；
/// 动词类硬标记（无法获取/未能获取/占位/TODO…）维持零抑制。
#[test]
fn g1_absence_context_applies_to_every_soft_marker() {
    // ① 穷举锁：每个软标记 + 同句「设计性缺席」⇒ 一律不得计数。
    //    这条是全测试的**结构锁**：下一个软标记若又漏配缺席语境，这里必然变红
    //    （不对称缺陷的形态就是「按词各配一份，新加的那一份漏了某个词」）。
    let soft_cases: &[(&str, &str)] = &[
        ("sent", "北向净流入数据不可用（设计性缺席），无法评估外资方向。"),
        ("news", "该维度无数据（设计性缺席），不影响主结论。"),
        ("news", "公告接口返回空（设计性缺席），按无记录处理。"),
        ("pol", "字段为空值（设计性缺席），按缺省口径处理。"),
        ("pol", "机构评级字段均为空（设计性缺席），仅看政策面。"),
        ("res", "参数未注入（设计性缺席），取脚本默认值。"),
        ("fund", "同行 ROE 数据缺失（设计性缺席），仅能以 PE/PB 粗比。"),
        ("sec", "行业排名获取失败（设计性缺席），按无排名处理。"),
    ];
    for (node, text) in soft_cases {
        let r = run_quality(&[(node, text)], &[(node, 60.0)]);
        assert_eq!(hits(&r, node), 0, "软标记句含「设计性缺席」必须豁免（节点 {node}）：{text}");
    }

    // ② 002371 实证原文：同句两个软标记，判决必须一致
    let a = "获取的社交情绪数据源为空（设计性缺席或无数据），无法直接量化散户/舆情极端度。";
    let r = run_quality(&[("sent", a)], &[("sent", 60.0)]);
    assert_eq!(hits(&r, "sent"), 0, "「设计性缺席或无数据」同句 ⇒ 两词种一律清零（G1 主证）");

    // ③ 600406 实证原文：事件型榜单「没上榜」不是取数失败
    let b = "国电南瑞近一年未上龙虎榜，拉萨天团、封单比两项规则无数据可用，\
             属该券低波动白马属性导致，不做强/弱封板推断。";
    let r = run_quality(&[("sent", b)], &[("sent", 60.0)]);
    assert_eq!(hits(&r, "sent"), 0, "「未上龙虎榜 ⇒ 规则无数据可用」属设计性缺席");

    // ④ 负控 1：去掉缺席语境（同一句式）⇒ 必须恢复计数
    let c = "获取的社交情绪数据源为空，本维度无数据，无法直接量化散户舆情极端度。";
    let r = run_quality(&[("sent", c)], &[("sent", 60.0)]);
    assert_eq!(hits(&r, "sent"), 1, "无缺席语境的裸「无数据」仍须计数");
    let d = "龙虎榜明细接口超时未响应，本维度无数据，无法判断游资动向。";
    let r = run_quality(&[("hm", d)], &[("hm", 55.0)]);
    assert_eq!(hits(&r, "hm"), 1, "真取数失败句（无缺席语境）不得被 G1 误杀");

    // ⑤ 负控 2：**硬标记零抑制**锁 —— 缺席语境只作用于软标记。
    //    动词类真缺口正是用这些词表达的，若将来有人把缺席组接进硬标记捷径分支，
    //    `无法获取` 会被「未上龙虎榜」这类无关语境吞掉（判据方向反转）。
    let e = "北向资金无法获取精确净额，且该股未上龙虎榜。";
    let r = run_quality(&[("hm", e)], &[("hm", 55.0)]);
    assert!(hits(&r, "hm") > 0, "硬标记「无法获取」不得被缺席语境抑制");
}

/// D1 后续（2026-09-27）：生产引擎 `max_operations=200_000` 压测。
/// 16:54 运行 2bb5bb9d 实锤：逐字符断句实现把 data-quality 节点整体打挂
/// （"Too many operations"），面板逐节点诊断全空。本测试用 10 份 KB 级报告
/// 复现生产规模 —— 若断句/计数实现再退化为逐字符循环，这里必须变红
/// （测试引擎上限已与生产逐项对齐，见 `build_engine_with_asof_methods`）。
#[test]
fn d1_long_reports_fit_production_max_operations() {
    let unit =
        "本季度营收同比增长 12%，毛利率 35%，PE 处于历史中位，成交量温和放大，均线多头排列。";
    let mut long = String::new();
    for _ in 0..80 {
        long.push_str(unit);
    }
    long.push_str("唯一缺口：商誉数据缺失，无法完成减值测试。");
    let nodes = ["mk", "sent", "news", "fund", "pol", "hm", "lk", "res", "sec", "cat"];
    let reports: Vec<(&str, &str)> = nodes.iter().map(|n| (*n, long.as_str())).collect();
    let confs: Vec<(&str, f64)> = nodes.iter().map(|n| (*n, 60.0)).collect();
    let r = run_quality(&reports, &confs);
    // 能走到断言即证明未烧穿操作数上限；语义侧：每份报告恰好 1 次裸「数据缺失」
    for n in nodes {
        assert_eq!(hits(&r, n), 1, "节点 {n}：长报告应恰好命中 1 次裸「数据缺失」（无同句语境）");
    }
}

/// G2（2026-10-01，600887 运行 `f474ec9b`）：**市场级口径停披**豁免动词类硬标记。
///
/// 被判的真实缺陷：资金面报告如实写着「受 2024 年 8 月**监管停披**影响…**未能获取**单股精确
/// 净买入数据」，而 `未能获取` 属硬标记 ⇒ 照计一处缺口 ⇒ 该节点被判「⚠️ 低置信」，
/// `gap_reason` 归因成「上游工具数据不完整」（真因是**监管口径**：北向个股净流入
/// 自 2024-08-16 起停披，**永久**没有 ⇒ 凡报告提及北向净流入的轮次都会复发）。
///
/// 两层判据的分界（本测试把边界一起锁住，防止后人「顺手」放宽）：
///   · **市场级**（停披）—— 与本次工具调用**正交**（工具坏了不会让交易所停止披露）
///     ⇒ 允许豁免硬标记；
///   · **个股级**（未上龙虎榜/设计性缺席/非两融标的）—— 不足以排除工具故障
///     （同句完全可能既说「没上榜」又说「接口超时」）⇒ **不得**豁免硬标记。
#[test]
fn g2_market_level_absence_suppresses_hard_marker_but_stock_level_does_not() {
    // ① 真证明文（资金面）：「监管停披」与动词命中同句 ⇒ 豁免
    let hm = "**北向资金**：受2024年8月监管停披影响，近期（2026-09-30）仅披露沪深股通成交额\
              （分别为101.26亿、106.68亿元），未能获取单股精确净买入数据。";
    let r = run_quality(&[("hm", hm)], &[("hm", 72.0)]);
    assert_eq!(hits(&r, "hm"), 0, "市场级停披与动词命中同句 ⇒ 不得计为数据缺口");

    // ② 负控：同样的动词、无停披语境 ⇒ 照计（真取数失败不得被吞）
    let real = "北向净买入接口调用超时重试三次仍失败，未能获取单股精确净买入数据。";
    let r = run_quality(&[("hm", real)], &[("hm", 60.0)]);
    assert_eq!(hits(&r, "hm"), 1, "无停披语境的真取数失败必须照计");

    // ③ **层级边界锁**：个股级缺席语境不得豁免硬标记 —— 否则「工具坏了」可以被
    //    「该股未上龙虎榜」这类无关断言吞掉（G1 负控⑤锁的正是这条，此处从另一切面再锁一次）
    let stock_level = "龙虎榜明细接口超时，无法获取当日席位数据，且该股近期未上龙虎榜。";
    let r = run_quality(&[("hm", stock_level)], &[("hm", 60.0)]);
    assert_eq!(hits(&r, "hm"), 1, "个股级缺席语境不得豁免动词类硬标记");

    // ④ 跨句不连坐：「停披」在第 1 句、真故障动词在第 2 句 ⇒ 第 2 句照计
    let cross = "北向个股净流入自 2024-08-16 起监管停披，属永久缺席。\n\
                 质押数据接口调用失败，未能获取平仓线数据。";
    let r = run_quality(&[("lk", cross)], &[("lk", 55.0)]);
    assert_eq!(hits(&r, "lk"), 1, "市场级豁免必须逐句生效，不得跨句连坐");

    // ⑤ 生产预算：硬标记改走句级分支（正文含停披）时仍不得烧穿 200k 操作。
    //    这一段是 G2 与 v87「预算捷径」的交点：捷径被加了一道例外，必须重新压测。
    let unit =
        "本季度营收同比增长 12%，毛利率 35%，PE 处于历史中位，成交量温和放大，均线多头排列。";
    let mut long = String::new();
    for _ in 0..80 {
        long.push_str(unit);
    }
    long.push_str("北向净流入自 2024 年 8 月起监管停披，未能获取单股净买入。");
    let nodes = ["mk", "sent", "news", "fund", "pol", "hm", "lk", "res", "sec", "cat"];
    let reports: Vec<(&str, &str)> = nodes.iter().map(|n| (*n, long.as_str())).collect();
    let confs: Vec<(&str, f64)> = nodes.iter().map(|n| (*n, 60.0)).collect();
    let r = run_quality(&reports, &confs);
    for n in nodes {
        assert_eq!(hits(&r, n), 0, "节点 {n}：停披句内的动词命中应被豁免（且未烧穿操作预算）");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// R1（2026-10-02）：VERDICT 专属字段未产出 —— 第三类缺席的检出与分列
// ─────────────────────────────────────────────────────────────────────────────

/// 通用 6 键 —— 与 `agent_executor.rs` 的 `VERDICT_MACHINE_FIELDS` 同一集合。
/// 本文件刻意**不**从 Rhai 侧读声明表，而是用「跑一遍看它报哪些字段缺」来反推，
/// 这样 `required_verdict_fields()` 与角色 `.md` 之间的绑定是**行为对行为**，
/// 不是两份文本清单的字符串比对（后者一改格式就静默失效）。
const GENERIC_VERDICT_FIELDS: [&str; 6] =
    ["verdict", "bull_score", "bear_score", "bull_points", "bear_points", "confidence"];

/// 同 `run_quality`，但可注入**完整**的 verdict map（覆盖 `{abbr}_verdict` 输入）。
///
/// 存在的理由：`run_quality_impl` 的 `confs` 只造 `{confidence}` 一种形态，
/// 无法表达「分析师产出了专属字段」这一正控 —— 那正是本组判据唯一能区分两种
/// 缺席（模型漏字段 vs 该维度真无催化剂）的输入。
fn run_quality_with_verdicts(reports: &[(&str, &str)], verdicts: &[(&str, Map)]) -> Map {
    let engine = build_engine();
    let ast = engine.compile(SCRIPT).expect("data-quality.rhai 编译失败");
    let mut scope = Scope::new();
    for v in EXTERNAL_VARS {
        scope.push_dynamic(*v, Dynamic::UNIT);
    }
    for (abbr, text) in reports {
        scope.push_dynamic(format!("{abbr}_report"), Dynamic::from(text.to_string()));
    }
    for (abbr, verdict) in verdicts {
        scope.push_dynamic(format!("{abbr}_verdict"), Dynamic::from(verdict.clone()));
    }
    engine
        .eval_ast_with_scope::<Map>(&mut scope, &ast)
        .expect("data-quality.rhai 执行失败（检查是否引用了未注册的宿主函数）")
}

/// 造一份 verdict map：给定键 → 字符串值，`confidence` 单独给数值。
fn verdict_with(conf: f64, extras: &[(&str, &str)]) -> Map {
    let mut m = Map::new();
    m.insert("verdict".into(), Dynamic::from("偏空".to_string()));
    m.insert("bull_score".into(), Dynamic::from(45_i64));
    m.insert("bear_score".into(), Dynamic::from(60_i64));
    m.insert("bull_points".into(), Dynamic::from(rhai::Array::new()));
    m.insert("bear_points".into(), Dynamic::from(rhai::Array::new()));
    m.insert("confidence".into(), Dynamic::from(conf));
    for (k, v) in extras {
        m.insert((*k).to_string().into(), Dynamic::from(v.to_string()));
    }
    m
}

/// 取 `verdict_field_gaps` 的字符串清单。
fn field_gaps(result: &Map) -> Vec<String> {
    let arr = result["verdict_field_gaps"]
        .clone()
        .try_cast::<rhai::Array>()
        .expect("verdict_field_gaps 不是数组");
    arr.iter().map(|d| d.to_string()).collect()
}

/// 从缺口描述串里取出**字段名**（形如 `催化剂分析师：VERDICT 缺字段 catalyst_level`）。
fn gap_field(desc: &str) -> String {
    desc.rsplit(' ').next().unwrap_or(desc).to_string()
}

/// R1-b 主判据：分析师已出 VERDICT、但缺角色专属字段 ⇒ 计入 `verdict_field_gaps`，
/// 且**不并入** `missing_factors`（两表并存，各自语义独立）。
///
/// 被判的真实缺陷（688498 运行 `129745a7`）：a-catalyst 五个工具调用全成功、
/// 正文写了「L2业绩拐点级利好」，verdict 却只剩通用 6 键 ⇒ 三个专属字段同时消失。
/// 修复前该事实在 UI 上**完全不可见**，只表现为「缺失因子：催化剂等级」，
/// 与真取数故障无法区分（用户据此去查数据源，而数据源是好的）。
#[test]
fn r1_missing_verdict_field_is_a_third_kind_of_absence() {
    let report =
        "催化剂级别判断依据：中报净利 6.07 亿元构成 L2 业绩拐点级利好，实控人拟减持构成利空。";

    // ① 修复前真实形态：只有通用 6 键 ⇒ 三个专属字段全部记为「未产出」
    let r = run_quality_with_verdicts(&[("cat", report)], &[("cat", verdict_with(45.0, &[]))]);
    let gaps = field_gaps(&r);
    assert_eq!(gaps.len(), 3, "三个专属字段同时缺失：{gaps:?}");
    let fields: Vec<String> = gaps.iter().map(|g| gap_field(g)).collect();
    for f in ["catalyst_level", "institutional_trace", "narrative_completeness"] {
        assert!(fields.contains(&f.to_string()), "应报出缺字段 {f}，实得 {fields:?}");
    }
    // 两表并存：因子侧仍记「催化剂等级」缺席（它确实没值，与分母口径绑死）
    let mf = names(&r, "missing_factors");
    assert!(mf.contains(&"催化剂等级".to_string()), "missing_factors 仍须含「催化剂等级」：{mf:?}");
    // 且**不得**把本类缺席混进上游取数缺口（那栏说的是「工具没取到数」）
    assert!(names(&r, "upstream_data_gaps").is_empty(), "模型漏字段不是上游取数故障");

    // ② 正控：专属字段齐全 ⇒ 本表为空（`catalyst_level: "无"` 是**有效产出**，
    //    含义是「该维度确无催化剂」，不得被当成字段缺失）
    let full = verdict_with(
        70.0,
        &[
            ("catalyst_level", "无"),
            ("institutional_trace", "无"),
            ("narrative_completeness", "40"),
        ],
    );
    let r = run_quality_with_verdicts(&[("cat", report)], &[("cat", full)]);
    assert!(field_gaps(&r).is_empty(), "字段齐全（即便取值为「无」）不得报未产出");

    // ③ 根本没有 VERDICT（scope 里是 `()`）⇒ 属 status=missing / missing_analysts 的
    //    语义，**不属**本类。误并会让一个缺陷在两张表里各报一次。
    let r = run_quality_with_verdicts(&[("cat", report)], &[]);
    assert!(field_gaps(&r).is_empty(), "无 VERDICT 产出时不得记「字段未产出」");

    // ④ strict_mode 降级：verdict 是引擎合成的通用壳，缺字段是降级的**必然结果**
    //    ⇒ 跳过，由 status=untrusted 表达（同 validate_verdict_schema 的 `!xx_u` 守卫）
    let mut degraded = verdict_with(0.0, &[]);
    degraded.insert("__untrusted".into(), Dynamic::from(true));
    degraded.insert("strict_mode_fallback".into(), Dynamic::from(true));
    let r = run_quality_with_verdicts(&[("cat", report)], &[("cat", degraded)]);
    assert!(field_gaps(&r).is_empty(), "降级壳不得重复报字段缺失");

    // ⑤ 快速链判定器形态（`j-*`，带 `node_id`）不是分析师 VERDICT ⇒ 不适用本判据。
    //    不锁这条 ⇒ 快速链每次运行都会凭空多出三条假「字段未产出」。
    let mut classifier = Map::new();
    classifier.insert("category".into(), Dynamic::from("中性".to_string()));
    classifier.insert("node_id".into(), Dynamic::from("j-catalyst".to_string()));
    let r = run_quality_with_verdicts(&[("cat", report)], &[("cat", classifier)]);
    assert!(field_gaps(&r).is_empty(), "判定器形态不适用分析师字段判据");
}

/// R1 防漂移绑定：`data-quality.rhai` 声明的催化剂专属字段 **必须逐字等于**
/// `catalyst-analyst.md`「输出格式」VERDICT 模板行里、通用 6 键之外的那批键。
///
/// 为什么必须锁：两侧任一方单独演进都会**静默**失效 ——
///   · md 加了字段、Rhai 没加 ⇒ 该字段漏产出无人检出（回到修复前的不可见状态）；
///   · Rhai 加了字段、md 没写 ⇒ 分析师被要求产出它从未见过的字段，恒报缺失。
/// 判据取「行为对行为」：Rhai 侧的清单由 ① 跑一遍它自己报出的字段名反推，
/// 不解析 Rhai 文本。
#[test]
fn r1_required_field_list_is_bound_to_the_expert_md_template() {
    const CATALYST_MD: &str =
        include_str!("../../../agency_experts/stock-analysis/catalyst-analyst.md");

    // 规范行 = 带 `|` 枚举占位的那一行（示例行写的是具体值，不含 `|`）
    let spec_line = CATALYST_MD
        .lines()
        .find(|l| l.starts_with("<!-- VERDICT: ") && l.contains("\"verdict\":\"看多|"))
        .expect("catalyst-analyst.md 找不到规范 VERDICT 模板行（格式变了？绑定测试需同步）");
    let md_fields = template_keys(spec_line);
    let mut md_extras: Vec<String> = md_fields
        .iter()
        .filter(|k| !GENERIC_VERDICT_FIELDS.contains(&k.as_str()))
        .cloned()
        .collect();
    md_extras.sort();
    assert!(!md_extras.is_empty(), "规范模板行除通用 6 键外必须有专属字段，否则本绑定无意义");

    // Rhai 侧：让催化剂分析师只产出通用 6 键，它报出的字段名就是它的声明清单
    let r = run_quality_with_verdicts(&[], &[("cat", verdict_with(45.0, &[]))]);
    let mut rhai_extras: Vec<String> = field_gaps(&r).iter().map(|g| gap_field(g)).collect();
    rhai_extras.sort();

    assert_eq!(
        rhai_extras, md_extras,
        "Rhai 的 required_verdict_fields 与 catalyst-analyst.md 模板行漂移了。\
         md 侧 {md_extras:?} / Rhai 侧 {rhai_extras:?}"
    );
}

/// 从 `<!-- VERDICT: {...} -->` 一行里取出**顶层**键名（不依赖 serde：
/// 模板值是 `0-100整数` 这类非 JSON 占位串，整行不是合法 JSON）。
fn template_keys(line: &str) -> Vec<String> {
    let bytes: Vec<char> = line.chars().collect();
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            '{' | '[' => depth += 1,
            '}' | ']' => depth = depth.saturating_sub(1),
            '"' if depth == 1 => {
                // 只在顶层取「紧跟冒号」的字符串 —— 即键名
                let mut j = i + 1;
                let mut buf = String::new();
                while j < bytes.len() && bytes[j] != '"' {
                    buf.push(bytes[j]);
                    j += 1;
                }
                if j + 1 < bytes.len() && bytes[j + 1] == ':' {
                    keys.push(buf);
                }
                i = j;
            },
            _ => {},
        }
        i += 1;
    }
    keys
}

/// N1/N2/N3（2026-10-02，603986 运行 `8d46751f`）：**提示词里指定的措辞必须逐字可验证**
/// 不会被失败标记词表计入 —— 否则「改提示词」只是一句没有后验的承诺。
///
/// 三处提示词（`catalyst-analyst.md` / `news-analyst.md` / `hot-money-tracker.md`）各新增一段
/// 「通道先天边界」，规定分析师**必须**用哪个说法、禁止用哪个说法。本测试锁两件事：
///   ① 指定措辞 ⇒ 命中 0（改措辞确实能摘掉那三行假「低置信」）；
///   ② 被禁止的措辞 ⇒ 仍命中 1（证明判据本身没被放宽，收紧的是**产出**不是**尺子**；
///      若哪天有人为了让 ① 通过而去放宽词表，② 会立刻红）。
/// 度量口径两侧完全相同（同一个 `marker_counts`、同一套句级抑制），差别只在输入文本。
#[test]
fn n1_n3_prescribed_wordings_are_clean_while_forbidden_wordings_still_count() {
    // ──  催化剂：指定「正文未解析（结构性）」，禁止「PDF关键数据缺失」──
    let cat_ok = "长电科技处于L3催化剂与中报高增长叠加期，定增扩产打开产能空间，\
                  但PE 59倍估值已透支部分预期且公告通道仅标题级信息、正文未解析（结构性）。";
    let r = run_quality(&[("cat", cat_ok)], &[("cat", 55.0)]);
    assert_eq!(hits(&r, "cat"), 0, "催化剂指定措辞不得计失败标记：{cat_ok}");

    let cat_bad = "PE 59倍估值已透支部分预期且PDF关键数据缺失，整体偏多看待。";
    let r = run_quality(&[("cat", cat_bad)], &[("cat", 55.0)]);
    assert_eq!(hits(&r, "cat"), 1, "被禁止的「PDF关键数据缺失」必须照计（尺子未放宽）");

    // ──  新闻面：指定「本系统无监管文书数据源（结构性）」，禁止「该维度按"无数据"处理」──
    let news_ok = "## 风险与数据缺口\n本系统无监管文书数据源（结构性），\
                   问询函/立案只能从公告标题间接识别；若存在未披露问询则可能推高空头。";
    let r = run_quality(&[("news", news_ok)], &[("news", 60.0)]);
    assert_eq!(hits(&r, "news"), 0, "新闻面指定措辞不得计失败标记：{news_ok}");

    let news_bad = "未获取到监管函/问询函/立案等A股特色风险源数据，该维度按\"无数据\"处理。";
    let r = run_quality(&[("news", news_bad)], &[("news", 60.0)]);
    assert_eq!(hits(&r, "news"), 1, "被禁止的「按\"无数据\"处理」必须照计");

    // ──  资金面：逐维度各写各的原因（北向带「停披」），禁止合并成「数据缺失维度（北向/两融）」──
    let hm_ok = "**风险提示：**\n北向个股净买入自 2024-08 起监管停披，仅有沪深股通成交额；\
                 两融侧该券属设计性缺席（非两融标的）。";
    let r = run_quality(&[("hm", hm_ok)], &[("hm", 85.0)]);
    assert_eq!(hits(&r, "hm"), 0, "资金面逐维度指定措辞不得计失败标记：{hm_ok}");

    let hm_bad = "1. 机构持续流出可能引发连锁抛售 2. 数据缺失维度（北向/两融）可能隐藏额外风险";
    let r = run_quality(&[("hm", hm_bad)], &[("hm", 85.0)]);
    assert_eq!(hits(&r, "hm"), 1, "被禁止的合并式「数据缺失维度」必须照计");
}

/// N1/N2/N3 的**提示词侧**绑定：三份 md 里必须真的写着指定的那个措辞。
///
/// 为什么必须锁：判据侧（上一条测试）只保证「那句话不计入」，它**看不见**提示词。
/// 若有人改提示词时把指定措辞换掉（或整段删掉），分析师会退回失败动词，
/// 而两条测试**都还是绿的** —— 那正是本仓反复踩过的「声明的机制 ≠ 运行的机制」。
/// 判据取 md 正文里的**引号内字面串**，与上一条测试用的是同一串，改一侧必红。
#[test]
fn n1_n3_expert_prompts_actually_prescribe_those_wordings() {
    const CAT_MD: &str = include_str!("../../../agency_experts/stock-analysis/catalyst-analyst.md");
    const NEWS_MD: &str = include_str!("../../../agency_experts/stock-analysis/news-analyst.md");
    const HM_MD: &str = include_str!("../../../agency_experts/stock-analysis/hot-money-tracker.md");

    assert!(
        CAT_MD.contains("正文未解析（结构性）") && CAT_MD.contains("PDF关键数据缺失"),
        "catalyst-analyst.md 的「公告通道先天边界」段缺失：\
         必须同时含指定措辞与被禁止措辞（后者是禁止清单，缺了就无法验证收紧的是产出不是尺子）"
    );
    assert!(
        NEWS_MD.contains("本系统无监管文书数据源（结构性）")
            && NEWS_MD.contains("该维度按\"无数据\"处理"),
        "news-analyst.md 的「无独立监管文书数据源」段缺失"
    );
    assert!(
        HM_MD.contains("监管停披") && HM_MD.contains("数据缺失维度（北向/两融）"),
        "hot-money-tracker.md 的「逐维度写原因、禁止合并式数据缺失」段缺失"
    );
    // N3 追加维度（000710 运行 `92849db2`）：游资席位/涨停接力
    assert!(
        HM_MD.contains("该窗口无游资席位上榜记录") && HM_MD.contains("涨停接力数据缺失"),
        "hot-money-tracker.md 的「游资席位/涨停接力」边界段缺失：\
         该维度是本轮唯一在 N3 落地后仍被计标记的资金面缺口"
    );
}

/// N4（2026-10-02，000710 运行 `92849db2`）：`=null` / `为 null` 归入**状态词**并按句级抑制。
///
/// 被判的真实缺陷：研报分析师写「`consensusEps=null (2026)` 表明当前**无机构**对该股提供
/// 2026 年 EPS 预测」——三个工具调用全 ok，该 null 是「无 2026 年覆盖」的真实状态，
/// 却因 `=null` 此前**没有任何抑制组**（走 v87「硬标记不做句级判定」捷径）被计 1 处失败标记
/// ⇒ 判「⚠️ 低置信」、归因「上游工具数据不完整」。
///
/// 本测试同时锁「尺子没放宽」的三侧：裸 `=null` 照计、跨句不连坐、动词类硬标记不受影响。
#[test]
fn n4_null_state_marker_is_sentence_suppressed_while_bare_null_still_counts() {
    // ① 真证明文：`=null` 与「无机构」同句 ⇒ 豁免
    let ok = "`consensusEps=null (2026)` 表明当前无机构对该股提供 2026 年 EPS 预测，\
              机构对 EPS 的认知无法被追踪。";
    let r = run_quality(&[("res", ok)], &[("res", 25.0)]);
    assert_eq!(hits(&r, "res"), 0, "=null 与无机构覆盖语境同句 ⇒ 不得计数据缺口：{ok}");

    // ② 负控：裸 `=null`、无任何缺席语境 ⇒ 照计（收紧的是分类，不是标准）
    let bare = "一致预期字段 consensusEps=null，本轮无法给出估值锚。";
    let r = run_quality(&[("res", bare)], &[("res", 60.0)]);
    assert_eq!(hits(&r, "res"), 1, "无缺席语境的裸 `=null` 必须照计");

    // ③ 跨句不连坐：无覆盖在第 1 句、`=null` 在第 2 句 ⇒ 第 2 句照计
    let cross = "该股暂无机构跟踪。\n研报接口返回的 consensusEps=null。";
    let r = run_quality(&[("res", cross)], &[("res", 60.0)]);
    assert_eq!(hits(&r, "res"), 1, "句级抑制必须逐句生效，不得跨句连坐");

    // ④ 动词类硬标记不受本次归类影响（`无法获取` 仍零抑制）
    let verb = "调用估值接口超时，无法获取一致预期数据。";
    let r = run_quality(&[("res", verb)], &[("res", 60.0)]);
    assert_eq!(hits(&r, "res"), 1, "动词类真故障标记必须维持零抑制");

    // ⑤ `为 null` 同族同判据（两串必须一起改，否则分析师换个写法就绕过）
    let asof = "北向个股净买入字段为 null（自 2024-08 起监管停披）。";
    let r = run_quality(&[("hm", asof)], &[("hm", 70.0)]);
    assert_eq!(hits(&r, "hm"), 0, "`为 null` 与停披语境同句 ⇒ 与 `=null` 同判据：{asof}");

    // ⑥ **归因对照**（非破坏性自证）：把命中的词换成动词类硬标记、**语境一字不改** ⇒ 必须照计。
    //    这条锁的是「豁免到底来自分组归属还是来自句子本身」——若有人日后把 `无法获取`
    //    也塞进抑制组，本条即红（那才是真的放宽尺子）。
    let verb_same_ctx = "一致预期无机构覆盖，无法获取 consensusEps。";
    let r = run_quality(&[("res", verb_same_ctx)], &[("res", 60.0)]);
    assert_eq!(
        hits(&r, "res"),
        1,
        "同一句「无机构」语境下，动词类硬标记必须照计 ⇒ ① 的豁免来自分组归属，不是来自文本"
    );
}

/// 同 `run_quality_with_verdicts`，但可注入**任意标量**外部输入。
///
/// 存在的理由：第四类缺席的判据是 `valuation_dcf_unavailable_reason`（一个字符串码），
/// `confs` / `verdicts` 两条通道都表达不了它 ⇒ 没有这个入口，该判据在门禁里恒不可达。
fn run_quality_with_scalars(reports: &[(&str, &str)], scalars: &[(&str, Dynamic)]) -> Map {
    let engine = build_engine();
    let ast = engine.compile(SCRIPT).expect("data-quality.rhai 编译失败");
    let mut scope = Scope::new();
    for v in EXTERNAL_VARS {
        scope.push_dynamic(*v, Dynamic::UNIT);
    }
    for (abbr, text) in reports {
        scope.push_dynamic(format!("{abbr}_report"), Dynamic::from(text.to_string()));
    }
    // 后 push 覆盖前面的 UNIT（Rhai scope 同名取最后一个）—— 与 run_quality_impl 同机制
    for (k, val) in scalars {
        scope.push_dynamic((*k).to_string(), val.clone());
    }
    engine
        .eval_ast_with_scope::<Map>(&mut scope, &ast)
        .expect("data-quality.rhai 执行失败（检查是否引用了未注册的宿主函数）")
}

/// 项3（2026-10-02，000710 运行 `92849db2`）：**第四类缺席** —— 估值方法对本标的不适用。
///
/// 被判的真实缺陷：PE −18、近5年报无正净利 ⇒ DCF 结构性不适用、`upsidePct` 恒 null，
/// 而 `missing_factors` 的判据只有 `!present(valuation_dcf_upside)` 一条 ⇒
/// 面板显示「缺失因子：估值上行空间」，把**标的属性**伪装成**我方取数缺口**。
///
/// 四张表的边界（本测试逐条锁住，防止后人合并）：
///   因子没值 / 上游没取到数 / 分析师漏字段 / **方法对本标的不适用**。
#[test]
fn item3_dcf_not_applicable_is_a_fourth_kind_of_absence() {
    let report = "该股持续亏损，估值以 PS 相对口径给出。";
    // 伞形判据 `available=false` + 细分码；两者必须一起注入（遮蔽态只有伞、没有码）
    let unavailable = |code: Option<&str>| {
        let mut v: Vec<(&str, Dynamic)> = vec![("valuation_dcf_available", Dynamic::from(false))];
        if let Some(c) = code {
            v.push(("valuation_dcf_unavailable_reason", Dynamic::from(c.to_string())));
        }
        v
    };

    // ① 标的属性：DCF 不适用（持续亏损）⇒ 记进第四张表，且**不**混进上游取数缺口
    let r = run_quality_with_scalars(&[("mk", report)], &unavailable(Some("persistent_loss")));
    let na = names(&r, "method_not_applicable");
    assert_eq!(na.len(), 1, "持续亏损应记一条方法不适用：{na:?}");
    assert!(na[0].contains("估值上行空间"), "须点明是哪个因子不适用：{}", na[0]);
    assert!(names(&r, "upstream_data_gaps").is_empty(), "标的属性不是我方采集缺陷");
    // 分母绑定不破：因子侧仍记缺失（`pm_compute_factor_completeness` 同样判它没值）
    assert!(
        names(&r, "missing_factors").contains(&"估值上行空间".to_string()),
        "missing_factors 仍须含「估值上行空间」—— 列表长度与公式分母绑死，不得摘除"
    );

    // ② 我方采集缺陷：同一路径的另一个码 ⇒ 记进**上游缺口**，且**不**记方法不适用
    let r = run_quality_with_scalars(&[("mk", report)], &unavailable(Some("fcf_data_missing")));
    assert!(names(&r, "method_not_applicable").is_empty(), "缺数不是「方法不适用」");
    assert_eq!(names(&r, "upstream_data_gaps").len(), 1, "缺数必须回到上游缺口那一栏");

    // ②b 遮蔽态：`available=false` 但**没有码**（中性档超出 现价×1%~×10000% 被遮蔽）
    //   ⇒ 仍属第四类缺席。只判码会漏掉这一路，遮蔽态又会只剩「缺失因子」一条孤讯。
    let r = run_quality_with_scalars(&[("mk", report)], &unavailable(None));
    let na = names(&r, "method_not_applicable");
    assert_eq!(na.len(), 1, "遮蔽态必须记一条方法不适用：{na:?}");
    assert!(na[0].contains("超出量程被遮蔽"), "应说明是遮蔽而非取数故障：{}", na[0]);
    assert!(names(&r, "upstream_data_gaps").is_empty(), "遮蔽不是采集缺陷，不得串栏");

    // ③ 只告警**不扣分**：同一个标的，报不报方法不适用，综合分必须一致
    //   （与 upstream_data_gaps / verdict_field_gaps 同一处置纪律）
    let scored = run_quality_with_scalars(&[("mk", report)], &unavailable(Some("persistent_loss")));
    let plain = run_quality_with_scalars(
        &[("mk", report)],
        &[("valuation_dcf_available", Dynamic::from(true))],
    );
    assert_eq!(
        scored["score"].clone().try_cast::<f64>().expect("score 应为浮点"),
        plain["score"].clone().try_cast::<f64>().expect("score 应为浮点"),
        "第四类缺席不得改变 score —— 改变即说明它被接进了扣分路径"
    );
    assert_eq!(
        scored["grade"].clone().try_cast::<String>().expect("grade 应为字符串"),
        plain["grade"].clone().try_cast::<String>().expect("grade 应为字符串"),
        "第四类缺席不得改变 grade"
    );
    assert!(names(&plain, "method_not_applicable").is_empty(), "DCF 可用时不得凭空造出告警");

    // ④ 旧快照 / 快速链降级：该路解析不到值 ⇒ 不得因此误报
    let r = run_quality_with_scalars(&[("mk", report)], &[]);
    assert!(names(&r, "method_not_applicable").is_empty(), "取不到值 ⇒ 保持沉默，不得猜");
}

/// M1（2026-10-02，000710 运行 `92849db2`）：状态三分类必须是**真划分**，且与逐节点表同源。
///
/// 被判的真实缺陷：弹窗把 `good_count / degraded_count / gap_count` 并排渲染成
/// 「对 10 个分析师的三分类」（下面标着「10 个分析师」），但 `degraded_count` 是
/// 「自评 ≥50 且含失败标记」的**虚高子集**、不是与 good 并列的桶，另有
/// 「自评 <50、无标记」一类三个都不落 ⇒ 表格显示 **2** 行「⚠️ 低置信」而芯片显示 8/1/0，
/// 加总 9 ≠ 10，**一个分析师在视觉上凭空消失**。
#[test]
fn status_counts_form_a_partition_of_total() {
    // 复刻该轮形态：资金面 conf 55 含标记（虚高）、研报 conf 25 含标记（单纯低把握）、
    // 技术面 conf 75 干净；其余 7 位本轮无 verdict。
    let hm = "北向净买入接口调用超时，未能获取单股精确净买入数据。";
    let res = "一致预期字段 consensusEps=null，本轮无估值锚。";
    let mk = "均线多头排列，量能温和放大，RSI 处于中性区。";
    let r = run_quality(
        &[("hm", hm), ("res", res), ("mk", mk)],
        &[("hm", 55.0), ("res", 25.0), ("mk", 75.0)],
    );

    let num =
        |k: &str| -> i64 { r[k].clone().try_cast::<i64>().expect("计数字段应为整数") };
    let normal = num("status_normal_count");
    let low = num("status_low_count");
    let missing = num("status_missing_count");
    let total = num("status_total_count");

    // ① 真划分：三栏加总 = 总数 = diagnostics 条目数
    assert_eq!(normal + low + missing, total, "三分类必须加总等于总数");
    let d = r["diagnostics"].clone().try_cast::<Map>().expect("diagnostics 不是 map");
    assert_eq!(total, d.len() as i64, "总数必须等于逐节点表行数（同源）");

    // ② 与表格「状态」列逐行对得上
    assert_eq!(low, 2, "资金面 + 研报两行应计 2 个低置信");
    assert_eq!(normal, 1, "技术面一行应计 1 个正常");
    assert_eq!(missing, 7, "其余 7 位无 verdict ⇒ 计缺失/不可信");
    for abbr in ["hm", "res"] {
        assert_eq!(status(&r, abbr), "low", "节点 {abbr} 表格应显示低置信");
    }

    // ③ **本条测试存在的全部理由**：旧芯片用的 degraded_count 在此只有 1
    //   （它只数「自评 ≥50 且含标记」的虚高那一个），若继续拿它当分类位，
    //   面板就会在表格显示 2 行低置信的同时标出 1 —— 正是被修的缺陷。
    assert_eq!(num("degraded_count"), 1, "degraded 是 low 的子集，不是并列桶");
    assert_ne!(
        low,
        num("degraded_count"),
        "低置信数与虚高数必须**可以不等** —— 相等只是巧合，用它当分类位必然少报"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// R2（2026-10-02）：措辞性缺席 —— 失败标记的成立条件从「文本出现了哪个词」
// 换成「本轮该节点的工具调用有没有失败」。
//
// 触发实证（000710 后续运行 `80a41e56`）：基本面 / 资金面 / 技术面 / 研报**四行**低置信，
// 而它们说的是**同一个事实** —— `get_stock_institutional_visits` 返回 `[]`，四种措辞：
//   「机构调研数据缺失」「调用返回空数组 []，属该股票当前暂无机构调研记录（事件型缺席，
//    非工具故障）」「机构调研返回空数组」「confidence 60 如实反映行业维度数据缺失」。
// 资金面那句**已经把性质写对了**却仍被计入 —— `返回空` 的豁免表里只有字面「暂无数据」，
// 实文是「暂无机构调研记录」，差两个字豁免失效。
// ⇒ 只要判据看措辞，补多少短语都收敛不了（措辞是开放集）。本组测试锁新轴。
// ─────────────────────────────────────────────────────────────────────────────

/// 真证明文：分析师如实描述「该维度没有记录」，措辞正确。
const R2_HONEST_ABSENCE: &str =
    "机构调研：调用返回空数组 []，属该股票当前暂无机构调研记录（事件型缺席，非工具故障）。";

#[test]
fn r2_state_word_absence_is_cleared_only_by_clean_successful_tool_calls() {
    // ① 有调用记录且零失败 ⇒ 不计标记、不降置信，但**必须留痕**（静默吞掉=另一种造假）
    let ok_calls =
        vec![tc("get_stock_institutional_visits", false), tc("get_stock_consensus_eps", false)];
    let r = run_quality_with_tool_calls(
        &[("hm", R2_HONEST_ABSENCE)],
        &[("hm", 55.0)],
        &[("hm", ok_calls.clone())],
    );
    assert_eq!(hits(&r, "hm"), 0, "状态词缺席 + 工具全成功 ⇒ 不得计数据缺口");
    assert_eq!(status(&r, "hm"), "normal", "不得因被豁免的措辞降 low");
    let g = gap_reason(&r, "hm");
    assert!(g.contains("无一失败"), "豁免必须说清理由，实得: {g}");
    assert!(g.contains("没有记录"), "理由要指到「该维度没有记录」这一性质，实得: {g}");

    // ② 负控（假阴性防线）：**有一条失败记录** ⇒ 标记成立，不得豁免
    let mut mixed = ok_calls.clone();
    mixed.push(tc("get_stock_dragon_tiger", true));
    let r = run_quality_with_tool_calls(
        &[("hm", R2_HONEST_ABSENCE)],
        &[("hm", 55.0)],
        &[("hm", mixed)],
    );
    assert_eq!(hits(&r, "hm"), 1, "本轮确有失败调用 ⇒ 缺席措辞成立，不得被吞");
    assert_eq!(status(&r, "hm"), "low", "有失败证据时照判低置信");

    // ③ 负控（守卫①）：**无可核对记录** ⇒ 不判。
    //    「测不出」不能当成「没问题」——旧快照缺字段、或该节点本就无工具时都走这条。
    let r = run_quality(&[("hm", R2_HONEST_ABSENCE)], &[("hm", 55.0)]);
    assert_eq!(hits(&r, "hm"), 1, "无调用记录时不得豁免（否则空记录即可洗白一切措辞）");
    let empty_calls: Vec<Dynamic> = vec![];
    let r = run_quality_with_tool_calls(
        &[("hm", R2_HONEST_ABSENCE)],
        &[("hm", 55.0)],
        &[("hm", empty_calls)],
    );
    assert_eq!(hits(&r, "hm"), 1, "空数组记录同样不可核对 ⇒ 不得豁免");

    // ④ 负控（动词零豁免）：换成动词类措辞、工具全成功 ⇒ **照计**。
    //    真缺口正是用动词表达的，这条是「宁漏不误伤真故障」的边界锁。
    let verb = "机构调研接口异常，无法获取调研记录。";
    let r =
        run_quality_with_tool_calls(&[("hm", verb)], &[("hm", 55.0)], &[("hm", ok_calls.clone())]);
    assert_eq!(hits(&r, "hm"), 1, "动词类硬标记不得因「工具无失败」被豁免");
    assert_eq!(status(&r, "hm"), "low", "动词类仍判低置信");

    // ⑤ 混合：同节点既有状态词又有动词 ⇒ **整体不豁免**（保守），两处措辞都保留计数
    let both = format!("{}\n{}", R2_HONEST_ABSENCE, "龙虎榜席位无法获取当日明细。");
    let r = run_quality_with_tool_calls(&[("hm", &both)], &[("hm", 55.0)], &[("hm", ok_calls)]);
    assert_eq!(hits(&r, "hm"), 2, "含动词时不得整节点豁免：状态词与动词两处都须保留");
}

/// R2 的**真实语料**回归：把 `80a41e56` 那轮四条命中的原句一起灌进来，
/// 断言在「各自工具调用全成功」下**四行全部转正常**。
///
/// 为什么单独一条：① 用的是我改写的例句，本条用的是**面板上真实出现过**的四句 ——
/// 判据换轴若只对自己造的样本成立，等于没修。
#[test]
fn r2_clears_the_four_real_reports_from_run_80a41e56() {
    let cases: [(&str, &str, &str); 4] = [
        // 研报：一句里同时出现两个状态词（数据缺失 + 返回空）
        (
            "res",
            "**机构调研数据缺失**：`get_stock_institutional_visits`返回空数组，无法判断近期是否有密集机构调研带来的认知增量，该维度对本次结论影响中等（调研缺席本身可解读为关注度阶段性下降）。",
            "get_stock_institutional_visits",
        ),
        ("hm", R2_HONEST_ABSENCE, "get_stock_institutional_visits"),
        (
            "fund",
            "机构调研返回空数组，无法判断近期机构行为方向，按“无机构覆盖/数据源未取到”处理，降低资金面置信度。",
            "get_stock_institutional_visits",
        ),
        // 技术面：说的是**别的分析师**那个维度的缺席
        ("mk", "confidence 60 如实反映行业维度数据缺失，本次不据此调整评分。", "compute_scoring"),
    ];

    for (abbr, text, tool) in cases {
        let calls = vec![tc(tool, false)];
        let r = run_quality_with_tool_calls(&[(abbr, text)], &[(abbr, 60.0)], &[(abbr, calls)]);
        assert_eq!(
            hits(&r, abbr),
            0,
            "节点 {abbr} 的真实原句应被事实判据豁免（面板上它曾被计标记并判低置信）"
        );
        assert_eq!(status(&r, abbr), "normal", "节点 {abbr} 应回到正常");
    }
}
