// SPDX-License-Identifier: AGPL-3.0-only

//! 编排步骤的纯校验/变量函数（docs/bot-integration/P3/MODEL-P3 §2.1.1、ADR-P3-7）。
//! 不碰 DB、不碰 AppState；所有定义期错误都带 §4 语义码，命令层只负责转成 ErrorResponse。

use std::collections::{HashMap, HashSet};

use axagent_harness::types::{BotWorkflowStep, BotWorkflowStepOutput};
use serde::{Deserialize, Serialize};

use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

pub const MAX_STEPS: usize = 12;
pub const MAX_CONCURRENCY: i32 = 4;
pub const MIN_TOTAL_TIMEOUT: i32 = 60;
pub const MAX_TOTAL_TIMEOUT: i32 = 3600;
pub const MAX_PROMPT_CHARS: usize = 20_000;
pub const MAX_OUTPUT_CHARS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepError {
    pub code: &'static str,
    pub field: &'static str,
    pub detail: String,
}

impl StepError {
    pub fn new(code: &'static str, field: &'static str, detail: impl Into<String>) -> Self {
        Self { code, field, detail: detail.into() }
    }

    pub fn into_response(self) -> String {
        ErrorResponse::new(self.code)
            .with_category(ErrorCategory::Validation)
            .with_param("field", self.field)
            .with_detail(self.detail)
            .to_string()
    }
}

fn invalid(field: &'static str, detail: impl Into<String>) -> StepError {
    StepError::new(bot_err::WORKFLOW_STEP_INVALID, field, detail)
}

/// JSON 整列 → DTO 数组（前端永不看 steps_json）。
pub fn parse_steps(raw: &str) -> Result<Vec<BotWorkflowStep>, StepError> {
    if raw.trim().is_empty() || raw.trim() == "[]" {
        return Err(invalid("steps", "编排至少需要 1 个步骤"));
    }
    serde_json::from_str(raw).map_err(|e| invalid("steps", format!("steps_json 解析失败: {e}")))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateRef {
    StepOutput(String),
    Input(String),
}

/// 提取 `{{steps.<key>.output}}` / `{{inputs.<key>}}`，其他形态一律拒绝（ADR-P3-7）。
pub fn find_refs(prompt: &str) -> Result<Vec<TemplateRef>, StepError> {
    let mut out = Vec::new();
    let bytes = prompt.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if &bytes[i..i + 2] != b"{{" {
            i += 1;
            continue;
        }
        let Some(end) = prompt[i + 2..].find("}}") else {
            return Err(invalid("promptTemplate", format!("第 {i} 字节处模板未闭合")));
        };
        let token = prompt[i + 2..i + 2 + end].trim();
        let parts: Vec<&str> = token.split('.').collect();
        let reference = match parts.as_slice() {
            ["steps", key, "output"] if is_step_key(key) => {
                TemplateRef::StepOutput((*key).to_string())
            },
            ["inputs", key]
                if !key.is_empty()
                    && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                TemplateRef::Input((*key).to_string())
            },
            _ => {
                return Err(invalid(
                    "promptTemplate",
                    format!("不支持的模板引用: {{{{{token}}}}}"),
                ));
            },
        };
        out.push(reference);
        i += 2 + end + 2;
    }
    Ok(out)
}

fn is_step_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 32
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 定义期纯校验（含拓扑排序）；返回拓扑序 stepKey 列表，供 runner 直接复用。
pub fn validate_dag(steps: &[BotWorkflowStep]) -> Result<Vec<String>, StepError> {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(invalid("steps", format!("步骤数必须为 1..={MAX_STEPS}")));
    }
    let mut keys = HashSet::new();
    for step in steps {
        if !is_step_key(&step.step_key) {
            return Err(invalid("stepKey", format!("非法 stepKey: {}", step.step_key)));
        }
        if !keys.insert(step.step_key.clone()) {
            return Err(invalid("stepKey", format!("重复 stepKey: {}", step.step_key)));
        }
        if step.bot_id.trim().is_empty() {
            return Err(invalid("botId", "botId 不能为空"));
        }
        let chars = step.prompt_template.chars().count();
        if chars < 1 || chars > MAX_PROMPT_CHARS {
            return Err(invalid(
                "promptTemplate",
                format!("promptTemplate 必须为 1..={MAX_PROMPT_CHARS} 字符"),
            ));
        }
        if step.timeout_secs < 1 || step.timeout_secs > 300 {
            return Err(invalid("timeoutSecs", "timeoutSecs 必须为 1..=300"));
        }
        if step.on_failure != "abort" && step.on_failure != "continue" {
            return Err(invalid("onFailure", "onFailure 只能是 abort/continue"));
        }
        if step.depends_on.len() > 11 {
            return Err(invalid("dependsOn", "单步最多 11 个依赖"));
        }
        for dep in &step.depends_on {
            if dep == &step.step_key {
                return Err(StepError::new(
                    bot_err::WORKFLOW_CYCLE,
                    "dependsOn",
                    format!("自引用: {dep}"),
                ));
            }
            if !keys.contains(dep) {
                // keys 可能尚未收齐，循环结束后再统一校验未知依赖。
                continue;
            }
        }
        for reference in find_refs(&step.prompt_template)? {
            if let TemplateRef::StepOutput(ref key) = reference {
                if key == &step.step_key {
                    return Err(StepError::new(
                        bot_err::WORKFLOW_CYCLE,
                        "promptTemplate",
                        format!("自引用输出: {key}"),
                    ));
                }
            }
        }
    }
    for step in steps {
        for dep in &step.depends_on {
            if !keys.contains(dep) {
                return Err(invalid("dependsOn", format!("未知依赖 key: {dep}")));
            }
        }
        for reference in find_refs(&step.prompt_template)? {
            if let TemplateRef::StepOutput(ref key) = reference {
                if !keys.contains(key) {
                    return Err(invalid("promptTemplate", format!("未知输出引用: {key}")));
                }
            }
        }
    }

    // Kahn 拓扑：同层按定义顺序，保证 runner 的调度可复现。
    let mut indegree: Vec<usize> = steps.iter().map(|s| s.depends_on.len()).collect();
    let mut queue: std::collections::VecDeque<usize> =
        (0..steps.len()).filter(|i| indegree[*i] == 0).collect();
    let mut order = Vec::with_capacity(steps.len());
    while let Some(i) = queue.pop_front() {
        order.push(steps[i].step_key.clone());
        for (j, step) in steps.iter().enumerate() {
            if step.depends_on.contains(&steps[i].step_key) {
                indegree[j] -= 1;
                if indegree[j] == 0 {
                    queue.push_back(j);
                }
            }
        }
    }
    if order.len() != steps.len() {
        return Err(StepError::new(bot_err::WORKFLOW_CYCLE, "dependsOn", "编排步骤存在循环依赖"));
    }
    Ok(order)
}

/// 定义全量校验（命令层在 create/update 前调用，纯函数）。
pub fn validate_definition(
    name: &str,
    description: &str,
    owner_scope: &str,
    department_id: Option<&str>,
    steps: &[BotWorkflowStep],
    max_concurrency: i32,
    total_timeout_secs: i32,
) -> Result<Vec<String>, StepError> {
    let name_chars = name.trim().chars().count();
    if name_chars < 1 || name_chars > 100 {
        return Err(invalid("name", "name 必须为 1..100 字符"));
    }
    if description.chars().count() > 1000 {
        return Err(invalid("description", "description 最多 1000 字符"));
    }
    if owner_scope != "instance" && owner_scope != "department" {
        return Err(invalid("ownerScope", "ownerScope 只能是 instance/department"));
    }
    if owner_scope == "department" && department_id.is_none_or(str::is_empty) {
        return Err(StepError::new(
            bot_err::WORKFLOW_DEPARTMENT_REQUIRED,
            "departmentId",
            "部门编排必须指定部门",
        ));
    }
    if max_concurrency < 1 || max_concurrency > MAX_CONCURRENCY {
        return Err(invalid("maxConcurrency", "maxConcurrency 必须为 1..4"));
    }
    if total_timeout_secs < MIN_TOTAL_TIMEOUT || total_timeout_secs > MAX_TOTAL_TIMEOUT {
        return Err(invalid("totalTimeoutSecs", "totalTimeoutSecs 必须为 60..3600"));
    }
    validate_dag(steps)
}

/// 派发前一次性替换；未知/未产出的引用保留空串（定义期已保证 key 合法）。
pub fn substitute_prompt(
    prompt: &str,
    inputs: &HashMap<String, String>,
    outputs: &HashMap<String, BotWorkflowStepOutput>,
) -> String {
    let mut out = prompt.to_string();
    for reference in find_refs(prompt).unwrap_or_default() {
        let (token, value) = match reference {
            TemplateRef::StepOutput(key) => {
                let token = format!("{{{{steps.{key}.output}}}}");
                let text = outputs
                    .get(&key)
                    .map(|o| o.result_text.chars().take(MAX_OUTPUT_CHARS).collect::<String>())
                    .unwrap_or_default();
                (token, text)
            },
            TemplateRef::Input(key) => {
                (format!("{{{{inputs.{key}}}}}"), inputs.get(&key).cloned().unwrap_or_default())
            },
        };
        out = out.replace(&token, &value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bot_err as e;

    fn step(key: &str, deps: &[&str], prompt: &str) -> BotWorkflowStep {
        BotWorkflowStep {
            step_key: key.into(),
            bot_id: "bot-a".into(),
            prompt_template: prompt.into(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            timeout_secs: 300,
            on_failure: "abort".into(),
        }
    }

    #[test]
    fn parse_and_topological_order() {
        let raw = serde_json::to_string(&vec![
            step("c", &["b"], "C {{steps.b.output}}"),
            step("a", &[], "A {{inputs.topic}}"),
            step("b", &["a"], "B"),
        ])
        .unwrap();
        let steps = parse_steps(&raw).unwrap();
        let order = validate_dag(&steps).unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
        assert_eq!(parse_steps("[]").unwrap_err().code, e::WORKFLOW_STEP_INVALID);
    }

    #[test]
    fn duplicate_empty_unknown_and_cycle_errors() {
        let dup = vec![step("a", &[], "A"), step("a", &[], "A")];
        assert_eq!(validate_dag(&dup).unwrap_err().code, e::WORKFLOW_STEP_INVALID);
        assert_eq!(validate_dag(&[]).unwrap_err().code, e::WORKFLOW_STEP_INVALID);
        let unknown = vec![step("a", &["missing"], "A")];
        assert_eq!(validate_dag(&unknown).unwrap_err().code, e::WORKFLOW_STEP_INVALID);
        let cycle = vec![step("a", &["b"], "A"), step("b", &["a"], "B")];
        assert_eq!(validate_dag(&cycle).unwrap_err().code, e::WORKFLOW_CYCLE);
        let self_ref = vec![step("a", &[], "{{steps.a.output}}")];
        assert_eq!(validate_dag(&self_ref).unwrap_err().code, e::WORKFLOW_CYCLE);
    }

    #[test]
    fn template_refs_and_substitution() {
        assert_eq!(
            find_refs("{{steps.a.output}} / {{inputs.topic}}").unwrap(),
            vec![TemplateRef::StepOutput("a".into()), TemplateRef::Input("topic".into())]
        );
        assert_eq!(find_refs("{{steps.a.text}}").unwrap_err().code, e::WORKFLOW_STEP_INVALID);
        assert_eq!(find_refs("{{ steps.a.output }}").unwrap().len(), 1);
        let prompt = "{{inputs.topic}}→{{steps.a.output}}";
        let inputs = [("topic".into(), "盘面".into())].into_iter().collect();
        let outputs = [(
            "a".into(),
            BotWorkflowStepOutput { assignment_id: "x".into(), result_text: "结论".into() },
        )]
        .into_iter()
        .collect();
        assert_eq!(substitute_prompt(prompt, &inputs, &outputs), "盘面→结论");
    }

    #[test]
    fn definition_validation_codes() {
        let steps = vec![step("a", &[], "A")];
        assert_eq!(
            validate_definition("", "", "instance", None, &steps, 2, 900).unwrap_err().code,
            e::WORKFLOW_STEP_INVALID
        );
        assert_eq!(
            validate_definition("wf", "", "department", None, &steps, 2, 900).unwrap_err().code,
            e::WORKFLOW_DEPARTMENT_REQUIRED
        );
        assert_eq!(
            validate_definition("wf", "", "instance", None, &steps, 5, 900).unwrap_err().code,
            e::WORKFLOW_STEP_INVALID
        );
        assert_eq!(
            validate_definition("wf", "", "instance", None, &steps, 2, 10).unwrap_err().code,
            e::WORKFLOW_STEP_INVALID
        );
        assert_eq!(
            validate_definition("wf", "", "department", Some("dept"), &steps, 2, 900)
                .unwrap()
                .len(),
            1
        );
        let bad = vec![step("a", &[], "{{steps.missing.output}}")];
        assert_eq!(
            validate_definition("wf", "", "instance", None, &bad, 2, 900).unwrap_err().code,
            e::WORKFLOW_STEP_INVALID
        );
    }
}
