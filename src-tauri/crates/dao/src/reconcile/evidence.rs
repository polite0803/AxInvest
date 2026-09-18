// SPDX-License-Identifier: AGPL-3.0-only

//! 导出「丢数据变更」的证据 —— 补上「导出成功才允许 DROP」这条契约**缺的那一端**。
//!
//! ## 为什么这个模块必须存在（P4 首次真库执行实测）
//!
//! `safety` 的模块文档写着「导出成功才允许 DROP」，并且 `missing_evidence` 会为此
//! **整批拒绝**。但 P4 落地时只有**强制端**，没有**产出端** —— 全项目没有任何工具能
//! 产出 `GraveEvidence`。实测症状（`output/tmp-p4-exec-run2.log`）：
//!
//! ```text
//! 整批拒绝 : [evidence] 42 条会丢数据的变更缺少导出证据：axagent_schema_version、
//!            …、narrative_structures.arcs（先把数据导出来，再重跑）
//! 执行     : 0
//! ```
//!
//! 42 条 = `DropTable` 8 + `AlterColumnType` 31 + `DropColumn` 3。在补上本模块之前，
//! 引擎**永远无法执行任何丢数据的变更** —— 闸是对的，但缺了一只手。
//!
//! ## 分工（不要再把这两件事混起来）
//!
//! | 端 | 位置 | 职责 |
//! |---|---|---|
//! | 强制端 | [`super::safety::missing_evidence`] | 没有证据就整批拒，**不猜**要导什么 |
//! | 产出端 | 本模块 | 真的把数据导出来，并**证明导完整了** |
//!
//! `safety` 侧「不自己去导出」是刻意的：它不知道要导整表还是某一列。那个知识在
//! **本模块的 [`target_of`]** 里，而它是个可单测的纯函数。
//!
//! ## 为什么导出格式是 PG 的 `to_jsonb(...)::text`（JSON Lines）
//!
//! 让**数据库自己**做「类型 → JSON」的映射，而不是在 Rust 里为每种 PG 类型写一遍
//! 取值分支（`timestamptz` / `numeric` / `jsonb` / `vector` / 数组 / 自定义类型……）。
//! 前者一行 SQL，后者是一个必然会漏掉某类型的 `match`。
//!
//! ## 完整性判据：**导出不足 = 假证据**
//!
//! [`verify_export`] **回读文件字节**算 sha256、数行数，再与 DB 的 `count(*)` 比对，
//! 不等即报错。`safety` 的 `GraveEvidence::is_complete` 文档已经写明理由：
//! 「只有回读 + 指纹比对才能区分『导成功』与『文件存在』」——
//! 一个中途失败留下的半截文件，光看「文件存在」是分辨不出来的。
//!
//! ⚠ 已知窗口：`count(*)` 与导出是两条独立查询，期间若有并发写入，比对会失败。
//! 这个方向是**故意选的** —— 宁可误报「导不全」让人重导，也不要放过一个真的导不足。
//!
//! ## 只做 PostgreSQL
//!
//! `to_jsonb` / `row_to_json` 是 PG 专有。SQLite 侧一律
//! [`ExportError::UnsupportedDialect`]，与 [`super::render`] 对 SQLite 的处理同源。
//! 故**本模块的 DB 路径无法在 CI 上端到端验证**（CI 没有 PG）—— 能验证的是
//! `target_of` 的分派、SQL 文本、路径安全化、回读校验这四件事（见文末测试）。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use sha2::{Digest, Sha256};

use super::apply::dialect_of;
use super::extras::Dialect;
use super::plan::{Change, ChangeKind, Plan};
use super::render::quote_ident;
use super::safety::GraveEvidence;

// ═══════════════════════════════════════════════════════════════════════════
// 要导出什么（纯函数，可单测）
// ═══════════════════════════════════════════════════════════════════════════

/// 一条变更要从库里导出什么。
///
/// `DropTable` 导**整表**；`DropColumn` / `AlterColumnType` 导**那一列**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    Table { table: String },
    Column { table: String, col: String },
}

impl ExportTarget {
    /// 被导出的对象名 —— **必须与 `Change::object` 逐字相同**。
    ///
    /// `safety::missing_evidence` 是按 `Change::object` 索引证据的，这里若做了任何
    /// 「美化」（去引号、大小写归一、把 `表.列` 写成 `表/列`），证据就会**永远配不上**，
    /// 而症状只是「每次都说缺证据」—— 看不出是键名不匹配。
    pub fn object(&self) -> String {
        match self {
            ExportTarget::Table { table } => table.clone(),
            ExportTarget::Column { table, col } => format!("{table}.{col}"),
        }
    }

    pub fn table(&self) -> &str {
        match self {
            ExportTarget::Table { table } | ExportTarget::Column { table, .. } => table,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// 该类别**不丢数据**，不该向它要证据。
    NotDataLosing {
        kind: &'static str,
        object: String,
    },
    /// `object` 的形态不是「表」或「表.列」。
    MalformedObject {
        object: String,
        expected: &'static str,
    },
    UnsupportedDialect {
        dialect: Dialect,
    },
    RowsExceedCap {
        object: String,
        rows: i64,
        cap: i64,
    },
    Db {
        object: String,
        source: String,
    },
    Io {
        path: String,
        source: String,
    },
    /// 回读校验不通过 —— 这是**假证据**的检测点。
    Verify {
        object: String,
        why: String,
    },
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::NotDataLosing { kind, object } => {
                write!(f, "`{kind}` 不丢数据（object={object}），不该向它要导出证据")
            },
            ExportError::MalformedObject { object, expected } => {
                write!(f, "object `{object}` 的形态不是 {expected}")
            },
            ExportError::UnsupportedDialect { dialect } => match dialect {
                Dialect::Postgres => write!(f, "（不该发生）Postgres 被当成不支持的方言"),
                Dialect::Sqlite => write!(
                    f,
                    "SQLite 不支持 `to_jsonb` —— 导出证据只做 PostgreSQL（与 render 同源）"
                ),
            },
            ExportError::RowsExceedCap { object, rows, cap } => write!(
                f,
                "`{object}` 有 {rows} 行，超过导出上限 {cap} 行 —— **不截断**，\
                 要么调大上限，要么先归档该对象"
            ),
            ExportError::Db { object, source } => write!(f, "导出 `{object}` 时报库错：{source}"),
            ExportError::Io { path, source } => write!(f, "读写 `{path}` 失败：{source}"),
            ExportError::Verify { object, why } => {
                write!(f, "`{object}` 的导出**未能通过完整性校验**：{why}")
            },
        }
    }
}

impl std::error::Error for ExportError {}

/// 从变更推出「要导什么」。
///
/// **对全部 21 个 [`ChangeKind`] 都有确定的答案**：
///
/// - `DropTable` / `DropColumn` / `AlterColumnType` ⇒ `Ok`（恰好是 `loses_data()` 的集合）
/// - 其余 18 类 ⇒ `Err(NotDataLosing)`
///
/// 这个「确定的答案」是刻意的，测试 `target_of_agrees_with_loses_data_on_all_kinds`
/// 会遍历 [`ChangeKind::ALL`] 断言 `target_of(k).is_ok() == k.loses_data()`。
/// 于是**将来新增一个 `loses_data` 类别却忘了教导出器怎么导**时，测试当场变红 ——
/// 而不是等到真跑那天才由 `missing_evidence` 报「缺证据」，
/// 让人以为「只是没导出」而不是「导出器不认识这一类」。
pub fn target_of(change: &Change) -> Result<ExportTarget, ExportError> {
    let object = change.object.clone();
    match change.kind {
        ChangeKind::DropTable => Ok(ExportTarget::Table { table: object }),
        ChangeKind::DropColumn | ChangeKind::AlterColumnType => {
            // 用 `rsplit_once` 而非 `split_once`：表名可能自带点（schema 限定名、
            // 或 PG 里合法的 `"a.b"` 表名），而列名不含点。
            let (t, c) = object.rsplit_once('.').ok_or(ExportError::MalformedObject {
                object: object.clone(),
                expected: "`表.列`（列级变更的 object 形态）",
            })?;
            if t.is_empty() || c.is_empty() {
                return Err(ExportError::MalformedObject {
                    object,
                    expected: "`表.列`，且表名与列名都非空",
                });
            }
            Ok(ExportTarget::Column { table: t.to_string(), col: c.to_string() })
        },
        other => Err(ExportError::NotDataLosing { kind: other.as_str(), object }),
    }
}

/// 导出用的 SQL：**一行一条 JSON Lines**。
pub fn export_sql(target: &ExportTarget, dialect: Dialect) -> Result<String, ExportError> {
    if dialect != Dialect::Postgres {
        return Err(ExportError::UnsupportedDialect { dialect });
    }
    Ok(match target {
        ExportTarget::Table { table } => {
            format!("SELECT to_jsonb(t)::text AS row_json FROM {} AS t", quote_ident(table))
        },
        ExportTarget::Column { table, col } => format!(
            "SELECT to_jsonb({})::text AS row_json FROM {}",
            quote_ident(col),
            quote_ident(table)
        ),
    })
}

/// `count(*)` 用的 SQL —— 与 [`export_sql`] 的**行集必须一致**，否则完整性比对恒不等。
///
/// 注意 `Column` 的计数是**表的行数**，不是该列非 NULL 的行数：导出侧对 NULL 行也会
/// 产出一行 `null`，两边口径必须一致。
pub fn count_sql(target: &ExportTarget, dialect: Dialect) -> Result<String, ExportError> {
    if dialect != Dialect::Postgres {
        return Err(ExportError::UnsupportedDialect { dialect });
    }
    Ok(format!("SELECT count(*) AS n FROM {}", quote_ident(target.table())))
}

// ═══════════════════════════════════════════════════════════════════════════
// 导出路径（安全化 —— 这是**路径穿越**的拦截点）
// ═══════════════════════════════════════════════════════════════════════════

/// 导出文件路径：`<dir>/<安全化后的 object>.jsonl`。
///
/// ⚠ 为什么必须安全化：`object` 来自**数据库标识符**，而 PG 允许 `"a/b"`、`".."`、
/// `"C:x"` 这类名字。直接拼进路径的话，一个名为 `../../etc/passwd` 的对象就能让
/// 导出写到目录外 —— 而这里的调用方是**自动化的**，没有人会在写之前看一眼路径。
/// 白名单而非黑名单：只留 `[A-Za-z0-9_-]`，其余（含 `.` `/` `\` `:`）一律换 `_`。
pub fn evidence_path(dir: &Path, object: &str) -> PathBuf {
    let safe: String = object
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{safe}.jsonl"))
}

// ═══════════════════════════════════════════════════════════════════════════
// 回读校验（假证据的检测点）
// ═══════════════════════════════════════════════════════════════════════════

/// 回读导出文件，产出**经过验证**的 [`GraveEvidence`]。
///
/// 三步，缺一不可：
///
/// 1. **读文件字节**（不是内存里那份字符串）算 sha256 —— 只有这样才能发现
///    「写了一半就失败」；
/// 2. 数行数（JSON Lines ⇒ 行数 = 记录数）；
/// 3. 与 `expected_rows`（DB 的 `count(*)`）比对，**不等即报错**。
///
/// ⚠ `expected_rows == 0` 且文件为空是**合法**的：0 行表的证据就是空文件。
/// 这不是「什么都没导」—— 是「确实没有东西可导」，而这两者在上一层的
/// `count(*)` 里已经被区分开了。
pub fn verify_export(
    object: &str,
    path: &Path,
    expected_rows: i64,
) -> Result<GraveEvidence, ExportError> {
    let bytes = std::fs::read(path)
        .map_err(|e| ExportError::Io { path: path.display().to_string(), source: e.to_string() })?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let text = String::from_utf8(bytes).map_err(|e| ExportError::Verify {
        object: object.to_string(),
        why: format!("导出文件不是合法 UTF-8：{e}"),
    })?;
    // JSON Lines：每条记录一行，故末尾换行不产生额外记录。
    let rows = text.lines().count() as i64;
    if rows != expected_rows {
        return Err(ExportError::Verify {
            object: object.to_string(),
            why: format!(
                "文件里 {rows} 行，但库里是 {expected_rows} 行 —— \
                 导出不足就是假证据，不能拿它去换一次 DROP"
            ),
        });
    }
    Ok(GraveEvidence {
        export_path: path.display().to_string(),
        export_sha256: sha256,
        row_count: Some(rows),
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// 编排
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub dir: PathBuf,
    /// 单对象导出上限。**超限报错，不截断** —— 截断出来的证据比没有证据更危险，
    /// 因为它通过了 `is_complete()`，看起来是完整的。
    pub max_rows: i64,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { dir: PathBuf::from("output/schema-evidence"), max_rows: 1_000_000 }
    }
}

/// 一次导出里的单个产物。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dumped {
    pub object: String,
    pub path: String,
    pub rows: i64,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Default)]
pub struct ExportReport {
    /// 直接可喂给 `ApplyOptions::evidence`。
    pub evidence: BTreeMap<String, GraveEvidence>,
    pub dumped: Vec<Dumped>,
    /// 计划里**不需要**证据的变更数（有它才能说清「42 / 712」而不是「42 / 未知」）。
    pub no_evidence_needed: usize,
}

impl ExportReport {
    /// 人读的一行摘要（进日志）。
    pub fn summary(&self) -> String {
        let rows: i64 = self.dumped.iter().map(|d| d.rows).sum();
        let bytes: u64 = self.dumped.iter().map(|d| d.bytes).sum();
        format!(
            "导出 {} 个对象 / {rows} 行 / {:.1} MB（另 {} 条变更不丢数据、无需证据）",
            self.dumped.len(),
            bytes as f64 / 1024.0 / 1024.0,
            self.no_evidence_needed
        )
    }
}

/// 为计划里**所有** `loses_data` 变更导出证据。
///
/// ⚠ 语义是「全量导出」而非「补齐缺失」：`plan` 里哪些是丢数据的完全由
/// `Change::loses_data` 决定，与「已经有哪些证据」无关。这样重跑是**幂等**的
/// （同样的输入产出同样的文件），不会因为「上次导了一半」而只补一半。
pub async fn export_for_plan(
    db: &DatabaseConnection,
    plan: &Plan,
    opts: &ExportOptions,
) -> Result<ExportReport, ExportError> {
    let dialect = dialect_of(db)
        .map_err(|e| ExportError::Db { object: "(dialect)".to_string(), source: e.to_string() })?;
    let backend = db.get_database_backend();
    std::fs::create_dir_all(&opts.dir).map_err(|e| ExportError::Io {
        path: opts.dir.display().to_string(),
        source: e.to_string(),
    })?;

    let mut report = ExportReport {
        no_evidence_needed: plan.changes.iter().filter(|c| !c.loses_data).count(),
        ..Default::default()
    };

    for change in plan.changes.iter().filter(|c| c.loses_data) {
        let target = target_of(change)?;
        let object = target.object();

        // 先计数：① 用来做上限判断（超限时**还没写任何文件**）② 用来做完整性比对基准。
        let n = db
            .query_all_raw(Statement::from_string(backend, count_sql(&target, dialect)?))
            .await
            .map_err(|e| ExportError::Db { object: object.clone(), source: e.to_string() })?
            .first()
            .and_then(|r| r.try_get_by_index::<i64>(0).ok())
            .ok_or_else(|| ExportError::Db {
                object: object.clone(),
                source: "count(*) 没返回可读的整数".to_string(),
            })?;

        if n > opts.max_rows {
            return Err(ExportError::RowsExceedCap { object, rows: n, cap: opts.max_rows });
        }

        let rows = db
            .query_all_raw(Statement::from_string(backend, export_sql(&target, dialect)?))
            .await
            .map_err(|e| ExportError::Db { object: object.clone(), source: e.to_string() })?;

        // JSON Lines：NULL 也要产出一行 `null`（用 `unwrap_or` 而不是 `?`），
        // 否则「某列全为 NULL」会让行数比对失败，而那是**正常数据**不是导出故障。
        let mut body = String::new();
        for r in &rows {
            let v: Option<String> = r.try_get_by_index(0).map_err(|e| ExportError::Db {
                object: object.clone(),
                source: format!("读 row_json 失败：{e}"),
            })?;
            body.push_str(v.as_deref().unwrap_or("null"));
            body.push('\n');
        }

        let path = evidence_path(&opts.dir, &object);
        std::fs::write(&path, &body).map_err(|e| ExportError::Io {
            path: path.display().to_string(),
            source: e.to_string(),
        })?;

        let ev = verify_export(&object, &path, n)?;
        report.dumped.push(Dumped {
            object: object.clone(),
            path: ev.export_path.clone(),
            rows: n,
            bytes: body.len() as u64,
            sha256: ev.export_sha256.clone(),
        });
        report.evidence.insert(object, ev);
    }

    Ok(report)
}

// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::plan::{Change, ChangePayload};

    /// 临时目录（不需要 `tempfile` 依赖，测试自己清）。
    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("axev-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// ⚠ `destructive` / `loses_data` 从 `kind` **派生**，不手填 ——
    /// 手填的话这个测试就变成「验证我自己填的两个 bool」，而它要验证的是
    /// 「kind → 是否需要证据」这条真实规则。
    fn change(kind: ChangeKind, object: &str) -> Change {
        Change {
            kind,
            object: object.to_string(),
            destructive: kind.destructive(),
            loses_data: kind.loses_data(),
            detail: String::new(),
            payload: ChangePayload::Bare,
        }
    }

    /// ⚠ 本模块最重要的判据：**「哪些类别要证据」与「导出器认识哪些类别」必须恒等**。
    ///
    /// 用 `ChangeKind::ALL` 遍历 ⇒ 新增一个 `loses_data` 类别而忘了教导出器时当场变红。
    /// 若只测「那 3 类返回 Ok」，漏掉的第 4 类会让测试**继续全绿**。
    #[test]
    fn target_of_agrees_with_loses_data_on_all_kinds() {
        assert_eq!(ChangeKind::ALL.len(), 21, "`ChangeKind::ALL` 长度变了");
        for k in ChangeKind::ALL {
            let c = change(k, "t.c");
            assert_eq!(
                target_of(&c).is_ok(),
                k.loses_data(),
                "`{}`：target_of 与 loses_data 不一致（loses_data={}）—— \
                 要么补导出分派，要么修正 loses_data 的定义",
                k.as_str(),
                k.loses_data()
            );
        }
    }

    #[test]
    fn drop_table_exports_the_whole_table() {
        let c = change(ChangeKind::DropTable, "opc_capability");
        assert_eq!(target_of(&c).unwrap(), ExportTarget::Table { table: "opc_capability".into() });
        // 整表导出的 object 就是表名本身（不是 `表.`）
        assert_eq!(target_of(&c).unwrap().object(), "opc_capability");
    }

    #[test]
    fn column_level_changes_export_that_column() {
        for kind in [ChangeKind::DropColumn, ChangeKind::AlterColumnType] {
            let c = change(kind, "knowledge_entities.metadata");
            assert_eq!(
                target_of(&c).unwrap(),
                ExportTarget::Column { table: "knowledge_entities".into(), col: "metadata".into() },
                "{}",
                kind.as_str()
            );
            assert_eq!(target_of(&c).unwrap().object(), "knowledge_entities.metadata");
        }
    }

    /// 表名自带点时按**最后一个**点切 —— 若用 `split_once`，`a.b.c` 会切出
    /// 表 `a` / 列 `b.c`，导出 SQL 指向一个不存在的对象。
    #[test]
    fn object_with_dots_splits_on_the_last_dot() {
        let c = change(ChangeKind::DropColumn, "public.a.c");
        assert_eq!(
            target_of(&c).unwrap(),
            ExportTarget::Column { table: "public.a".into(), col: "c".into() }
        );
    }

    #[test]
    fn malformed_column_object_is_rejected() {
        for bad in ["no_dot_at_all", "t.", ".c"] {
            let e = target_of(&change(ChangeKind::DropColumn, bad)).unwrap_err();
            assert!(matches!(e, ExportError::MalformedObject { .. }), "{bad} 未被拒绝：{e}");
        }
    }

    #[test]
    fn non_data_losing_kinds_are_rejected_by_name() {
        let e = target_of(&change(ChangeKind::DropIndex, "idx_x")).unwrap_err();
        assert!(matches!(e, ExportError::NotDataLosing { .. }), "{e}");
        assert!(e.to_string().contains("DROP INDEX"), "{e}");
    }

    #[test]
    fn export_sql_is_postgres_only_and_quotes_identifiers() {
        let t = ExportTarget::Table { table: "opc_capability".into() };
        let sql = export_sql(&t, Dialect::Postgres).unwrap();
        assert_eq!(sql, "SELECT to_jsonb(t)::text AS row_json FROM \"opc_capability\" AS t");

        let c = ExportTarget::Column { table: "t".into(), col: "metadata".into() };
        assert_eq!(
            export_sql(&c, Dialect::Postgres).unwrap(),
            "SELECT to_jsonb(\"metadata\")::text AS row_json FROM \"t\""
        );

        // SQLite 侧必须**明确报错**而不是产出一句跑不通的 SQL
        assert!(matches!(
            export_sql(&t, Dialect::Sqlite),
            Err(ExportError::UnsupportedDialect { .. })
        ));
        assert!(matches!(
            count_sql(&t, Dialect::Sqlite),
            Err(ExportError::UnsupportedDialect { .. })
        ));
    }

    /// 标识符里的 `"` 必须双写 —— 这是 `quote_ident` 被提到 `pub(crate)` 的理由。
    #[test]
    fn quote_ident_survives_a_quote_in_the_name() {
        let t = ExportTarget::Table { table: "we\"ird".into() };
        let sql = export_sql(&t, Dialect::Postgres).unwrap();
        assert!(sql.contains("\"we\"\"ird\""), "{sql}");
    }

    /// `count_sql` 的列版数的是**表行数**，不是该列非 NULL 行数 —— 与导出侧口径一致。
    #[test]
    fn count_sql_counts_table_rows_for_column_targets() {
        let c = ExportTarget::Column { table: "t".into(), col: "c".into() };
        assert_eq!(count_sql(&c, Dialect::Postgres).unwrap(), "SELECT count(*) AS n FROM \"t\"");
    }

    /// ⚠ 路径穿越：`object` 来自数据库标识符，PG 允许里面带 `/`、`..`、`:`。
    #[test]
    fn evidence_path_cannot_escape_the_directory() {
        let d = Path::new("/tmp/ev");
        for evil in ["../escape", "..\\escape", "a/../../etc/passwd", "C:evil", "t.列"] {
            let p = evidence_path(d, evil);
            assert_eq!(p.parent().unwrap(), d, "`{evil}` 逃出了导出目录：{}", p.display());
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            assert!(!name.contains('/') && !name.contains('\\') && !name.contains(':'), "{name}");
        }
        assert_eq!(
            evidence_path(d, "narrative_structures.arcs"),
            PathBuf::from("/tmp/ev/narrative_structures_arcs.jsonl")
        );
    }

    #[test]
    fn verify_export_accepts_a_complete_file() {
        let d = tmpdir("ok");
        let p = d.join("x.jsonl");
        std::fs::write(&p, "{\"a\":1}\n{\"a\":2}\n").unwrap();
        let ev = verify_export("t", &p, 2).unwrap();
        assert!(ev.is_complete(), "产出的证据必须能通过 safety 的完整性检查");
        assert_eq!(ev.row_count, Some(2));
        assert_eq!(ev.export_sha256.len(), 64, "应该是完整 sha256 十六进制");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// **半截文件必须被发现** —— 这正是「回读 + 比对」相对于「文件存在」的全部价值。
    #[test]
    fn verify_export_detects_a_truncated_file() {
        let d = tmpdir("trunc");
        let p = d.join("x.jsonl");
        std::fs::write(&p, "{\"a\":1}\n").unwrap(); // 库里本该有 3 行
        let e = verify_export("t", &p, 3).unwrap_err();
        assert!(matches!(e, ExportError::Verify { .. }), "{e}");
        assert!(e.to_string().contains("导出不足就是假证据"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0 行对象的证据是**合法的空文件** —— 不是「没导」。
    #[test]
    fn verify_export_accepts_an_empty_file_for_zero_rows() {
        let d = tmpdir("zero");
        let p = d.join("x.jsonl");
        std::fs::write(&p, "").unwrap();
        let ev = verify_export("t", &p, 0).unwrap();
        assert!(ev.is_complete());
        assert_eq!(ev.row_count, Some(0));
        // 但空文件 + 期望非 0 ⇒ 必须报错（反向）
        assert!(verify_export("t", &p, 1).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn verify_export_reports_a_missing_file() {
        let d = tmpdir("miss");
        let e = verify_export("t", &d.join("nope.jsonl"), 1).unwrap_err();
        assert!(matches!(e, ExportError::Io { .. }), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 证据的 `export_path` 必须指向**真实存在**的文件（不是只填了个字符串）。
    #[test]
    fn verified_evidence_points_at_a_real_file() {
        let d = tmpdir("real");
        let p = d.join("x.jsonl");
        std::fs::write(&p, "1\n").unwrap();
        let ev = verify_export("t", &p, 1).unwrap();
        assert!(Path::new(&ev.export_path).is_file(), "{}", ev.export_path);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `ExportReport::summary` 必须能区分「导了几个对象」与「多少条无需证据」——
    /// 否则日志里的「42」会被读成「全部」。
    #[test]
    fn report_summary_distinguishes_dumped_from_no_evidence_needed() {
        let r = ExportReport {
            evidence: BTreeMap::new(),
            dumped: vec![Dumped {
                object: "t".into(),
                path: "p".into(),
                rows: 3,
                bytes: 10,
                sha256: "x".into(),
            }],
            no_evidence_needed: 670,
        };
        let s = r.summary();
        assert!(s.contains("1 个对象"), "{s}");
        assert!(s.contains("670"), "{s}");
    }
}
