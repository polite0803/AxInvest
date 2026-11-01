// SPDX-License-Identifier: AGPL-3.0-only

//! PostgreSQL 实况读取（`pg_catalog`）。
//!
//! ## 为什么用 `pg_catalog` 而不是 `information_schema`
//!
//! `information_schema` 是 SQL 标准视图，**信息有损**：生成列的表达式在
//! `generation_expression` 里被包了一层、`column_default` 对生成列为 NULL、
//! 看不到索引访问方法（GIN/HNSW）、看不到约束背后是否有扩展拥有。
//! 本模块要的是「精确复刻 DDL」，故一律直查 `pg_catalog`。
//!
//! ## 三条被实测逼出来的口径
//!
//! 1. **schema 用 `current_schema()` 而非硬编码 `public`** —— 但 `pg_stat_user_tables`
//!    之类不区分 schema 的视图一律不用（P0b 踩过：`ax_backup` 备份表混进统计）。
//! 2. **扩展拥有的对象必须排除**：实测 `public` 下 119 个函数里 **118 个属
//!    pgvector 扩展**（`pg_depend.deptype='e'`）。若按「未被 L2 认领 = 孤儿」判定，
//!    引擎会试图 DROP 扩展函数。索引同理（`pg_depend` 同样适用）。
//! 3. **CHECK 只按名字比对，不比表达式**：PG 会重写表达式
//!    （实测 `period ~ '^\d{4}-\d{2}$'` → `CHECK ((period ~ '^\d{4}-\d{2}$'::text))`），
//!    逐字比较必然不等。表达式仍读进模型供诊断，但不参与身份判定。

use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

use crate::reconcile::extras::Dialect;
use crate::reconcile::introspect::Introspector;
use crate::reconcile::introspect::parse;
use crate::reconcile::model::{
    CheckModel, ColumnModel, FkModel, IndexModel, SchemaModel, canonical_type_actual,
    normalize_default, normalize_sql_expr,
};

pub struct PgIntrospector;

#[async_trait]
impl Introspector for PgIntrospector {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    async fn read(&self, db: &DatabaseConnection) -> Result<SchemaModel, DbErr> {
        let mut m = SchemaModel::new(Dialect::Postgres);
        read_tables_and_columns(db, &mut m).await?;
        read_primary_keys(db, &mut m).await?;
        read_indexes(db, &mut m).await?;
        read_foreign_keys(db, &mut m).await?;
        read_checks(db, &mut m).await?;
        m.finalize();
        Ok(m)
    }
}

/// 执行一条无参只读查询。
async fn q(db: &DatabaseConnection, sql: &str) -> Result<Vec<sea_orm::QueryResult>, DbErr> {
    db.query_all_raw(Statement::from_string(DbBackend::Postgres, sql.to_string())).await
}

/// ① 表 + 列（类型 / 可空 / 默认 / 生成列）。
///
/// `relkind='r'` 只取普通表 —— 视图（`v`）、物化视图（`m`）、分区表（`p`）不在
/// 引擎管辖内（实测当前库这两种均为 0 条，但过滤不能因此省略）。
async fn read_tables_and_columns(
    db: &DatabaseConnection,
    m: &mut SchemaModel,
) -> Result<(), DbErr> {
    let rows = q(
        db,
        r#"
SELECT c.relname AS tbl,
       a.attname AS col,
       format_type(a.atttypid, a.atttypmod) AS ftype,
       a.attnotnull AS notnull,
       a.attgenerated::text AS genflag,
       a.attidentity::text AS ident,
       pg_get_expr(d.adbin, d.adrelid) AS adexpr
  FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace
  JOIN pg_attribute a ON a.attrelid = c.oid
  LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
 WHERE n.nspname = current_schema()
   AND c.relkind = 'r'
   AND a.attnum > 0
   AND NOT a.attisdropped
 ORDER BY c.relname, a.attnum
"#,
    )
    .await?;

    for row in rows {
        let tbl: String = row.try_get("", "tbl")?;
        let name: String = row.try_get("", "col")?;
        let ftype: String = row.try_get("", "ftype")?;
        let notnull: bool = row.try_get("", "notnull")?;
        let genflag: String = row.try_get("", "genflag")?;
        let ident: String = row.try_get("", "ident")?;
        let adexpr: Option<String> = row.try_get("", "adexpr")?;

        // `attgenerated='s'` = STORED 生成列；此时 adexpr 是**生成表达式**，不是默认值
        let (generated, default) = if genflag == "s" {
            (adexpr.as_deref().map(normalize_sql_expr), None)
        } else {
            (None, adexpr.as_deref().map(normalize_default))
        };

        let t = m.table_or_insert(&tbl);
        let mut c = ColumnModel {
            name,
            sql_type: canonical_type_actual(&ftype, Dialect::Postgres),
            nullable: !notnull,
            default,
            primary_key: false,
            unique: false,
            generated,
            renamed_from: None,
            auto_increment: false,
        };
        // PG 里自增有**两种实现**，都要认，否则会误判成「多出来的默认值」并 `DROP DEFAULT`
        // （实测生产库 10 列，详见 `ColumnModel::auto_increment`）：
        //   ① `SERIAL`   → 表上没有 identity，但有一条 `DEFAULT nextval('…_seq')`
        //   ② `IDENTITY` → `attidentity` ∈ {`a`(ALWAYS), `d`(BY DEFAULT)}，且 `column_default` 为 NULL
        // 判据不拼序列名：PG 会按 63 字节截断，拼名字在长表名上必然出错。
        c.auto_increment = !ident.is_empty() || c.default_is_sequence_binding();
        t.upsert_column(c);
    }
    Ok(())
}

/// ② 主键列。
async fn read_primary_keys(db: &DatabaseConnection, m: &mut SchemaModel) -> Result<(), DbErr> {
    let rows = q(
        db,
        r#"
SELECT c.relname AS tbl, a.attname AS col
  FROM pg_index i
  JOIN pg_class c ON c.oid = i.indrelid
  JOIN pg_namespace n ON n.oid = c.relnamespace
  JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY (i.indkey)
 WHERE n.nspname = current_schema()
   AND i.indisprimary
"#,
    )
    .await?;

    for row in rows {
        let tbl: String = row.try_get("", "tbl")?;
        let col: String = row.try_get("", "col")?;
        if let Some(t) = m.table_mut(&tbl) {
            t.primary_key.push(col.clone());
            if let Some(c) = t.columns.iter_mut().find(|c| c.name == col) {
                c.primary_key = true;
            }
        }
    }
    Ok(())
}

/// ③ 索引 + UNIQUE 约束。
///
/// 分流规则（必须与 `expected.rs` 严格镜像，否则永远差一条）：
/// - 主键索引 ⇒ 已由 ② 处理，跳过；
/// - **单列** `contype='u'` ⇒ 落到列的 `unique` 标志（对齐 L1 `#[sea_orm(unique)]`）；
/// - **多列** `contype='u'` ⇒ 落成 `IndexModel{unique:true}`（对齐 L2 `IndexDecl`）；
/// - 扩展拥有的索引 ⇒ 跳过（引擎不拥有扩展对象）。
async fn read_indexes(db: &DatabaseConnection, m: &mut SchemaModel) -> Result<(), DbErr> {
    let rows = q(
        db,
        r#"
SELECT c.relname AS tbl,
       ic.relname AS idx,
       pg_get_indexdef(i.indexrelid) AS def,
       i.indisunique AS uniq,
       -- ⚠ `contype` 是 PG 的内部类型 `"char"`（1 字节），**必须 `::text`**：
       -- sea-orm 把它解码成 `Option<String>`（SQL type TEXT）会直接报
       -- `mismatched types … as SQL type "CHAR"` 并让整次 introspect 失败。
       -- 同族：`pg_class.relkind` / `pg_attribute.attgenerated` / `pg_type.typtype` /
       -- `pg_constraint.confdeltype` 全是这个类型（本文件里逐处已投或已避开）。
       con.contype::text AS contype,
       con.conname AS conname,
       (SELECT count(*)::int4
          FROM pg_attribute a
         WHERE a.attrelid = c.oid AND a.attnum = ANY (i.indkey)) AS ncols
  FROM pg_index i
  JOIN pg_class ic ON ic.oid = i.indexrelid
  JOIN pg_class c ON c.oid = i.indrelid
  JOIN pg_namespace n ON n.oid = c.relnamespace
  LEFT JOIN pg_constraint con ON con.conindid = i.indexrelid AND con.contype IN ('p','u')
  LEFT JOIN pg_depend dep ON dep.objid = ic.oid AND dep.deptype = 'e'
 WHERE n.nspname = current_schema()
   AND NOT i.indisprimary
   AND dep.objid IS NULL
 ORDER BY c.relname, ic.relname
"#,
    )
    .await?;

    for row in rows {
        let tbl: String = row.try_get("", "tbl")?;
        let def: String = row.try_get("", "def")?;
        let uniq: bool = row.try_get("", "uniq")?;
        let contype: Option<String> = row.try_get("", "contype")?;
        let conname: Option<String> = row.try_get("", "conname")?;
        let ncols: i32 = row.try_get("", "ncols")?;

        let parsed = parse::parse_index_ddl(&def).ok_or_else(|| {
            DbErr::Custom(format!(
                "pg introspect: 无法解析 {tbl} 的索引 DDL，拒绝臆造（原文：{def}）"
            ))
        })?;

        if contype.as_deref() == Some("u") && ncols == 1 {
            // 单列 UNIQUE ⇒ 列标志；约束名与索引名在 PG 里通常同为 `{表}_{列}_key`
            let col = parsed.cols.first().cloned().unwrap_or_default();
            if let Some(t) = m.table_mut(&tbl)
                && let Some(c) = t.columns.iter_mut().find(|c| c.name == col)
            {
                c.unique = true;
            }
            continue;
        }

        // 约束型 UNIQUE 用**约束名**（`conname`）作身份：L2 声明里写的就是约束名
        let name = if contype.is_some() {
            conname.unwrap_or(parsed.name)
        } else {
            parsed.name
        };
        let t = m.table_or_insert(&tbl);
        t.upsert_index(IndexModel {
            name,
            cols: parsed.cols,
            unique: uniq,
            method: parsed.method.filter(|m| m != "btree"),
            where_clause: parsed.where_clause,
        });
    }
    Ok(())
}

/// ④ 外键。
async fn read_foreign_keys(db: &DatabaseConnection, m: &mut SchemaModel) -> Result<(), DbErr> {
    let rows = q(
        db,
        r#"
SELECT c.relname AS tbl,
       con.conname AS name,
       a.attname AS col,
       rc.relname AS ref_tbl,
       ra.attname AS ref_col,
       con.confdeltype::text AS del_type,
       con.confupdtype::text AS upd_type,
       k.ord AS ord
  FROM pg_constraint con
  JOIN pg_class c ON c.oid = con.conrelid
  JOIN pg_class rc ON rc.oid = con.confrelid
  JOIN pg_namespace n ON n.oid = c.relnamespace
  CROSS JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS k(attnum, refattnum, ord)
  JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum
  JOIN pg_attribute ra ON ra.attrelid = con.confrelid AND ra.attnum = k.refattnum
 WHERE n.nspname = current_schema()
   AND con.contype = 'f'
 ORDER BY c.relname, con.conname, k.ord
"#,
    )
    .await?;

    // 多列外键按 (表, 约束名) 聚合，保持 conkey 顺序（列顺序是语义）
    let mut acc: Vec<(String, String, FkModel)> = Vec::new();
    for row in rows {
        let tbl: String = row.try_get("", "tbl")?;
        let name: String = row.try_get("", "name")?;
        let col: String = row.try_get("", "col")?;
        let ref_tbl: String = row.try_get("", "ref_tbl")?;
        let ref_col: String = row.try_get("", "ref_col")?;
        let del: String = row.try_get("", "del_type")?;
        let upd: String = row.try_get("", "upd_type")?;

        match acc.iter_mut().find(|(t, n, _)| t == &tbl && n == &name) {
            Some((_, _, fk)) => {
                fk.cols.push(col);
                fk.ref_cols.push(ref_col);
            },
            None => acc.push((
                tbl.clone(),
                name.clone(),
                FkModel {
                    name: Some(name.clone()),
                    cols: vec![col],
                    ref_table: ref_tbl,
                    ref_cols: vec![ref_col],
                    on_delete: fk_action(&del),
                    on_update: fk_action(&upd),
                },
            )),
        }
    }

    for (tbl, _, fk) in acc {
        m.table_or_insert(&tbl).fks.push(fk);
    }
    Ok(())
}

/// `pg_constraint.confdeltype` / `confupdtype` 的**单字符**编码 → 稳定字符串。
///
/// 编码取自 PG 文档（`pg_constraint`）：a=NO ACTION, r=RESTRICT, c=CASCADE,
/// n=SET NULL, d=SET DEFAULT。**不认识的编码一律报 `None` 之外的显式串**，
/// 免得把未知动作静默当成 NO ACTION。
fn fk_action(code: &str) -> Option<String> {
    let s = match code {
        "" => return None,
        "a" => "NoAction",
        "r" => "Restrict",
        "c" => "Cascade",
        "n" => "SetNull",
        "d" => "SetDefault",
        other => return Some(format!("UNKNOWN({other})")),
    };
    Some(s.to_string())
}

/// ⑤ CHECK 约束。
///
/// ⚠ 表达式**只做诊断用途，不参与身份判定**：PG 会把 `period ~ '^\d{4}-\d{2}$'`
/// 重写成 `(period ~ '^\d{4}-\d{2}$'::text)`（实测），逐字比较必然不等。
/// 身份用 [`CheckModel::key`]（PG 侧恒有名字 ⇒ 按名）。
async fn read_checks(db: &DatabaseConnection, m: &mut SchemaModel) -> Result<(), DbErr> {
    let rows = q(
        db,
        r#"
SELECT c.relname AS tbl, con.conname AS name, pg_get_constraintdef(con.oid) AS def
  FROM pg_constraint con
  JOIN pg_class c ON c.oid = con.conrelid
  JOIN pg_namespace n ON n.oid = c.relnamespace
 WHERE n.nspname = current_schema()
   AND con.contype = 'c'
 ORDER BY c.relname, con.conname
"#,
    )
    .await?;

    for row in rows {
        let tbl: String = row.try_get("", "tbl")?;
        let name: String = row.try_get("", "name")?;
        let def: String = row.try_get("", "def")?;
        // `CHECK ((expr))` → 剥掉 `CHECK ` 前缀，表达式本身保留原文（诊断用）
        let expr = def
            .strip_prefix("CHECK ")
            .map(normalize_sql_expr)
            .unwrap_or_else(|| normalize_sql_expr(&def));
        m.table_or_insert(&tbl).upsert_check(CheckModel { name, expr });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FK 动作编码映射：未知编码必须显式暴露，不能静默当成 NoAction。
    #[test]
    fn fk_action_codes_map_explicitly() {
        assert_eq!(fk_action("a").as_deref(), Some("NoAction"));
        assert_eq!(fk_action("r").as_deref(), Some("Restrict"));
        assert_eq!(fk_action("c").as_deref(), Some("Cascade"));
        assert_eq!(fk_action("n").as_deref(), Some("SetNull"));
        assert_eq!(fk_action("d").as_deref(), Some("SetDefault"));
        assert_eq!(fk_action(""), None, "未设动作");
        assert_eq!(
            fk_action("z").as_deref(),
            Some("UNKNOWN(z)"),
            "未知编码必须显式暴露（否则会被当成 NoAction 静默判等）"
        );
    }

    /// **PG 内部类型 `"char"` 的列在 SELECT 列表里必须 `::text` 投射。**
    ///
    /// 实测（P3 首次真库运行）：`pg_constraint.contype` 漏了投射 ⇒ 整次 introspect
    /// 以 `Query Error: error occurred while decoding column "contype": mismatched
    /// types; Rust type Option<String> … not compatible with SQL type "CHAR"` 失败。
    ///
    /// 为什么用**源码文本**而不是运行期断言：这条缺陷只在**真库**上炸，而单测没有
    /// PG 连接 —— 真库测试要靠 `AXAGENT_TEST_PG_URL` 且会腐烂（本仓 2026-09 就踩过：
    /// 4 个 PG 测试各自 gate，变量名有三种写法，**在 CI 里一条都没跑过**；
    /// 其中 `pg_migrations.rs` 已随 74 个版本化迁移于 2026-09-16 一起退休，
    /// 现在这批测试的代表是 `dao/tests/pg_cjk_fts.rs` 与 `search/tests/pg_integration.rs`）。
    /// 扫描自身源码是这里唯一能常驻的守卫，且它按**同型缺陷穷举**：PG 里所有
    /// `"char"` 型列名都在 `CHAR_TYPED` 里，新加一处漏投射即红。
    #[test]
    fn char_typed_pg_columns_are_always_cast_in_select_list() {
        const CHAR_TYPED: &[&str] = &[
            "contype",
            "confdeltype",
            "confupdtype",
            "confmatchtype",
            "attgenerated",
            "attidentity",
            "relkind",
            "relpersistence",
            "typtype",
            "prokind",
        ];
        let src = include_str!("pg.rs");
        let mut checked = 0usize;
        for (i, line) in src.lines().enumerate() {
            let Some((lhs, _)) = line.split_once(" AS ") else { continue };
            // 去掉 `::类型` 投射后再取**最后一段标识符**（`con.contype::text` → `contype`）
            let base = lhs.split("::").next().unwrap_or(lhs);
            let last = base.trim().rsplit('.').next().unwrap_or(base).trim();
            if CHAR_TYPED.contains(&last) {
                checked += 1;
                assert!(
                    lhs.contains("::text"),
                    "pg.rs:{} 选择列表里的 `{last}` 未 `::text` 投射 —— PG 内部类型 \"char\" \
                     无法被 sea-orm 解码成 String，整次 introspect 会失败：{line}",
                    i + 1
                );
            }
        }
        assert!(checked >= 3, "守卫自身的命中数异常（{checked}）—— 扫描逻辑可能已失效");
    }
}
