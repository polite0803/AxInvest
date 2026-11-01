// SPDX-License-Identifier: AGPL-3.0-only

//! 需求发现 LLM 精评桥（依赖倒置）
//!
//! 需求线索的规则评分（关键词/数值启发式，见 `marketplace_scanner::evaluate_lead`）
//! 只看得到文本表面信号：市场空白度恒为默认值、反讽与含蓄表达会误判。本模块
//! 定义一个**窄接口**让 lib 层把已配置的 LLM 能力注入进来，扫描管线对通过预筛
//! 的候选做一轮批量精评（低温、结构化 JSON 输出）。
//!
//! 为什么是 trait 而不是直接依赖 `axagent_agent::ProviderLlmBridge`：
//! `tools` 是被 `agent`/`runtime` 依赖的底层 crate，反向依赖会造成循环。
//! lib 层持有 `ProviderLlmBridge`（自带 provider 选择/fallback），包一层
//! [`DemandLlmBridge`] 后经 `global_state::set_demand_llm` 注册即可。
//!
//! 未注册（未配置任何 provider）或调用失败时，精评静默跳过，规则评分兜底
//! —— 精评是增强，不是依赖。

use crate::tools::marketplace_scanner::DemandType;
use crate::tools::scan_policy::ScanPolicy;
use async_trait::async_trait;
use axagent_dao::repo::opc_demand;
use axagent_harness::types::opc_demand::DemandLeadDto;
use sea_orm::DatabaseConnection;

/// 需求精评 LLM 桥：一次「system+user → 文本」补全
#[async_trait]
pub trait DemandLlmBridge: Send + Sync {
    /// 低温结构化调用。返回原始文本（由调用方做 JSON 容错解析）。
    async fn call(&self, system: &str, user: &str) -> Result<String, String>;
}

/// 单条精评结论（LLM 输出，经容错解析后）
#[derive(Debug, Clone, PartialEq)]
pub struct RefineVerdict {
    pub id: String,
    pub pain_score: f64,
    pub market_gap_score: f64,
    pub commercial_value_score: f64,
    pub confidence: f64,
    pub demand_type: String,
    pub reason: String,
}

/// 精评系统提示（低温 + 严格 JSON 数组契约）
fn refine_system_prompt() -> String {
    "你是需求价值评估专家。对给定的每条需求线索独立评估：\n\
     - pain_score 痛点强度(0-100)：需求方表达的急迫、痛苦、受阻程度（反讽吐槽不算）\n\
     - market_gap_score 市场空白度(0-100)：现有解决方案越少、越难替代，分越高\n\
     - commercial_value_score 商业价值(0-100)：付费意愿 × 需求频率 × 可远程交付性\n\
     - confidence 置信度(0.1-0.9)：文本信息对以上判断的支持程度\n\
     - demand_type：只能是 tool_software / content_creation / design / development / operations / marketing / education / enterprise_service / outsourcing / consulting / unknown 之一\n\
     - reason：一句中文理由（不超过 50 字）\n\
     只输出 JSON 数组，形如 [{\"id\":\"...\",\"pain_score\":80,\"market_gap_score\":60,\"commercial_value_score\":75,\"confidence\":0.7,\"demand_type\":\"development\",\"reason\":\"...\"}]，不要输出数组以外的任何文本。"
        .to_string()
}

/// 组装精评 user 提示（候选线索的紧凑 JSON 列表）
fn build_user_prompt(candidates: &[DemandLeadDto]) -> String {
    let items: Vec<serde_json::Value> = candidates
        .iter()
        .map(|l| {
            serde_json::json!({
                "id": l.id,
                "platform": l.platform,
                "title": l.title,
                "description": l.description,
                "budget_min": l.budget_min,
                "budget_max": l.budget_max,
                "rule_commercial_value": l.commercial_value_score,
            })
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
}

/// 从 LLM 原始输出容错解析精评结论
///
/// 剥掉 markdown 代码围栏后按 JSON 数组解析；单条字段缺失/类型错就丢弃该条
/// （保留规则分），`id` 不在候选集合里的丢弃。`fallback_demand_type` 给出
/// demand_type 无效时的回退值（候选自己的规则分类）。
fn parse_verdicts(raw: &str, valid_ids: &std::collections::HashSet<String>) -> Vec<RefineVerdict> {
    let trimmed = raw.trim();
    // 剥 ```json ... ``` / ``` ... ``` 围栏
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let body = body.strip_suffix("```").map(str::trim).unwrap_or(body.trim());

    let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(body) else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|v| {
            let id = v.get("id")?.as_str()?.to_string();
            if !valid_ids.contains(&id) {
                return None;
            }
            let num = |key: &str| v.get(key).and_then(serde_json::Value::as_f64);
            Some(RefineVerdict {
                id,
                pain_score: num("pain_score")?.clamp(0.0, 100.0),
                market_gap_score: num("market_gap_score")?.clamp(0.0, 100.0),
                commercial_value_score: num("commercial_value_score")?.clamp(0.0, 100.0),
                confidence: num("confidence")?.clamp(0.1, 0.9),
                demand_type: v.get("demand_type")?.as_str()?.to_string(),
                reason: v
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// 对本轮入库/刷新的线索做 LLM 精评并回写
///
/// 返回成功精评的条数。任何环节失败（未配置 bridge、调用失败、解析为空）
/// 都只记日志返回 0 —— 规则评分已在入库时落库，精评是纯增强。
pub async fn refine_round_leads(
    db: &DatabaseConnection,
    round_leads: &[DemandLeadDto],
    policy: &ScanPolicy,
) -> usize {
    if !policy.llm_eval_enabled || round_leads.is_empty() {
        return 0;
    }

    // 预筛：规则分 ≥ 阈值，按规则分降序，截断到上限（控 token 成本）
    let mut candidates: Vec<DemandLeadDto> = round_leads
        .iter()
        .filter(|l| l.commercial_value_score >= policy.llm_eval_min_rule_score)
        .cloned()
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    candidates.sort_by(|a, b| b.commercial_value_score.total_cmp(&a.commercial_value_score));
    candidates.truncate(policy.llm_eval_max_leads);

    let Some(bridge) = crate::global_state::get_demand_llm() else {
        tracing::debug!(
            "[demand_llm] 未注册精评 LLM 桥（未配置 provider），本轮 {} 条候选保留规则评分",
            candidates.len()
        );
        return 0;
    };

    let raw = bridge.call(&refine_system_prompt(), &build_user_prompt(&candidates)).await;
    let raw = match raw {
        Ok(text) => text,
        Err(e) => {
            tracing::warn!(error = %e, "[demand_llm] 精评调用失败，本轮保留规则评分");
            return 0;
        },
    };

    let valid_ids: std::collections::HashSet<String> =
        candidates.iter().map(|l| l.id.clone()).collect();
    let verdicts = parse_verdicts(&raw, &valid_ids);
    if verdicts.is_empty() {
        tracing::warn!("[demand_llm] 精评输出解析为空，本轮保留规则评分");
        return 0;
    }

    let mut refined = 0usize;
    for verdict in &verdicts {
        // demand_type 无效（不在枚举内）→ 回退候选自身的规则分类
        let demand_type = if verdict.demand_type.parse::<DemandType>().is_ok() {
            verdict.demand_type.clone()
        } else {
            candidates
                .iter()
                .find(|l| l.id == verdict.id)
                .map(|l| l.demand_type.clone())
                .unwrap_or_else(|| "unknown".to_string())
        };
        match opc_demand::refine_lead_scores(
            db,
            &verdict.id,
            opc_demand::LeadScoreRefinement {
                pain_score: verdict.pain_score,
                market_gap_score: verdict.market_gap_score,
                commercial_value_score: verdict.commercial_value_score,
                confidence: verdict.confidence,
                demand_type,
                llm_analysis: verdict.reason.clone(),
            },
        )
        .await
        {
            Ok(()) => refined += 1,
            Err(e) => {
                tracing::warn!(lead_id = %verdict.id, error = %e, "[demand_llm] 精评结果回写失败")
            },
        }
    }
    tracing::info!(total = candidates.len(), refined, "[demand_llm] LLM 精评完成");
    refined
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn ids(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_plain_json_array() {
        let raw = r#"[{"id":"a","pain_score":80,"market_gap_score":60,
            "commercial_value_score":75,"confidence":0.7,
            "demand_type":"development","reason":"明确开发需求"}]"#;
        let verdicts = parse_verdicts(raw, &ids(&["a", "b"]));
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].id, "a");
        assert_eq!(verdicts[0].pain_score, 80.0);
        assert_eq!(verdicts[0].demand_type, "development");
    }

    #[test]
    fn parse_strips_markdown_fence() {
        let raw = "```json\n[{\"id\":\"a\",\"pain_score\":50,\"market_gap_score\":50,\
                   \"commercial_value_score\":50,\"confidence\":0.5,\
                   \"demand_type\":\"design\",\"reason\":\"ok\"}]\n```";
        assert_eq!(parse_verdicts(raw, &ids(&["a"])).len(), 1);
    }

    #[test]
    fn parse_drops_unknown_ids_and_bad_items() {
        let raw = r#"[
            {"id":"ghost","pain_score":1,"market_gap_score":1,"commercial_value_score":1,"confidence":0.5,"demand_type":"design","reason":""},
            {"id":"a","pain_score":"high","market_gap_score":50,"commercial_value_score":50,"confidence":0.5,"demand_type":"design","reason":""},
            {"id":"a","pain_score":70,"market_gap_score":60,"commercial_value_score":80,"confidence":0.8,"demand_type":"weird_type","reason":"x"}
        ]"#;
        let verdicts = parse_verdicts(raw, &ids(&["a"]));
        assert_eq!(verdicts.len(), 1, "ghost 被丢弃、类型错被丢弃，只留有效条");
        assert_eq!(verdicts[0].commercial_value_score, 80.0);
    }

    #[test]
    fn parse_clamps_out_of_range_scores() {
        let raw = r#"[{"id":"a","pain_score":150,"market_gap_score":-5,
            "commercial_value_score":75,"confidence":5,
            "demand_type":"development","reason":""}]"#;
        let v = &parse_verdicts(raw, &ids(&["a"]))[0];
        assert_eq!(v.pain_score, 100.0);
        assert_eq!(v.market_gap_score, 0.0);
        assert_eq!(v.confidence, 0.9);
    }

    #[test]
    fn parse_garbage_returns_empty() {
        assert!(parse_verdicts("我不是 JSON", &ids(&["a"])).is_empty());
        assert!(parse_verdicts("[]", &ids(&["a"])).is_empty());
    }
}
