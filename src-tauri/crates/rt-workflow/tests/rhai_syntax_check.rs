//! 语法编译测试：用 rhai v1.25.0 编译 src/commands 下所有 .rhai 文件。
//!
//! 目的：CI 阶段就一次性捕获所有 Rhai 语法错误（如 `as f64`/`let mut` 等
//! Rust 残留语法），避免运行时才报错导致反复修复。
//!
//! 注意：本测试只编译（engine.compile），不执行。输入变量全部注入空值 (),
//! 因为 compile 阶段不需要变量实际有值——只需要语法合法。
use rhai::Engine;
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
