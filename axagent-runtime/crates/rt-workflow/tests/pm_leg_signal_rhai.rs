//! 行为测试（R-3c，2026-10-25）：`pm_leg_signal` 的**运行期可解析性**。
//!
//! 现象：回放分析链（`analysis_kind='replay'`）在 `template_version=184` 上出现
//! ```
//! portfolio-mgr Rhai 执行异常，已降级为保守决策:
//! ErrorFunctionNotFound: Function not found: pm_leg_signal (&str | ImmutableString | String, map)
//! ```
//! 该行被降级为「数据缺失」⇒ 分析链少一条决策。
//!
//! **为什么静态门禁漏掉它**：`src/commands/stock_workflow/rhai_registry.rs` 的
//! `scripts_only_call_registered_fns` 只查**函数名**是否出现在注册点源码里 ——
//! `pm_leg_signal` 确实注册了 ⇒ 假绿。
//! 运行期是否可解析**取决于实参类型**，静态判据看不到这一层
//! （与 `band_for_score` 那次的教训同族：只查名字会漏签名不匹配）。
//!
//! 本文件是**最小隔离复现**：用裸 `Engine::new()` + 与生产**逐字相同**的注册形态，
//! 跑与 `portfolio-mgr.rhai:1714` **逐字相同**的调用形态。
//! 只依赖 `rhai`（不依赖主 crate 的注册层）⇒ 编译快、可独立验证。
//!
//! ⚠ 纪律：**注册形态与调用形态都必须照抄生产**，否则测的是另一个形态
//! （这正是本缺陷此前没被发现的形态：别处都用变量传字符串，只有生产这一处用字面量）。

use rhai::Engine;

/// 与 `src/commands/stock_workflow/rhai_pm.rs:441` **逐字相同**的注册形态。
///
/// ⚠ 参数类型是 `(&str, rhai::Map)` —— 不是 `Dynamic`、不是 `&str + Dynamic`。
fn register_pm_leg_signal(engine: &mut Engine) {
    engine.register_fn("pm_leg_signal", |factor: &str, inputs: rhai::Map| -> rhai::Dynamic {
        // 生产实现走 `analysis-engine::leg_signal`（serde 口径）；
        // 本测试只需验证**可解析 + 返回非 unit**，故用最小等价逻辑占位。
        // 关键判据在「能不能调到」，不在「算得对不对」。
        if factor == "momentumSignal" && inputs.contains_key("macdDif") {
            rhai::Dynamic::from(0.7_f64)
        } else {
            rhai::Dynamic::UNIT
        }
    });
}

/// `portfolio-mgr.rhai:22` 的脚本内自定义 helper（**逐字一致**）。
///
/// ⚠ 它**不是**宿主函数 ⇒ 必须随调用形态一起带上，否则报的是
/// `Function not found: present`（会把判据指向错误的函数）。
const PRESENT_FN: &str = "fn present(x) { type_of(x) != \"()\" }";

/// 与 `portfolio-mgr.rhai:1714` **逐字相同**的调用形态：
/// 第一个实参是**字符串字面量**，第二个是**map 字面量**（含 `()` 单位值）。
const CALL_LITERAL_FORM: &str = r#"pm_leg_signal(
    "momentumSignal",
    #{
        macdDif: if present(macd_dif) && type_of(macd_dif) == "f64" { macd_dif } else { () },
        macdDea: if present(macd_dea) && type_of(macd_dea) == "f64" { macd_dea } else { () },
        rsi: if present(rsi_14) && type_of(rsi_14) == "f64" { rsi_14 } else { () },
    }
)"#;

/// 与 `portfolio-mgr.rhai:551` / 四份逐档分支脚本的调用形态：
/// 第一个实参是**变量**（从 scope 取出的字符串），第二个是 map 字面量。
const CALL_VARIABLE_FORM: &str = r#"pm_leg_signal(FACTOR, #{ macdDif: 1.2, macdDea: 0.6 })"#;

/// 生产执行三段式：const scope + AST + `eval_ast_with_scope`。
///
/// ⚠ 脚本前置 `PRESENT_FN`：`present` 是脚本内自定义 fn，不带它则报的是
/// `Function not found: present`（判据会指向错误的函数）。
fn eval_with_scope(
    engine: &Engine,
    script: &str,
    scope: &rhai::Scope,
) -> Result<rhai::Dynamic, String> {
    let full = format!("{PRESENT_FN}\n{script}");
    let ast = engine.compile_with_scope(scope, &full).map_err(|e| e.to_string())?;
    engine.eval_ast_with_scope(&mut scope.clone(), &ast).map_err(|e| e.to_string())
}

/// **核心回归**：字符串**字面量**作为第一实参时必须可解析。
///
/// 这是生产的真实形态（`portfolio-mgr.rhai:1714`）⇒ 一旦不可解析，
/// 每次 portfolio-mgr 执行都会抛 `Function not found` ⇒ 整条决策链降级。
#[test]
fn pm_leg_signal_resolves_with_string_literal_first_arg() {
    let mut engine = Engine::new();
    register_pm_leg_signal(&mut engine);

    // scope 里放三个变量，让 `if present(...) && type_of(...) == "f64"` 走 true 分支
    let mut scope = rhai::Scope::new();
    scope.push_constant("macd_dif", rhai::Dynamic::from(1.2_f64));
    scope.push_constant("macd_dea", rhai::Dynamic::from(0.6_f64));
    scope.push_constant("rsi_14", rhai::Dynamic::from(60.0_f64));

    let out = eval_with_scope(&engine, CALL_LITERAL_FORM, &scope)
        .unwrap_or_else(|e| panic!("字面量形态必须可解析：{e}"));
    assert!(out.is::<f64>(), "应返回 f64，实际 {:?}", out.type_name());
    assert_eq!(
        out.cast::<f64>(),
        0.7,
        "应命中 mock 返回的 +0.7（mock 只验**可解析**，不验真实阶梯口径）"
    );
}

/// **负控**：变量形态（`portfolio-mgr.rhai:551` 与四份逐档分支脚本）也必须可解析。
///
/// 若这条也失败 ⇒ 缺陷面比「一处调用点」大得多。
#[test]
fn pm_leg_signal_resolves_with_variable_first_arg() {
    let mut engine = Engine::new();
    register_pm_leg_signal(&mut engine);

    let mut scope = rhai::Scope::new();
    scope.push_constant("FACTOR", rhai::Dynamic::from("momentumSignal".to_string()));

    let out = eval_with_scope(&engine, CALL_VARIABLE_FORM, &scope)
        .unwrap_or_else(|e| panic!("变量形态必须可解析：{e}"));
    assert!(out.is::<f64>(), "应返回 f64，实际 {:?}", out.type_name());
}

/// **负控（区分力）**：**未注册**的同名函数必须报错。
///
/// 少了这条，本文件的「通过」可能只是「Rhai 把所有未解析调用都静默降级成 unit」
/// —— 那正是本缺陷在生产里的伪装形态（`try/catch` 吞掉后静默走保守兜底）。
#[test]
fn unregistered_function_still_errors() {
    let engine = Engine::new();
    let scope = rhai::Scope::new();
    let err = eval_with_scope(&engine, CALL_VARIABLE_FORM, &scope)
        .expect_err("未注册函数必须报错，否则本文件无区分力");
    assert!(
        err.contains("Function not found") || err.contains("not found"),
        "报错应指向函数未找到，实际：{err}"
    );
}
