// SPDX-License-Identifier: AGPL-3.0-only

//! `workflow_param_changes` 的读写收口（参数变更台账）。
//!
//! 状态机与值域见实体文档；本模块只提供写入 / 到期待评估 / 回滚落账三类原语，
//! 评估判据（命中率、样本门槛）留在命令层 —— 那里同时能看到反思行与模板版本。

pub use axagent_entities::workflow_param_changes::Model as ParamChangeModel;
use axagent_entities::workflow_param_changes::{self, Column, Entity};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};

/// 一次参数变更的写入载荷（id / status / 评估列由 DAO 或评估命令填）。
#[derive(Debug, Clone)]
pub struct NewParamChange {
    pub template_id: String,
    pub param: String,
    pub from_version: i32,
    pub to_version: i32,
    pub old_value_json: String,
    pub new_value_json: String,
    pub reflection_id: Option<String>,
    pub reason: Option<String>,
    pub review_after_ms: i64,
    pub applied_at_ms: i64,
}

/// 批量落账；单次应用（一次换代）里的每个参数各一行。
pub async fn insert_many(db: &DatabaseConnection, rows: &[NewParamChange]) -> Result<usize, DbErr> {
    if rows.is_empty() {
        return Ok(0);
    }
    let models: Vec<workflow_param_changes::ActiveModel> = rows
        .iter()
        .map(|r| workflow_param_changes::ActiveModel {
            id: Set(format!("wpc-{}", uuid::Uuid::new_v4())),
            template_id: Set(r.template_id.clone()),
            param: Set(r.param.clone()),
            from_version: Set(r.from_version),
            to_version: Set(r.to_version),
            old_value_json: Set(r.old_value_json.clone()),
            new_value_json: Set(r.new_value_json.clone()),
            reflection_id: Set(r.reflection_id.clone()),
            reason: Set(r.reason.clone()),
            status: Set("pending".to_string()),
            review_after_ms: Set(r.review_after_ms),
            baseline_rate: Set(None),
            baseline_samples: Set(None),
            after_rate: Set(None),
            after_samples: Set(None),
            verdict_note: Set(None),
            rollback_version: Set(None),
            applied_at_ms: Set(r.applied_at_ms),
            evaluated_at_ms: Set(None),
        })
        .collect();
    Entity::insert_many(models).exec_without_returning(db).await?;
    Ok(rows.len())
}

/// 最近若干条（面板台账）。
pub async fn list_recent(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<ParamChangeModel>, DbErr> {
    Entity::find().order_by_desc(Column::AppliedAtMs).limit(limit).all(db).await
}

/// 已到观察期、仍待判定的行（`pending` / `insufficient` 会被重新评估）。
pub async fn list_due(
    db: &DatabaseConnection,
    now_ms: i64,
    limit: u64,
) -> Result<Vec<ParamChangeModel>, DbErr> {
    Entity::find()
        .filter(Column::ReviewAfterMs.lte(now_ms))
        .filter(Column::Status.is_in(["pending", "insufficient"]))
        .order_by_asc(Column::ReviewAfterMs)
        .limit(limit)
        .all(db)
        .await
}

pub async fn fetch_by_id(
    db: &DatabaseConnection,
    id: &str,
) -> Result<Option<ParamChangeModel>, DbErr> {
    Entity::find_by_id(id.to_owned()).one(db).await
}

/// 到期评估结果落账（不改模板）。
#[allow(clippy::too_many_arguments)]
pub async fn mark_evaluated(
    db: &DatabaseConnection,
    id: &str,
    status: &str,
    baseline_rate: Option<f64>,
    baseline_samples: i32,
    after_rate: Option<f64>,
    after_samples: i32,
    note: &str,
    now_ms: i64,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value(status.to_owned()))
        .col_expr(Column::BaselineRate, Expr::value(baseline_rate))
        .col_expr(Column::BaselineSamples, Expr::value(baseline_samples))
        .col_expr(Column::AfterRate, Expr::value(after_rate))
        .col_expr(Column::AfterSamples, Expr::value(after_samples))
        .col_expr(Column::VerdictNote, Expr::value(note.to_owned()))
        .col_expr(Column::EvaluatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.eq(id.to_owned()))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// 回滚已执行：记回落的新版本号与说明。
pub async fn mark_rolled_back(
    db: &DatabaseConnection,
    id: &str,
    rollback_version: i32,
    note: &str,
    now_ms: i64,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Status, Expr::value("rolled_back".to_owned()))
        .col_expr(Column::RollbackVersion, Expr::value(rollback_version))
        .col_expr(Column::VerdictNote, Expr::value(note.to_owned()))
        .col_expr(Column::EvaluatedAtMs, Expr::value(now_ms))
        .filter(Column::Id.eq(id.to_owned()))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(param: &str, review_after_ms: i64) -> NewParamChange {
        NewParamChange {
            template_id: "stock-analysis".to_string(),
            param: param.to_string(),
            from_version: 10,
            to_version: 11,
            old_value_json: "0.5".to_string(),
            new_value_json: "0.6".to_string(),
            reflection_id: Some("rf-1".to_string()),
            reason: Some("测试".to_string()),
            review_after_ms,
            applied_at_ms: 1_000,
        }
    }

    /// 台账三态：到期的 `pending/insufficient` 才进评估；评估/回滚落账按 id 生效。
    #[tokio::test]
    async fn ledger_insert_list_due_and_status_updates() {
        let db = crate::db::create_test_pool().await.expect("测试库应可建立").conn;
        insert_many(&db, &[row("buy_threshold", 500), row("sell_threshold", 5_000)])
            .await
            .expect("台账应可写入");

        let recent = list_recent(&db, 10).await.expect("应可读取");
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().all(|r| r.status == "pending"));

        // 只取出已到期的第一条
        let due = list_due(&db, 1_000, 10).await.expect("应可读取到期项");
        assert_eq!(due.len(), 1, "未到观察期的行不得进评估");
        assert_eq!(due[0].param, "buy_threshold");

        let id = due[0].id.clone();
        let affected = mark_evaluated(
            &db,
            &id,
            "rollback_suggested",
            Some(0.6),
            10,
            Some(0.4),
            12,
            "before=6/10 after=5/12",
            2_000,
        )
        .await
        .expect("评估落账应成功");
        assert_eq!(affected, 1);
        let after = fetch_by_id(&db, &id).await.expect("应可读取").expect("行应存在");
        assert_eq!(after.status, "rollback_suggested");
        assert_eq!(after.baseline_samples, Some(10));
        assert_eq!(after.after_samples, Some(12));

        // insufficient 到期后仍会被重评估（list_due 包含它）
        mark_evaluated(&db, &id, "insufficient", None, 3, None, 2, "need>=10", 2_100)
            .await
            .expect("应可写");
        assert_eq!(list_due(&db, 2_200, 10).await.expect("应可读取").len(), 1);

        mark_rolled_back(&db, &id, 12, "rolled back: v11→v12", 2_300)
            .await
            .expect("回滚落账应成功");
        let done = fetch_by_id(&db, &id).await.expect("应可读取").expect("行应存在");
        assert_eq!(done.status, "rolled_back");
        assert_eq!(done.rollback_version, Some(12));
        // 已回滚不再进到期评估；另一条仍 pending 的到期行必须还在（负控：不是整表滤空）
        let remaining = list_due(&db, 9_999, 10).await.expect("应可读取");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].param, "sell_threshold");
    }
}
