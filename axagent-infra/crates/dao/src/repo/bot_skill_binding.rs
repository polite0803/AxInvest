// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_skill_bindings` 仓库层（docs/bot-integration/P5/MODEL-P5.md §4）。
//!
//! 目录是真相源，本表只作归属声明与审计（ADR-P5-5）。

use sea_orm::sea_query::Expr;
use sea_orm::*;

use axagent_entities::bot_skill_bindings;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::BotSkillBinding;
use axagent_harness::util_fns::{gen_id, now_ms};

fn to_dto(m: bot_skill_bindings::Model) -> BotSkillBinding {
    BotSkillBinding {
        id: m.id,
        bot_id: m.bot_id,
        skill_name: m.skill_name,
        source: m.source,
        preset_id: m.preset_id,
        enabled: m.enabled,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

pub async fn list_by_bot(db: &DatabaseConnection, bot_id: &str) -> Result<Vec<BotSkillBinding>> {
    let rows = bot_skill_bindings::Entity::find()
        .filter(bot_skill_bindings::Column::BotId.eq(bot_id))
        .order_by_asc(bot_skill_bindings::Column::SkillName)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(to_dto).collect())
}

pub async fn upsert(
    db: &DatabaseConnection,
    bot_id: &str,
    skill_name: &str,
    source: &str,
    preset_id: Option<&str>,
    enabled: bool,
) -> Result<BotSkillBinding> {
    let now = now_ms();
    let existing = bot_skill_bindings::Entity::find()
        .filter(bot_skill_bindings::Column::BotId.eq(bot_id))
        .filter(bot_skill_bindings::Column::SkillName.eq(skill_name))
        .one(db)
        .await?;
    match existing {
        Some(row) => {
            let mut am: bot_skill_bindings::ActiveModel = row.into();
            am.source = Set(source.to_string());
            am.preset_id = Set(preset_id.map(str::to_string));
            am.enabled = Set(enabled);
            am.updated_at = Set(now);
            am.update(db).await.map_err(unique_conflict)?;
        },
        None => {
            let am = bot_skill_bindings::ActiveModel {
                id: Set(gen_id()),
                bot_id: Set(bot_id.to_string()),
                skill_name: Set(skill_name.to_string()),
                source: Set(source.to_string()),
                preset_id: Set(preset_id.map(str::to_string)),
                enabled: Set(enabled),
                created_at: Set(now),
                updated_at: Set(now),
            };
            am.insert(db).await.map_err(unique_conflict)?;
        },
    }
    let row = bot_skill_bindings::Entity::find()
        .filter(bot_skill_bindings::Column::BotId.eq(bot_id))
        .filter(bot_skill_bindings::Column::SkillName.eq(skill_name))
        .one(db)
        .await?
        .ok_or_else(|| {
            AxAgentError::NotFound(format!("BotSkillBinding {}:{}", bot_id, skill_name))
        })?;
    Ok(to_dto(row))
}

pub async fn set_enabled(
    db: &DatabaseConnection,
    bot_id: &str,
    skill_name: &str,
    enabled: bool,
) -> Result<bool> {
    let now = now_ms();
    let res = bot_skill_bindings::Entity::update_many()
        .col_expr(bot_skill_bindings::Column::Enabled, Expr::value(enabled))
        .col_expr(bot_skill_bindings::Column::UpdatedAt, Expr::value(now))
        .filter(bot_skill_bindings::Column::BotId.eq(bot_id))
        .filter(bot_skill_bindings::Column::SkillName.eq(skill_name))
        .exec(db)
        .await?;
    Ok(res.rows_affected == 1)
}

pub async fn delete(db: &DatabaseConnection, bot_id: &str, skill_name: &str) -> Result<u64> {
    let res = bot_skill_bindings::Entity::delete_many()
        .filter(bot_skill_bindings::Column::BotId.eq(bot_id))
        .filter(bot_skill_bindings::Column::SkillName.eq(skill_name))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

pub async fn delete_by_bot(db: &DatabaseConnection, bot_id: &str) -> Result<u64> {
    let res = bot_skill_bindings::Entity::delete_many()
        .filter(bot_skill_bindings::Column::BotId.eq(bot_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

fn unique_conflict(err: DbErr) -> AxAgentError {
    let msg = err.to_string();
    if msg.contains("UNIQUE constraint failed") || msg.contains("duplicate key") {
        AxAgentError::Validation("bot+skill_name already has a binding row".to_string())
    } else {
        AxAgentError::Database(err)
    }
}
