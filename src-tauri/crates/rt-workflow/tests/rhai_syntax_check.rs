//! 语法编译测试：用 rhai v1.25.0 编译 src/commands 下所有 .rhai 文件。
//!
//! 目的：CI 阶段就一次性捕获所有 Rhai 语法错误（如 `as f64`/`let mut` 等
//! Rust 残留语法），避免运行时才报错导致反复修复。
//!
//! 注意：本测试只编译（engine.compile），不执行。输入变量全部注入空值 ()，
//! 因为 compile 阶段不需要变量实际有值——只需要语法合法。
//!
//! ⚠ 本文件同时承载 **`.rhai` 契约 / 语义门禁**（不是语法）：见文件末的
//! `regime_weights_consumed_set_matches_portfolio_mgr`（跨文件名字集合对齐）与
//! `rhai_weights_semantics_consistent` / `rhai_weights_semantics_gate_discriminates`
//! （值形状、清单双向往返、白名单可达性、契约表声明↔代码实现）。
use rhai::Engine;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// 注册所有脚本中用到的全局辅助函数（与运行时注册的函数保持一致）。
fn register_globals(engine: &mut Engine) {
    engine.register_fn("clamp", |v: f64, min: f64, max: f64| -> f64 {
        if v < min {
            min
        } else if v > max {
            max
        } else {
            v
        }
    });
    engine.register_fn("join", |arr: rhai::Array, sep: &str| -> String {
        arr.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(sep)
    });
    // json_parse 返回 unit 即可，编译阶段不解析 JSON
    engine.register_fn("json_parse", |_s: &str| -> rhai::Dynamic { rhai::Dynamic::UNIT });
    // print 在测试中无操作
    engine.register_fn("print", |_s: &str| {});
}

/// 列出 src/commands 下所有 .rhai 文件路径。
fn collect_rhai_files() -> Vec<PathBuf> {
    // CARGO_MANIFEST_DIR = src-tauri/crates/rt-workflow
    // 需要到 src-tauri/src/commands，即向上两级再进入 src/commands
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("src")
        .join("commands");
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("读取 rhai 目录失败 {:?}: {e}", dir))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "rhai").unwrap_or(false))
        .collect()
}

/// 对单个 .rhai 文件做编译检查。返回 Ok(()) 或 Err(错误信息)。
fn compile_one(path: &PathBuf) -> Result<(), String> {
    let code = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
    let mut engine = Engine::new();
    engine.set_max_expr_depths(1024, 1024);
    register_globals(&mut engine);
    // 编译阶段不需要注入变量值——未定义变量在 compile 时不会报错
    // （Rhai 的变量解析在运行时）。这里只测语法合法性。
    engine.compile(&code).map(|_| ()).map_err(|e| format!("{e}"))
}

#[test]
fn all_rhai_scripts_compile() {
    let files = collect_rhai_files();
    assert!(!files.is_empty(), "未找到任何 .rhai 文件，测试目录可能配置错误");

    let mut failures = Vec::new();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        match compile_one(f) {
            Ok(()) => eprintln!("=== PARSE OK: {name} ==="),
            Err(e) => failures.push(format!("[{name}] {e}")),
        }
    }

    if !failures.is_empty() {
        panic!(
            "以下 .rhai 文件编译失败 (共 {}/{}):\n\n{}",
            failures.len(),
            files.len(),
            failures.join("\n\n")
        );
    }
}

/// 兼容旧测试名：单独验证 bottleneck-calc.rhai 仍可编译。
#[test]
fn bottleneck_calc_v9_compiles() {
    let code = include_str!("../../../src/commands/bottleneck-calc.rhai");
    let mut engine = Engine::new();
    engine.set_max_expr_depths(1024, 1024);
    register_globals(&mut engine);
    match engine.compile(code) {
        Ok(_) => eprintln!("=== PARSE OK ==="),
        Err(e) => panic!("编译失败: {e}"),
    }
}

/// 防回归：`portfolio-mgr.rhai` 不得重新引入「观望 ⇄ 持有」互改。
///
/// 背景（2026-09-14）：`action` 曾同时承载「方向强度」与「持仓状态」两个维度 ——
/// 同一中性档因仓位有无被**双向**改写：
///   · 升级向：试探仓块把 `base_action` 由「观望」改成「持有」；
///   · 降级向：`position_pct<=0` 时把 买入/增持/持有 统一改成「观望」。
/// 两轴拆开后，持仓状态由 `positionState` 独立表达，上述互改已移除；
/// 落库的 `action` 因而保真（空仓看多的记录保留「买入」而非被改写成「观望」）。
///
/// 本测试用**文本判据**钉住，防止后续改动无意中把它加回来。
/// （行为级测试需要 DB + 完整工作流环境，成本过高；此处防的是「重新引入」这一类回归。）
#[test]
fn portfolio_mgr_has_no_hold_wait_mutual_rewrite() {
    let code = include_str!("../../../src/commands/portfolio-mgr.rhai");
    // 只判**代码行**：注释里必然出现这些字样的说明文字，不能误当代码
    let code_only: String =
        code.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");

    // ① 「观望 → 持有」升级向：`base_action = "持有"` 应只剩后验阶梯判定那一处
    assert_eq!(
        code_only.matches("base_action = \"持有\";").count(),
        1,
        "portfolio-mgr.rhai 重新出现了试探仓的 `base_action = \"持有\"`（互改的升级向）"
    );
    // ② 「零仓位 ⇒ 降级为观望」降级向
    assert!(
        !code_only.contains("position_pct <= 0.0 && (base_action == \"买入\""),
        "「零仓位 ⇒ 降级为观望」分支回归了（互改的降级向）"
    );
    // ③ 「观望 ⇒ 清零仓位」反向耦合（不删则试探仓会被静默清零）
    assert!(
        !code_only.contains("final_action == \"观望\" && position_pct > 0.0"),
        "「观望 ⇒ 清零仓位」分支回归了（会使试探仓静默失效）"
    );
    // ④ 持仓状态轴必须仍然输出 —— 它是「两轴正交」成立的前提
    assert!(
        code_only.contains("\"positionState\": position_state"),
        "portfolio-mgr.rhai 不再输出 positionState ⇒ 两轴正交被破坏，互改的前提回来了"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// v61(2026-09-20) 跨文件契约门禁：`regime-weights.rhai` 产出 ⇄ `portfolio-mgr.rhai` 消费
// ─────────────────────────────────────────────────────────────────────────────

/// 从 pm 源码里提取所有 `get_weight(<weights>, "factor", …)` 的 factor 名。
///
/// 排除两类噪声（二者都真实存在于 pm 文件里，误收会造成假红）：
///   · **注释行** —— pm 有大量形如「改走 `get_weight(f_weights, "bottleneck", 0.10)`」的说明文字；
///   · **`fn get_weight(weights, factor, fallback)` 定义行** —— 其函数体内含
///     `type_of(weights) == "map"` 这类字符串字面量，会被「首个 `"` 即 factor」的规则误收成 `"map"`。
fn extract_get_weight_factors(pm_code: &str) -> BTreeSet<String> {
    let code_only: String = pm_code
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .filter(|l| !l.contains("fn get_weight"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut out = BTreeSet::new();
    let mut rest: &str = &code_only;
    while let Some(idx) = rest.find("get_weight(") {
        rest = &rest[idx + "get_weight(".len()..];
        // 只在有限窗口内找第一个字符串字面量：多行调用的窗口足够，又不会吃到下一个调用。
        // 注意必须回退到 char 边界 —— 切在多字节字符中间会 panic。
        let mut cut = rest.len().min(120);
        while cut > 0 && !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let window = &rest[..cut];
        // 用 let-else + continue 而非嵌套 if：本仓风格（同文件其它处亦如此），
        // 且语义完全等价 —— 窗口内无字符串字面量时本来就应该继续找下一个调用。
        // （嵌套 if 会被 clippy::collapsible_if 要求折叠成 let-chain。）
        let Some(q1) = window.find('"') else { continue };
        let Some(q2) = window[q1 + 1..].find('"') else { continue };
        out.insert(window[q1 + 1..q1 + 1 + q2].to_string());
    }
    out
}

/// 从 regime-weights 源码里提取 `consumed_names = [...]` 声明的因子名。
fn extract_consumed_names(rw_code: &str) -> BTreeSet<String> {
    let code_only: String = rw_code
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut out = BTreeSet::new();
    let Some(i) = code_only.find("consumed_names = [") else {
        return out;
    };
    let seg = &code_only[i..];
    let Some(j) = seg.find(']') else {
        return out;
    };
    // 只在 `[` … `]` 之间取样，避免串到下一个数组
    let arr = &seg[..j];
    let mut rest: &str = arr;
    while let Some(q1) = rest.find('"') {
        rest = &rest[q1 + 1..];
        match rest.find('"') {
            Some(q2) => {
                out.insert(rest[..q2].to_string());
                rest = &rest[q2 + 1..];
            },
            None => break,
        }
    }
    out
}

/// 防回归：`regime-weights.rhai` 的消费白名单必须与 `portfolio-mgr.rhai` 的
/// `get_weight` 调用集合**精确对齐**。
///
/// 背景（v61, 2026-09-20）：f13 瓶颈因子由硬编码 `0.10` 改为
/// `get_weight(f_weights, "bottleneck", f13_default)`，这条改动跨**两个文件**：
///   · `portfolio-mgr.rhai` = **消费端**（`get_weight` 读 `weights[factor]["weight"]`）
///   · `regime-weights.rhai` = **产出端**（`consumed_names` 决定谁进 `factor_weights`）
///
/// 两文件各自被 `include_str!` 塞进**不同的 CodeNode**，彼此不可见；`.rhai` 又无类型系统
/// ⇒ **只改一侧不报任何错**，且两种漏法都是**静默**的：
///   · 只改 pm（消费端加了因子名，产出端未提供条目）
///     ⇒ `get_weight` 走 `type_of(f) != "map"` 分支**恒取 fallback**
///     ⇒ 白名单等于空接：市况自适应**看起来改了、实际没改**。
///   · 只改 regime-weights（产出端加了条目，消费端不读）
///     ⇒ 条目落进 `unconsumed_suggestions` ⇒ 又一处「死计算」（同文件头 P1-F 记录的问题）。
///
/// 等式：`pm 的 get_weight factor 集合 == consumed_names ∪ IC 加权因子`。
/// `pace`/`momentum` 由 pm 侧注释明确「走 IC 回测权重」，**故意不在**产出端提供条目
/// ⇒ 它们的 `get_weight` 恒取 fallback（fallback 即回测权重），属**设计如此**。
/// 新增因子时本测试强制作者同时改两侧，或在下面白名单里显式登记。
#[test]
fn regime_weights_consumed_set_matches_portfolio_mgr() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    let rw = include_str!("../../../src/commands/regime-weights.rhai");

    let pm_factors = extract_get_weight_factors(pm);
    let consumed = extract_consumed_names(rw);

    // 前提自证：两侧都必须真的提取到内容，否则「相等」可能是两个空集的假绿
    assert!(
        pm_factors.len() >= 6,
        "pm 侧未提取到 get_weight 调用（扫描器失效或文件结构已变）: {pm_factors:?}"
    );
    assert!(
        consumed.len() >= 4,
        "regime-weights 侧未提取到 consumed_names（扫描器失效或声明已改名）: {consumed:?}"
    );

    let ic_weighted: BTreeSet<String> =
        ["pace", "momentum"].iter().map(|s| (*s).to_string()).collect();
    let declared: BTreeSet<String> = consumed.union(&ic_weighted).cloned().collect();

    let only_in_pm: Vec<_> = pm_factors.difference(&declared).cloned().collect();
    let only_in_rw: Vec<_> = declared.difference(&pm_factors).cloned().collect();

    assert!(
        only_in_pm.is_empty(),
        "以下因子只被 portfolio-mgr 消费，但 regime-weights 未产出条目 ⇒ \
         get_weight 恒取 fallback，市况自适应静默失效（白名单空接）: {only_in_pm:?}"
    );
    assert!(
        only_in_rw.is_empty(),
        "以下因子在 regime-weights 声明产出，但 portfolio-mgr 不消费 ⇒ \
         条目落进 unconsumed_suggestions，属死计算: {only_in_rw:?}"
    );
}

/// 负对照：证明上面的门禁**真的会告警**（否则可能只是「两个空集相等」）。
#[test]
fn regime_weights_contract_scanner_discriminates() {
    // ① 新增一个只被 pm 消费的因子 ⇒ 必须被逐个扫出 ⇒ 等式会破
    let synthetic = "let a = get_weight(f_weights, \"trend\", 0.15);\n\
                     let b = get_weight(f_weights, \"brand_new_factor\", 0.10);\n";
    let got = extract_get_weight_factors(synthetic);
    assert_eq!(got.len(), 2, "扫描器必须逐个抓出调用: {got:?}");
    assert!(got.contains("brand_new_factor"), "扫描器抓不出新增因子 ⇒ 门禁永不告警");

    // ② 注释里的调用不得计入（pm 文件里确实存在这类说明文字）
    let commented = "// get_weight(f_weights, \"ghost\", 0.1)\n\
                     let a = get_weight(f_weights, \"trend\", 0.15);";
    let got2 = extract_get_weight_factors(commented);
    assert!(!got2.contains("ghost"), "注释里的调用被计入 ⇒ 会假红: {got2:?}");

    // ③ 定义行 + 函数体内的字符串字面量不得被误当成 factor（典型误收 "\"map\""）
    let def = "fn get_weight(weights, factor, fallback) {\n\
                   if type_of(weights) == \"map\" { 1.0 } else { 0.0 }\n\
               }";
    assert!(
        extract_get_weight_factors(def).is_empty(),
        "定义行/函数体被误当成调用（典型误收 \"map\"）: {:?}",
        extract_get_weight_factors(def)
    );

    // ④ consumed_names 提取器不得串到下一个数组
    let rw_frag = "let consumed_names = [\"a\", \"b\"];\nlet other = [\"c\"];";
    let expect: BTreeSet<String> = ["a", "b"].iter().map(|s| (*s).to_string()).collect();
    assert_eq!(extract_consumed_names(rw_frag), expect, "consumed_names 提取器串到了下一个数组");

    // ⑤ 缺声明时必须返回空集而不是 panic（真实文件被改名/删声明时的降级路径）
    assert!(extract_consumed_names("let x = 1;").is_empty(), "无 consumed_names 声明时应返回空集");
}

// ─────────────────────────────────────────────────────────────────────────────
// 第二阶段：`.rhai` **语义**（而非语法）门禁
// ─────────────────────────────────────────────────────────────────────────────
// 背景：上面的 `regime_weights_consumed_set_matches_portfolio_mgr` 只覆盖**跨文件名字集合
// 对齐**；「`get_weight` 的取值逻辑对不对」当时被判定为「无门禁、只能靠源码复核」
// （结转记录见 `AUDIT-codebase-review-roadmap-2026-09-19.md` §10.3）。本批把其中
// **静态可判**的四类补上门禁：
//
//   A 值形状     `factors` 每个条目必须含 `base` + `regime_key` 取的**每一档**市况。
//                缺档 ⇒ `f[regime_key]` 得 `()` ⇒ `base * ()`，而且**只在该市况下**才走到
//                ⇒ `compile` 门禁（本文件上半部分）永远看不见。
//   B 清单齐全   `factor_names ⊆ factors`（否则 `factors[name]` 得 `()`）；反向
//                `factors ⊆ factor_names`（否则该条目永不产出 = 死数据）。
//   C 白名单可达 `consumed_names ⊆ factor_names`。
//                ★ 这是**现有跨文件门禁查不出**的一类：产出端循环只遍历 `factor_names`，
//                所以 `consumed_names` 里多出的名字永远不会进 `factor_weights`
//                ⇒ `get_weight` 恒取 fallback。而现有门禁只比对 `consumed_names ↔ pm 字面量`，
//                两侧都错成同一个名字时**一致通过**。
//   D 声明↔代码  `portfolio-mgr.rhai:216-229` 的「权重政策契约表」是**声明**，代码是**实现**，
//                逐行核对。三类来源各有**不同**的正确含义（v1 原型在此处踩坑，见下）：
//                  · `regime_factor_weights.<X>` ⇒ 必须调 `get_weight("<X>")`，且 X 在 consumed_names
//                  · `回测 IC 权重（fallback V）`   ⇒ 必须调 `get_weight`（**故意**靠 fallback），
//                    且该名**不在** consumed_names，且 `fN_default == V`
//                  · `硬编码 V`                     ⇒ 必须**不**调 get_weight，且实际值 == V
//                ⚠ v1 把「不消费 regime」误读成「不得调 get_weight」，于是在 f11/f12 上报了假红 ——
//                  门禁在正确代码上必须恒绿，否则只会训练人绕过它。
//   D 反向      代码里出现的每个 `fN_weight` 都必须在契约表有行（否则改代码无处核对）。

/// 一个 `factors` 条目的解析结果：(因子名, [(键, 值)])。
type FactorEntry = (String, Vec<(String, f64)>);

/// 去掉**整行**注释（口径与上方既有 `extract_*` 一致：只滤 `trim_start` 以 `//` 开头的行）。
fn strip_line_comments(src: &str) -> String {
    src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

/// 取 `s` 中**第一个**引号对的内容及其之后的偏移。
fn first_quoted(s: &str) -> Option<(String, usize)> {
    let a = s.find('"')?;
    let b = a + 1 + s[a + 1..].find('"')?;
    Some((s[a + 1..b].to_string(), b + 1))
}

/// 取 `s` 中**最后一个**引号对的内容（用于 `": #{"` 之前的条目名）。
fn last_quoted(s: &str) -> Option<String> {
    let e = s.rfind('"')?;
    let a = s[..e].rfind('"')?;
    Some(s[a + 1..e].to_string())
}

/// 从 `open`（必须是 `{` 的下标）起配平花括号，返回**内部**文本。
fn brace_body_at(src: &str, open: usize) -> Option<String> {
    let mut depth = 0i32;
    for (off, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(src[open + 1..open + off].to_string());
                }
            },
            _ => {},
        }
    }
    None
}

/// 取 `decl` 之后第一个 `{` 起、配平到 `}` 的**内部**文本。
fn block_body_after(src: &str, decl: &str) -> Option<String> {
    let i = src.find(decl)?;
    let open = i + src[i..].find('{')?;
    brace_body_at(src, open)
}

/// 跳过空白与首个逗号（用于 `get_weight(f_weights, "<name>", ...)`）。
fn after_comma(s: &str) -> Option<&str> {
    Some(s.trim_start().strip_prefix(',')?.trim_start())
}

/// 跳过非数字前缀，读出一个 `f64` 字面量（`fallback 0.08` / `硬编码 0.20` / `= 0.10;`）。
fn number_after(s: &str) -> Option<f64> {
    let start = s.as_bytes().iter().position(|b| b.is_ascii_digit() || *b == b'-' || *b == b'.')?;
    let num: String =
        s[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
    num.parse::<f64>().ok()
}

/// `#{ "k": v, ... }` 的内部 ⇒ [(k, v)]。
fn key_values(body: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some((k, after)) = first_quoted(rest) {
        let num = rest[after..].trim_start().strip_prefix(':').and_then(number_after);
        if let Some(n) = num {
            out.push((k, n));
        }
        rest = &rest[after..];
    }
    out
}

/// `let factors = #{ "n": #{ "k": v, ... }, ... }` ⇒ [(n, [(k, v)])]。
fn extract_factors(rw: &str) -> Vec<FactorEntry> {
    let code = strip_line_comments(rw);
    let Some(body) = block_body_after(&code, "let factors = #{") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut rest = body.as_str();
    while let Some(p) = rest.find(": #{") {
        let name = last_quoted(&rest[..p]).unwrap_or_default();
        let inner = brace_body_at(rest, p + 3).unwrap_or_default();
        out.push((name, key_values(&inner)));
        rest = &rest[p + 3..];
    }
    out
}

/// `let <name> = [ "a", "b" ]` ⇒ ["a","b"]（只取 `[`…`]` 之间，不串到下一个数组）。
fn extract_str_array(src: &str, name: &str) -> Vec<String> {
    let code = strip_line_comments(src);
    let pat = format!("{name} = [");
    let Some(i) = code.find(&pat) else {
        return Vec::new();
    };
    let from = i + pat.len();
    let Some(j) = code[from..].find(']') else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut rest = &code[from..from + j];
    while let Some((v, after)) = first_quoted(rest) {
        out.push(v);
        rest = &rest[after..];
    }
    out
}

/// `let regime_key = if volatility == "high" {...}` 的市况档位字面量。
/// **自锚定**：不写死「四档」，档位集会随代码变化（`"high"` 是 volatility 的取值，非档位键）。
fn extract_regime_keys(rw: &str) -> Vec<String> {
    let code = strip_line_comments(rw);
    let Some(i) = code.find("let regime_key =") else {
        return Vec::new();
    };
    let end = match code[i..].find(';') {
        Some(d) => i + d,
        None => code.len(),
    };
    let mut out: Vec<String> = Vec::new();
    let mut rest = &code[i..end];
    while let Some((v, after)) = first_quoted(rest) {
        if v != "high" && !out.contains(&v) {
            out.push(v);
        }
        rest = &rest[after..];
    }
    out
}

/// 条目必需键 = 产出端**循环体**里 `f["<字面量>"]` 的键，外加（若写了 `f[regime_key]`）全部档位键。
/// 只看循环体：`get_weight` 函数体内也有 `f["weight"]`，全局扫会把它误收成必需键（假红）。
fn extract_loop_required_keys(rw: &str) -> (Vec<String>, bool) {
    let code = strip_line_comments(rw);
    let Some(body) = block_body_after(&code, "for name in factor_names {") else {
        return (Vec::new(), false);
    };
    let mut out: Vec<String> = Vec::new();
    let mut rest = body.as_str();
    while let Some(i) = rest.find("f[") {
        let key = rest[i + 2..]
            .trim_start()
            .strip_prefix('"')
            .and_then(|inner| inner.find('"').map(|e| inner[..e].to_string()))
            .filter(|k| !out.contains(k));
        if let Some(k) = key {
            out.push(k);
        }
        rest = &rest[i + 2..];
    }
    (out, body.contains("f[regime_key]"))
}

/// pm 侧一个 `fN_weight` 的取值面。
#[derive(Default)]
struct PmWeight {
    /// `get_weight(f_weights, "<name>", ...)` 里的因子名（可多个：初始化 + 条件赋值）。
    factors: Vec<String>,
    /// 纯数字字面量体（`let f7_weight = 0.10;` ⇒ [0.1]）。
    literals: Vec<f64>,
    /// 原始右值摘录，供报错时给出「实为 …」的证据。
    raw: Vec<String>,
}

/// 在 `s[i]`（必须是 `'f'`）处尝试匹配 `f<digits>_weight = <body>;`。
/// 同时认 `let fN_weight = …` 与**裸赋值** `fN_weight = …`（f13 是后者）。
fn match_pm_weight(s: &str, i: usize) -> Option<(u32, String)> {
    let tail = &s[i + 1..];
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let after = tail[digits.len()..].strip_prefix("_weight")?;
    let body = after.trim_start().strip_prefix('=')?;
    let semi = body.find(';')?;
    Some((
        digits.parse::<u32>().ok()?,
        body[..semi].split_whitespace().collect::<Vec<_>>().join(" "),
    ))
}

/// `get_weight(f_weights, "<name>", ...)` ⇒ ["<name>"]，其它形态返回空。
fn weight_factor_names(body: &str) -> Vec<String> {
    let Some(i) = body.find("get_weight") else {
        return Vec::new();
    };
    let Some(p) = body[i..].find("f_weights") else {
        return Vec::new();
    };
    let Some(t) = after_comma(&body[i + p + "f_weights".len()..]) else {
        return Vec::new();
    };
    match first_quoted(t) {
        Some((name, _)) => vec![name],
        None => Vec::new(),
    }
}

fn extract_pm_weights(pm: &str) -> Vec<(u32, PmWeight)> {
    let code = strip_line_comments(pm);
    let mut hits: Vec<(u32, String)> = Vec::new();
    let mut idx = 0usize;
    while let Some(off) = code[idx..].find('f') {
        let i = idx + off;
        if let Some(h) = match_pm_weight(&code, i) {
            hits.push(h);
        }
        idx = i + 1;
    }

    let mut out: Vec<(u32, PmWeight)> = Vec::new();
    for (n, body) in hits {
        if !out.iter().any(|(k, _)| *k == n) {
            out.push((n, PmWeight::default()));
        }
        if let Some((_, e)) = out.iter_mut().find(|(k, _)| *k == n) {
            e.factors.extend(weight_factor_names(&body));
            if let Ok(v) = body.trim().parse::<f64>() {
                e.literals.push(v);
            }
            e.raw.push(body.chars().take(80).collect());
        }
    }
    for entry in &mut out {
        let e = &mut entry.1;
        e.factors.sort();
        e.factors.dedup();
        e.literals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        e.literals.dedup();
    }
    out.sort_by_key(|(k, _)| *k);
    out
}

/// 在 `s[at]`（`_default` 的起始）处回读 `let f<digits>_default = <num>;`。
fn match_default_decl(s: &str, at: usize) -> Option<(u32, f64)> {
    let l = s[..at].rfind("let f")?;
    let mid = &s[l + "let f".len()..at];
    if mid.is_empty() || !mid.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let body = s[at + "_default".len()..].trim_start().strip_prefix('=')?;
    Some((mid.parse::<u32>().ok()?, number_after(body)?))
}

fn extract_defaults(pm: &str) -> Vec<(u32, f64)> {
    let code = strip_line_comments(pm);
    let mut out: Vec<(u32, f64)> = Vec::new();
    let mut idx = 0usize;
    while let Some(off) = code[idx..].find("_default") {
        let at = idx + off;
        if let Some(h) = match_default_decl(&code, at) {
            out.push(h);
        }
        idx = at + 1;
    }
    out.sort_by_key(|(k, _)| *k);
    out.dedup_by_key(|(k, _)| *k);
    out
}

/// 契约表一行（`portfolio-mgr.rhai:216-229` 的注释表格）。
struct ContractRow {
    f: u32,
    name: String,
    source: String,
}

fn extract_contract_table(pm: &str) -> Vec<ContractRow> {
    let mut out = Vec::new();
    for line in pm.lines() {
        if !line.contains('│') {
            continue;
        }
        let cells: Vec<&str> = line.split('│').map(str::trim).collect();
        if cells.len() < 5 {
            continue;
        }
        let mut it = cells[1].split_whitespace();
        let (Some(fs), Some(name)) = (it.next(), it.next()) else {
            continue;
        };
        let Some(digits) = fs.strip_prefix('f') else {
            continue;
        };
        let Ok(f) = digits.parse::<u32>() else {
            continue;
        };
        out.push(ContractRow { f, name: name.to_string(), source: cells[2].to_string() });
    }
    out
}

/// 显式豁免槽位（每条必须带理由）。命中即从 problems 移到 notes 留痕。
///
/// ⚠ **当前为空**（2026-09-20 按裁决清空）—— 空 **≠** 门禁形同虚设：D 规则仍在逐行核对
///   契约表「声明 ↔ 实现」，且负对照测试逐条证明每条规则真的会告警。
///
/// ★ 裁决记录（原登记理由是「无法判定哪侧权威」，取证后**被推翻**）：
///   契约表 `portfolio-mgr.rhai:227` 原写「回测 IC 权重（fallback 0.08）」，代码 `:1037`
///   是 `let f12_default = 0.10;`。三处独立证据 + 一条机制论证一致指向 **0.10 才是真值**：
///     · `:1037` `f12_default`；`:1357` `max_weight` 第 11 项（**硬编码字面量，不引用变量**）；
///       `:1339` 注释「v19: + f12=0.10 → 1.49」（复算该行 11 项和 = 1.49 ✓）
///     · `momentum` 不在 regime-weights 的 `factor_names` 内，且 `factor_weights` 的来源
///       `factor_backtest.factors` 是**占位空 map**（`astock-data/src/mcp_tools.rs:1254`
///       「因子回测引擎未实现」）⇒ `get_weight` 在**两条路径下都**回落 `f12_default`
///       ⇒ **0.10 是 f12 权重的唯一实际取值来源**（历史全部运行皆用 0.10）。
///   ⇒ 改**契约表**（注释）即闭口，**零运行时影响**。原注释那句「改任一方向都会改变
///     posterior 权重」是**错的** —— 只有改**代码**才动 posterior；且 `max_weight` 是手抄
///     字面量，只改 `:1037` 一处会让分子/分母失配（比例 0.9866 ⇒ 系统性偏低 1.34%），
///     正是 `:1344-1350` 警告的 `weights_collapsed` 边界误触发形态（同 V66）。
///   ⇒ 本表已清空（`portfolio-mgr.rhai:227` 已改为 0.10）。**机制保留**：未来若再命中无法
///     即时裁决的不一致，仍需此槽位；配套断言已改为「非空才要求留痕」。
const SEMANTICS_EXEMPT: &[(&str, &str)] = &[];

/// 把命中豁免的问题移入 notes（**必须留痕**：静默吞掉等于删判据）。
fn apply_exemptions(problems: Vec<String>, notes: &mut Vec<String>) -> Vec<String> {
    let mut kept = Vec::new();
    for p in problems {
        match SEMANTICS_EXEMPT.iter().find(|(p0, _)| p.starts_with(p0)) {
            Some((_, why)) => notes.push(format!("已豁免【{why}】: {p}")),
            None => kept.push(p),
        }
    }
    kept
}

/// 语义门禁的判定结果。`n_*` 是**前提自证**用的规模，防止「抽取器全空 ⇒ 空集相等 ⇒ 假绿」。
struct SemanticsReport {
    problems: Vec<String>,
    notes: Vec<String>,
    n_factors: usize,
    n_names: usize,
    n_consumed: usize,
    n_rows: usize,
}

/// 对 `regime-weights.rhai`（产出端）+ `portfolio-mgr.rhai`（消费端/声明）跑全部语义判据。
/// 全部抽取器都是纯函数，便于负对照注入合成片段。
fn judge_rhai_weights(rw: &str, pm: &str) -> SemanticsReport {
    let mut problems: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    let factors = extract_factors(rw);
    let factor_names = extract_str_array(rw, "factor_names");
    let consumed = extract_str_array(rw, "consumed_names");
    let regime_keys = extract_regime_keys(rw);
    let (lit_keys, uses_regime_var) = extract_loop_required_keys(rw);
    let weights = extract_pm_weights(pm);
    let defaults = extract_defaults(pm);
    let rows = extract_contract_table(pm);

    // ── 前提自证：抽取器失效时不得沉默通过 ──
    if factors.is_empty()
        || factor_names.is_empty()
        || consumed.is_empty()
        || regime_keys.is_empty()
    {
        problems.push(format!(
            "前提自证：抽取器失效（factors={} factor_names={} consumed_names={} regime_keys={}）\
             ⇒ 空集相等不得记为通过",
            factors.len(),
            factor_names.len(),
            consumed.len(),
            regime_keys.len()
        ));
        return SemanticsReport {
            problems,
            notes,
            n_factors: factors.len(),
            n_names: factor_names.len(),
            n_consumed: consumed.len(),
            n_rows: rows.len(),
        };
    }
    if factors.len() < 5 {
        problems.push(format!("前提自证：factors 只抽到 {} 条，扫描器可能已失效", factors.len()));
    }
    if rows.len() < 5 {
        problems.push(format!("前提自证：契约表只抽到 {} 行，表格解析可能已失效", rows.len()));
    }
    if lit_keys.is_empty() {
        problems.push("前提自证：抽不到 `f[\"…\"]` 键 ⇒ 值形状判据退化成只查市况档位".to_string());
    }

    // ── A 值形状 ──
    let mut need = lit_keys.clone();
    if uses_regime_var {
        for k in &regime_keys {
            if !need.contains(k) {
                need.push(k.clone());
            }
        }
    }
    for (name, kv) in &factors {
        let missing: Vec<&String> =
            need.iter().filter(|k| !kv.iter().any(|(kk, _)| kk == *k)).collect();
        if !missing.is_empty() {
            problems.push(format!(
                "A factors[\"{name}\"] 缺键 {missing:?} ⇒ f[regime_key] 得 ()，base*() 在该档位出错\
                 （语法门禁看不见，只在该市况下触发）"
            ));
        }
    }

    // ── B 清单齐全（双向） ──
    let ghosts: Vec<&String> =
        factor_names.iter().filter(|n| !factors.iter().any(|(k, _)| k == *n)).collect();
    if !ghosts.is_empty() {
        problems.push(format!(
            "B factor_names 列了 factors 里不存在的名字 {ghosts:?} ⇒ factors[name] 得 ()"
        ));
    }
    let unlisted: Vec<&String> =
        factors.iter().map(|(k, _)| k).filter(|k| !factor_names.contains(*k)).collect();
    if !unlisted.is_empty() {
        problems.push(format!(
            "B' factors 有未被 factor_names 列出的条目 {unlisted:?} ⇒ 循环只遍历 factor_names，这些条目永不产出（死数据）"
        ));
    }

    // ── C 白名单可达（★ 现有跨文件门禁对此不可见） ──
    let orphan: Vec<&String> = consumed.iter().filter(|n| !factor_names.contains(*n)).collect();
    if !orphan.is_empty() {
        problems.push(format!(
            "C consumed_names 含 factor_names 里不存在的名字 {orphan:?} ⇒ 循环永不产出该键、\
             get_weight 恒取 fallback（白名单空接）"
        ));
    }

    // ── D 契约表声明 ↔ 代码实现 ──
    for row in &rows {
        let Some((_, w)) = weights.iter().find(|(k, _)| *k == row.f) else {
            problems.push(format!(
                "D f{} {}: 契约表有声明，但代码里找不到 f{}_weight",
                row.f, row.name, row.f
            ));
            continue;
        };
        let def = defaults.iter().find(|(k, _)| *k == row.f).map(|(_, v)| *v);

        if let Some(want) = row.source.strip_prefix("regime_factor_weights.") {
            if want != row.name {
                problems.push(format!(
                    "D f{} {}: 表里因子名与来源名不一致（regime_factor_weights.{want}）",
                    row.f, row.name
                ));
            }
            if !w.factors.iter().any(|x| x == want) {
                problems.push(format!(
                    "D f{} {}: 表声明走 regime_factor_weights.{want}，但代码**未**调 get_weight(\"{want}\")\
                     （实为 {:?}）⇒ 恒取 fallback",
                    row.f, row.name, w.raw
                ));
            }
            if !consumed.iter().any(|x| x == want) {
                problems.push(format!(
                    "D f{} {}: 表声明走 regime，但 consumed_names 未列出 \"{want}\" ⇒ 产出端不给条目",
                    row.f, row.name
                ));
            }
        } else if row.source.contains("回测") && row.source.contains("IC") {
            // ★ 正确含义：**必须**调 get_weight，且**故意**靠 fallback ⇒
            //   （i）该名不在 consumed_names；（ii）默认值 == 表声明值。
            if !w.factors.iter().any(|x| x == &row.name) {
                problems.push(format!(
                    "D f{} {}: 表声明「回测 IC 权重」应调 get_weight(\"{}\") 并由 fallback 兜底，但代码未调",
                    row.f, row.name, row.name
                ));
            }
            if consumed.iter().any(|x| x == &row.name) {
                problems.push(format!(
                    "D f{} {}: 表声明「回测 IC 权重」，但 consumed_names 含它 ⇒ 产出端会提供条目、\
                     不再是 fallback（表与代码口径冲突）",
                    row.f, row.name
                ));
            }
            let fb = row.source.find("fallback").and_then(|i| number_after(&row.source[i..]));
            if let Some(want) = fb {
                match def {
                    None => problems.push(format!(
                        "D f{} {}: 表声明 fallback {want}，但代码无 f{}_default",
                        row.f, row.name, row.f
                    )),
                    Some(v) if (v - want).abs() > 1e-9 => problems.push(format!(
                        "D f{} {}: 表声明 fallback {want}，代码 f{}_default={v}",
                        row.f, row.name, row.f
                    )),
                    Some(_) => {},
                }
            }
        } else if let Some(i) = row.source.find("硬编码") {
            let want = number_after(&row.source[i..]).unwrap_or(f64::NAN);
            if !w.factors.is_empty() {
                problems.push(format!(
                    "D f{} {}: 表声明「硬编码 {want}」（不消费 regime），但代码调了 get_weight({:?})",
                    row.f, row.name, w.factors
                ));
            } else {
                let got = def.or(if w.literals.len() == 1 {
                    Some(w.literals[0])
                } else {
                    None
                });
                match got {
                    None => notes.push(format!(
                        "D f{} {}: 表声明硬编码 {want}，代码侧取不到单一常量（raw={:?}）",
                        row.f, row.name, w.raw
                    )),
                    Some(v) if (v - want).abs() > 1e-9 => problems.push(format!(
                        "D f{} {}: 表声明硬编码 {want}，代码实际 {v}",
                        row.f, row.name
                    )),
                    Some(_) => {},
                }
            }
        }
    }
    // ── D 反向：代码有 fN_weight 但表无行 ──
    for w in &weights {
        if !rows.iter().any(|r| r.f == w.0) {
            problems.push(format!(
                "D f{}: 代码有 f{}_weight，但契约表无对应行 ⇒ 声明缺失，无处核对",
                w.0, w.0
            ));
        }
    }

    let problems = apply_exemptions(problems, &mut notes);
    SemanticsReport {
        problems,
        notes,
        n_factors: factors.len(),
        n_names: factor_names.len(),
        n_consumed: consumed.len(),
        n_rows: rows.len(),
    }
}

/// 正例：`.rhai` 权重语义在当前仓库必须恒绿（含豁免留痕）。
#[test]
fn rhai_weights_semantics_consistent() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    let rw = include_str!("../../../src/commands/regime-weights.rhai");
    let r = judge_rhai_weights(rw, pm);

    // 规模下界 —— 抽取器全空时「没有问题」是假绿，必须先证扫描面确实张开了
    assert!(r.n_factors >= 5, "factors 抽取器失效（只抽到 {}）", r.n_factors);
    assert!(r.n_names >= 5, "factor_names 抽取器失效（只抽到 {}）", r.n_names);
    assert!(r.n_consumed >= 3, "consumed_names 抽取器失效（只抽到 {}）", r.n_consumed);
    assert!(r.n_rows >= 5, "契约表解析失效（只抽到 {} 行）", r.n_rows);
    assert!(
        r.problems.is_empty(),
        "`.rhai` 权重语义不一致（{} 条）:\n{}",
        r.problems.len(),
        r.problems.join("\n")
    );
    // 豁免表**非空** ⇒ 必须真的在 notes 里留痕（防「豁免静默吞掉判据」）。
    // ⚠ 不用「非空」硬断言：表为空是**合法终局**（2026-09-20 f12 裁决后即此态），
    //   硬断言会让门禁在清空豁免的那一刻自己变红 ⇒ 反过来诱导人「为了过门禁保留豁免」。
    if !SEMANTICS_EXEMPT.is_empty() {
        assert!(
            r.notes.iter().any(|n| n.contains("已豁免")),
            "豁免留痕缺失：豁免表非空却无 notes: {:?}",
            r.notes
        );
    }
}

/// 负对照：证明每条语义规则**真的会告警**（否则可能只是「抽取器什么都没扫到」）。
#[test]
fn rhai_weights_semantics_gate_discriminates() {
    let pm = include_str!("../../../src/commands/portfolio-mgr.rhai");
    let rw = include_str!("../../../src/commands/regime-weights.rhai");

    // 每个变异都必须**真的改动了输入**：目标字符串一旦漂移，`replace` 会静默不生效
    // ⇒ 负对照退化成空跑。本判据的 Node 原型就在此处踩过一次：`factor_names` 与
    //   `consumed_names` 都以 `"chip", "bottleneck"]` 结尾，变异打到了前一个数组上，
    //   于是 C 规则从未被检验（B 替它红了）。
    let check = |label: &str, mrw: &str, mpm: &str, prefix: &str| {
        assert!(
            mrw != rw || mpm != pm,
            "负对照「{label}」变异未生效 ⇒ 目标字符串已漂移，必须修本测试"
        );
        let r = judge_rhai_weights(mrw, mpm);
        assert!(
            r.problems.iter().any(|p| p.starts_with(prefix)),
            "负对照「{label}」未触发「{prefix}」⇒ 该规则永不告警: {:?}",
            r.problems
        );
    };

    // ① A：条目少一个市况档 ⇒ f[regime_key] 得 ()，且只在该档位炸
    let m = rw.replace(
        r#""bottleneck": #{ "base": 0.10, "bull": 0.9, "bear": 1.2, "neutral": 1.0, "volatile": 1.3 }"#,
        r#""bottleneck": #{ "base": 0.10, "bull": 0.9, "bear": 1.2, "neutral": 1.0 }"#,
    );
    check("A 去 volatile 档", &m, pm, "A ");

    // ② B：factor_names 列了 factors 里不存在的名字
    let m =
        rw.replace(r#"let factor_names = ["trend","#, r#"let factor_names = ["ghost_x", "trend","#);
    check("B factor_names 幽灵名", &m, pm, "B ");

    // ③ B'：factors 有未被 factor_names 列出的条目
    let m = rw.replace(
        r#""trend": #{ "base": 0.15"#,
        r#""orphan_entry": #{ "base": 0.15, "bull": 1.0, "bear": 1.0, "neutral": 1.0, "volatile": 1.0 },
    "trend": #{ "base": 0.15"#,
    );
    check("B' factors 未列出条目", &m, pm, "B' ");

    // ④ C：consumed_names 含 factor_names 里不存在的名字（现有跨文件门禁看不见）
    let m = rw.replace(
        r#"let consumed_names = ["trend""#,
        r#"let consumed_names = ["not_in_factor_names", "trend""#,
    );
    check("C consumed_names 幽灵名", &m, pm, "C ");

    // ⑤ D：pm 把 get_weight 的因子名写错 ⇒ 消费端恒取 fallback
    let m = pm.replace(
        r#"get_weight(f_weights, "bottleneck", f13_default)"#,
        r#"get_weight(f_weights, "typo_factor", f13_default)"#,
    );
    check("D get_weight 因子名错", rw, &m, "D f13 ");

    // ⑥ D：契约表声明值与代码不符
    let m = pm.replace("│ f4  risk       │ 硬编码 0.15", "│ f4  risk       │ 硬编码 0.18");
    check("D 表值≠代码值", rw, &m, "D f4 ");

    // ⑦ D：表里 f13 改回「硬编码」，但代码仍走 regime ⇒ 表与代码口径冲突
    let m = pm.replace(
        "│ f13 bottleneck │ regime_factor_weights.bottleneck     │",
        "│ f13 bottleneck │ 硬编码 0.10                          │",
    );
    check("D 表口径与代码冲突", rw, &m, "D f13 ");

    // ⑧ D：代码新增 fN_weight，但契约表无行
    let m = pm.replace(
        "let f13_default = 0.10;",
        "let f13_default = 0.10;\nlet f14_weight = if present(x) { get_weight(f_weights, \"valuation\", 0.05) } else { 0.0 };",
    );
    check("D 表缺行", rw, &m, "D f14");
}

/// 防回归：`portfolio-risk-gate.rhai` 覆盖 `action` / `positionPct` 时，
/// 必须同步重算由它们派生的三个面。
///
/// 背景（2026-09-21）：风控门此前只改 `action` 与 `positionPct`，而
/// `positionState` / `reasoning` 结论名 / `stopLossPct`·`takeProfitPct`
/// 仍留在 portfolio-mgr 的原值上 ⇒ **同一条落库记录**内部互相矛盾。
/// 最隐蔽的是 `positionState`：前端 `resolveDisplayAction` **以 state 优先**，
/// 会把已被改成「观望」的档又派生回「持有」——展示层补丁兜不住这个。
///
/// 判据形态：文本级 + **变异检验**（逐条摘掉/改名后判据必须报错），
/// 与本节 `rhai_weights_semantics_gate_discriminates` 同一策略 ——
/// 只断言「存在某行」容易被无关改动蒙对，变异检验才证明判据有区分力。
/// 行为级测试需要 host 函数 `pm_portfolio_risk_gate`（注册在 `axagent` 主 crate，
/// 与 rt-workflow 无依赖关系）加真库，依赖方向与成本都不允许。
#[test]
fn portfolio_risk_gate_syncs_derived_fields() {
    let code = include_str!("../../../src/commands/portfolio-risk-gate.rhai");

    // 返回「未满足」的条目名；空 ⇒ 全部满足
    fn unsatisfied(code: &str) -> Vec<&'static str> {
        // 只判**代码行**：注释里必然出现这些字段名，误当代码会得到假绿
        let only: String = code
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut miss = Vec::new();
        // ① 对齐函数必须真的定义（否则 ③ 的调用只是个不存在的名字）
        if !only.contains("fn align_decision_label(") {
            miss.push("align_decision_label 未定义");
        }
        // ② 覆盖 positionPct 时同步重算状态轴
        if !only.contains("gate_result[\"positionState\"] = ") {
            miss.push("未同步 positionState");
        }
        // ③ reasoning 结论名必须经对齐函数改写（不得原样透传 pm 的结论头）
        if !only.contains("gate_result[\"reasoning\"] = align_decision_label(") {
            miss.push("未对齐 reasoning 结论名");
        }
        // ④ 仓位归零 ⇒ 止损 / 止盈档一并归零（pm 的判据是 `position_pct > 0.0`）
        if !only.contains("gate_result[\"stopLossPct\"] = 0.0") {
            miss.push("未归零 stopLossPct");
        }
        if !only.contains("gate_result[\"takeProfitPct\"] = 0.0") {
            miss.push("未归零 takeProfitPct");
        }
        miss
    }

    let base = unsatisfied(code);
    assert!(base.is_empty(), "portfolio-risk-gate.rhai 派生面未同步: {base:?}");

    // 变异检验：每条判据都要能被对应的破坏检出，否则它是空转的
    let mutants: [(&str, &str, &str); 5] = [
        ("① 对齐函数改名", "fn align_decision_label(", "fn renamed_align("),
        ("② 状态轴赋值摘除", "gate_result[\"positionState\"] = gate_position_state;", ""),
        (
            "③ 结论名未走对齐",
            "gate_result[\"reasoning\"] = align_decision_label(",
            "gate_result[\"reasoning\"] = (",
        ),
        ("④ 止损档未归零", "gate_result[\"stopLossPct\"] = 0.0;", ""),
        ("⑤ 止盈档未归零", "gate_result[\"takeProfitPct\"] = 0.0;", ""),
    ];
    for (name, from, to) in mutants {
        assert!(code.contains(from), "变异点未命中 —— 判据与被测对象已脱节: {name}");
        let mutated = code.replace(from, to);
        assert!(
            !unsatisfied(&mutated).is_empty(),
            "变异「{name}」未被判据检出 —— 该条判据没有区分力"
        );
    }
}
