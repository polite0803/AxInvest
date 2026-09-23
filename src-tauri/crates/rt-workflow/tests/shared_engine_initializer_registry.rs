// SPDX-License-Identifier: AGPL-3.0-only

//! 共享 Rhai Engine **注册通道**的行为测试。
//!
//! 背景（2026-09-22 缺陷）：该通道原为单槽 `OnceLock<EngineInitializer>`（容量 1），
//! `src/init/services.rs` 先后两次注册 ⇒ 第二次被静默丢弃（只打一条 WARN，且文案把
//! 根因误报为「Engine 已初始化」），`bottleneck_node_score` 从未进入共享 Engine。
//!
//! ⚠ 本文件必须**独占一个 test binary**（`tests/` 下每个文件是独立二进制）：
//! `shared_rhai_engine()` 是进程级 OnceLock，一旦被同 binary 的其它测试先取用，
//! 本文件的注册就不再有机会生效。同时两个断言必须写在**同一个 `#[test]` 内**、
//! 按「先注册 → 再冻结 → 再验证」顺序执行 —— 拆成两个测试会被并行调度，
//! 后者的 `shared_rhai_engine()` 会抢先把 Engine 冻结，前者注册即失败。

use axagent_rt_workflow::work_engine::executors::{
    RegisterInitializerError, register_shared_engine_initializer, shared_rhai_engine,
};

#[test]
fn registration_channel_applies_every_registration_then_hard_fails() {
    // ── ① 两个注册器都必须生效（单槽实现下第二个会丢失）──────────────
    register_shared_engine_initializer(Box::new(|engine| {
        engine.register_fn("probe_first_registry_entry", || 11_i64);
    }))
    .expect("首次注册不应失败");

    register_shared_engine_initializer(Box::new(|engine| {
        engine.register_fn("probe_second_registry_entry", || 22_i64);
    }))
    .expect("第二次注册**必须**成功 —— 单槽实现会在此静默丢弃（本测试的回归点）");

    let engine = shared_rhai_engine();

    // 判据锚定「调用真的能解析」，而非「compile 通过」：Rhai 编译期不校验未知
    // 函数名，只 compile 的门禁看不见「函数未注册」这类缺陷（这正是本缺陷能逃过
    // rhai_syntax_check 的原因）。
    let first: i64 = engine.eval("probe_first_registry_entry()").expect("第一个注册器应已生效");
    let second: i64 = engine.eval("probe_second_registry_entry()").expect("第二个注册器应已生效");
    assert_eq!((first, second), (11, 22));

    // 负对照：证明上面两条断言有区分力 —— 未注册的名字必须解析失败。
    assert!(
        engine.eval::<i64>("probe_never_registered()").is_err(),
        "负对照失败：未注册的函数竟可解析 ⇒ 上面两条断言没有区分力"
    );

    // ── ② 冻结之后注册必须返回 Err，而不是静默丢弃 ────────────────
    let err = register_shared_engine_initializer(Box::new(|engine| {
        engine.register_fn("probe_too_late_registry_entry", || 33_i64);
    }))
    .expect_err("Engine 冻结后注册必须返回 Err —— 静默丢弃正是本次缺陷的本质");

    assert!(matches!(err, RegisterInitializerError::AlreadyFrozen));

    // 不能只信返回值：该函数必须**确实**没进 Engine。
    assert!(
        shared_rhai_engine().eval::<i64>("probe_too_late_registry_entry()").is_err(),
        "注册返回了 Err，但函数却在 Engine 上可解析 ⇒ 冻结判定与 Engine 实际状态不一致"
    );
}
