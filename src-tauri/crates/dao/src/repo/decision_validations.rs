// SPDX-License-Identifier: AGPL-3.0-only
//! 决策回写验证表 `decision_validations` 的读原语（进化计划聚合的样本来源之一）。

/// 验证行类型重导出（命令侧按 `t_plus_n_price` / `final_return_pct` 判数据可达性）。
pub use axagent_entities::decision_validations::Model as ValidationModel;
use axagent_entities::decision_validations::{Column, Entity};
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};

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
