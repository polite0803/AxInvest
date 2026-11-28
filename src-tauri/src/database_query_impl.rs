// SPDX-License-Identifier: AGPL-3.0-only

//! DatabaseQueryService 的 sqlx 实现。连接外部数据库并执行通用 SQL 查询。

use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::{DatabaseQueryResult, DatabaseQueryService};
use sqlx::Column;
use sqlx::Row;
use sqlx::any::AnyPoolOptions;

pub struct SqlxDatabaseQueryService;

/// 把 workflow 配置里的连接串归一化成本仓支持的形式。
///
/// 只放行 SQLite 与 PostgreSQL：
/// - `postgres://` / `postgresql://` 原样返回；
/// - `sqlite:`（含 `sqlite::memory:`）原样返回，`sqlite://` 也归到这一类；
/// - 裸文件路径视作 SQLite，补上 `sqlite:` 前缀；
/// - 其它带 `scheme://` 的连接串（尤其是 `mysql://`）一律拒绝，避免让未编进
///   宿主的驱动伪装成“可以连接”。
fn normalize_supported_connection_string(conn_str: &str) -> std::result::Result<String, String> {
    let raw = conn_str.trim();
    if raw.is_empty() {
        return Err("databaseQuery 连接串为空".to_string());
    }

    if raw.starts_with("postgres://") || raw.starts_with("postgresql://") {
        return Ok(raw.to_string());
    }

    // `sqlite://path` 在 sqlx 里等价于 `sqlite:path`；统一去掉 `//` 可避免
    // 两套写法在后续路径处理上分叉。
    if let Some(rest) = raw.strip_prefix("sqlite://") {
        return Ok(format!("sqlite:{rest}"));
    }
    if raw.starts_with("sqlite:") {
        return Ok(raw.to_string());
    }

    if let Some(scheme) = raw.split_once("://").map(|(scheme, _)| scheme) {
        return Err(format!(
            "databaseQuery 不支持连接串 scheme `{scheme}`：本构建只支持 sqlite 和 postgresql"
        ));
    }

    Ok(format!("sqlite:{raw}"))
}

impl SqlxDatabaseQueryService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SqlxDatabaseQueryService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DatabaseQueryService for SqlxDatabaseQueryService {
    async fn execute_query(&self, conn_str: &str, query: &str) -> Result<DatabaseQueryResult> {
        let conn_str =
            normalize_supported_connection_string(conn_str).map_err(AxAgentError::internal)?;

        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .connect(&conn_str)
            .await
            .map_err(|e| AxAgentError::internal(e.to_string()))?;

        // sqlx::query 需要 &'static str，使用 leak 延长生命周期
        let query_static: &'static str = query.to_string().leak();
        let rows = sqlx::query(query_static)
            .fetch_all(&pool)
            .await
            .map_err(|e| AxAgentError::internal(format!("Database query failed: {}", e)))?;

        let mut columns = Vec::new();
        if let Some(first_row) = rows.first() {
            for col in first_row.columns() {
                columns.push(col.name().to_string());
            }
        }

        let mut result_rows: Vec<Vec<serde_json::Value>> = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut vals: Vec<serde_json::Value> = Vec::with_capacity(columns.len());
            for i in 0..columns.len() {
                let val = if let Ok(v) = row.try_get::<i64, _>(i) {
                    serde_json::json!(v)
                } else if let Ok(v) = row.try_get::<f64, _>(i) {
                    serde_json::json!(v)
                } else if let Ok(v) = row.try_get::<String, _>(i) {
                    serde_json::json!(v)
                } else if let Ok(v) = row.try_get::<bool, _>(i) {
                    serde_json::json!(v)
                } else {
                    serde_json::Value::Null
                };
                vals.push(val);
            }
            result_rows.push(vals);
        }

        Ok(DatabaseQueryResult { columns, rows: result_rows })
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_supported_connection_string;

    #[test]
    fn database_query_accepts_sqlite_and_postgresql_connection_strings() {
        assert_eq!(
            normalize_supported_connection_string("sqlite::memory:").unwrap(),
            "sqlite::memory:"
        );
        assert_eq!(
            normalize_supported_connection_string("sqlite:/tmp/axagent.db").unwrap(),
            "sqlite:/tmp/axagent.db"
        );
        assert_eq!(
            // `sqlite://` 与 `sqlite:` 归一到同一形式，避免同库两套路径语义。
            normalize_supported_connection_string("sqlite:///tmp/axagent.db").unwrap(),
            "sqlite:/tmp/axagent.db"
        );
        assert_eq!(
            normalize_supported_connection_string(r"C:\data\axagent.db").unwrap(),
            r"sqlite:C:\data\axagent.db"
        );
        assert_eq!(
            normalize_supported_connection_string("postgres://u:p@localhost:5432/db").unwrap(),
            "postgres://u:p@localhost:5432/db"
        );
        assert_eq!(
            normalize_supported_connection_string("postgresql://u@localhost/db").unwrap(),
            "postgresql://u@localhost/db"
        );
    }

    #[test]
    fn database_query_rejects_mysql_and_unknown_scheme() {
        assert!(normalize_supported_connection_string("mysql://u:p@localhost/db").is_err());
        assert!(normalize_supported_connection_string("mariadb://u:p@localhost/db").is_err());
        assert!(normalize_supported_connection_string("mongodb://localhost/db").is_err());
        assert!(normalize_supported_connection_string("   ").is_err());
    }
}
