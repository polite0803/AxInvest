// SPDX-License-Identifier: AGPL-3.0-only

//! DDL 微解析 —— 双方言共用的**低层**解析原语。
//!
//! ## 为什么值得单独成模块
//!
//! 这是整个引擎里**唯一会「静默给错答案」的地方**：PG 的 `pg_get_indexdef` 与
//! SQLite 的 `sqlite_master.sql` 都只能拿到 **DDL 文本**（`pg_index` 的列结构在
//! `sea-query` 里是 `pub(crate)`，`TableIndex` 无公开 getter —— 实测见 PLAN §六）。
//! 解析错一位，引擎就会把一个「其实一致」的索引判为不一致并**反复重建**，
//! 或者把一个「其实不一致」的索引判为一致而漏改。两者都不报错。
//!
//! ## 三条纪律
//!
//! 1. **按配平括号扫描，不按 `lastIndexOf('(')`** —— P2 施工时正是踩了这个坑：
//!    partial 索引 `... (feedback) WHERE (feedback IS NOT NULL)` 用「最后一个左括号」
//!    取列集，会把解析锚点落到 WHERE 的括号里，产出 `cols: ["feedback IS NOT NULL"]`
//!    这种语法上就不成立的声明（判据 #367）。
//! 2. **引号内的一切原样保留** —— `('it''s')` 里的逗号不是分隔符，`'('` 不是括号。
//! 3. **`Group` 的下标是相对「喂给 `first_group_after` 的那个字符串」的** ——
//!    取文本时必须把**同一个**字符串传回 `Group::inner`。把相对 `after_on` 的下标
//!    拿去索引 `head` 会让列集整体左移若干字节，产出的列名语法上都不成立，
//!    却**不报错**（P3 实测：`["tus ON public.index_jobs USING bt"]`）。
//! 4. **解析失败必须能被上层察觉** —— 全部返回 `Option`，绝不返回「尽力而为的默认值」。

/// 一对配平括号的下标（闭区间，含 `(` 与 `)`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub open: usize,
    pub close: usize,
}

impl Group {
    /// 括号内的文本（不含括号本身）。
    pub fn inner<'a>(&self, s: &'a str) -> &'a str {
        &s[self.open + 1..self.close]
    }
}

/// 从 `open_idx`（必须指向 `(`）起扫描到配平的那个 `)`。
///
/// 引号状态机覆盖 SQL 的三种引号：`'…'`（字符串）、`"…"`（标识符，PG 标准）、
/// `` `…` ``（MySQL/SQLite 扩展）。`''` 在字符串内表示转义的引号本身。
pub fn balanced(s: &str, open_idx: usize) -> Option<Group> {
    let bytes = s.as_bytes();
    if bytes.get(open_idx) != Some(&b'(') {
        return None;
    }
    let mut depth: i32 = 0;
    let mut i = open_idx;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            // 引号内：只找配对的收尾引号；`''` 视为转义（跳过下一个）
            Some(q) => {
                if c == q {
                    if bytes.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
            },
            None => match c {
                b'\'' | b'"' | b'`' => quote = Some(c),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Group { open: open_idx, close: i });
                    }
                },
                _ => {},
            },
        }
        i += 1;
    }
    None
}

/// 在 `from` 之后找**第一个** `(` 并返回其配平括号组。
pub fn first_group_after(s: &str, from: usize) -> Option<Group> {
    let rel = s.get(from..)?.find('(')?;
    balanced(s, from + rel)
}

/// 在**顶层**（深度 0、引号外）查找子串，返回其起始下标。
pub fn find_top_level(s: &str, needle: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let nb = needle.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                if c == q {
                    if bytes.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
            },
            None => match c {
                b'\'' | b'"' | b'`' => quote = Some(c),
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {
                    if depth == 0 && bytes[i..].starts_with(nb) {
                        return Some(i);
                    }
                },
            },
        }
        i += 1;
    }
    None
}

/// 按**顶层**逗号切分（括号内与引号内的逗号不算分隔符）。
///
/// 空段被丢弃（`(a, b,)` 这类尾逗号不产生空列）。
pub fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let bytes = s.as_bytes();
    let sep_b = sep as u8;
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth: i32 = 0;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                if c == q {
                    if bytes.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
            },
            None => match c {
                b'\'' | b'"' | b'`' => quote = Some(c),
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ if c == sep_b && depth == 0 => {
                    let seg = s[start..i].trim();
                    if !seg.is_empty() {
                        out.push(seg.to_string());
                    }
                    start = i + 1;
                },
                _ => {},
            },
        }
        i += 1;
    }
    let seg = s[start..].trim();
    if !seg.is_empty() {
        out.push(seg.to_string());
    }
    out
}

/// 去掉标识符外层引号（`"x"` / `` `x` `` / `[x]`），并还原 `""` → `"`。
/// 非引号形态原样返回（PG 只在需要时加引号，故两种都必须支持）。
pub fn unquote_ident(s: &str) -> String {
    let t = s.trim();
    let bytes = t.as_bytes();
    match (bytes.first(), bytes.last()) {
        (Some(b'"'), Some(b'"')) if bytes.len() >= 2 => t[1..t.len() - 1].replace("\"\"", "\""),
        (Some(b'`'), Some(b'`')) if bytes.len() >= 2 => t[1..t.len() - 1].replace("``", "`"),
        (Some(b'['), Some(b']')) if bytes.len() >= 2 => t[1..t.len() - 1].to_string(),
        _ => t.to_string(),
    }
}

/// 解析出的一条索引（双方言统一形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedIndex {
    pub name: String,
    pub table: String,
    /// 原始列项（保留 `DESC` 后缀与表达式原文；未剥引号之外的东西）
    pub cols: Vec<String>,
    pub unique: bool,
    /// PG 的 `USING <method>`；SQLite 恒为 `None`。
    pub method: Option<String>,
    /// 含外括号的谓词（对齐 `pg_get_indexdef` 形态）。
    pub where_clause: Option<String>,
}

/// 解析一条索引 DDL。同时覆盖两种方言：
///
/// ```text
/// PG    ：CREATE [UNIQUE ]INDEX [CONCURRENTLY ]<name> ON <schema>.<table> [USING <m>] (<cols>) [WHERE <pred>]
/// SQLite：CREATE [UNIQUE ]INDEX <name> ON <table> (<cols>) [WHERE <pred>]
/// ```
///
/// 失败（找不到列组/表名）返回 `None` —— **上层必须把它当错误处理**，
/// 不能当「没有索引」。
pub fn parse_index_ddl(sql: &str) -> Option<ParsedIndex> {
    let head_end = find_top_level(sql, " WHERE ").unwrap_or(sql.len());
    let head = &sql[..head_end];
    let tail = &sql[head_end..];

    // ⚠ 锚点必须是**前缀**，不能用 `find("INDEX ")` —— 后者会在索引名自身含
    // `INDEX ` 时取错位置（`CREATE INDEX "idx index a" ON …`）。
    let up = head.to_ascii_uppercase();
    let (unique, after_create) = if up.starts_with("CREATE UNIQUE INDEX ") {
        (true, &head["CREATE UNIQUE INDEX ".len()..])
    } else if up.starts_with("CREATE INDEX ") {
        (false, &head["CREATE INDEX ".len()..])
    } else {
        return None;
    };
    // `CREATE INDEX CONCURRENTLY x ON …`
    let after_index = after_create
        .strip_prefix("CONCURRENTLY ")
        .or_else(|| after_create.strip_prefix("concurrently "))
        .unwrap_or(after_create);

    let on_pos = find_top_level(after_index, " ON ")?;
    let name = unquote_ident(&after_index[..on_pos]);
    let after_on = &after_index[on_pos + " ON ".len()..];

    // 列组 = **本 region 内**的第一个配平括号对。
    //
    // ⚠⚠ `Group` 的下标是**相对 `after_on`** 的 ⇒ 后续一切取文本都必须传
    // `after_on`，不能传 `head`。P3 修掉的正是一个真实缺陷：拿相对 `after_on` 的
    // 下标去索引 `head`（少了 `on_pos + " ON ".len()` 的位移），列集整体左移，
    // 产出 `["tus ON public.index_jobs USING bt"]` 这种语法上不可能的列名 ——
    // 而它**不会报错**，只会让引擎判「索引不一致」并每轮重建。
    let col_group = first_group_after(after_on, 0)?;

    // 表名 = `after_on` 到 `USING <method>`（PG），或到列组开头（SQLite 无 `USING`）。
    // 少了后一个截断，`CREATE UNIQUE INDEX … ON [tbl] (a, b)` 的表名会变成
    // `[tbl] (a, b)`（P3 同步修掉的第二个缺陷）。
    let using_pos = find_top_level(after_on, " USING ");
    let table_end = using_pos.filter(|p| *p < col_group.open).unwrap_or(col_group.open);
    let table_region = &after_on[..table_end];
    // 表名可能带 schema 限定（PG）：取最后一段
    let table =
        unquote_ident(table_region.trim().rsplit('.').next().unwrap_or(table_region.trim()));

    let method = using_pos.and_then(|p| {
        let m = &after_on[p + " USING ".len()..];
        let m = m.split_whitespace().next()?;
        Some(m.to_ascii_lowercase())
    });

    let cols = split_top_level(col_group.inner(after_on), ',')
        .into_iter()
        .map(|c| normalize_index_col(&c))
        .collect();

    let where_clause = if tail.is_empty() {
        None
    } else {
        let pred = tail.trim_start_matches(" WHERE ").trim();
        if pred.is_empty() {
            None
        } else {
            Some(pred.to_string())
        }
    };

    Some(ParsedIndex { name, table, cols, unique, method, where_clause })
}

/// 归一单个索引列项：剥标识符引号，保留 ` DESC` / ` ASC` / `NULLS FIRST` 等后缀。
///
/// 表达式列（`lower(x)`）原样保留 —— 它没有实体侧对应物，保留原文才能让 diff
/// **把它显式暴露出来**而不是静默判等。
fn normalize_index_col(raw: &str) -> String {
    let t = raw.trim().trim_end_matches("::text");
    // 后缀：只识别 ASC/DESC/NULLS，其余（表达式）整体保留
    for suffix in [" DESC", " ASC", " DESC NULLS FIRST", " DESC NULLS LAST"] {
        if let Some(base) = t.strip_suffix(suffix) {
            let base = unquote_ident(base);
            return format!("{base}{suffix}");
        }
    }
    if t.contains('(') {
        // 表达式列：原样（只折空白）
        return t.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    unquote_ident(t)
}

/// [`split_create_table_body`] 的返回类型：`(列定义 (列名, 定义原文), 其余顶层项)`。
///
/// 抽成别名**只为满足 clippy `type_complexity`**（在 `-D warnings` 下是硬门禁），
/// 类型本身完全等价 ⇒ 调用方无需改动。别把它当语义抽象来用。
pub type CreateTableBody = (Vec<(String, String)>, Vec<String>);

/// 解析 `CREATE TABLE` 里**顶层**的列定义与表级约束。
///
/// 返回 `(列定义列表, 其余顶层项)`：
/// - 列定义以「首 token 是标识符」判定；
/// - `PRIMARY KEY` / `UNIQUE` / `FOREIGN KEY` / `CHECK` / `CONSTRAINT` 开头的项归入第二项。
///
/// 用于 SQLite：「列定义列表」取生成列表达式，「其余顶层项」取 CHECK。
pub fn split_create_table_body(create_sql: &str) -> Option<CreateTableBody> {
    let group = first_group_after(create_sql, 0)?;
    let body = group.inner(create_sql);
    let mut cols = Vec::new();
    let mut table_items = Vec::new();
    for item in split_top_level(body, ',') {
        let first = item.split_whitespace().next().unwrap_or("");
        let upper = first.to_ascii_uppercase();
        let is_table_item = matches!(
            upper.as_str(),
            "PRIMARY" | "UNIQUE" | "FOREIGN" | "CHECK" | "CONSTRAINT" | "EXCLUDE"
        );
        if is_table_item {
            table_items.push(item);
        } else {
            let name = unquote_ident(first);
            cols.push((name, item));
        }
    }
    Some((cols, table_items))
}

/// 从表级项里抽出 `CONSTRAINT <name> CHECK (<expr>)` / `CHECK (<expr>)`。
///
/// SQLite 通常**不给** CHECK 约束命名 ⇒ `name` 可能是空串；此时调用方按表达式匹配
/// （见 [`crate::reconcile::model::CheckModel::key`]）。
pub fn parse_check_from_item(item: &str) -> Option<(String, String)> {
    let upper = item.to_ascii_uppercase();
    let name = if upper.trim_start().starts_with("CONSTRAINT ") {
        let rest = item.trim_start()[("CONSTRAINT ".len())..].trim_start();
        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        unquote_ident(&rest[..end])
    } else {
        String::new()
    };
    let ck = find_top_level_ci(item, "CHECK")?;
    let group = first_group_after(item, ck)?;
    let expr = group.inner(item).trim().to_string();
    if expr.is_empty() {
        return None;
    }
    Some((name, expr))
}

/// 大小写不敏感的顶层查找。
fn find_top_level_ci(s: &str, needle: &str) -> Option<usize> {
    let upper_needle = needle.to_ascii_uppercase();
    let mut from = 0usize;
    while let Some(pos) = find_top_level(&s[from..], &upper_needle) {
        // 词边界校验：`CHECK` 前后不能是标识符字符（防 `RECHECKER`）
        let a = from + pos;
        let before_ok =
            a == 0 || !s.as_bytes()[a - 1].is_ascii_alphanumeric() && s.as_bytes()[a - 1] != b'_';
        let after = a + needle.len();
        let after_ok = after >= s.len()
            || (!s.as_bytes()[after].is_ascii_alphanumeric() && s.as_bytes()[after] != b'_');
        if before_ok && after_ok {
            return Some(a);
        }
        from = a + needle.len();
        if from >= s.len() {
            return None;
        }
    }
    None
}

/// 从列定义里抽生成列表达式：`… GENERATED ALWAYS AS (<expr>) [STORED|VIRTUAL]` 或 `… AS (<expr>)`。
pub fn parse_generated_expr(col_def: &str) -> Option<String> {
    let as_pos = find_top_level_ci(col_def, "AS")?;
    let group = first_group_after(col_def, as_pos)?;
    let expr = group.inner(col_def).trim();
    if expr.is_empty() {
        None
    } else {
        Some(expr.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P2 施工时踩过的坑（判据 #367）：partial 索引的谓词括号**不能**被当成列组。
    #[test]
    fn partial_predicate_parens_are_not_columns() {
        let sql = "CREATE INDEX idx_retrieval_hits_feedback ON public.retrieval_hits \
                   USING btree (feedback) WHERE (feedback IS NOT NULL)";
        let p = parse_index_ddl(sql).expect("应解析成功");
        assert_eq!(p.cols, vec!["feedback"], "列集只能是 feedback");
        assert_eq!(p.where_clause.as_deref(), Some("(feedback IS NOT NULL)"));
        assert_eq!(p.method.as_deref(), Some("btree"));
        assert_eq!(p.table, "retrieval_hits");
        assert!(!p.unique);
    }

    /// DESC 方向必须保留：丢掉会让引擎判「索引不一致」并反复重建（PLAN §六 已登记）。
    #[test]
    fn desc_direction_is_preserved() {
        let sql = "CREATE INDEX idx_index_jobs_status ON public.index_jobs \
                   USING btree (status, priority DESC, created_at)";
        let p = parse_index_ddl(sql).unwrap();
        assert_eq!(p.cols, vec!["status", "priority DESC", "created_at"]);
    }

    /// 保留字做列名时 PG 会加双引号：必须剥掉，否则与实体侧 `position` 不相等。
    #[test]
    fn pg_quoted_reserved_word_columns_are_unquoted() {
        let sql = "CREATE INDEX \"idx-t-position\" ON public.\"t\" USING btree (\"position\")";
        let p = parse_index_ddl(sql).unwrap();
        assert_eq!(p.name, "idx-t-position");
        assert_eq!(p.table, "t");
        assert_eq!(p.cols, vec!["position"]);
    }

    /// SQLite 形态：无 `USING`、表名可能被 `[...]`/反引号包裹。
    #[test]
    fn sqlite_style_ddl_parses() {
        let p = parse_index_ddl("CREATE UNIQUE INDEX `uq_a` ON [tbl] (a, b)").unwrap();
        assert_eq!(p.name, "uq_a");
        assert_eq!(p.table, "tbl");
        assert!(p.unique);
        assert_eq!(p.cols, vec!["a", "b"]);
        assert_eq!(p.method, None);
        assert_eq!(p.where_clause, None);
    }

    /// 表达式索引：逗号在括号里，不能当分隔符；表达式原样保留以便显式暴露差异。
    #[test]
    fn expression_index_keeps_commas_inside_parens() {
        let p = parse_index_ddl(
            "CREATE INDEX idx_lower ON public.t USING btree (lower(name), (a + b))",
        )
        .unwrap();
        assert_eq!(p.cols, vec!["lower(name)", "(a + b)"]);
    }

    /// 字符串字面量里的括号与逗号不参与配对/切分。
    #[test]
    fn quotes_shield_parens_and_commas() {
        assert_eq!(split_top_level("a, 'x,(y', b", ','), vec!["a", "'x,(y'", "b"]);
        let g = first_group_after("f(')(') rest", 0).unwrap();
        assert_eq!(g.inner("f(')(') rest"), "')('");
        // '' 是转义引号，不应提前结束字符串
        assert_eq!(split_top_level("'it''s, ok', b", ','), vec!["'it''s, ok'", "b"]);
    }

    /// 解析失败必须是 `None`（上层当错误），不能返回「尽力而为」的空结构。
    #[test]
    fn unparsable_input_returns_none() {
        assert!(parse_index_ddl("CREATE TABLE t (a INT)").is_none());
        assert!(parse_index_ddl("CREATE INDEX idx_no_cols").is_none());
    }

    /// SQLite 建表体拆分：列定义 vs 表级项；表级 `UNIQUE`/`CHECK` 不能混进列。
    #[test]
    fn create_table_body_splits_columns_from_table_items() {
        let sql = "CREATE TABLE t (id TEXT PRIMARY KEY, a INTEGER NOT NULL, \
                   CONSTRAINT ck_a CHECK (a >= 0), UNIQUE (id, a))";
        let (cols, items) = split_create_table_body(sql).unwrap();
        assert_eq!(cols.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["id", "a"]);
        assert_eq!(items.len(), 2, "CONSTRAINT 与 UNIQUE 两项：{items:?}");
        let (name, expr) = parse_check_from_item(&items[0]).unwrap();
        assert_eq!(name, "ck_a");
        assert_eq!(expr, "a >= 0");
    }

    /// 匿名 CHECK（SQLite 常态）：名字为空，表达式仍要拿到。
    #[test]
    fn anonymous_check_yields_empty_name_with_expr() {
        let (name, expr) = parse_check_from_item("CHECK (status IN ('a', 'b'))").unwrap();
        assert_eq!(name, "");
        assert_eq!(expr, "status IN ('a', 'b')");
    }

    /// `RECHECKER` 这类含 CHECK 子串的标识符不能被误判为 CHECK 关键字。
    #[test]
    fn check_keyword_needs_word_boundary() {
        assert!(parse_check_from_item("RECHECKER (x)").is_none());
    }

    /// 生成列表达式抽取（SQLite 的 pragma 不给表达式，只能从 DDL 取）。
    #[test]
    fn generated_column_expr_is_extracted() {
        assert_eq!(
            parse_generated_expr("tsv TEXT GENERATED ALWAYS AS (lower(a)) STORED").as_deref(),
            Some("lower(a)")
        );
        assert_eq!(parse_generated_expr("a TEXT").as_deref(), None);
    }

    /// 配平扫描：嵌套括号必须配对到最外层，而不是第一个 `)`。
    #[test]
    fn balanced_scan_handles_nesting() {
        let s = "f(a, g(b, c), d)";
        let g = first_group_after(s, 0).unwrap();
        assert_eq!(g.close, s.len() - 1);
        assert_eq!(g.inner(s), "a, g(b, c), d");
    }

    /// 表名区域必须止于列组开头（SQLite 无 `USING`）—— 否则 `[tbl] (a, b)` 整段
    /// 被当成表名（P3 实测缺陷之一）。
    #[test]
    fn table_region_stops_at_column_group_without_using() {
        let p = parse_index_ddl("CREATE INDEX i ON tbl (a, b)").unwrap();
        assert_eq!(p.table, "tbl");
        assert_eq!(p.cols, vec!["a", "b"]);
        // 带 schema 限定时取最后一段
        let p = parse_index_ddl("CREATE INDEX i ON public.tbl (a)").unwrap();
        assert_eq!(p.table, "tbl");
    }

    /// 索引名里含 `INDEX ` 不能把锚点带偏（故用前缀匹配而不是 `find("INDEX ")`）。
    #[test]
    fn anchor_is_a_prefix_not_a_substring_search() {
        let p = parse_index_ddl("CREATE INDEX \"idx index a\" ON tbl (a)").unwrap();
        assert_eq!(p.name, "idx index a");
        assert_eq!(p.table, "tbl");
        assert_eq!(p.cols, vec!["a"]);
    }
}
