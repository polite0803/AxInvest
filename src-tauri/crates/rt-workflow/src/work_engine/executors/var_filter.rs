//! 共享变量过滤器：把 `ExecutionState.variables` 中的"模板变量"和"数据变量"分离。
//!
//! 背景：在股票分析工作流中，100+ 用户配置参数（如 `scoring_trend`、`fscore_roe_min`、
//! `risk_hhi_concentrated`）会通过 `start_workflow_execution` 的 `options.variables`
//! 全部灌入 `state.variables`。这些是**模板变量**：本意是给 Tool 节点通过 `_template_vars`
//! 消费，**不应该**自动进 LLM 的 user_prompt。
//!
//! 而"数据变量"则是上游节点输出（tool/agent/decision/subworkflow 等节点 ID 作为 key），
//! 以及用户输入（`stock_code`、`stock_name` 等），这些才是 LLM 真正该看到的。
//!
//! 用法：所有 LLM 调用的"全变量 fallback"路径都应使用 `is_data_var()` 过滤。

use serde_json::Value;
use std::collections::HashMap;

/// 节点输出变量的 key 前缀。匹配 `state.variables` 中所有节点 ID 形式的 key。
const NODE_PREFIXES: &[&str] = &["t-", "a-", "d-", "s-", "j-", "m-", "r-"];

/// 系统级变量（不应进入 LLM 上下文）。如 `__workflow_model__` / `__workflow_provider_id__`。
const SYSTEM_VARS: &[&str] = &["__workflow_model__", "__workflow_provider_id__"];

/// 用户输入参数（work flow 启动时由 caller 传入的 input_params）。
/// 这些是非节点输出、非模板变量的"数据"，可以作为 LLM 上下文。
const USER_INPUT_KEYS: &[&str] = &["stock_code", "stock_name"];

/// 判断 `state.variables` 中的一个 key 是否属于"应该发给 LLM"的数据变量。
///
/// 规则：
/// 1. 节点输出（`t-`/`a-`/`d-`/`s-`/`j-`/`m-`/`r-` 前缀）→ ✅
/// 2. 系统变量（`__*`）→ ❌
/// 3. 已知用户输入 key（`stock_code`/`stock_name`）→ ✅
/// 4. 其他（如 `scoring_trend`/`fscore_roe_min` 等模板变量）→ ❌
///
/// 这是一个保守的实现：宁可漏发（让 LLM 通过 context_sources 精确获取），
/// 也不要把 100+ 模板变量全部硬灌到 LLM user_prompt 里。
pub fn is_data_var(key: &str) -> bool {
    // 系统变量永远不发
    if key.starts_with("__") || SYSTEM_VARS.contains(&key) {
        return false;
    }
    // 节点输出
    if NODE_PREFIXES.iter().any(|p| key.starts_with(p)) {
        return true;
    }
    // 已知用户输入
    if USER_INPUT_KEYS.contains(&key) {
        return true;
    }
    // 其他一律视为模板变量，不发
    false
}

/// 过滤 ExecutionState.variables，只保留数据变量（节点输出 + 已知用户输入）。
///
/// 用于替换 LLM 调用的"全变量 fallback"路径（agent_executor / llm_classifier_executor
/// 中当 `context_sources` 或 `input_var` 为空时的兼容分支），避免把 100+ 模板参数
/// 全部硬灌给 LLM。
pub fn collect_data_vars(variables: &HashMap<String, Value>) -> Vec<(&String, &Value)> {
    let mut out: Vec<(&String, &Value)> =
        variables.iter().filter(|(k, _)| is_data_var(k)).collect();
    // 排序确保稳定输出（便于调试 + 单元测试断言）
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// 从 `ExecutionState.variables` 中解析点分隔路径。
///
/// `path` 支持两种模式：
/// 1. `a-market-analyst.params.bull_score` → 点号路径，逐层导航 JSON 嵌套
/// 2. `stock_code` → 无点号，直接按 key 查找（完全兼容旧行为）
///
/// 实现逻辑与 tool_executor/condition_executor 中的同名函数保持一致。
pub fn resolve_var_path(path: &str, variables: &HashMap<String, Value>) -> Option<Value> {
    if path.is_empty() {
        return None;
    }
    let parts: Vec<&str> = path.split('.').collect();
    // 尝试按节点输出路径解析：root 为节点 ID，后续为嵌套字段
    if let Some(root) = variables.get(parts[0]) {
        let mut current = root.clone();
        for part in &parts[1..] {
            current = current.get(part)?.clone();
        }
        return Some(current);
    }
    // fallback：root 不是变量名，将整个 path 作为变量名直查
    variables.get(path).cloned()
}

/// 从 LLM 回复文本中提取 JSON 结构化参数。
///
/// 提取策略（按优先级）：
/// 1. 查找 ```json ... ``` 代码块中的内容
/// 2. 查找顶层 `{...}` 结构并尝试解析
/// 3. 直接尝试解析整个文本
///
/// 如果 LLM 输出为纯文本（无 JSON 结构），返回 `None`。
pub fn extract_json_params(text: &str) -> Option<Value> {
    // 策略 1：查找 ```json ... ``` 代码块
    for marker in &["```json\n", "```json\r\n", "```\n", "```\r\n"] {
        if let Some(after_marker) = text.split(marker).nth(1) {
            if let Some(json_text) = after_marker.split("```").next() {
                let trimmed = json_text.trim();
                if !trimmed.is_empty() {
                    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                        return Some(v);
                    }
                }
            }
        }
    }

    // 策略 2：查找顶层 { ... } 结构，追踪花括号深度
    if let Some(start) = text.find('{') {
        let mut depth = 0u32;
        for (i, ch) in text[start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let candidate = &text[start..start + i + 1];
                        if let Ok(v) = serde_json::from_str::<Value>(candidate) {
                            return Some(v);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    // 策略 3：直接解析整个文本
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return Some(v);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filters_template_vars() {
        let mut vars = HashMap::new();
        vars.insert("stock_code".into(), json!("000001"));
        vars.insert("stock_name".into(), json!("平安银行"));
        vars.insert("scoring_trend".into(), json!(30));
        vars.insert("fscore_roe_min".into(), json!(0.10));
        vars.insert("t-fundamentals".into(), json!({"eps": 1.5}));
        vars.insert("a-market-analyst".into(), json!({"content": "..."}));
        vars.insert("__workflow_model__".into(), json!("gpt-4"));

        let data = collect_data_vars(&vars);
        let keys: Vec<&str> = data.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"stock_code"));
        assert!(keys.contains(&"stock_name"));
        assert!(keys.contains(&"t-fundamentals"));
        assert!(keys.contains(&"a-market-analyst"));
        assert!(!keys.contains(&"scoring_trend"));
        assert!(!keys.contains(&"fscore_roe_min"));
        assert!(!keys.contains(&"__workflow_model__"));
    }

    #[test]
    fn is_data_var_classifies_correctly() {
        assert!(is_data_var("t-fundamentals"));
        assert!(is_data_var("a-market-analyst"));
        assert!(is_data_var("d-final"));
        assert!(is_data_var("stock_code"));
        assert!(is_data_var("stock_name"));
        assert!(!is_data_var("scoring_trend"));
        assert!(!is_data_var("fscore_roe_min"));
        assert!(!is_data_var("risk_hhi_concentrated"));
        assert!(!is_data_var("__workflow_model__"));
        assert!(!is_data_var("execution_id"));
    }

    #[test]
    fn resolve_var_path_flat_key() {
        let mut vars = HashMap::new();
        vars.insert("stock_code".into(), json!("600036"));
        assert_eq!(resolve_var_path("stock_code", &vars), Some(json!("600036")));
    }

    #[test]
    fn resolve_var_path_dot_path() {
        let mut vars = HashMap::new();
        vars.insert(
            "a-market-analyst".into(),
            json!({
                "params": {
                    "bull_score": 40,
                    "bear_score": 50,
                    "confidence": 70,
                },
                "content": "分析文本...",
            }),
        );
        assert_eq!(
            resolve_var_path("a-market-analyst.params.bull_score", &vars),
            Some(json!(40))
        );
        assert_eq!(
            resolve_var_path("a-market-analyst.params", &vars),
            Some(json!({"bull_score": 40, "bear_score": 50, "confidence": 70}))
        );
        assert_eq!(
            resolve_var_path("a-market-analyst.content", &vars),
            Some(json!("分析文本..."))
        );
    }

    #[test]
    fn resolve_var_path_missing_key() {
        let vars = HashMap::new();
        assert_eq!(resolve_var_path("nonexistent", &vars), None);
    }

    #[test]
    fn resolve_var_path_empty() {
        let vars = HashMap::new();
        assert_eq!(resolve_var_path("", &vars), None);
    }

    #[test]
    fn extract_json_params_from_code_block() {
        let text = r##"这是分析文本。

```json
{
  "trend_state": "震荡",
  "bull_score": 40,
  "bear_score": 50,
  "confidence": 70
}
```

以上是我的分析。"##;
        let result = extract_json_params(text).unwrap();
        assert_eq!(result["bull_score"], json!(40));
        assert_eq!(result["trend_state"], json!("震荡"));
    }

    #[test]
    fn extract_json_params_from_top_level() {
        let text = r##"分析结果：{"action":"buy","positionPct":30,"confidence":75}"##;
        let result = extract_json_params(text).unwrap();
        assert_eq!(result["action"], json!("buy"));
        assert_eq!(result["positionPct"], json!(30));
    }

    #[test]
    fn extract_json_params_no_json() {
        let text = "这是一个纯文本回复，没有任何 JSON 结构。";
        assert_eq!(extract_json_params(text), None);
    }

    #[test]
    fn extract_json_params_from_plain_json() {
        let text = r##"{"sentiment":"bullish","confidence":0.8}"##;
        let result = extract_json_params(text).unwrap();
        assert_eq!(result["sentiment"], json!("bullish"));
        assert_eq!(result["confidence"], json!(0.8));
    }
}
