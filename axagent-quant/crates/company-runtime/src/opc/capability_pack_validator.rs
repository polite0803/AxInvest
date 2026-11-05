//! 域包验证服务 — 迁移自 OpcCapabilityPackAdapter::validate
//!
//! **阶段 A3（YAML 成唯一权威）：校验规则不再硬编码，改从各域包
//! `runtime.yaml` 的 `validations` 段读取并通用求值。**
//!
//! 求值语义对齐迁移前的 `if let Some(field) = entity_data.get(field)` 习惯：
//! **字段缺失 → 视为通过（不报错）**；字段存在但值不符 → 报错。
//! 这条基线让既有 YAML 声明（如 `customer.email not_empty`）在缺失字段时
//! 也不误报，与历史硬编码一致。

use std::path::Path;

use serde_json::Value;

use super::capability_pack::resolve_capability_packs_dir;
use super::capability_pack::runtime_schema::{self, RuntimeValidation};
use super::error::OpcResult;
use super::rules::ValidationError;

/// 判断一组 YAML 校验规则是否作用于当前实体类型。
///
/// `entity_type == "*"` 为通配（对所有实体生效），如 finance_invest 的
/// `investment_amount` 校验。
fn rule_applies(rt: &RuntimeValidation, entity_type: &str) -> bool {
    rt.entity_type == "*" || rt.entity_type == entity_type
}

/// 单个字段值是否为「空」（空串 / null / 空数组 / 空对象）。
fn is_empty(v: &Value) -> bool {
    match v {
        Value::String(s) => s.is_empty(),
        Value::Null => true,
        Value::Bool(_) | Value::Number(_) => false,
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

fn make_error(rt: &RuntimeValidation) -> ValidationError {
    ValidationError::new(&rt.field, &rt.message)
}

/// 对一条 YAML 规则求值，返回「报错」或「通过（None）」。
///
/// 除 `any_not_empty` 外的所有 operator 都会先取 `entity_data.get(field)`；
/// **字段缺失即跳过该规则**（对齐历史 `if let Some` 基线）。
fn evaluate_one(rt: &RuntimeValidation, entity: &Value) -> Option<ValidationError> {
    // any_not_empty：field 是虚拟名，value 是字段名数组，任一字段非空即通过。
    if rt.operator == "any_not_empty" {
        let any_non_empty = rt
            .value
            .as_array()
            .map(|fields| {
                fields.iter().any(|f| {
                    f.as_str().and_then(|name| entity.get(name)).is_some_and(|v| !is_empty(v))
                })
            })
            .unwrap_or(false);
        return if any_non_empty {
            None
        } else {
            Some(make_error(rt))
        };
    }

    let Some(target) = entity.get(&rt.field) else {
        return None; // 字段缺失 → 通过
    };

    let threshold = rt.value.as_f64();
    let failed = match rt.operator.as_str() {
        "not_empty" => is_empty(target),
        "ge" | "gt" => match (target.as_f64(), threshold) {
            (Some(v), Some(t)) => {
                if rt.operator == "ge" {
                    v < t
                } else {
                    v <= t
                }
            },
            _ => true, // 非数值 → 按不满足处理
        },
        "contains" | "starts_with" => {
            let needle = rt.value.as_str().unwrap_or("");
            match target.as_str() {
                Some(s) => {
                    if rt.operator == "contains" {
                        !s.contains(needle)
                    } else {
                        !s.starts_with(needle)
                    }
                },
                None => true, // 目标非字符串 → 不满足
            }
        },
        "in" => match target.as_str() {
            Some(s) => {
                rt.value.as_array().is_none_or(|arr| !arr.iter().any(|v| v.as_str() == Some(s)))
            },
            None => true, // 目标非字符串值不在集合 → 不满足
        },
        "len" => {
            let expect = threshold.unwrap_or(f64::NAN) as usize;
            target.as_str().is_none_or(|s| s.chars().count() != expect)
        },
        // 未知 operator：不阻断校验（保守放行）。
        _ => false,
    };

    failed.then(|| make_error(rt))
}

/// 读取域包 `runtime.yaml` 的 `validations` 段。
///
/// 文件缺失 / 解析失败返回 `None` → 无校验（与「该域包没有规则」等价，不报错）。
fn load_validations(
    domain_pack_id: &str,
    app_dir: Option<&Path>,
) -> Option<Vec<RuntimeValidation>> {
    let base = resolve_capability_packs_dir(app_dir);
    let dir = base.join(domain_pack_id.replace('-', "_"));
    if !dir.is_dir() {
        return Some(Vec::new());
    }
    let raw = std::fs::read_to_string(dir.join("runtime.yaml")).ok()?;
    serde_yaml::from_str::<runtime_schema::RuntimeKpiConfig>(&raw).map(|c| c.validations).ok()
}

/// 验证域包实体（统一入口，YAML 驱动）。
pub async fn validate_entity(
    domain_pack_id: &str,
    entity_type: &str,
    entity_data: &Value,
    app_dir: Option<&Path>,
) -> OpcResult<Vec<ValidationError>> {
    let Some(rules) = load_validations(domain_pack_id, app_dir) else {
        return Ok(Vec::new());
    };
    Ok(rules
        .iter()
        .filter(|r| rule_applies(r, entity_type))
        .filter_map(|r| evaluate_one(r, entity_data))
        .collect())
}

/// 批量验证域包实体。
pub async fn validate_batch(
    domain_pack_id: &str,
    entities: &[(String, Value)],
    app_dir: Option<&Path>,
) -> OpcResult<Vec<(String, Vec<ValidationError>)>> {
    let mut results = Vec::with_capacity(entities.len());
    for (entity_type, entity_data) in entities {
        let errors = validate_entity(domain_pack_id, entity_type, entity_data, app_dir).await?;
        results.push((entity_type.clone(), errors));
    }
    Ok(results)
}
