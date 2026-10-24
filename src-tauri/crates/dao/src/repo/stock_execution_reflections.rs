// SPDX-License-Identifier: AGPL-3.0-only

//! `stock_execution_reflections` 的读写收口。
//!
//! 写侧只有自适应闭环（分析完成后异步一次）；读侧只有历史列表与详情。
//! 与市场结果反思（`stock_reflections`）是两张表，`analysis_id` 是唯一关联键。

pub use axagent_entities::stock_execution_reflections::Model as ExecutionReflectionModel;
use axagent_entities::stock_execution_reflections::{self, Column, Entity};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    Set,
};
use std::collections::HashMap;

/// 一条待落库的执行层反思（字段均为纯数据，序列化在命令/调用方完成）。
#[derive(Debug, Clone)]
pub struct NewExecutionReflection {
    pub analysis_id: Option<String>,
    pub stock_code: String,
    pub execution_id: String,
    pub workflow_id: String,
    pub overall_score: i32,
    pub dimension_scores_json: String,
    pub bottleneck_nodes_json: String,
    pub error_patterns_json: String,
    pub reusable_patterns_json: String,
    pub improvement_suggestions_json: String,
    pub should_trigger_evolution: bool,
    pub evolution_trigger_reason: Option<String>,
    pub created_at_ms: i64,
}

pub async fn insert(db: &DatabaseConnection, row: &NewExecutionReflection) -> Result<(), DbErr> {
    stock_execution_reflections::ActiveModel {
        id: Set(format!("ser-{}", uuid::Uuid::new_v4())),
        analysis_id: Set(row.analysis_id.clone()),
        stock_code: Set(row.stock_code.clone()),
        execution_id: Set(row.execution_id.clone()),
        workflow_id: Set(row.workflow_id.clone()),
        overall_score: Set(row.overall_score),
        dimension_scores_json: Set(row.dimension_scores_json.clone()),
        bottleneck_nodes_json: Set(row.bottleneck_nodes_json.clone()),
        error_patterns_json: Set(row.error_patterns_json.clone()),
        reusable_patterns_json: Set(row.reusable_patterns_json.clone()),
        improvement_suggestions_json: Set(row.improvement_suggestions_json.clone()),
        should_trigger_evolution: Set(row.should_trigger_evolution),
        evolution_trigger_reason: Set(row.evolution_trigger_reason.clone()),
        created_at_ms: Set(row.created_at_ms),
    }
    .insert(db)
    .await?;
    Ok(())
}

/// 某条分析的**最新一条**执行层反思（详情页用）。
pub async fn latest_by_analysis(
    db: &DatabaseConnection,
    analysis_id: &str,
) -> Result<Option<ExecutionReflectionModel>, DbErr> {
    Entity::find()
        .filter(Column::AnalysisId.eq(analysis_id.to_owned()))
        .order_by_desc(Column::CreatedAtMs)
        .one(db)
        .await
}

/// 一批分析的**最新执行质量分**（历史列表用，避免 N+1）。
///
/// 只回 `analysis_id -> overall_score`；缺失的分析不出键（前端按「未记录」渲染，
/// 不得折成 0 分）。
pub async fn latest_scores_for_analyses(
    db: &DatabaseConnection,
    analysis_ids: &[String],
) -> Result<HashMap<String, i32>, DbErr> {
    if analysis_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = Entity::find()
        .filter(Column::AnalysisId.is_in(analysis_ids.to_vec()))
        .order_by_desc(Column::CreatedAtMs)
        .all(db)
        .await?;
    let mut out = HashMap::new();
    for r in rows {
        if let Some(aid) = r.analysis_id {
            out.entry(aid).or_insert(r.overall_score);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(analysis_id: &str, score: i32, created: i64) -> NewExecutionReflection {
        NewExecutionReflection {
            analysis_id: Some(analysis_id.to_string()),
            stock_code: "600519".to_string(),
            execution_id: format!("wf-{created}"),
            workflow_id: "stock-analysis".to_string(),
            overall_score: score,
            dimension_scores_json: "{}".to_string(),
            bottleneck_nodes_json: "[]".to_string(),
            error_patterns_json: "[]".to_string(),
            reusable_patterns_json: "[]".to_string(),
            improvement_suggestions_json: "[]".to_string(),
            should_trigger_evolution: false,
            evolution_trigger_reason: None,
            created_at_ms: created,
        }
    }

    /// 同一分析多次闭环 ⇒ 读侧只取最新；批量映射不产生 0 分占位。
    #[tokio::test]
    async fn latest_score_wins_and_missing_stays_absent() {
        let db = crate::db::create_test_pool().await.expect("测试库应可建立").conn;
        insert(&db, &row("a-1", 4, 100)).await.expect("应可写入");
        insert(&db, &row("a-1", 7, 200)).await.expect("应可写入");
        insert(&db, &row("a-2", 9, 150)).await.expect("应可写入");

        let latest = latest_by_analysis(&db, "a-1").await.expect("应可读取").expect("应存在");
        assert_eq!(latest.overall_score, 7, "同一分析取最新一条");

        let map = latest_scores_for_analyses(
            &db,
            &["a-1".to_string(), "a-2".to_string(), "a-3".to_string()],
        )
        .await
        .expect("应可读取");
        assert_eq!(map.get("a-1"), Some(&7));
        assert_eq!(map.get("a-2"), Some(&9));
        assert!(!map.contains_key("a-3"), "没有执行反思的分析不得出现 0 分占位");
    }
}
