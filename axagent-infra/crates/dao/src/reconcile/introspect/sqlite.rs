// SPDX-License-Identifier: AGPL-3.0-only

//! SQLite 实况读取（`sqlite_master` + `pragma_*` 表值函数）。
//!
//! ## 与 PG 侧的关键差异：SQLite **忠实保存**原始 DDL
//!
//! PG 会重写表达式（补 `::type`、补括号，见 `pg.rs` 文档），SQLite 则把
//! `CREATE TABLE` / `CREATE INDEX` 的原文逐字存进 `sqlite_master.sql`。
//! ⇒ **SQLite 侧可以放心解析 DDL**，PG 侧不能（只能按名比身份）。
//!
//! ## 三类必须排除的对象（漏一个 = 引擎误判孤儿并 DROP）
//!
//! | 对象 | 来源 | 排除方式 |
//! |---|---|---|
//! | `sqlite_%` 内部表（`sqlite_sequence` 等） | SQLite 自身 | 名字前缀 |
//! | FTS5 **影子表**（`x_data`/`x_idx`/`x_content`/`x_docsize`/`x_config`） | 虚表自动创建 | 由**虚表名**派生后缀（不能按「名字含 `_data`」模糊匹配，会误伤真表） |
//! | **自动索引**（`sqlite_autoindex_*`） | PK / UNIQUE 约束的产物 | `pragma_index_list.origin` 分流 |
//!
//! ## 生成列：pragma 不给表达式
//!
//! `pragma_table_xinfo.hidden` 只区分「2=VIRTUAL / 3=STORED 生成列」，表达式只存在于
//! `sqlite_master.sql`。解析不到时**返回错误**而不是猜 —— 猜错会让引擎按错误的
//! 表达式重建生成列（数据静默变形）。

use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, DbErr, QueryResult, Statement, Value,
};

use crate::reconcile::extras::Dialect;
use crate::reconcile::introspect::Introspector;
use crate::reconcile::introspect::parse;
use crate::reconcile::model::{
    CheckModel, ColumnModel, FkModel, IndexModel, SchemaModel, canonical_type_actual,
    normalize_default,
};

/// FTS5 影子表的后缀（`CREATE VIRTUAL TABLE x USING fts5(...)` 会连带创建它们）。
const FTS5_SHADOW_SUFFIXES: &[&str] = &["_data", "_idx", "_content", "_docsize", "_config"];

/// `pragma_index_list.origin` 的取值。
const ORIGIN_CREATE_INDEX: &str = "c";
const ORIGIN_UNIQUE_CONSTRAINT: &str = "u";

pub struct SqliteIntrospector;

#[async_trait]
impl Introspector for SqliteIntrospector {
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    async fn read(&self, db: &DatabaseConnection) -> Result<SchemaModel, DbErr> {
        let mut m = SchemaModel::new(Dialect::Sqlite);
        for t in load_tables(db).await? {
            read_table(db, &mut m, &t).await?;
        }
        m.finalize();
        Ok(m)
    }
}

/// `sqlite_master` 里的一张表（或虚表）。
struct RawTable {
    name: String,
    /// `sqlite_master.sql` 原文；`sqlite_master` 对虚表也存原文，故正常为 `Some`。
    sql: Option<String>,
    is_virtual: bool,
}

async fn q(
    db: &DatabaseConnection,
    sql: &str,
    params: Vec<Value>,
) -> Result<Vec<QueryResult>, DbErr> {
    db.query_all_raw(Statement::from_sql_and_values(DbBackend::Sqlite, sql, params)).await
}

/// 只传一个表名参数的查询（`pragma_*` 表值函数统一形态）。
async fn q_table(
    db: &DatabaseConnection,
    sql: &str,
    table: &str,
) -> Result<Vec<QueryResult>, DbErr> {
    q(db, sql, vec![table.to_string().into()]).await
}

/// ① 全部表，排除 `sqlite_%` 与 FTS5 影子表。
async fn load_tables(db: &DatabaseConnection) -> Result<Vec<RawTable>, DbErr> {
    let rows =
        q(db, "SELECT name, sql FROM sqlite_master WHERE type = 'table' ORDER BY name", vec![])
            .await?;

    let mut all: Vec<RawTable> = Vec::new();
    for row in rows {
        let name: String = row.try_get("", "name")?;
        let sql: Option<String> = row.try_get("", "sql")?;
        if name.starts_with("sqlite_") {
            continue;
        }
        let is_virtual = sql.as_deref().is_some_and(|s| {
            s.trim_start().to_ascii_uppercase().starts_with("CREATE VIRTUAL TABLE")
        });
        all.push(RawTable { name, sql, is_virtual });
    }

    // 影子表排除：由**虚表名**派生后缀（模糊匹配会误伤真表）
    let virtual_names: Vec<String> =
        all.iter().filter(|t| t.is_virtual).map(|t| t.name.clone()).collect();
    all.retain(|t| {
        !virtual_names.iter().any(|v| {
            t.name
                .strip_prefix(v.as_str())
                .is_some_and(|suffix| FTS5_SHADOW_SUFFIXES.contains(&suffix))
        })
    });
    Ok(all)
}

/// ② 逐表读取列、索引、外键、CHECK。
async fn read_table(
    db: &DatabaseConnection,
    m: &mut SchemaModel,
    raw: &RawTable,
) -> Result<(), DbErr> {
    // 虚表：结构由 L2 的 `extras::VIRTUAL_TABLES` 定义，此处只登记存在性（0 列）。
    //
    // ⚠ 原文写的是「（L2 `VirtualTableDecl` 认领）」—— **那句话不成立**，已订正：
    // `claim_extra` 只是**指纹成分**（`plan.rs` 对它零消费），且两侧 claim 串格式根本
    // 不同（此处 `virtual_table:<名>`，期望侧 `l2.fts5:<名>|<DDL>`）⇒ 从来没有「认领」。
    // 真正的接线是 `plan.rs::l2_virtual_tables_of`（按声明名豁免）：缺了它，这里建出的
    // 表会被表级判定判成孤儿 ⇒ `DROP TABLE messages_fts`（2026-09-17 实测）。
    if raw.is_virtual {
        m.table_or_insert(&raw.name).claim_extra(format!("virtual_table:{}", raw.name));
        return Ok(());
    }

    let create_sql = raw.sql.clone().unwrap_or_default();
    let body = parse::split_create_table_body(&create_sql);

    read_columns(db, m, raw, body.as_ref()).await?;
    read_indexes(db, m, raw, &create_sql).await?;
    read_foreign_keys(db, m, raw).await?;
    if let Some((_, items)) = &body {
        for item in items {
            if let Some((name, expr)) = parse::parse_check_from_item(item) {
                m.table_or_insert(&raw.name).upsert_check(CheckModel { name, expr });
            }
        }
    }
    Ok(())
}

/// ②-a 列（类型 / 可空 / 默认 / 生成列 / 主键标记）。
async fn read_columns(
    db: &DatabaseConnection,
    m: &mut SchemaModel,
    raw: &RawTable,
    body: Option<&super::parse::CreateTableBody>,
) -> Result<(), DbErr> {
    let rows = q_table(
        db,
        "SELECT cid, name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?)",
        &raw.name,
    )
    .await?;

    // 主键列按 `pk` 序号排序（`pk` 是 1-based 的 PK 内序号，0 = 非主键列）
    let mut pk_cols: Vec<(i64, String)> = Vec::new();
    let mut cols: Vec<ColumnModel> = Vec::new();
    // 主键首列的**声明类型原文**（判 rowid 别名用，见循环后的自增判定）。
    // 用 `//` 而非 `///`：这是语句不是 item，`///` 会被 rustdoc 判为 unused_doc_comments。
    let mut pk_decl: Option<String> = None;

    for row in rows {
        let name: String = row.try_get("", "name")?;
        let decl_type: String = row.try_get("", "type").unwrap_or_default();
        let notnull: i64 = row.try_get("", "notnull").unwrap_or(0);
        let dflt: Option<String> = row.try_get("", "dflt_value").unwrap_or(None);
        let pk: i64 = row.try_get("", "pk").unwrap_or(0);
        let hidden: i64 = row.try_get("", "hidden").unwrap_or(0);

        // SQLite 生成列：hidden = 2(VIRTUAL) / 3(STORED)；表达式只在 DDL 里
        let generated = if hidden == 2 || hidden == 3 {
            let col_def = body
                .and_then(|(cols, _)| cols.iter().find(|(n, _)| n == &name))
                .map(|(_, def)| def.clone());
            let expr = col_def.as_deref().and_then(parse::parse_generated_expr);
            Some(expr.ok_or_else(|| {
                DbErr::Custom(format!(
                    "sqlite introspect: {}.{name} 是生成列（hidden={hidden}）但无法从 \
                     sqlite_master.sql 解析表达式；拒绝臆造（DDL：{})",
                    raw.name,
                    body.map(|(c, _)| format!("{c:?}")).unwrap_or_default()
                ))
            })?)
        } else {
            None
        };

        if pk > 0 {
            pk_cols.push((pk, name.clone()));
            if pk == 1 {
                // 自增判据要的是**声明类型原文**，不是亲和性等价类：`utils` 把 `BOOLEAN`
                // 也归成 `"integer"`，而 SQLite 的 rowid 别名要求声明类型**逐字是 INTEGER**。
                pk_decl = Some(decl_type.trim().to_string());
            }
        }
        cols.push(ColumnModel {
            name,
            sql_type: canonical_type_actual(&decl_type, Dialect::Sqlite),
            nullable: notnull == 0,
            // 生成列没有默认值（与 PG 侧 `attgenerated='s'` 的处理一致）
            default: if generated.is_some() {
                None
            } else {
                dflt.as_deref().map(normalize_default)
            },
            primary_key: pk > 0,
            unique: false,
            generated,
            renamed_from: None,
            auto_increment: false, // 循环内判不出来（要看是不是**单列**主键），循环后统一修
        });
    }

    // SQLite 的「自增」= rowid 别名：**单列主键且声明类型恰为 `INTEGER`**（`AUTOINCREMENT`
    // 关键字只允许出现在这种列上，所以不必单独识别它）。必须在循环**之后**判定 ——
    // 「是不是单列主键」要等所有列读完才知道（复合主键没有 rowid 别名）。
    //
    // ⚠ 未区分 `WITHOUT ROWID` 表（那类表没有 rowid 别名，会误判成自增）。实测本仓迁移里
    // 零出现（`grep -rn "WITHOUT ROWID" src-tauri` 只命中文档注释）。若将来出现，判据要
    // 一并收紧（读 `sqlite_master.sql` / `pragma_table_list.wr`）。
    if pk_cols.len() == 1 {
        let pk_name = pk_cols[0].1.as_str();
        let is_rowid_alias = pk_decl.as_deref().is_some_and(|d| d.eq_ignore_ascii_case("INTEGER"));
        if is_rowid_alias {
            for c in cols.iter_mut() {
                if c.name == pk_name {
                    c.auto_increment = true;
                }
            }
        }
    }

    pk_cols.sort_by_key(|(ord, _)| *ord);
    let t = m.table_or_insert(&raw.name);
    for c in cols {
        t.upsert_column(c);
    }
    t.primary_key = pk_cols.into_iter().map(|(_, n)| n).collect();
    Ok(())
}

/// ②-b 索引 + UNIQUE 约束。
///
/// 分流与 PG 侧严格镜像：`origin='pk'` 跳过（已由主键处理）；`origin='u'` 且单列 ⇒
/// 落到列的 `unique` 标志；多列 ⇒ `IndexModel{unique:true}`；`origin='c'` ⇒ 真索引。
///
/// 约束型索引在 `sqlite_master` 里 `sql IS NULL` ⇒ 名字只能从 `pragma` 拿
/// （`sqlite_autoindex_{表}_{N}`，是**位置名**）。故再回 `CREATE TABLE` 原文里找
/// `CONSTRAINT <名> UNIQUE (...)`；找不到就用空名，由
/// [`IndexModel::key`] 按列集识别。
async fn read_indexes(
    db: &DatabaseConnection,
    m: &mut SchemaModel,
    raw: &RawTable,
    create_sql: &str,
) -> Result<(), DbErr> {
    let rows = q_table(
        db,
        "SELECT name, \"unique\", partial, origin FROM pragma_index_list(?)",
        &raw.name,
    )
    .await?;

    for row in rows {
        let idx_name: String = row.try_get("", "name")?;
        let unique: i64 = row.try_get("", "unique").unwrap_or(0);
        let origin: String = row.try_get("", "origin").unwrap_or_default();

        match origin.as_str() {
            // PK 约束的自动索引：主键已由 read_columns 处理
            "pk" => continue,
            ORIGIN_UNIQUE_CONSTRAINT => {
                // ⚠ `index_cols` 必须**在分流之后**才调用：`pragma_index_xinfo` 对
                // 自动索引（`sqlite_autoindex_*`，由 PK / UNIQUE 约束产生）会返回
                // 含 NULL 名的辅助行 ⇒ 在 `pk` 分支之前无条件调用会让每个带复合主键
                // 或列级 UNIQUE 的表都报「含表达式列」而整库 introspect 失败
                // （P3 实测：`reads_real_sqlite_schema` 等 3 条测试因此红）。
                let cols = index_cols(db, &idx_name).await?;
                if cols.len() == 1 {
                    // 单列 UNIQUE ⇒ 列标志（对齐 L1 `#[sea_orm(unique)]` 与 PG 侧分流）
                    if let Some(t) = m.table_mut(&raw.name)
                        && let Some(c) = t.columns.iter_mut().find(|c| c.name == cols[0])
                    {
                        c.unique = true;
                    }
                    continue;
                }
                let name = named_unique_constraint(create_sql, &cols).unwrap_or_default();
                m.table_or_insert(&raw.name).upsert_index(IndexModel {
                    name,
                    cols,
                    unique: true,
                    method: None,
                    where_clause: None,
                });
            },
            ORIGIN_CREATE_INDEX => {
                // 真索引：`sqlite_master.sql` 是原文，直接解析（含 partial 谓词与
                // 表达式列的原文）⇒ 不走 `pragma_index_xinfo`（它对表达式列只给 NULL）
                let sql = index_sql(db, &idx_name).await?;
                let parsed = sql.as_deref().and_then(parse::parse_index_ddl).ok_or_else(|| {
                    DbErr::Custom(format!(
                        "sqlite introspect: 无法取得/解析索引 {idx_name} 的 DDL，拒绝臆造"
                    ))
                })?;
                m.table_or_insert(&raw.name).upsert_index(IndexModel {
                    name: parsed.name,
                    cols: parsed.cols,
                    unique: unique != 0 || parsed.unique,
                    method: None,
                    where_clause: parsed.where_clause,
                });
            },
            other => {
                return Err(DbErr::Custom(format!(
                    "sqlite introspect: 索引 {idx_name} 的 origin='{other}' 未知（已知 c/u/pk），\
                     拒绝按猜测归类"
                )));
            },
        }
    }
    Ok(())
}

/// 取某索引的**键列**列名（`pragma_index_xinfo`，按 `seqno` 排序）。
///
/// ⚠ `index_xinfo` 与 `index_info` 的关键差异：前者**多返回非键列** —— 既包括末尾
/// 的 rowid 辅助行（`cid=-1`），也包括覆盖索引携带的普通列。这些行的 `key=0`
/// 且 `name` 为 `NULL`。不过滤 `key` 就会把「NULL 名」误判成表达式列并把整库
/// introspect 打红（P3 实测：每个复合主键 / 列级 UNIQUE 都触发）。
///
/// 过滤后剩下的 NULL 名**才是**真表达式列 ⇒ 返回错误：表达式列必须回 DDL 解析才能
/// 拿到原文（本函数只处理约束型索引，调用方对 `origin='c'` 走 DDL 解析），
/// 而拿不到原文时按列名对账必然错。
async fn index_cols(db: &DatabaseConnection, idx: &str) -> Result<Vec<String>, DbErr> {
    let rows = q_table(db, "SELECT seqno, cid, name, key FROM pragma_index_xinfo(?)", idx).await?;
    let mut out: Vec<(i64, String)> = Vec::new();
    for row in rows {
        let seqno: i64 = row.try_get("", "seqno").unwrap_or(0);
        // 缺列时按「是键列」处理 ⇒ 维持 fail-closed（宁可报错也不静默丢列）
        let is_key: i64 = row.try_get("", "key").unwrap_or(1);
        if is_key == 0 {
            continue;
        }
        let name: Option<String> = row.try_get("", "name").unwrap_or(None);
        let Some(name) = name else {
            return Err(DbErr::Custom(format!(
                "sqlite introspect: 索引 {idx} 含表达式列（pragma 只给 NULL），无法按列名对账"
            )));
        };
        out.push((seqno, name));
    }
    out.sort_by_key(|(s, _)| *s);
    Ok(out.into_iter().map(|(_, n)| n).collect())
}

/// 从 `CREATE TABLE` 原文里找 `CONSTRAINT <名> UNIQUE (<列集>)` 的**约束名**。
///
/// 找不到返回 `None`（调用方用空名 ⇒ 由列集识别身份）。
fn named_unique_constraint(create_sql: &str, cols: &[String]) -> Option<String> {
    let (_, items) = parse::split_create_table_body(create_sql)?;
    for item in items {
        let upper = item.trim_start().to_ascii_uppercase();
        if !upper.starts_with("CONSTRAINT ") {
            continue;
        }
        let Some(uq) = find_keyword(&item, "UNIQUE") else { continue };
        let group = parse::first_group_after(&item, uq)?;
        let item_cols: Vec<String> = parse::split_top_level(group.inner(&item), ',')
            .into_iter()
            .map(|c| parse::unquote_ident(&c))
            .collect();
        if item_cols == cols {
            let rest = item.trim_start()[("CONSTRAINT ".len())..].trim_start();
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            return Some(parse::unquote_ident(&rest[..end]));
        }
    }
    None
}

/// 顶层关键字查找（大小写不敏感 + 词边界），供约束名解析使用。
fn find_keyword(s: &str, kw: &str) -> Option<usize> {
    let upper = s.to_ascii_uppercase();
    let mut from = 0usize;
    while let Some(rel) = upper[from..].find(kw) {
        let at = from + rel;
        let before_ok = at == 0
            || !s.as_bytes()[at - 1].is_ascii_alphanumeric() && s.as_bytes()[at - 1] != b'_';
        let after = at + kw.len();
        let after_ok = after >= s.len()
            || (!s.as_bytes()[after].is_ascii_alphanumeric() && s.as_bytes()[after] != b'_');
        if before_ok && after_ok {
            return Some(at);
        }
        from = at + kw.len();
        if from >= s.len() {
            return None;
        }
    }
    None
}

async fn index_sql(db: &DatabaseConnection, idx: &str) -> Result<Option<String>, DbErr> {
    let rows = q(
        db,
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?",
        vec![idx.to_string().into()],
    )
    .await?;
    Ok(rows.first().and_then(|r| r.try_get::<Option<String>>("", "sql").unwrap_or(None)))
}

/// ②-c 外键。
///
/// SQLite 的 `pragma_foreign_key_list` 按 `id` 分组、`seq` 排序；**不含约束名**
/// （SQLite 不保存 FK 名）⇒ `name: None`。多列外键的列顺序由 `seq` 决定（语义）。
async fn read_foreign_keys(
    db: &DatabaseConnection,
    m: &mut SchemaModel,
    raw: &RawTable,
) -> Result<(), DbErr> {
    let rows = q_table(
        db,
        "SELECT id, seq, \"table\", \"from\", \"to\", on_update, on_delete \
         FROM pragma_foreign_key_list(?)",
        &raw.name,
    )
    .await?;

    /// `pragma_foreign_key_list` 按 `id` 聚合的中间形态：
    /// `(fk id, 该 fk 的列对 (seq, from, to), 引用表, on_update, on_delete)`。
    /// 抽成别名**只为满足 clippy `type_complexity`**（`-D warnings` 硬门禁）；
    /// 定义在函数内是因为它只是一处局部聚合，不是对外抽象。
    type FkAcc = (i64, Vec<(i64, String, String)>, String, String, String);
    let mut acc: Vec<FkAcc> = Vec::new();
    for row in rows {
        let id: i64 = row.try_get("", "id")?;
        let seq: i64 = row.try_get("", "seq").unwrap_or(0);
        let ref_tbl: String = row.try_get("", "table")?;
        let from_col: String = row.try_get("", "from")?;
        let to_col: Option<String> = row.try_get("", "to").unwrap_or(None);
        let on_update: String = row.try_get("", "on_update").unwrap_or_default();
        let on_delete: String = row.try_get("", "on_delete").unwrap_or_default();

        let entry = match acc.iter_mut().find(|(i, ..)| *i == id) {
            Some(e) => e,
            None => {
                acc.push((id, Vec::new(), ref_tbl.clone(), on_update.clone(), on_delete.clone()));
                acc.last_mut().expect("刚插入")
            },
        };
        entry.1.push((seq, from_col, to_col.unwrap_or_default()));
    }

    for (_, mut pairs, ref_tbl, on_update, on_delete) in acc {
        pairs.sort_by_key(|(s, _, _)| *s);
        let fk = FkModel {
            name: None,
            cols: pairs.iter().map(|(_, f, _)| f.clone()).collect(),
            ref_table: ref_tbl,
            ref_cols: pairs.iter().map(|(_, _, t)| t.clone()).collect(),
            // SQLite 用大写 `CASCADE` / `NO ACTION`；归一成 PG 侧同一套字符串
            on_delete: sqlite_fk_action(&on_delete),
            on_update: sqlite_fk_action(&on_update),
        };
        m.table_or_insert(&raw.name).fks.push(fk);
    }
    Ok(())
}

/// SQLite 的 FK 动作字符串 → 与 [`crate::reconcile::introspect::pg`] 同一套取值。
///
/// 两个方言必须归一到**同一个字符串空间**，否则同一份 schema 在两侧永远显示差异。
fn sqlite_fk_action(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "" => None,
        "NO ACTION" => Some("NoAction".to_string()),
        "RESTRICT" => Some("Restrict".to_string()),
        "CASCADE" => Some("Cascade".to_string()),
        "SET NULL" => Some("SetNull".to_string()),
        "SET DEFAULT" => Some("SetDefault".to_string()),
        other => Some(format!("UNKNOWN({other})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::introspect;
    use sea_orm::Database;

    /// FK 动作归一：两个方言必须落在同一字符串空间。
    #[test]
    fn fk_actions_match_pg_string_space() {
        assert_eq!(sqlite_fk_action("CASCADE").as_deref(), Some("Cascade"));
        assert_eq!(sqlite_fk_action("NO ACTION").as_deref(), Some("NoAction"));
        assert_eq!(sqlite_fk_action("SET NULL").as_deref(), Some("SetNull"));
        assert_eq!(sqlite_fk_action("").as_deref(), None);
        assert_eq!(sqlite_fk_action("WEIRD").as_deref(), Some("UNKNOWN(WEIRD)"));
    }

    /// 匿名表级 UNIQUE 约束：`pragma` 只给位置名，必须回 DDL 取真名。
    #[test]
    fn named_unique_constraint_is_recovered_from_ddl() {
        let sql = "CREATE TABLE t (a TEXT, b TEXT, CONSTRAINT uq_ab UNIQUE (a, b))";
        assert_eq!(
            named_unique_constraint(sql, &["a".into(), "b".into()]).as_deref(),
            Some("uq_ab")
        );
        // 列集不匹配 ⇒ 不能张冠李戴
        assert_eq!(named_unique_constraint(sql, &["a".into()]), None);
        // 匿名 ⇒ None（调用方用空名 + 列集识别）
        let anon = "CREATE TABLE t (a TEXT, b TEXT, UNIQUE (a, b))";
        assert_eq!(named_unique_constraint(anon, &["a".into(), "b".into()]), None);
    }

    /// 端到端（内存库）：列/主键/唯一/索引/部分索引/外键/CHECK 全部要读出来。
    ///
    /// 这是 SQLite 侧唯一的真实夹具 —— 断言的是「读出来的结构」而不是「有没有报错」。
    #[tokio::test]
    async fn reads_real_sqlite_schema() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        db.execute_unprepared(
            "CREATE TABLE parent (id TEXT NOT NULL PRIMARY KEY, code TEXT UNIQUE)",
        )
        .await
        .expect("建表应成功");
        db.execute_unprepared(
            "CREATE TABLE child ( \
                id TEXT NOT NULL, \
                parent_id TEXT NOT NULL, \
                n INTEGER NOT NULL DEFAULT 0, \
                kind TEXT, \
                PRIMARY KEY (id, parent_id), \
                CONSTRAINT fk_child_parent FOREIGN KEY (parent_id) REFERENCES parent (id) ON DELETE CASCADE, \
                CONSTRAINT ck_child_n CHECK (n >= 0) \
             )",
        )
        .await
        .expect("建表应成功");
        db.execute_unprepared("CREATE INDEX idx_child_kind ON child (kind) WHERE kind IS NOT NULL")
            .await
            .expect("建索引应成功");
        db.execute_unprepared("CREATE UNIQUE INDEX uq_child_kind_n ON child (kind, n)")
            .await
            .expect("建索引应成功");

        let m = introspect::read(&db).await.expect("introspect 应成功");
        assert_eq!(m.dialect, Dialect::Sqlite);

        let p = m.table("parent").expect("parent 应被读出");
        assert_eq!(p.primary_key, vec!["id"]);
        assert!(p.column("id").expect("id 列").primary_key);
        assert!(p.column("code").expect("code 列").unique, "列级 UNIQUE 应落到列标志");

        let c = m.table("child").expect("child 应被读出");
        assert_eq!(c.primary_key, vec!["id", "parent_id"], "复合主键按 pk 序号");
        assert_eq!(c.column("n").expect("n 列").default.as_deref(), Some("0"));
        assert!(!c.column("n").expect("n 列").nullable, "NOT NULL 应读出");

        let kind_idx = c.index("idx_child_kind").expect("普通索引应被读出");
        assert_eq!(kind_idx.cols, vec!["kind"]);
        assert!(!kind_idx.unique);
        // ⚠ SQLite 存的是 DDL **原文**，不会像 `pg_get_indexdef` 那样补外括号
        // ⇒ 这里如实断言无括号形态。两侧比较一律经 `model::normalize_sql_expr`
        // （它会剥配平外括号），故 L2 声明里写成 `(kind IS NOT NULL)` 也能对上。
        assert_eq!(kind_idx.where_clause.as_deref(), Some("kind IS NOT NULL"), "partial 谓词");

        let uq = c.index("uq_child_kind_n").expect("UNIQUE 索引应被读出");
        assert!(uq.unique);
        assert_eq!(uq.cols, vec!["kind", "n"]);

        // 自动索引不能混进来（`sqlite_autoindex_*` 是位置名，不是声明对象）
        assert!(
            !c.indexes.iter().any(|i| i.name.starts_with("sqlite_autoindex")),
            "自动索引必须排除：{:?}",
            c.indexes
        );

        let fk = c.fks.first().expect("外键应被读出");
        assert_eq!(fk.cols, vec!["parent_id"]);
        assert_eq!(fk.ref_table, "parent");
        assert_eq!(fk.ref_cols, vec!["id"]);
        assert_eq!(fk.on_delete.as_deref(), Some("Cascade"));

        let ck = c.checks.first().expect("CHECK 应被读出");
        assert_eq!(ck.name, "ck_child_n");
        assert_eq!(ck.expr, "n >= 0");
    }

    /// FTS5 虚表：自身要读出（由 L2 认领），影子表必须排除。
    #[tokio::test]
    async fn fts5_virtual_table_is_kept_and_shadows_excluded() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        db.execute_unprepared(
            "CREATE TABLE notes (id TEXT NOT NULL PRIMARY KEY, title TEXT, content TEXT)",
        )
        .await
        .expect("建表应成功");
        db.execute_unprepared(
            "CREATE VIRTUAL TABLE notes_fts USING fts5(title, content, content='notes', content_rowid='rowid', tokenize='porter unicode61')",
        )
        .await
        .expect("建虚表应成功");

        let m = introspect::read(&db).await.expect("introspect 应成功");
        let names = m.table_names();
        assert!(names.contains(&"notes_fts"), "虚表本身应被读出：{names:?}");
        assert!(
            !names.iter().any(|n| n.starts_with("notes_fts_")),
            "FTS5 影子表必须排除（否则引擎会 DROP 掉索引本体）：{names:?}"
        );
        assert!(
            m.table("notes_fts").expect("虚表").extras.iter().any(|e| e.contains("notes_fts")),
            "虚表应带 L2 认领标记"
        );
    }

    /// 生成列：表达式从 DDL 取；取不到必须报错（不能静默当普通列）。
    #[tokio::test]
    async fn generated_column_expr_comes_from_ddl() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        db.execute_unprepared(
            "CREATE TABLE g (a TEXT, b TEXT GENERATED ALWAYS AS (upper(a)) STORED)",
        )
        .await
        .expect("建表应成功");

        let m = introspect::read(&db).await.expect("introspect 应成功");
        let b = m.table("g").expect("g 表").column("b").expect("b 列");
        assert_eq!(b.generated.as_deref(), Some("upper(a)"));
        assert_eq!(b.default, None, "生成列不应有默认值");
    }
}
