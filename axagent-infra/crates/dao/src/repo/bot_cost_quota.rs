// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_cost_quotas` 仓库层（docs/bot-integration/P5/MODEL-P5.md §3）。
//!
//! 一个 scope 对象至多一行配额（`UNIQUE(scope, scope_id)`）；没有行 = 不限。
//! 全 0 上限在命令层归一为删行（IPC-P5 §1.3）。

use sea_orm::*;

use axagent_entities::bot_cost_quotas;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{BotCostQuota, BotCostQuotaInput, BotCostQuotaScope};
use axagent_harness::util_fns::{gen_id, now_ms};

fn to_dto(m: bot_cost_quotas::Model, is_default: bool) -> BotCostQuota {
    BotCostQuota {
        scope: m.scope.parse::<BotCostQuotaScope>().unwrap_or(BotCostQuotaScope::Bot),
        scope_id: m.scope_id,
        max_total_tokens: m.max_total_tokens,
        max_cost_usd: m.max_cost_usd,
        window_hours: m.window_hours,
        is_default,
        updated_at: Some(m.updated_at),
    }
}

pub async fn get_by_scope(
    db: &DatabaseConnection,
    scope: &str,
    scope_id: &str,
) -> Result<Option<BotCostQuota>> {
    let row = bot_cost_quotas::Entity::find()
        .filter(bot_cost_quotas::Column::Scope.eq(scope))
        .filter(bot_cost_quotas::Column::ScopeId.eq(scope_id))
        .one(db)
        .await?;
    Ok(row.map(|m| to_dto(m, false)))
}

pub async fn upsert(db: &DatabaseConnection, input: &BotCostQuotaInput) -> Result<BotCostQuota> {
    let now = now_ms();
    let scope = input.scope.as_str();
    let existing = bot_cost_quotas::Entity::find()
        .filter(bot_cost_quotas::Column::Scope.eq(scope))
        .filter(bot_cost_quotas::Column::ScopeId.eq(&input.scope_id))
        .one(db)
        .await?;
    match existing {
        Some(row) => {
            let mut am: bot_cost_quotas::ActiveModel = row.into();
            am.max_total_tokens = Set(input.max_total_tokens);
            am.max_cost_usd = Set(input.max_cost_usd);
            am.window_hours = Set(input.window_hours);
            am.updated_at = Set(now);
            am.update(db).await.map_err(unique_conflict)?;
        },
        None => {
            let am = bot_cost_quotas::ActiveModel {
                id: Set(gen_id()),
                scope: Set(scope.to_string()),
                scope_id: Set(input.scope_id.clone()),
                max_total_tokens: Set(input.max_total_tokens),
                max_cost_usd: Set(input.max_cost_usd),
                window_hours: Set(input.window_hours),
                created_at: Set(now),
                updated_at: Set(now),
            };
            am.insert(db).await.map_err(unique_conflict)?;
        },
    }
    get_by_scope(db, scope, &input.scope_id)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("BotCostQuota {}:{}", scope, input.scope_id)))
}

pub async fn delete_by_scope(db: &DatabaseConnection, scope: &str, scope_id: &str) -> Result<u64> {
    let res = bot_cost_quotas::Entity::delete_many()
        .filter(bot_cost_quotas::Column::Scope.eq(scope))
        .filter(bot_cost_quotas::Column::ScopeId.eq(scope_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

pub async fn list_all(db: &DatabaseConnection) -> Result<Vec<BotCostQuota>> {
    let rows = bot_cost_quotas::Entity::find()
        .order_by_asc(bot_cost_quotas::Column::Scope)
        .order_by_asc(bot_cost_quotas::Column::ScopeId)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|m| to_dto(m, false)).collect())
}

fn unique_conflict(err: DbErr) -> AxAgentError {
    let msg = err.to_string();
    if msg.contains("UNIQUE constraint failed") || msg.contains("duplicate key") {
        AxAgentError::Validation("scope+scope_id already has a cost quota row".to_string())
    } else {
        AxAgentError::Database(err)
    }
}
