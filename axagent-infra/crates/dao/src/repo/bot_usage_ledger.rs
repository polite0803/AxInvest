// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_usage_ledger` 仓库层（docs/bot-integration/P5/MODEL-P5.md §2）。
//!
//! append-only：只 INSERT；不 UPDATE、不 DELETE（ADR-P5-2）。

use sea_orm::*;

use axagent_entities::bot_usage_ledger;
use axagent_harness::core_error::Result;
use axagent_harness::types::{
    BotPricingSource, BotUsageEntry, BotUsageFilter, BotUsageOrigin, BotUsageTotals,
};
use axagent_harness::util_fns::{gen_id, now_ms};

fn to_dto(m: bot_usage_ledger::Model) -> BotUsageEntry {
    BotUsageEntry {
        id: m.id,
        bot_id: m.bot_id,
        department_id: m.department_id,
        conversation_id: m.conversation_id,
        execution_id: m.execution_id,
        workflow_run_id: m.workflow_run_id,
        assignment_id: m.assignment_id,
        model_id: m.model_id,
        origin: m.origin.parse::<BotUsageOrigin>().unwrap_or(BotUsageOrigin::Routine),
        input_tokens: m.input_tokens,
        output_tokens: m.output_tokens,
        cache_creation_tokens: m.cache_creation_tokens,
        cache_read_tokens: m.cache_read_tokens,
        total_tokens: m.total_tokens,
        estimated_cost_usd: m.estimated_cost_usd,
        pricing_source: m
            .pricing_source
            .parse::<BotPricingSource>()
            .unwrap_or(BotPricingSource::Heuristic),
        created_at: m.created_at,
    }
}

/// 追加一条账本记录（只 INSERT）。
pub async fn append(db: &DatabaseConnection, entry: &BotUsageEntry) -> Result<()> {
    let id = if entry.id.is_empty() { gen_id() } else { entry.id.clone() };
    let created_at = if entry.created_at > 0 { entry.created_at } else { now_ms() };
    let am = bot_usage_ledger::ActiveModel {
        id: Set(id),
        bot_id: Set(entry.bot_id.clone()),
        department_id: Set(entry.department_id.clone()),
        conversation_id: Set(entry.conversation_id.clone()),
        execution_id: Set(entry.execution_id.clone()),
        workflow_run_id: Set(entry.workflow_run_id.clone()),
        assignment_id: Set(entry.assignment_id.clone()),
        model_id: Set(entry.model_id.clone()),
        origin: Set(entry.origin.as_str().to_string()),
        input_tokens: Set(entry.input_tokens),
        output_tokens: Set(entry.output_tokens),
        cache_creation_tokens: Set(entry.cache_creation_tokens),
        cache_read_tokens: Set(entry.cache_read_tokens),
        total_tokens: Set(entry.total_tokens),
        estimated_cost_usd: Set(entry.estimated_cost_usd),
        pricing_source: Set(entry.pricing_source.as_str().to_string()),
        created_at: Set(created_at),
    };
    bot_usage_ledger::Entity::insert(am).exec(db).await?;
    Ok(())
}

fn apply_filter(
    mut q: Select<bot_usage_ledger::Entity>,
    filter: &BotUsageFilter,
) -> Select<bot_usage_ledger::Entity> {
    if let Some(v) = filter.bot_id.as_deref().filter(|s| !s.is_empty()) {
        q = q.filter(bot_usage_ledger::Column::BotId.eq(v));
    }
    if let Some(v) = filter.department_id.as_deref().filter(|s| !s.is_empty()) {
        q = q.filter(bot_usage_ledger::Column::DepartmentId.eq(v));
    }
    if let Some(v) = filter.origin.as_ref() {
        q = q.filter(bot_usage_ledger::Column::Origin.eq(v.as_str()));
    }
    if let Some(v) = filter.workflow_run_id.as_deref().filter(|s| !s.is_empty()) {
        q = q.filter(bot_usage_ledger::Column::WorkflowRunId.eq(v));
    }
    if let Some(v) = filter.assignment_id.as_deref().filter(|s| !s.is_empty()) {
        q = q.filter(bot_usage_ledger::Column::AssignmentId.eq(v));
    }
    if let Some(since) = filter.since {
        q = q.filter(bot_usage_ledger::Column::CreatedAt.gte(since));
    }
    if let Some(until) = filter.until {
        q = q.filter(bot_usage_ledger::Column::CreatedAt.lte(until));
    }
    q
}

/// 过滤查询，按 `created_at DESC`。
pub async fn list(
    db: &DatabaseConnection,
    filter: &BotUsageFilter,
) -> Result<Vec<BotUsageEntry>> {
    let q = bot_usage_ledger::Entity::find().order_by_desc(bot_usage_ledger::Column::CreatedAt);
    let rows = apply_filter(q, filter).all(db).await?;
    Ok(rows.into_iter().map(to_dto).collect())
}

async fn sum_filtered(
    db: &DatabaseConnection,
    filter: &BotUsageFilter,
) -> Result<BotUsageTotals> {
    let rows: Vec<bot_usage_ledger::Model> =
        apply_filter(bot_usage_ledger::Entity::find(), filter).all(db).await?;
    let mut t = BotUsageTotals::default();
    for r in &rows {
        t.input_tokens += r.input_tokens;
        t.output_tokens += r.output_tokens;
        t.cache_creation_tokens += r.cache_creation_tokens;
        t.cache_read_tokens += r.cache_read_tokens;
        t.total_tokens += r.total_tokens;
        t.estimated_cost_usd += r.estimated_cost_usd;
    }
    t.entry_count = rows.len() as i64;
    Ok(t)
}

pub async fn sum_by_bot(
    db: &DatabaseConnection,
    bot_id: &str,
    since: i64,
    until: i64,
) -> Result<BotUsageTotals> {
    sum_filtered(
        db,
        &BotUsageFilter {
            bot_id: Some(bot_id.to_string()),
            since: Some(since),
            until: Some(until),
            ..Default::default()
        },
    )
    .await
}

pub async fn sum_by_department(
    db: &DatabaseConnection,
    department_id: &str,
    since: i64,
    until: i64,
) -> Result<BotUsageTotals> {
    sum_filtered(
        db,
        &BotUsageFilter {
            department_id: Some(department_id.to_string()),
            since: Some(since),
            until: Some(until),
            ..Default::default()
        },
    )
    .await
}

pub async fn sum_by_workflow_run(
    db: &DatabaseConnection,
    workflow_run_id: &str,
) -> Result<BotUsageTotals> {
    sum_filtered(
        db,
        &BotUsageFilter {
            workflow_run_id: Some(workflow_run_id.to_string()),
            ..Default::default()
        },
    )
    .await
}
