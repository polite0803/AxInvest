// SPDX-License-Identifier: AGPL-3.0-only

//! Rhai AST 全局缓存（单例）。
//!
//! 避免重复编译静态脚本（如 portfolio-mgr.rhai / data-quality.rhai 等）。
//! 在批量股票分析场景下，N 只股票会触发 N 次工作流执行，
//! 若每次都重新编译 1373 行的 portfolio-mgr.rhai，会造成显著的 CPU 浪费。
//!
//! # 设计
//!
//! - 全局 `OnceLock<RwLock<HashMap<String, Arc<AST>>>>`，进程级单例
//! - key = `SHA256(code)` 前 16 hex 字符 **+ 解析期配置指纹**（见 [`parse_config_fingerprint`]）
//! - 编译时需要一个 `&Engine` 引用（调用 `engine.compile(code)`）
//! - 缓存的 AST 可被任何 Engine **执行**（`engine.eval_ast_with_scope(&ast)`），
//!   **执行期**绑定（函数查找、`max_operations` 等）在 eval 时按当时的 Engine 决定，
//!   与 AST 本身无关
//!
//! ⚠️ **上面这条不适用于解析期参数** —— 这曾是本模块一个真实的静默缺陷（2026-09-21 修）：
//! 早期注释写成「缓存的 AST 可被任何 Engine 执行 …… 与 AST 本身无关」，读起来像
//! 「Engine 配置与缓存无关」。但 `max_expr_depths` / `optimization_level` 是**解析期**的，
//! 它们决定**这份 AST 能不能编出来、编成什么样**。缓存键若只含 code hash，
//! 则同一份脚本被不同深度上限的引擎编译时会**共用一个桶** ⇒ 先跑者的宽松配置
//! 会静默短路后跑者自己的严格限制，且故障形态**取决于调用顺序**。
//! `crates/rt-workflow/.../code_executor.rs:55` 用 `(1024, 1024)`，
//! 而 `src/commands/stock_analysis.rs:199` 与 `stock_workflow/decision.rs:2693`
//! 用 `(256, 256)` —— 两条路径编译同一份 `portfolio-mgr.rhai`，正是该形态。
//! - AST 实现 Send + Sync（rhai 开启 sync feature），可安全跨线程共享
//!
//! # 线程安全
//!
//! 使用 `std::sync::RwLock`（非 `tokio::sync::RwLock`），因为：
//! 1. 读写操作不跨 await（编译是同步操作）
//! 2. 主要在 `spawn_blocking` 上下文中调用
//! 3. 锁持有时间极短（HashMap 查找或单次插入）
//!
//! AGENTS.md 禁区第 8 条针对"跨 await 持有锁"的场景，本模块不属此列。

// SAFETY: 本文件的 std::sync 锁仅在同步临界区使用，guard 不跨 await（无死锁 / 毒化风险）。
// [2026-09-03] 由 crate 级 disallowed_types 豁免局部化到具体触发点（不含字面量，便于 grep 审计）。
#![allow(clippy::disallowed_types)]

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use rhai::AST;

/// 全局 AST 缓存（进程级单例）。
///
/// key = code 的 SHA256 短 hash，value = 编译后的 AST（Arc 共享）。
fn ast_cache() -> &'static RwLock<HashMap<String, Arc<AST>>> {
    static CACHE: OnceLock<RwLock<HashMap<String, Arc<AST>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 计算 code 的短 hash（SHA256 前 8 字节 = 16 hex 字符），用作缓存 key。
///
/// 用 SHA256 而非 DefaultHasher：DefaultHasher 每次进程启动结果不同
/// （RandomState 种子随机），无法跨进程复用；SHA256 确定性更高，便于诊断。
fn code_hash(code: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..8])
}

/// 解析期配置指纹 —— 决定「同一份 code 在两个 Engine 下编译出的 AST 是否可互换」。
///
/// # 为什么要进缓存键（2026-09-21 修的静默缺陷）
///
/// 缓存的 AST 是**编译产物**，其形态由「源码 + **解析期**配置」共同决定。
/// 只把 code hash 当键，等于假设「同一份源码在任何引擎下都编译出等价 AST」——
/// 该假设对**执行期**配置成立（函数注册、`max_operations` 在 `eval_ast` 时按当时
/// 的 Engine 查），对**解析期**配置**不成立**。
///
/// 实证：本仓三处生产调用点，两套深度配置
/// （`stock_analysis.rs:199` / `stock_workflow/decision.rs:2693` = `(256, 256)`；
/// `rt-workflow/.../code_executor.rs:55` = `(1024, 1024)`）编译**同一份**
/// `portfolio-mgr.rhai`。键不含配置时它们共用一个桶：
/// **哪条路径先跑，缓存里那份 AST 就带先跑者的上限**，后跑者命中后自己的限制被绕过。
///
/// # 收了哪些旋钮 —— rhai 1.26 全部 `set_*` 的分类
///
/// | 分类 | 旋钮 | 入键 |
/// |---|---|---|
/// | **解析期**（决定能否编过 / 报错行号） | `set_max_expr_depths(a, b)` | ✅ 两个 getter 都收 |
/// | **解析期**（`api/compile.rs:207` 把 level 传进编译链 ⇒ **改写 AST 内容**） | `set_optimization_level` | ✅ |
/// | 执行期（`api/limits.rs` 只在 eval 时查） | `set_max_operations` / `set_max_call_levels` / `set_max_variables` / `set_max_functions` / `set_max_modules` / `set_max_string_size` / `set_max_array_size` / `set_max_map_size` | ❌ 入键只会让缓存无谓分裂 |
/// | 解析期，但**本仓零调用点** | `set_allow_looping` / `set_allow_if_expr` / `set_allow_switch` / `set_allow_statement_expr` / `set_disabled_symbols` / `set_strict_variables` / `set_fast_operators` | ⚠️ 见下方「回流提醒」 |
///
/// **回流提醒**：若将来任何调用点开始设置上表最后一行的旋钮，**必须同步扩本指纹**，
/// 否则缺陷原样复发。判据：`grep -rn "set_allow_looping\|set_disabled_symbols\|
/// set_strict_variables\|set_fast_operators\|set_optimization_level" src crates`
/// 的结果里若出现本仓调用点，先扩指纹再放行。
///
/// getter 可用性：`max_expr_depth()`/`max_function_expr_depth()` 由 rhai 的 `api::limits` 模块提供，
/// `optimization_level()` 由 `api::optimize` 提供；本仓 rhai feature 组合为
/// `sync` + `serde`（`Cargo.toml:72`，无 `unchecked`/`no_function`/`no_optimize`），
/// 三者均可用。若将来启用 `no_optimize`，`optimization_level()` 会消失 ⇒
/// **编译期报错**（响亮失败，非静默），届时改指纹实现即可。
fn parse_config_fingerprint(engine: &rhai::Engine) -> String {
    format!(
        "expr={}/fnexpr={}/opt={:?}",
        engine.max_expr_depth(),
        engine.max_function_expr_depth(),
        engine.optimization_level()
    )
}

/// 获取或编译 AST。
///
/// - `cache_key`：脚本逻辑名（如 "portfolio-mgr"、"data-quality"），仅用于日志诊断
/// - `code`：Rhai 脚本源码
/// - `engine`：用于编译的 Engine（需注册脚本依赖的函数）
///
/// 返回 `Arc<AST>`，可被任何 Engine 执行（函数在 eval_ast 时按 Engine 查找）。
///
/// # 线程安全
///
/// 内部用 RwLock，读多写少。编译只在首次或 code 变化时发生。
/// 锁不跨 await，使用 std::sync::RwLock 即可。
///
/// # 性能
///
/// - 缓存命中：1 次 RwLock 读锁 + 1 次 HashMap 查找 + 1 次 Arc clone
/// - 缓存未命中：1 次 RwLock 读锁（未命中）+ 1 次 engine.compile + 1 次 RwLock 写锁 + 1 次 HashMap insert
///
/// 在批量分析 100 只股票的场景下，portfolio-mgr.rhai（1373 行）只编译 1 次，
/// 后续 99 次命中缓存，节省约 99 × ~5ms = ~500ms 的编译开销。
pub fn get_or_compile_ast(
    cache_key: &str,
    code: &str,
    engine: &rhai::Engine,
) -> Result<Arc<AST>, String> {
    let hash = code_hash(code);
    // 键必须同时含「源码」与「解析期配置」：两者任一变化，编译产物就不可互换。
    let parse_fp = parse_config_fingerprint(engine);
    let bucket = format!("{hash}|{parse_fp}");

    // 先尝试读缓存（读锁，允许多线程并发读）
    {
        let cache = ast_cache().read().unwrap_or_else(|e| e.into_inner());
        if let Some(ast) = cache.get(&bucket) {
            tracing::debug!(
                cache_key = cache_key,
                hash = %hash,
                parse_config = %parse_fp,
                "[harness::rhai_ast_cache] 缓存命中"
            );
            return Ok(ast.clone());
        }
    }

    // 缓存未命中，编译 AST（编译是 CPU 密集操作，但不持锁）
    tracing::info!(
        cache_key = cache_key,
        hash = %hash,
        parse_config = %parse_fp,
        code_len = code.len(),
        "[harness::rhai_ast_cache] 缓存未命中，编译 AST"
    );
    let ast = engine
        .compile(code)
        .map_err(|e| format!("Rhai 编译失败 (cache_key={cache_key}, hash={hash}): {e}"))?;
    let ast = Arc::new(ast);

    // 写入缓存（写锁，短暂持有）
    {
        let mut cache = ast_cache().write().unwrap_or_else(|e| e.into_inner());
        // Double-check：其他线程可能已在此期间编译并写入
        if let Some(existing) = cache.get(&bucket) {
            return Ok(existing.clone());
        }
        cache.insert(bucket, ast.clone());
    }

    Ok(ast)
}

/// 清除所有缓存的 AST。
///
/// 仅供测试使用。生产环境中 AST 缓存是进程级单例，无需清除
/// （code 变化时会自动产生新 key，旧条目不再命中）。
#[cfg(test)]
pub fn clear_cache() {
    let mut cache = ast_cache().write().unwrap_or_else(|e| e.into_inner());
    cache.clear();
}

/// 返回当前缓存条目数（用于诊断/测试）。
pub fn cache_size() -> usize {
    let cache = ast_cache().read().unwrap_or_else(|e| e.into_inner());
    cache.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // 测试串行锁：全局 AST 缓存是进程级单例，并行测试会互相干扰。
    // 用 Mutex 保证同一时间只有一个测试操作缓存。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn cache_hit_avoids_recompile() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        let mut engine = rhai::Engine::new();
        crate::register_common_functions(&mut engine);
        let code = "let x = 1 + 2; x";

        // 首次编译
        let ast1 = get_or_compile_ast("test", code, &engine).unwrap();
        assert_eq!(cache_size(), 1);

        // 第二次应命中缓存（Arc 指针相等）
        let ast2 = get_or_compile_ast("test", code, &engine).unwrap();
        assert_eq!(cache_size(), 1);
        assert!(Arc::ptr_eq(&ast1, &ast2));
    }

    #[test]
    fn code_change_invalidates_cache() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        let mut engine = rhai::Engine::new();
        crate::register_common_functions(&mut engine);

        let code1 = "let x = 1; x";
        let code2 = "let x = 2; x";

        let ast1 = get_or_compile_ast("test", code1, &engine).unwrap();
        let ast2 = get_or_compile_ast("test", code2, &engine).unwrap();

        // code 不同 → 两个缓存条目
        assert_eq!(cache_size(), 2);
        // AST 指针不同
        assert!(!Arc::ptr_eq(&ast1, &ast2));
    }

    #[test]
    fn same_code_different_cache_key_shares_ast() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // ⚠ 区分两个「key」：
        //   - `cache_key` 形参 = 脚本**逻辑名**（"portfolio-mgr" 等），只进日志，从不参与分桶；
        //   - 缓存**桶键** = code hash + 解析期配置指纹。
        // 本测试判的是前者：同一份 code、同一套解析期配置，换个逻辑名调用 ⇒ 必须共用同一 AST。
        clear_cache();
        let mut engine = rhai::Engine::new();
        crate::register_common_functions(&mut engine);

        let code = "let x = 42; x";
        let ast1 = get_or_compile_ast("script-a", code, &engine).unwrap();
        let ast2 = get_or_compile_ast("script-b", code, &engine).unwrap();

        // 相同 code → 只有一个缓存条目
        assert_eq!(cache_size(), 1);
        // Arc 指针相等（共享 AST）
        assert!(Arc::ptr_eq(&ast1, &ast2));
    }

    #[test]
    fn cached_ast_executes_correctly() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 验证缓存的 AST 能被任意 Engine 正确执行
        clear_cache();
        let mut engine = rhai::Engine::new();
        crate::register_common_functions(&mut engine);

        let code = "clamp(15.0, 0.0, 10.0)";
        let ast = get_or_compile_ast("clamp-test", code, &engine).unwrap();

        // 用另一个 Engine 执行缓存的 AST
        let mut engine2 = rhai::Engine::new();
        crate::register_common_functions(&mut engine2);
        let mut scope = rhai::Scope::new();
        let result: f64 = engine2.eval_ast_with_scope(&mut scope, &ast).expect("AST 执行失败");
        assert_eq!(result, 10.0);
    }

    // ─────────────────────────────────────────────────────────────────────
    // 解析期配置必须进缓存键（2026-09-21 修的静默缺陷）
    //
    // 缺陷形态：键只含 code hash ⇒ 三处生产引擎（两套深度配置 256 / 1024）编译
    // 同一份 `portfolio-mgr.rhai` 时共用一个桶，**先跑者的配置决定缓存里那份 AST**，
    // 后跑者命中后自己的限制被静默绕过，且故障取决于按钮点击顺序。
    // ─────────────────────────────────────────────────────────────────────

    /// 缺陷的**原症状**：严格配置不得借用宽松配置编译出来的 AST。
    #[test]
    fn strict_parse_limits_do_not_borrow_loose_cached_ast() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();

        // 夹具：12 层嵌套数组字面量。**深度是量出来的，不是猜的** ——
        // 零锁探针（`output/tmp-rhai-depth-probe2.rs`，rhai 1.26）实测最小可用
        // `max_expr_depths` = 39（nested_array(12)）/ 83（parens(40)）/
        // 123（nested_array(40)）：三者都受顶层深度限制，取最短的 nested_array(12)。
        let code = format!("let x = {}1{}; x.len()", "[".repeat(12), "]".repeat(12));

        let mut loose = rhai::Engine::new();
        loose.set_max_expr_depths(64, 64);
        let mut strict = rhai::Engine::new();
        strict.set_max_expr_depths(8, 8);

        // 前提自证：夹具在严格配置下**确实**编不过。若这条先失败，说明夹具选错了
        // （换更深的），而不是缓存有问题 —— 避免本测试退化成恒绿。
        assert!(
            strict.compile(&code).is_err(),
            "夹具前提不成立：{code} 在 (8,8) 下能编过 ⇒ 本测试无区分力，需换更深的夹具"
        );

        // ① 宽松配置先编译并占住缓存
        let loose_ast = get_or_compile_ast("loose-first", &code, &loose).expect("宽松配置应能编过");

        // ② 严格配置必须**自己失败**，而不是命中宽松那份（修复前走的正是后者）
        assert!(
            get_or_compile_ast("strict-second", &code, &strict).is_err(),
            "严格配置 (8,8) 拿到了宽松配置 (64,64) 编译的 AST ⇒ 解析期限制被静默绕过"
        );

        // ③ 宽松那份没被污染，且仍是同一个对象；失败路径不新增桶
        assert_eq!(cache_size(), 1, "严格配置编译失败 ⇒ 不应新增缓存桶");
        let again = get_or_compile_ast("loose-again", &code, &loose).unwrap();
        assert!(Arc::ptr_eq(&loose_ast, &again), "同配置再取应命中同一 AST");
    }

    /// 宽度覆盖：两个**解析期**旋钮各自都要能把桶分开；同配置则必须共用（反向对照）。
    #[test]
    fn different_parse_config_uses_distinct_cache_buckets() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_cache();
        // 用最简 code：本测试判的是**桶的划分**，不需要夹具本身有深度敏感性
        let code = "let x = 1 + 2; x";

        let default_engine = rhai::Engine::new();
        let mut deep = rhai::Engine::new();
        deep.set_max_expr_depths(1024, 1024);
        let mut unopt = rhai::Engine::new();
        unopt.set_optimization_level(rhai::OptimizationLevel::None);

        let ast_default = get_or_compile_ast("default", code, &default_engine).unwrap();
        let ast_deep = get_or_compile_ast("deep", code, &deep).unwrap();
        let ast_unopt = get_or_compile_ast("unopt", code, &unopt).unwrap();

        assert!(
            !Arc::ptr_eq(&ast_default, &ast_deep),
            "深度上限 (32,16) vs (1024,1024) ⇒ 编译产物不可互换，不得共用 AST"
        );
        assert!(
            !Arc::ptr_eq(&ast_default, &ast_unopt),
            "优化级别 Simple vs None ⇒ 产出 AST 内容不同（api/compile.rs:207 把 level 传进编译链），不得共用"
        );
        assert_eq!(cache_size(), 3, "三种解析期配置应各占一个桶");

        // 反向对照：配置相同的新 Engine 实例 ⇒ 必须命中同一 AST。
        // 没有这条，把随机数/时间戳混进键也能让上面三条通过（缓存被切碎 = 假修复）。
        let mut same_as_default = rhai::Engine::new();
        crate::register_common_functions(&mut same_as_default);
        let ast_same = get_or_compile_ast("default-again", code, &same_as_default).unwrap();
        assert!(
            Arc::ptr_eq(&ast_default, &ast_same),
            "解析期配置相同（注册的函数不参与解析期）⇒ 应命中同一 AST；若这里失败说明键切得过细"
        );
        assert_eq!(cache_size(), 3, "配置相同不得新增桶");
    }
}
