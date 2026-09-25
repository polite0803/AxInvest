// SPDX-License-Identifier: AGPL-3.0-only

//! session_events 事件流 —— 跨进程 Resume 的持久化载体。
//!
//! ## 定位
//! `session_events` 表的权威 DTO + 写入 trait。
//! 与 `messages` 表（持久化对话文本）、`trajectory_steps` 表（RL 学习轨迹）
//! 形成互补：session_events 只关心「哪一步开始/结束/中断/压缩」
//! 这种**执行态**事件，不关心具体 LLM 输出文本。
//!
//! ## 核心用例
//! - 进程在 `run_turn` 中途 kill → 下次 `agent_resume_from_events`
//!   从事件流重建 `ThoughtChain` + `ContextWindow`，续上 Interrupted 点继续。
//! - P1-1 TokenBudget compact → 发 `Compacted` 事件落表，便于追溯。
//! - retention 归并：`TurnEnded` 后一段时间归并早期事件（防表膨胀）。
//!
//! ## 事件写入频率
//! 只在**结构性节点**写事件：`TurnStarted` / `ToolCall` / `ToolResult` / `Compacted` / `TurnEnded`。
//! 不在每轮 `while !state.is_terminal()` 循环里都写 —— 避免和 `messages` 表重复。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// session_events 事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventType {
    /// run_turn 开始（一次完整对话 turn 的开始点）
    TurnStarted,
    /// assistant 消息已落库（可选，用于前端时间线标记）
    Message,
    /// tool_call 已发出（含 id / name / input）
    ToolCall,
    /// tool_result 已返回（含 tool_use_id / output / is_error）
    ToolResult,
    /// TokenBudget compact 执行完毕（含被压缩步骤的摘要）
    Compacted,
    /// run_turn 正常结束（Reached terminal state）
    TurnEnded,
    /// 进程在 run_turn 中途 kill（未配对的 ToolCall 视为 Interrupted）
    Interrupted,
}

impl SessionEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionEventType::TurnStarted => "turn_started",
            SessionEventType::Message => "message",
            SessionEventType::ToolCall => "tool_call",
            SessionEventType::ToolResult => "tool_result",
            SessionEventType::Compacted => "compacted",
            SessionEventType::TurnEnded => "turn_ended",
            SessionEventType::Interrupted => "interrupted",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "turn_started" => SessionEventType::TurnStarted,
            "message" => SessionEventType::Message,
            "tool_call" => SessionEventType::ToolCall,
            "tool_result" => SessionEventType::ToolResult,
            "compacted" => SessionEventType::Compacted,
            "turn_ended" => SessionEventType::TurnEnded,
            "interrupted" => SessionEventType::Interrupted,
            _ => return None,
        })
    }
}

/// session_events 行的 payload JSON 字段反序列化目标。
///
/// 不同事件类型的 payload 结构不同：
/// - `TurnStarted`：无 payload（NULL）
/// - `Message`：`{ role: "assistant", content_preview: "..." }`
/// - `ToolCall`：`{ id, name, input }`
/// - `ToolResult`：`{ tool_use_id, output_preview, is_error }`
/// - `Compacted`：`{ summary_preview, covered_range, tokens_before, tokens_after }`
///   （由 [`build_compacted_event_payload`] 构造；`covered_range` 为被摘要覆盖的
///   消息区间 `[start, end)`，用于回放时还原「当时模型看到的是摘要」）
/// - `TurnEnded`：`{ outcome: "success" | "failure", total_iterations, total_tokens }`
/// - `Interrupted`：`{ last_event_seq, reason: "tool_call_without_result" | "kill" }`
///
/// 统一用 `serde_json::Value` 存，避免为每类定义独立 DTO 做频繁序列化。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEventPayload {
    #[serde(rename = "toolUseId", skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(rename = "toolName", skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(rename = "contentPreview", skip_serializing_if = "Option::is_none")]
    pub content_preview: Option<String>,
    #[serde(rename = "stepCountBefore", skip_serializing_if = "Option::is_none")]
    pub step_count_before: Option<i64>,
    #[serde(rename = "stepCountAfter", skip_serializing_if = "Option::is_none")]
    pub step_count_after: Option<i64>,
    /// `Compacted`：被摘要覆盖的原始消息区间 `[start, end)`（end 不含）
    #[serde(rename = "coveredRange", skip_serializing_if = "Option::is_none")]
    pub covered_range: Option<(usize, usize)>,
    /// `Compacted`：压缩后模型实际看到的摘要文本（截断为预览）
    #[serde(rename = "summaryPreview", skip_serializing_if = "Option::is_none")]
    pub summary_preview: Option<String>,
    /// `Compacted`：压缩前会话 token 估算
    #[serde(rename = "tokensBefore", skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
    /// `Compacted`：压缩后会话 token 估算
    #[serde(rename = "tokensAfter", skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 兜底：额外自定义字段（用 serde_json::Map 追加）
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `Compacted` 事件里摘要预览的字符上限。
///
/// 取 2000 字符：足够还原「当时模型看到的是摘要」这一事实与摘要主体，
/// 又不至于让 `session_events` 表随压缩次数线性膨胀（事件表定位是**执行态**轨迹，
/// 完整对话文本另有 `messages` 表承载）。
pub const COMPACT_SUMMARY_PREVIEW_CHARS: usize = 2000;

/// 构造 `Compacted` 事件的 payload。
///
/// 抽到 harness 是为了让 agent（`session_manager` 的 turn 前压缩）与
/// runtime-core（`ConversationRuntime` 的阈值/轮次压缩）两条生产路径
/// **发出结构完全一致**的事件 —— 否则回放侧要按调用方分叉解析。
///
/// # 参数
/// - `summary`：压缩产出、随后被写入续接 system 消息的摘要文本；
/// - `covered_range`：被摘要覆盖的原始消息区间 `[start, end)`；
/// - `tokens_before` / `tokens_after`：压缩前后会话 token 估算。
#[must_use]
pub fn build_compacted_event_payload(
    summary: &str,
    covered_range: Option<(usize, usize)>,
    tokens_before: u64,
    tokens_after: u64,
) -> serde_json::Value {
    let payload = SessionEventPayload {
        summary_preview: Some(preview_chars(summary, COMPACT_SUMMARY_PREVIEW_CHARS)),
        covered_range,
        tokens_before: Some(tokens_before),
        tokens_after: Some(tokens_after),
        ..Default::default()
    };
    serde_json::to_value(payload).unwrap_or(serde_json::Value::Null)
}

/// 按**字符**（非字节）截断预览，避免在多字节边界切开 UTF-8。
fn preview_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars).collect();
    truncated.push('…');
    truncated
}

/// 单条 session_event（从 DB 读回后的值对象）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub id: i64,
    pub session_id: String,
    pub seq: i64,
    pub event_type: SessionEventType,
    pub payload: Option<SessionEventPayload>,
    pub created_at: DateTime<Utc>,
}

/// session_events 写入 trait（SessionManager 实现）。
///
/// harness 定义 trait（consumer 层依赖），agent 层 SessionManager 实现
/// （impl SessionEventSink for SessionManager，内部经 dao repo 落表）。
///
/// 所有 async 方法返回 `Result<(), String>` —— 简化层间错误处理
/// （impl 内部只做 try_insert + log，事件落表失败不阻塞主流程）。
#[async_trait::async_trait]
pub trait SessionEventSink: Send + Sync {
    /// 发一个事件。impl 内部保证按 seq 递增 + 失败静默 log。
    async fn emit(
        &self,
        session_id: &str,
        event_type: SessionEventType,
        payload: Option<serde_json::Value>,
    );

    /// 批量发事件（compact 后可能需要发一条 Compacted + 若干 Message 事件）。
    async fn emit_many(
        &self,
        session_id: &str,
        events: Vec<(SessionEventType, Option<serde_json::Value>)>,
    ) {
        for (ty, payload) in events {
            self.emit(session_id, ty, payload).await;
        }
    }

    /// 清理某个 session 的全部事件（retention 归并或显式清除）。
    async fn clear(&self, session_id: &str);
}

/// 默认空实现（no-op sink）—— 当没有 DB connection 或 CLI 模式下，
/// 可以用空 sink 占位，不阻塞主流程。
pub struct NullSessionEventSink;

#[async_trait::async_trait]
impl SessionEventSink for NullSessionEventSink {
    async fn emit(
        &self,
        _session_id: &str,
        _event_type: SessionEventType,
        _payload: Option<serde_json::Value>,
    ) {
    }
    async fn clear(&self, _session_id: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_roundtrip() {
        for (ty, s) in [
            (SessionEventType::TurnStarted, "turn_started"),
            (SessionEventType::Message, "message"),
            (SessionEventType::ToolCall, "tool_call"),
            (SessionEventType::ToolResult, "tool_result"),
            (SessionEventType::Compacted, "compacted"),
            (SessionEventType::TurnEnded, "turn_ended"),
            (SessionEventType::Interrupted, "interrupted"),
        ] {
            assert_eq!(ty.as_str(), s);
            assert_eq!(SessionEventType::from_str(s), Some(ty));
        }
    }

    #[test]
    fn event_type_unknown_returns_none() {
        assert_eq!(SessionEventType::from_str("bogus"), None);
        assert_eq!(SessionEventType::from_str(""), None);
    }

    #[test]
    fn payload_roundtrip_tool_result() {
        let p = SessionEventPayload {
            tool_use_id: Some("call_1".into()),
            tool_name: Some("bash".into()),
            output: Some("hello".into()),
            is_error: Some(false),
            ..Default::default()
        };
        let json = serde_json::to_value(&p).unwrap();
        let back: SessionEventPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.tool_use_id.as_deref(), Some("call_1"));
        assert_eq!(back.tool_name.as_deref(), Some("bash"));
        assert_eq!(back.is_error, Some(false));
    }

    #[test]
    fn compacted_payload_shape_roundtrips() {
        let json = build_compacted_event_payload("早期对话摘要", Some((1, 13)), 120_000, 8_000);
        // 区间以 JSON 数组承载，回放侧按 [start, end) 还原被替换的消息范围。
        assert_eq!(json.get("coveredRange"), Some(&serde_json::json!([1, 13])));
        let back: SessionEventPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.summary_preview.as_deref(), Some("早期对话摘要"));
        assert_eq!(back.covered_range, Some((1, 13)));
        assert_eq!(back.tokens_before, Some(120_000));
        assert_eq!(back.tokens_after, Some(8_000));
    }

    #[test]
    fn compacted_payload_keeps_short_summary_intact() {
        let json = build_compacted_event_payload("短摘要", None, 10, 5);
        let back: SessionEventPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.summary_preview.as_deref(), Some("短摘要"));
        assert_eq!(back.covered_range, None);
    }

    #[test]
    fn compacted_payload_truncates_preview_by_chars_not_bytes() {
        // 全为多字节字符：按字节截断会 panic 或产出乱码，故必须按字符截断。
        let summary = "汉".repeat(COMPACT_SUMMARY_PREVIEW_CHARS + 500);
        let json = build_compacted_event_payload(&summary, None, 0, 0);
        let back: SessionEventPayload = serde_json::from_value(json).unwrap();
        let preview = back.summary_preview.expect("预览必须存在");
        assert_eq!(preview.chars().count(), COMPACT_SUMMARY_PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));
    }
}
