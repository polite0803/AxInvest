// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;
use std::sync::Arc;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{
    NodeError, NodeExecutorTrait, NodeOutput, error_code,
};

pub struct SwitchExecutor {
    // SwitchExecutor 在 use_llm 模式下需要执行 LLM 调用，需要 master_key 解密 provider key；
    // 同时保持与 AgentExecutor / LlmExecutor / ConditionExecutor 等兄弟 executor
    // 的构造接口一致(均由 WorkEngine::new 统一注入 master_key)。
    master_key: [u8; 32],
    /// 由 Harness 注入的 ProviderRegistry（运行时按 provider 类型查找 adapter）
    provider_registry: Option<Arc<dyn axagent_harness::registry::ProviderRegistry>>,
}

impl SwitchExecutor {
    pub fn new(master_key: [u8; 32]) -> Self {
        Self { master_key, provider_registry: None }
    }
}

impl Default for SwitchExecutor {
    fn default() -> Self {
        Self::new([0u8; 32])
    }
}

impl axagent_harness::HasProviderRegistry for SwitchExecutor {
    fn set_provider_registry(
        &mut self,
        registry: Arc<dyn axagent_harness::registry::ProviderRegistry>,
    ) {
        self.provider_registry = Some(registry);
    }
}

/// 解析点号分隔路径，从 ExecutionState.variables 提取目标值。
/// 空路径直接返回 None；segments 中间值非对象也返回 None。
fn resolve_var_path(path: &str, context: &ExecutionState) -> Option<serde_json::Value> {
    super::resolve_var_path(path, &context.variables)
}

/// expression 型 switch case 中**注入变量的合法名字**。
///
/// ⚠️ 切勿改回 `_value`：Rhai 拒绝一切以下划线开头的标识符，`let _value = …` 直接报
/// `ErrorParsing(BadInput(MalformedIdentifier("_value")))`。2026-09-13 用 rhai 1.26 实测：
/// `_value` / `_v` / `__v` 全部失败，`value` 通过。
const SWITCH_INPUT_VAR: &str = "value";

/// 把历史写法 `_value` 规范成合法标识符 `value`。
///
/// 2026-09-13 修复「expression 型 case 恒不命中」：
///   执行器原先把输入值注入为 `let _value = …;`，而该标识符在 Rhai 中**无法解析**，
///   于是每个 expression case 都在 `eval` 时失败（仅打一条 `warn!` 后 `continue`），
///   `found` 永远为 `None` ⇒ **恒回落 `default_case`**。
///   实证：`quality-gate` 的 `data-quality.result.grade` 实际为 `"C"`，但落库的
///   `matched_label` 却是 `low-quality` ⇒ C 级数据被误判为低质量，整条决策尾链被路由
///   到保守的 `quality-fallback`（LLM 覆盖公式决策），且无任何用户可见告警。
///
/// 由于 `_value` 在 Rhai 里不可能作为标识符出现，除字符串字面量之外的任何 `_value`
/// 片段都只可能指代本执行器注入的输入值，故在求值前统一改写。改写会跳过字符串字面量，
/// 并要求标识符边界（避免误伤 `_valuex` 这类名字）。
fn normalize_case_expr(expr: &str) -> String {
    const NEEDLE: &str = "_value";
    let chars: Vec<char> = expr.chars().collect();
    let needle: Vec<char> = NEEDLE.chars().collect();
    let is_ident_char = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(expr.len());
    let mut i = 0usize;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            // 字符串字面量内原样透传（含反斜杠转义）
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            quote = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        let boundary_ok = (i == 0 || !is_ident_char(chars[i - 1]))
            && (i + needle.len() >= chars.len() || !is_ident_char(chars[i + needle.len()]));
        if c == '_' && boundary_ok && chars[i..].starts_with(&needle[..]) {
            out.push_str(SWITCH_INPUT_VAR);
            i += needle.len();
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 构造 expression 型 case 的求值脚本（注入变量 + 规范化后的表达式）。
///
/// 抽成独立函数是为了让单测能直接断言「裸跑脚本能解析并得到正确布尔值」——
/// 这正是原实现缺失的验证（原实现只依赖 `tracing::warn!`，静默腐烂了两轮迭代）。
fn build_case_script(rhai_value: &str, expr: &str) -> String {
    format!("let {SWITCH_INPUT_VAR} = {rhai_value}; {}", normalize_case_expr(expr))
}

/// 将 serde_json::Value 转为 Rhai 兼容的字面量表达式。
/// - String → `"value"` (JSON 序列化自带引号和转义)
/// - Number → 原样数字
/// - Bool → true/false
/// - Null → "()" (Rhai 的 unit)
/// - Array → `[v1, v2, ...]` (递归转换)
/// - Object → `#{k1: v1, k2: v2}` (Rhai 对象映射语法)
fn json_to_rhai_literal(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => {
            // 使用 to_string 获得 JSON 字符串格式（带双引号和转义）
            serde_json::Value::String(s.clone()).to_string()
        },
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "()".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_rhai_literal).collect();
            format!("[{}]", items.join(", "))
        },
        serde_json::Value::Object(obj) => {
            let items: Vec<String> = obj
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {}",
                        json_to_rhai_literal(&serde_json::Value::String(k.clone())),
                        json_to_rhai_literal(v)
                    )
                })
                .collect();
            format!("#{{{}}}", items.join(", "))
        },
    }
}

#[async_trait]
impl NodeExecutorTrait for SwitchExecutor {
    fn node_type(&self) -> &'static str {
        "switch"
    }
    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::Switch(n) = node else {
            return Err(NodeError::type_mismatch("switch", self.node_type()));
        };
        let c = &n.config;

        // 1. 取 input_var 的实际值
        let actual = resolve_var_path(&c.input_var, context);
        let actual_str = actual.as_ref().map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        });

        // 2. 确定匹配的 case（matched_label）
        let matched_label: Option<String> = match actual.as_ref() {
            None => c.default_case.clone(),
            Some(actual_val) => {
                let mut found: Option<String> = None;

                // ── expression 模式：用 Rhai 表达式匹配 ──
                if c.match_mode == "expression" {
                    // 将 serde_json::Value 转为 Rhai 兼容字面量
                    let rhai_value = json_to_rhai_literal(actual_val);
                    for case in &c.cases {
                        let expr = &case.value;
                        if expr.is_empty() {
                            continue;
                        }
                        // 构造 Rhai 脚本：注入输入值（合法标识符 `value`）并把历史
                        // `_value` 写法规范化，见 build_case_script / normalize_case_expr。
                        let script = build_case_script(&rhai_value, expr);
                        let mut e = rhai::Engine::new();
                        e.set_max_operations(10_000);
                        e.set_max_call_levels(8);
                        e.set_max_string_size(64_000);
                        e.set_max_array_size(1_000);
                        match e.eval::<bool>(&script) {
                            Ok(true) => {
                                found = Some(case.label.clone());
                                break;
                            },
                            Ok(false) => continue,
                            Err(e) => {
                                tracing::warn!(
                                    "[SwitchExecutor] case '{}' expression 求值失败 (script: {script:?}): {e}",
                                    case.label,
                                );
                                continue;
                            },
                        }
                    }
                    found.or_else(|| c.default_case.clone())
                }
                // ── use_llm 模式：用 LLM 判断 ──
                else if c.use_llm.unwrap_or(false) {
                    // 构造 LLM 路由 prompt：列出所有 case label + default，让 LLM 选最匹配的 label
                    let input_text = actual_str.as_deref().unwrap_or("");
                    let cases_list = c
                        .cases
                        .iter()
                        .enumerate()
                        .map(|(i, case)| format!("{}. {}", i + 1, case.label))
                        .collect::<Vec<_>>()
                        .join("\n");

                    let default_label = c.default_case.as_deref().unwrap_or("(none)");
                    let prompt = if let Some(ref custom_prompt) = c.llm_prompt {
                        format!(
                            "{custom_prompt}\n\n\
                             ## 可选分支（请输出最匹配的分支 label，仅输出 label 文本）\n{cases_list}\n\
                             ## 默认分支\n{default_label}\n\n\
                             ## 输入文本\n{input_text}\n\n\
                             请仅输出最匹配的分支 label 文本，不要包含任何其他内容。",
                        )
                    } else {
                        format!(
                            "你是一个路由判断器。请根据输入文本，选择最匹配的分支。\n\n\
                             ## 可选分支（请输出最匹配的分支 label，仅输出 label 文本）\n{cases_list}\n\
                             ## 默认分支\n{default_label}\n\n\
                             ## 输入文本\n{input_text}\n\n\
                             请仅输出最匹配的分支 label 文本，不要包含任何其他内容。",
                        )
                    };

                    let node_model = c.llm_model.as_deref().filter(|m| !m.is_empty());
                    let session_model =
                        context.variables.get(super::WORKFLOW_MODEL_VAR).and_then(|v| v.as_str());
                    let session_provider_id = context
                        .variables
                        .get(super::WORKFLOW_PROVIDER_ID_VAR)
                        .and_then(|v| v.as_str());

                    let (prov, key, model, adapter, api_key) = super::resolve_provider_and_adapter(
                        &self.master_key,
                        self.provider_registry.as_ref(),
                        node_model,
                        session_model,
                        session_provider_id,
                        None,
                        "SwitchExecutor",
                    )
                    .await?;

                    if context.dry_run {
                        tracing::info!("[SwitchExecutor] dry_run 模式：LLM 路由短路返回首个 case");
                        found = c
                            .cases
                            .first()
                            .map(|case| case.label.clone())
                            .or_else(|| c.default_case.clone());
                    } else {
                        use axagent_harness::build_provider_request_context;
                        use axagent_harness::types::{ChatContent, ChatMessage, ChatRequest};

                        let req_ctx = build_provider_request_context(&prov, &key, api_key);
                        let request = ChatRequest {
                            model: model.clone(),
                            messages: vec![ChatMessage {
                                role: "user".to_string(),
                                content: ChatContent::Text(prompt),
                                tool_calls: None,
                                tool_call_id: None,
                                thinking: None,
                            }],
                            stream: false,
                            temperature: Some(0.0),
                            // P0 FIX (2026-09-08): 64 → 512，与 llm_classifier_executor.rs
                            // 同语义。思考型模型思维链即可耗尽 64 tokens，case label 还没
                            // 输出就被截断 (finish_reason=length)，路由恒失败。
                            max_tokens: Some(512),
                            top_p: None,
                            tools: None,
                            thinking_budget: None,
                            use_max_completion_tokens: None,
                            thinking_param_style: None,
                            api_mode: None,
                            instructions: None,
                            conversation: None,
                            previous_response_id: None,
                            store: None,
                            response_format: None,
                        };

                        let llm_config = axagent_harness::LlmCallConfig::default();
                        let response =
                            axagent_harness::execute_llm(&*adapter, &req_ctx, request, &llm_config)
                                .await
                                .map_err(|e| {
                                    NodeError::exec_failed(
                                        error_code::UNSUPPORTED_PROVIDER,
                                        format!("Switch LLM routing call failed: {e}"),
                                    )
                                })?;

                        let raw_label = response.response.content.trim();
                        // 优先精确匹配 case label；未命中则尝试包含匹配；最后 fallback 到 default
                        let matched = c
                            .cases
                            .iter()
                            .find(|case| case.label == raw_label)
                            .or_else(|| c.cases.iter().find(|case| raw_label.contains(&case.label)))
                            .map(|case| case.label.clone());

                        if let Some(label) = matched {
                            tracing::info!(
                                "[SwitchExecutor] LLM 路由命中 case '{}' (raw: {raw_label:?})",
                                label
                            );
                            found = Some(label);
                        } else {
                            tracing::warn!(
                                "[SwitchExecutor] LLM 路由未命中任何 case (raw: {raw_label:?})，回退到默认分支"
                            );
                            found = c.default_case.clone();
                        }
                    }

                    found
                }
                // ── 传统模式：exact / regex / contains ──
                else {
                    let needle = match actual_str.as_deref() {
                        Some(s) => s,
                        None => return Ok(Self::build_output(c, actual, c.default_case.clone())),
                    };
                    for case in &c.cases {
                        let hit = match c.match_mode.as_str() {
                            "regex" => match regex::Regex::new(&case.value) {
                                Ok(re) => re.is_match(needle),
                                Err(e) => {
                                    tracing::warn!(
                                        "[SwitchExecutor] case '{}' regex 编译失败: {e}",
                                        case.label
                                    );
                                    false
                                },
                            },
                            "contains" => needle.contains(&case.value),
                            _ => needle == case.value,
                        };
                        if hit {
                            found = Some(case.label.clone());
                            break;
                        }
                    }
                    found.or_else(|| c.default_case.clone())
                }
            },
        };

        Ok(Self::build_output(c, actual, matched_label))
    }
}

impl SwitchExecutor {
    fn build_output(
        c: &axagent_harness::workflow_types::SwitchNodeConfig,
        actual: Option<serde_json::Value>,
        matched_label: Option<String>,
    ) -> NodeOutput {
        NodeOutput {
            output: serde_json::json!({
                "input_var": c.input_var,
                "actual_value": actual,
                "matched_label": matched_label,
                "case_count": c.cases.len(),
                "match_mode": c.match_mode,
                "node_id": "",
                "use_llm": c.use_llm.unwrap_or(false),
            }),
            output_var: if c.output_var.is_empty() {
                None
            } else {
                Some(c.output_var.clone())
            },
            control: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归（2026-09-13）：expression 型 case 必须真的能命中。
    ///
    /// 原实现注入 `let _value = …`，而 Rhai 无法解析下划线开头的标识符 ⇒ 表达式恒失败、
    /// 恒回落 default。本测试直接对「执行器实际构造的脚本」断言求值结果，覆盖：
    ///   - 历史写法 `_value`（DB 中已落库的模板仍需可用，靠 normalize 兼容）
    ///   - 规范写法 `value`
    #[test]
    fn expression_case_script_evaluates_and_matches() {
        let cases: &[(&str, &str, bool)] = &[
            ("C", r#"_value == "A" || _value == "B" || _value == "C""#, true),
            ("D", r#"_value == "A" || _value == "B" || _value == "C""#, false),
            ("C", r#"value == "A" || value == "B" || value == "C""#, true),
            ("F", r#"value == "A" || value == "B" || value == "C""#, false),
            ("C", r#"["A","B","C"].contains(_value)"#, true),
            ("D", r#"["A","B","C"].contains(value)"#, false),
        ];
        for (input, expr, expected) in cases {
            let rhai_value = json_to_rhai_literal(&serde_json::Value::String((*input).into()));
            let script = build_case_script(&rhai_value, expr);
            let mut engine = rhai::Engine::new();
            engine.set_max_operations(10_000);
            let got = engine.eval::<bool>(&script);
            assert_eq!(
                got.as_ref().ok().copied(),
                Some(*expected),
                "脚本求值不符合预期: expr={expr:?} value={input} script={script:?} got={got:?}"
            );
        }
    }

    /// 字符串字面量里的 `_value` 不被改写；`_valuex` 这类更长标识符不被误伤。
    ///
    /// 关于属性访问 `x._value`：Rhai 的字段访问与裸标识符共用同一套标识符规则，
    /// `x._value` 与 `_value` 一样在解析期就被拒（`MalformedIdentifier`）。因此这里
    /// 把它一并改写成 `x.value` 是**严格更优**的选择 —— 保留原样只会留下一个必然
    /// 求值失败的表达式；改写后至少语义上指向同名字段且可解析。
    #[test]
    fn normalize_case_expr_skips_string_literals_and_non_identifiers() {
        assert_eq!(normalize_case_expr(r#"_value == "x""#), r#"value == "x""#);
        assert_eq!(normalize_case_expr(r#"_value == "_value""#), r#"value == "_value""#);
        assert_eq!(normalize_case_expr("_valuex == 1"), "_valuex == 1");
        // `.` 不是标识符字符 ⇒ 边界判定通过 ⇒ `_value` 被规范化为可解析的 `value`
        assert_eq!(normalize_case_expr("x._value == 1"), "x.value == 1");
    }
}
