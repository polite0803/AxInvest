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
//!
//! 定位器自己也有两条自证（2026-10-08 补）：① 本函数是文件最后一个顶层 item 时窗口照样定得了
//! （现网形态，旧判据正因它而从没绿过）；② 收尾 `}` 被摘掉 ⇒ 窗口越界吃兄弟 item ⇒ 必须红。

/// 取 `run_single_stock_analysis` 函数体（窗口失效即红，不静默变宽/变窄）。
///
/// ⚠ 窗口终点**只用本函数自己的边界**，不用「后面还有没有别的顶层函数」：
/// `run_single_stock_analysis` 恰是 `core.rs` 里最后一个顶层 `pub fn` ⇒ 旧写法（找下一个
/// `\npub async fn ` / `\npub fn `）在这份文件上永远定不了终点，本门**从落地那天（`507a4af69`）
/// 就没绿过** —— 那批跑的是 `--lib`，集成测试目录不在覆盖面里。
/// 新终点 = 第一个列 0 的收尾 `}`（rustfmt 对顶层 item 保证），并显式拒绝吞掉任何兄弟 item。
fn single_stock_window(core_src: &str) -> &str {
    let start = core_src
        .find("pub async fn run_single_stock_analysis(")
        .expect("core.rs 里应能定位 run_single_stock_analysis（改名/搬迁须同步本门）");
    let rest = &core_src[start..];
    let end =
        rest.find("\n}").expect("找不到本函数列 0 的收尾 `}` ⇒ 函数体形态/缩进已变，须同步本门");
    let win = &rest[..end + 2];
    assert!(
        !win[1..].contains("\npub async fn ") && !win[1..].contains("\npub fn "),
        "窗口吞掉了后面的顶层 item ⇒ 边界判据失效（会把兄弟函数里的字面量当本函数的落库形态）"
    );
    win
}

/// 代际写入语句的字面量。**带尾逗号**不是随手写的：本函数里有一条注释
/// （`core.rs:2177`）逐字抄了同一个表达式但没有逗号 ⇒ 少了逗号判据就会被注释喂成「已写代际」，
/// 负控也就失效（正是本仓登记的「注释里抄一遍常量名原本算已筛」那一族伪装）。
const PERSIST_WRITE: &str = "template_version: Set(Some(loaded.version)),";

/// 该通道的落库 update 是否写了代际。
fn single_stock_persist_sets_version(win: &str) -> bool {
    // 只认 `Set(Some(loaded.version))`：`loaded` 是本函数 `load_and_inject_template` 的返回值，
    // 即「本轮实际执行的那一代模板」。写死数字 / 写 `Set(Some(125))` 都不算数（换代即失真）。
    win.contains(PERSIST_WRITE)
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
    let n_write = win.matches(PERSIST_WRITE).count();
    assert_eq!(
        n_write, 1,
        "本函数的代际写入点从 1 处变成 {n_write} 处 ⇒ 落库面积变了，须重新数（多一个 update 分支\
         漏写代际就是又一类「有结论、无代际」）"
    );
    let mutated = win.replace(PERSIST_WRITE, "");
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

/// 定位器自证 ①：**本函数是文件里最后一个顶层 item** 时窗口照样定得了。
///
/// 这条就是 `core.rs` 的现网形态（也正是旧判据从此没绿过的原因），把合成样本单独钉住，
/// 免得将来有人「为了让老判据通过」往文件尾补一个空函数 —— 那是改被锁对象来骗过检法。
#[test]
fn window_locator_works_when_function_is_the_last_item() {
    let src =
        "pub async fn other() {}\npub async fn run_single_stock_analysis() {\n    let a = 1;\n}\n";
    let win = single_stock_window(src);
    assert_eq!(
        win, "pub async fn run_single_stock_analysis() {\n    let a = 1;\n}",
        "窗口边界不对（应正好到本函数列 0 的收尾 `}}`）"
    );
}

/// 定位器自证 ②（负控）：收尾 `}` 被摘掉 ⇒ 窗口会越界吃掉后面的兄弟 item ⇒ **必须红**。
#[should_panic(expected = "窗口吞掉了后面的顶层 item")]
#[test]
fn window_locator_rejects_swallowing_a_sibling_item() {
    let src = "pub async fn run_single_stock_analysis() {\n    let a = 1;\n\
               pub async fn sibling() {\n    let b = 2;\n}\n";
    let win = single_stock_window(src);
    assert!(win.contains("let b = 2"), "负控没走到断言，判据没电");
}
