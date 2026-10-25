// SPDX-License-Identifier: AGPL-3.0-only

//! 回归（R-3c，2026-10-25）：**所有构造 AppState 的路径**都必须让共享 Rhai Engine
//! 拿到 AxInvest 宿主函数集。
//!
//! ## 缺陷本体
//!
//! 宿主函数注册此前只写在 `src/init/services.rs` 的 `start_background_services` 里 ——
//! 那是 **Tauri 应用启动路径**。CLI（`axagent-validate-backfill` /
//! `axagent-batch-rerun`）与无头服务（`server_main`）都直接走
//! `create_app_state`、**不经过**该函数 ⇒ 共享 Engine 里**一个 `pm_*` 都没有**。
//!
//! 实测症状（CLI 跑时间旅行批次，`tt3.log`）：
//! ```text
//! ERROR code_executor: Rhai 执行失败 error=Function not found: pm_asof_degraded_methods () (line 863, position 24)
//! WARN  engine: Node failed node_id=data-quality error=VALIDATION_FAILED: …
//! ```
//!
//! `pm_asof_degraded_methods` 是**零参数**闭包 ⇒ 与签名无关 ⇒ 只有一种解释：
//! **整个宿主函数集没进 Engine**。后果不是报错而是**静默降级** ——
//! 脚本里的 `pm_*` 调用被 `try/catch` 吞掉后走保守兜底
//! ⇒ 回放分析链测的是**降级版系统**的决策。
//!
//! ## 判据形态
//!
//! **真调用**（不是查注册表、不是 compile）：共享 Engine 上直接 `eval` 一个
//! `pm_*` 调用，看它能否解析。
//! ⚠ 本文件**必须独占一个 test binary**：`shared_rhai_engine()` 是进程级
//! `OnceLock`，一旦被同 binary 的其它测试先取用，注册就再无机会生效
//! （同 `shared_engine_initializer_registry.rs` 的纪律）。

use axagent_rt_workflow::work_engine::executors::shared_rhai_engine;

/// 断言共享 Engine 上**至少这几个**宿主函数可解析。
///
/// 选这几个的理由：
/// - `pm_asof_degraded_methods`：**零参数**闭包 ⇒ 与签名无关，
///   它能解析就证明「整个函数集进了 Engine」（缺陷本体正是它报 not found）。
/// - `pm_leg_signal`：**两参数**（`&str` + `Map`）⇒ 覆盖签名解析这一层。
/// - `pm_score_to_signal`：**一参数** `f64` ⇒ 覆盖最简形态。
/// - `pm_evidence_scale`：出现在 `portfolio-mgr.rhai` 的主决策路径上。
///
/// ⚠ 判据用**真调用**而非「查注册表」：Rhai 编译期不校验未知函数名，
/// 只 compile 的门禁看不见「函数未注册」这类缺陷
/// （这正是本缺陷能逃过既有静态门禁的原因）。
#[test]
fn shared_engine_resolves_axinvest_host_functions() {
    // 先触发注册（幂等入口）。生产里这一步在 `create_app_state` 内完成，
    // 本测试直接调它以验证「该入口确实把函数送进了共享 Engine」。
    //
    // ⚠ 走 lib 层的**再导出**（`axagent_lib::ensure_shared_engine_has_axinvest_functions`）：
    // `commands` 是私有模块，集成测试无法走 `axagent_lib::commands::…` 路径。
    axagent_lib::ensure_shared_engine_has_axinvest_functions();

    let engine = shared_rhai_engine();

    // ① 零参数形态（缺陷本体就是它报 not found）。
    let degraded: String = engine
        .eval("pm_asof_degraded_methods()")
        .expect("pm_asof_degraded_methods 必须可解析 —— 整个宿主函数集没进 Engine");
    assert!(degraded.starts_with('['), "应返回 JSON 数组字符串，实际 {degraded}");

    // ② 一参数形态（i64 重载：`totalScore` 来自 JSON 数值 ⇒ Rhai 侧是 INT）。
    // `score_to_signal(72)` = (72 − 50) / 50 = **0.44**（`portfolio_formula.rs:33`）
    let sig: f64 = engine.eval("pm_score_to_signal(72)").expect("pm_score_to_signal 必须可解析");
    assert_eq!(sig, 0.44, "score_to_signal(72) 应为 0.44");

    // ③ 两参数形态（字符串 + map）—— 覆盖 `pm_leg_signal` 的签名解析。
    // ⚠ 期望值按 `analysis-engine::leg_signal` 的**真实阶梯**算：
    //   macdDif(1.2) > macdDea(0.6) ⇒ base = +0.7（零上多头）
    //   rsi(60.0) ∈ (50, 75] ⇒ **健康调制 +0.2**
    //   ⇒ 合计 **+0.9**（不是 +0.7 —— 漏掉 RSI 调制就会写出一个看着合理但错的期望值）
    let leg: rhai::Dynamic = engine
        .eval(r#"pm_leg_signal("momentumSignal", #{ macdDif: 1.2, macdDea: 0.6, rsi: 60.0 })"#)
        .expect("pm_leg_signal 必须可解析");
    assert!(
        leg.is::<f64>(),
        "momentumSignal（macdDif > macdDea 且 rsi 健康）应返回 f64，实际 {:?}",
        leg.type_name()
    );
    // 0.7 + 0.2 在 f64 下是 0.8999999999999999 ⇒ 用容差，不写等值断言
    let leg_v = leg.cast::<f64>();
    assert!((leg_v - 0.9).abs() < 1e-9, "多头 +0.7，RSI 健康再 +0.2 = +0.9，实际 {leg_v}");

    // ④ 主决策路径上的函数。
    // `compute_evidence_scale(1.0, 1.0)` = 0.10 + 0.45 × min(1.0,1.0) = **0.55**
    // （`portfolio_formula.rs:162`）⇒ 断言精确值而非「有限」——
    // 「有限」只能证明没崩，证明不了走对了分支。
    let scale: f64 =
        engine.eval("pm_evidence_scale(1.0, 1.0)").expect("pm_evidence_scale 必须可解析");
    assert_eq!(scale, 0.55, "evidence_scale(1.0, 1.0) 应为 0.55");
    // 负控形态：total_weight < 0.3 ⇒ 保守档 0.20（另一条分支）
    let scale_low: f64 =
        engine.eval("pm_evidence_scale(0.1, 1.0)").expect("pm_evidence_scale 必须可解析");
    assert_eq!(scale_low, 0.20, "total_weight < 0.3 ⇒ 保守档 0.20");

    // ── 负对照：证明上面四条断言有区分力 ──
    // 少了这条，若「所有 eval 都成功但返回默认值」也可能假绿。
    assert!(
        engine.eval::<rhai::Dynamic>("pm_never_registered_probe_xyz()").is_err(),
        "负对照失败：未注册的函数竟可解析 ⇒ 上面断言没有区分力"
    );
}
