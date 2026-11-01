// SPDX-License-Identifier: AGPL-3.0-only
//! PostgreSQL 侧检索分词器 `ax_cjk_ngram(text)` 的**唯一定义处**。
//!
//! ## 为什么这个模块独立存在（而不是留在迁移里）
//!
//! 本定义原先住在 `migrations/v227_cjk_fts.rs` 里，与「把生成列重建为 n-gram 版」
//! 这个**一次性修复动作**混在一起。这两件事的生命周期不同：
//!
//! - **函数定义是长期契约**：声明式引擎（`reconcile::extras::FUNCTIONS`）在全新库上
//!   必须能独立建出它；索引侧（生成列）与查询侧（`hybrid_search` 构造 `tsquery`）
//!   都依赖它。
//! - **重建生成列是一次性数据修复**：只对「迁移时代建出来的旧表达式生成列」有意义，
//!   全新库的生成列从一开始就是 n-gram 版本，不需要重建。
//!
//! 把它留在 `migrations/` 会造成一个**编译期硬耦合**：`reconcile` 引用
//! `crate::migrations::v227_cjk_fts`，于是「删掉 migrations 目录」这件事做不成。
//! 本模块把契约提出来，让两侧共用同一份定义 ——
//! **不另抄一份**（两份会各自腐烂，属于「手抄常量多副本」反模式）。
//!
//! ## 三层一致性的分工
//!
//! | 角色 | 位置 |
//! |---|---|
//! | 语义规范（Rust 实现，唯一权威） | `crates/search/src/text_ngram.rs` |
//! | SQL 侧实现（本模块渲染的函数体） | `src/sql/ax_cjk_ngram.sql` |
//! | 双向锁定 | 共享 fixture `crates/search/tests/fixtures/ngram_cases.json`，Rust 侧由 `tests/ngram_consistency.rs` 校验，PG 侧由 `scripts/check-ngram-consistency.mjs` 校验 |
//!
//! ⚠ 改 `CJK_CLASS` / `SEP_CLASS` **必须**同步改 `crates/search/src/text_ngram.rs` 的
//! `is_cjk` / `is_separator`，否则索引侧与查询侧会**静默失配**（索引里有、查询里取不到，
//! 双方都不报错）。反之亦然。

/// CJK 字符类**范围片段**（不带方括号），与
/// `crates/search/src/text_ngram.rs::is_cjk` 逐范围一一对应。
///
/// ⚠️ 修改此处必须同步修改 Rust 侧 `is_cjk`，否则索引侧与查询侧会静默失配
/// （索引里有、查询里取不到，且双方都不报错）。反之亦然。
pub const CJK_CLASS: &str = concat!(
    r"\u3400-\u4DBF",         // CJK 统一表意文字扩展 A
    r"\u4E00-\u9FFF",         // CJK 统一表意文字（基本区）
    r"\uF900-\uFAFF",         // CJK 兼容表意文字
    r"\u3040-\u30FF",         // 平假名 + 片假名
    r"\uAC00-\uD7AF",         // 谚文音节
    r"\U00020000-\U0002A6DF", // CJK 扩展 B
    r"\U0002A700-\U0002EBEF", // CJK 扩展 C/D/E/F
);

/// 分隔符字符类**范围片段**（不带方括号），与
/// `crates/search/src/text_ngram.rs::is_separator` 逐范围一一对应。
///
/// 采用显式范围列举而非 `[[:punct:]]` 之类的 locale 相关类别：后者在 PG 里
/// 依赖 `lc_ctype`，与 Rust 的 Unicode 判定无法保证一致，而两侧不一致的后果是
/// 静默漏分词。显式列举是唯一能同时满足「两侧严格同构」与「locale 无关」的形式。
///
/// ⚠️ 范围里刻意**不含**全角字母数字（`０-９` U+FF10-19、`Ａ-Ｚ` U+FF21-3A、
/// `ａ-ｚ` U+FF41-5A）与 Latin-1 字母区（`À-ÿ`）—— 那些是文字，必须作为 token 保留。
pub const SEP_CLASS: &str = concat!(
    r"\s",            // 空白
    r"\u0021-\u002F", // ASCII ! " # $ % & ' ( ) * + , - . /
    r"\u003A-\u0040", // ASCII : ; < = > ? @
    r"\u005B-\u0060", // ASCII [ \ ] ^ _ `
    r"\u007B-\u007E", // ASCII { | } ~
    r"\u00A0-\u00BF", // Latin-1 标点与符号（不含字母）
    r"\u00D7\u00F7",  // × ÷
    r"\u2000-\u206F", // 通用标点（en/em dash、省略号、弯引号等）
    r"\u2190-\u2BFF", // 箭头 / 数学运算符 / 几何图形 / 杂项符号
    r"\u3000-\u303F", // CJK 标点（。、「」等）
    r"\uFE30-\uFE4F", // CJK 兼容形式（竖排标点）
    r"\uFF01-\uFF0F", // 全角 ！＂＃＄％＆＇（）＊＋，－．／
    r"\uFF1A-\uFF20", // 全角 ：；＜＝＞？＠
    r"\uFF3B-\uFF40", // 全角 ［＼］＾＿｀
    r"\uFF5B-\uFF65", // 全角 ｛｜｝～｟｠｡｢｣､･
    r"\uFFE0-\uFFE6", // 全角货币符号
);

/// `ax_cjk_ngram()` 的建函数 SQL 模板（含 `__CJK__` / `__SEP__` 占位符）。
///
/// 模板里两个字符类各出现 3 次，内联会让「改了这处忘了那处」变成必然
/// （而正则类不一致的后果是静默漏分词，不会报错），故用占位符注入。
const FUNCTION_SQL_TEMPLATE: &str = include_str!("sql/ax_cjk_ngram.sql");

/// 渲染建函数 SQL：把两个字符类注入模板。
///
/// 抽成 `pub` 是为了让一致性校验脚本能取到**与运行时完全相同**的 SQL ——
/// 若脚本另行拼装一份，就又多了一处可能漂移的定义。
///
/// 消费者（两处，都是「建函数」语义，但生命周期不同）：
/// 1. `reconcile::extras::FUNCTIONS` —— 声明式引擎在**全新库**上建函数；
/// 2. `migrations::v227_cjk_fts::up` —— 存量库上的一次性修复（重建生成列）。
pub fn render_function_sql() -> String {
    FUNCTION_SQL_TEMPLATE.replace("__CJK__", CJK_CLASS).replace("__SEP__", SEP_CLASS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 占位符必须被**全部**替换 —— 若模板新增了占位符而这里忘了注入，
    /// 产出的 SQL 会带着 `__XXX__` 直接发给 PG（建函数报语法错，或在更坏的情况下
    /// 建出一个正则类为字面量 `__XXX__` 的函数：**不报错但永远匹配不到中文**）。
    #[test]
    fn render_leaves_no_placeholder_behind() {
        let sql = render_function_sql();
        for ph in ["__CJK__", "__SEP__"] {
            assert!(!sql.contains(ph), "渲染后仍残留占位符 {ph}：模板新增了占位符却没在此注入");
        }
        assert!(
            sql.contains("CREATE OR REPLACE FUNCTION ax_cjk_ngram"),
            "模板疑似被换成了别的东西"
        );
    }

    /// 两个字符类的**内容**必须真的进了 SQL（不是被空串替换掉）。
    #[test]
    fn render_injects_both_classes() {
        let sql = render_function_sql();
        assert!(sql.contains(CJK_CLASS), "CJK_CLASS 未注入");
        assert!(sql.contains(SEP_CLASS), "SEP_CLASS 未注入");
        assert!(!CJK_CLASS.is_empty() && !SEP_CLASS.is_empty(), "字符类被清空");
    }
}
