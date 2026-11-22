// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_departments` / `bot_department_members` 表的仓库层
//! （docs/bot-integration/P2/MODEL-P2B §2.1–2.2）。
//!
//! 只做数据访问：UNIQUE 冲突（一个 Bot 只属一个部门，Q2）由本层捕获
//! SQLite/PG 唯一约束错误转 `Validation`，语义判断（Chief 是否 enabled、
//! 部门是否存在）在命令层。

use sea_orm::prelude::Expr;
use sea_orm::*;

use axagent_entities::{bot_department_members, bot_departments};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{BotDepartment, UpdateBotDepartmentPatch};
use axagent_harness::util_fns::{gen_id, now_ts};

fn department_from_entity(m: bot_departments::Model, bot_ids: Vec<String>) -> BotDepartment {
    BotDepartment {
        id: m.id,
        name: m.name,
        description: m.description,
        chief_bot_id: m.chief_bot_id,
        bot_ids,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

/// 读取部门完整 DTO（含成员 join）。部门不存在返回 NotFound。
pub async fn get_department(db: &DatabaseConnection, id: &str) -> Result<BotDepartment> {
    let row = bot_departments::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("BotDepartment {}", id)))?;
    let bot_ids = member_bot_ids(db, id).await?;
    Ok(department_from_entity(row, bot_ids))
}

pub async fn list_departments(db: &DatabaseConnection) -> Result<Vec<BotDepartment>> {
    let rows = bot_departments::Entity::find()
        .order_by_asc(bot_departments::Column::CreatedAt)
        .all(db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let bot_ids = member_bot_ids(db, &row.id).await?;
        out.push(department_from_entity(row, bot_ids));
    }
    Ok(out)
}

/// 部门成员 bot_id 列表，按加入时间升序。
pub async fn member_bot_ids(db: &DatabaseConnection, department_id: &str) -> Result<Vec<String>> {
    let rows = bot_department_members::Entity::find()
        .filter(bot_department_members::Column::DepartmentId.eq(department_id))
        .order_by_asc(bot_department_members::Column::JoinedAt)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(|m| m.bot_id).collect())
}

/// 某 Bot 当前所属的部门 id（UNIQUE 约束下至多一个）。
pub async fn department_of_bot(db: &DatabaseConnection, bot_id: &str) -> Result<Option<String>> {
    let row = bot_department_members::Entity::find()
        .filter(bot_department_members::Column::BotId.eq(bot_id))
        .one(db)
        .await?;
    Ok(row.map(|m| m.department_id))
}

pub async fn create_department(
    db: &DatabaseConnection,
    name: &str,
    description: &str,
    chief_bot_id: Option<&str>,
) -> Result<BotDepartment> {
    let id = gen_id();
    let now = now_ts();
    let am = bot_departments::ActiveModel {
        id: Set(id.clone()),
        name: Set(name.to_string()),
        description: Set(description.to_string()),
        chief_bot_id: Set(chief_bot_id.map(|s| s.to_string())),
        metadata_json: Set("{}".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    };
    am.insert(db).await?;
    get_department(db, &id).await
}

/// 局部更新：`None` = 不改，`Some(None)` = 置空。
pub async fn update_department(
    db: &DatabaseConnection,
    id: &str,
    patch: &UpdateBotDepartmentPatch,
) -> Result<BotDepartment> {
    let row = bot_departments::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("BotDepartment {}", id)))?;
    let mut am: bot_departments::ActiveModel = row.into();
    if let Some(name) = &patch.name {
        am.name = Set(name.clone());
    }
    if let Some(description) = &patch.description {
        am.description =
            Set(description.clone().unwrap_or_else(|| String::new()));
    }
    if let Some(chief) = &patch.chief_bot_id {
        am.chief_bot_id = Set(chief.clone());
    }
    am.updated_at = Set(now_ts());
    am.update(db).await?;
    get_department(db, id).await
}

/// 删除部门记录本身（成员行的清理由调用方先走 [`delete_department_members`]，
/// 便于逐行审计）。
pub async fn delete_department(db: &DatabaseConnection, id: &str) -> Result<()> {
    let res = bot_departments::Entity::delete_by_id(id).exec(db).await?;
    if res.rows_affected == 0 {
        return Err(AxAgentError::NotFound(format!("BotDepartment {}", id)));
    }
    Ok(())
}

/// 清空部门的全部成员行（删部门级联，ADR-P2B-6）。返回清掉的行数。
pub async fn delete_department_members(db: &DatabaseConnection, department_id: &str) -> Result<u64> {
    let res = bot_department_members::Entity::delete_many()
        .filter(bot_department_members::Column::DepartmentId.eq(department_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

/// 把 Bot 加入部门。重复归属（已在任一部门）由唯一约束兜底并转 Validation。
pub async fn add_bot_to_department(
    db: &DatabaseConnection,
    department_id: &str,
    bot_id: &str,
) -> Result<()> {
    let am = bot_department_members::ActiveModel {
        id: Set(gen_id()),
        department_id: Set(department_id.to_string()),
        bot_id: Set(bot_id.to_string()),
        joined_at: Set(now_ts()),
    };
    am.insert(db).await.map_err(unique_conflict)?;
    Ok(())
}

/// 把 Bot 移出指定部门。不在该部门时返回 NotFound。
pub async fn remove_bot_from_department(
    db: &DatabaseConnection,
    department_id: &str,
    bot_id: &str,
) -> Result<()> {
    let res = bot_department_members::Entity::delete_many()
        .filter(bot_department_members::Column::DepartmentId.eq(department_id))
        .filter(bot_department_members::Column::BotId.eq(bot_id))
        .exec(db)
        .await?;
    if res.rows_affected == 0 {
        return Err(AxAgentError::NotFound(format!(
            "BotDepartmentMember department={department_id} bot={bot_id}"
        )));
    }
    Ok(())
}

/// 删 Bot 级联：清掉该 Bot 的全部归属行（GAP G-10 同款扫描）。返回行数。
pub async fn delete_memberships_of_bot(db: &DatabaseConnection, bot_id: &str) -> Result<u64> {
    let res = bot_department_members::Entity::delete_many()
        .filter(bot_department_members::Column::BotId.eq(bot_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

/// 删 Bot 级联：把以其为 Chief 的部门的 `chief_bot_id` 置 NULL
/// （部门保留，Chief 空缺，ADR-P2B-6）。返回受影响部门数。
pub async fn clear_chief_for_bot(db: &DatabaseConnection, bot_id: &str) -> Result<u64> {
    let res = bot_departments::Entity::update_many()
        .col_expr(bot_departments::Column::ChiefBotId, Expr::value(Option::<String>::None))
        .col_expr(bot_departments::Column::UpdatedAt, Expr::value(now_ts()))
        .filter(bot_departments::Column::ChiefBotId.eq(bot_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

/// 唯一约束冲突（SQLite 1 与 PG 23505）→ Validation。
fn unique_conflict(err: DbErr) -> AxAgentError {
    let msg = err.to_string();
    if msg.contains("UNIQUE constraint failed") || msg.contains("duplicate key") {
        AxAgentError::Validation("bot already belongs to a department".to_string())
    } else {
        AxAgentError::Database(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::types::UpdateBotDepartmentPatch;

    async fn pool() -> crate::db::DbHandle {
        crate::db::create_test_pool().await.expect("测试库应可建立")
    }

    #[tokio::test]
    async fn department_crud_roundtrip() {
        let h = pool().await;
        let db = &h.conn;
        let dept = create_department(db, "投研一部", "A股研究", None).await.expect("建部门");
        assert_eq!(dept.name, "投研一部");
        assert!(dept.bot_ids.is_empty());
        assert_eq!(dept.chief_bot_id, None);

        let renamed = update_department(
            db,
            &dept.id,
            &UpdateBotDepartmentPatch { name: Some("投研二部".into()), ..Default::default() },
        )
        .await
        .expect("改名");
        assert_eq!(renamed.name, "投研二部");

        let listed = list_departments(db).await.expect("列表");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "投研二部");

        delete_department(db, &dept.id).await.expect("删部门");
        assert!(get_department(db, &dept.id).await.is_err());
    }

    #[tokio::test]
    async fn membership_unique_and_cascade_helpers() {
        let h = pool().await;
        let db = &h.conn;
        let dept = create_department(db, "部门A", "", None).await.expect("建部门");
        let other = create_department(db, "部门B", "", None).await.expect("建部门");

        add_bot_to_department(db, &dept.id, "bot-1").await.expect("首次加入");
        assert_eq!(
            department_of_bot(db, "bot-1").await.expect("查询归属").as_deref(),
            Some(dept.id.as_str())
        );

        // Q2 复合 UNIQUE（规格冻结口径）：同部门重复登记撞约束 → Validation。
        // 「一个 Bot 只属一个部门」的跨部门语义由命令层查 department_of_bot 把关
        // （repo 层保持数据访问原语）。
        let dup = add_bot_to_department(db, &dept.id, "bot-1").await;
        assert!(matches!(dup, Err(AxAgentError::Validation(_))), "同部门重复应报 Validation: {dup:?}");

        // 移出后可再加入（同部门：复合 UNIQUE 只约束行重复，不约束归属流）。
        remove_bot_from_department(db, &dept.id, "bot-1").await.expect("移出");
        add_bot_to_department(db, &dept.id, "bot-1").await.expect("重新加入");
        assert_eq!(
            department_of_bot(db, "bot-1").await.expect("查询").as_deref(),
            Some(dept.id.as_str())
        );

        // 删 Bot 级联两件套：清成员行 + Chief 置空。
        let chief_dept =
            create_department(db, "有Chief的部门", "", Some("chief-bot")).await.expect("建");
        let cleared = clear_chief_for_bot(db, "chief-bot").await.expect("清Chief");
        assert_eq!(cleared, 1);
        let after = get_department(db, &chief_dept.id).await.expect("部门保留");
        assert_eq!(after.chief_bot_id, None, "Chief 置空但部门保留");

        let removed = delete_memberships_of_bot(db, "bot-1").await.expect("清成员行");
        assert_eq!(removed, 1);
        assert_eq!(department_of_bot(db, "bot-1").await.expect("查询"), None);
    }

    #[tokio::test]
    async fn delete_missing_department_is_not_found() {
        let h = pool().await;
        let err = delete_department(&h.conn, "missing").await.expect_err("应报 NotFound");
        assert!(matches!(err, AxAgentError::NotFound(_)));
    }
}
