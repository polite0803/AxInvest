// SPDX-License-Identifier: AGPL-3.0-only

//! AxInvest 业务 Rhai 宿主函数集的**唯一定义点**。
//!
//! ## 背景（2026-09-22 修复）
//!
//! `pm_*`（`rhai_pm`）与 `bottleneck_*`（`rhai_bottleneck`）两组函数集此前被拆成
//! 两处独立注册，而共享 Engine 的注册通道当时是**单槽** `OnceLock`（容量 1）
//! ⇒ 第二次注册被静默丢弃，`bottleneck_node_score` 从未进入共享 Engine。
//! （该通道已改为**可增长队列 + 冻结即硬失败**，见
//! `rt-workflow::work_engine::executors::code_executor` 的
//! `register_shared_engine_initializer`；单点注册则是本模块的职责。）
//!
//! 同一根因还有第二个面向：本仓存在**多个自建 Rhai Engine 的入口**，各自手工
//! 注册函数集，历史上出现三种不一致 ——
//!
//! | 入口 | 历史函数集 |
//! |---|---|
//! | 共享 Engine（DAG 主路径，`rt-workflow::code_executor`） | common + 回调（单槽 ⇒ 实际只有 pm_*） |
//! | rerun 决策（`stock_workflow/decision.rs`） | common + pm_*（缺 bottleneck_*） |
//! | What-If 回测（`commands/stock_analysis.rs`） | **仅 common**（pm_* / bottleneck_* 全缺） |
//! | 动态工具引擎（`crates/tools/src/rhai_engine.rs`） | 仅 `band_for_score`（缺 common / pm_* / bottleneck_*） |
//!
//! 三处都执行 `portfolio-mgr.rhai`，缺失的宿主函数会被脚本内 `try/catch` 吞掉，
//! 症状是**静默走保守兜底**（action="观望"、confidence=0），不是报错。
//!
//! 本模块把「AxInvest 需要哪些宿主函数」收敛成**一个**函数，消除两类隐患：
//! ① 新增函数时漏改某个注册点；② 共享 Engine 与本地自建 Engine 的函数集漂移。
//!
//! ## 消费方
//!
//! - 共享 Engine（DAG 主路径）：`src/init/services.rs` 将其作为初始化回调注册；
//! - 本地自建 Engine（What-If / rerun）：[`build_stock_rhai_engine`]。
//!
//! ⚠ 新增任何 `pm_*` / `bottleneck_*` 宿主函数时**只改本文件**；
//! 脚本调用点与已注册函数的覆盖关系由本模块末尾的测试守住。

use rhai::Engine;

use super::{rhai_bottleneck, rhai_pm};

/// 把 AxInvest 业务脚本依赖的全部宿主函数注册到指定 Engine。
///
/// 纯注册、无副作用；沙箱限制由调用方负责（见 [`build_stock_rhai_engine`]）。
pub fn register_axinvest_rhai_functions(engine: &mut Engine) {
    rhai_pm::register_pm_functions(engine);
    rhai_bottleneck::register_bottleneck_functions(engine);
    // 领域本体查询（`band_for_score`）。权威注册入口在 harness；
    // 历史上**只有** `crates/tools` 的动态工具引擎注册了它，共享 Engine 没有
    // ⇒ `strategy-scorer.rhai:68` / `bottleneck-calc.rhai:148` 一执行就 Function not found。
    axagent_harness::register_ontology_functions(engine);
}

/// Rhai 沙箱档位：操作数 / 调用深度 / 表达式嵌套上限。
///
/// 历史三处自建 Engine 档位不一致（共享 Engine `1024/1024`、What-If `256/256`），
/// 收敛为具名档位，避免「同一脚本在不同入口能跑 / 不能跑」这类环境差异。
/// 本次只留下 `PORTFOLIO` 一档 —— 原先为 DAG 主路径另设的常量已删（本地侧无消费方，
/// 留着重名反而会让人以为共享 Engine 的档位也归本模块管）。
#[derive(Debug, Clone, Copy)]
pub struct RhaiSandboxLimits {
    /// 单次执行的最大操作数（防死循环）
    pub max_operations: u64,
    /// 最大函数调用层数（防递归爆栈）
    pub max_call_levels: usize,
    /// 最大表达式嵌套深度
    pub max_expr_depths: usize,
}

impl RhaiSandboxLimits {
    /// `portfolio-mgr.rhai` 规模档位（本地自建 Engine 当前唯一使用的档位）。
    ///
    /// 该脚本因子多、表达式嵌套深（实测 line 518 处超默认上限，下限 48），
    /// 故取 256（约 5 倍余量）；执行总操作数仍受 `max_operations` 约束。
    ///
    /// 注：**共享 Engine（DAG 主路径）的档位不在此处** —— 它由 `rt-workflow`
    /// 自己维护（`code_executor.rs` 里的 `1024/1024`），本 crate 不能反向依赖它。
    /// 两侧档位有差异不影响函数集（`set_max_*` 与 `register_fn` 互不相干）。
    pub const PORTFOLIO: Self =
        Self { max_operations: 200_000, max_call_levels: 32, max_expr_depths: 256 };
}

/// 构造一个「函数集与共享 Engine 一致」的独立 Rhai Engine。
///
/// 用于无法走共享 Engine 的入口（What-If 回测、rerun 决策）。这些入口历史上
/// 各自手工注册函数集：rerun 决策只有 `common + pm_*`（漏 `bottleneck_*`），
/// What-If 回测**只有 `common`**（两组全漏）—— 脚本内 `Function not found` 被
/// `try/catch` 吞掉后静默走保守兜底路径，表现为「怎么调参数结果都不变」，不是报错。
pub fn build_stock_rhai_engine(limits: RhaiSandboxLimits) -> Engine {
    let mut engine = Engine::new();
    // SECURITY (C4): Rhai 沙箱限制 — 防 DoS
    engine.set_max_operations(limits.max_operations);
    engine.set_max_call_levels(limits.max_call_levels);
    engine.set_max_modules(0);
    engine.set_max_string_size(2_000_000);
    engine.set_max_array_size(50_000);
    engine.set_max_expr_depths(limits.max_expr_depths, limits.max_expr_depths);
    axagent_harness::rhai_engine::register_common_functions(&mut engine);
    register_axinvest_rhai_functions(&mut engine);
    engine
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 生产 Rhai 脚本清单（`include_str!` 编译期嵌入，路径相对本文件）。
    ///
    /// 孤儿脚本（无 seed / 无 DB 节点，如 `bottleneck-calc.rhai`）同样纳入：
    /// 它们仍会被测试或手工路径加载，函数集必须同样完整。
    const PRODUCTION_SCRIPTS: [(&str, &str); 17] = [
        ("analyst-brief", include_str!("../analyst-brief.rhai")),
        ("bottleneck-calc", include_str!("../bottleneck-calc.rhai")),
        ("consistency-check", include_str!("../consistency-check.rhai")),
        ("data-quality", include_str!("../data-quality.rhai")),
        ("data-verifier", include_str!("../data-verifier.rhai")),
        ("pace-calc", include_str!("../pace-calc.rhai")),
        ("portfolio-mgr", include_str!("../portfolio-mgr.rhai")),
        ("portfolio-risk-gate", include_str!("../portfolio-risk-gate.rhai")),
        ("raw-digest", include_str!("../raw-digest.rhai")),
        ("reflection-comparator", include_str!("../reflection-comparator.rhai")),
        ("reflection_validator", include_str!("../reflection_validator.rhai")),
        ("regime-weights", include_str!("../regime-weights.rhai")),
        ("risk-level", include_str!("../risk-level.rhai")),
        ("sim-verify", include_str!("../sim-verify.rhai")),
        ("strategy-scorer", include_str!("../strategy-scorer.rhai")),
        ("trader-proxy", include_str!("../trader-proxy.rhai")),
        ("demand-keywords", include_str!("../opc_setup/demand-keywords.rhai")),
    ];

    /// 宿主函数前缀。
    ///
    /// 这 3 个前缀在 Rhai 标准包里不存在，因此可以**零假阳性**地把「宿主注入的
    /// 函数」与「语言内建 / 脚本自带」区分开 —— 后者随 Rhai 版本变化，靠穷举
    /// 内建清单做差集必然误报。
    ///
    /// ⚠ 但**前缀不是判据的全部**：`band_for_score` 是既有反例 —— 它不带上述任何
    /// 前缀、只注册在 `crates/tools/src/rhai_engine.rs`，于是长期逃过人工核对，
    /// 直到本次（2026-09-22）才发现共享 Engine 缺它。故判据 = 前缀 ∪ 注册点声明。
    const HOST_FN_PREFIXES: [&str; 3] = ["pm_", "bottleneck_", "sim_"];

    /// **AxInvest 业务脚本的函数集注册点**（`include_str!`，路径相对本文件）。
    ///
    /// 判据：脚本调用到的宿主函数**必须**出现在这些源码的 `register_fn("name"...)` 里
    /// —— 这样新增非前缀命名的宿主函数时无需手改白名单。
    ///
    /// ⚠ 新增「服务 AxInvest 脚本」的注册点时**必须加到本清单**，否则又成盲区。
    const AXINVEST_REGISTRATION_SOURCES: [(&str, &str); 3] = [
        ("harness::rhai_engine", include_str!("../../../crates/harness/src/rhai_engine.rs")),
        ("rhai_pm", include_str!("rhai_pm.rs")),
        ("rhai_bottleneck", include_str!("rhai_bottleneck.rs")),
    ];

    /// **其它域的**宿主函数注册点。
    ///
    /// 只用来「识别某个名字确实是宿主函数」（从而与 Rhai 语言内建区分开），
    /// **不**算作 AxInvest 函数集的可满足来源。
    ///
    /// 之所以要让它们参与识别：`band_for_score` 长期**只**存在于
    /// `tools::rhai_engine` —— 这一格（「识别为宿主函数，但不在 AxInvest 注册点里」）
    /// 正是修复前缺陷的本体形态，判据必须能落在它上面。
    const OTHER_REGISTRATION_SOURCES: [(&str, &str); 3] = [
        ("tools::rhai_engine", include_str!("../../../crates/tools/src/rhai_engine.rs")),
        ("quant::script", include_str!("../../../crates/quant/src/script.rs")),
        (
            "rt-workflow::engine",
            include_str!("../../../crates/rt-workflow/src/work_engine/engine/mod.rs"),
        ),
    ];

    /// 从注册点源码抽取 `register_fn` 注册的函数名。
    ///
    /// 先剥注释：**被注释掉的注册行不算声明** —— 否则「把注册行注释掉」会让
    /// 纯文本判据假绿，这是它最容易失守的一格。
    fn declared_fn_names(sources: [(&str, &str); 3]) -> HashSet<String> {
        let mut out = HashSet::new();
        for (_label, raw) in sources {
            let src = strip_comments_only(raw);
            let bytes = src.as_bytes();
            let needle = b"register_fn";
            let mut i = 0;
            while i + needle.len() <= bytes.len() {
                if &bytes[i..i + needle.len()] != needle {
                    i += 1;
                    continue;
                }
                // 名字是 `register_fn(` 后的第一个字符串字面量。限制搜索窗口，
                // 避免「无字符串字面量参数的 register_fn」一路吃掉下一个调用的名字。
                let limit = (i + needle.len() + 200).min(bytes.len());
                let mut j = i + needle.len();
                while j < limit && bytes[j] != b'"' {
                    j += 1;
                }
                if j >= limit {
                    i += 1;
                    continue;
                }
                let start = j + 1;
                let mut k = start;
                while k < bytes.len() && bytes[k] != b'"' {
                    k += 1;
                }
                if k >= bytes.len() {
                    i += 1;
                    continue;
                }
                let name = &src[start..k];
                if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    out.insert(name.to_string());
                }
                i = k;
            }
        }
        out
    }

    fn axinvest_host_fn_names() -> HashSet<String> {
        declared_fn_names(AXINVEST_REGISTRATION_SOURCES)
    }

    fn other_host_fn_names() -> HashSet<String> {
        declared_fn_names(OTHER_REGISTRATION_SOURCES)
    }

    /// 剥除注释与字符串字面量。
    ///
    /// 必要性（本项目已有先例）：注释里常写「本节点会调用 `pm_xxx()`」这类说明，
    /// 若不剥除会被当成真实调用，让门禁产生假红；反之字符串里的内容也不是调用。
    fn strip_comments_and_strings(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        let mut chars = src.chars().peekable();
        let mut in_str = false;
        let mut in_line_comment = false;
        let mut in_block_comment = false;
        while let Some(c) = chars.next() {
            if in_line_comment {
                if c == '\n' {
                    in_line_comment = false;
                    out.push('\n');
                }
                continue;
            }
            if in_block_comment {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    in_block_comment = false;
                }
                continue;
            }
            if in_str {
                if c == '\\' {
                    chars.next();
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            if c == '/' {
                match chars.peek() {
                    Some('/') => {
                        chars.next();
                        in_line_comment = true;
                        continue;
                    },
                    Some('*') => {
                        chars.next();
                        in_block_comment = true;
                        continue;
                    },
                    _ => {},
                }
            }
            if c == '"' {
                in_str = true;
                continue;
            }
            out.push(c);
        }
        out
    }

    /// 只剥注释、**保留字符串字面量**。
    ///
    /// 与 `strip_comments_and_strings` 的分工：后者用于**脚本**侧（那里的字符串
    /// 内容不是调用），本函数用于**注册点源码** —— `register_fn("name", ...)` 的
    /// 名字**就在字符串里**，一并剥掉会让判据彻底失效。
    fn strip_comments_only(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        let mut chars = src.chars().peekable();
        let mut in_str = false;
        let mut in_line_comment = false;
        let mut in_block_comment = false;
        while let Some(c) = chars.next() {
            if in_line_comment {
                if c == '\n' {
                    in_line_comment = false;
                    out.push('\n');
                }
                continue;
            }
            if in_block_comment {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    in_block_comment = false;
                }
                continue;
            }
            if in_str {
                out.push(c);
                if c == '\\' {
                    if let Some(escaped) = chars.next() {
                        out.push(escaped);
                    }
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            if c == '/' {
                match chars.peek() {
                    Some('/') => {
                        chars.next();
                        in_line_comment = true;
                        continue;
                    },
                    Some('*') => {
                        chars.next();
                        in_block_comment = true;
                        continue;
                    },
                    _ => {},
                }
            }
            if c == '"' {
                in_str = true;
            }
            out.push(c);
        }
        out
    }

    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    /// 提取所有「标识符后紧跟 `(`」的名字（即函数/方法调用形态）。
    fn called_identifiers(src: &str) -> HashSet<String> {
        let b = src.as_bytes();
        let mut out = HashSet::new();
        let mut i = 0;
        while i < b.len() {
            if !b[i].is_ascii_alphabetic() {
                i += 1;
                continue;
            }
            let start = i;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            let mut j = i;
            while j < b.len() && (b[j] as char).is_ascii_whitespace() {
                j += 1;
            }
            if j < b.len() && b[j] == b'(' {
                out.insert(src[start..i].to_string());
            }
        }
        out
    }

    /// 提取脚本内 `fn` 定义的名字。
    ///
    /// 必须排除它们：`portfolio-mgr.rhai` 自定义了 `pm_avg_turnover` / `pm_saturate`
    /// 两个同前缀函数，若当成宿主调用会误判为「未注册」。
    fn local_fn_names(src: &str) -> HashSet<String> {
        let b = src.as_bytes();
        let mut out = HashSet::new();
        let mut i = 0;
        while i + 2 < b.len() {
            // `fn` 前不得是标识符字符，后必须紧跟空白 —— 排除 `fnord` 之类误匹配
            let boundary = i == 0 || !is_ident_byte(b[i - 1]);
            let followed_by_ws = (b[i + 2] as char).is_ascii_whitespace();
            if boundary && &b[i..i + 2] == b"fn" && followed_by_ws {
                let mut j = i + 2;
                while j < b.len() && (b[j] as char).is_ascii_whitespace() {
                    j += 1;
                }
                let start = j;
                while j < b.len() && is_ident_byte(b[j]) {
                    j += 1;
                }
                if j > start {
                    out.insert(src[start..j].to_string());
                }
                i = j;
                continue;
            }
            i += 1;
        }
        out
    }

    /// 门禁本体：返回「脚本调用到了、却不在 AxInvest 函数集注册点里」的宿主函数。
    ///
    /// 为什么不直接问 Engine「有没有这个函数」：Rhai 的
    /// `Engine::gen_fn_signatures` 挂在 `#[cfg(feature = "metadata")]` 之下，
    /// 而本仓 rhai 只启用 `sync`（`rhai = { version = "1", features = ["sync"] }`）
    /// —— 为一道测试去扩大依赖 feature 不划算。
    /// 改为对**注册点声明**做对账：判据等价（脚本用到 ⇒ 必须在 AxInvest 函数集的
    /// 声明里），零依赖，也不随 rhai 版本变化。
    /// 「注册链真的被执行」由 `band_for_score_resolves_on_every_axinvest_engine`
    /// 用真调用覆盖（写成纯代码引用而非 intra-doc 链接：本模块在 `#[cfg(test)]` 下，
    /// 跨 cfg 的链接解析不值得赌）。
    fn find_unregistered_host_calls(scripts: &[(&str, &str)]) -> Vec<String> {
        let axinvest = axinvest_host_fn_names();
        let other = other_host_fn_names();
        let mut problems = Vec::new();
        for (label, src) in scripts {
            let code = strip_comments_and_strings(src);
            let locals = local_fn_names(&code);
            for name in called_identifiers(&code) {
                if locals.contains(&name) {
                    continue; // 脚本内自定义（如 portfolio-mgr.rhai 的 pm_avg_turnover）
                }
                let is_host_fn = HOST_FN_PREFIXES.iter().any(|p| name.starts_with(p))
                    || axinvest.contains(&name)
                    || other.contains(&name);
                if !is_host_fn {
                    continue; // Rhai 语言内建 / 标准库方法，不在宿主注入范围内
                }
                if !axinvest.contains(&name) {
                    problems.push(format!("{label}: {name}"));
                }
            }
        }
        problems.sort();
        problems.dedup();
        problems
    }

    /// 提取「`let name = |...|` 定义的**闭包**变量名」。
    ///
    /// 只认「`let` + 标识符 + `=` +（可选 `move`）+ `|`」这一形态 —— `let x = a | b;`
    /// （按位或）与 `let c = a || b;`（逻辑或）在 `=` 后**不是** `|`，天然不命中，
    /// 无须枚举例外（判据尽量落在「Rhai 的闭包语法本身」上，而非逐例排除）。
    fn closure_var_names(src: &str) -> HashSet<String> {
        let b = src.as_bytes();
        let mut out = HashSet::new();
        let mut i = 0;
        while i + 3 <= b.len() {
            // `let` 前不得是标识符字符（排除 `outlet` / `inlet` 之类误匹配）
            if (i > 0 && is_ident_byte(b[i - 1])) || &b[i..i + 3] != b"let" {
                i += 1;
                continue;
            }
            let mut j = i + 3;
            if j >= b.len() || !(b[j] as char).is_ascii_whitespace() {
                i += 1;
                continue;
            }
            while j < b.len() && (b[j] as char).is_ascii_whitespace() {
                j += 1;
            }
            let start = j;
            while j < b.len() && is_ident_byte(b[j]) {
                j += 1;
            }
            if j == start {
                i += 1;
                continue;
            }
            let name = src[start..j].to_string();
            while j < b.len() && (b[j] as char).is_ascii_whitespace() {
                j += 1;
            }
            // 必须是**赋值** `=`（排除 `==` 等比较/复合赋值）
            if j + 1 >= b.len() || b[j] != b'=' || b[j + 1] == b'=' {
                i = j.max(i + 1);
                continue;
            }
            j += 1;
            while j < b.len() && (b[j] as char).is_ascii_whitespace() {
                j += 1;
            }
            // 可选的 `move`（Rhai 支持 `let f = move |x| ...`）
            if j + 4 <= b.len()
                && &b[j..j + 4] == b"move"
                && (j + 4 == b.len() || (b[j + 4] as char).is_ascii_whitespace())
            {
                j += 4;
                while j < b.len() && (b[j] as char).is_ascii_whitespace() {
                    j += 1;
                }
            }
            if j < b.len() && b[j] == b'|' {
                out.insert(name);
            }
            i = j.max(i + 1);
        }
        out
    }

    /// 门禁本体：返回「以 `name(...)` **按名调用**的闭包变量」。
    ///
    /// 为什么这是缺陷而非风格问题：Rhai 的按名调用
    /// （`rhai/src/func/call.rs::exec_fn_call`）只查两处 —— ① 本 AST 内的脚本 `fn` 库
    /// （按函数名 hash）② 宿主注册的 native 函数；**没有「查作用域里那个变量是不是
    /// FnPtr」这一步**（Rhai Book「Function Pointers」页亦明写“Call a function pointer
    /// via the `call` method”，且 “function pointers are *not* first-class functions”）。
    /// 故 `let f = |...|; f(x)` 稳抛 `ErrorFunctionNotFound`，且**编译期不报**。
    /// 本仓既有约定见 `strategy-scorer.rhai:9`：「调用统一用 `f.call(...)`」。
    fn find_closures_called_by_name(scripts: &[(&str, &str)]) -> Vec<String> {
        let mut problems = Vec::new();
        for (label, src) in scripts {
            let code = strip_comments_and_strings(src);
            let called = called_identifiers(&code);
            for name in closure_var_names(&code) {
                if called.contains(&name) {
                    problems.push(format!("{label}: {name}"));
                }
            }
        }
        problems.sort();
        problems.dedup();
        problems
    }

    /// 正对照 + 回归：生产脚本用到的每个宿主函数都必须可解析。
    ///
    /// 本测试直接锁住 2026-09-22 那个缺陷：单槽注册通道丢掉 `bottleneck_*` 之后，
    /// `strategy-scorer.rhai` / `bottleneck-calc.rhai` 会报
    /// `Function not found: bottleneck_node_score`，而当时**没有任何测试**能发现
    /// —— 原有的 `rhai_syntax_check` 只 `compile`，Rhai 编译期不校验未知函数名。
    #[test]
    fn every_host_fn_used_by_production_scripts_is_registered() {
        // 正对照：注册点抽取链路本身必须有效（否则空集合会让断言恒真）。
        //
        // 三个名字各有分工，缺一不可：
        // - `bottleneck_node_score`：**跨行** `register_fn(\n  "name", ...)` 形态
        //   （`rhai_bottleneck.rs:236`）—— 锁住 200 字节窗口够用；
        // - `pm_evidence_scale`：单行形态（`rhai_pm.rs:24`）；
        // - `band_for_score`：**不带** `pm_/bottleneck_/sim_` 前缀，只在
        //   `harness` 注册 —— 锁住「前缀判据会漏」，即本次缺陷的本体形态。
        let declared = axinvest_host_fn_names();
        assert!(
            declared.contains("bottleneck_node_score")
                && declared.contains("pm_evidence_scale")
                && declared.contains("band_for_score"),
            "正对照失败：注册点扫描没抽出预期函数名（实际 {} 个）—— \
             include_str! 路径、剥注释或 200 字节窗口逻辑可能已失效，\
             后续断言会失去区分力",
            declared.len()
        );

        let problems = find_unregistered_host_calls(&PRODUCTION_SCRIPTS);
        assert!(
            problems.is_empty(),
            "以下脚本调用点没有对应的宿主函数注册（运行时会报 Function not found）:\n  {}",
            problems.join("\n  ")
        );
    }

    /// 负对照：证明上面的门禁**真的会告警**（0 命中 ≠ 没问题）。
    ///
    /// 判据锚定被测逻辑自身 —— 同一函数 `find_unregistered_host_calls`，
    /// 只把输入从「生产脚本」换成「注入的假脚本」。
    #[test]
    fn gate_reports_unregistered_host_call() {
        let fake = "// 注释里写 pm_commented_out() 不算调用\n\
                    let a = pm_this_host_fn_does_not_exist(1.0);\n\
                    let b = \"pm_inside_string()\";\n";
        let problems = find_unregistered_host_calls(&[("injected", fake)]);
        assert_eq!(
            problems,
            vec!["injected: pm_this_host_fn_does_not_exist".to_string()],
            "门禁没能报出注入的未注册函数（且注释/字符串不得被误判）"
        );
    }

    /// 脚本内自定义的同前缀函数不得被误判为宿主缺失。
    #[test]
    fn script_local_same_prefix_fn_is_not_reported() {
        let script = "fn pm_local_helper(x) { x + 1 }\nlet y = pm_local_helper(1.0);\n";
        let problems = find_unregistered_host_calls(&[("local", script)]);
        assert!(problems.is_empty(), "脚本内自定义 fn 被误判：{problems:?}");
    }

    /// `portfolio-mgr.rhai` 确实自定义了同前缀函数 —— 这个事实本身要留住，
    /// 否则将来有人「顺手」把它注册到宿主侧，会与脚本定义形成两套语义。
    #[test]
    fn portfolio_mgr_keeps_its_local_pm_helpers() {
        let src =
            PRODUCTION_SCRIPTS.iter().find(|(l, _)| *l == "portfolio-mgr").expect("清单缺该脚本").1;
        let locals = local_fn_names(&strip_comments_and_strings(src));
        assert!(
            locals.contains("pm_avg_turnover") && locals.contains("pm_saturate"),
            "portfolio-mgr.rhai 的本地 pm_* helper 结构已变，请复核本测试的排除逻辑。实际：{locals:?}"
        );
    }

    /// 回归：`band_for_score` 曾**只**注册在 `crates/tools` 的动态工具引擎上，
    /// 共享 Engine 与 AxInvest 本地自建 Engine 都缺它 —— 而
    /// `strategy-scorer.rhai:68` / `bottleneck-calc.rhai:148` 调用它。
    ///
    /// 判据用**真调用**而非只看名字：函数名在签名表里但签名不匹配时，
    /// 运行期一样是 `Function not found`，只查名字会漏。
    #[test]
    fn band_for_score_resolves_on_every_axinvest_engine() {
        // 两条构造路径都必须覆盖 —— 它们此前各缺一份注册，正是缺陷 ④ 的形态：
        // ① 裸 `Engine::new()` + 单点注册函数（共享 Engine 走的就是这条）；
        // ② 带沙箱档位的本地工厂（`decision.rs` / `stock_analysis.rs` 走这条）。
        let mut bare = Engine::new();
        register_axinvest_rhai_functions(&mut bare);
        let sandboxed = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);

        for (label, engine) in [("bare", &bare), ("sandboxed", &sandboxed)] {
            let band: String = engine.eval("band_for_score(80.0)").unwrap_or_else(|e| {
                panic!("{label} 路径的 Engine 上 band_for_score 真调用失败：{e}")
            });
            // 80.0 落在 `READINESS_BANDS` 首档 strong_bottleneck（min = 75.0）。
            assert_eq!(band, "strong_bottleneck", "{label} 路径的分档结果与本体不一致");
        }
    }

    /// 执行侧契约：`raw-digest.rhai`（快速链专属）必须真的产出「分维度段」的 map。
    ///
    /// 为什么不能只靠 `all_rhai_scripts_compile`：那只 `compile`，而本脚本的关键行为
    /// （`#{}` 逐键赋值、`for (k, v) in map` 迭代、`sub_string` 裁剪、`try/catch` 兜住
    /// JSON 解析失败）**编译期一律不校验** —— 语法过了，运行时 `Function not found`
    /// 或 `Variable not found` 一样会把整段静默降级成空，且不报错。
    ///
    /// 输入按 engine 的真实行为构造：`code_executor` 会把 `input_mapping` 的**每个键**
    /// 都推进 scope（解析不到时推 `unit`），故这里既有「有数据」也有「留空」的维度。
    #[test]
    fn raw_digest_script_produces_dimension_sections() {
        let src = PRODUCTION_SCRIPTS
            .iter()
            .find(|(l, _)| *l == "raw-digest")
            .expect("清单缺 raw-digest")
            .1;
        let engine = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);
        let ast = engine.compile(src).expect("raw-digest.rhai 应能编译");

        let mut scope = rhai::Scope::new();
        // 有数据的两个维度：列表类（新闻）+ 对象类（算法腿）
        scope.push_constant(
            "news_data".to_string(),
            rhai::Dynamic::from(
                r#"[{"title":"公告A","summary":"摘要","publishTime":"2026-09-20"},
                    {"title":"B","summary":"x","publishTime":"2026-09-19"}]"#
                    .to_string(),
            ),
        );
        scope.push_constant(
            "algo_scoring".to_string(),
            rhai::Dynamic::from(r#"{"totalScore":72.5,"grade":"B"}"#.to_string()),
        );
        // 其余 18 路按「工具失败 / 未接线」注入 unit（engine 的真实降级形态）
        for name in [
            "market_data",
            "sentiment_data",
            "fundamentals_data",
            "policy_data",
            "hotmoney_data",
            "lockup_data",
            "research_data",
            "sector_data",
            "catalyst_data",
            "pledge_data",
            "index_quotes",
            "institutional_visits",
            "dragon_tiger_data",
            "algo_valuation",
            "algo_valuation_band",
            "algo_risk",
            "algo_scoring_week",
            "algo_scoring_month",
        ] {
            scope.push_constant(name.to_string(), rhai::Dynamic::UNIT);
        }

        let out: rhai::Dynamic =
            engine.eval_ast_with_scope(&mut scope, &ast).expect("raw-digest.rhai 应能执行");
        let out = axagent_harness::dynamic_to_json_value(&out);
        let map = out.as_object().expect("脚本应返回 map（下游读 `analyst-brief.result.<段>`）");
        assert_eq!(map.len(), 20, "应恰有 20 个维度段，实际：{:?}", map.keys().collect::<Vec<_>>());

        let news = map["news"].as_str().expect("news 段应是字符串");
        assert!(news.contains("title=公告A"), "列表类段应逐条渲染：{news}");
        assert!(news.contains("2026-09-20"), "列表类段应带时间：{news}");
        let algo = map["algo_scoring"].as_str().expect("algo_scoring 段应是字符串");
        assert!(algo.contains("totalScore=72.5"), "对象类段应扁平化渲染：{algo}");
        assert_eq!(map["policy"].as_str(), Some("（无数据）"), "缺数据的段应显式标注而非留空");
    }

    /// 回归：`portfolio-mgr.rhai` 的阶段1/阶段2（四周期）曾把 5 个闭包**全部按名调用**，
    /// 生产上必抛 `Function not found: sl_pct_for (&str | ImmutableString | String)`
    /// （实报 line 2505），节点被判「执行异常」⇒ **降级为保守决策**（action=观望、
    /// confidence=0）。当时**没有任何门禁**能发现它：
    ///
    /// - `all_rhai_scripts_compile` 只 `compile` —— Rhai 编译期**不校验未知函数名**；
    /// - `rt-workflow/tests/portfolio_mgr_veto_rhai.rs` 只跑**抽取出的片段**，不执行整脚本；
    /// - 本模块既有的 `find_unregistered_host_calls` 只看「宿主函数注册没注册」，
    ///   而 `sl_pct_for` 是**脚本自己的闭包变量**，不在它的判据面上。
    #[test]
    fn every_production_script_calls_closures_via_method_call() {
        // 正对照：扫描器必须真的抽出闭包名，否则下面的空断言恒真。
        // 四个名字各有分工：
        // - `sl_pct_for`：单参 + 一体式 `switch`（本次缺陷本体）；
        // - `horizon_price_of`：4 参 + 多行体（锁住多参形态）；
        // - `sink`：定义在 `fn main()` **内部**（锁住「不只在顶层」）；
        // - `read_weight`：`strategy-scorer.rhai:57` 定义了却从未调用（锁住「定义即入集」）。
        let all: HashSet<String> = PRODUCTION_SCRIPTS
            .iter()
            .flat_map(|(_, src)| closure_var_names(&strip_comments_and_strings(src)))
            .collect();
        for expect in ["sl_pct_for", "horizon_price_of", "sink", "read_weight"] {
            assert!(
                all.contains(expect),
                "闭包扫描没抽到 `{expect}`（实际抽出 {all:?}）—— 判据可能已失效，\
                 后续断言会失去区分力"
            );
        }

        let problems = find_closures_called_by_name(&PRODUCTION_SCRIPTS);
        assert!(
            problems.is_empty(),
            "以下闭包被**按名调用**。Rhai 的按名调用不查作用域里的 FnPtr ⇒ \
             运行时必报 Function not found（且编译期不报），请改为 `name.call(...)`:\n  {}",
            problems.join("\n  ")
        );
    }

    /// 负对照：证明上面的门禁**真的会告警**（0 命中 ≠ 没问题）。
    #[test]
    fn gate_reports_closure_called_by_name() {
        let fake = "// 注释里 by_name(2) 不算调用\n\
                    let by_name = |x| x + 1;\n\
                    let y = by_name(1);\n\
                    let ok = |x| x + 1;\n\
                    let z = ok.call(1);\n\
                    let w = \"by_name(3)\";\n";
        let problems = find_closures_called_by_name(&[("injected", fake)]);
        assert_eq!(
            problems,
            vec!["injected: by_name".to_string()],
            "门禁没能报出按名调用的闭包（且 `.call(...)` / 注释 / 字符串不得被误判）"
        );
    }

    /// 语言语义锁定：`let f = |...|` 存的是 **FnPtr**，只能 `f.call(...)` 调用。
    ///
    /// 上一条是**静态**判据，它成立的前提是「`f(x)` 真的会失败」—— 本测试用真执行
    /// 把这个前提钉住：Rhai 若哪天支持了 `f(x)`，本测试会失败并提示判据前提已变
    /// （而不是让整族门禁悄悄退化成假绿）。
    #[test]
    fn closure_in_variable_resolves_only_via_dot_call() {
        let engine = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);

        let err = engine
            .eval::<i64>("let f = |x| x + 1; f(1)")
            .expect_err("按名调用闭包应当报错 —— 若此处不报错，本模块的闭包门禁前提已变")
            .to_string();
        assert!(err.contains("Function not found"), "错误形态与判据预期不符：{err}");

        let via_call: i64 =
            engine.eval("let f = |x| x + 1; f.call(1)").expect("f.call(1) 应当可用");
        assert_eq!(via_call, 2, "闭包经 .call() 调用的返回值不符");
    }
}
