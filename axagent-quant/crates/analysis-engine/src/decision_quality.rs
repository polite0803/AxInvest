// SPDX-License-Identifier: AGPL-3.0-only

//! 最终决策 JSON 的**形状质量**检查（R-4，2026-10-24）。
//!
//! 为什么放在 analysis-engine 而不是 `commands/stock_workflow/decision.rs`：
//! 这是纯函数，`stock_analysis` 命令调用它时不应新增一条「命令模块横向互调」
//! （`check-layer-discipline.mjs` 的 `commands-no-sibling-call` 会红）。放在引擎层，
//! 命令层只经 `axagent_analysis_engine::decision_quality` 引用。
//!
//! 为什么是「报缺口」而不是「拒绝」：`stock_analyses.decision_json` 是**已经落库**的
//! 决策产物，拒绝写入会把一条可解释的历史记录变成不可读；而静默接受又让缺字段的
//! 决策混进统计。这里返回缺失/类型不符的字段名，调用方把它作为 `decisionSchemaGaps`
//! 附在详情响应上（只读、非破坏），让「这条决策不完整」在 UI/审计里显式可见。

/// 必填字段（与 `seed_stock_analysis` 的 output_schema `required` 同步）。
///
/// `targetPrice` / `stopLoss` 对「持有/观望」本就可空，不列为必填。
pub const DECISION_REQUIRED_FIELDS: [&str; 5] =
    ["action", "confidence", "positionPct", "riskLevel", "reasoning"];

/// 返回缺失 / 空白 / 越界的必填字段名；空数组 = 形状完整。
///
/// 数值字段只接受有限且 0..=100 的数；字符串字段只接受非空（trim 后）串。
/// `decision_json` 缺失或不是合法 JSON 时返回一个合成标记，而不是伪造五个字段。
pub fn schema_gaps(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return vec!["decision_json".to_string()];
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return vec!["decision_json:unparseable".to_string()];
    };
    let str_ok =
        |key: &str| v.get(key).and_then(|x| x.as_str()).is_some_and(|s| !s.trim().is_empty());
    let num_in_0_100 = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_f64())
            .map(|n| n.is_finite() && (0.0..=100.0).contains(&n))
            .unwrap_or(false)
    };
    let mut gaps = Vec::new();
    if !str_ok("action") {
        gaps.push("action".to_string());
    }
    if !num_in_0_100("confidence") {
        gaps.push("confidence".to_string());
    }
    if !num_in_0_100("positionPct") {
        gaps.push("positionPct".to_string());
    }
    if !str_ok("riskLevel") {
        gaps.push("riskLevel".to_string());
    }
    if !str_ok("reasoning") {
        gaps.push("reasoning".to_string());
    }
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_a_complete_decision() {
        let raw = json!({
            "action": "持有",
            "confidence": 66.5,
            "positionPct": 8.7,
            "riskLevel": "中风险",
            "reasoning": "证据充分"
        })
        .to_string();
        assert!(schema_gaps(Some(&raw)).is_empty());
    }

    #[test]
    fn flags_missing_blank_and_out_of_range_fields() {
        let raw = json!({
            "action": "  ",
            "confidence": 101.0,
            "positionPct": -1.0,
            "riskLevel": "",
            "reasoning": ""
        })
        .to_string();
        assert_eq!(schema_gaps(Some(&raw)), DECISION_REQUIRED_FIELDS.map(str::to_string));
    }

    #[test]
    fn flags_absent_and_unparseable_decision_json() {
        assert_eq!(schema_gaps(None), vec!["decision_json"]);
        assert_eq!(schema_gaps(Some("   ")), vec!["decision_json"]);
        assert_eq!(schema_gaps(Some("{not json")), vec!["decision_json:unparseable"]);
    }
}
