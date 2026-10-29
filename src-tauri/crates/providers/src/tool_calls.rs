// SPDX-License-Identifier: AGPL-3.0-only

//! tool_call 合法性的唯一收口：流式累积 → 终结 → 序列化三道口共用同一套判据。
//!
//! 线上实证（2026-10-29，自行搭建的 OpenAI 兼容中转 + DeepSeek + 股票分析工作流）：
//! 中转会丢 `tool_call` 的首帧（`id` / `function.name` 所在帧）或让 `index` 跳号。
//! 旧实现用空串补齐槽位、终结时只校验 `arguments`，于是把
//! `{"id":"","function":{"name":""}}` 原样写回下一轮请求，被上游以
//! `assistant.tool_calls contains an invalid function call` 整条拒绝。
//! 该失败是**确定性**的：坏调用已进内存历史，重试只会再撞一次，长任务因此整轮报废。
//!
//! 三条判据（与 OpenAI / DeepSeek 的线上校验对齐）：
//! 1. `function.name` 为空 ⇒ 丢弃该调用（无从执行，也无从修复）；
//! 2. `id` 为空 ⇒ 按槽位号合成，并让配对的 `tool` 消息回写同一 id（配对不破）；
//! 3. `arguments` 非法 JSON ⇒ 流式终结时上报流错误；序列化时降级为 `{}`。
//!
//! 只处理 assistant.tool_calls，不改正文、不改工具语义。

use std::collections::{BTreeMap, HashSet, VecDeque};

use axagent_harness::types::{ChatMessage, ToolCall, ToolCallFunction};

/// 单个响应里允许累积的 tool_call 槽位上限（防止异常上游把 index 推大后吃内存）。
const MAX_PENDING_TOOL_CALLS: usize = 256;

/// 校验并归一化 tool_call 的 `arguments`。
///
/// 上游模型（免费档、被截断的流、中转丢帧）可能吐出截断/畸形 JSON 参数。坏参数一旦
/// 写入会话历史，下一轮请求必被上游以 400 拒绝，且重试时历史不变 ⇒ 确定性失败。
/// 返回 `None` 表示参数非法（调用方决定丢弃还是降级）。
pub(crate) fn validate_tool_call_arguments(args: &str) -> Option<String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        // 无参调用归一化为 "{}"：部分上游对空字符串参数同样报 400
        return Some("{}".to_string());
    }
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return Some(args.to_string());
    }
    tracing::warn!(
        target: "axagent.providers.tool_call",
        args_preview = %axagent_harness::util_fns::truncate_to_char_boundary(trimmed, 200),
        "tool_call arguments 不是合法 JSON（上游截断或丢帧）"
    );
    None
}

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// 把一条上游返回的原始 tool_call 归一化为可安全写回历史的形状。
///
/// `index` 仅用于在 `id` 缺失时合成稳定标识（`call_{index}`），调用方保证同一
/// 条 assistant 消息内不重复。
pub(crate) fn normalize_tool_call(
    id: Option<&str>,
    call_type: Option<&str>,
    name: Option<&str>,
    arguments: Option<&str>,
    index: usize,
) -> Option<ToolCall> {
    let name = trimmed(name)?;
    let id = trimmed(id).map_or_else(|| format!("call_{index}"), str::to_string);
    let call_type = trimmed(call_type).map_or_else(|| "function".to_string(), str::to_string);
    let arguments = validate_tool_call_arguments(arguments.unwrap_or_default())
        .unwrap_or_else(|| "{}".to_string());
    Some(ToolCall {
        id,
        call_type,
        function: ToolCallFunction { name: name.to_string(), arguments },
    })
}

/// 把 `arguments` 归一化为 JSON **对象**（Anthropic 的 `input` / Gemini 的 `args`
/// 都只接受对象；数组、字符串、`null` 会被上游判参数不合法）。
pub(crate) fn normalize_arguments_object(args: &str) -> serde_json::Value {
    let parsed = serde_json::from_str::<serde_json::Value>(args).ok();
    match parsed {
        Some(value @ serde_json::Value::Object(_)) => value,
        Some(_) => {
            tracing::warn!(
                target: "axagent.providers.tool_call",
                args_preview = %axagent_harness::util_fns::truncate_to_char_boundary(args.trim(), 200),
                "tool_call arguments 不是 JSON 对象，降级为 {{}}"
            );
            serde_json::Value::Object(serde_json::Map::new())
        },
        None => serde_json::Value::Object(serde_json::Map::new()),
    }
}

/// 结构化协议（Anthropic / Gemini / Responses）的流式终结判据。
///
/// 这些适配器的 tool_call 是完整字段而非分片累积，因此不需要累积器；但「空名不可
/// 执行、非法参数必 400、空 id 没有配对锚点」三条判据与 OpenAI 路径完全一致。
/// 返回 `Err` 表示本轮不能写进历史（上层改发流错误，触发重新生成）。
pub(crate) fn finalize_tool_calls(
    calls: Vec<ToolCall>,
) -> Result<Option<Vec<ToolCall>>, FinalizeError> {
    let mut finalized = Vec::with_capacity(calls.len());
    for (index, mut call) in calls.into_iter().enumerate() {
        if call.function.name.trim().is_empty() {
            return Err(FinalizeError::MissingFunctionName { index });
        }
        let arguments = validate_tool_call_arguments(&call.function.arguments)
            .ok_or_else(|| FinalizeError::InvalidArguments { name: call.function.name.clone() })?;
        call.function.arguments = arguments;
        call.function.name = call.function.name.trim().to_string();
        call.id = if call.id.trim().is_empty() {
            format!("call_{index}")
        } else {
            call.id.trim().to_string()
        };
        call.call_type = if call.call_type.trim().is_empty() {
            "function".to_string()
        } else {
            call.call_type.trim().to_string()
        };
        finalized.push(call);
    }
    Ok(if finalized.is_empty() {
        None
    } else {
        Some(finalized)
    })
}

/// 结构化协议终结失败时的用户可见文案（各协议共用，只换调用名词）。
pub(crate) fn finalize_error_message(
    protocol: &str,
    call_kind: &str,
    err: &FinalizeError,
) -> String {
    let detail = match err {
        FinalizeError::MissingFunctionName { index } => {
            format!("{call_kind} at index {index} has an empty function name")
        },
        FinalizeError::InvalidArguments { name } => {
            format!("{call_kind} `{name}` produced invalid JSON arguments")
        },
    };
    format!(
        "{protocol} API error 400: assistant {detail}. Retrying to regenerate this turn is \
         recommended; the malformed call was NOT written into conversation history."
    )
}

/// 流式 tool_call 分片累积器。
///
/// 与旧 `Vec<(String, String, String, String)>` 的差别：
/// - 按 `index` 存 `BTreeMap`，**不再用空串补齐中间槽位**（跳号不会再产出幽灵调用）；
/// - `id` / `name` 只接受非空值覆盖（`id` 常只在首帧出现，空帧不能把它冲掉）；
/// - `arguments` 仍是分片拼接（OpenAI 流式的参数是逐字符下发的）。
#[derive(Debug, Default)]
pub(crate) struct ToolCallAccumulator {
    slots: BTreeMap<usize, PendingToolCall>,
}

#[derive(Debug, Default)]
struct PendingToolCall {
    id: String,
    call_type: String,
    name: String,
    arguments: String,
}

/// 终结阶段发现的不合法调用。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FinalizeError {
    /// 有参数/有 id 但没有函数名：无法执行，也不能写回历史。
    MissingFunctionName { index: usize },
    /// 参数不是合法 JSON：写回必 400，交上层重新生成这一轮。
    InvalidArguments { name: String },
}

impl ToolCallAccumulator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len()
    }

    /// 应用一条流式分片。空 `id` / `call_type` / `name` 不覆盖已累积的值。
    pub(crate) fn apply_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        call_type: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) {
        // 越界 index 直接忽略：夹到 255 会把两条不同调用合并成一条畸形调用
        if index >= MAX_PENDING_TOOL_CALLS {
            tracing::warn!(
                target: "axagent.providers.tool_call",
                index,
                "tool_call index 超出上限，忽略该分片"
            );
            return;
        }
        let slot = self.slots.entry(index).or_default();
        if let Some(id) = trimmed(id) {
            slot.id = id.to_string();
        }
        if let Some(call_type) = trimmed(call_type) {
            slot.call_type = call_type.to_string();
        }
        if let Some(name) = trimmed(name) {
            // name 可能整名重复下发，也可能被切成碎片：前缀关系按「更长者为准」，
            // 互不为前缀则视为碎片拼接（旧实现直接覆盖，碎片流会只剩最后一段）。
            if slot.name.is_empty() || name.starts_with(slot.name.as_str()) {
                slot.name = name.to_string();
            } else if !slot.name.starts_with(name) {
                slot.name.push_str(name);
            }
        }
        if let Some(arguments) = arguments {
            slot.arguments.push_str(arguments);
        }
    }

    /// 终结：产出可写回历史的调用列表，或明确指出这一轮不能写回。
    ///
    /// 全空槽位（上游心跳/占位）静默跳过；空名槽位视为上游丢帧，直接报错而不是
    /// 伪造一个「无名函数」——伪造只会把 400 推迟到下一轮。
    pub(crate) fn finalize(&mut self) -> Result<Option<Vec<ToolCall>>, FinalizeError> {
        let slots = std::mem::take(&mut self.slots);
        let mut calls = Vec::with_capacity(slots.len());
        for (index, slot) in slots {
            if slot.id.is_empty() && slot.name.is_empty() && slot.arguments.is_empty() {
                continue;
            }
            if slot.name.is_empty() {
                return Err(FinalizeError::MissingFunctionName { index });
            }
            let arguments = validate_tool_call_arguments(&slot.arguments)
                .ok_or_else(|| FinalizeError::InvalidArguments { name: slot.name.clone() })?;
            calls.push(ToolCall {
                id: if slot.id.is_empty() {
                    format!("call_{index}")
                } else {
                    slot.id
                },
                call_type: if slot.call_type.is_empty() {
                    "function".to_string()
                } else {
                    slot.call_type
                },
                function: ToolCallFunction { name: slot.name, arguments },
            });
        }
        Ok(if calls.is_empty() { None } else { Some(calls) })
    }
}

/// 单条消息在序列化前要做的处理（内部计划，不对外暴露）。
#[derive(Debug, Clone)]
enum ToolCallPlan {
    /// 原样序列化。
    Unchanged,
    /// 整条丢弃（空内容的孤儿 assistant / 无承接的 tool 消息）。
    Drop,
    /// assistant：用这份（可能为 `None`）替换原 `tool_calls`。
    Assistant(Option<Vec<ToolCall>>),
    /// tool：`tool_call_id` 需要改写为合成 id。
    ToolCallId(String),
}

/// 为整段历史生成序列化计划：修复空 id、丢弃空名调用、丢弃无承接的 tool 消息。
///
/// 只有真正需要改动时才返回非 `Unchanged` 计划，正常历史零额外分配、零拷贝。
fn plan_messages(messages: &[ChatMessage]) -> Vec<ToolCallPlan> {
    let mut plans: Vec<ToolCallPlan> = vec![ToolCallPlan::Unchanged; messages.len()];
    let mut cursor = 0;

    while cursor < messages.len() {
        // 不在任何 assistant.tool_calls 之后的 tool 消息 = 孤儿，上游必拒
        if messages[cursor].role == "tool" {
            plans[cursor] = ToolCallPlan::Drop;
            cursor += 1;
            continue;
        }

        let Some(calls) = messages[cursor].tool_calls.as_deref() else {
            cursor += 1;
            continue;
        };

        // `"tool_calls": []` 同样会被上游按空实现拒绝，直接摘掉
        if calls.is_empty() {
            let has_visible_text =
                !crate::extract_text_content(&messages[cursor].content).is_empty();
            plans[cursor] = if has_visible_text {
                ToolCallPlan::Assistant(None)
            } else {
                ToolCallPlan::Drop
            };
            cursor += 1;
            continue;
        }

        // 紧跟其后的 tool 结果块（结果顺序允许与调用顺序不同）
        let mut block_end = cursor + 1;
        while block_end < messages.len() && messages[block_end].role == "tool" {
            block_end += 1;
        }

        let mut answered: HashSet<&str> = HashSet::new();
        let mut blank_slots: VecDeque<usize> = VecDeque::new();
        for (offset, message) in messages[cursor + 1..block_end].iter().enumerate() {
            match trimmed(message.tool_call_id.as_deref()) {
                Some(id) => {
                    answered.insert(id);
                },
                None => blank_slots.push_back(cursor + 1 + offset),
            }
        }

        let mut repaired: Vec<ToolCall> = Vec::with_capacity(calls.len());
        let mut rewrites: BTreeMap<usize, String> = BTreeMap::new();
        let mut kept_ids: HashSet<String> = HashSet::new();
        let mut dropped_names = 0_usize;
        let mut dropped_unanswered = 0_usize;

        for (position, call) in calls.iter().enumerate() {
            let Some(name) = trimmed(Some(call.function.name.as_str())) else {
                dropped_names += 1;
                continue;
            };
            let mut id = call.id.trim().to_string();
            if id.is_empty() {
                // 只有存在空 id 的 tool 结果时才认领，否则这条调用没有承接
                let Some(tool_index) = blank_slots.pop_front() else {
                    dropped_unanswered += 1;
                    continue;
                };
                id = format!("call_{cursor}_{position}");
                rewrites.insert(tool_index, id.clone());
            } else if !answered.contains(id.as_str()) {
                dropped_unanswered += 1;
                continue;
            }
            let arguments = validate_tool_call_arguments(&call.function.arguments)
                .unwrap_or_else(|| "{}".to_string());
            kept_ids.insert(id.clone());
            repaired.push(ToolCall {
                id,
                call_type: if call.call_type.trim().is_empty() {
                    "function".to_string()
                } else {
                    call.call_type.trim().to_string()
                },
                function: ToolCallFunction { name: name.to_string(), arguments },
            });
        }

        for index in cursor + 1..block_end {
            if let Some(new_id) = rewrites.get(&index) {
                plans[index] = ToolCallPlan::ToolCallId(new_id.clone());
                continue;
            }
            let keep = trimmed(messages[index].tool_call_id.as_deref())
                .is_some_and(|id| kept_ids.contains(id));
            if !keep {
                plans[index] = ToolCallPlan::Drop;
            }
        }

        let unchanged = repaired.len() == calls.len()
            && repaired.iter().zip(calls.iter()).all(|(repaired, original)| {
                repaired.id == original.id
                    && repaired.call_type == original.call_type
                    && repaired.function.name == original.function.name
                    && repaired.function.arguments == original.function.arguments
            });

        if !unchanged {
            let has_visible_text =
                !crate::extract_text_content(&messages[cursor].content).is_empty();
            plans[cursor] = if repaired.is_empty() && !has_visible_text {
                // 只剩一个空 assistant 壳子也不要发：`content: null` + 无 tool_calls 同样非法
                ToolCallPlan::Drop
            } else if repaired.is_empty() {
                ToolCallPlan::Assistant(None)
            } else {
                ToolCallPlan::Assistant(Some(repaired))
            };
            tracing::warn!(
                target: "axagent.providers.tool_call",
                dropped_names,
                dropped_unanswered,
                repaired_ids = rewrites.len(),
                kept = kept_ids.len(),
                "assistant.tool_calls 历史已净化（空名/无响应/空 id）"
            );
        }

        cursor = block_end;
    }

    plans
}

/// 净化一段待发送历史；无需改动时返回 `None`（调用方零拷贝走原路径）。
pub(crate) fn sanitize_messages(messages: &[ChatMessage]) -> Option<Vec<ChatMessage>> {
    let plans = plan_messages(messages);
    if plans.iter().all(|plan| matches!(plan, ToolCallPlan::Unchanged)) {
        return None;
    }

    let mut sanitized = Vec::with_capacity(messages.len());
    for (message, plan) in messages.iter().zip(plans) {
        match plan {
            ToolCallPlan::Drop => {},
            ToolCallPlan::Unchanged => sanitized.push(message.clone()),
            ToolCallPlan::Assistant(calls) => {
                let mut message = message.clone();
                message.tool_calls = calls;
                sanitized.push(message);
            },
            ToolCallPlan::ToolCallId(id) => {
                let mut message = message.clone();
                message.tool_call_id = Some(id);
                sanitized.push(message);
            },
        }
    }
    Some(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::types::ChatContent;

    fn tool_call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: ToolCallFunction { name: name.to_string(), arguments: arguments.to_string() },
        }
    }

    fn assistant(calls: Vec<ToolCall>) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::Text(String::new()),
            tool_calls: Some(calls),
            tool_call_id: None,
            thinking: None,
        }
    }

    fn tool_result(id: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::Text("ok".to_string()),
            tool_calls: None,
            tool_call_id: Some(id.to_string()),
            thinking: None,
        }
    }

    fn user(text: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Text(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
        }
    }

    #[test]
    fn accumulator_ignores_index_gaps() {
        let mut acc = ToolCallAccumulator::new();
        acc.apply_delta(0, Some("call_a"), Some("function"), Some("read_file"), Some("{}"));
        // index 1 从未出现：旧实现会补一条空串占位
        acc.apply_delta(2, Some("call_c"), Some("function"), Some("write_file"), Some("{}"));
        let calls = acc.finalize().expect("合法槽位应通过").expect("应有调用");
        assert_eq!(calls.len(), 2, "跳号不得产出幽灵调用");
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[1].id, "call_c");
    }

    #[test]
    fn accumulator_synthesizes_missing_id() {
        let mut acc = ToolCallAccumulator::new();
        acc.apply_delta(0, None, Some("function"), Some("read_file"), Some("{\"path\":\"a\"}"));
        let calls = acc.finalize().expect("应通过").expect("应有调用");
        assert_eq!(calls[0].id, "call_0", "空 id 必须合成，不能原样回写");

        // 后续空 id 分片不得把已合成的语义冲掉（name 保持）
        assert_eq!(calls[0].function.name, "read_file");
        assert_eq!(calls[0].function.arguments, "{\"path\":\"a\"}");
    }

    #[test]
    fn accumulator_rejects_nameless_call() {
        let mut acc = ToolCallAccumulator::new();
        acc.apply_delta(0, Some("call_a"), Some("function"), None, Some("{}"));
        assert!(
            matches!(acc.finalize(), Err(FinalizeError::MissingFunctionName { index: 0 })),
            "空名调用必须报 MissingFunctionName"
        );
    }

    #[test]
    fn accumulator_skips_fully_blank_slot() {
        let mut acc = ToolCallAccumulator::new();
        acc.apply_delta(0, None, None, None, Some(""));
        assert!(acc.finalize().expect("纯空槽位应忽略").is_none());
    }

    #[test]
    fn accumulator_normalizes_empty_arguments() {
        let mut acc = ToolCallAccumulator::new();
        acc.apply_delta(0, Some("call_a"), Some("function"), Some("ping"), Some(""));
        let calls = acc.finalize().expect("应通过").expect("应有调用");
        assert_eq!(calls[0].function.arguments, "{}");
    }

    #[test]
    fn accumulator_rejects_invalid_arguments() {
        let mut acc = ToolCallAccumulator::new();
        acc.apply_delta(0, Some("call_a"), Some("function"), Some("read_file"), Some("{\"path\":"));
        match acc.finalize() {
            Err(FinalizeError::InvalidArguments { name }) => assert_eq!(name, "read_file"),
            other => panic!("应报 InvalidArguments，实得 {other:?}"),
        }
    }

    #[test]
    fn accumulator_joins_fragmented_names() {
        let mut acc = ToolCallAccumulator::new();
        acc.apply_delta(0, Some("call_a"), Some("function"), Some("read"), Some("{\"pa"));
        acc.apply_delta(0, None, None, Some("_file"), Some("th\":\"a\"}"));
        let calls = acc.finalize().expect("应通过").expect("应有调用");
        assert_eq!(calls[0].function.name, "read_file", "碎片名必须拼接而非覆盖");
        assert_eq!(calls[0].function.arguments, "{\"path\":\"a\"}");
    }

    #[test]
    fn accumulator_ignores_repeated_full_name_and_out_of_range_index() {
        let mut acc = ToolCallAccumulator::new();
        acc.apply_delta(0, Some("call_a"), Some("function"), Some("read_file"), Some("{}"));
        acc.apply_delta(0, None, None, Some("read_file"), Some(""));
        // 越界 index 不得夹到 255 去和别的调用合并
        acc.apply_delta(999, Some("call_x"), Some("function"), Some("write_file"), Some("{}"));
        let calls = acc.finalize().expect("应通过").expect("应有调用");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "read_file");
    }

    #[test]
    fn plan_is_noop_for_valid_history() {
        let messages = vec![
            user("hi"),
            assistant(vec![tool_call("call_a", "read_file", "{}")]),
            tool_result("call_a"),
        ];
        let plans = plan_messages(&messages);
        assert!(plans.iter().all(|plan| matches!(plan, ToolCallPlan::Unchanged)), "{plans:?}");
    }

    #[test]
    fn plan_drops_nameless_call_with_its_result() {
        let messages = vec![
            user("hi"),
            assistant(vec![tool_call("call_a", "read_file", "{}"), tool_call("call_b", "", "{}")]),
            tool_result("call_a"),
            tool_result("call_b"),
        ];
        let plans = plan_messages(&messages);

        match &plans[1] {
            ToolCallPlan::Assistant(Some(calls)) => {
                assert_eq!(calls.len(), 1, "空名调用必须被丢弃");
                assert_eq!(calls[0].id, "call_a");
            },
            other => panic!("assistant 应被改写，实得 {other:?}"),
        }
        assert!(matches!(plans[2], ToolCallPlan::Unchanged));
        assert!(matches!(plans[3], ToolCallPlan::Drop), "被丢调用的结果不得留下");
    }

    #[test]
    fn plan_repairs_blank_ids_in_order() {
        let messages = vec![
            assistant(vec![tool_call("", "read_file", "{}"), tool_call("", "write_file", "{}")]),
            tool_result(""),
            tool_result(""),
        ];
        let plans = plan_messages(&messages);

        let ids = match &plans[0] {
            ToolCallPlan::Assistant(Some(calls)) => {
                assert_eq!(calls.len(), 2);
                calls.iter().map(|c| c.id.clone()).collect::<Vec<_>>()
            },
            other => panic!("assistant 应被改写，实得 {other:?}"),
        };
        assert_eq!(ids, vec!["call_0_0".to_string(), "call_0_1".to_string()]);
        assert!(matches!(&plans[1], ToolCallPlan::ToolCallId(id) if id == "call_0_0"));
        assert!(matches!(&plans[2], ToolCallPlan::ToolCallId(id) if id == "call_0_1"));
    }

    #[test]
    fn plan_drops_unanswered_call() {
        // 会话在工具执行前中断：assistant 带调用但没有任何 tool 结果
        let messages =
            vec![user("分析这只股票"), assistant(vec![tool_call("call_a", "get_quote", "{}")])];
        let plans = plan_messages(&messages);
        assert!(
            matches!(&plans[1], ToolCallPlan::Drop),
            "空内容 + 无响应的 assistant 必须整条丢弃，实得 {:?}",
            plans[1]
        );
    }

    #[test]
    fn plan_keeps_text_when_all_calls_dropped() {
        let mut message = assistant(vec![tool_call("call_a", "get_quote", "{}")]);
        message.content = ChatContent::Text("先看行情".to_string());
        let messages = vec![message];
        let plans = plan_messages(&messages);
        assert!(
            matches!(&plans[0], ToolCallPlan::Assistant(None)),
            "有正文时保留消息、只去掉 tool_calls，实得 {:?}",
            plans[0]
        );
    }

    #[test]
    fn plan_drops_orphan_tool_message() {
        let messages = vec![user("hi"), tool_result("call_ghost")];
        let plans = plan_messages(&messages);
        assert!(matches!(plans[1], ToolCallPlan::Drop), "无承接的 tool 消息必须丢");
    }

    #[test]
    fn sanitize_is_noop_for_valid_history() {
        let messages = vec![
            user("hi"),
            assistant(vec![tool_call("call_a", "read_file", "{}")]),
            tool_result("call_a"),
        ];
        assert!(sanitize_messages(&messages).is_none(), "合法历史不得复制/改写");
    }

    #[test]
    fn sanitize_repairs_ids_and_pairs_results_by_order() {
        let messages = vec![
            assistant(vec![tool_call("", "read_file", "{}"), tool_call("call_b", "", "{}")]),
            tool_result(""),
            tool_result("call_b"),
        ];
        let fixed = sanitize_messages(&messages).expect("需要净化");
        assert_eq!(fixed.len(), 2, "被丢调用（空名）及其结果都要剔除");

        let calls = fixed[0].tool_calls.as_ref().expect("应保留调用");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_0_0");
        assert_eq!(calls[0].function.name, "read_file");
        assert_eq!(fixed[1].tool_call_id.as_deref(), Some("call_0_0"), "结果必须回写合成 id");
    }

    #[test]
    fn normalize_arguments_object_keeps_only_objects() {
        assert_eq!(normalize_arguments_object("{\"a\":1}"), serde_json::json!({ "a": 1 }));
        assert_eq!(normalize_arguments_object("[1,2]"), serde_json::json!({}));
        assert_eq!(normalize_arguments_object("\"text\""), serde_json::json!({}));
        assert_eq!(normalize_arguments_object("not json"), serde_json::json!({}));
    }

    #[test]
    fn finalize_tool_calls_repairs_id_and_rejects_bad_fields() {
        let finalized = finalize_tool_calls(vec![tool_call("  ", " read_file ", "{}")])
            .expect("应通过")
            .expect("应有调用");
        assert_eq!(finalized[0].id, "call_0");
        assert_eq!(finalized[0].function.name, "read_file");

        assert!(matches!(
            finalize_tool_calls(vec![tool_call("call_a", " ", "{}")]),
            Err(FinalizeError::MissingFunctionName { index: 0 })
        ));

        match finalize_tool_calls(vec![tool_call("call_a", "read_file", "{")]) {
            Err(FinalizeError::InvalidArguments { name }) => assert_eq!(name, "read_file"),
            other => panic!("应报 InvalidArguments，实得 {other:?}"),
        }
    }
}
