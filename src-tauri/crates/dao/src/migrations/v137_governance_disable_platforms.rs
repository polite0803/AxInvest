// SPDX-License-Identifier: AGPL-3.0-only
//! v137: 数据源治理 —— 默认禁用无公开检索 API / 需官方凭证的平台。
//!
//! ## Background
//!
//! 18 个内置平台全部默认启用（缺陷复审 `output/demand-discovery-defects-
//! 2026-09-02.md` P1-1），但真正默认可用的免费源只有 8 个（HackerNews /
//! GitHub Issue+Discussion / StackOverflow / arXiv / HuggingFace /
//! package_ecosystem / Reddit 存疑）。其余 10 个（Twitter / LinkedIn /
//! CSDN / 掘金 / Dribbble / ProductHunt / Upwork / 知乎 / 猪八戒 / 闲鱼）
//! 无公开检索 API 或需凭证，永远「合规跳过」或 404，却每轮各占并发额度
//! 并刷同步状态。
//!
//! ## 治理语义
//!
//! - 对上述 10 个平台：**仅在 `config_json` 未配置非空 `api_token` 时**
//!   置 `enabled = 0`。用户已配好凭证的平台不受影响（种子默认禁用不回填
//!   存量，但这里主动治理存量 —— 前提是没有 token，即本来就跑不通）。
//! - 新装环境的默认值由 `repo::opc_demand::DEFAULT_PLATFORMS` 的第三元
//!   （default_enabled）控制，本迁移只处理存量行。
//! - 幂等：重复执行时已禁用行再次置 0 无副作用。

use sea_orm::ConnectionTrait;
use sea_orm::DbBackend;
use sea_orm::DbErr;

/// 需治理的平台（无公开检索 API 或需凭证；与 DEFAULT_PLATFORMS 的
/// default_enabled = false 清单一致，不含 ProductHunt —— 它可自申请 token，
/// 但同样无凭证时默认跑不通，一并治理）
const CREDENTIAL_REQUIRED_PLATFORMS: &[&str] = &[
    "producthunt",
    "twitter",
    "zhubajie",
    "xianyu",
    "linkedin",
    "zhihu",
    "csdn",
    "juejin",
    "dribbble",
    "upwork",
];

/// 判断 config_json 是否配置了非空 api_token
fn has_api_token(config_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(config_json)
        .ok()
        .and_then(|v| {
            v.get("api_token").and_then(|t| t.as_str()).map(str::trim).map(|t| !t.is_empty())
        })
        .unwrap_or(false)
}

pub async fn up(db: sea_orm::DatabaseConnection) -> Result<(), DbErr> {
    let is_pg = db.get_database_backend() == DbBackend::Postgres;
    let placeholder = |i: usize| {
        if is_pg {
            format!("${i}")
        } else {
            format!("?{i}")
        }
    };
    let in_list = CREDENTIAL_REQUIRED_PLATFORMS
        .iter()
        .enumerate()
        .map(|(i, _)| placeholder(i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    // 逐行判断：只禁用未配置 token 的行（SELECT 后逐行 UPDATE，保证
    // token 判断用 serde 解析而非 SQL JSON 函数 —— SQLite 无 JSON1 保证）
    let select_sql =
        format!("SELECT id, config_json FROM opc_demand_platforms WHERE id IN ({in_list})");
    let stmt = if is_pg {
        sea_orm::Statement::from_sql_and_values(
            DbBackend::Postgres,
            &select_sql,
            CREDENTIAL_REQUIRED_PLATFORMS.iter().map(|p| (*p).into()),
        )
    } else {
        sea_orm::Statement::from_sql_and_values(
            DbBackend::Sqlite,
            &select_sql,
            CREDENTIAL_REQUIRED_PLATFORMS.iter().map(|p| (*p).into()),
        )
    };
    let rows = db.query_all_raw(stmt).await?;

    let now = axagent_harness::util_fns::now_ts();
    let mut disabled = 0usize;
    for row in rows {
        let id: String = row.try_get_by("id")?;
        let config_json: String = row.try_get_by("config_json").unwrap_or_default();
        if has_api_token(&config_json) {
            continue;
        }
        let update_sql = format!(
            "UPDATE opc_demand_platforms SET enabled = 0, updated_at = {} WHERE id = '{}'",
            now,
            id.replace('\'', "''")
        );
        db.execute_unprepared(&update_sql).await?;
        disabled += 1;
    }

    tracing::info!(
        total = CREDENTIAL_REQUIRED_PLATFORMS.len(),
        disabled,
        "[v137] opc_demand_platforms: 已禁用未配置凭证的摆设平台"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;
    use sea_orm::Statement;

    async fn setup(db: &sea_orm::DatabaseConnection) {
        crate::migrations::v133_opc_demand_discovery::up(db.clone()).await.unwrap();
        crate::migrations::v134_lead_workflow_link::up(db.clone()).await.unwrap();
    }

    #[tokio::test]
    async fn v137_disables_only_tokenless_rows_sqlite() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        setup(&db).await;

        // 三种存量：无 token（应禁用）/ 空 token（应禁用）/ 已配 token（保留）
        let now = 1_700_000_000i64;
        let cases = [
            ("twitter", "{\"auto_sync\":true}"),
            ("upwork", "{\"api_token\":\"\"}"),
            ("linkedin", "{\"api_token\":\"tok-1\"}"),
        ];
        for (id, config) in cases {
            let sql = format!(
                "INSERT INTO opc_demand_platforms (id, name, platform_type, enabled, \
                 config_json, status, created_at, updated_at) \
                 VALUES ('{id}', '{id}', 'scanner', 1, '{config}', 'idle', {now}, {now})"
            );
            db.execute_unprepared(&sql).await.unwrap();
        }

        up(db.clone()).await.unwrap();

        let row = db
            .query_one_raw(Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM opc_demand_platforms \
                 WHERE id IN ('twitter','upwork') AND enabled = 0"
                    .to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by::<i64, _>("n").unwrap(), 2, "无 token 的平台应被禁用");

        let row = db
            .query_one_raw(Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT enabled FROM opc_demand_platforms WHERE id = 'linkedin'".to_string(),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by::<i32, _>("enabled").unwrap(), 1, "已配 token 的平台应保留");

        // 幂等
        up(db).await.unwrap();
    }
}
