// SPDX-License-Identifier: AGPL-3.0-only

use crate::session::{ContentBlock, ConversationMessage, MessageRole, Session};

use axagent_harness::prompt_provider::PromptProvider;

/// CompactionConfig — 权威源在 `axagent_harness::runtime_types::compact`
pub use axagent_harness::runtime_types::compact::{
    CompactionConfig, CompactionResult, emergency_compaction_config, estimate_message_tokens,
    estimate_session_tokens, should_compact,
};

/// 摘要归一化 / 续接消息构造 — 权威源在 `axagent_harness::compact_session`
///
/// 此前本文件与 harness 各存一份实现（连同 `extract_tag_block` /
/// `strip_tag_block` / `collapse_blank_lines` 三个私有助手），改一处必漏一处。
/// 统一 re-export harness 版本。
///
/// 收敛时发现 harness 侧 `extract_existing_compacted_summary` **缺少空串守卫** ——
/// prompt 取空串（如 `NoopPromptProvider`）时，分隔符退化成 `"\n\n"` / `"\n"`，
/// 会把既有摘要截成第一行 `"Summary:"`，二次压缩丢失「之前已压缩上下文」。
/// 已把守卫补进 harness 权威实现，并加回归测试
/// `extract_existing_summary_keeps_text_when_prompt_strings_are_empty`。
pub use axagent_harness::compact_session::{
    format_compact_summary, get_compact_continuation_message,
};

/// 使用多层阈值系统判断是否需要压缩（增强版）。
///
/// 与 `should_compact` 不同，此函数：
/// - 考虑有效上下文窗口大小
/// - 使用四层阈值（warning / auto_compact / error / blocking_limit）
/// - 返回详细的阈值状态而非简单布尔值
///
/// # 参数
/// - `session`: 当前会话
/// - `effective_window`: 模型的有效上下文窗口 token 数
#[must_use]
pub fn evaluate_compact_threshold(
    session: &Session,
    effective_window: u64,
) -> crate::compact_thresholds::CompactThresholdState {
    crate::compact_thresholds::CompactThresholdState::compute(session, effective_window)
}

/// 获取建议的压缩配置，根据当前阈值状态自动调整激进程度。
///
/// 越接近上下文窗口限制，配置越激进（保留更少的最近消息）。
#[must_use]
pub fn adaptive_compaction_config(session: &Session, effective_window: u64) -> CompactionConfig {
    crate::compact_thresholds::recommended_compaction_config(session, effective_window)
}

/// 智能压缩：优先尝试会话记忆压缩，失败时回退到传统 LLM 压缩。
///
/// 此函数将 session_memory_compact 和传统的 compact_session 串联起来：
/// 1. 如果有结构化记忆可用，先尝试 session_memory_compact
/// 2. 如果记忆压缩成功，返回其结果
/// 3. 如果记忆压缩不适用或失败，回退到传统 compact_session
///
/// # 参数
/// - `session`: 要压缩的会话
/// - `config`: 基础压缩配置
/// - `memories`: 从轨迹系统提取的结构化记忆（可为空）
#[must_use]
pub fn smart_compact(
    session: &Session,
    config: CompactionConfig,
    memories: &[crate::session_memory_compact::StructuredMemory],
    provider: &dyn PromptProvider,
) -> CompactionResult {
    // 尝试会话记忆压缩
    let sm_config = crate::session_memory_compact::SessionMemoryCompactConfig::default();
    if let Some(sm_result) = crate::session_memory_compact::try_session_memory_compact(
        session, memories, &sm_config, config,
    ) {
        return crate::session_memory_compact::to_compaction_result(&sm_result, session);
    }

    // 回退到传统 LLM 压缩
    compact_session(session, config, provider)
}

/// Compacts a session by summarizing older messages and preserving the recent tail.
///
/// 算法本体（tool-use/tool-result 边界回退、重要性评分、摘要续接）**只有一份实现** ——
/// `axagent_harness::compact_session::compact_session`。本函数只在其前后补 runtime-core
/// 专属的 PreCompact / PostCompact hook。
///
/// 收敛理由：此前 harness 与本文件各存一份 `compact_session`（含相同的边界回退与
/// 重要性评分分支），改一处必漏一处；两版的重要性评分同源于
/// `axagent_harness::runtime_types::compact`（本 crate 的 `message_importance` 只是它的
/// re-export）。收敛路径上唯一的实际差异是 `extract_existing_compacted_summary` 的
/// 空串守卫（见文件头说明），已补进 harness 权威实现 —— 因此收敛后行为与收敛前一致，
/// 并有 `keeps_previous_compacted_context_when_compacting_again` 锁住。
#[must_use]
pub fn compact_session(
    session: &Session,
    config: CompactionConfig,
    provider: &dyn PromptProvider,
) -> CompactionResult {
    // PreCompact hook — 在压缩前通知外部监听器
    let _ = crate::hooks::HookRunner::new(crate::config::RuntimeHookConfig::default()).run_event(
        crate::hooks::HookEvent::PreCompact,
        &serde_json::json!({
            "session_id": session.session_id,
            "message_count": session.messages.len(),
        })
        .to_string(),
    );

    let result = axagent_harness::compact_session::compact_session(session, config, provider);

    // PostCompact hook — 压缩完成后通知外部监听器
    let _ = crate::hooks::HookRunner::new(crate::config::RuntimeHookConfig::default()).run_event(
        crate::hooks::HookEvent::PostCompact,
        &serde_json::json!({
            "session_id": session.session_id,
            "removed_messages": result.removed_message_count,
            "remaining_messages": result.compacted_session.messages.len(),
        })
        .to_string(),
    );

    result
}

fn first_text_block(message: &ConversationMessage) -> Option<&str> {
    message.blocks.iter().find_map(|block| match block {
        ContentBlock::Text { text } if !text.trim().is_empty() => Some(text.as_str()),
        ContentBlock::ToolUse { .. }
        | ContentBlock::ToolResult { .. }
        | ContentBlock::Text { .. } => None,
    })
}

/// Extract a concise summary of a single conversation turn.
///
/// Scans the message blocks for user intent, tool usage, and results,
/// producing a short one-line description suitable for inclusion in
/// compacted summaries.
#[must_use]
pub fn summarize_turn(messages: &[ConversationMessage]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for message in messages {
        match message.role {
            MessageRole::User => {
                if let Some(text) = first_text_block(message) {
                    let short = text.chars().take(200).collect::<String>();
                    if !short.trim().is_empty() {
                        parts.push(format!("User: {}", short));
                    }
                }
            },
            MessageRole::Assistant => {
                let tool_uses: Vec<&str> = message
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolUse { name, .. } => Some(name.as_str()),
                        _ => None,
                    })
                    .collect();
                if !tool_uses.is_empty() {
                    parts.push(format!("Used: {}", tool_uses.join(", ")));
                } else if let Some(text) = first_text_block(message) {
                    let short = text.chars().take(150).collect::<String>();
                    if !short.trim().is_empty() {
                        parts.push(short);
                    }
                }
            },
            MessageRole::Tool => {
                for block in &message.blocks {
                    if let ContentBlock::ToolResult { tool_name, output, is_error, .. } = block {
                        let status = if *is_error { "failed" } else { "ok" };
                        let output_short = output.chars().take(80).collect::<String>();
                        parts.push(format!("{tool_name}: {status} ({output_short})"));
                    }
                }
            },
            MessageRole::System => {},
        }
    }

    if parts.is_empty() {
        "(empty turn)".to_string()
    } else {
        parts.join(" | ")
    }
}

/// Compute a relevance decay weight based on distance from the current turn.
///
/// Messages closer to the current turn receive higher weight.
/// Uses exponential decay: `weight = base_weight * decay_factor^(distance)`
///
/// `position` is 0-indexed from the end (0 = most recent, N = furthest back).
#[must_use]
pub fn decay_weight(position: usize, base_weight: f64, decay_factor: f64) -> f64 {
    if decay_factor <= 0.0 || decay_factor >= 1.0 {
        return base_weight;
    }
    base_weight * decay_factor.powi(position as i32)
}

/// Detect task boundaries in a sequence of messages and return the index
/// after which earlier messages can be safely cleaned up.
///
/// A task boundary is detected when:
/// - A user message signals task completion (e.g. "thanks", "done", "looks good")
/// - A significant gap in conversation context is detected
/// - A new, distinct task request begins
///
/// Returns `Some(index)` of the first message of the new task, or `None`
/// if no clear boundary is found.
#[must_use]
pub fn detect_task_boundary(messages: &[ConversationMessage]) -> Option<usize> {
    if messages.len() < 4 {
        return None;
    }

    let completion_markers = [
        "thanks",
        "thank you",
        "done",
        "looks good",
        "lgtm",
        "works",
        "working",
        "perfect",
        "great",
        "awesome",
        "completed",
        "resolved",
        "fixed",
    ];

    let new_task_markers = [
        "now let's",
        "next,",
        "can you also",
        "additionally",
        "separately",
        "another thing",
        "new task",
        "moving on",
        "also,",
        "one more",
        "by the way",
    ];

    // Search from newest backwards for completion markers followed by new task
    for i in (1..messages.len()).rev() {
        if messages[i].role == MessageRole::User
            && let Some(text) = first_text_block(&messages[i])
        {
            let lowered = text.to_lowercase();
            // Check if this is a new task request
            if new_task_markers.iter().any(|m| lowered.contains(m)) {
                return Some(i);
            }
        }
        // Check if the previous message pair signals completion
        if i > 0
            && messages[i - 1].role == MessageRole::User
            && let Some(text) = first_text_block(&messages[i - 1])
        {
            let lowered = text.to_lowercase();
            if completion_markers.iter().any(|m| lowered.contains(m)) {
                // Found completion — check if next message is a new task
                if i < messages.len()
                    && messages[i].role == MessageRole::User
                    && let Some(next_text) = first_text_block(&messages[i])
                {
                    let next_lower = next_text.to_lowercase();
                    if new_task_markers.iter().any(|m| next_lower.contains(m))
                        || !completion_markers.iter().any(|m| next_lower.contains(m))
                    {
                        return Some(i);
                    }
                }
            }
        }
    }

    None
}

/// Clean up messages before a detected task boundary.
///
/// When a task boundary is found at `boundary_index`, messages before that
/// index can be replaced with a compact summary, reducing context bloat
/// from completed tasks.
///
/// Returns the number of messages that should be compacted (pre-boundary count).
#[must_use]
pub fn cleanup_task_boundary(messages: &[ConversationMessage]) -> Option<usize> {
    detect_task_boundary(messages)
}

#[cfg(test)]
mod tests {
    use super::{
        CompactionConfig, compact_session, format_compact_summary,
        get_compact_continuation_message, should_compact,
    };
    use crate::session::{
        ContentBlock, ConversationMessage, ConversationMessageExt, MessageRole, Session,
    };
    use axagent_harness::prompt_provider::NoopPromptProvider;

    const NP: &NoopPromptProvider = &NoopPromptProvider;

    #[test]
    fn formats_compact_summary_like_upstream() {
        let summary = "<analysis>scratch</analysis>\n<summary>Kept work</summary>";
        assert_eq!(format_compact_summary(summary), "Summary:\nKept work");
    }

    #[test]
    fn leaves_small_sessions_unchanged() {
        let mut session = Session::new();
        session.messages = vec![ConversationMessage::user_text("hello")];

        let result = compact_session(&session, CompactionConfig::default(), NP);
        assert_eq!(result.removed_message_count, 0);
        assert_eq!(result.compacted_session, session);
        assert!(result.summary.is_empty());
        assert!(result.formatted_summary.is_empty());
    }

    #[test]
    fn compacts_older_messages_into_a_system_summary() {
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage::user_text("one ".repeat(200)),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: "two ".repeat(200) }]),
            ConversationMessage::tool_result("1", "bash", "ok ".repeat(200), false),
            ConversationMessage {
                role: MessageRole::Assistant,
                blocks: vec![ContentBlock::Text { text: "recent".to_string() }],
                usage: None,
            },
        ];

        let result = compact_session(
            &session,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 1,
                ..Default::default()
            },
            NP,
        );
        // one extra message to avoid an orphaned tool result at the boundary.
        // messages[1] (assistant) must be kept along with messages[2] (tool result).
        assert!(
            result.removed_message_count <= 2,
            "expected at most 2 removed, got {}",
            result.removed_message_count
        );
        assert_eq!(result.compacted_session.messages[0].role, MessageRole::System);
        assert!(matches!(
            &result.compacted_session.messages[0].blocks[0],
            ContentBlock::Text { text } if text.contains("Summary:")
        ));
        assert!(result.formatted_summary.contains("Scope:"));
        assert!(result.formatted_summary.contains("Key timeline:"));
        assert!(should_compact(
            &session,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 1,
                ..Default::default()
            },
            NP,
        ));
        // Note: with the tool-use/tool-result boundary guard the compacted session
        // may preserve one extra message at the boundary, so token reduction is
        // not guaranteed for small sessions. The invariant that matters is that
        // the removed_message_count is non-zero (something was compacted).
        assert!(result.removed_message_count > 0, "compaction must remove at least one message");
    }

    #[test]
    fn keeps_previous_compacted_context_when_compacting_again() {
        let mut initial_session = Session::new();
        initial_session.messages = vec![
            ConversationMessage::user_text("Investigate rust/crates/runtime/src/compact.rs"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "I will inspect the compact flow.".to_string(),
            }]),
            ConversationMessage::user_text("Also update rust/crates/runtime/src/conversation.rs"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "Next: preserve prior summary context during auto compact.".to_string(),
            }]),
        ];
        let config = CompactionConfig {
            preserve_recent_messages: 2,
            max_estimated_tokens: 1,
            ..Default::default()
        };

        let first = compact_session(&initial_session, config, NP);
        let mut follow_up_messages = first.compacted_session.messages.clone();
        follow_up_messages.extend([
            ConversationMessage::user_text("Please add regression tests for compaction."),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "Working on regression coverage now.".to_string(),
            }]),
        ]);

        let mut second_session = Session::new();
        second_session.messages = follow_up_messages;
        let second = compact_session(&second_session, config, NP);

        assert!(second.formatted_summary.contains("Previously compacted context:"));
        assert!(second.formatted_summary.contains("Scope: 2 earlier messages compacted"));
        assert!(second.formatted_summary.contains("Newly compacted context:"));
        assert!(
            second
                .formatted_summary
                .contains("Also update rust/crates/runtime/src/conversation.rs")
        );
        assert!(matches!(
            &second.compacted_session.messages[0].blocks[0],
            ContentBlock::Text { text }
                if text.contains("Previously compacted context:")
                    && text.contains("Newly compacted context:")
        ));
        assert!(matches!(
            &second.compacted_session.messages[1].blocks[0],
            ContentBlock::Text { text } if text.contains("Please add regression tests for compaction.")
        ));
    }

    #[test]
    fn ignores_existing_compacted_summary_when_deciding_to_recompact() {
        let summary = "<summary>Conversation summary:\n- Scope: earlier work preserved.\n- Key timeline:\n  - user: large preserved context\n</summary>";
        let mut session = Session::new();
        session.messages = vec![
            ConversationMessage {
                role: MessageRole::System,
                blocks: vec![ContentBlock::Text {
                    text: get_compact_continuation_message(summary, true, true, NP),
                }],
                usage: None,
            },
            ConversationMessage::user_text("tiny"),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: "recent".to_string() }]),
        ];

        // 意图：摘要自身不计入判定（start=1），剩余 2 条 ≤ preserve_recent → 不再压缩。
        // token 阈值必须取不会误触发的值 —— should_compact 是「条数或 token 任一超限即压缩」，
        // 之前这里写 max=1，"tiny"+"recent" 共约 4 token 必然触发，测的根本不是本意。
        assert!(!should_compact(
            &session,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 10_000,
                ..Default::default()
            },
            NP,
        ));

        // 反向锚定：同配置下若把摘要也算进判定（模拟实现退化），消息数 3 > 2 必然触发 ——
        // 保证本测试真的能捕获「摘要未被忽略」的回归，而非恒假通过。
        let mut no_summary = Session::new();
        no_summary.messages = session.messages[1..].to_vec();
        no_summary.messages.insert(0, ConversationMessage::user_text("older"));
        no_summary.messages.insert(1, ConversationMessage::user_text("older2"));
        assert!(should_compact(
            &no_summary,
            CompactionConfig {
                preserve_recent_messages: 2,
                max_estimated_tokens: 10_000,
                ..Default::default()
            },
            NP,
        ));
    }

    /// Regression: compaction must not split an assistant(ToolUse) /
    /// user(ToolResult) pair at the boundary. An orphaned tool-result message
    /// without the preceding assistant `tool_calls` causes a 400 on the
    /// OpenAI-compat path (gaebal-gajae repro 2026-04-09).
    #[test]
    fn compaction_does_not_split_tool_use_tool_result_pair() {
        use crate::session::{ContentBlock, Session};

        let tool_id = "call_abc";
        let mut session = Session::default();
        // Turn 1: user prompt
        session
            .push_message(ConversationMessage::user_text("Search for files"))
            .expect("测试应成功");
        // Turn 2: assistant calls a tool
        session
            .push_message(ConversationMessage::assistant(vec![ContentBlock::ToolUse {
                id: tool_id.to_string(),
                name: "search".to_string(),
                input: "{\"q\":\"*.rs\"}".to_string(),
            }]))
            .expect("测试应成功");
        // Turn 3: tool result
        session
            .push_message(ConversationMessage::tool_result(
                tool_id,
                "search",
                "found 5 files",
                false,
            ))
            .expect("测试应成功");
        // Turn 4: assistant final response
        session
            .push_message(ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "Done.".to_string(),
            }]))
            .expect("测试应成功");

        // Compact preserving only 1 recent message — without the fix this
        // would cut the boundary so that the tool result (turn 3) is first,
        // without its preceding assistant tool_calls (turn 2).
        let config =
            CompactionConfig { preserve_recent_messages: 1, ..CompactionConfig::default() };
        let result = compact_session(&session, config, NP);
        // After compaction, no two consecutive messages should have the pattern
        // tool_result immediately following a non-assistant message (i.e. an
        // orphaned tool result without a preceding assistant ToolUse).
        let messages = &result.compacted_session.messages;
        for i in 1..messages.len() {
            let curr_is_tool_result = messages[i]
                .blocks
                .first()
                .is_some_and(|b| matches!(b, ContentBlock::ToolResult { .. }));
            if curr_is_tool_result {
                let prev_has_tool_use = messages[i - 1]
                    .blocks
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
                assert!(
                    prev_has_tool_use,
                    "message[{}] is a ToolResult but message[{}] has no ToolUse: {:?}",
                    i,
                    i - 1,
                    messages[i - 1].blocks
                );
            }
        }
    }

    #[test]
    fn test_summarize_turn_simple() {
        let messages = vec![
            ConversationMessage::user_text("Fix the bug in main.rs"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "I'll fix that.".to_string(),
            }]),
        ];
        let summary = super::summarize_turn(&messages);
        assert!(summary.contains("Fix the bug"));
        assert!(summary.contains("I'll fix that"));
    }

    #[test]
    fn test_summarize_turn_with_tools() {
        let messages = vec![
            ConversationMessage::user_text("Read the file"),
            ConversationMessage {
                role: MessageRole::Assistant,
                blocks: vec![ContentBlock::ToolUse {
                    id: "1".to_string(),
                    name: "read_file".to_string(),
                    input: "main.rs".to_string(),
                }],
                usage: None,
            },
            ConversationMessage::tool_result("1", "read_file", "file contents", false),
        ];
        let summary = super::summarize_turn(&messages);
        assert!(summary.contains("Used: read_file"));
        assert!(summary.contains("read_file: ok"));
    }

    #[test]
    fn test_decay_weight_values() {
        let w0 = super::decay_weight(0, 1.0, 0.9);
        let w1 = super::decay_weight(1, 1.0, 0.9);
        let w5 = super::decay_weight(5, 1.0, 0.9);
        assert!((w0 - 1.0).abs() < 0.001);
        assert!((w1 - 0.9).abs() < 0.001);
        assert!(w5 < w1);
        assert!(w5 > 0.4);
    }

    #[test]
    fn test_decay_weight_invalid_factor() {
        let w = super::decay_weight(3, 1.0, 1.5);
        assert!((w - 1.0).abs() < 0.001);
        let w2 = super::decay_weight(3, 1.0, 0.0);
        assert!((w2 - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detect_task_boundary_finds_transition() {
        let messages = vec![
            ConversationMessage::user_text("Fix the bug"),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: "Done".to_string() }]),
            ConversationMessage::user_text("Thanks, looks good!"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "You're welcome".to_string(),
            }]),
            ConversationMessage::user_text("Now let's add a new feature"),
        ];
        let boundary = super::detect_task_boundary(&messages);
        assert_eq!(boundary, Some(4));
    }

    #[test]
    fn test_detect_task_boundary_short_conversation() {
        let messages = vec![
            ConversationMessage::user_text("Hi"),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: "Hello".to_string() }]),
        ];
        let boundary = super::detect_task_boundary(&messages);
        assert_eq!(boundary, None);
    }

    #[test]
    fn test_cleanup_task_boundary_returns_count() {
        let messages = vec![
            ConversationMessage::user_text("do task A"),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: "done A".to_string() }]),
            ConversationMessage::user_text("thanks, looks good"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "welcome".to_string(),
            }]),
            ConversationMessage::user_text("Now let's do task B"),
        ];
        let cleanup = super::cleanup_task_boundary(&messages);
        assert_eq!(cleanup, Some(4));
    }
}
