// SPDX-License-Identifier: AGPL-3.0-only

use super::ExpressionContext;
use regex::Regex;
use serde_json::Value;

/// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("表达式解析失败: {0}")]
    ParseError(String),
    #[error("表达式求值失败: {0}")]
    EvalError(String),
    #[error("表达式返回非字符串值")]
    NonStringValue,
}

/// 单个 `{{ }}` 占位符的求值结果。
enum SlotOutcome {
    /// 成功取到值，替换进文本
    Value(Value),
    /// 取不到值 ⇒ **原样保留** `{{...}}`，把决定权交给下游 executor
    KeepLiteral,
}

/// 解析单个占位符 —— **一套语法，两条查值通道**（2026-10-23 合并两套模板引擎）。
///
/// ## 合并前的两套语义（互不兼容）
/// - **dispatcher 侧**（本函数，走 Rhai）：只认 `{{ $vars.x }}` / `{{ $node["id"].y }}`；
///   裸名在 Rhai 里是「变量未定义」，带连字符的路径（`a-catalyst.content`）更是被
///   解析成**减法**。
/// - **agent prompt 侧**（`prompt_template::render_prompt`，走点路径）：认 `{{stock_code}}`
///   / `{{a-catalyst.content.action}}`，且被大量写进 `agency_experts/*.md` 与 seed 的
///   `system_prompt`。
///
/// 而 dispatcher 在节点执行**之前**就会对整个节点 JSON（含 `config.system_prompt`）
/// 递归扫描 ⇒ prompt 里的裸路径必然抛错 ⇒ `resolve_value_templates` 整体失败 ⇒
/// **整个节点回退**，同节点里合法的 `{{ $vars.x }}` 也跟着一起失去解析机会。
/// 2026-10-23 运行日志实证：8 次 `Variable not found: stock_code` + 整节点回退；
/// 另有 65 处 `{{input}}` 同型（`input` 同样不在 Rhai scope 里）。
///
/// ## 合并后的唯一规则
/// - 以 `$` 开头 ⇒ Rhai 表达式（算术 / 比较 / `$node[...]` / `$now` / `$env` 能力不变）；
/// - 其余 ⇒ 点路径查 `ctx.variables`，实现复用 `prompt_template::resolve_dot_path`
///   （与 prompt 渲染**同一份**代码，不会漂移）。
///
/// 两通道读的是同一个变量表（`ExecutionState.variables`），所以「路径命中」的语义
/// 与 prompt 渲染完全一致。
///
/// **取不到值时不报错、保留原文**：此前是抛错 → 整节点回退，代价是「一个占位符
/// 让整份节点配置失去解析」；现在保留 `{{path}}`，由下游 executor 按自身语义处理
/// （agent 节点仍会由 `render_prompt` 抛 `TEMPLATE_VAR_NOT_FOUND`，可观测性不丢）。
fn resolve_slot(expr: &str, ctx: &ExpressionContext) -> SlotOutcome {
    if expr.starts_with('$') {
        return match super::rhai_eval::resolve_expression(expr, ctx) {
            Ok(v) => SlotOutcome::Value(v),
            Err(e) => {
                // 显式表达式写错就是写错，但**不再连坐整个节点**
                tracing::warn!(expr = %expr, error = %e, "表达式占位符求值失败，保留原文");
                SlotOutcome::KeepLiteral
            },
        };
    }
    match crate::work_engine::prompt_template::resolve_dot_path(expr, &ctx.variables) {
        Ok(v) => SlotOutcome::Value(v.clone()),
        Err(_) => {
            tracing::debug!(
                expr = %expr,
                "路径占位符未在变量表中命中，保留原文交由下游 executor 处理"
            );
            SlotOutcome::KeepLiteral
        },
    }
}

/// 解析字符串模板中的 {{ expression }}，提取表达式，求值，拼接结果
///
/// 示例:
///   "Hello {{ $vars.name }}, total: {{ $vars.qty * $vars.price }}"
///   → 扫描到两个表达式，逐个求值后拼接
///   "股票 {{stock_code}}" → 按路径取变量表中的 `stock_code`（见 `resolve_slot`）
pub fn resolve_template(template: &str, ctx: &ExpressionContext) -> Result<String, TemplateError> {
    let re = Regex::new(r"\{\{(.+?)\}\}").map_err(|e| TemplateError::ParseError(e.to_string()))?;
    let mut result = String::new();
    let mut last_end = 0;

    for caps in re.captures_iter(template) {
        let full_match = caps
            .get(0)
            .ok_or_else(|| TemplateError::ParseError("missing capture group 0".into()))?;
        let expr = caps
            .get(1)
            .ok_or_else(|| TemplateError::ParseError("missing capture group 1".into()))?
            .as_str()
            .trim();

        // 添加模板片段
        result.push_str(&template[last_end..full_match.start()]);

        // 求值表达式
        match resolve_slot(expr, ctx) {
            // 转字符串拼接
            SlotOutcome::Value(Value::String(s)) => result.push_str(&s),
            SlotOutcome::Value(Value::Null) => {}, // null 插入空字符串
            SlotOutcome::Value(other) => result.push_str(&other.to_string()),
            // 原样保留（含原始空格形态），交给下游 executor
            SlotOutcome::KeepLiteral => result.push_str(full_match.as_str()),
        }

        last_end = full_match.end();
    }

    // 添加尾部片段
    result.push_str(&template[last_end..]);
    Ok(result)
}

/// 对 Value 递归解析模板
/// 遍历 JSON 树，对所有字符串值调用 resolve_template
pub fn resolve_value_templates(
    value: &Value,
    ctx: &ExpressionContext,
) -> Result<Value, TemplateError> {
    match value {
        Value::String(s) => {
            if s.contains("{{") {
                Ok(Value::String(resolve_template(s, ctx)?))
            } else {
                Ok(value.clone())
            }
        },
        Value::Array(arr) => {
            let resolved: Result<Vec<Value>, _> =
                arr.iter().map(|v| resolve_value_templates(v, ctx)).collect();
            Ok(Value::Array(resolved?))
        },
        Value::Object(obj) => {
            let resolved: Result<serde_json::Map<String, Value>, _> = obj
                .iter()
                .map(|(k, v)| Ok((k.clone(), resolve_value_templates(v, ctx)?)))
                .collect();
            Ok(Value::Object(resolved?))
        },
        _ => Ok(value.clone()),
    }
}

#[cfg(test)]
mod unified_syntax_tests {
    use super::*;
    use serde_json::json;

    fn ctx_with(vars: serde_json::Value) -> ExpressionContext {
        let mut ctx = ExpressionContext::new();
        if let serde_json::Value::Object(obj) = vars {
            for (k, v) in obj {
                ctx.variables.insert(k, v);
            }
        }
        ctx
    }

    /// 合并前后的**核心差异**：prompt 里写裸路径（无 `$`）必须能取到值，
    /// 而不是抛 `Variable not found` 让整个节点回退。
    #[test]
    fn bare_path_slot_reads_variable_table() {
        let ctx = ctx_with(json!({ "stock_code": "002242", "stock_name": "九阳股份" }));
        let out = resolve_template("分析 {{stock_code}}（{{ stock_name }}）", &ctx).unwrap();
        assert_eq!(out, "分析 002242（九阳股份）");
    }

    /// 连字符 / 中文 key 在 Rhai 里会被当减法解析 —— 这正是旧实现必然失败、
    /// 而 prompt 渲染本来支持的那批占位符（`a-catalyst` / `t-scoring` 等节点名）。
    #[test]
    fn hyphenated_and_nested_path_slots_resolve() {
        let ctx = ctx_with(json!({
            "a-catalyst": { "content": { "catalyst_level": "L2" } },
            "sub-analysis": { "trader": { "action": "持有" } }
        }));
        let out = resolve_template(
            "催化剂 {{a-catalyst.content.catalyst_level}} / 动作 {{sub-analysis.trader.action}}",
            &ctx,
        )
        .unwrap();
        assert_eq!(out, "催化剂 L2 / 动作 持有");
    }

    /// `$` 前缀仍走 Rhai，算术与 `$vars.` 能力不能退化。
    #[test]
    fn dollar_prefixed_slot_still_uses_rhai() {
        let ctx = ctx_with(json!({ "qty": 3, "price": 8.5 }));
        let out = resolve_template("合计 {{ $vars.qty * $vars.price }}", &ctx).unwrap();
        assert_eq!(out, "合计 25.5");
    }

    /// 取不到值 ⇒ 保留原文（不报错）。此前会让 `resolve_value_templates` 整体 Err
    /// ⇒ dispatcher 丢掉整份节点配置（含同节点里合法的 `$vars` 占位符）。
    #[test]
    fn unresolvable_slot_is_kept_verbatim() {
        let ctx = ctx_with(json!({ "stock_code": "002242" }));
        let out = resolve_template("代码 {{stock_code}} / 未命中 {{not_in_table}}", &ctx).unwrap();
        assert_eq!(out, "代码 002242 / 未命中 {{not_in_table}}");
    }

    /// 显式表达式写错也不得连坐：整串其它占位符照常解析。
    #[test]
    fn broken_dollar_expression_does_not_drop_siblings() {
        let ctx = ctx_with(json!({ "stock_code": "002242" }));
        let out = resolve_template("{{ $vars.nope.deeper }} + {{stock_code}}", &ctx).unwrap();
        assert!(out.contains("002242"), "同节点的路径占位符必须仍被解析: {out}");
        assert!(out.contains("{{ $vars.nope.deeper }}"), "失败的表达式保留原文: {out}");
    }

    /// 递归入口不因单个槽位失败而整体失败（这是 dispatcher 回退的触发点）。
    #[test]
    fn value_templates_never_fail_on_missing_slot() {
        let ctx = ctx_with(json!({ "stock_code": "002242" }));
        let node = json!({
            "system_prompt": "{{stock_code}} / {{missing_thing}}",
            "nested": { "arr": ["{{stock_code}}"] }
        });
        let resolved = resolve_value_templates(&node, &ctx).expect("不得整体失败");
        assert_eq!(resolved["system_prompt"], json!("002242 / {{missing_thing}}"));
        assert_eq!(resolved["nested"]["arr"][0], json!("002242"));
    }
}
