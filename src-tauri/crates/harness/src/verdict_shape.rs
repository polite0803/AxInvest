//! `verdict` 形态归一 —— 两处「形态改写」的**单点**判据。
//!
//! 为什么单独成模块：本仓「同一份 Agent 产物，有的消费端按顶层读、有的按 `verdict.*` 读」
//! 从未被生产端契约钉住。2026-10-23 审计（`AUDIT-verdict-shape-normalization-2026-10-23.md`）
//! 清出四处同族问题，其中两处必须由**同一份判据**驱动，否则两处实现必然漂移：
//!
//! - `agent_executor` 的通用后处理：`verdict` 是**空对象**而业务字段仍在顶层 ⇒ 合并进 verdict；
//! - 反思链 `build_sub_analysis_from_snapshot`：快照里的 `content` 已是引擎重构后的**嵌套**
//!   形态，而模板映射按**顶层**读 ⇒ 把 verdict 内字段镜像回顶层。
//!
//! 两个方向都遵循同一纪律：**只补缺，不覆盖、不删除**。这不是洁癖 ——
//! 覆盖会把「LLM 两处结论不一致」变成「被脚本悄悄抹平」（隐藏证据）；
//! 删除会打破另一侧消费端（顶层读法在 `ValueAssessmentPanel` / 导出里仍在用）。
//!
//! 刻意不处理的三类（没有判据、只能猜）：
//! 1. `verdict` 是**非空**对象而另有业务字段留在顶层 —— 两处都有值，不构成 miss；
//! 2. **没有** `verdict` 键的扁平产物（如 `debate-convergence` 的 `consensus_score`）——
//!    那是刻意设计，消费端也按顶层读，包裹它会把可读字段全部搬走；
//! 3. `verdict` 是标量（方向字符串）—— 由 `agent_executor` 的 `needs_flat_reconstruct`
//!    分支负责，本模块不重复实现（否则又是一份实现）。

use serde_json::{Map, Value};

/// 包裹层保留键：不参与「业务字段搬运」。
const RESERVED_KEYS: [&str; 2] = ["report", "verdict"];

fn is_empty_object(v: Option<&Value>) -> bool {
    v.and_then(Value::as_object).is_some_and(Map::is_empty)
}

/// 判据：`verdict` 存在且为空对象 ∧ 顶层还有 `report` / `verdict` 之外的键。
///
/// 命中即「业务字段被晾在顶层、verdict 只剩空壳」—— 此时 `.verdict.<field>` 恒取不到，
/// 而 `.content.<field>` 反而成立，两条读法各读一半。
pub fn needs_merge_flat_into_verdict(content: &Value) -> bool {
    let Some(obj) = content.as_object() else {
        return false;
    };
    is_empty_object(obj.get("verdict")) && obj.keys().any(|k| !RESERVED_KEYS.contains(&k.as_str()))
}

/// 归一：把顶层业务字段合并进空 `verdict` 对象，产出 `{report, verdict: {…}}`。
///
/// 未命中判据 ⇒ `None`（调用方保持原值，不做任何复制）。
pub fn merge_flat_into_verdict(content: &Value) -> Option<Value> {
    if !needs_merge_flat_into_verdict(content) {
        return None;
    }
    let obj = content.as_object()?;
    let mut out = obj.clone();
    // 原 verdict 是空对象，先把它（未来可能出现的半填充）当底，再覆盖业务字段。
    let mut verdict = obj.get("verdict").and_then(Value::as_object).cloned().unwrap_or_default();
    for (k, v) in obj {
        if RESERVED_KEYS.contains(&k.as_str()) {
            continue;
        }
        verdict.insert(k.clone(), v.clone());
    }
    out.insert("verdict".into(), Value::Object(verdict));
    Some(Value::Object(out))
}

/// 反向归一：把 `verdict` 内的业务字段**镜像**回顶层（verdict 原样保留）。
///
/// 用途：快照记忆（`_raw.*`）里存的是引擎重构后的嵌套 `content`，而反思模板的
/// `input_mapping` 按 `content.<field>` 顶层读（`stock_analysis_setup/mod.rs` 的
/// `trader_action` 等三条）。镜像后两套读法同时成立。
///
/// 跳过 `verdict` 键本身（容器不可被内部的方向串覆盖）；顶层已有值不覆盖。
/// 没有任何字段被镜像（verdict 为空 / 缺失 / 顶层已齐全）⇒ `None`。
pub fn mirror_verdict_fields_to_top(content: &Value) -> Option<Value> {
    let obj = content.as_object()?;
    let verdict = obj.get("verdict")?.as_object()?;
    if verdict.is_empty() {
        return None;
    }
    let mut out = obj.clone();
    let mut mirrored = 0usize;
    for (k, v) in verdict {
        if k == "verdict" || out.contains_key(k.as_str()) {
            continue;
        }
        out.insert(k.clone(), v.clone());
        mirrored += 1;
    }
    (mirrored > 0).then_some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 正例：2026-10-23 实测的 value-investor 形态（verdict 空壳 + 打分在顶层）。
    #[test]
    fn merges_business_fields_into_empty_verdict() {
        let content = json!({
            "report": "护城河稳固",
            "verdict": {},
            "bull_score": 70,
            "bear_score": 30,
            "confidence": 65,
        });
        assert!(needs_merge_flat_into_verdict(&content));
        let merged = merge_flat_into_verdict(&content).expect("应命中并归一");
        assert_eq!(merged["report"], json!("护城河稳固"));
        assert_eq!(merged["verdict"]["bull_score"], json!(70));
        assert_eq!(merged["verdict"]["bear_score"], json!(30));
        assert_eq!(merged["verdict"]["confidence"], json!(65));
        // 顶层不删（另一侧消费端仍按顶层读）
        assert_eq!(merged["bull_score"], json!(70));
    }

    /// 反例①：`debate-convergence` 刻意没有 `verdict` 键 ⇒ 不得包裹。
    #[test]
    fn leaves_flat_output_without_verdict_key_alone() {
        let content = json!({ "report": "收敛报告", "consensus_score": 65 });
        assert!(!needs_merge_flat_into_verdict(&content));
        assert!(merge_flat_into_verdict(&content).is_none());
    }

    /// 反例②：`verdict` 已是非空对象（引擎分支 B 的产物）⇒ 不再动。
    #[test]
    fn leaves_nested_non_empty_verdict_alone() {
        let content = json!({
            "report": "r",
            "verdict": { "verdict": "看多", "bull_score": 70 },
        });
        assert!(!needs_merge_flat_into_verdict(&content));
        assert!(merge_flat_into_verdict(&content).is_none());
    }

    /// 反例③：空 verdict 且**只有** report ⇒ 没有业务字段可搬。
    #[test]
    fn ignores_empty_verdict_without_business_fields() {
        let content = json!({ "report": "只有正文", "verdict": {} });
        assert!(!needs_merge_flat_into_verdict(&content));
    }

    /// 反射链用途：嵌套 content ⇒ 顶层也能读到 `action` / `confidence`。
    #[test]
    fn mirrors_nested_verdict_fields_to_top_level() {
        let content = json!({
            "report": "交易员结论",
            "verdict": { "verdict": "看多", "action": "买入", "confidence": 72, "targetPrice": 31.2 },
        });
        let mirrored = mirror_verdict_fields_to_top(&content).expect("应镜像");
        assert_eq!(mirrored["action"], json!("买入"));
        assert_eq!(mirrored["confidence"], json!(72));
        assert_eq!(mirrored["targetPrice"], json!(31.2));
        // verdict 保留；顶层 `verdict` 容器不被内部方向串覆盖
        assert_eq!(mirrored["verdict"]["action"], json!("买入"));
        assert!(mirrored["verdict"].is_object());
    }

    /// 顶层已有值不覆盖（LLM 两处结论不一致时，保留原顶层口径）。
    #[test]
    fn mirror_never_overwrites_existing_top_level_value() {
        let content = json!({
            "report": "r",
            "action": "卖出",
            "verdict": { "action": "买入", "confidence": 50 },
        });
        let mirrored = mirror_verdict_fields_to_top(&content).expect("应镜像 confidence");
        assert_eq!(mirrored["action"], json!("卖出"), "顶层原值优先");
        assert_eq!(mirrored["confidence"], json!(50));
    }

    /// 反例：无 verdict / 空 verdict ⇒ 无事可做。
    #[test]
    fn mirror_returns_none_when_nothing_to_do() {
        assert!(
            mirror_verdict_fields_to_top(&json!({ "report": "r", "consensus_score": 60 }))
                .is_none()
        );
        assert!(mirror_verdict_fields_to_top(&json!({ "report": "r", "verdict": {} })).is_none());
        // 顶层已齐全 ⇒ 不产生新对象（moved == 0）
        assert!(
            mirror_verdict_fields_to_top(&json!({
                "report": "r",
                "action": "买入",
                "verdict": { "action": "买入" },
            }))
            .is_none()
        );
    }
}
