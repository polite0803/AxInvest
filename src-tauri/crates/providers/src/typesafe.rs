// SPDX-License-Identifier: AGPL-3.0-only

//! TypeSafe **Jev** 决策模型适配器。
//!
//! Jev 不是 chat 模型：它不生成文本，而是接收一段 `state` 与一组**类型化问题**
//! （`choice` / `score` / `noul`），在一次前向中并行返回带概率的结构化判定
//! （System One 模型，TypeSafe AI 于 2026-09-15 发布）。
//!
//! - 端点：`POST {base}/alpha/decisions`
//! - 默认 base：`https://openrouter.ai/api`（OpenRouter 已开放，无需 waitlist）
//! - 默认模型 ID：`typesafe/jev-1.13`
//! - 计费：输入 `$0.042 / 1M tokens`，**输出免费**；端到端 70–500ms
//!
//! # 为什么这里要做「协议转换」
//!
//! `ProviderAdapter::chat` 的契约是 OpenAI 风格的 `ChatRequest → ChatResponse`，
//! 而 Jev 只有 decisions 接口。工作流侧的两个消费方 —— `llmClassifier` 与
//! `condition` 的 `judge_by_llm` —— **已经**把判断问题写在 prompt 里，并通过
//! prompt 内的 JSON 契约声明了期望的返回形态。因此本适配器按以下规则转换：
//!
//! 1. 取全部消息的文本拼接作为 Jev 的 `state`；
//! 2. 判定问题类型（判据自上而下，先命中者生效）：
//!    - state 含 `"decision"` 字段名（条件节点的 JSON 契约）→ `noul` 是非判断；
//!    - state 含 `"label"` 字段名（分类节点的 JSON 契约）→ `choice` 多选一；
//!    - state 含 ≥2 项编号列表（`1. xxx`，分类器的类别清单）→ `choice`；
//!    - 其余 → `noul` 兜底。
//! 3. 按 state 是否声明了 `confidence` 契约决定回填形态：
//!    - 声明了 → 回 JSON：choice 为 `{"label","confidence","probabilities"}`，
//!      noul 为 `{"decision","confidence"}`；
//!    - 未声明 → 回裸值：choice 为类别名本身，noul 为 `true` / `false`。
//!
//! 前两条判据依赖的是**执行器自己拼接的英文 JSON 字段名**（`confidence` /
//! `decision` / `label`），不随 i18n 文案变化，因此是稳定的机器可读信号；
//! 编号列表仅作结构性兜底。
//!
//! 这样 `ConditionExecutor`（[`condition_executor.rs`]）与
//! `LlmClassifierExecutor`（[`llm_classifier_executor.rs`]）**零改动**即可
//! 把节点上的 `routing_model` / `model` 指向 Jev。
//!
//! # 能力边界（务必遵守）
//!
//! - **不做文本生成**：不要把 `LLMNode` 这类生成节点指向 Jev。
//! - **不做算术 / 计数 / 日期比较**：这类判断仍应交给条件节点的静态规则。
//! - **无视觉输入**：只接受文本 `state`。
//! - **无流式**：`chat_stream` 以「单分片」形式降级返回。
//!
//! [`condition_executor.rs`]: ../../../rt-workflow/src/work_engine/executors/condition_executor.rs
//! [`llm_classifier_executor.rs`]: ../../../rt-workflow/src/work_engine/executors/llm_classifier_executor.rs

use std::sync::Arc;

use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::*;
use futures::Stream;
use serde::Deserialize;
use std::pin::Pin;

use crate::compat::impl_default_via_new;
use crate::{ProviderAdapter, ProviderRequestContext};

/// TypeSafe / OpenRouter decisions 端点默认 base。
const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api";

/// decisions 端点相对路径。
const DECISIONS_PATH: &str = "/alpha/decisions";

/// 默认模型 ID（OpenRouter 上的 Jev 当前版本）。
const DEFAULT_MODEL: &str = "typesafe/jev-1.13";

/// Jev 单次请求的上下文窗口（state 占其中约一半）。
const CONTEXT_WINDOW: u32 = 32_000;

/// choice 问题最多支持的选项数（Jev 原生上限 255）。
const MAX_CHOICE_OPTIONS: usize = 255;

/// `choice` 问题的固定指令。写给模型看，故用英文，不随 UI 语言变化。
const CHOICE_INSTRUCTIONS: &str =
    "Based on the state, select the single most appropriate option from the criteria.";

/// `noul` 问题的固定指令。条件节点已把判断规则写在 state 内，这里只作引导。
const NOUL_INSTRUCTIONS: &str =
    "Does the condition described in the state hold? Answer the probability that it holds.";

/// 标识「调用方声明了 JSON 契约」的英文 token。执行器在要求结构化输出时
/// 必然把该字段名拼进 prompt，与 UI 语言无关。
const JSON_CONTRACT_TOKEN: &str = "confidence";

/// JSON 契约中 `noul` 的字段名。
const DECISION_FIELD_TOKEN: &str = "\"decision\"";

/// JSON 契约中 `choice` 的字段名。
const LABEL_FIELD_TOKEN: &str = "\"label\"";

pub struct TypeSafeAdapter {
    client: reqwest::Client,
}

impl_default_via_new!(TypeSafeAdapter);

impl TypeSafeAdapter {
    pub fn new() -> Self {
        Self {
            client: crate::build_default_http_client().unwrap_or_else(|e| {
                tracing::warn!("无法构建 TypeSafe HTTP 客户端: {e}，降级为默认客户端");
                reqwest::Client::new()
            }),
        }
    }

    /// 解析有效的 base URL（未配置时回落到 OpenRouter）。
    fn base_url(ctx: &ProviderRequestContext) -> String {
        ctx.base_url
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
    }

    /// 构建带代理支持的 HTTP 客户端。
    #[allow(clippy::result_large_err)]
    fn get_client(&self, ctx: &ProviderRequestContext) -> Result<reqwest::Client> {
        match &ctx.proxy_config {
            Some(c) if c.proxy_type.as_deref() != Some("none") => crate::build_http_client(Some(c)),
            _ => Ok(self.client.clone()),
        }
    }

    /// Jev 内置模型列表（不调用 API）。
    fn builtin_models(provider_id: &str) -> Vec<Model> {
        vec![Model {
            provider_id: provider_id.to_string(),
            model_id: DEFAULT_MODEL.to_string(),
            name: "Jev 1.13 (TypeSafe)".to_string(),
            group_name: Some("TypeSafe".to_string()),
            // Jev 是决策模型，只能被 llmClassifier / condition 节点选中；
            // LLMNode / Agent 侧会按 Decision 类型拒绝（见 `ModelSelect` 的
            // 类型约束与 `llm_resolve.rs` 的节点类型硬校验）。
            model_type: ModelType::Decision,
            // 决策模型不具备 chat 能力标签（前端 getEditableCapabilities 对非 Chat
            // 类型同样返回空集）。
            capabilities: vec![],
            max_tokens: Some(CONTEXT_WINDOW),
            // 输出只有类型化判定，token 量极小且免费。
            max_output_tokens: Some(1_024),
            enabled: true,
            param_overrides: None,
            input_price_per_mtok: Some(0.042),
            output_price_per_mtok: Some(0.0),
        }]
    }

    /// 取全部消息的文本拼接作为 Jev 的 `state`。
    fn extract_state(request: &ChatRequest) -> String {
        let joined = request
            .messages
            .iter()
            .map(Self::message_text)
            .filter(|t| !t.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        joined.trim().to_string()
    }

    fn message_text(msg: &ChatMessage) -> String {
        match &msg.content {
            ChatContent::Text(t) => t.clone(),
            ChatContent::Multipart(parts) => {
                parts.iter().filter_map(|p| p.text.clone()).collect::<Vec<_>>().join("\n")
            },
        }
    }

    /// 从文本中提取编号列表项（`1. xxx` / `2) xxx` / `3、xxx`）。
    ///
    /// 只识别「纯数字 + 分隔符」开头的行，返回其后的标签文本。
    fn extract_choice_options(text: &str) -> Vec<String> {
        text.lines().filter_map(Self::strip_numbered_prefix).take(MAX_CHOICE_OPTIONS).collect()
    }

    fn strip_numbered_prefix(line: &str) -> Option<String> {
        let t = line.trim();
        // 前导数字必须是 ASCII，故其字符数即字节数。
        let digits_len = t.chars().take_while(char::is_ascii_digit).count();
        if digits_len == 0 || digits_len > 3 {
            return None;
        }
        let rest = &t[digits_len..];
        let mut chars = rest.chars();
        let sep = chars.next()?;
        if !matches!(sep, '.' | ')' | '、' | '．') {
            return None;
        }
        let label = chars.as_str().trim();
        if label.is_empty() {
            None
        } else {
            Some(label.to_string())
        }
    }

    /// 判定本次调用应使用的问题类型。
    fn resolve_question(state: &str) -> QuestionKind {
        if state.contains(DECISION_FIELD_TOKEN) {
            return QuestionKind::Noul;
        }
        if state.contains(LABEL_FIELD_TOKEN) {
            return QuestionKind::Choice;
        }
        if Self::extract_choice_options(state).len() >= 2 {
            return QuestionKind::Choice;
        }
        QuestionKind::Noul
    }

    /// 构造 decisions 请求体。
    fn build_request_body(model: &str, state: &str, kind: QuestionKind) -> serde_json::Value {
        let question = match kind {
            QuestionKind::Choice => {
                let options = Self::extract_choice_options(state);
                let criteria: serde_json::Map<String, serde_json::Value> = options
                    .into_iter()
                    .map(|o| {
                        let v = serde_json::Value::String(o.clone());
                        (o, v)
                    })
                    .collect();
                serde_json::json!({
                    "type": "choice",
                    "instructions": CHOICE_INSTRUCTIONS,
                    "criteria": criteria,
                })
            },
            QuestionKind::Noul => serde_json::json!({
                "type": "noul",
                "instructions": NOUL_INSTRUCTIONS,
                "criteria": {
                    "true": "The state indicates the condition holds.",
                    "false": "The state does not indicate the condition holds.",
                },
            }),
        };

        let model = if model.trim().is_empty() {
            DEFAULT_MODEL
        } else {
            model.trim()
        };

        serde_json::json!({
            "model": model,
            "state": state,
            "questions": { "answer": question },
        })
    }

    /// 把 Jev 的类型化判定渲染成 `ChatResponse.content`。
    ///
    /// `wants_json` 为真时回填执行器可解析的 JSON，否则回填裸值
    /// （解析逻辑见 `LlmClassifierExecutor` 与 `ConditionExecutor`）。
    fn render_content(
        answer: &DecisionAnswer,
        kind: QuestionKind,
        wants_json: bool,
    ) -> Result<String> {
        match kind {
            QuestionKind::Choice => {
                let choice =
                    answer.choice.clone().filter(|c| !c.trim().is_empty()).ok_or_else(|| {
                        AxAgentError::Provider("Jev 响应缺少 choice 字段".to_string())
                    })?;
                if !wants_json {
                    return Ok(choice);
                }
                let confidence = answer.confidence.unwrap_or(1.0);
                let mut obj = serde_json::Map::new();
                obj.insert("label".to_string(), serde_json::json!(choice));
                obj.insert("confidence".to_string(), serde_json::json!(confidence));
                if let Some(probs) = &answer.probabilities {
                    obj.insert("probabilities".to_string(), probs.clone());
                }
                Ok(serde_json::Value::Object(obj).to_string())
            },
            QuestionKind::Noul => {
                // noul 无独立 confidence 字段 —— 概率本身就是置信度。
                let p = answer
                    .noul
                    .ok_or_else(|| AxAgentError::Provider("Jev 响应缺少 noul 字段".to_string()))?;
                let decision = p >= 0.5;
                if !wants_json {
                    return Ok(if decision { "true" } else { "false" }.to_string());
                }
                Ok(serde_json::json!({
                    "decision": decision,
                    "confidence": p,
                })
                .to_string())
            },
        }
    }

    /// 核心调用：`ChatRequest` → decisions → `ChatResponse`。
    ///
    /// 抽成不依赖 `self` 的关联函数，以便 `chat_stream` 能把客户端与上下文
    /// move 进异步块。
    async fn call_decisions(
        client: reqwest::Client,
        ctx: ProviderRequestContext,
        request: ChatRequest,
    ) -> Result<ChatResponse> {
        let state = Self::extract_state(&request);
        if state.is_empty() {
            return Err(AxAgentError::Provider(
                "Jev 需要非空 state：请求中没有任何文本消息".to_string(),
            ));
        }

        let kind = Self::resolve_question(&state);
        let wants_json = state.contains(JSON_CONTRACT_TOKEN);
        let body = Self::build_request_body(&request.model, &state, kind);
        let url = format!("{}{}", Self::base_url(&ctx), DECISIONS_PATH);

        let resp = crate::apply_request_headers(
            client
                .post(&url)
                .header("Authorization", format!("Bearer {}", ctx.api_key))
                .json(&body),
            &ctx,
        )
        .send()
        .await
        .map_err(|e| AxAgentError::Provider(format!("Jev 请求失败: {e}")))?;

        let status = resp.status();
        let raw = resp
            .text()
            .await
            .map_err(|e| AxAgentError::Provider(format!("Jev 响应读取失败: {e}")))?;

        if !status.is_success() {
            return Err(AxAgentError::Provider(format!(
                "Jev 返回 HTTP {}: {}",
                status.as_u16(),
                raw.chars().take(500).collect::<String>()
            )));
        }

        let parsed: DecisionsResponse = serde_json::from_str(&raw).map_err(|e| {
            AxAgentError::Provider(format!(
                "Jev 响应解析失败: {e}, raw: {}",
                raw.chars().take(500).collect::<String>()
            ))
        })?;

        let answer = parsed
            .answers
            .get("answer")
            .ok_or_else(|| AxAgentError::Provider("Jev 响应缺少 answers.answer".to_string()))?;

        let content = Self::render_content(answer, kind, wants_json)?;
        let usage = parsed
            .usage
            .map(|u| TokenUsage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                cache_miss_input_tokens: None,
            })
            .unwrap_or_default();

        Ok(ChatResponse {
            id: format!("jev-{}", uuid::Uuid::new_v4()),
            model: parsed.model.unwrap_or(request.model),
            content,
            thinking: None,
            usage,
            tool_calls: None,
        })
    }
}

/// Jev 的三种问题类型。当前只用到 `Choice` 与 `Noul`；
/// `Score` 的量表语义暂无对应的工作流消费方，故不暴露。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuestionKind {
    Choice,
    Noul,
}

// ── decisions 接口的请求 / 响应 DTO（仅本模块使用）──

#[derive(Debug, Deserialize)]
struct DecisionsResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    answers: std::collections::HashMap<String, DecisionAnswer>,
    #[serde(default)]
    usage: Option<DecisionsUsage>,
}

#[derive(Debug, Deserialize)]
struct DecisionAnswer {
    #[serde(default, rename = "type")]
    #[allow(dead_code)]
    kind: Option<String>,
    #[serde(default)]
    choice: Option<String>,
    #[serde(default)]
    noul: Option<f64>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    probabilities: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct DecisionsUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[async_trait]
impl ProviderAdapter for TypeSafeAdapter {
    async fn chat(
        &self,
        ctx: &ProviderRequestContext,
        request: Arc<ChatRequest>,
    ) -> Result<ChatResponse> {
        let client = self.get_client(ctx)?;
        Self::call_decisions(client, ctx.clone(), (*request).clone()).await
    }

    /// Jev 无流式接口，降级为「单分片」返回：一次性调用后把结果作为唯一的
    /// 终态分片吐出，保持 `execute_llm_stream` 的调用方无需分支。
    fn chat_stream(
        &self,
        ctx: &ProviderRequestContext,
        request: ChatRequest,
        _cancel_token: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> Pin<Box<dyn Stream<Item = Result<ChatStreamChunk>> + Send>> {
        let client = self.get_client(ctx);
        let ctx = ctx.clone();

        let fut = async move {
            let client = client?;
            let resp = Self::call_decisions(client, ctx, request).await?;
            Ok(ChatStreamChunk {
                content: Some(resp.content),
                thinking: None,
                done: true,
                is_final: Some(true),
                usage: Some(resp.usage),
                tool_calls: None,
            })
        };

        Box::pin(futures::stream::once(fut))
    }

    async fn list_models(&self, ctx: &ProviderRequestContext) -> Result<Vec<Model>> {
        Ok(Self::builtin_models(&ctx.provider_id))
    }

    /// Jev 无 `/models` 端点，改用一次最小 decisions 调用探测鉴权。
    async fn validate_key(&self, ctx: &ProviderRequestContext) -> Result<bool> {
        let body = serde_json::json!({
            "model": DEFAULT_MODEL,
            "state": "connectivity check",
            "questions": {
                "ok": {
                    "type": "noul",
                    "instructions": "Is this a connectivity check?",
                },
            },
        });
        let url = format!("{}{}", Self::base_url(ctx), DECISIONS_PATH);
        let resp = crate::apply_request_headers(
            self.get_client(ctx)?
                .post(&url)
                .header("Authorization", format!("Bearer {}", ctx.api_key))
                .json(&body),
            ctx,
        )
        .send()
        .await
        .map_err(|e| AxAgentError::Provider(format!("Request failed: {e}")))?;
        let status = resp.status().as_u16();
        Ok(status != 401 && status != 403)
    }

    /// Jev 不提供向量嵌入。
    async fn embed(
        &self,
        _ctx: &ProviderRequestContext,
        _request: EmbedRequest,
    ) -> Result<EmbedResponse> {
        Err(AxAgentError::Provider("TypeSafe Jev 是决策模型，不支持 embedding".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_request(prompt: &str) -> ChatRequest {
        ChatRequest {
            model: DEFAULT_MODEL.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Text(prompt.to_string()),
                tool_calls: None,
                tool_call_id: None,
                thinking: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn strip_numbered_prefix_accepts_common_separators() {
        assert_eq!(
            TypeSafeAdapter::strip_numbered_prefix("1. 订单状态"),
            Some("订单状态".to_string())
        );
        assert_eq!(
            TypeSafeAdapter::strip_numbered_prefix("  12) billing  "),
            Some("billing".to_string())
        );
        assert_eq!(
            TypeSafeAdapter::strip_numbered_prefix("3、技术故障"),
            Some("技术故障".to_string())
        );
        // 非编号行不识别
        assert_eq!(TypeSafeAdapter::strip_numbered_prefix("名称: 值为 1. 5"), None);
        assert_eq!(TypeSafeAdapter::strip_numbered_prefix("1."), None);
        assert_eq!(TypeSafeAdapter::strip_numbered_prefix("1234. too many digits"), None);
    }

    #[test]
    fn resolve_question_prefers_json_contract_tokens() {
        // 条件节点带阈值时的契约 → noul
        let cond = "你是一个条件判断器\n只输出 {\"decision\": true, \"confidence\": 0.95}";
        assert_eq!(TypeSafeAdapter::resolve_question(cond), QuestionKind::Noul);

        // 分类节点带阈值时的契约 → choice
        let cls =
            "可选类别\n1. billing\n2. technical\n输出 {\"label\": \"类别名\", \"confidence\": 0.9}";
        assert_eq!(TypeSafeAdapter::resolve_question(cls), QuestionKind::Choice);
    }

    #[test]
    fn resolve_question_falls_back_to_numbered_list_then_noul() {
        // 无 JSON 契约但存在类别清单 → choice
        let cls = "请归类\n1. billing\n2. technical";
        assert_eq!(TypeSafeAdapter::resolve_question(cls), QuestionKind::Choice);

        // 既无契约也无清单 → noul 兜底
        assert_eq!(TypeSafeAdapter::resolve_question("只回答 true 或 false"), QuestionKind::Noul);
    }

    #[test]
    fn build_request_body_uses_choice_criteria_from_numbered_list() {
        let state = "可选类别\n1. billing\n2. technical";
        let body =
            TypeSafeAdapter::build_request_body("typesafe/jev-1.13", state, QuestionKind::Choice);
        let q = &body["questions"]["answer"];
        assert_eq!(q["type"], "choice");
        assert_eq!(q["criteria"]["billing"], "billing");
        assert_eq!(q["criteria"]["technical"], "technical");
        assert_eq!(body["state"], state);
    }

    #[test]
    fn build_request_body_uses_default_model_when_empty() {
        let body = TypeSafeAdapter::build_request_body("", "x", QuestionKind::Noul);
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["questions"]["answer"]["type"], "noul");
    }

    #[test]
    fn render_content_choice_json_and_bare() {
        let answer = DecisionAnswer {
            kind: Some("choice".to_string()),
            choice: Some("billing".to_string()),
            noul: None,
            confidence: Some(0.87),
            probabilities: Some(serde_json::json!({ "billing": 0.87, "technical": 0.13 })),
        };

        let json = TypeSafeAdapter::render_content(&answer, QuestionKind::Choice, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["label"], "billing");
        assert_eq!(v["confidence"], 0.87);
        assert_eq!(v["probabilities"]["technical"], 0.13);

        // 未声明契约时回裸类别名 —— 与 LlmClassifierExecutor 的无阈值分支对齐
        let bare = TypeSafeAdapter::render_content(&answer, QuestionKind::Choice, false).unwrap();
        assert_eq!(bare, "billing");
    }

    #[test]
    fn render_content_noul_thresholds_at_half() {
        let yes = DecisionAnswer {
            kind: Some("noul".to_string()),
            choice: None,
            noul: Some(0.97),
            confidence: None,
            probabilities: None,
        };
        let json = TypeSafeAdapter::render_content(&yes, QuestionKind::Noul, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["decision"], true);
        // noul 的概率本身即置信度
        assert_eq!(v["confidence"], 0.97);

        assert_eq!(
            TypeSafeAdapter::render_content(&yes, QuestionKind::Noul, false).unwrap(),
            "true"
        );

        let no = DecisionAnswer {
            kind: Some("noul".to_string()),
            choice: None,
            noul: Some(0.30),
            confidence: None,
            probabilities: None,
        };
        assert_eq!(
            TypeSafeAdapter::render_content(&no, QuestionKind::Noul, false).unwrap(),
            "false"
        );
    }

    #[test]
    fn extract_state_joins_multipart_text_parts() {
        let request = ChatRequest {
            model: DEFAULT_MODEL.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Multipart(vec![
                    ContentPart {
                        r#type: "text".to_string(),
                        text: Some("第一段".to_string()),
                        image_url: None,
                    },
                    ContentPart {
                        r#type: "image_url".to_string(),
                        text: None,
                        image_url: Some(ImageUrl { url: "http://x".to_string() }),
                    },
                    ContentPart {
                        r#type: "text".to_string(),
                        text: Some("第二段".to_string()),
                        image_url: None,
                    },
                ]),
                tool_calls: None,
                tool_call_id: None,
                thinking: None,
            }],
            ..Default::default()
        };
        assert_eq!(TypeSafeAdapter::extract_state(&request), "第一段\n第二段");
    }

    #[test]
    fn builtin_models_expose_jev_with_pricing() {
        let models = TypeSafeAdapter::builtin_models("prov-1");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, DEFAULT_MODEL);
        assert_eq!(models[0].input_price_per_mtok, Some(0.042));
        assert_eq!(models[0].output_price_per_mtok, Some(0.0));
        assert_eq!(models[0].max_tokens, Some(CONTEXT_WINDOW));
    }

    #[test]
    fn jev_is_typed_as_decision_model() {
        // 决策类型是「不能被配到 LLMNode / Agent」的判据来源，必须锁住
        let models = TypeSafeAdapter::builtin_models("prov-1");
        assert_eq!(models[0].model_type, ModelType::Decision);
        assert!(models[0].capabilities.is_empty());

        // 手工挂到别的 provider 下时，靠命名推断也应得到 Decision
        assert_eq!(
            axagent_harness::types::provider_model::detect_model_type("typesafe/jev-1.13"),
            ModelType::Decision
        );
        assert_eq!(
            axagent_harness::types::provider_model::detect_model_type("gpt-4o"),
            ModelType::Chat
        );
    }

    #[test]
    fn empty_state_is_rejected_before_network_call() {
        // 空消息不应触发网络请求，也不应 panic。
        let request = user_request("   ");
        assert_eq!(TypeSafeAdapter::extract_state(&request), "");
    }
}
