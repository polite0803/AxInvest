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
    const PRODUCTION_SCRIPTS: [(&str, &str); 22] = [
        ("analyst-brief", include_str!("../analyst-brief.rhai")),
        ("bottleneck-calc", include_str!("../bottleneck-calc.rhai")),
        ("consistency-check", include_str!("../consistency-check.rhai")),
        ("data-quality", include_str!("../data-quality.rhai")),
        ("data-verifier", include_str!("../data-verifier.rhai")),
        ("pace-calc", include_str!("../pace-calc.rhai")),
        // P4′-c（R-11 逐档分支）：四条分支脚本 + 仲裁节点也吃宿主函数
        // （`pm_leg_signal` / `pm_vol_move_pct` / `pm_band_z` / `pm_kelly_position` /
        //  `pm_kelly_growth`），注册面判据必须覆盖到它们 —— 本清单**漏一条脚本**，
        // 那一条的「调了没注册的函数」就回到 V54 事故的原点（运行期才 Function not found）。
        ("portfolio-mgr-arbiter", include_str!("../portfolio-mgr-arbiter.rhai")),
        ("portfolio-mgr-h-long", include_str!("../portfolio-mgr-h-long.rhai")),
        ("portfolio-mgr-h-mid", include_str!("../portfolio-mgr-h-mid.rhai")),
        ("portfolio-mgr-h-short", include_str!("../portfolio-mgr-h-short.rhai")),
        ("portfolio-mgr-h-ultra-short", include_str!("../portfolio-mgr-h-ultra-short.rhai")),
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
        // - `horizon_const`：2 参 + 多行体（锁住多参形态）。原样本 `horizon_price_of` 随
        //   v125「价位映射改投影」退役 ⇒ 判据不删、只换样本（否则多参这条覆盖面静默消失）；
        // - `sink`：定义在 `fn main()` **内部**（锁住「不只在顶层」）；
        // - `read_weight`：`strategy-scorer.rhai:57` 定义了却从未调用（锁住「定义即入集」）。
        let all: HashSet<String> = PRODUCTION_SCRIPTS
            .iter()
            .flat_map(|(_, src)| closure_var_names(&strip_comments_and_strings(src)))
            .collect();
        for expect in ["sl_pct_for", "horizon_const", "sink", "read_weight"] {
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

    // ─────────────────────────────────────────────────────────────────────────────
    // 运行期契约：`portfolio-mgr.rhai` 必须**真的跑完**（而不是只编译过）
    // ─────────────────────────────────────────────────────────────────────────────

    /// portfolio-mgr 节点 `input_mapping` 的 key 侧（= 该脚本**唯一**的注入通道）。
    ///
    /// 抽取面只取**本节点窗口**（`id: "portfolio-mgr"` → `nodes.push(pm);`）：全 seed 宽口径会把
    /// 别的节点的映射算成本节点的来源 —— 那正是 2026-10-01 漏判的形态（口径越宽，越只能漏判）。
    /// 窗口失效由紧随其后的规模自证兜住，不会静默变宽/变窄。
    fn portfolio_mgr_mapping_names(seed: &str) -> std::collections::BTreeSet<String> {
        let win_start = seed
            .find(r#"id: "portfolio-mgr".into()"#)
            .expect("seed 里应能定位 portfolio-mgr 节点（改名/搬迁须同步本门）");
        let win_end = seed[win_start..]
            .find("nodes.push(pm);")
            .map(|i| win_start + i)
            .expect("portfolio-mgr 节点窗口的终点 `nodes.push(pm);` 已变，须同步本门");
        let tuple_re = regex::Regex::new(r##"\(\s*"([A-Za-z_][A-Za-z0-9_]*)"\s*,"##).unwrap();
        let mut names: std::collections::BTreeSet<String> =
            tuple_re.captures_iter(&seed[win_start..win_end]).map(|c| c[1].to_string()).collect();
        // 可调参数由常量派生（源文本里**没有**字面量）⇒ 显式并入（它们同样进该节点的映射）。
        names.extend(
            crate::commands::stock_analysis_setup::seed_stock_analysis::PORTFOLIO_MGR_TUNABLE_PARAMS
                .iter()
                .map(|n| (*n).to_string()),
        );
        // 前提自证：抽取面必须真的张开（窗口或常量一旦失效，本门会退化成「什么都没扫到」）。
        // ⚠ 探针名要跟着注入面走：`horizon_leg_weights_json` 已随 R-11 退役（乘数语义 =
        //   「一个算法四套参数」的藏身处，PLAN §十一-3），拿退役名当探针会让本门
        //   在**映射真的被删掉时**恰好报「抽取失效」而看不出是接线被删。
        //   现用 `h_ultra_short`（换心脏后的四路分支输出之一）当代替它承担那一格。
        for probe in ["h_ultra_short", "valuation_dcf_upside", "action_buy_threshold"] {
            assert!(
                names.contains(probe),
                "注入面抽取失效：缺 `{probe}`（窗口/常量已漂移，实际 {names:?}）"
            );
        }
        names
    }

    /// 脚本里 `present(x)` 的名字（V57 会为它们补 unit）。含脚本自己的 `let` / `fn` 形参也无妨：
    /// 它们会遮蔽同名注入变量（`allow_shadowing` 默认 true，生产同样如此）。
    fn present_guard_names(script: &str) -> std::collections::BTreeSet<String> {
        let present_re = regex::Regex::new(r"present\(\s*([A-Za-z_][A-Za-z0-9_]*)").unwrap();
        present_re.captures_iter(script).map(|c| c[1].to_string()).collect()
    }

    /// 生产注入面 —— 造 scope 必须按 `code_executor::execute_rhai_directly` 的**同一份规则**，
    /// 否则本测试要么造出「生产不会发生的失败」（误报），要么放过「生产必然发生的失败」（假绿）。
    ///
    /// 规则（`code_executor.rs` 的 `execute_rhai_directly`）：
    ///   ① 逐条 `input_mapping`（`target_key ← source_key`）解析后 `push_constant`
    ///      （解析不到 ⇒ unit）⇒ **脚本能看到的注入名 = 本节点 input_mapping 的 key 侧**；
    ///   ② 其外再给**脚本里 `present(x)` 的未注入名字**补 unit（V57）—— 这一步
    ///      **只覆盖 `present()` 的面**：裸引用不在其判据面上。
    ///
    /// ⚠ 2026-10-01 收紧（首版正是**在这一点上瞎了**）：初版把 `hooks.rs` 里所有
    ///   `Variable { name: "x" }` 也算作注入面 ⇒ 「hooks 注入了 `horizon_prior_json`、
    ///   但 portfolio-mgr 的 `input_mapping` **漏了同名映射**」这一格恰好落在门的盲区里：
    ///   门替它补了 unit ⇒ 恒绿；而生产在 `portfolio-mgr.rhai` **当日 2853 行**抛
    ///   `Variable not found: horizon_prior_json`（实测 2026-10-01 09:37 的 live 运行）。
    ///   ⚠ 该数字**只存于当日日志**：`horizon_prior_json` 的裸读已随 R-11 重做退役，
    ///   按现行号去定位会指到空行（`check-single-source-facts` 正是这样抓到过它）——
    ///   历史事故引用行号时，必须同时写明它是「当日行号」而不是「当前第 N 行」。
    ///   ⇒ **hooks 的 `name:` 字面量不能算来源**：它只证明「变量进了黑板的 variables」，
    ///   不证明「进了这个脚本的 scope」；后者由该节点的 `input_mapping` 单独决定。
    ///   （同一份「hooks 名字」改由下面 `every_injected_var_read_bare_has_a_mapping` 当**反例来源**用。）
    ///
    /// ⚠ 清单从**源码文本**派生而不是手抄：手抄清单会在下一次 seed / 脚本改动时静默失效。
    fn production_injection_names(script: &str) -> std::collections::BTreeSet<String> {
        let mut names = portfolio_mgr_mapping_names(include_str!(
            "../stock_analysis_setup/seed_stock_analysis.rs"
        ));
        names.extend(present_guard_names(script));
        names
    }

    /// 一路分支的**输出夹具**（形状 = `portfolio-mgr-h-*.rhai` 的返回段，逐键对齐）。
    ///
    /// 为什么在这儿造而不是读真节点输出：本门测的是**装配段 + 选档段**，输入就是
    /// 「某路节点交上来的东西」。值本身不必与生产同（那些数由分支脚本与门
    /// `branch_node_scope_is_exactly_the_seed_mapping` 各自钉），但**键集合必须真** ——
    /// 装配段靠 `confidenceMethod` 自证字段判断「这是分支输出」，
    /// 键名写错就会走「缺自证 ⇒ 按缺席处理」那条路，正是要被本门看见的形态。
    ///
    /// ⚠ 四档的 `confidence` / `positionPct` **刻意不同**（阶段2 选档判据需要可区分的输入）：
    ///   long 置信最高但仓位 0、mid 置信次高且仓位 >0 ⇒ 「可执行优先」若被写反，
    ///   选档会落到 long，本门的 `timeHorizon` 断言当场红。全给同一个数就等于没测。
    fn branch_row_fixture(tier: &str, method: &str) -> serde_json::Value {
        // 四档的置信 / 仓位 / 天数 / 价带**全部不同**：阶段2 的选档判据与阶段3′ 的
        // 「主档数值取自所选档」都需要可区分的输入 —— 任何一项给成同一个数，对应那条
        // 断言就退化成恒真（原夹具四档同天数同档位，就是这么漏掉「两数同屏」的）。
        // ⚠ 天数一律**整数字面量**：分支表经生产注入落 Rhai i64（见 `spec_int` 的口径注），
        //   夹具写 5.0 会既测不到 i64 通路，又让断言侧的 `as_i64()` 拿到 None。
        let (conf, pos, action, days, sl, tp) = match tier {
            "ultra_short" => (48.0_f64, 0.0_f64, "观望", 2_i64, 2.6_f64, 2.6_f64),
            "short" => (55.0, 0.0, "观望", 5, 6.0, 12.0),
            "mid" => (61.0, 12.0, "增持", 28, 4.0, 8.0),
            "long" => (70.0, 0.0, "观望", 90, 15.5, 41.0),
            _ => (52.0, 0.0, "观望", 5, 6.0, 12.0),
        };
        serde_json::json!({
            "horizon": tier,
            "action": action,
            "verdict": format!("决策={action} 置信={conf}% 档={tier}"),
            "positionPct": pos,
            "confidence": conf,
            "posterior": conf / 100.0,
            "expectedHoldingDays": days,
            "stopLossPct": sl,
            "takeProfitPct": tp,
            "odds": (tp / sl * 100.0).round() / 100.0,
            "evidenceScale": 60.0,
            "scoreSource": "tier_native",
            "priorSource": "gate_hitrate",
            "priorSamples": 12.0,
            "stopSource": "vol_band",
            "positionSource": "kelly_x_position_multiplier",
            "confidenceMethod": method,
            "exitRule": "time_stop+fixed_stop",
            "entryGate": "none",
            "entryGatePassed": true,
            "legs": [],
            "absentLegs": [],
            "dataGaps": [],
        })
    }

    /// 按生产注入面造 scope 并执行 `portfolio-mgr.rhai`，返回脚本输出的 JSON。
    ///
    /// 「有估值证据」是本函数的**引爆条件**：`f5_weight > 0` 才会走进 f5 融合段 ——
    /// 2026-09-30 的生产事故正落在该段（裸引用 `time_horizon` ⇒ 运行期 `Variable not found`
    /// ⇒ 被文件末尾的 catch 整体兜成 `action="数据缺失"`）。估值腿两腿给值即可引爆。
    fn run_portfolio_mgr(script: &str) -> serde_json::Value {
        run_portfolio_mgr_with(script, &[])
    }

    /// `absent` 列出**不注入**的分支档名（`ultra_short` / `short` / `mid` / `long`）。
    /// 缺席不是「传 0」：装配段读到的必须是**没这个键**（生产里 = 该路节点失败或未接线 ⇒ unit），
    /// 才能验已批口径 A（PLAN §四十一）——该档不出现，且 `data_gaps` 必须点名。
    fn run_portfolio_mgr_with(script: &str, absent: &[&str]) -> serde_json::Value {
        run_portfolio_mgr_fully(script, absent, &[])
    }

    /// `overrides` 把某些注入名从「unit（= 上游节点失败这一常态）」换成**具体值**。
    ///
    /// B1(v128) 用它喂四档风险档（`overall_risk_ultra_short` / `_short` / `_mid` / `_long`）：
    /// 那四个键生产里是字符串档名，其余既有测试继续走 unit，两条通路互不影响。
    /// ⚠ 覆盖名在补 unit 的循环里**显式跳过** —— 同名 push 两次要靠 Rhai「后入遮蔽」才对，
    ///   而那是实现细节不是契约；本门不给自己造「scope 里两条都在」的模糊态。
    fn run_portfolio_mgr_fully(
        script: &str,
        absent: &[&str],
        overrides: &[(&str, serde_json::Value)],
    ) -> serde_json::Value {
        let engine = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);
        let ast = engine
            .compile(script)
            .unwrap_or_else(|e| panic!("portfolio-mgr.rhai 编译失败（生产同配置）: {e}"));

        let mut scope = rhai::Scope::new();
        for name in production_injection_names(script) {
            if overrides.iter().any(|(k, _)| *k == name) {
                continue;
            }
            scope.push_constant(name, rhai::Dynamic::UNIT);
        }
        // 周期常量表：缺失会让脚本**按设计** throw（未知周期静默兜天数 = 错档入库）⇒ 必须给真表。
        // 权威源 `Period::decision_consts_map`，本测试不手抄天数/乘数。
        scope.push_constant(
            "horizon_consts_json",
            axagent_harness::json_value_to_dynamic(
                &axagent_harness::holding_period::Period::decision_consts_map(),
            ),
        );
        // R-11 换心脏后的四路输入：装配段的**唯一**逐档来源。
        // 退役掉的 `horizon_leg_weights_json`（乘数表）不再注入 —— 脚本侧若还有人读它，
        // 会在这里以 `Variable not found` 红出来，而不是靠一张没人消费的表把它喂活。
        for (key, tier, method) in [
            ("h_ultra_short", "ultra_short", "no_cross_horizon_scaling"),
            ("h_short", "short", "no_cross_horizon_scaling"),
            ("h_mid", "mid", "inside_sigma_band"),
            ("h_long", "long", "snr_free_target_and_falsified"),
        ] {
            if absent.contains(&tier) {
                continue;
            }
            scope.push_constant(
                key,
                axagent_harness::json_value_to_dynamic(&branch_row_fixture(tier, method)),
            );
        }
        // 估值两腿有值 ⇒ f5_weight > 0 ⇒ 进入 f5 融合段（本次回归的引爆条件）。
        // 其余 input_mapping 键保持 unit = 「上游节点失败」这一生产常态。
        scope.push_constant("valuation_dcf_upside", -12.5_f64);
        scope.push_constant("valuation_graham_upside", -25.0_f64);
        scope.push_constant("valuation_dcf_applicable", true);
        scope.push_constant("valuation_dcf_anchor_is_fallback", false);
        // K3(2026-10-02)：三档增速全负标记。此处给 `false` = 「区间确实是保守—乐观带」，
        // 使本组既有测试的断言不被新变量改变（缺省留 unit 会让面板走另一条文案分支）。
        scope.push_constant("valuation_dcf_growth_band_all_negative", false);

        for ov in overrides {
            scope.push_constant(ov.0, axagent_harness::json_value_to_dynamic(&ov.1));
        }

        let out: rhai::Dynamic = engine
            .eval_ast_with_scope(&mut scope, &ast)
            .unwrap_or_else(|e| panic!("portfolio-mgr.rhai 执行失败（非 catch 路径）: {e}"));
        axagent_harness::dynamic_to_json_value(&out)
    }

    /// 运行期契约：**有估值证据**时脚本必须跑完全程，不得落进 catch 兜底。
    ///
    /// ## 为什么必须有这道门（2026-09-30 生产实证）
    ///
    /// f5 融合段曾经裸引用 `time_horizon` —— 它的定义在文件**后面的定档段**（Rhai 顺序求值
    /// ⇒ 运行期 `Variable not found`），被文件末尾的 catch 整体兜成 `action="数据缺失"`。
    /// 即**凡有估值数据的分析全部静默降级为保守决策**，而当时**没有任何门禁**能发现它：
    ///
    /// | 既有门 | 为什么看不见 |
    /// |---|---|
    /// | `rhai_syntax_check::all_rhai_scripts_compile` | 只 `compile`；Rhai 编译期**不校验变量** |
    /// | `code_executor` 的 V57 补 unit | 只覆盖 `present(x)` 里的名字，**裸引用不在判据面上** |
    /// | `portfolio_mgr_*_rhai.rs` 各门 | 只跑**抽出来的片段**，从不执行整脚本 |
    ///
    /// ⇒ 本门是「整脚本真执行」这一格的第一道。断言分四层，缺一层就会假绿：
    ///   ① **没降级**：`reasoning` 不含「执行异常」、`action != "数据缺失"`（catch 的指纹）；
    ///   ② **四档真的来自四路分支**：`decisionsByHorizon` 四行齐备、逐行带分支自证
    ///      （`confidenceMethod` 非空 + `horizon` 与档位键匹配）。
    ///      退役前这一格断的是 `weightsSource == "table"`（乘数表生效）—— 乘数语义随 R-11 退役后
    ///      换成读分支自己的口径，否则「跑完了」可能只是主链代算出四行。
    ///   ③ **乘数通道必须整体不存在**：`weightAdjustments` 这个键不得出现在输出里
    ///      （3b-α 恒空 → 3b-β 删字段），且反向锁「降权/乘数类文案不得回流 `data_gaps`」
    ///      —— 那正是本仓为「常驻误报」付过代价的复发形态；
    ///   ④ **已批口径 A**（PLAN §四十一）：抽掉一路输入 ⇒ `decisionsByHorizon` **少一个键**
    ///      （不是补一行「无结论」、更不是退回闭包代算），且 `data_gaps` 必须点名那一路。
    /// 末尾附**负控**：把脚本改回「裸引用一个未定义名字」的形态，断言上面的判据真的会报红 ——
    /// 否则本门只是一组恒真断言（本仓对每道新门都要求这一步）。
    #[test]
    fn portfolio_mgr_runs_with_valuation_evidence_without_degrading() {
        let src = PRODUCTION_SCRIPTS
            .iter()
            .find(|(l, _)| *l == "portfolio-mgr")
            .expect("脚本清单缺 portfolio-mgr")
            .1;

        let out = run_portfolio_mgr(src);

        // ── ① 没降级 ──
        let reasoning = out["reasoning"].as_str().unwrap_or_default();
        assert!(
            !reasoning.contains("执行异常"),
            "portfolio-mgr.rhai 落进了 catch 兜底（脚本崩了 ≠ 判断为保守）: {reasoning}"
        );
        assert_ne!(
            out["action"].as_str(),
            Some("数据缺失"),
            "catch 兜底指纹：action=\"数据缺失\"。完整输出: {out}"
        );

        // ── ② 四档真的产出，且权重来自权威乘数表 ──
        let tiers = out["decisionsByHorizon"].as_object().unwrap_or_else(|| {
            panic!("decisionsByHorizon 应为四档 map，实际: {}", out["decisionsByHorizon"])
        });
        assert_eq!(
            tiers.len(),
            4,
            "四档决策应齐备，实际键: {:?}",
            tiers.keys().collect::<Vec<_>>()
        );
        // 每行必须是**分支节点交上来的东西**（自证键在），而不是主链代算出来的一行。
        // 退役前的断言是 `weightsSource == "table"`（乘数表生效），那个字段随 `leg_mult` 一起没了 ⇒
        // 换成读分支自己的口径自证：`confidenceMethod` 由 `horizon_branch_specs` 给，
        // 装配段还额外用它判「接错节点」（缺自证字段 ⇒ 按缺席处理），所以这一断言同时锁两件事。
        for (k, v) in tiers {
            assert!(
                v["confidenceMethod"].as_str().is_some_and(|s| !s.is_empty()),
                "{k} 行没有分支自证字段 confidenceMethod ⇒ 不是四路分支的输出（疑主链代算或接错节点）: {v}"
            );
            assert_eq!(
                v["horizon"].as_str().unwrap_or_default(),
                match k.as_str() {
                    "ultraShort" => "ultra_short",
                    other => other,
                },
                "{k} 行的 horizon 与档位键不匹配（装配段接错路）: {v}"
            );
        }

        // ── ③ 乘数通道整体退役 ⇒ 输出里不得再出现 `weightAdjustments` 这个键 ──
        // 判据翻过一次：3b-α 时它**恒空**（前端仍读该键，留空数组避免走进另一条渲染分支），
        // 3b-β 连字段一起删除 ⇒ 现在要求「不存在」。留一个永不成立的键和留一个永不亮起的
        // 面板分支是同一件事的两面 —— 都让读者以为那条通道还活着。
        let gaps: Vec<String> = out["data_gaps"]
            .as_array()
            .unwrap_or_else(|| panic!("data_gaps 应为数组: {}", out["data_gaps"]))
            .iter()
            .filter_map(|g| g.as_str().map(str::to_string))
            .collect();
        assert!(
            out.get("weightAdjustments").is_none(),
            "weightAdjustments 应随逐档乘数一起退役（前端类型 / 渲染 / 11 语言键同批删除），\
             输出里不该再有这个键: {:?}",
            out.get("weightAdjustments")
        );
        // 反向锁（常驻误报的复发形态，与退役前同一判据）：设计选择不得占用「数据缺口」通道。
        assert!(
            !gaps.iter().any(|g| g.contains("估值腿周期降权") || g.contains("乘数")),
            "降权/乘数类文案又回到 data_gaps（该通道语义是「本该拿到的数据没拿到」）: {gaps:?}"
        );

        // ── ④ 已批口径 A（PLAN §四十一）：某一路没产出 ⇒ 该档**不出现在** decisionsByHorizon ──
        // 三种「不允许的替代做法」都在这里被反向锁住：补一行无结论 / 退回闭包代算 / 用主链后验凑数。
        //
        // ⚠ 逐档**各剥一遍**（2026-10-05 补齐，任务 #14 的覆盖面缺口）：首版只剥 mid 一路，
        //   若某一路在装配段被特殊对待（例如超短的 days/position 走另一支、或某档的缺席
        //   没有进 data_gaps），单路样本永远看不见 —— 「只测一路」等于另外三路没有门。
        // ⚠ 三个名字族各有拼写，用错一族就恒真：`absent` 参数按 **snake**（夹具 `tier` 用它），
        //   `decisionsByHorizon` 的键按 **camel**（装配段 `decisions_by_horizon[b.camel]`，
        //   :3026 明文「前端按 camelCase 键分组」），而 data_gaps 的文案用**中文档名**
        //   （`cn_tier` 闭包 :2988）。`ultra_short` 是唯一 snake≠camel 的那一族 ⇒ 首版正是它会被漏掉。
        for (snake, camel, cn) in [
            ("ultra_short", "ultraShort", "超短线"),
            ("short", "short", "短线"),
            ("mid", "mid", "中线"),
            ("long", "long", "长线"),
        ] {
            let one_missing = run_portfolio_mgr_with(src, &[snake]);
            let tiers2 = one_missing["decisionsByHorizon"].as_object().unwrap_or_else(|| {
                panic!(
                    "{snake} 缺席时 decisionsByHorizon 应为 map，实际: {}",
                    one_missing["decisionsByHorizon"]
                )
            });
            assert_eq!(
                tiers2.len(),
                3,
                "{snake} 少注入一路却仍有四行 ⇒ 装配段在代算，缺席被伪装成结论: {:?}",
                tiers2.keys().collect::<Vec<_>>()
            );
            assert!(
                !tiers2.contains_key(camel),
                "{camel} 缺席档仍以空行/占位行出现（口径 A 要求不出键）: {:?}",
                tiers2.keys().collect::<Vec<_>>()
            );
            let gaps2: Vec<String> = one_missing["data_gaps"]
                .as_array()
                .map(|a| a.iter().filter_map(|g| g.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            assert!(
                gaps2.iter().any(|g| g.contains(&format!("{cn}分支未产出"))),
                "{snake} 的缺席必须在 data_gaps 点名（结构性缺口不得在呈现层造成歧义），实际 {gaps2:?}"
            );
        }

        // ── ⑤ 阶段2（Q1=C）：主档由四档结论选出，且**可执行优先于置信最大** ──
        // 夹具里 long 置信最高（70）但仓位 0、mid 次高（61）且仓位 12 ⇒ 正确产物是 mid。
        // 判据写反（只按 confidence 取最大）就会落到 long，本断言当场红 —— 这就是夹具
        // 四档数值刻意不同的理由。
        assert_eq!(
            out["horizonSource"].as_str(),
            Some("branch_pick"),
            "四路都在场时主档来源必须是 branch_pick（不再是后验阈值映射），实际: {}",
            out["horizonSource"]
        );
        assert_eq!(
            out["timeHorizon"].as_str(),
            Some("mid"),
            "选档没体现「可执行优先」（夹具里只有 mid 仓位 >0）: {}",
            out["timeHorizon"]
        );
        assert_eq!(
            out["action"].as_str(),
            Some("增持"),
            "主 action 必须取自所选档（夹具里 mid 是「增持」）；落回主链阶梯或被判风控改档都要在这里暴露: {}",
            out["action"]
        );
        // 抽掉所选那一档 ⇒ 必须换档而不是消失（防止「主档指向一行根本不在面板里的输出」）。
        // 夹具里撤掉 mid 后已无可执行档 ⇒ 按判据②取剩余置信最大的 long（70）。
        let pick_gone = run_portfolio_mgr_with(src, &["mid"]);
        assert_eq!(
            pick_gone["timeHorizon"].as_str(),
            Some("long"),
            "撤掉被选中的 mid 后应改选剩余最强档（long 置信 70），实得 {}",
            pick_gone["timeHorizon"]
        );
        assert_eq!(
            pick_gone["horizonSource"].as_str(),
            Some("branch_pick"),
            "换档后来源仍须是 branch_pick，实得 {}",
            pick_gone["horizonSource"]
        );

        // ──  阶段2 兜底通路：四路全缺席 ⇒ 退回阈值定档，但**必须自报兜底身份** ──
        // 未重播种的存量库正是这个形态。允许兜底（否则历史记录一次性失效），
        // 不允许的是「兜底冒充正常路径」—— 所以来源值与 data_gaps 两条都要锁。
        let no_branch = run_portfolio_mgr_with(src, &["ultra_short", "short", "mid", "long"]);
        assert_eq!(
            no_branch["horizonSource"].as_str(),
            Some("formula_no_branch"),
            "四路全缺席时必须标 formula_no_branch，不得静默冒充 branch_pick: {}",
            no_branch["horizonSource"]
        );
        assert!(
            no_branch["decisionsByHorizon"].as_object().is_some_and(|m| m.is_empty()),
            "四路全缺席却仍产出逐档行 ⇒ 主链在代算: {}",
            no_branch["decisionsByHorizon"]
        );
        let gaps3: Vec<String> = no_branch["data_gaps"]
            .as_array()
            .map(|a| a.iter().filter_map(|g| g.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        assert_eq!(
            gaps3.iter().filter(|g| g.contains("分支未产出")).count(),
            4,
            "四路缺席必须逐档各点一条（合并成一条就看不出缺哪几路），实际 {gaps3:?}"
        );

        // ── ⑦ 阶段3′（批准 ①）：主档的**数值**也必须取自所选档，不得由主链重算 ──
        // 夹具里 mid 行是 仓位 12 / 止损 4 / 止盈 8 / 天数 28 / stopSource=vol_band，
        // 而主链日线口径在同一输入下给的是**另一组数**（mid 兜底档位 8/18、stopSource 只会是
        // `vol` 或 `fallback_pct`）⇒ 下面每一条都在「改回主链公式」时必红。
        // 这一族断言的存在理由就是用户本轮的抱怨：档是分支选的、数却是主链算的，同屏两个口径。
        let mid_row = &out["decisionsByHorizon"]["mid"];
        assert!(!mid_row.is_null(), "主档选中 mid，但 decisionsByHorizon 里没有 mid 行");
        let row_pos = mid_row["positionPct"].as_f64().unwrap_or(-1.0);
        let main_pos = out["positionPct"].as_f64().unwrap_or(-1.0);
        // 只锁**方向**（只降不升），不锁非零：主档在选出之后还要过若干同样只做减法的封顶
        // （证据不足 ⇒ 空仓、风险等级上限、试探仓），本门的合成输入正命中其中一条 ⇒ 主档 0
        // 是合法产物。「主档确实拿到过分支那个数」由 `positionSource` 那条断言负责，
        // 「非零仓位在真数据下成立」由阶段 5 的实机重放核对负责 —— 各锁各的层次。
        assert!(
            main_pos <= row_pos,
            "主档仓位只能被**往下**封顶（分支 {row_pos}，主档 {main_pos}）⇒ 出现抬升就是两处口径互相加成"
        );
        assert_eq!(
            out["expectedHoldingDays"].as_i64(),
            mid_row["expectedHoldingDays"].as_i64(),
            "主档持有天数与所选档不符（两套算法又同屏）: 主档 {} vs 行内 {}",
            out["expectedHoldingDays"],
            mid_row["expectedHoldingDays"]
        );
        // 出场三件（止损 / 止盈 / 来历标签）与**空仓不变式**绑定：主档被风险预算封成 0 仓位时，
        // 档位必须归零、标签退回主链 —— 那时它本来就没有采用分支的数，硬要求相等反而是假锁。
        if main_pos > 0.0 {
            assert_eq!(
                out["stopLossPct"].as_f64(),
                mid_row["stopLossPct"].as_f64(),
                "主档止损% 与所选档不符: 主档 {} vs 行内 {}",
                out["stopLossPct"],
                mid_row["stopLossPct"]
            );
            assert_eq!(
                out["takeProfitPct"].as_f64(),
                mid_row["takeProfitPct"].as_f64(),
                "主档止盈% 与所选档不符: 主档 {} vs 行内 {}",
                out["takeProfitPct"],
                mid_row["takeProfitPct"]
            );
            assert_eq!(
                out["stopSource"].as_str(),
                mid_row["stopSource"].as_str(),
                "主档止损的来历必须是被采用那条算法**自己**的标签（退回主链标签就是在冒充）"
            );
        } else {
            assert_eq!(
                out["stopLossPct"].as_f64(),
                Some(0.0),
                "空仓主档却带非零止损 ⇒ 把「不下注」伪装成「等执行」: {}",
                out["stopLossPct"]
            );
        }
        let psrc = out["positionSource"].as_str().unwrap_or_default();
        assert!(
            psrc == "branch_position" || psrc == "branch_position_capped",
            "选中了分支档却标注主链仓位来源 ⇒ 「谁定的数」再次不可反解: {psrc}"
        );
        // ── ′ 主档 action / confidence 的来历（§五十三 ①，v127）──
        //   000710 实测的形态：档是超短线选的、分支给的是「买入」，主档却是「持有」，
        //   唯一解释在 reasoning 的中文句子里 ⇒ 展示层要成句、反思要按档统计都不能靠解析文本。
        //   本段把「标签与实际路径必须自洽」钉住：标签说直取，action 就必须等于该档 action。
        const ACTION_SOURCES: [&str; 7] = [
            "branch_pick",
            "risk_veto_downgrade",
            "bearish_veto_downgrade",
            "sim_veto_downgrade",
            "sanity_cap_downgrade",
            "partial_low_confidence_cap",
            "main_chain",
        ];
        let asrc = out["actionSource"].as_str().unwrap_or_default();
        assert!(
            ACTION_SOURCES.contains(&asrc),
            "actionSource 缺失或落在值域外 ⇒ 主档 action 的来历又不可反解: {:?}",
            out["actionSource"]
        );
        let main_act = out["action"].as_str().unwrap_or_default();
        let row_act = mid_row["action"].as_str().unwrap_or_default();
        if asrc == "branch_pick" {
            assert_eq!(
                main_act, row_act,
                "actionSource 说「直取所选档」，action 却不等于该档 action ⇒ 标签在说谎"
            );
        }
        assert_ne!(asrc, "main_chain", "本门四路输入齐备（branch_picked 必真），却标成主链定档");
        let csrc = out["confidenceSource"].as_str().unwrap_or_default();
        assert!(
            csrc == "branch_row" || csrc == "main_chain_posterior",
            "confidenceSource 缺失或值域外 ⇒ 同屏两个置信度又无从分辨: {:?}",
            out["confidenceSource"]
        );
        if csrc == "branch_row" {
            let row_conf = mid_row["confidence"].as_f64().unwrap_or(-1.0);
            let main_conf = out["confidence"].as_f64().unwrap_or(-1.0);
            assert!(
                main_conf <= row_conf + f64::EPSILON,
                "主档置信度**高于**所选档自己报的数 ⇒ 「同源 + 只降不升」被绕过: 主 {main_conf} vs 档 {row_conf}"
            );
        }
        // 价位映射 = 逐档行的**投影**：任何字段不相等都说明它又自己算了一遍。
        let price_mid = &out["horizonPriceMap"]["mid"];
        assert_eq!(
            price_mid["stopLossPct"].as_f64(),
            mid_row["stopLossPct"].as_f64(),
            "horizonPriceMap 与 decisionsByHorizon 不同源 ⇒ 同一档两个价位（R-11 判废的形状）"
        );
        assert_eq!(
            price_mid["stopLoss"].as_f64(),
            mid_row["stopLoss"].as_f64(),
            "绝对止损价不同源（投影被改成了重算）"
        );
        assert_eq!(
            price_mid["expectedHoldingDays"].as_i64(),
            mid_row["expectedHoldingDays"].as_i64(),
            "持有天数不同源"
        );
        assert!(
            out["horizonPriceMap"].get("short").is_some(),
            "在场档（short 节点有输出）却没进价位映射 ⇒ 投影的缺席判据用错了对象"
        );
        // 键拼写：映射必须与它投影的 `decisionsByHorizon` 同族（camelCase）。
        // 这条锁的是本轮顺手修掉的既有缺陷 —— 旧映射出 snake 键，而展示层按 camel 读、
        // 透传只做类型断言不转键 ⇒ 四键里只有 `ultraShort` 拼写错开（其余三档两族同名），
        // 表现为「超短档价位行从来不显示」，而库里数据是齐的（000710 现网行四键都在）。
        assert!(
            out["horizonPriceMap"].get("ultraShort").is_some(),
            "价位映射没出 camel 键 ⇒ 超短档那一行会在展示层静默丢失: {}",
            out["horizonPriceMap"]
        );
        assert!(
            out["horizonPriceMap"].get("ultra_short").is_none(),
            "价位映射又出 snake 键（与 decisionsByHorizon 不同族，历史形态）"
        );
        // 口径 A 的延伸：四路全缺席 ⇒ 价位映射也必须**空**（不得由主链代算四档价位）。
        assert!(
            no_branch["horizonPriceMap"].as_object().is_some_and(|m| m.is_empty()),
            "四路缺席却仍有价位映射条目 ⇒ 主链在代算逐档价位: {}",
            no_branch["horizonPriceMap"]
        );

        // ── 负控：判据必须能报出「生产事故形态」──
        // 把 f5 段的入口改成**裸引用一个从未定义的名字**（= 修复前的 `time_horizon` 形态；
        // 注意它不在任何 `present(...)` 里 ⇒ 连 V57 也不会补 unit ⇒ 生产同样必崩）。
        // ⚠ 名字必须是**合法标识符**且不带前导双下划线：Rhai 对 `__x__` 这类名字直接报
        //   `Variable name is not proper`（编译期），那会变成「编译失败」而不是「运行期降级」，
        //   负控就测不到本门真正要测的那条路径（首版实测踩到）。
        let mutated = src.replace(
            "let f5_weight = if f5_has_valuation {",
            "let f5_weight = if forward_ref_probe == true { 0.0 } else if f5_has_valuation {",
        );
        assert_ne!(mutated, src, "负控变异点未命中 —— 判据与被测对象已脱节，须同步本测试");
        let degraded = run_portfolio_mgr(&mutated);
        let degraded_reasoning = degraded["reasoning"].as_str().unwrap_or_default();
        assert!(
            degraded_reasoning.contains("执行异常")
                && degraded_reasoning.contains("Variable not found"),
            "负控失效：裸引用未定义变量时脚本没有降级 ⇒ 上面的断言没有区分力。实际: {degraded_reasoning}"
        );
        assert_eq!(
            degraded["action"].as_str(),
            Some("数据缺失"),
            "负控失效：降级路径的 action 指纹不符"
        );
    }

    /// 纯判据：`(hooks/模板变量) ∩ 脚本裸读 ∩ ¬映射 ∩ ¬present ∩ ¬脚本局部`。
    ///
    /// 抽成纯函数是为了让**负控**能喂一份「摘掉映射行」的 seed 文本（见下方测试），
    /// 否则负控只能改磁盘上的生产文件 —— 那正是本仓反复禁止的形态。
    fn bare_injected_reads(
        script: &str,
        seed: &str,
        injected_elsewhere: &std::collections::BTreeSet<String>,
    ) -> Vec<String> {
        // 剥注释：注释里提到的工作流变量名不算「读」（本文件注释里大量出现变量名）。
        let code: String = script
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("//") {
                    ""
                } else {
                    l.split("//").next().unwrap_or(l)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mapping = portfolio_mgr_mapping_names(seed);
        let present = present_guard_names(script);

        // 脚本自己的局部名：`let x` / `for x in` / `fn f(a, b)` 形参 / 闭包 `|a, b|` 形参。
        let is_ident = |s: &str| {
            !s.is_empty() && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        };
        let mut locals: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for pat in [r"\blet\s+([A-Za-z_][A-Za-z0-9_]*)", r"\bfor\s+([A-Za-z_][A-Za-z0-9_]*)\s+in\b"]
        {
            let re = regex::Regex::new(pat).unwrap();
            locals.extend(re.captures_iter(&code).map(|c| c[1].to_string()));
        }
        for pat in [r"\bfn\s+[A-Za-z_][A-Za-z0-9_]*\s*\(([^)]*)\)", r"\|\s*([^|]*?)\s*\|"] {
            let re = regex::Regex::new(pat).unwrap();
            for c in re.captures_iter(&code) {
                for p in c[1].split(',') {
                    let n = p.trim();
                    if is_ident(n) {
                        locals.insert(n.to_string());
                    }
                }
            }
        }

        let mut out: Vec<String> = injected_elsewhere
            .iter()
            .filter(|n| !mapping.contains(*n) && !present.contains(*n) && !locals.contains(*n))
            .filter(|n| {
                regex::Regex::new(&format!(r"\b{}\b", regex::escape(n.as_str())))
                    .unwrap()
                    .is_match(&code)
            })
            .cloned()
            .collect();
        out.sort();
        out
    }

    /// 防回归：**「别处注入了」不等于「这个脚本看得到」** —— 凡被 hooks / 模板变量表注入、
    /// 又在 `portfolio-mgr.rhai` 里**裸读**（不在 `present()` 里）的名字，必须在本节点的
    /// `input_mapping` 里有同名条目；否则运行期 `Variable not found`，并被脚本自身的 catch
    /// 兜成 `action="数据缺失"`（**不是**降级为共用先验／默认值）。
    ///
    /// ## 为什么单独立这道门（与上面的真执行门互补）
    ///
    /// 真执行门只点亮**它那条夹具走到的分支**；本门是**全文件静态**判据，与分支覆盖无关。
    /// 两者的分工正是 2026-10-01 那次事故暴露的：Phase C（v102）只加了
    /// 「hooks 注入 `horizon_prior_json` + 脚本 `prior_for` 裸读」，**漏了本节点映射** ——
    /// 该缺陷被同一文件更早的崩溃（line 844 前向引用）遮住，直到 v112 修掉前者才在
    /// **当日 line 2853** 爆出来（实测那一轮 live 运行；该变量的裸读已随 R-11 重做退役，
    /// 两个数字都只是「当时第几行」，不可按现行号解析 —— 见 `production_injection_names` 的同类注）。
    /// 注入面三个点（hooks 注入 / 节点映射 /
    /// 脚本消费）少任何一个，症状都是同一条「数据缺失」，看不出缺的是哪一环。
    ///
    /// 判据口径（三处都靠源码文本派生，避免手抄清单漂移）：候选面 = hooks/seed-mod 的
    /// `Variable { name }` ∪ 模板变量表；排除面 = 本节点 `input_mapping` ∪ 脚本 `present(x)`
    /// ∪ 脚本局部名（`let` / `for..in` / `fn` 形参 / 闭包形参）；命中面 = 剥注释后按词边界
    /// 在脚本里出现。**四段都带规模自证**，任一段失效即报红而不是静默通过。
    #[test]
    fn every_injected_var_read_bare_has_a_mapping() {
        let script = PRODUCTION_SCRIPTS
            .iter()
            .find(|(l, _)| *l == "portfolio-mgr")
            .expect("脚本清单缺 portfolio-mgr")
            .1;
        let seed = include_str!("../stock_analysis_setup/seed_stock_analysis.rs");

        // 候选面：hooks / seed-mod 现场注入 ∪ 模板变量表（`build_template_variables` 是权威源）。
        let name_re = regex::Regex::new(r##"name:\s*"([A-Za-z_][A-Za-z0-9_]*)"##).unwrap();
        let mut elsewhere: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for src in [include_str!("hooks.rs"), include_str!("../stock_analysis_setup/mod.rs")] {
            elsewhere.extend(name_re.captures_iter(src).map(|c| c[1].to_string()));
        }
        elsewhere.extend(
            crate::commands::stock_analysis_setup::seed_variables::build_template_variables()
                .into_iter()
                .map(|v| v.name),
        );

        // 前提自证：候选面与排除面都必须真的张开（否则「没问题」只是没扫到 —— 本仓 #704/#711）。
        assert!(elsewhere.len() >= 100, "候选注入面抽取失效（只抽到 {} 个名字）", elsewhere.len());
        assert!(
            portfolio_mgr_mapping_names(seed).len() >= 50,
            "本节点映射抽取失效（只抽到 {} 条）",
            portfolio_mgr_mapping_names(seed).len()
        );
        assert!(
            present_guard_names(script).len() >= 50,
            "present 面抽取失效（只抽到 {} 个）",
            present_guard_names(script).len()
        );

        let bad = bare_injected_reads(script, &seed, &elsewhere);
        assert!(
            bad.is_empty(),
            "以下名字被 hooks/模板注入、在 portfolio-mgr.rhai 里**裸读**，却没有本节点 input_mapping \
             ⇒ 运行期 `Variable not found` 并被 catch 兜成「数据缺失」: {bad:?}"
        );

        // ── 负控：摘掉一行**主脚本真读**的映射 ⇒ 判据必须报出那个名字 ──
        // 首版这里用 `horizon_prior_json`（v113 事故的那个键）。3b 之后逐档先验表改由四路
        // 分支节点自己映射，主链不再读它 ⇒ 摘掉主链那行映射不再产生「裸读无映射」，
        // 负控当场失效（实测：`实际 []`）。换一个仍然三条件齐备的名字（hooks 注入 +
        // 主脚本裸读 + 本节点同名映射，且该行在 seed 里唯一，replace 不会误伤别处）。
        let mutated = seed.replace(r#"("horizon_consts_json", "horizon_consts_json"),"#, "");
        assert_ne!(mutated, seed, "负控变异点未命中 —— 映射行已改名，须同步本测试");
        let bad2 = bare_injected_reads(script, &mutated, &elsewhere);
        assert!(
            bad2.contains(&"horizon_consts_json".to_string()),
            "负控失效：摘掉 `horizon_consts_json` 的映射后判据没报出来，实际 {bad2:?}"
        );
    }

    /// pace-calc：LLM 手写 JSON 里的**整数置信度**必须当数读，不得塌成 0.5 兜底。
    ///
    /// 缺陷形态（`PLAN-four-horizon-workflow-alignment.md` §五十二 ②，已批）：
    /// `llm_events` = `a-catalyst.content`，是**模型手写的 JSON**，置信度写成整数（`85` / `1`）
    /// 是常态；脚本侧 `json_parse` 走 `json_value_to_dynamic`（`as_i64` 优先）⇒ 它是 i64，
    /// 而原判据 `type_of(..) == "f64"` 恒假、紧跟的 `== "int"` 在 64-bit 构建下也恒假 ⇒
    /// 落 `0.5` 兜底 —— 把「模型给了 85%」呈现成「50% 中性」，属用近似值顶替而非显式缺席。
    ///
    /// 三次同输入对照，缺任何一个都区分不出「桥没桥」：
    ///   ① `85`（i64）与 ② `85.0`（f64）必须**逐维相等** —— 型别不该改变结论；
    ///   ③ 不给 `confidence`（真缺席）必须与 ① **不相等** —— 这条就是负控：一旦有人把桥改回
    ///      f64 单型判据，① 立刻塌成 ③，本断言当场红（不需要另外变异源文件）。
    ///
    /// 夹具不手抄注入清单：`present(x)` 面从脚本自己派生，映射键则逐条对 seed 断言存在
    /// （改名即红，同 `production_injection_names` 那条「手抄清单会静默失效」的教训）。
    #[test]
    fn pace_calc_integer_confidence_is_not_read_as_absent() {
        let src = PRODUCTION_SCRIPTS
            .iter()
            .find(|(l, _)| *l == "pace-calc")
            .expect("脚本清单缺 pace-calc")
            .1;
        let seed = include_str!("../stock_analysis_setup/seed_stock_analysis.rs");
        let mut names = present_guard_names(src);
        for key in [
            "announcement_events",
            "money_flow_net",
            "money_flow_history",
            "sector_etf_direction",
            "p_history",
            "llm_events",
        ] {
            assert!(
                seed.contains(&format!("(\"{key}\",")),
                "pace-calc 节点的映射键 `{key}` 在 seed 里已改名/搬走 ⇒ 本测试的注入面须同步"
            );
            names.insert(key.to_string());
        }

        let run = |content: &str| -> serde_json::Value {
            let engine = build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO);
            let ast = engine
                .compile(src)
                .unwrap_or_else(|e| panic!("pace-calc.rhai 编译失败（生产同配置）: {e}"));
            let mut scope = rhai::Scope::new();
            for name in &names {
                scope.push_constant(name.as_str(), rhai::Dynamic::UNIT);
            }
            // 生产形态：input_mapping 给的是 {role, content} 包装对象（V69 实证过的那条）。
            scope.push_constant(
                "llm_events",
                axagent_harness::json_value_to_dynamic(&serde_json::json!({
                    "role": "a-catalyst",
                    "content": content,
                })),
            );
            let out: rhai::Dynamic = engine
                .eval_ast_with_scope(&mut scope, &ast)
                .unwrap_or_else(|e| panic!("pace-calc.rhai 执行失败: {e}"));
            axagent_harness::dynamic_to_json_value(&out)
        };

        let with_conf = |conf: &str| {
            format!(
                r#"{{"report":"催化剂评估","verdict":{{"catalyst_level":"L2政策利好","confidence":{conf},"verdict":"看多"}}}}"#
            )
        };
        let as_int = run(&with_conf("85"));
        let as_float = run(&with_conf("85.0"));
        let absent = run(
            r#"{"report":"催化剂评估","verdict":{"catalyst_level":"L2政策利好","verdict":"看多"}}"#,
        );

        // 前提自证：夹具真的引爆了事件路径，否则下面比的是两个 0。
        assert!(
            as_int["valid_events"].as_i64().unwrap_or(0) >= 1,
            "夹具未产出有效事件（valid_events={:?}）⇒ 三维对照不具区分力",
            as_int["valid_events"]
        );
        for dim in ["P", "A", "C", "E"] {
            assert_eq!(
                as_int["pace_vector"][dim], as_float["pace_vector"][dim],
                "同一置信度写成 `85` 与 `85.0` 竟得出不同的 {dim} 维 ⇒ 有一种型别被判成了缺席"
            );
        }
        assert_ne!(
            as_int["pace_vector"]["C"], absent["pace_vector"]["C"],
            "负控命中：整数 `85` 的结果与「根本不给置信度」一致 ⇒ `num_of` 桥失效，整数置信度又被读成 0.5"
        );
    }

    /// v128（B1）：主链**否决**按所选档收紧，且 ① 只升不降 ② 增量必须可归因。
    ///
    /// 四段一组，缺任一段都能假绿（v129 把 v128 的「只有否决按档」扩成整条主链按档）：
    ///   A 基线（四格与全局节点档全 unit）⇒ 不得出现 R-211 ——
    ///     `risk_rank(unit)` 的默认档是 1（中风险），不加 `present()` 守卫就会把「没接到」
    ///     读成「该档是中风险」，在低风险标的上凭空抬一档（放大器）。
    ///   B 归因基准缺失（只给本档格、不给全局节点档）⇒ **必须不加严、不留痕**：
    ///     两套规则体系（主链 V54 算法 vs prompt 规则）之间没有可归因的差值，
    ///     拿它们直接比就是我这条更正要挡的形态。
    ///   C 同体系内更严（本档 > 全局节点档）⇒ 必须留痕 R-211，且否决入参确实换了（源码锁）。
    ///   D 更松（本档 ≤ 全局节点档）⇒ action 与基线**逐字相同**且无 R-211 ——
    ///     这是「按档不能成为放松通道」唯一的检出点。
    #[test]
    fn v129_tier_risk_scopes_the_whole_main_chain_one_way_only() {
        let src = PRODUCTION_SCRIPTS
            .iter()
            .find(|(l, _)| *l == "portfolio-mgr")
            .expect("脚本清单缺 portfolio-mgr")
            .1;

        let trail_ids = |out: &serde_json::Value| -> Vec<String> {
            // ⚠ 键名是 **snake** `decision_trail`（产出点 `portfolio-mgr.rhai:3291`，
            //   catch 兜底那份 :3441 也是同名）。首版按 camel 读 `decisionTrail` ⇒ 每次都是空数组：
            //   三条「不该留痕」的断言**恒真**，只有「必须留痕」那条会红 —— 而这正是本测试
            //   自己必须非空的意义：它同时是另外三条负控的**读面自证**。
            out["decision_trail"]
                .as_array()
                .map(|a| {
                    a.iter().filter_map(|e| e["rule_id"].as_str().map(str::to_string)).collect()
                })
                .unwrap_or_default()
        };
        let has = |out: &serde_json::Value, id: &str| trail_ids(out).iter().any(|x| x == id);

        let baseline = run_portfolio_mgr_fully(src, &[], &[]);
        // B：只有本档格，没有同体系基准
        let no_baseline = run_portfolio_mgr_fully(
            src,
            &[],
            &[("overall_risk_mid", serde_json::json!("极高风险"))],
        );
        // C：本档 > 全局节点档（差值只可能来自深度臂）
        let strict = run_portfolio_mgr_fully(
            src,
            &[],
            &[
                ("overall_risk_llm", serde_json::json!("中风险")),
                ("overall_risk_mid", serde_json::json!("极高风险")),
            ],
        );
        // D：同体系内不比全局更严 ⇒ 即便比主链自算档更严也不加严
        let same_as_global = run_portfolio_mgr_fully(
            src,
            &[],
            &[
                ("overall_risk_llm", serde_json::json!("极高风险")),
                ("overall_risk_mid", serde_json::json!("极高风险")),
            ],
        );
        // E：本档更松
        let mild = run_portfolio_mgr_fully(
            src,
            &[],
            &[
                ("overall_risk_llm", serde_json::json!("中风险")),
                ("overall_risk_mid", serde_json::json!("低风险")),
            ],
        );

        for (name, out) in [
            ("A 基线（全缺席）", &baseline),
            ("B 缺同体系基准", &no_baseline),
            ("D 本档与全局同档", &same_as_global),
            ("E 本档更松", &mild),
        ] {
            assert!(!has(out, "R-211"), "{name} 不该加严却留了 R-211: {:?}", trail_ids(out));
        }
        assert_eq!(
            baseline["action"], mild["action"],
            "所选档更松时结论必须逐字不变 —— 按档不是放松风险的通道"
        );
        assert!(
            has(&strict, "R-211"),
            "同体系内本档更严必须留痕（R-211）: {:?}",
            trail_ids(&strict)
        );

        // 行为面（仅当基线 action 落在极高风险否决的处理档时可观测）
        let base_action = baseline["action"].as_str().unwrap_or_default();
        if matches!(base_action, "买入" | "增持" | "持有") {
            assert_ne!(
                strict["action"], baseline["action"],
                "基线 action={base_action} 属极高风险否决的处理档，却未被降级 ⇒ `pm_risk_veto` 仍在读全局档"
            );
        }

        // 源码锁（v129 翻向）：否决实参回到 `overall_risk`，而**按档发生在 `overall_risk` 自己身上**
        // —— v128 那两条断言（必须传 `risk_for_veto`、不得传 `overall_risk`）在本版逐条反过来，
        // 保留旧写法就是给「只有否决一处按档」的形态背书。
        assert_eq!(
            src.matches("pm_risk_veto(final_action, overall_risk)").count(),
            1,
            "portfolio-mgr.rhai 应恰有一处 `pm_risk_veto(final_action, overall_risk)`"
        );
        // ⚠ 另两条同判据的锁（`risk_for_veto` 不得复活 / 按档值必须落进 `overall_risk` 本体）
        //   **不在这里查**：本文件拿不到 `code_only()`，不剥注释就 contains 必然假红
        //   （首轮实测就红在退役注释上 —— 提到退役变量名是合法散文）。
        //   代码域那两条住在
        //   `seed_consistency_tests::main_chain_risk_grade_is_tier_scoped_in_code_domain`
        //   —— 门的存放位置由可用基础设施决定，判据只有一位主人。
    }
}
