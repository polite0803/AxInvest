// SPDX-License-Identifier: AGPL-3.0-only
//!
//! 用**共享 fixture** 校验 Rust 侧 ngram 实现符合规范。
//!
//! 同一份 `tests/fixtures/ngram_cases.json` 也被
//! `scripts/check-ngram-consistency.mjs` 用来校验 PostgreSQL 侧的
//! `ax_cjk_ngram()`。两侧若对同一输入产出不同结果，索引与查询会**静默失配**
//! （索引里有、查询里取不到），既不报错也不抛异常，只有这份 fixture 能拦住。
//!
//! 运行：`cargo test -p axagent-search --test ngram_consistency`

use axagent_search::text_ngram::{cjk_ngram, cjk_ngram_query_tokens, is_tsquery_safe_token};
use serde::Deserialize;

/// fixture 里至少要有这么多条用例。
///
/// 设下限而非「≥1」是为了防止 fixture 被误清空、路径写错、JSON 结构变更后
/// 静默变成「0 条用例全部通过」——那种绿是假绿。
const MIN_CASES: usize = 30;

#[derive(Debug, Deserialize)]
struct Case {
    /// 该条用例想锁定的规则/意图（失败时打印，便于定位）。
    note: String,
    input: String,
    expected: String,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

fn load_fixture() -> Fixture {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("ngram_cases.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取共享 fixture 失败 {}: {e}", path.display()));
    let fixture: Fixture = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("解析共享 fixture 失败 {}: {e}", path.display()));
    assert!(
        fixture.cases.len() >= MIN_CASES,
        "fixture 用例数 {} 少于下限 {MIN_CASES}，疑似被清空或路径/结构变更",
        fixture.cases.len()
    );
    fixture
}

#[test]
fn rust_ngram_matches_shared_fixture() {
    let fixture = load_fixture();
    let mut failures: Vec<String> = Vec::new();

    for case in &fixture.cases {
        let actual = cjk_ngram(&case.input);
        if actual != case.expected {
            failures.push(format!(
                "  ── 规则: {}\n     输入: {:?}\n     期望: {:?}\n     实际: {:?}",
                case.note, case.input, case.expected, actual
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Rust 侧 ngram 与共享 fixture 不符：{} / {} 条失败\n{}",
        failures.len(),
        fixture.cases.len(),
        failures.join("\n")
    );
}

/// 索引侧（内容）与查询侧（查询）用同一套切分，只是查询侧去重。
///
/// 若两侧切分规则不同，索引里存在的 token 在查询里取不到 —— 静默零结果。
#[test]
fn query_side_is_index_side_deduplicated() {
    let fixture = load_fixture();

    for case in &fixture.cases {
        let index_tokens: Vec<String> =
            cjk_ngram(&case.input).split(' ').filter(|t| !t.is_empty()).map(String::from).collect();
        let query_tokens = cjk_ngram_query_tokens(&case.input);

        // 查询侧 token 集合必须与索引侧完全一致（仅去掉重复项）
        let mut index_unique: Vec<&String> = index_tokens.iter().collect();
        index_unique.sort();
        index_unique.dedup();
        let mut query_sorted: Vec<&String> = query_tokens.iter().collect();
        query_sorted.sort();

        assert_eq!(
            index_unique, query_sorted,
            "查询侧与索引侧 token 集合不一致（输入 {:?}）",
            case.input
        );

        // 且查询侧确实无重复
        let mut deduped = query_tokens.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            query_tokens.len(),
            "查询 token 必须已去重（输入 {:?}）",
            case.input
        );
    }
}

/// 构造 `to_tsquery` 前每个 token 都必须可安全嵌入单引号。
///
/// 归一化产物本不该含 tsquery 元字符，但若有人改动 `is_separator` 而忘了同步，
/// 攻击者输入就能造成语法错误或语义篡改。此处对 fixture 全量下钉。
#[test]
fn all_fixture_tokens_are_tsquery_safe() {
    let fixture = load_fixture();

    for case in &fixture.cases {
        for token in cjk_ngram_query_tokens(&case.input) {
            assert!(
                is_tsquery_safe_token(&token),
                "输入 {:?} 产出的 token {:?} 含 tsquery 元字符，构造查询前必须拦截",
                case.input,
                token
            );
        }
    }
}
