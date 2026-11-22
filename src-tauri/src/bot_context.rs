// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 的 SOUL / MEMORY 提示词上下文（docs/bot-integration/ADR-004 / ADR-005）。
//!
//! **唯一入口**：主对话路径（`agent_query`）与流式路径（`send_message` /
//! `regenerate`）都从这里取 SOUL 与 MEMORY，避免两套拼装产生「人格分叉」。
//!
//! 口径：
//! - SOUL 以 DB `bots.soul` 为真相源（ADR-003），空串视为未配置。
//! - MEMORY 以 `~/.axagent/bots/<botId>/MEMORY.md` 为真相源，注入前按
//!   200 行 / 24 KB 预算截断（`BotWorkspace::render_memory_for_prompt`）。
//! - 非 Bot 会话（`bot_id IS NULL`）返回 `None`，行为与改动前逐字节一致。

use axagent_agent::bot_workspace::BotWorkspace;
use axagent_dao::db::DatabaseConnection;
use axagent_dao::repo::bot as bot_repo;
use axagent_harness::types::Conversation;

/// 一次请求内解析出的 Bot 提示词素材。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BotSoulContext {
    /// SOUL 正文（DB 真相源），空白内容已折叠为 `None`。
    pub soul: Option<String>,
    /// 已截断的 MEMORY.md（文件真相源），空文件已折叠为 `None`。
    pub memory: Option<String>,
}

impl BotSoulContext {
    /// `<bot-soul>` 独立边界段；内容按原文注入，不解析其中指令（ADR-004）。
    pub fn soul_segment(&self) -> Option<String> {
        self.soul.as_deref().map(|s| format!("<bot-soul>\n{s}\n</bot-soul>"))
    }

    /// `<bot-memory>` 独立边界段；与 RAG、working-memory 分离（ADR-005）。
    pub fn memory_segment(&self) -> Option<String> {
        self.memory.as_deref().map(|m| format!("<bot-memory>\n{m}\n</bot-memory>"))
    }

    pub fn is_empty(&self) -> bool {
        self.soul.is_none() && self.memory.is_none()
    }
}

/// 解析会话所属 Bot 的提示词上下文。
///
/// 返回 `None` 表示「按非 Bot 会话处理」：未归属、Bot 已删除、读取失败或
/// SOUL/MEMORY 都为空。解析失败只记日志，不让一次提示词组装炸掉整轮对话。
pub async fn resolve_bot_context(
    db: &DatabaseConnection,
    conversation: &Conversation,
) -> Option<BotSoulContext> {
    resolve_bot_context_for_bot(db, conversation.bot_id.as_deref()).await
}

/// 同 [`resolve_bot_context`]，但只接收归属键（测试与非会话调用点用）。
pub async fn resolve_bot_context_for_bot(
    db: &DatabaseConnection,
    bot_id: Option<&str>,
) -> Option<BotSoulContext> {
    let bot_id = bot_id?;

    let bot = match bot_repo::get_bot(db, bot_id).await {
        Ok(bot) => bot,
        Err(e) => {
            tracing::warn!(bot_id, error = %e, "Bot 上下文解析失败：读取 bots 记录出错，按非 Bot 会话处理");
            return None;
        },
    };

    // 空 SOUL 视为未配置：此时保留全局 persona（ADR-004 互斥条件）。
    let soul = if bot.soul.trim().is_empty() {
        None
    } else {
        Some(bot.soul.clone())
    };

    let workspace = BotWorkspace::for_bot(&bot.id);
    let memory = match workspace.render_memory_for_prompt() {
        Ok(memory) => memory,
        Err(e) => {
            tracing::warn!(bot_id = %bot.id, error = %e, "Bot 上下文解析失败：读取 MEMORY.md 出错，跳过记忆注入");
            None
        },
    };

    let ctx = BotSoulContext { soul, memory };
    if ctx.is_empty() { None } else { Some(ctx) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::types::CreateBot;

    #[test]
    fn segments_use_dedicated_boundaries() {
        let ctx = BotSoulContext {
            soul: Some("# 投研助手".to_string()),
            memory: Some("- 用户偏好 A 股".to_string()),
        };
        assert_eq!(ctx.soul_segment().as_deref(), Some("<bot-soul>\n# 投研助手\n</bot-soul>"));
        assert_eq!(
            ctx.memory_segment().as_deref(),
            Some("<bot-memory>\n- 用户偏好 A 股\n</bot-memory>")
        );
        assert!(!ctx.is_empty());
        assert!(BotSoulContext::default().is_empty());
    }

    #[tokio::test]
    async fn unowned_conversation_has_no_bot_context() {
        let handle = axagent_dao::db::create_test_pool().await.expect("测试库应可建立");
        assert!(resolve_bot_context_for_bot(&handle.conn, None).await.is_none());
    }

    #[tokio::test]
    async fn dangling_bot_id_degrades_to_none() {
        let handle = axagent_dao::db::create_test_pool().await.expect("测试库应可建立");
        assert!(resolve_bot_context_for_bot(&handle.conn, Some("missing-bot")).await.is_none());
    }

    #[tokio::test]
    async fn empty_soul_and_missing_memory_degrade_to_none() {
        let handle = axagent_dao::db::create_test_pool().await.expect("测试库应可建立");
        let bot = bot_repo::create_bot(
            &handle.conn,
            &CreateBot { name: "空 Bot".to_string(), ..Default::default() },
        )
        .await
        .expect("创建 Bot 应成功");
        // 真实 HOME 下不会有这个新 id 的 MEMORY.md，SOUL 也是空串 ⇒ 无上下文。
        assert!(resolve_bot_context_for_bot(&handle.conn, Some(&bot.id)).await.is_none());
    }
}
