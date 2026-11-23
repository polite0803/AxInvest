// SPDX-License-Identifier: AGPL-3.0-only

//! `delegate_to_bot` — requester 轮内同步征询另一个 Bot（P2-b ADR-P2B-4/5）。
//!
//! 与 dispatch 不共用执行路径：本工具同步等待结果，Assignment 直接落终态审计行、
//! 不落回执线程。真实执行由 wiring 层注入的 [`BotDelegateRunner`] 完成
//! （照抄 `multi_agent.rs` 的 OnceLock runner 模式）。
//!
//! 注册条件：只出现在 person / 例程发起的轮次；assignee 轮的工具表在
//! `bot_routine_runner` 中显式禁用本工具，保证委派深度恒为 1。

use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

/// 单轮内 consult 上限（IPC-P2B §3）：超出返回 LLM 可读错误文本，不落错误码。
pub const MAX_CONSULT_PER_TURN: usize = 2;

static BOT_DELEGATE_RUNNER: OnceLock<Arc<dyn BotDelegateRunner>> = OnceLock::new();
static TURN_COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

/// 征询入参：`origin` 取 `bot` / `routine`（Q6 例程可发起，仍是一层）。
#[derive(Debug, Clone)]
pub struct BotDelegateInput {
    pub bot_id: String,
    pub question: String,
    pub requester_bot_id: Option<String>,
    pub origin: String,
}

#[derive(Debug, Clone)]
pub struct BotDelegateOutcome {
    pub assignment_id: String,
    pub result_text: String,
    pub status: String,
}

#[async_trait]
pub trait BotDelegateRunner: Send + Sync {
    async fn consult(&self, input: BotDelegateInput) -> Result<BotDelegateOutcome, String>;
}

/// 注入真实执行器（进程启动时一次，见 `bot_assignment_runner::install_consult_runner`）。
pub fn set_bot_delegate_runner(runner: Arc<dyn BotDelegateRunner>) {
    let _ = BOT_DELEGATE_RUNNER.set(runner);
}

fn turn_key(ctx: &ToolContext) -> String {
    format!(
        "{}|{}",
        ctx.conversation_id.as_deref().unwrap_or(""),
        ctx.message_id.as_deref().unwrap_or("")
    )
}

pub struct BotDelegateTool;

#[async_trait]
impl Tool for BotDelegateTool {
    fn name(&self) -> &str {
        "delegate_to_bot"
    }

    fn description(&self) -> &str {
        "同步征询另一个 Bot 的知识/结论（只读语义，不落回执线程）；\
         征询结果会作为工具结果出现在当前轮，Assignment 仅作审计。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "botId": {
                    "type": "string",
                    "description": "被征询的 Bot id"
                },
                "question": {
                    "type": "string",
                    "description": "征询问题（只读，不要求对方执行写操作）"
                },
                "requesterBotId": {
                    "type": "string",
                    "description": "发起方 Bot id；省略时按 person 处理"
                },
                "origin": {
                    "type": "string",
                    "enum": ["bot", "routine"],
                    "description": "发起来源，默认 bot；例程轮传 routine"
                }
            },
            "required": ["botId", "question"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Agent
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let bot_id = input
            .get("botId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: botId"))?
            .to_string();
        let question = input
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ToolError::invalid_input("缺少必需参数: question"))?
            .to_string();

        let counts = TURN_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
        let key = turn_key(ctx);
        {
            let mut guard = counts.lock().await;
            // 长驻进程防无界增长：只保留近期轮次；当前轮刚插入不受影响。
            if guard.len() > 256 {
                guard.clear();
            }
            let used = guard.entry(key.clone()).or_insert(0);
            if *used >= MAX_CONSULT_PER_TURN {
                return Ok(ToolResult::error(format!(
                    "本轮 consult 调用已达上限 {MAX_CONSULT_PER_TURN} 次；请直接综合作答"
                )));
            }
            *used += 1;
        }

        let runner = BOT_DELEGATE_RUNNER.get().ok_or_else(|| {
            ToolError::execution_failed("BotDelegateRunner 未初始化（启动装配遗漏）")
        })?;
        let requester_bot_id = input
            .get("requesterBotId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| ctx.extra.get("botId").cloned());
        let origin = input
            .get("origin")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| ctx.extra.get("botDelegationOrigin").cloned())
            .unwrap_or_else(|| "bot".to_string());

        match runner
            .consult(BotDelegateInput { bot_id, question, requester_bot_id, origin })
            .await
        {
            Ok(outcome) => Ok(ToolResult::success(format!(
                "## Bot 征询结果\n\n{}\n\n（Assignment: {} · status: {}）",
                outcome.result_text, outcome.assignment_id, outcome.status
            ))),
            Err(e) => Ok(ToolResult::error(format!("Bot 征询失败：{e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRunner;

    #[async_trait]
    impl BotDelegateRunner for FakeRunner {
        async fn consult(&self, input: BotDelegateInput) -> Result<BotDelegateOutcome, String> {
            if input.question.contains("失败") {
                return Err("query rejected".to_string());
            }
            if input.origin == "routine" && input.bot_id == "bot-cron" {
                return Ok(BotDelegateOutcome {
                    assignment_id: "a-cron".into(),
                    result_text: "例程征询成功".into(),
                    status: "completed".into(),
                });
            }
            Ok(BotDelegateOutcome {
                assignment_id: format!("a-{}", input.bot_id),
                result_text: format!("来自 {} 的结论", input.bot_id),
                status: "completed".into(),
            })
        }
    }

    fn init_runner() {
        let _ = set_bot_delegate_runner(Arc::new(FakeRunner));
    }

    fn ctx(turn: &str) -> ToolContext {
        let mut c = ToolContext::new(".");
        c.conversation_id = Some("conv-1".to_string());
        c.message_id = Some(format!("msg-{turn}"));
        c
    }

    #[tokio::test]
    async fn consult_returns_runner_text_and_assignment() {
        init_runner();
        let out = BotDelegateTool
            .call(
                serde_json::json!({ "botId": "bot-a", "question": "盘面如何？" }),
                &ctx("success"),
            )
            .await
            .expect("工具调用");
        assert!(!out.is_error, "{:?}", out.content);
        assert!(out.content.contains("来自 bot-a 的结论"));
        assert!(out.content.contains("a-bot-a"));
    }

    #[tokio::test]
    async fn per_turn_gate_allows_two_then_readable_error() {
        init_runner();
        let tool = BotDelegateTool;
        let same_turn = ctx("gate");
        let input = serde_json::json!({ "botId": "bot-b", "question": "怎么看？" });
        let first = tool.call(input.clone(), &same_turn).await.expect("第 1 次");
        let second = tool.call(input.clone(), &same_turn).await.expect("第 2 次");
        let third = tool.call(input, &same_turn).await.expect("第 3 次");
        assert!(!first.is_error && !second.is_error);
        assert!(third.is_error, "第 3 次应被单轮门拦下");
        assert!(third.content.contains("上限 2 次"), "{}", third.content);

        // 新的一轮（message_id 不同）重新计数。
        let next = tool
            .call(
                serde_json::json!({ "botId": "bot-c", "question": "新轮征询" }),
                &ctx("gate-next"),
            )
            .await
            .expect("新轮");
        assert!(!next.is_error);
    }

    #[tokio::test]
    async fn runner_error_is_llm_readable_not_panic() {
        init_runner();
        let out = BotDelegateTool
            .call(
                serde_json::json!({ "botId": "bot-d", "question": "失败场景" }),
                &ctx("err"),
            )
            .await
            .expect("工具调用");
        assert!(out.is_error);
        assert!(out.content.contains("query rejected"));
    }

    #[test]
    fn tool_name_and_schema_are_frozen() {
        let tool = BotDelegateTool;
        assert_eq!(tool.name(), "delegate_to_bot");
        let schema = tool.input_schema();
        assert!(schema["properties"]["botId"].is_object());
        assert!(schema["properties"]["question"].is_object());
    }
}
