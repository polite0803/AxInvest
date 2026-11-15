// SPDX-License-Identifier: AGPL-3.0-only

//! 插件表声明（`schema.json`）→ [`TableModel`]。
//!
//! 形状与判据的**权威描述**在 `PLAN-shell-plus-plugins.md` §69；本模块只做三件事：
//!
//! 1. 反序列化（`deny_unknown_fields`：拼错一个键就报错，不静默忽略）；
//! 2. 把 JSON 里的**类型 token** 换成 `sea_query::ColumnType`，再交给
//!    [`canonical_type_expected`] 归一 —— **不建第二张方言类型映射表**；
//! 3. 结构校验（表名 / 主键 / 列引用 / 默认值表达式）。
//!
//! ## 为什么不在这里做「与实体重名」检查
//!
//! 那需要完整的期望集（L1 实体 + L2 声明），属于 [`super::expected::build_with_external`]
//! 的职责。本模块只保证「单份声明自身合法」，跨声明冲突在期望侧统一拒。

use sea_orm::sea_query::ColumnType;
use serde::Deserialize;

use super::{
    extras::Dialect,
    model::{CheckModel, ColumnModel, FkModel, IndexModel, TableModel, canonical_type_expected},
};

/// 插件安装根下的固定文件名。
pub const SCHEMA_FILE_NAME: &str = "schema.json";

/// 本模块认识的文件版本。**不降级、不猜测**：将来 v2 必须显式加分支。
pub const SCHEMA_VERSION: u32 = 1;

/// 宿主簿记表的命名空间（`safety` 的审计 / 待删表都挂在这里）。
const HOST_META_PREFIX: &str = "_ax_";

/// 插件表声明的错误。
///
/// 全部带「哪一层、哪个字段」，因为这份文件来自第三方包 —— 只说「解析失败」
/// 等于让插件作者去猜。
#[derive(Debug, thiserror::Error)]
pub enum ExternalSchemaError {
    #[error("schema.json 解析失败：{0}")]
    Malformed(String),
    #[error("schema.json 版本不受支持：{0}（只接受 {SCHEMA_VERSION}）")]
    UnsupportedVersion(u32),
    #[error("{path}：{reason}")]
    Invalid { path: String, reason: String },
}

/// 一份完整的 `schema.json`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalSchema {
    pub version: u32,
    #[serde(default)]
    pub tables: Vec<ExternalTable>,
}

/// 一张插件表。字段刻意只到 v1 边界（见 PLAN §69.1）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalTable {
    pub name: String,
    pub columns: Vec<ExternalColumn>,
    /// 复合主键。与列级 `primaryKey` **互斥**。
    #[serde(default)]
    pub primary_key: Vec<String>,
    #[serde(default)]
    pub indexes: Vec<ExternalIndex>,
    #[serde(default)]
    pub checks: Vec<ExternalCheck>,
    #[serde(default)]
    pub foreign_keys: Vec<ExternalForeignKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub sql_type: String,
    /// 缺省 **false（NOT NULL）** —— 与实体侧「默认非空」一致。
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub primary_key: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub auto_increment: bool,
    /// 原样 SQL 表达式（渲染直接拼进 DDL）。
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalIndex {
    pub name: String,
    pub cols: Vec<String>,
    #[serde(default)]
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalCheck {
    pub name: String,
    pub expr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExternalForeignKey {
    #[serde(default)]
    pub name: Option<String>,
    pub cols: Vec<String>,
    pub ref_table: String,
    pub ref_cols: Vec<String>,
    #[serde(default)]
    pub on_delete: Option<String>,
    #[serde(default)]
    pub on_update: Option<String>,
}

impl ExternalSchema {
    /// 从 JSON 文本解析（不校验语义；语义校验在 [`Self::into_tables`]）。
    pub fn parse(json: &str) -> Result<Self, ExternalSchemaError> {
        let schema: Self = serde_json::from_str(json)
            .map_err(|e| ExternalSchemaError::Malformed(e.to_string()))?;
        if schema.version != SCHEMA_VERSION {
            return Err(ExternalSchemaError::UnsupportedVersion(schema.version));
        }
        Ok(schema)
    }

    /// 转成期望侧的表模型。`dialect` 决定 `sql_type` 的归一形态（见 PLAN §69.1 词表）。
    pub fn into_tables(self, dialect: Dialect) -> Result<Vec<TableModel>, ExternalSchemaError> {
        self.tables.into_iter().enumerate().map(|(i, t)| t.into_table(dialect, i)).collect()
    }
}

impl ExternalTable {
    fn into_table(
        self,
        dialect: Dialect,
        table_index: usize,
    ) -> Result<TableModel, ExternalSchemaError> {
        let table_path = format!("tables[{table_index}]");
        validate_name(&self.name, &format!("{table_path}.name"))?;
        if self.name.starts_with(HOST_META_PREFIX) {
            return Err(invalid(
                format!("{table_path}.name"),
                format!(
                    "表名 `{}` 落在宿主簿记命名空间 `{HOST_META_PREFIX}` 里 —— 插件不得声明宿主内部表",
                    self.name
                ),
            ));
        }
        if self.columns.is_empty() {
            return Err(invalid(format!("{table_path}.columns"), "至少要声明一列"));
        }

        let mut seen_columns = std::collections::BTreeSet::new();
        for (i, col) in self.columns.iter().enumerate() {
            validate_name(&col.name, &format!("{table_path}.columns[{i}].name"))?;
            if !seen_columns.insert(col.name.as_str()) {
                return Err(invalid(
                    format!("{table_path}.columns[{i}].name"),
                    format!("列名重复：`{}`", col.name),
                ));
            }
        }

        let column_level_pk: Vec<&str> =
            self.columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
        if !self.primary_key.is_empty() && !column_level_pk.is_empty() {
            return Err(invalid(
                format!("{table_path}.primaryKey"),
                "列级 `primaryKey` 与表级 `primaryKey` 只能二选一",
            ));
        }
        let primary_key: Vec<String> = match self.primary_key.is_empty() {
            true => column_level_pk.iter().map(|s| (*s).to_string()).collect(),
            false => self.primary_key.clone(),
        };

        let mut seen_pk = std::collections::BTreeSet::new();
        for (i, name) in primary_key.iter().enumerate() {
            if !seen_pk.insert(name.as_str()) {
                return Err(invalid(
                    format!("{table_path}.primaryKey[{i}]"),
                    format!("主键列重复：`{name}`"),
                ));
            }
            let Some(col) = self.columns.iter().find(|c| &c.name == name) else {
                return Err(invalid(
                    format!("{table_path}.primaryKey[{i}]"),
                    format!("主键列 `{name}` 不存在"),
                ));
            };
            if col.nullable {
                return Err(invalid(
                    format!("{table_path}.primaryKey[{i}]"),
                    format!("主键列 `{name}` 声明为 nullable —— 主键隐含 NOT NULL"),
                ));
            }
        }

        let single_pk = match primary_key.len() {
            1 => Some(primary_key[0].clone()),
            _ => None,
        };
        let mut table = TableModel::new(self.name.clone());
        table.primary_key = primary_key.clone();

        for (i, col) in self.columns.iter().enumerate() {
            let path = format!("{table_path}.columns[{i}]");
            let column_type = parse_column_type(&col.sql_type)
                .map_err(|reason| invalid(format!("{path}.type"), reason))?;
            reject_unsupported_type(&column_type, dialect, &path)?;
            if col.auto_increment {
                if !matches!(
                    &column_type,
                    ColumnType::SmallInteger | ColumnType::Integer | ColumnType::BigInteger
                ) {
                    return Err(invalid(
                        // 错误路径名用 snake（ 会逐字撞 check-sql-dialect 的 AUTOINCREMENT 规则，
                        // 而这里只是 JSON 键路径、不是 SQL 语法）。
                        format!("{path}.auto_increment"),
                        format!("只有整数族可自增，`{}` 不支持", col.sql_type),
                    ));
                }
                if col.default.is_some() {
                    return Err(invalid(
                        format!("{path}.default"),
                        "自增列不能同时声明默认值（渲染会撞 `multiple default values`）",
                    ));
                }
            }
            if let Some(expr) = &col.default {
                validate_expr(expr, &format!("{path}.default"))?;
            }

            let mut model = ColumnModel::new(
                col.name.clone(),
                canonical_type_expected(&column_type, dialect),
                col.nullable,
            );
            model.primary_key = single_pk.as_deref() == Some(col.name.as_str());
            model.unique = col.unique;
            model.auto_increment = col.auto_increment;
            model.default = col.default.clone();
            table.upsert_column(model);
        }

        let mut seen_indexes = std::collections::BTreeSet::new();
        for (i, idx) in self.indexes.iter().enumerate() {
            let path = format!("{table_path}.indexes[{i}]");
            validate_name(&idx.name, &format!("{path}.name"))?;
            if !seen_indexes.insert(idx.name.as_str()) {
                return Err(invalid(format!("{path}.name"), format!("索引名重复：`{}`", idx.name)));
            }
            if idx.cols.is_empty() {
                return Err(invalid(format!("{path}.cols"), "索引至少要一列"));
            }
            for (j, col) in idx.cols.iter().enumerate() {
                let col_name = index_column_name(col)
                    .map_err(|reason| invalid(format!("{path}.cols[{j}]"), reason))?;
                require_column(&self.columns, col_name, &format!("{path}.cols[{j}]"))?;
            }
            table.upsert_index(IndexModel::new(idx.name.clone(), idx.cols.clone(), idx.unique));
        }

        let mut seen_checks = std::collections::BTreeSet::new();
        for (i, ck) in self.checks.iter().enumerate() {
            let path = format!("{table_path}.checks[{i}]");
            validate_name(&ck.name, &format!("{path}.name"))?;
            if !seen_checks.insert(ck.name.as_str()) {
                return Err(invalid(
                    format!("{path}.name"),
                    format!("CHECK 名重复：`{}`", ck.name),
                ));
            }
            validate_expr(&ck.expr, &format!("{path}.expr"))?;
            table.upsert_check(CheckModel::new(ck.name.clone(), ck.expr.clone()));
        }

        let mut seen_fks = std::collections::BTreeSet::new();
        for (i, fk) in self.foreign_keys.iter().enumerate() {
            let path = format!("{table_path}.foreignKeys[{i}]");
            validate_name(&fk.ref_table, &format!("{path}.refTable"))?;
            if fk.cols.is_empty() || fk.ref_cols.is_empty() {
                return Err(invalid(path.clone(), "外键的 `cols` 与 `refCols` 都不能为空"));
            }
            if fk.cols.len() != fk.ref_cols.len() {
                return Err(invalid(
                    path.clone(),
                    format!(
                        "外键列数不一致：cols={} / refCols={}",
                        fk.cols.len(),
                        fk.ref_cols.len()
                    ),
                ));
            }
            for (j, col) in fk.cols.iter().enumerate() {
                require_column(&self.columns, col, &format!("{path}.cols[{j}]"))?;
            }
            for (j, action) in
                [("onDelete", fk.on_delete.as_deref()), ("onUpdate", fk.on_update.as_deref())]
                    .into_iter()
            {
                if let Some(raw) = action {
                    parse_fk_action(raw)
                        .map_err(|reason| invalid(format!("{path}.{j}"), reason))?;
                }
            }
            let identity =
                format!("{}:{}:{}", fk.cols.join(","), fk.ref_table, fk.ref_cols.join(","));
            if !seen_fks.insert(identity.clone()) {
                return Err(invalid(path, "同一条外键声明了两次"));
            }
            table.fks.push(FkModel {
                name: fk.name.clone(),
                cols: fk.cols.clone(),
                ref_table: fk.ref_table.clone(),
                ref_cols: fk.ref_cols.clone(),
                on_delete: fk
                    .on_delete
                    .as_deref()
                    .and_then(|a| parse_fk_action(a).ok())
                    .map(str::to_string),
                on_update: fk
                    .on_update
                    .as_deref()
                    .and_then(|a| parse_fk_action(a).ok())
                    .map(str::to_string),
            });
        }

        Ok(table)
    }
}

fn invalid(path: impl Into<String>, reason: impl Into<String>) -> ExternalSchemaError {
    ExternalSchemaError::Invalid { path: path.into(), reason: reason.into() }
}

fn validate_name(name: &str, path: &str) -> Result<(), ExternalSchemaError> {
    if name.trim().is_empty() {
        return Err(invalid(path, "名字不能为空"));
    }
    if name != name.trim() {
        return Err(invalid(path, "名字首尾不能有空白"));
    }
    if name.contains('\0') {
        return Err(invalid(path, "名字不能含 NUL"));
    }
    Ok(())
}

fn require_column(
    columns: &[ExternalColumn],
    name: &str,
    path: &str,
) -> Result<(), ExternalSchemaError> {
    match columns.iter().any(|c| c.name == name) {
        true => Ok(()),
        false => Err(invalid(path, format!("列 `{name}` 不在本表声明里"))),
    }
}

/// 解析 JSON 的类型 token。
///
/// **只此一处**把 token 变成 `ColumnType`；方言归一仍由
/// [`canonical_type_expected`] 负责（单一权威，见模块文档）。
fn parse_column_type(token: &str) -> Result<ColumnType, String> {
    let raw = token.trim();
    let lowered = raw.to_ascii_lowercase();
    let (base, args) = match split_type_args(&lowered) {
        Some((base, args)) => (base, Some(args)),
        None => (lowered.as_str(), None),
    };
    let no_args = |name: &str| -> Result<(), String> {
        match args {
            Some(_) => Err(format!("`{name}` 不接受长度/精度参数")),
            None => Ok(()),
        }
    };
    match base {
        "text" => no_args("text").map(|()| ColumnType::Text),
        "varchar" => match args.as_deref() {
            Some(n) => parse_single_len(n)
                .map(|n| ColumnType::String(sea_orm::sea_query::StringLen::N(n)))
                .map_err(|e| format!("varchar({n})：{e}")),
            None => Ok(ColumnType::String(sea_orm::sea_query::StringLen::None)),
        },
        "char" => match args.as_deref() {
            Some(n) => parse_single_len(n)
                .map(|n| ColumnType::Char(Some(n)))
                .map_err(|e| format!("char({n})：{e}")),
            None => Ok(ColumnType::Char(None)),
        },
        "smallint" => no_args("smallint").map(|()| ColumnType::SmallInteger),
        "integer" | "int" => no_args("integer").map(|()| ColumnType::Integer),
        "bigint" => no_args("bigint").map(|()| ColumnType::BigInteger),
        "real" => no_args("real").map(|()| ColumnType::Float),
        "double" | "double precision" => no_args("double").map(|()| ColumnType::Double),
        "numeric" | "decimal" => match args.as_deref() {
            Some(a) => parse_precision_scale(a)
                .map(|ps| ColumnType::Decimal(Some(ps)))
                .map_err(|e| format!("numeric({a})：{e}")),
            None => Ok(ColumnType::Decimal(None)),
        },
        "boolean" | "bool" => no_args("boolean").map(|()| ColumnType::Boolean),
        "blob" | "bytea" => no_args("blob").map(|()| ColumnType::Blob),
        "timestamp" => no_args("timestamp").map(|()| ColumnType::Timestamp),
        "timestamptz" | "timestamp with time zone" => {
            no_args("timestamptz").map(|()| ColumnType::TimestampWithTimeZone)
        },
        "date" => no_args("date").map(|()| ColumnType::Date),
        "time" => no_args("time").map(|()| ColumnType::Time),
        "json" => no_args("json").map(|()| ColumnType::Json),
        "jsonb" => no_args("jsonb").map(|()| ColumnType::JsonBinary),
        "uuid" => no_args("uuid").map(|()| ColumnType::Uuid),
        "vector" => match args.as_deref() {
            Some(a) => parse_single_len(a)
                .map(|n| ColumnType::Vector(Some(n)))
                .map_err(|e| format!("vector({a})：{e}")),
            None => Ok(ColumnType::Vector(None)),
        },
        other => Err(format!("未知类型 `{other}`（词表见 PLAN §69.1）")),
    }
}

fn reject_unsupported_type(
    column_type: &ColumnType,
    dialect: Dialect,
    path: &str,
) -> Result<(), ExternalSchemaError> {
    if dialect == Dialect::Sqlite && matches!(column_type, ColumnType::Vector(_)) {
        return Err(invalid(
            format!("{path}.type"),
            "SQLite 渲染器没有 vector 类型（声明期拒绝，避免执行期 panic）",
        ));
    }
    Ok(())
}

fn split_type_args(s: &str) -> Option<(&str, String)> {
    let open = s.find('(')?;
    if !s.ends_with(')') {
        return None;
    }
    Some((&s[..open], s[open + 1..s.len() - 1].replace(' ', "")))
}

fn parse_single_len(raw: &str) -> Result<u32, String> {
    raw.parse::<u32>().map_err(|_| "参数必须是非负整数".to_string())
}

fn parse_precision_scale(raw: &str) -> Result<(u32, u32), String> {
    let (p, s) = raw.split_once(',').ok_or_else(|| "参数必须是 `(精度,标度)`".to_string())?;
    let p = parse_single_len(p)?;
    let s = parse_single_len(s)?;
    if s > p {
        return Err("标度不能大于精度".to_string());
    }
    Ok((p, s))
}

/// 索引列：v1 只接受**裸列名**（排序方向属于声明面扩展，见 PLAN §69.5）。
fn index_column_name(raw: &str) -> Result<&str, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Err("索引列名不能为空".to_string());
    }
    if t != raw {
        return Err("索引列名首尾不能有空白".to_string());
    }
    if t.split_whitespace().count() > 1 {
        return Err(format!("v1 索引列只接受裸列名（不支持排序方向）：`{raw}`"));
    }
    Ok(t)
}

fn parse_fk_action(raw: &str) -> Result<&'static str, String> {
    let normalized = raw.trim().to_ascii_lowercase().replace(['_', ' '], "");
    match normalized.as_str() {
        "cascade" => Ok("Cascade"),
        "setnull" => Ok("SetNull"),
        "restrict" => Ok("Restrict"),
        "noaction" => Ok("NoAction"),
        "setdefault" => Ok("SetDefault"),
        other => Err(format!(
            "外键动作 `{other}` 不在词表内（Cascade / SetNull / Restrict / NoAction / SetDefault）"
        )),
    }
}

/// 结构校验：非空、无语句分隔/注释标记、引号与括号配平。
///
/// **不是 SQL 防火墙** —— 插件本来就是可执行包；这里只保证表达式不会把
/// 整条 `CREATE TABLE` 拆成两条，或留下一个永不闭合的引号。
fn validate_expr(raw: &str, path: &str) -> Result<(), ExternalSchemaError> {
    let expr = raw.trim();
    if expr.is_empty() {
        return Err(invalid(path, "表达式不能为空"));
    }
    if expr.contains(';') {
        return Err(invalid(path, "表达式不能含 `;`"));
    }
    if expr.contains("--") || expr.contains("/*") || expr.contains("*/") {
        return Err(invalid(path, "表达式不能含 SQL 注释标记"));
    }

    let mut depth = 0i32;
    let mut in_str = false;
    let mut chars = expr.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_str {
            if ch == '\'' {
                // SQL 的转义写法：`''` 表示一个引号本身，不结束字符串。
                if chars.peek() == Some(&'\'') {
                    let _ = chars.next();
                } else {
                    in_str = false;
                }
            }
            continue;
        }
        match ch {
            '\'' => in_str = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(invalid(path, "括号不配平"));
                }
            },
            _ => {},
        }
    }
    if in_str {
        return Err(invalid(path, "引号未闭合"));
    }
    if depth != 0 {
        return Err(invalid(path, "括号不配平"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::model::canonical_type_actual;

    fn sample() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "tables": [{
                "name": "plugin_stock_watch",
                "columns": [
                    {"name": "id", "type": "bigint", "primaryKey": true, "autoIncrement": true},
                    {"name": "code", "type": "text"},
                    {"name": "score", "type": "double", "nullable": true},
                    {"name": "created_at", "type": "timestamp", "default": "CURRENT_TIMESTAMP"}
                ],
                "indexes": [
                    {"name": "idx_plugin_stock_watch_code", "cols": ["code"], "unique": false}
                ],
                "checks": [
                    {"name": "ck_plugin_stock_watch_score", "expr": "score >= 0"}
                ],
                "foreignKeys": []
            }]
        })
    }

    fn parse(value: &serde_json::Value) -> Result<ExternalSchema, ExternalSchemaError> {
        ExternalSchema::parse(&value.to_string())
    }

    fn one_table(value: &serde_json::Value, dialect: Dialect) -> TableModel {
        parse(value)
            .expect("示例声明应可解析")
            .into_tables(dialect)
            .expect("示例声明应可转换")
            .remove(0)
    }

    fn expect_invalid(value: &serde_json::Value, needle: &str) {
        let err = parse(value)
            .expect("JSON 本身应合法")
            .into_tables(Dialect::Postgres)
            .expect_err("该声明应被硬判据拒绝");
        let text = err.to_string();
        assert!(text.contains(needle), "错误信息应含 `{needle}`，实际是：{text}");
    }

    /// 类型词表逐行钉住：token → 期望侧归一串 → 实况侧再归一**必须回到同一个串**。
    ///
    /// 这是 PLAN §69.1 那张表的可执行版本；两边任一格漂了，这条测试先红。
    #[test]
    fn type_vocabulary_round_trips_through_actual_normalization() {
        let tokens = [
            "text",
            "varchar(32)",
            "char(2)",
            "smallint",
            "integer",
            "bigint",
            "real",
            "double",
            "numeric",
            "numeric(10,2)",
            "boolean",
            "blob",
            "timestamp",
            "timestamptz",
            "date",
            "time",
            "json",
            "jsonb",
            "uuid",
        ];
        for dialect in [Dialect::Postgres, Dialect::Sqlite] {
            for token in tokens {
                let ct = parse_column_type(token).unwrap_or_else(|e| panic!("{token}: {e}"));
                let expected = canonical_type_expected(&ct, dialect);
                let actual = canonical_type_actual(&expected, dialect);
                assert_eq!(
                    actual, expected,
                    "{dialect:?} 下 `{token}` 的归一串 `{expected}` 回读成了 `{actual}`"
                );
            }
        }
        // 同一 token 在两个方言下的期望串也逐条钉住（防止「两边一起错」）。
        assert_eq!(
            canonical_type_expected(&parse_column_type("boolean").unwrap(), Dialect::Postgres),
            "boolean"
        );
        assert_eq!(
            canonical_type_expected(&parse_column_type("boolean").unwrap(), Dialect::Sqlite),
            "integer"
        );
        assert_eq!(
            canonical_type_expected(&parse_column_type("double").unwrap(), Dialect::Postgres),
            "double precision"
        );
        assert_eq!(
            canonical_type_expected(&parse_column_type("double").unwrap(), Dialect::Sqlite),
            "real"
        );
        assert_eq!(
            canonical_type_expected(&parse_column_type("timestamp").unwrap(), Dialect::Sqlite),
            "text"
        );
        assert_eq!(
            canonical_type_expected(
                &parse_column_type("numeric(10,2)").unwrap(),
                Dialect::Postgres
            ),
            "numeric(10,2)"
        );
        assert_eq!(
            canonical_type_expected(&parse_column_type("numeric(10,2)").unwrap(), Dialect::Sqlite),
            "real"
        );
    }

    #[test]
    fn sample_schema_becomes_the_expected_table_model() {
        let table = one_table(&sample(), Dialect::Postgres);
        assert_eq!(table.name, "plugin_stock_watch");
        assert_eq!(table.primary_key, vec!["id".to_string()]);
        let id = table.column("id").expect("id 列");
        assert!(id.primary_key);
        assert!(id.auto_increment);
        assert!(!id.nullable);
        assert_eq!(id.sql_type, "bigint");
        let code = table.column("code").expect("code 列");
        assert!(!code.nullable, "nullable 缺省必须是 NOT NULL");
        let score = table.column("score").expect("score 列");
        assert!(score.nullable);
        assert_eq!(score.sql_type, "double precision");
        assert_eq!(
            table.column("created_at").and_then(|c| c.default.as_deref()),
            Some("CURRENT_TIMESTAMP")
        );
        assert_eq!(table.indexes.len(), 1);
        assert_eq!(table.checks.len(), 1);
    }

    #[test]
    fn sqlite_uses_affinity_classes() {
        let table = one_table(&sample(), Dialect::Sqlite);
        assert_eq!(table.column("id").expect("id").sql_type, "integer");
        assert_eq!(table.column("score").expect("score").sql_type, "real");
        assert_eq!(table.column("created_at").expect("created_at").sql_type, "text");
    }

    #[test]
    fn composite_primary_key_uses_table_level_only() {
        let mut value = sample();
        let table = &mut value["tables"][0];
        table["columns"] = serde_json::json!([
            {"name": "a", "type": "text"},
            {"name": "b", "type": "integer"}
        ]);
        table["primaryKey"] = serde_json::json!(["a", "b"]);
        table["indexes"] = serde_json::json!([]);
        table["checks"] = serde_json::json!([]);
        let model = one_table(&value, Dialect::Postgres);
        assert_eq!(model.primary_key, vec!["a".to_string(), "b".to_string()]);
        assert!(!model.column("a").expect("a").primary_key);
        assert!(!model.column("b").expect("b").primary_key);
    }

    #[test]
    fn unknown_version_and_unknown_fields_are_rejected() {
        let mut value = sample();
        value["version"] = serde_json::json!(2);
        let err = parse(&value).expect_err("未知版本必须拒绝");
        assert!(matches!(err, ExternalSchemaError::UnsupportedVersion(2)));

        let mut value = sample();
        value["tables"][0]["surprise"] = serde_json::json!(true);
        let err = parse(&value).expect_err("未知键必须拒绝");
        assert!(matches!(err, ExternalSchemaError::Malformed(_)), "实际：{err}");
    }

    #[test]
    fn host_meta_namespace_is_rejected() {
        let mut value = sample();
        value["tables"][0]["name"] = serde_json::json!("_ax_schema_pending_drops");
        expect_invalid(&value, "_ax_");
    }

    #[test]
    fn two_primary_key_forms_are_mutually_exclusive() {
        let mut value = sample();
        value["tables"][0]["primaryKey"] = serde_json::json!(["id"]);
        expect_invalid(&value, "二选一");
    }

    #[test]
    fn nullable_primary_key_is_rejected() {
        let mut value = sample();
        value["tables"][0]["columns"][0]["nullable"] = serde_json::json!(true);
        expect_invalid(&value, "主键隐含 NOT NULL");
    }

    #[test]
    fn index_and_foreign_key_columns_must_exist() {
        let mut value = sample();
        value["tables"][0]["indexes"][0]["cols"] = serde_json::json!(["nope"]);
        expect_invalid(&value, "不在本表声明里");

        let mut value = sample();
        value["tables"][0]["foreignKeys"] = serde_json::json!([{
            "cols": ["nope"], "refTable": "other", "refCols": ["id"]
        }]);
        expect_invalid(&value, "不在本表声明里");

        let mut value = sample();
        value["tables"][0]["foreignKeys"] = serde_json::json!([{
            "cols": ["code"], "refTable": "other", "refCols": ["a", "b"]
        }]);
        expect_invalid(&value, "列数不一致");
    }

    #[test]
    fn auto_increment_is_integer_only_and_excludes_default() {
        let mut value = sample();
        value["tables"][0]["columns"][1]["autoIncrement"] = serde_json::json!(true);
        expect_invalid(&value, "只有整数族可自增");

        let mut value = sample();
        value["tables"][0]["columns"][0]["default"] = serde_json::json!("0");
        expect_invalid(&value, "自增列不能同时声明默认值");
    }

    #[test]
    fn default_expressions_are_structurally_checked() {
        for bad in ["", "1; DROP TABLE x", "'oops", "(a"] {
            let mut value = sample();
            value["tables"][0]["columns"][3]["default"] = serde_json::json!(bad);
            expect_invalid(&value, "default");
        }
        // `''` 是合法转义，不能被当成未闭合。
        let mut value = sample();
        value["tables"][0]["columns"][3]["default"] = serde_json::json!("'it''s ok'");
        let _ = one_table(&value, Dialect::Postgres);
    }

    #[test]
    fn foreign_key_actions_are_whitelisted() {
        let mut value = sample();
        value["tables"][0]["foreignKeys"] = serde_json::json!([{
            "cols": ["code"], "refTable": "other", "refCols": ["id"],
            "onDelete": "explode"
        }]);
        expect_invalid(&value, "不在词表内");

        let mut value = sample();
        value["tables"][0]["foreignKeys"] = serde_json::json!([{
            "cols": ["code"], "refTable": "other", "refCols": ["id"],
            "onDelete": "set null", "onUpdate": "CASCADE"
        }]);
        let table = one_table(&value, Dialect::Postgres);
        assert_eq!(table.fks[0].on_delete.as_deref(), Some("SetNull"));
        assert_eq!(table.fks[0].on_update.as_deref(), Some("Cascade"));
    }

    #[test]
    fn vector_is_rejected_on_sqlite_only() {
        let mut value = sample();
        value["tables"][0]["columns"].as_array_mut().unwrap().push(serde_json::json!({
            "name": "embedding", "type": "vector(1536)", "nullable": true
        }));
        let _ = one_table(&value, Dialect::Postgres);
        let err = parse(&value)
            .expect("JSON 合法")
            .into_tables(Dialect::Sqlite)
            .expect_err("SQLite 必须拒绝 vector");
        assert!(err.to_string().contains("vector"), "实际：{err}");
    }

    #[test]
    fn duplicate_columns_and_index_names_are_rejected() {
        let mut value = sample();
        value["tables"][0]["columns"].as_array_mut().unwrap().push(serde_json::json!({
            "name": "code", "type": "text"
        }));
        expect_invalid(&value, "列名重复");

        let mut value = sample();
        let idx = value["tables"][0]["indexes"][0].clone();
        value["tables"][0]["indexes"].as_array_mut().unwrap().push(idx);
        expect_invalid(&value, "索引名重复");
    }

    #[test]
    fn index_sort_direction_is_out_of_v1_scope() {
        let mut value = sample();
        value["tables"][0]["indexes"][0]["cols"] = serde_json::json!(["code DESC"]);
        expect_invalid(&value, "不支持排序方向");
    }
}
