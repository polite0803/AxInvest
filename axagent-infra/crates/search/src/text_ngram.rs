// SPDX-License-Identifier: AGPL-3.0-only
//! 中文友好的检索 n-gram 归一化（全文索引与查询的**单一真源**）。
//!
//! ## 为什么需要它
//!
//! 生产库实测（PostgreSQL 18.4）：
//!
//! ```sql
//! SELECT to_tsvector('simple', '向量索引实现方案');
//! -- => '向量索引实现方案':1        ← 整句一个 token
//! SELECT to_tsvector('simple','向量索引实现方案') @@ plainto_tsquery('simple','向量');
//! -- => false                       ← 中文检索完全不可用
//! SELECT to_tsvector('simple','向量 索引 实现');
//! -- => '向量':1 '索引':2 '实现':3   ← 空格是唯一的分词边界
//! ```
//!
//! PG 内置 parser 对 CJK 的规则是「连续 CJK 字符视为一个单词」，而中文没有词间空格
//! ⇒ 整段中文塌缩成一个词元，子串查询永不命中。专用中文分词器（`zhparser` /
//! `pg_jieba`）在本环境**不可安装**（`pg_available_extensions` 中不存在）。
//! 因此唯一可行路径是**在索引与查询两侧对文本做同一套 n-gram 归一化**，把它变成
//! 空格分隔的 token 串；此时 PG 的 `simple` 配置不再承担分词，只承担 lowercase，
//! 语义完全可控。
//!
//! ## 归一化规则
//!
//! 遍历字符，分三类：
//!
//! | 类别 | 判定 | 处理 |
//! |---|---|---|
//! | CJK | [`is_cjk`] | 累积成 CJK 段 |
//! | 分隔符 | [`is_separator`] | 结束当前段，自身不产出 token |
//! | 其他（字母/数字/任何非分隔字符） | 其余全部 | 累积成 word 段 |
//!
//! 段产出方式：
//! - **CJK 段**：逐字输出**单字** token，再输出相邻 **2-gram** token
//! - **word 段**：整段作为一个 token
//!
//! 例：`"使用pgvector实现"` → `"使 用 使用 pgvector 实 现 实现"`
//!
//! ### 为什么「其他」是兜底而非只用 ASCII 字母数字
//!
//! 若把 word 段限定为 ASCII `[A-Za-z0-9]`，则俄语 / 希腊语等**有空格分词**的文字
//! 会全部落入分隔符，变成不可检索 —— 而它们在改造前（`to_tsvector('simple')` 直吃原文）
//! 是**可用**的。那将是一次功能回退。故 word 段取「非 CJK 且非分隔符」的兜底语义，
//! 与改造前能力对齐（有空格的自然语言照常整词索引）。
//!
//! ### 为什么单字与 2-gram 都要
//!
//! - 只要 2-gram：内容中**孤立的单个汉字**（如标题 `"股"`）无法被任何 bigram 表示，
//!   该记录永久不可检索。
//! - 只要单字：`"股票"` 会命中 `"股东名单"`（含"股"）之类无关内容，精度崩塌。
//! - 两者都给：精度由查询侧的**组合语义**（`OR` + `ts_rank` 排序）决定，
//!   归一化层不替它做取舍。
//!
//! ## 刻意不做的事
//!
//! - **不做大小写转换**：交给消费侧（PG `simple` 配置、SQLite `unicode61` 均内建
//!   lowercase）。保持与 PG 侧 SQL 函数 `ax_cjk_ngram()` 严格同构，便于逐字节比对。
//! - **不做 token 去重**：保留重复以保留**词频**（`ts_rank` 依赖词频），去重会把它
//!   退化成"命中 token 个数"。tsvector 内部自会去重词元。查询侧另见
//!   [`cjk_ngram_query_tokens`]。
//! - **不做停用词过滤**：中文停用词表会让"的/了/是"这类查询静默零结果，
//!   而它们在短查询里恰恰可能是有效信息。
//!
//! ## 与 SQL 侧的契约
//!
//! 本文件是语义规范。PG 侧的 `ax_cjk_ngram()`（定义在 `dao/src/cjk_ngram.rs` +
//! `dao/src/sql/ax_cjk_ngram.sql` —— 2026-09-16 前在迁移 `v227_cjk_fts`，
//! 该迁移已随 74 个迁移文件一起删除）必须产出
//! **逐字节相同**的结果 —— 索引与查询两侧若有任何分歧，会静默失配（索引里有、
//! 查询里取不到），且不报错。两侧的一致性由
//! `crates/search/tests/ngram_consistency.rs` + `scripts/check-ngram-consistency.mjs`
//! 用同一份 fixture（`crates/search/tests/fixtures/ngram_cases.tsv`）锁定。
//!
//! ⚠️ 修改 [`is_cjk`] 或 [`is_separator`] 的范围时，**必须同步修改 SQL 文件中的两处
//! 字符类**（`ax_cjk_ngram()` 内的段提取正则与 CJK 判定正则），否则一致性检查会失败。

use std::collections::HashSet;

/// 判断字符是否属于「无词间空格」的文字：CJK 表意文字、假名、谚文。
///
/// 这些文字必须由 n-gram 切分，否则整段会塌缩成单个词元。
///
/// ⚠️ 与 SQL 侧 `ax_cjk_ngram()` 的 CJK 字符类逐范围对应，改动需同步。
pub fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3400..=0x4DBF     // CJK 统一表意文字扩展 A
        | 0x4E00..=0x9FFF   // CJK 统一表意文字（基本区）
        | 0xF900..=0xFAFF   // CJK 兼容表意文字
        | 0x3040..=0x30FF   // 平假名 + 片假名
        | 0xAC00..=0xD7AF   // 谚文音节
        | 0x20000..=0x2A6DF // CJK 扩展 B
        | 0x2A700..=0x2EBEF // CJK 扩展 C/D/E/F
    )
}

/// 判断字符是否应作为**段分隔符**（自身不产出 token）。
///
/// 采用**显式范围列举**而非「ASCII 标点」或依赖 locale 的 Unicode 类别判定：
/// 该范围必须在 PostgreSQL 中可被等价表达，且不受 DB 的 `lc_ctype` 影响。
/// 显式列举是唯一能同时满足「两侧严格同构」与「locale 无关」的形式。
///
/// ⚠️ 与 SQL 侧 `ax_cjk_ngram()` 的分隔符字符类逐范围对应，改动需同步。
pub fn is_separator(ch: char) -> bool {
    if ch.is_whitespace() {
        return true;
    }
    matches!(ch as u32,
        // ── ASCII 标点与符号 ──
        0x0021..=0x002F   // ! " # $ % & ' ( ) * + , - . /
        | 0x003A..=0x0040 // : ; < = > ? @
        | 0x005B..=0x0060 // [ \ ] ^ _ `
        | 0x007B..=0x007E // { | } ~
        // ── Latin-1 标点与符号（刻意不含 À-ÿ 字母区）──
        | 0x00A0..=0x00BF // NBSP ¡ ¢ £ ¤ ¥ ¦ § ¨ © ª « ¬  ® ¯ ° ± ² ³ ´ µ ¶ · ¸ ¹ º » ¼ ½ ¾ ¿
        | 0x00D7          // ×
        | 0x00F7          // ÷
        // ── 通用标点 ──
        | 0x2000..=0x206F // en/em dash、省略号、弯引号、上下标数字等
        // ── 箭头 / 数学运算符 / 几何图形 / 杂项符号 ──
        // 金融与技术文本高频：≥ ≤ ≠ ≈ ± → ↑ ↓ △ ▲ ★ 等。
        // 归入分隔符可让 "ROE≥15%" 切出可独立检索的 "ROE" 与 "15"。
        | 0x2190..=0x2BFF
        // ── CJK 标点 ──
        | 0x3000..=0x303F // 　、。〈〉《》「」『』【】〜
        // ── CJK 兼容形式（竖排标点等）──
        | 0xFE30..=0xFE4F
        // ── 全角标点与符号 ──
        // 刻意避开全角字母数字（０-９ 0xFF10-19、Ａ-Ｚ 0xFF21-3A、ａ-ｚ 0xFF41-5A），
        // 那些应作为 word 保留。
        | 0xFF01..=0xFF0F // ！＂＃＄％＆＇（）＊＋，－．／
        | 0xFF1A..=0xFF20 // ：；＜＝＞？＠
        | 0xFF3B..=0xFF40 // ［＼］＾＿｀
        | 0xFF5B..=0xFF65 // ｛｜｝～｟｠｡｢｣､･
        | 0xFFE0..=0xFFE6 // ￠￡￤￥￦￧
    )
}

/// 把一段连续 CJK 字符展开为「单字 + 相邻 2-gram」，追加进 `out`。
///
/// 单字构成一个长度 1 的段时只产出该单字（`"股"` → `"股"`）。
fn expand_cjk_run(run: &[char], out: &mut Vec<String>) {
    if run.is_empty() {
        return;
    }
    for ch in run {
        out.push(ch.to_string());
    }
    for pair in run.windows(2) {
        out.push(format!("{}{}", pair[0], pair[1]));
    }
}

/// 把文本归一化为空格分隔的检索 token 串。
///
/// 该串同时用于**索引侧**（喂给 `to_tsvector`）与**查询侧**（构造 `tsquery`），
/// 两侧必须逐字节一致，否则会静默失配。
///
/// ```
/// # use axagent_search::text_ngram::cjk_ngram;
/// assert_eq!(cjk_ngram("向量索引"), "向 量 索 引 向量 量索 索引");
/// assert_eq!(cjk_ngram("股票 600519"), "股 票 股票 600519");
/// // 标点作分隔符，不产出 token
/// assert_eq!(cjk_ngram("你好，世界"), "你 好 你好 世 界 世界");
/// // 有空格分词的语言保持整词（不因改造而回退）
/// assert_eq!(cjk_ngram("привет мир"), "привет мир");
/// ```
pub fn cjk_ngram(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut cjk_run: Vec<char> = Vec::new();
    let mut word = String::new();

    /// 结束 CJK 段与 word 段，把已累积的内容产出。
    macro_rules! flush {
        () => {
            expand_cjk_run(&cjk_run, &mut out);
            cjk_run.clear();
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
        };
    }

    for ch in text.chars() {
        if is_cjk(ch) {
            // CJK 段**继续累积**，只在此结束可能存在的 word 段。
            //
            // ⚠️ 不要在此 expand cjk_run：那会在每遇到一个 CJK 字符时就把累积
            // 清空，使 run 长度恒为 1，`windows(2)` 永远为空 —— 结果是单字 token
            // 齐全、**bigram 全部消失**。该缺陷不会报错，双字查询会静默全不命中，
            // 只有按规范逐条比对 fixture 才能发现（本文件的大段注释即为此而存）。
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            cjk_run.push(ch);
        } else if is_separator(ch) {
            flush!();
        } else {
            // word 段开始：CJK 段在此结束，需要 expand
            expand_cjk_run(&cjk_run, &mut out);
            cjk_run.clear();
            word.push(ch);
        }
    }
    flush!();

    out.join(" ")
}

/// 把查询文本归一化为**去重**的 token 列表，供构造 `tsquery` 使用。
///
/// 与 [`cjk_ngram`] 的区别仅在去重：查询串重复 token 会构造出重复的 OR 项，
/// 徒增 SQL 长度而不改变语义。内容侧不去重（需保留词频），查询侧去重。
///
/// 产出的 token 只可能是 CJK 字符或非分隔符字符，**不含单引号等 tsquery 元字符**，
/// 因此调用方用单引号包裹构造 `'a' | 'b'` 是安全的。为防御性起见，
/// [`is_tsquery_safe_token`] 提供显式校验。
pub fn cjk_ngram_query_tokens(query: &str) -> Vec<String> {
    let normalized = cjk_ngram(query);
    let mut seen: HashSet<String> = HashSet::new();
    let mut tokens: Vec<String> = Vec::new();
    for token in normalized.split(' ').filter(|t| !t.is_empty()) {
        if seen.insert(token.to_string()) {
            tokens.push(token.to_string());
        }
    }
    tokens
}

/// 检验 token 是否可安全嵌入 `to_tsquery` 的字面量。
///
/// 归一化产物本不该含 tsquery 元字符（`'` `&` `|` `!` `(` `)` `:` `*`），
/// 因为那些字符都在 [`is_separator`] 范围内。但归一化函数若被改动而此处未同步，
/// 攻击者输入即可构造 `to_tsquery` 语法错误乃至语义篡改。故构造查询前显式校验，
/// 不依赖"上游一定正确"的假设。
pub fn is_tsquery_safe_token(token: &str) -> bool {
    !token.is_empty()
        && !token
            .chars()
            .any(|c| matches!(c, '\'' | '&' | '|' | '!' | '(' | ')' | ':' | '*' | '<' | '>'))
}

/// 判断归一化结果是否为空（即文本里没有任何可检索内容，如纯标点）。
pub fn is_empty_ngram(text: &str) -> bool {
    cjk_ngram(text).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_single_run_emits_uni_and_bigrams() {
        assert_eq!(cjk_ngram("向量索引"), "向 量 索 引 向量 量索 索引");
        assert_eq!(cjk_ngram("股票"), "股 票 股票");
    }

    #[test]
    fn single_cjk_char_is_still_searchable() {
        // 孤立单字必须产出 token，否则该记录永久不可检索
        assert_eq!(cjk_ngram("股"), "股");
        assert_eq!(cjk_ngram("好"), "好");
    }

    #[test]
    fn punctuation_splits_runs_and_emits_nothing() {
        // 全角逗号 U+FF0C 是分隔符
        assert_eq!(cjk_ngram("你好，世界"), "你 好 你好 世 界 世界");
        // 标点本身不产出 token
        assert_eq!(cjk_ngram("，。！？"), "");
        assert_eq!(cjk_ngram("   "), "");
    }

    #[test]
    fn latin_word_kept_whole() {
        assert_eq!(cjk_ngram("vector"), "vector");
        assert_eq!(cjk_ngram("pgvector"), "pgvector");
        assert_eq!(cjk_ngram("hello world"), "hello world");
    }

    #[test]
    fn mixed_cjk_latin_splits_at_boundary() {
        assert_eq!(cjk_ngram("使用pgvector实现"), "使 用 使用 pgvector 实 现 实现");
        assert_eq!(cjk_ngram("股票 600519"), "股 票 股票 600519");
        assert_eq!(cjk_ngram("代码600519"), "代 码 代码 600519");
        assert_eq!(cjk_ngram("中文abc中文"), "中 文 中文 abc 中 文 中文");
    }

    #[test]
    fn non_space_delimited_languages_still_indexed() {
        // 改造不得造成功能回退：有空格分词的文字保持整词可检索
        assert_eq!(cjk_ngram("привет мир"), "привет мир");
        assert_eq!(cjk_ngram("中文Привет中文"), "中 文 中文 Привет 中 文 中文");
    }

    #[test]
    fn digits_are_one_token_not_split() {
        assert_eq!(cjk_ngram("600519"), "600519");
        assert_eq!(cjk_ngram("2026-09-12"), "2026 09 12");
    }

    #[test]
    fn underscore_is_separator_like_any_punct() {
        assert_eq!(cjk_ngram("foo_bar"), "foo bar");
    }

    #[test]
    fn empty_and_whitespace_inputs_yield_empty() {
        assert_eq!(cjk_ngram(""), "");
        assert!(is_empty_ngram(""));
        assert!(is_empty_ngram("!!!"));
        assert!(!is_empty_ngram("a"));
    }

    #[test]
    fn query_tokens_are_deduplicated() {
        let tokens = cjk_ngram_query_tokens("股票股票");
        let unique: HashSet<&String> = tokens.iter().collect();
        assert_eq!(tokens.len(), unique.len(), "查询 token 必须无重复");
    }

    #[test]
    fn index_side_keeps_frequency_for_ranking() {
        // 内容侧不去重：重复 token 保留，ts_rank 才有词频可用。
        // 注意输出是「先全部单字、再全部 bigram」的两段式，不是交替输出。
        assert_eq!(cjk_ngram("股票股票"), "股 票 股 票 股票 票股 股票");
    }

    #[test]
    fn japanese_kana_and_hangul_are_cjk() {
        assert!(is_cjk('あ'));
        assert!(is_cjk('ア'));
        assert!(is_cjk('한'));
        assert!(is_cjk('中'));
        assert!(is_cjk('鿿'));
        // 全角字母不是 CJK，也不是分隔符 ⇒ 应进 word 段
        assert!(!is_cjk('Ａ'));
        assert!(!is_separator('Ａ'));
    }

    #[test]
    fn long_run_bigrams_are_contiguous() {
        let out = cjk_ngram("基本面分析");
        let tokens: Vec<&str> = out.split(' ').collect();
        assert_eq!(tokens.len(), 5 + 4); // 5 单字 + 4 bigram
        assert!(tokens.contains(&"基本"));
        assert!(tokens.contains(&"本面"));
        assert!(tokens.contains(&"面分"));
        assert!(tokens.contains(&"分析"));
        assert!(!tokens.contains(&"基面"), "bigram 不可跨字组合");
    }

    #[test]
    fn cjk_boundary_resets_bigrams() {
        // 标点两侧的 bigram 不得跨标点组合
        let out = cjk_ngram("你好，世界");
        assert!(!out.contains("好世"), "bigram 不可跨分隔符");
        let out2 = cjk_ngram("你好世界");
        assert!(out2.contains("好世"), "连续 CJK 段的 bigram 必须相邻");
    }

    #[test]
    fn tsquery_tokens_never_contain_metacharacters() {
        // 归一化产物必须天然远离 tsquery 语法字符
        for probe in ["a'b", "a|b", "a&b", "a(b)", "a:b", "a*b", "a<b>"] {
            let normalized = cjk_ngram(probe);
            for token in normalized.split(' ') {
                assert!(
                    is_tsquery_safe_token(token),
                    "token {token:?} from {probe:?} 含 tsquery 元字符"
                );
            }
        }
    }

    #[test]
    fn safety_check_rejects_raw_metacharacters() {
        assert!(!is_tsquery_safe_token("a'b"));
        assert!(!is_tsquery_safe_token("a|b"));
        assert!(!is_tsquery_safe_token(""));
        assert!(is_tsquery_safe_token("向量"));
        assert!(is_tsquery_safe_token("pgvector"));
    }
}
