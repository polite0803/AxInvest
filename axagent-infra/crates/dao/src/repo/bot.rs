// SPDX-License-Identifier: AGPL-3.0-only

//! `bots` 表的实现者层仓库（docs/bot-integration/MODEL.md §4）。
//!
//! 真相源：`bots` 记录在 SQLite；SOUL 文件只是镜像，MEMORY 文件才是记忆真相源。
//! 本模块只做数据访问，不碰文件系统（文件侧归 `axagent_agent::bot_workspace`）。

use sea_orm::prelude::Expr;
use sea_orm::*;

use axagent_entities::{bots, conversations};
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{Bot, Conversation, CreateBot, UpdateBotPatch};
use axagent_harness::util_fns::{gen_id, now_ts};

use super::conversation::{attach_workspace_dirs, conversation_from_entity};

fn bot_from_entity(m: bots::Model) -> Bot {
    Bot {
        id: m.id,
        name: m.name,
        title: m.title,
        description: m.description,
        soul: m.soul,
        soul_hash: m.soul_hash,
        soul_drift: m.soul_drift != 0,
        icon: m.icon,
        color: m.color,
        section: m.section,
        model_id: m.model_id,
        provider_id: m.provider_id,
        effort: m.effort,
        workdir: m.workdir,
        approval_mode: m.approval_mode,
        agent_profile_id: m.agent_profile_id,
        is_enabled: m.is_enabled != 0,
        sort_order: m.sort_order,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

/// 侧栏列表：默认只返回启用中的 Bot，按 `sort_order`、创建时间升序。
pub async fn list_bots(db: &DatabaseConnection, include_disabled: bool) -> Result<Vec<Bot>> {
    let mut query = bots::Entity::find();
    if !include_disabled {
        query = query.filter(bots::Column::IsEnabled.eq(1));
    }
    let rows = query
        .order_by_asc(bots::Column::SortOrder)
        .order_by_asc(bots::Column::CreatedAt)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(bot_from_entity).collect())
}

pub async fn get_bot(db: &DatabaseConnection, id: &str) -> Result<Bot> {
    let row = bots::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("Bot {}", id)))?;
    Ok(bot_from_entity(row))
}

/// 新建 Bot 记录。id / 时间戳由本层生成；`soul_hash` 由命令层写完镜像后回填。
pub async fn create_bot(db: &DatabaseConnection, input: &CreateBot) -> Result<Bot> {
    let id = gen_id();
    let now = now_ts();
    let am = bots::ActiveModel {
        id: Set(id),
        name: Set(input.name.clone()),
        title: Set(input.title.clone()),
        description: Set(input.description.clone()),
        soul: Set(input.soul.clone().unwrap_or_default()),
        soul_hash: Set(String::new()),
        soul_drift: Set(0),
        icon: Set(input.icon.clone().unwrap_or_else(|| "🤖".to_string())),
        color: Set(input.color.clone()),
        section: Set(input.section.clone().unwrap_or_else(|| "general".to_string())),
        model_id: Set(input.model_id.clone()),
        provider_id: Set(input.provider_id.clone()),
        effort: Set(input.effort.clone()),
        workdir: Set(input.workdir.clone()),
        approval_mode: Set(input.approval_mode.clone().unwrap_or_else(|| "full".to_string())),
        agent_profile_id: Set(input.agent_profile_id.clone()),
        is_enabled: Set(1),
        sort_order: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let row = am.insert(db).await?;
    Ok(bot_from_entity(row))
}

/// 局部更新：`None` = 不改，`Some(None)` = 置空。
pub async fn update_bot(db: &DatabaseConnection, id: &str, patch: &UpdateBotPatch) -> Result<Bot> {
    let row = bots::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("Bot {}", id)))?;

    let mut am: bots::ActiveModel = row.into();
    if let Some(name) = &patch.name {
        am.name = Set(name.clone());
    }
    if let Some(title) = &patch.title {
        am.title = Set(title.clone());
    }
    if let Some(description) = &patch.description {
        am.description = Set(description.clone());
    }
    if let Some(soul) = &patch.soul {
        am.soul = Set(soul.clone());
    }
    if let Some(icon) = &patch.icon {
        am.icon = Set(icon.clone());
    }
    if let Some(color) = &patch.color {
        am.color = Set(color.clone());
    }
    if let Some(section) = &patch.section {
        am.section = Set(section.clone());
    }
    if let Some(model_id) = &patch.model_id {
        am.model_id = Set(model_id.clone());
    }
    if let Some(provider_id) = &patch.provider_id {
        am.provider_id = Set(provider_id.clone());
    }
    if let Some(effort) = &patch.effort {
        am.effort = Set(effort.clone());
    }
    if let Some(workdir) = &patch.workdir {
        am.workdir = Set(workdir.clone());
    }
    if let Some(approval_mode) = &patch.approval_mode {
        am.approval_mode = Set(approval_mode.clone());
    }
    if let Some(agent_profile_id) = &patch.agent_profile_id {
        am.agent_profile_id = Set(agent_profile_id.clone());
    }
    if let Some(is_enabled) = patch.is_enabled {
        am.is_enabled = Set(if is_enabled { 1 } else { 0 });
    }
    if let Some(sort_order) = patch.sort_order {
        am.sort_order = Set(sort_order);
    }
    am.updated_at = Set(now_ts());
    am.update(db).await?;

    get_bot(db, id).await
}

/// 删除 Bot 记录，并把其线程的 `bot_id` 置 NULL（ADR-009：不级联删消息、不删记忆目录）。
///
/// 返回解绑的线程数，供 `delete_bot` 命令回传前端。
pub async fn delete_bot(db: &DatabaseConnection, id: &str) -> Result<u64> {
    let txn = db.begin().await?;
    let detached = conversations::Entity::update_many()
        .col_expr(conversations::Column::BotId, Expr::value(Option::<String>::None))
        .filter(conversations::Column::BotId.eq(id))
        .exec(&txn)
        .await?
        .rows_affected;
    let deleted = bots::Entity::delete_by_id(id).exec(&txn).await?.rows_affected;
    if deleted == 0 {
        txn.rollback().await?;
        return Err(AxAgentError::NotFound(format!("Bot {}", id)));
    }
    txn.commit().await?;
    Ok(detached)
}

/// 某 Bot 的线程，按 `updated_at DESC`；默认过滤归档。
pub async fn list_bot_threads(
    db: &DatabaseConnection,
    bot_id: &str,
    include_archived: bool,
) -> Result<Vec<Conversation>> {
    let mut query = conversations::Entity::find().filter(conversations::Column::BotId.eq(bot_id));
    if !include_archived {
        query = query.filter(conversations::Column::IsArchived.eq(0));
    }
    let rows = query.order_by_desc(conversations::Column::UpdatedAt).all(db).await?;
    let convs: Vec<Conversation> = rows.into_iter().map(conversation_from_entity).collect();
    attach_workspace_dirs(db, convs).await
}

/// 写入 SOUL 正文与 hash（镜像已落盘后调用），并把漂移标记清零。
pub async fn set_soul(db: &DatabaseConnection, id: &str, soul: &str, hash: &str) -> Result<Bot> {
    let row = bots::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| AxAgentError::NotFound(format!("Bot {}", id)))?;
    let mut am: bots::ActiveModel = row.into();
    am.soul = Set(soul.to_string());
    am.soul_hash = Set(hash.to_string());
    am.soul_drift = Set(0);
    am.updated_at = Set(now_ts());
    am.update(db).await?;
    get_bot(db, id).await
}

/// 记录漂移标记（读取时发现镜像与 DB 不一致才调用；不覆盖任何一侧内容）。
pub async fn set_soul_drift(db: &DatabaseConnection, id: &str, drift: bool) -> Result<()> {
    bots::Entity::update_many()
        .col_expr(bots::Column::SoulDrift, Expr::value(if drift { 1 } else { 0 }))
        .col_expr(bots::Column::UpdatedAt, Expr::value(now_ts()))
        .filter(bots::Column::Id.eq(id))
        .exec(db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::conversation;

    async fn pool() -> crate::db::DbHandle {
        crate::db::create_test_pool().await.expect("测试库应可建立")
    }

    fn input(name: &str) -> CreateBot {
        CreateBot {
            name: name.to_string(),
            title: Some("投研助手".to_string()),
            color: Some("#336699".to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_update_and_double_option_semantics() {
        let h = pool().await;
        let db = &h.conn;
        let created = create_bot(db, &input("投研助手")).await.expect("创建应成功");
        assert_eq!(created.section, "general");
        assert_eq!(created.approval_mode, "full");
        assert_eq!(created.icon, "🤖");
        assert!(!created.soul_drift);
        assert_eq!(created.soul, "");
        assert_eq!(created.soul_hash, "");

        // 只改名字：其余字段原样保留
        let renamed = update_bot(
            db,
            &created.id,
            &UpdateBotPatch { name: Some("改名".to_string()), ..Default::default() },
        )
        .await
        .expect("更新应成功");
        assert_eq!(renamed.name, "改名");
        assert_eq!(renamed.title.as_deref(), Some("投研助手"));

        // Some(None) = 显式置空；未出现的键 = 不改
        let cleared = update_bot(
            db,
            &created.id,
            &UpdateBotPatch { title: Some(None), ..Default::default() },
        )
        .await
        .expect("置空应成功");
        assert_eq!(cleared.title, None);
        assert_eq!(cleared.color.as_deref(), Some("#336699"));

        let souled = set_soul(db, &created.id, "只回答 A 股问题", "abc").await.expect("写 SOUL");
        assert_eq!(souled.soul, "只回答 A 股问题");
        assert_eq!(souled.soul_hash, "abc");

        let listed = list_bots(db, false).await.expect("列表应成功");
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn list_hides_disabled_by_default() {
        let h = pool().await;
        let db = &h.conn;
        let bot = create_bot(db, &input("归档 Bot")).await.expect("创建应成功");
        update_bot(db, &bot.id, &UpdateBotPatch { is_enabled: Some(false), ..Default::default() })
            .await
            .expect("归档应成功");

        assert!(list_bots(db, false).await.expect("列表").is_empty());
        assert_eq!(list_bots(db, true).await.expect("列表").len(), 1);
    }

    async fn make_thread(db: &DatabaseConnection, title: &str, bot_id: &str) -> Conversation {
        conversation::create_conversation_with_defaults(
            db,
            title,
            "model-1",
            "provider-1",
            None,
            conversation::ConversationParamDefaults::default(),
            Some(bot_id),
        )
        .await
        .expect("建线程应成功")
    }

    #[tokio::test]
    async fn threads_are_scoped_and_filter_archived() {
        let h = pool().await;
        let db = &h.conn;
        let bot = create_bot(db, &input("线程 Bot")).await.expect("创建应成功");
        let other = create_bot(db, &input("别的 Bot")).await.expect("创建应成功");

        let t1 = make_thread(db, "线程一", &bot.id).await;
        let t2 = make_thread(db, "线程二", &bot.id).await;
        let _other_thread = make_thread(db, "别人的线程", &other.id).await;
        assert_eq!(t1.bot_id.as_deref(), Some(bot.id.as_str()));

        assert_eq!(list_bot_threads(db, &bot.id, false).await.expect("线程列表").len(), 2);

        conversation::update_conversation(
            db,
            &t2.id,
            axagent_harness::types::UpdateConversationInput {
                is_archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("归档线程应成功");

        assert_eq!(list_bot_threads(db, &bot.id, false).await.expect("线程列表").len(), 1);
        assert_eq!(list_bot_threads(db, &bot.id, true).await.expect("线程列表").len(), 2);
    }

    #[tokio::test]
    async fn delete_unbinds_threads_but_keeps_them() {
        let h = pool().await;
        let db = &h.conn;
        let bot = create_bot(db, &input("待删 Bot")).await.expect("创建应成功");
        let thread = make_thread(db, "保留线程", &bot.id).await;

        let detached = delete_bot(db, &bot.id).await.expect("删除应成功");
        assert_eq!(detached, 1);
        assert!(get_bot(db, &bot.id).await.is_err(), "记录应已删除");

        let survived = conversation::get_conversation(db, &thread.id).await.expect("线程应保留");
        assert_eq!(survived.bot_id, None, "线程应解绑而不是被删");
        assert!(list_bot_threads(db, &bot.id, true).await.expect("线程列表").is_empty());
    }

    #[tokio::test]
    async fn delete_missing_bot_is_not_found() {
        let h = pool().await;
        let err = delete_bot(&h.conn, "missing").await.expect_err("应报 NotFound");
        assert!(matches!(err, AxAgentError::NotFound(_)));
    }

    #[tokio::test]
    async fn drift_flag_roundtrip() {
        let h = pool().await;
        let db = &h.conn;
        let bot = create_bot(db, &input("漂移 Bot")).await.expect("创建应成功");
        set_soul_drift(db, &bot.id, true).await.expect("写漂移标记");
        assert!(get_bot(db, &bot.id).await.expect("读取").soul_drift);
        set_soul(db, &bot.id, "x", "h").await.expect("写 SOUL 应清漂移");
        assert!(!get_bot(db, &bot.id).await.expect("读取").soul_drift);
    }
}
