// SPDX-License-Identifier: AGPL-3.0-only

//! 代码执行器 —— 执行 CodeNode 中的代码片段。
//!
//! 支持两种模式：
//! - `execute_directly = false`（默认）：Rhai 脚本注册为工具，由 Agent/LLM 调用
//! - `execute_directly = true`：在 DAG 中直接执行 Rhai 代码，通过 input_mapping
//!   从 context.variables 读取结构化参数，输出 JSON 结果

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;
use serde_json::Value;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{
    NodeError, NodeExecutorTrait, NodeOutput, error_code,
};

pub struct CodeExecutor;

impl CodeExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodeExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// 执行 Rhai 脚本的 in-process 引擎。
/// 通过 `input_mapping` 从 context.variables 读取注入值为数字/字符串，
/// 并通过 Rhai 的 `Scope` 传递给脚本，执行后收集结果构造 JSON 输出。
///
/// Phase 5: 返回 (script_result, input_params_snapshot) 二元组。
/// input_params_snapshot 是所有 input_mapping 解析值的快照，
/// 用于 What-If 回测 UI 读取原始参数值。
fn execute_rhai_directly(
    code: &str,
    input_mapping: &std::collections::HashMap<String, String>,
    context: &ExecutionState,
) -> Result<(serde_json::Value, serde_json::Value), NodeError> {
    use rhai::{Engine, Scope};

    let mut engine = Engine::new();
    // Rhai 无内建 clamp，portfolio-mgr.rhai 等脚本依赖
    engine.register_fn("clamp", |value: f64, min: f64, max: f64| -> f64 {
        if value < min {
            min
        } else if value > max {
            max
        } else {
            value
        }
    });
    let mut scope = Scope::new();
    let mut input_params_snapshot = serde_json::Map::new();

    // 将 input_mapping 的值注入 Rhai scope
    for (target_key, source_key) in input_mapping {
        let value = super::resolve_var_path(source_key, &context.variables);
        // 记录解析值的快照（Phase 5: What-If 回测参数持久化）
        let snapshot_value = value.clone().unwrap_or(Value::Null);
        input_params_snapshot.insert(target_key.clone(), snapshot_value);
        // 将 Value 转换为 Rhai 动态类型；解析失败时推入 ()（单元值/空）
        match &value {
            Some(Value::Null) | None => {
                let _ = scope.push_constant(target_key.as_str(), ());
            },
            Some(Value::Bool(b)) => {
                let _ = scope.push_constant(target_key.as_str(), *b);
            },
            Some(Value::Number(n)) => {
                let val = if let Some(f) = n.as_f64() {
                    f
                } else if let Some(i) = n.as_i64() {
                    i as f64
                } else if let Some(u) = n.as_u64() {
                    u as f64
                } else {
                    0.0_f64
                };
                let _ = scope.push_constant(target_key.as_str(), val);
            },
            Some(Value::String(s)) => {
                let _ = scope.push_constant(target_key.as_str(), s.clone());
            },
            Some(Value::Array(arr)) => {
                let rhai_arr: rhai::Array = arr.iter().map(json_value_to_dynamic).collect();
                let dyn_arr: rhai::Dynamic = rhai_arr.into();
                scope.push_dynamic(target_key.as_str(), dyn_arr);
            },
            Some(Value::Object(obj)) => {
                let mut map = rhai::Map::new();
                for (k, v) in obj {
                    map.insert(k.clone().into(), json_value_to_dynamic(v));
                }
                scope.push_dynamic(target_key.as_str(), map.into());
            },
        }
    }

    // 执行脚本，期望返回一个 map
    let result: rhai::Dynamic = engine.eval_with_scope(&mut scope, code).map_err(|e| {
        NodeError::exec_failed(error_code::VALIDATION_FAILED, format!("Rhai execution failed: {e}"))
    })?;

    // 将 Rhai 结果转换回 JSON
    Ok((dynamic_to_json_value(&result), Value::Object(input_params_snapshot)))
}

/// 将 serde_json::Value 转换为 Rhai Dynamic
fn json_value_to_dynamic(v: &Value) -> rhai::Dynamic {
    match v {
        Value::Null => rhai::Dynamic::UNIT,
        Value::Bool(b) => rhai::Dynamic::from(*b),
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                rhai::Dynamic::from(f)
            } else if let Some(i) = n.as_i64() {
                rhai::Dynamic::from(i as f64)
            } else {
                rhai::Dynamic::from(0.0_f64)
            }
        },
        Value::String(s) => rhai::Dynamic::from(s.clone()),
        Value::Array(arr) => {
            let items: rhai::Array = arr.iter().map(json_value_to_dynamic).collect();
            rhai::Dynamic::from(items)
        },
        Value::Object(obj) => {
            let mut map = rhai::Map::new();
            for (k, v) in obj {
                map.insert(k.clone().into(), json_value_to_dynamic(v));
            }
            rhai::Dynamic::from(map)
        },
    }
}

/// 将 Rhai Dynamic 转换回 serde_json::Value
fn dynamic_to_json_value(v: &rhai::Dynamic) -> Value {
    if v.is_unit() {
        return Value::Null;
    }
    if v.is_bool() {
        return Value::Bool(v.as_bool().unwrap_or(false));
    }
    if let Ok(s) = v.clone().into_string() {
        return Value::String(s);
    }
    if let Ok(i) = v.as_int() {
        return Value::Number(serde_json::Number::from(i));
    }
    if let Ok(f) = v.as_float() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
        return Value::Number(serde_json::Number::from(0));
    }
    // Array
    if let Some(arr) = v.clone().try_cast::<rhai::Array>() {
        return Value::Array(
            arr.into_iter()
                .map(|item| dynamic_to_json_value(&item))
                .collect(),
        );
    }
    // Map
    if let Some(map) = v.clone().try_cast::<rhai::Map>() {
        let mut obj = serde_json::Map::new();
        for (k, val) in &map {
            obj.insert(format!("{k}"), dynamic_to_json_value(val));
        }
        return Value::Object(obj);
    }
    Value::String(format!("{v}"))
}

#[async_trait]
impl NodeExecutorTrait for CodeExecutor {
    fn node_type(&self) -> &'static str {
        "code"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::Code(code_node) = node else {
            return Err(NodeError::type_mismatch(
                "code".to_string(),
                super::node_type_name(node).to_string(),
            ));
        };

        // ── 直接执行模式（execute_directly=true）──
        // Rhai 脚本在 DAG 中直接执行，通过 input_mapping 消费上游结构化参数。
        if code_node.config.execute_directly && code_node.config.language == "rhai" {
            tracing::info!(
                "[code_executor] Rhai execution: node_type={:?}, input_mapping keys={:?}, variables keys={:?}",
                super::node_type_name(node),
                code_node.config.input_mapping.keys().collect::<Vec<_>>(),
                context.variables.keys().collect::<Vec<_>>(),
            );
            let (result, input_params) = execute_rhai_directly(
                &code_node.config.code,
                &code_node.config.input_mapping,
                context,
            )?;
            // Phase 5: 将 input_mapping 解析值快照嵌入 output.input_params，
            // 确保 What-If 回测 UI 可直接读取原始参数值，无需从上游节点重建。
            return Ok(NodeOutput {
                output: serde_json::json!({
                    "status": "executed",
                    "language": "rhai",
                    "result": result,
                    "input_params": input_params,
                    "node_id": node.base_id(),
                    // 将 result 中的关键决策字段提升到 params 层，供下游 resolve_var_path 消费
                    "params": result,
                }),
                output_var: Some(code_node.config.output_var.clone()),
            });
        }

        // ── 工具注册模式（向后兼容）──
        // Rhai 脚本已在预处理阶段编译并注册为工具，DAG 中无需执行
        if code_node.config.language == "rhai" {
            let tool_name = code_node
                .config
                .tool_name
                .clone()
                .unwrap_or_else(|| format!("code_{}", code_node.base.id));
            return Ok(NodeOutput {
                output: serde_json::json!({
                    "status": "tool_registered",
                    "tool_name": tool_name,
                    "note": "Rhai 脚本已注册为工具，由 Agent/LLM 调用，无需 DAG 执行",
                    "node_id": node.base_id(),
                }),
                output_var: Some(code_node.config.output_var.clone()),
            });
        }

        // 非 Rhai 语言：返回代码摘要供 LLM 或下游节点使用
        let code_lines = code_node.config.code.lines().count();
        Ok(NodeOutput {
            output: serde_json::json!({
                "status": "code_ready",
                "language": code_node.config.language,
                "code_lines": code_lines,
                "code_preview": &code_node.config.code[..code_node.config.code.len().min(500)],
                "node_id": node.base_id(),
            }),
            output_var: Some(code_node.config.output_var.clone()),
        })
    }
}
