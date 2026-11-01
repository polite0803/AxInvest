-- SPDX-License-Identifier: AGPL-3.0-only
--
-- ax_cjk_ngram(text) -> text
--
-- 中文友好的检索 n-gram 归一化（PostgreSQL 侧实现）。
-- 语义规范见 crates/search/src/text_ngram.rs；两侧必须逐字节一致，
-- 否则索引与查询会**静默失配**（索引里有、查询里取不到，不报错）。
--
-- 由 scripts/check-ngram-consistency.mjs 用共享 fixture
-- （crates/search/tests/fixtures/ngram_cases.json）对真实 PG 校验。
--
-- ── 占位符 ──
-- __CJK__ 与 __SEP__ 由调用方（Rust 迁移 / 一致性脚本）替换为**范围片段**，
-- 定义分别在 cjk_ngram.rs 的 CJK_CLASS / SEP_CLASS 常量里（不含方括号，
-- 方括号写在下方正则里）。用占位符而非直接内联，是因为这两个类在本文件里
-- 出现 3 次，内联会让「改了这处忘了那处」变成必然（而正则类不一致的后果是
-- 静默漏分词，不会报错）。
--
-- ── 为什么不用 pg 的内置分词 ──
-- 实测 PG 18.4：to_tsvector('simple','向量索引实现方案') => '向量索引实现方案':1
-- 即连续 CJK 被视为**一个单词**，子串查询永不命中。zhparser / pg_jieba 在本环境
-- 的 pg_available_extensions 中不存在，无法安装。故只能由本项目自己做 n-gram。

CREATE OR REPLACE FUNCTION ax_cjk_ngram(txt text)
RETURNS text
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
AS $ax_cjk_ngram$
    SELECT COALESCE(
        -- 排序键 (seg_ord, kind, tok_ord) 复刻 Rust 侧的输出顺序：
        -- 按段出现顺序；每段内「先全部单字，再全部 bigram」；word 段整段一个 token。
        string_agg(x.tok, ' ' ORDER BY x.seg_ord, x.kind, x.tok_ord),
        ''
    )
    FROM regexp_matches(
             COALESCE(txt, ''),
             '([__CJK__]+|[^__CJK____SEP__]+)',
             'g'
         ) WITH ORDINALITY AS m(arr, seg_ord)
    CROSS JOIN LATERAL (SELECT m.arr[1] AS seg, m.seg_ord AS seg_ord) AS s
    CROSS JOIN LATERAL (
        -- word 段（非 CJK）：整段作为一个 token
        SELECT s.seg_ord, 0 AS kind, 1 AS tok_ord, s.seg AS tok
         WHERE s.seg !~ '^[__CJK__]'
        UNION ALL
        -- CJK 段：全部单字
        SELECT s.seg_ord, 1 AS kind, g.i AS tok_ord, substr(s.seg, g.i, 1) AS tok
          FROM generate_series(1, char_length(s.seg)) AS g(i)
         WHERE s.seg ~ '^[__CJK__]'
        UNION ALL
        -- CJK 段：全部相邻 bigram（长度为 1 的段不产出，generate_series(1,0) 为空集）
        SELECT s.seg_ord, 2 AS kind, g.i AS tok_ord, substr(s.seg, g.i, 2) AS tok
          FROM generate_series(1, GREATEST(char_length(s.seg) - 1, 0)) AS g(i)
         WHERE s.seg ~ '^[__CJK__]'
    ) AS x
$ax_cjk_ngram$;

COMMENT ON FUNCTION ax_cjk_ngram(text) IS
    '检索用 n-gram 归一化：CJK 段输出单字+相邻 bigram，非 CJK 段整词保留，'
    '标点空白作分隔符。索引侧 to_tsvector 与查询侧 to_tsquery 必须使用同一函数，'
    '否则静默失配。规范见 crates/search/src/text_ngram.rs。';
