// SPDX-License-Identifier: AGPL-3.0-only

//! Context manager for conversation history compression.
//!
//! Two modes:
//! - **Sliding window** (compression OFF): trims oldest messages to fit the token budget.
//! - **Compression** (manual or auto): all messages are compressed into an LLM summary,
//!   a `<!-- context-compressed -->` marker is inserted, and subsequent sends use
//!   the summary + only messages after the marker.
//!
//! Token budget management：分量预算按模型窗口取（`axagent_harness::context_budget`），
//! 由该模块统一供值，本文件的 [`token_budget`] 只是其 cap 的别名（供尚不知窗口的调用方使用）。

use axagent_harness::context_budget::budgets_for;
use axagent_harness::types::{ChatContent, ChatMessage};
use axagent_harness::util_fns::truncate_to_char_boundary;
use axagent_kit::token_counter;

/// Token budget allocation — 各分量的**上限**（cap）。
///
/// ⚠ 这些常数不再直接当作预算用：已知模型窗口时应走
/// [`axagent_harness::context_budget::budgets_for`] 取 `min(ratio × window, cap)`。
/// 保留本模块是为了给**尚不知窗口**的调用方（如 RAG 注入、技能索引、
/// 上下文分解展示）一个稳定引用，取值与
/// `axagent_harness::context_budget::*_CAP` 同源（非重复定义）。
///
/// 只保留**确有消费方**的别名：`NUDGES` 与 `HISTORY_RATIO` 在全仓零引用，已删除
/// （`-D dead-code` 会拦；无消费方的别名属投机性定义，不该留）。
/// 需要这两者的调用方请直接用 `axagent_harness::context_budget` 的同名常量。
///
/// Note: the permission notice rendered by [`render_permission_notice`] is pushed
/// into the system messages and therefore spends part of the `SYSTEM_PROMPT`
/// allowance — it has no separate budget of its own.
pub mod token_budget {
    /// system prompt 上限。Includes the permission notice
    /// (sandbox mode + approval policy) rendered by [`super::render_permission_notice`].
    pub const SYSTEM_PROMPT: usize = axagent_harness::context_budget::SYSTEM_PROMPT_CAP;
    /// working memory 注入上限。
    pub const WORKING_MEMORY: usize = axagent_harness::context_budget::WORKING_MEMORY_CAP;
    /// RAG 检索结果注入上限。
    pub const RETRIEVED_MEMORIES: usize = axagent_harness::context_budget::RETRIEVED_MEMORIES_CAP;
    /// 技能索引上限。
    pub const SKILLS: usize = axagent_harness::context_budget::SKILLS_CAP;
}

/// Content string for the compression marker message.
pub const COMPRESSION_MARKER: &str = "<!-- context-compressed -->";

/// 权限说明段的模板（给模型看的 system prompt 片段，不是 UI 文本，
/// 因此不进 `src/i18n/locales/`）。
mod permission_templates {
    /// 沙箱档位片段，与 `axagent_harness::SandboxMode` 一一对应。
    pub mod sandbox_mode {
        pub const READ_ONLY: &str = include_str!("prompts/permissions/sandbox_mode/read-only.md");
        pub const WORKSPACE_WRITE: &str =
            include_str!("prompts/permissions/sandbox_mode/workspace-write.md");
        pub const DANGER_FULL_ACCESS: &str =
            include_str!("prompts/permissions/sandbox_mode/danger-full-access.md");
    }

    /// 审批档位片段，与 `axagent_harness::ApprovalPolicy` 一一对应。
    pub mod approval_policy {
        pub const UNTRUSTED: &str =
            include_str!("prompts/permissions/approval_policy/untrusted.md");
        pub const ON_FAILURE: &str =
            include_str!("prompts/permissions/approval_policy/on-failure.md");
        pub const ON_REQUEST: &str =
            include_str!("prompts/permissions/approval_policy/on-request.md");
        pub const NEVER: &str = include_str!("prompts/permissions/approval_policy/never.md");
    }
}

/// 按当前沙箱档位 + 审批档位渲染权限说明段（纯函数）。
///
/// 目的：让模型知道自己在什么边界内工作，从而主动规避越界写路径、主动请求审批，
/// 而不是先撞沙箱再重试。对标 codex 的 `prompts/src/permissions_instructions.rs`
/// 把 sandbox mode / approval policy 写成 system prompt 模板的做法。
///
/// 档位语义以 `axagent_harness::SandboxMode` / `ApprovalPolicy` 为唯一来源，
/// 模板文案必须与之一致（特别是 `DangerFullAccess` 不得被描述成仍有写限制）。
#[must_use]
pub fn render_permission_notice(
    sandbox_mode: axagent_harness::SandboxMode,
    approval_policy: axagent_harness::ApprovalPolicy,
) -> String {
    use axagent_harness::{ApprovalPolicy, SandboxMode};

    let sandbox = match sandbox_mode {
        SandboxMode::ReadOnly => permission_templates::sandbox_mode::READ_ONLY,
        SandboxMode::WorkspaceWrite => permission_templates::sandbox_mode::WORKSPACE_WRITE,
        SandboxMode::DangerFullAccess => permission_templates::sandbox_mode::DANGER_FULL_ACCESS,
    };
    let approval = match approval_policy {
        ApprovalPolicy::Untrusted => permission_templates::approval_policy::UNTRUSTED,
        ApprovalPolicy::OnFailure => permission_templates::approval_policy::ON_FAILURE,
        ApprovalPolicy::OnRequest => permission_templates::approval_policy::ON_REQUEST,
        ApprovalPolicy::Never => permission_templates::approval_policy::NEVER,
    };

    format!("<permissions>\n{}\n\n{}\n</permissions>", sandbox.trim_end(), approval.trim_end())
}

/// 把权限说明段作为一条 system message 追加到 `messages`。
///
/// 入参是 settings 里存储的档位字符串（`sandbox_mode` / `approval_policy`），
/// 未识别值沿用各自的默认档（`DangerFullAccess` / `OnRequest`）。
pub fn push_permission_notice(
    messages: &mut Vec<ChatMessage>,
    sandbox_mode: &str,
    approval_policy: &str,
) {
    let notice = render_permission_notice(
        axagent_harness::SandboxMode::from_mode_str(sandbox_mode),
        axagent_harness::ApprovalPolicy::from_policy_str(approval_policy),
    );
    messages.push(ChatMessage {
        role: "system".to_string(),
        content: ChatContent::Text(notice),
        tool_calls: None,
        tool_call_id: None,
        thinking: None,
    });
}

/// Estimate the token count of a single `ChatMessage`.
pub fn message_tokens(msg: &ChatMessage) -> usize {
    let text = match &msg.content {
        ChatContent::Text(s) => s.as_str(),
        ChatContent::Multipart(parts) => {
            return token_counter::estimate_tokens(
                &parts.iter().filter_map(|p| p.text.as_deref()).collect::<Vec<_>>().join(" "),
            ) + parts.iter().filter(|p| p.image_url.is_some()).count() * 85
                + 4;
        },
    };
    token_counter::estimate_message_tokens(&msg.role, text)
}

/// Check whether the current context exceeds the auto-compression threshold.
///
/// Returns `true` if total tokens (system + history) > model_context_window * 0.70.
///
/// When `model_context_window` is `None` (model has no configured limit), always
/// returns `false` — we never auto-compress without a known budget.
///
/// 阈值比例（0.70）与「剩余额度」工具同源：均取自
/// [`axagent_harness::context_budget::budgets_for`] 的 `auto_compact_threshold()`，
/// 本函数**不再**自行乘比例（两处各乘一次是口径漂移的典型来源）。
pub fn should_auto_compress(
    system_messages: &[ChatMessage],
    history_messages: &[ChatMessage],
    model_context_window: Option<u32>,
) -> bool {
    let context_window = match model_context_window {
        Some(v) => v as usize,
        None => return false,
    };
    let threshold = budgets_for(context_window).auto_compact_threshold();

    let total: usize = system_messages
        .iter()
        .chain(history_messages.iter())
        .map(message_tokens)
        .try_fold(0usize, |acc, tokens| acc.checked_add(tokens))
        .unwrap_or(usize::MAX);

    total > threshold
}

/// Build the final context for LLM from system messages + optional summary + history.
///
/// If a summary exists, it is prepended as a system message.
/// Sliding window is applied only when `model_context_window` is `Some`.
/// When the model has no configured limit, all history messages are included.
///
/// Uses the `token_budget` constants for budget-aware history allocation,
/// ensuring consistent token allocation across all context components.
///
/// When `query` is provided, uses relevance-based pruning instead of simple
/// sliding window, keeping messages most pertinent to the current conversation.
pub fn build_context(
    system_messages: &[ChatMessage],
    history_messages: &[ChatMessage],
    existing_summary: Option<&str>,
    model_context_window: Option<u32>,
) -> Vec<ChatMessage> {
    build_context_with_query(
        system_messages,
        history_messages,
        existing_summary,
        model_context_window,
        None,
    )
}

/// Extended version of `build_context` that accepts an optional query for
/// relevance-based history pruning.
pub fn build_context_with_query(
    system_messages: &[ChatMessage],
    history_messages: &[ChatMessage],
    existing_summary: Option<&str>,
    model_context_window: Option<u32>,
    query: Option<&str>,
) -> Vec<ChatMessage> {
    let mut out = system_messages.to_vec();

    // Insert summary as a system message if present
    if let Some(summary_text) = existing_summary {
        out.push(ChatMessage {
            role: "system".to_string(),
            content: ChatContent::Text(format!(
                "[UNTRUSTED-SOURCE:summary/conversation-history]\n\
                 [对话历史摘要 / Conversation History Summary]\n{}\n\
                 [/UNTRUSTED-SOURCE]",
                summary_text
            )),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
        });
    }

    match model_context_window {
        Some(ctx_window) => {
            // Calculate history budget: total window minus fixed component budgets.
            // 分量预算按窗口取（min(ratio × window, cap)），不再用绝对值常量 —— 小窗口
            // 下五个分量之和几乎吃掉整个窗口，历史只剩几百 token 可用的老问题由此消解。
            let budgets = budgets_for(ctx_window as usize);
            let history_budget = budgets.history_budget();
            let system_tokens: usize = out
                .iter()
                .map(message_tokens)
                .try_fold(0usize, |acc, t| acc.checked_add(t))
                .unwrap_or(usize::MAX);
            let available = history_budget.saturating_sub(system_tokens);

            // Use relevance pruning when query is available; otherwise sliding window
            let trimmed = match query {
                Some(q) if !q.is_empty() => prune_by_relevance(history_messages, q, available),
                _ => sliding_window(history_messages, available),
            };
            let trimmed_len = trimmed.len();
            out.extend(trimmed);

            // Log budget utilization for diagnostics
            let total_used: usize = out
                .iter()
                .map(message_tokens)
                .try_fold(0usize, |acc, t| acc.checked_add(t))
                .unwrap_or(usize::MAX);
            tracing::debug!(
                "Context built: {} system + {} history = {} total tokens (budget: {})",
                system_tokens,
                trimmed_len,
                total_used,
                ctx_window
            );
        },
        None => {
            // No known context limit — include all history messages
            out.extend(history_messages.iter().cloned());
        },
    }

    out
}

/// Sliding window: keep as many recent messages as fit within `budget` tokens.
/// Always includes at least the last message to prevent the current user input
/// from being silently dropped.
fn sliding_window(history: &[ChatMessage], budget: usize) -> Vec<ChatMessage> {
    if history.is_empty() {
        return Vec::new();
    }

    let mut total = 0usize;
    let mut start_idx = history.len();

    for (i, msg) in history.iter().enumerate().rev() {
        let tokens = message_tokens(msg);
        if total + tokens > budget {
            break;
        }
        total += tokens;
        start_idx = i;
    }

    // Always include at least the last message
    if start_idx == history.len() {
        start_idx = history.len() - 1;
    }

    history[start_idx..].to_vec()
}

// ─── Relevance-based pruning ───

/// Minimum number of most recent messages to always keep regardless of relevance.
const RECENCY_WINDOW: usize = 5;

/// Weight of recency vs relevance in the combined score (0.0 = pure relevance, 1.0 = pure recency).
const RECENCY_WEIGHT: f64 = 0.3;

/// Score a message's relevance to the query using TF-IDF-like word overlap.
/// Returns a score between 0.0 (irrelevant) and 1.0 (highly relevant).
fn relevance_score(message_text: &str, query_terms: &[String]) -> f64 {
    if query_terms.is_empty() || message_text.is_empty() {
        return 0.0;
    }

    let msg_lower = message_text.to_lowercase();
    let mut hits = 0usize;
    let mut total_term_weight = 0usize;

    for term in query_terms {
        let count = msg_lower.matches(term.as_str()).count();
        hits += count;
        // Weight longer terms more heavily (less likely to be noise)
        total_term_weight += term.len().max(1);
    }

    if hits == 0 {
        return 0.0;
    }

    // Normalize: hits / (msg_length * term_count) with bonus for multiple matches
    let msg_len = msg_lower.len().max(1) as f64;
    let density = hits as f64 / msg_len;
    let term_weight =
        total_term_weight as f64 / query_terms.iter().map(|t| t.len().max(1)).sum::<usize>() as f64;
    (density * term_weight).min(1.0)
}

/// Extract meaningful query terms from a user message.
fn extract_query_terms(query: &str) -> Vec<String> {
    let lower = query.to_lowercase();
    // Split on non-alphanumeric, filter short/stop words
    lower
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| w.len() >= 3)
        .filter(|w| !is_stop_word(w))
        .map(|w| w.to_string())
        .collect()
}

/// Check if a word is a common stop word (English + Chinese).
// i18n-exempt: NLP data - Chinese/English stop word list used for relevance scoring
fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "the"
            | "and"
            | "for"
            | "that"
            | "this"
            | "with"
            | "you"
            | "are"
            | "not"
            | "but"
            | "from"
            | "have"
            | "has"
            | "was"
            | "were"
            | "can"
            | "will"
            | "what"
            | "when"
            | "where"
            | "which"
            | "how"
            | "all"
            | "just"
            | "like"
            | "very"
            | "been"
            | "would"
            | "could"
            | "should"
            | "about"
            | "also"
            | "的"
            | "了"
            | "是"
            | "在"
            | "我"
            | "有"
            | "和"
            | "就"
            | "不"
            | "人"
            | "都"
            | "一"
            | "一个"
            | "上"
            | "也"
            | "很"
            | "到"
            | "说"
            | "要"
            | "去"
            | "你"
            | "会"
            | "着"
            | "没有"
            | "看"
            | "好"
            | "自己"
            | "这"
    )
}

/// Recency weight: exponential decay by position (most recent = 1.0).
fn recency_weight(position_from_end: usize, total_messages: usize) -> f64 {
    if total_messages <= RECENCY_WINDOW {
        return 1.0;
    }
    let position = position_from_end as f64;
    let max_pos = (total_messages - RECENCY_WINDOW) as f64;
    if position <= RECENCY_WINDOW as f64 {
        1.0
    } else {
        // Exponential decay after recency window
        let normalized = (position - RECENCY_WINDOW as f64) / max_pos.max(1.0);
        (-3.0 * normalized).exp()
    }
}

/// Prune history messages by relevance to the current query + recency.
///
/// Returns a subset of history that fits within `budget` tokens, prioritizing
/// messages that are both recent AND relevant to the query.
///
/// Strategy:
/// 1. Always keep the most recent `RECENCY_WINDOW` messages (guaranteed recency)
/// 2. Score remaining messages by combined relevance + recency
/// 3. Greedily select highest-scoring messages until budget exhausted
pub fn prune_by_relevance(history: &[ChatMessage], query: &str, budget: usize) -> Vec<ChatMessage> {
    if history.is_empty() || budget == 0 {
        return Vec::new();
    }

    let query_terms = extract_query_terms(query);

    // If no meaningful query terms, fall back to sliding window
    if query_terms.is_empty() {
        return sliding_window(history, budget);
    }

    let n = history.len();

    // Always include the last RECENCY_WINDOW messages
    let recency_start = n.saturating_sub(RECENCY_WINDOW);

    let mut selected: Vec<bool> = vec![false; n];
    let mut used_tokens = 0usize;

    // Mark recency window as always selected
    for i in recency_start..n {
        selected[i] = true;
        used_tokens += message_tokens(&history[i]);
    }

    // If recency window alone exceeds budget, trim from oldest within window
    if used_tokens > budget {
        let mut trimmed: Vec<ChatMessage> = Vec::new();
        let mut tokens = 0usize;
        for i in (0..n).rev() {
            if !selected[i] {
                continue;
            }
            let t = message_tokens(&history[i]);
            if tokens + t > budget {
                break;
            }
            tokens += t;
            trimmed.push(history[i].clone());
        }
        trimmed.reverse();
        return trimmed;
    }

    // Score remaining messages for relevance + recency
    let mut scored: Vec<(usize, f64)> = Vec::new();

    for (i, msg) in history.iter().enumerate().take(recency_start) {
        let content_text = match &msg.content {
            ChatContent::Text(s) => s.as_str(),
            ChatContent::Multipart(_) => "",
        };
        let rel = if content_text.is_empty() {
            0.0
        } else {
            relevance_score(content_text.trim(), &query_terms)
        };
        let rec = recency_weight(n - 1 - i, n);
        let combined = rel * (1.0 - RECENCY_WEIGHT) + rec * RECENCY_WEIGHT;

        if combined > 0.01 {
            scored.push((i, combined));
        }
    }

    // Sort by score descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Greedily fill budget
    for (idx, _score) in scored {
        let t = message_tokens(&history[idx]);
        if used_tokens + t > budget {
            break;
        }
        used_tokens += t;
        selected[idx] = true;
    }

    // Collect selected messages in order
    let result: Vec<ChatMessage> = history
        .iter()
        .enumerate()
        .filter(|(i, _)| selected[*i])
        .map(|(_, msg)| msg.clone())
        .collect();

    tracing::debug!(
        "Relevance pruning: {}/{} messages selected, {} tokens (budget: {})",
        result.len(),
        n,
        used_tokens,
        budget
    );

    result
}

/// Messages that need to be summarized (passed to LLM).
pub struct SummarizationRequest {
    /// Existing summary to merge with, if any.
    pub existing_summary: Option<String>,
    /// Messages to incorporate into the summary.
    pub messages_to_compress: Vec<ChatMessage>,
}

/// Build the LLM prompt for generating a conversation summary.
// i18n-exempt: LLM prompt templates for conversation summary/compression — model interaction data, not UI
pub fn build_summary_prompt(request: &SummarizationRequest) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    let instruction = if request.existing_summary.is_some() {
        "你是一个对话摘要助手。请将以下新增对话内容合并到已有摘要中。\n\n\
         要求：\n\
         1. 保留所有用户明确表达的需求、偏好和决策\n\
         2. 保留关键技术细节（代码片段、配置、错误信息等）\n\
         3. 保留待办事项和未解决的问题\n\
         4. 用简洁的要点形式组织\n\
         5. 如果有冲突信息，以最新的为准\n\
         6. 保持摘要简洁，不超过 500 字"
    } else {
        "你是一个对话摘要助手。请将以下对话历史压缩为简洁摘要。\n\n\
         要求：\n\
         1. 保留所有用户明确表达的需求、偏好和决策\n\
         2. 保留关键技术细节（代码片段、配置、错误信息等）\n\
         3. 保留待办事项和未解决的问题\n\
         4. 用简洁的要点形式组织\n\
         5. 保持摘要简洁，不超过 500 字"
    };

    messages.push(ChatMessage {
        role: "system".to_string(),
        content: ChatContent::Text(instruction.to_string()),
        tool_calls: None,
        tool_call_id: None,
        thinking: None,
    });

    if let Some(ref summary) = request.existing_summary {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Text(format!("已有摘要：\n{}", summary)),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
        });
    }

    let conversation_text: Vec<String> = request
        .messages_to_compress
        .iter()
        .map(|m| {
            let content_text = match &m.content {
                ChatContent::Text(s) => s.clone(),
                ChatContent::Multipart(parts) => {
                    parts.iter().filter_map(|p| p.text.as_deref()).collect::<Vec<_>>().join(" ")
                },
            };
            let truncated = if content_text.len() > 2000 {
                format!("{}...[已截断]", truncate_to_char_boundary(&content_text, 2000))
            } else {
                content_text
            };
            format!("{}: {}", m.role, truncated)
        })
        .collect();

    messages.push(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::Text(format!(
            "{}对话内容：\n{}",
            if request.existing_summary.is_some() {
                "新增"
            } else {
                ""
            },
            conversation_text.join("\n")
        )),
        tool_calls: None,
        tool_call_id: None,
        thinking: None,
    });

    messages
}

/// Build summary prompt with a custom system instruction (from settings).
pub fn build_summary_prompt_with_custom(
    request: &SummarizationRequest,
    custom_prompt: &str,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    messages.push(ChatMessage {
        role: "system".to_string(),
        content: ChatContent::Text(custom_prompt.to_string()),
        tool_calls: None,
        tool_call_id: None,
        thinking: None,
    });

    if let Some(ref summary) = request.existing_summary {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Text(format!("已有摘要：\n{}", summary)),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
        });
    }

    let conversation_text: Vec<String> = request
        .messages_to_compress
        .iter()
        .map(|m| {
            let content_text = match &m.content {
                ChatContent::Text(s) => s.clone(),
                ChatContent::Multipart(parts) => {
                    parts.iter().filter_map(|p| p.text.as_deref()).collect::<Vec<_>>().join(" ")
                },
            };
            let truncated = if content_text.len() > 2000 {
                format!("{}...[已截断]", truncate_to_char_boundary(&content_text, 2000))
            } else {
                content_text
            };
            format!("{}: {}", m.role, truncated)
        })
        .collect();

    messages.push(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::Text(format!(
            "{}对话内容：\n{}",
            if request.existing_summary.is_some() {
                "新增"
            } else {
                ""
            },
            conversation_text.join("\n")
        )),
        tool_calls: None,
        tool_call_id: None,
        thinking: None,
    });

    messages
}

#[cfg(test)]
mod permission_notice_tests {
    use super::{ChatContent, ChatMessage, push_permission_notice, render_permission_notice};
    use axagent_harness::{ApprovalPolicy, SandboxMode};

    const SANDBOX_MODES: [SandboxMode; 3] =
        [SandboxMode::ReadOnly, SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess];

    const APPROVAL_POLICIES: [ApprovalPolicy; 4] = [
        ApprovalPolicy::Untrusted,
        ApprovalPolicy::OnFailure,
        ApprovalPolicy::OnRequest,
        ApprovalPolicy::Never,
    ];

    /// 3 档沙箱 × 4 档策略 = 12 个组合，两两渲染必须不同。
    #[test]
    fn all_twelve_combinations_render_differently() {
        let mut rendered: Vec<(SandboxMode, ApprovalPolicy, String)> = Vec::new();
        for sm in SANDBOX_MODES {
            for ap in APPROVAL_POLICIES {
                rendered.push((sm, ap, render_permission_notice(sm, ap)));
            }
        }
        assert_eq!(rendered.len(), 12);
        for i in 0..rendered.len() {
            for j in (i + 1)..rendered.len() {
                let (sm_i, ap_i, ref text_i) = rendered[i];
                let (sm_j, ap_j, ref text_j) = rendered[j];
                assert_ne!(text_i, text_j, "{sm_i:?}/{ap_i:?} 与 {sm_j:?}/{ap_j:?} 渲染结果相同");
            }
        }
    }

    /// 同一沙箱档换审批档、同一审批档换沙箱档，都必须改变文本。
    #[test]
    fn each_dimension_alone_changes_output() {
        for sm in SANDBOX_MODES {
            let texts: Vec<String> =
                APPROVAL_POLICIES.iter().map(|ap| render_permission_notice(sm, *ap)).collect();
            for i in 0..texts.len() {
                for j in (i + 1)..texts.len() {
                    assert_ne!(texts[i], texts[j], "沙箱 {sm:?} 下审批档未区分");
                }
            }
        }
        for ap in APPROVAL_POLICIES {
            let texts: Vec<String> =
                SANDBOX_MODES.iter().map(|sm| render_permission_notice(*sm, ap)).collect();
            for i in 0..texts.len() {
                for j in (i + 1)..texts.len() {
                    assert_ne!(texts[i], texts[j], "审批 {ap:?} 下沙箱档未区分");
                }
            }
        }
    }

    /// 段落必须带 `<permissions>` 边界（与其它 system 段同构，便于模型与调试定位）。
    #[test]
    fn notice_is_wrapped_in_permissions_tag() {
        let text = render_permission_notice(SandboxMode::ReadOnly, ApprovalPolicy::OnRequest);
        assert!(text.starts_with("<permissions>\n"), "{text}");
        assert!(text.ends_with("\n</permissions>"), "{text}");
    }

    /// `DangerFullAccess` 的语义必须与 `sandbox_policy.rs` 一致：
    /// 不得被描述成仍有写限制（那会让模型误以为越界写会被拦，从而放弃合法写入）。
    #[test]
    fn danger_full_access_is_not_described_as_write_restricted() {
        let text =
            render_permission_notice(SandboxMode::DangerFullAccess, ApprovalPolicy::OnRequest);
        assert!(
            text.contains("No sandbox restriction is applied"),
            "DangerFullAccess 必须明写不施加沙箱限制: {text}"
        );
        // 不得出现其它两档的限制性表述
        for forbidden in ["can NOT write", "write only inside the workspace root", "are denied"] {
            assert!(
                !text.contains(forbidden),
                "DangerFullAccess 段不应含限制表述 {forbidden:?}: {text}"
            );
        }
    }

    /// 只读 / 工作区可写两档必须各保留自己的写限制声明（与枚举定义一致）。
    #[test]
    fn restrictive_modes_state_their_limits() {
        let ro = render_permission_notice(SandboxMode::ReadOnly, ApprovalPolicy::OnRequest);
        assert!(ro.contains("can NOT write"), "ReadOnly 应声明禁止写入: {ro}");
        let ww = render_permission_notice(SandboxMode::WorkspaceWrite, ApprovalPolicy::OnRequest);
        assert!(
            ww.contains("write only inside the workspace root"),
            "WorkspaceWrite 应声明仅工作区可写: {ww}"
        );
    }

    /// 注入形态：一条 system message，内容为渲染结果。
    #[test]
    fn push_appends_single_system_message() {
        let mut messages: Vec<ChatMessage> = Vec::new();
        push_permission_notice(&mut messages, "read-only", "on-failure");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "system");
        let ChatContent::Text(text) = &messages[0].content else {
            panic!("权限说明段应为纯文本");
        };
        assert_eq!(
            text,
            &render_permission_notice(SandboxMode::ReadOnly, ApprovalPolicy::OnFailure)
        );
    }

    /// 未识别档位回退默认（与 `from_mode_str` / `from_policy_str` 的承诺一致）。
    #[test]
    fn unknown_values_fall_back_to_defaults() {
        let mut messages: Vec<ChatMessage> = Vec::new();
        push_permission_notice(&mut messages, "garbage", "garbage");
        let ChatContent::Text(text) = &messages[0].content else {
            panic!("权限说明段应为纯文本");
        };
        assert_eq!(
            text,
            &render_permission_notice(SandboxMode::DangerFullAccess, ApprovalPolicy::OnRequest)
        );
    }
}

#[cfg(test)]
mod context_budget_tests {
    use super::{ChatContent, ChatMessage, budgets_for, message_tokens, should_auto_compress};

    fn msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Text(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
        }
    }

    /// `should_auto_compress` 的翻转点必须**逐 token 对齐** `budgets_for` 的阈值 ——
    /// 这是「判据与剩余额度工具共用同一供值函数」的唯一机械判据（防两处各乘一次比例）。
    #[test]
    fn auto_compress_flips_exactly_at_shared_threshold() {
        let window: u32 = 10_000;
        let threshold = budgets_for(window as usize).auto_compact_threshold();
        assert_eq!(threshold, 7_000, "10k 窗口阈值应为 70%");

        let unit = "abcdefghij".repeat(100);
        let unit_tokens = message_tokens(&msg(&unit));
        assert!(unit_tokens > 0, "计数函数必须给出正数，否则本测试无意义");

        let mut history = Vec::new();
        while !should_auto_compress(&[], &history, Some(window)) {
            history.push(msg(&unit));
            assert!(history.len() < 5_000, "构造的历史始终无法越过阈值");
        }

        let total: usize = history.iter().map(message_tokens).sum();
        assert!(total > threshold, "翻转时总量 {total} 应已越过阈值 {threshold}");

        // 少一条消息必然未越阈 —— 证明翻转点就落在阈值上，而非被别的口径误触发
        let mut below = history.clone();
        below.pop();
        let below_total: usize = below.iter().map(message_tokens).sum();
        assert!(!should_auto_compress(&[], &below, Some(window)));
        assert!(below_total <= threshold, "未翻转时总量 {below_total} 应不超过阈值 {threshold}");
    }

    /// 窗口未知时永不压缩（原语义，不得因改造而改变）。
    #[test]
    fn unknown_window_never_compresses() {
        let history: Vec<ChatMessage> = (0..50).map(|_| msg(&"x".repeat(4000))).collect();
        assert!(!should_auto_compress(&[], &history, None));
    }

    /// 小窗口下历史额度必须比大窗口**更小**（分量预算随动的直接体现）。
    #[test]
    fn smaller_window_leaves_less_history_room() {
        let small = budgets_for(32_000).history_budget();
        let reference = budgets_for(200_000).history_budget();
        assert!(small < reference, "32k 窗口的历史额度 {small} 应小于 200k 的 {reference}");
        // 32k 窗口下固定分量按比例收缩到 4 128，历史额度不再被 25 800 的绝对值吃掉
        assert_eq!(budgets_for(32_000).fixed_overhead(), 4_128);
        assert!(small > 15_000, "32k 窗口历史额度应显著高于旧绝对值口径下的 4 030，实得 {small}");
    }
}
