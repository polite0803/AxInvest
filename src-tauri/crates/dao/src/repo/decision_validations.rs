// SPDX-License-Identifier: AGPL-3.0-only
//! 决策回写验证表 `decision_validations` 的读原语（进化计划聚合的样本来源之一）。

/// 验证行类型重导出（命令侧按 `t_plus_n_price` / `final_return_pct` 判数据可达性）。
pub use axagent_entities::decision_validations::Model as ValidationModel;
use axagent_entities::decision_validations::{Column, Entity};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder};

/// 取 `validated_date >= validated_since`（YYYY-MM-DD 文本比较）的全部验证行。
///
/// 调用方（进化计划聚合）自行区分业务类样本与数据可达性失败样本，
/// repo 只做原样取数，不做 `t_plus_n_price IS NOT NULL` 之类的语义过滤。
pub async fn list_validated_since_date(
    db: &DatabaseConnection,
    validated_since: &str,
) -> Result<Vec<ValidationModel>, DbErr> {
    Entity::find().filter(Column::ValidatedAt.gte(validated_since.to_owned())).all(db).await
}

/// 影子回放样本：已结算（`final_return_pct` 非空）且置信度 > 0 的最近 `limit` 行。
///
/// 返回 `(置信度原值 0..100, 最终收益百分比)`，**不做**归一化与截断 —— 分数代理口径
/// 属于回放算法（`analysis-engine::param_replay`）的语义，repo 只做取数。
/// 两个命令入口（参数应用前的影子闸 / WFO 网格的影子闸）共用这一份，口径不分裂。
pub async fn list_shadow_replay_samples(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<(f64, f64)>, DbErr> {
    let rows = Entity::find()
        .filter(Column::FinalReturnPct.is_not_null())
        .filter(Column::Confidence.gt(0))
        .order_by_desc(Column::ValidatedAt)
        .limit(limit)
        .all(db)
        .await?;
    Ok(rows.into_iter().filter_map(|m| Some((m.confidence as f64, m.final_return_pct?))).collect())
}
