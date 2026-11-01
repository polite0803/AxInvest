// SPDX-License-Identifier: AGPL-3.0-only
//! OntologyQuery — 领域业务本体的只读查询工具（P1 §4.3.3）
//!
//! 按能力域（`domain`）路由到 `harness::domain_ontology::ONTOLOGY_DOMAINS` 注册表，
//! 返回该域的类层级 / 对象属性 / 度量 / 分档 / 顶层权重快照；可按 `concept` 查单个类
//! （含直接子类），按 `score` 查所属分档。
//!
//! **本工具一次注册、全域可见** —— 新增能力域本体只需在 `ONTOLOGY_DOMAINS` 加一行，
//! 无需再造工具、也无需改本文件（「扩展能力内生」的落点）。
//! 未登记域返回 `DOMAIN_UNREGISTERED`，而非空洞的成功（避免调用方误以为查到了内容）。
//!
//! 类别取 `Knowledge`（只读）：`is_read_only` 由 `category().is_read_only()` 派生，
//! 默认组 `builtin-knowledge` 已在组表里存在 —— 无需改动权限白名单。

use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolErrorKind, ToolResult};
use async_trait::async_trait;
use axagent_harness::capability::CapabilityDomain;
use axagent_harness::domain_ontology::{class_by_id_in, ontology_of};
use axagent_harness::error_codes::ontology::DOMAIN_UNREGISTERED;
use serde_json::{Value, json};

pub struct OntologyQueryTool;

#[async_trait]
impl Tool for OntologyQueryTool {
    fn name(&self) -> &str {
        "ontology_query"
    }

    fn description(&self) -> &str {
        "查询某能力域的业务本体（类层级 / 对象属性 / 度量 / 分档 / 顶层权重）。\
         入参 domain 取能力域协议 id（如 finance / devops / ai_media …），必填。\
         提供 concept 返回该类的定义与其直接子类；提供 score 返回该分数所属分档。\
         均为只读查询，不改任何状态。"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "domain": {
                    "type": "string",
                    "description": "能力域 id（CapabilityDomain.as_str()，如 finance）"
                },
                "concept": {
                    "type": "string",
                    "description": "类 ID（PascalCase，如 Chokepoint）；给则同时返回该类的直接子类"
                },
                "score": {
                    "type": "number",
                    "description": "综合分（0-100）；给则返回该分数在该域分档表中所属的分档 id"
                }
            },
            "required": ["domain"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Knowledge
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let domain_str = input["domain"].as_str().unwrap_or("").trim();
        if domain_str.is_empty() {
            return Err(ToolError::invalid_input_for(
                "ontology_query",
                "domain 为必填参数（能力域协议 id，如 finance）",
            ));
        }
        let domain: CapabilityDomain = domain_str.parse().map_err(|_| ToolError {
            message: format!("能力域 id '{domain_str}' 无法解析（枚举值见 capability.rs）"),
            kind: ToolErrorKind::InvalidInput,
            error_code: DOMAIN_UNREGISTERED.to_string(),
        })?;

        // 未登记域 → DOMAIN_UNREGISTERED（明确报错，不放空洞成功）
        let Some(onto) = ontology_of(domain) else {
            return Err(ToolError {
                message: format!(
                    "能力域 '{domain_str}' 尚未注册领域本体（ONTOLOGY_DOMAINS 无此域，\
                     仅 finance 已登记）。新增域本体 = 在注册表加一行，无需改本工具。"
                ),
                kind: ToolErrorKind::NotFound,
                error_code: DOMAIN_UNREGISTERED.to_string(),
            });
        };

        let classes = into_values(
            onto.classes,
            |c| json!({ "id": c.id, "label": c.label, "parent": c.parent }),
        );
        let object_properties = into_values(
            onto.object_properties,
            |p| json!({ "id": p.id, "label": p.label, "domain": p.domain, "range": p.range }),
        );
        let metrics = into_values(
            onto.metrics,
            |m| json!({ "id": m.id, "label": m.label, "owner": m.owner, "unit": m.unit, "min": m.min, "max": m.max }),
        );
        let bands =
            into_values(onto.bands, |b| json!({ "id": b.id, "min": b.min, "label": b.label }));
        let weights = onto.weights.map(|w| {
            json!({ "supply": w.supply, "demand": w.demand, "irreplaceability": w.irreplaceability })
        });

        let mut out = json!({
            "domain": domain.as_str(),
            "registered": true,
            "classes": classes,
            "objectProperties": object_properties,
            "metrics": metrics,
            "bands": bands,
            "weights": weights,
        });

        // concept → 单个类 + 直接子类（域内）
        let concept = input["concept"].as_str().unwrap_or("").trim();
        if !concept.is_empty() {
            match class_by_id_in(domain, concept) {
                Some(c) => {
                    let subs: Vec<Value> = onto
                        .classes
                        .iter()
                        .filter(|x| x.parent == Some(c.id))
                        .map(|x| json!({ "id": x.id, "label": x.label }))
                        .collect();
                    out["conceptMatch"] = json!({
                        "id": c.id,
                        "label": c.label,
                        "parent": c.parent,
                        "meaning": c.meaning,
                        "subclasses": subs,
                    });
                },
                None => {
                    out["conceptMatch"] = json!(null);
                },
            }
        }

        // score → 所属分档 id（域内分档表，与 band_for_score 同一规则）
        if let Some(score) = input["score"].as_f64() {
            let s = if score.is_nan() {
                0.0
            } else {
                score.clamp(0.0, 100.0)
            };
            let band_id =
                onto.bands.iter().find(|b| s >= b.min).map(|b| b.id).unwrap_or("no_signal");
            out["scoreBand"] = json!(band_id);
        }
        if input.get("score").is_some() && input["score"].is_null() {
            out["scoreBand"] = json!("no_signal");
        }

        let body = serde_json::to_string_pretty(&out)
            .unwrap_or_else(|_| format!("能力域 '{domain_str}' 本体序列化失败"));
        Ok(ToolResult::success(body))
    }
}

/// 把静态切片按映射转成 JSON 数组。
fn into_values<T>(slice: &'static [T], f: impl Fn(&T) -> Value) -> Vec<Value> {
    slice.iter().map(f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::domain_ontology::ONTOLOGY_DOMAINS;

    #[test]
    fn category_is_read_only_knowledge() {
        let tool = OntologyQueryTool;
        assert_eq!(tool.category(), ToolCategory::Knowledge);
        assert!(tool.category().is_read_only(), "ontology_query 必须只读，不得产生副作用");
        assert_eq!(tool.category().default_group(), "builtin-knowledge");
    }

    #[test]
    fn finance_is_registered_and_hits_classes() {
        // 与注册表自洽：finance 固有一条，且 class_by_id_in 能查到本体的顶层类
        let onto = ontology_of(CapabilityDomain::Finance).expect("finance 必须已登记");
        let chokepoint =
            class_by_id_in(CapabilityDomain::Finance, "Chokepoint").expect("Chokepoint 应在本体里");
        assert_eq!(chokepoint.label, "瓶颈环节");
        assert!(onto.bands.len() >= 4, "finance 分档应完整");
        assert!(onto.weights.is_some(), "finance 应带顶层权重");
        assert!(
            ONTOLOGY_DOMAINS.iter().any(|d| d.domain == CapabilityDomain::Finance),
            "注册表第一条应为 finance"
        );
    }
}
