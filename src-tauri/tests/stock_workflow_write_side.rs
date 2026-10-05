//! 写侧代际（`template_version`）落库形态门
//! —— `PLAN-four-horizon-workflow-alignment.md` §五十一-①（批准于 2026-10-04）。
//!
//! # 为什么锁源码形态而不是跑数据库
//!
//! 分代筛样的前提不是「算法对不对」，而是**每一行决策样本自己说得出它是哪一代**。实测现网
//! `template_version IS NULL` 的 44 行**全是 `analysis_kind = live` 的现役行**（回放/线上都有结论），
//! 而它们来自单股即时分析与每日自动化闭环共用的 `run_single_stock_analysis` —— 该函数的建行占位行
//! 写 `Set(None)`（正确：那时还没有决策），**落库那次 update 却从不补写**，于是「有结论、无代际」。
//! 这类缺陷的三个特征决定了检法：
//!
//! | 特征 | 后果 | 本门怎么检 |
//! |---|---|---|
//! | 编译器不参与 | 少写一个字段是合法的 `..Default::default()` | 逐字读该函数的落库 update 块 |
//! | 跑一次也不报错 | 行照样写进去，只是那列 NULL | 断言「会被当样本的通道必须带代」 |
//! | 读侧无从区分 | 筛样时整批排除 ⇒ 指标假性「样本不足」 | 反向锁：不同名目（交易意图）**不得**被套上版本号 |
//!
//! 最后一行是本计划的一条硬边界（禁区 12）：`crates/analysis-engine/src/trade_intent.rs` 写的是
//! **交易意图行**，不是分析结论 ⇒ 它的 `template_version` 必须保持 NULL；给它套一个分析模板版本号
//! 等于把两个名目焊成一个，比 NULL 更坏。
//!
//! # 负控
//!
//! `single_stock_persist_sets_version` 对「摘掉那一行」的变异副本必须返回 `false` ——
//! 否则本门只是恒真断言（本仓对每道新门的统一要求）。

/// 取 `run_single_stock_analysis` 函数体（窗口失效即红，不静默变宽/变窄）。
fn single_stock_window(core_src: &str) -> &str {
    let start = core_src
        .find("pub async fn run_single_stock_analysis(")
        .expect("core.rs 里应能定位 run_single_stock_analysis（改名/搬迁须同步本门）");
    let rest = &core_src[start..];
    // 下一个顶层 `pub async fn` / `pub fn` 即本函数终点（本文件里它是靠后的函数之一）。
    let next = rest[1..]
        .find("\npub async fn ")
        .or_else(|| rest[1..].find("\npub fn "))
        .map(|i| i + 1)
        .expect("run_single_stock_analysis 之后应还有别的顶层函数，用于给本门定窗口终点");
    &rest[..next]
}

/// 该通道的落库 update 是否写了代际。
fn single_stock_persist_sets_version(win: &str) -> bool {
    // 只认 `Set(Some(loaded.version))`：`loaded` 是本函数 `load_and_inject_template` 的返回值，
    // 即「本轮实际执行的那一代模板」。写死数字 / 写 `Set(Some(125))` 都不算数（换代即失真）。
    win.contains("template_version: Set(Some(loaded.version))")
}

#[test]
fn single_stock_channel_writes_generation_at_persist() {
    let core_src = include_str!("../src/commands/stock_workflow/core.rs");
    let win = single_stock_window(core_src);
    assert!(
        win.contains("horizon_decisions: Set(horizon_decisions.clone())"),
        "窗口没圈到决策落库那次 update（该函数的落库形态已变，须同步本门）"
    );
    assert!(
        single_stock_persist_sets_version(win),
        "单股 / 每日自动化通道产出的 live 决策行不写 `template_version` ⇒ \
         读侧按代筛样只能把它们整批排除，指标假性「样本不足」而真因在写侧（PLAN §五十一-①）"
    );
    // 建行占位行**必须**继续留 NULL（那时决策未产生，写版本就是伪造代际）。
    assert!(
        win.contains("template_version: Set(None),"),
        "run_single_stock_analysis 的 \"running\" 占位行应继续显式 `Set(None)` \
         （决策尚未产生 ⇒ 无代可写）"
    );

    // ── 负控：摘掉代际那一行，判据必须失效 ──
    let mutated = win.replace("template_version: Set(Some(loaded.version)),", "");
    assert_ne!(mutated, win, "负控变异点未命中 —— 落库行的字面量已变，须同步本测试");
    assert!(
        !single_stock_persist_sets_version(&mutated),
        "负控失效：删掉代际写入后判据仍然为真，本门不构成防线"
    );
}

#[test]
fn trade_intent_rows_keep_generation_null() {
    // 名目边界：交易意图行不是分析结论 ⇒ 不得被焊上分析模板版本号（禁区 12）。
    let intent_src = include_str!("../crates/analysis-engine/src/trade_intent.rs");
    assert!(
        !intent_src.contains("template_version: Set(Some("),
        "trade_intent.rs 给交易意图行写了分析代际 ⇒ 两个名目被焊成一个；\
         正确形态是保持 NULL，并在读侧按名目排除（PLAN §五十一-①）"
    );
    let n_null = intent_src.matches("template_version: Set(None)").count();
    assert!(
        n_null >= 5,
        "trade_intent.rs 的 `template_version: Set(None)` 写入点从 5 处变成 {n_null} 处 —— \
         本门「各行建行都不得带代」的面积需重新数（红了先确认是改名/搬迁还是真少了一处）"
    );
}
