//! 工作流结果 → blackboard_snapshot 转换器
//!
//! 时间旅行模式下，snapshot 自动注入 `_meta` 块（mode / as_of_date / source / built_at），
//! 供回放审计与跨日报告聚合使用。

use axagent_astock_data::as_of::{AsOfContext, DegradationEntry};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// 工作流结果 → blackboard_snapshot（JSON 对象，键名采用约定前缀）
///
/// 键名约定（与 `stock_analysis.rs` 读取端保持一致）：
///   - `report.<nodeId>`         分析师报告（如 `report.a-fundamentals`）
///   - `report.investment-plan`  交易员方案（trader 节点映射到该键）
///   - `value.assessment`        价值投资（巴菲特）评估
///   - `rule_check.summary`      规则检查结果
///   - `data_quality_summary`    数据质量总评
///   - `raw.combined`            原始数据聚合
///   - `degraded`                时间锚定降级报告（spec §4.1）
///   - `_meta`                   元数据：mode / as_of_date / source / built_at / cost（本轮
///     token 与缓存计量汇总，构造见 `build_cost_summary`）
///   - `params.<nodeId>`         结构化参数（Agent 节点的 .params + CodeNode 的 .result）
///   - `decision-explainer`      决策依据说明书（保留 JSON 结构：summary / explanation /
///     rule_trace[] / risk_comment / confidence_note）
///   - 其余节点（debate/risk/research-mgr/portfolio-mgr 等）按 nodeId 存
///
/// `degradation` 由调用方在 workflow 完成时通过 `as_of::take_asof_degradation_report()`
/// 取出后传入。live 模式下该函数返回空 Vec,生成的 `degraded` 块也为空对象。
pub fn build_blackboard_snapshot(
    results: &HashMap<String, Value>,
    as_of_ctx: Option<&AsOfContext>,
    degradation: &[DegradationEntry],
) -> HashMap<String, Value> {
    let mut bb: HashMap<String, Value> = HashMap::new();
    for (node_id, raw_output) in results {
        // v133（B2-2）：value-investor 按档产出（`value-investor--mid|long`）——
        // 键按档区分（`value.assessment--{tier}`），两个实例不会互相覆盖；
        // 裸 base（历史快照）仍映射到 `value.assessment`（向后兼容）。
        let node_base =
            axagent_harness::holding_period::analyst_base_of(node_id).unwrap_or(node_id);
        let key = match node_id.as_str() {
            // 9 个分析师（含逐档实例 `a-*--{tier}`）+ value-investor：归到 report.* 前缀
            id if id.starts_with("a-") => format!("report.{id}"),
            // 交易员：归到 report.investment-plan
            "trader" => "report.investment-plan".to_string(),
            // 价值投资评估（带档实例经 node_base 判定）
            _ if node_base == "value-investor" => match node_id.strip_prefix("value-investor--") {
                Some(tier) => format!("value.assessment--{tier}"),
                None => "value.assessment".to_string(),
            },
            // 规则检查
            "rule-check" => "rule_check.summary".to_string(),
            // 数据质量
            "data-quality" => "data_quality_summary".to_string(),
            // 原始数据聚合
            "raw-data" => "raw.combined".to_string(),
            // _meta 保留给元数据，节点不会命名为这个
            "_meta" => continue,
            // 其余按 nodeId 直接存
            _ => node_id.clone(),
        };

        // 辩论/风险/研究经理/投资组合经理等结构化节点：保留原始 JSON
        // 前端 loadAnalysis 根据 key 前缀做结构化还原。这些节点的输出含有嵌套
        // 结构（如 { content, params, rounds }），flatten 成纯文本会丢失信息。
        let is_structured = node_id.starts_with("bull-r")
            || node_id.starts_with("bear-r")
            || node_id.starts_with("risk-")
            || *node_id == "agg-risk"
            || *node_id == "debate-bull-bear"
            || *node_id == "debate-convergence"
            || node_base == "value-investor"
            || *node_id == "research-mgr"
            || *node_id == "portfolio-mgr"
            // V40 修复: data-quality(JSON模式含grade/score字段)、
            // rule-check(含violations/corrections)、
            // quality-fallback(含决策JSON)的输出均含结构化字段，
            // 需保留JSON结构而非用extract_node_text提取纯文本。
            || *node_id == "data-quality"
            || *node_id == "rule-check"
            || *node_id == "quality-fallback"
            // 2026-09-14: decision-explainer（决策依据说明书）输出为
            // {summary, explanation, rule_trace[], risk_comment, confidence_note}。
            // 若走下面的 extract_node_text 分支会被压成纯文本，前端拿不到
            // `rule_trace` 数组（规则追溯码无法逐条渲染）⇒ 必须保留 JSON 结构。
            // 该节点是 AgentNode 包装（{role, content: <json_string>, ...}），
            // 三个 if 分支都不命中，value_to_store 保持包装对象原样，
            // 前端用 extractContent 取 content 后再解析（与 risk-* 同款处理）。
            || *node_id == "decision-explainer";
        if is_structured {
            // 结构化节点：优先用 report + verdict 重构带 VERDICT 标签的文本
            let mut value_to_store = raw_output.clone();
            if let Some(obj) = raw_output.as_object() {
                if let Some(verdict) = obj.get("verdict") {
                    if let Some(report) = obj.get("report").and_then(|v| v.as_str()) {
                        value_to_store = Value::String(reconstruct_verdict_text(
                            obj.get("__verdict_only"),
                            report,
                            verdict,
                        ));
                    }
                } else if let Some(result) = obj.get("result") {
                    // V41 修复 (2026-07-24): CodeNode 包装但无 verdict 字段
                    //   (data-quality / rule-check / quality-fallback 等 RHAI 确定性节点
                    //    的 output 形如 {status, language, result: <诊断报告>, input_params, node_id, params})
                    // 历史 bug: 直接存整个包装对象 → 前端 String(对象) = "[object Object]" (15 字符)，
                    //          JSON.parse 解析失败 → DecisionBanner 触发"数据质量诊断未渲染"降级面板。
                    // 这里提取 result 字段并 JSON.stringify 成字符串存入 snapshot,
                    // 前端 loadAnalysis 拿到的是纯诊断报告 JSON 字符串,可直接 JSON.parse。
                    let json_str = match serde_json::to_string(result) {
                        Ok(s) => s,
                        Err(_) => result.to_string(),
                    };
                    value_to_store = Value::String(json_str);
                } else if let Some(output) = obj.get("output") {
                    // V63 修复: portfolio-mgr 格式为 {node_id, output, source, status}，
                    // 决策数据在 .output 字段中（无 result/verdict）。提取 output 字段
                    // 让前端 loadAnalysis 能正确解析决策数据。
                    value_to_store = output.clone();
                }
            }
            bb.insert(key, value_to_store);
        } else {
            let mut text = extract_node_text(raw_output);
            // 如果 content 是 JSON 且含有 verdict 字段（strict_mode 重构格式），
            // 把 verdict 补回 <!-- VERDICT: ... --> 标签，让前端 tryParseVerdictFormat 能识别
            if let Some(obj) = raw_output.as_object() {
                if let Some(content_str) = obj.get("content").and_then(|v| v.as_str()) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(content_str) {
                        if let Some(verdict) = json.get("verdict") {
                            if let Some(report) = json.get("report").and_then(|v| v.as_str()) {
                                text = reconstruct_verdict_text(
                                    json.get("__verdict_only"),
                                    report,
                                    verdict,
                                );
                            }
                        }
                    }
                }
            }
            bb.insert(key, Value::String(text));
        }

        // ── 结构化参数专用存储（结构化参数方案 Phase 4）──
        // 保存每个节点的结构化参数，供 What-If 回测 UI 读取原始参数值
        // 并允许用户修改后重算。
        // 注意：extract_node_text 会丢失 JSON 结构，因此需要单独保存。
        if let Some(obj) = raw_output.as_object() {
            // V37 修复: AgentNode 输出无顶层 .params 字段（只有 CodeNode 有）。
            // AgentNode 的业务参数在 .content JSON 字符串内部，需解析后提取。
            // CodeNode 的决策结果在 .result 字段，存到 result.{node_id} 以示区分。
            if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                if let Ok(parsed) = serde_json::from_str::<Value>(content) {
                    if parsed.is_object() {
                        bb.insert(format!("params.{node_id}"), parsed.clone());
                    }
                }
            }
            // CodeNode 直接执行：.result 在顶层，存到 result.{node_id}
            if let Some(result) = obj.get("result") {
                bb.insert(format!("result.{node_id}"), result.clone());
            }
            // 兼容：如果 AgentNode 的顶层有 .params（非标准但保留），
            // 同时存到 .content 解析结果之后（后者优先）
            if let Some(params) = obj.get("params") {
                if !params.is_null() && !bb.contains_key(&format!("params.{node_id}")) {
                    bb.insert(format!("params.{node_id}"), params.clone());
                }
            }
        }
    }
    let mut meta = build_meta(as_of_ctx);
    // 每轮计量事实（tokens / 节点数 / 缓存命中率）。写进 `_meta` 而不是新表：
    // 快照本来就要落库，加一个对象是零迁移、零前端改动；查询侧
    // `blackboard_snapshot::jsonb #> '{_meta,cost}'` 一条即可。
    if let Some(cost) = build_cost_summary(results) {
        meta["cost"] = cost;
    }
    bb.insert("_meta".into(), meta);
    // 降级报告: spec §4.1 统一降级协议
    // 结构: { count, sources: [{ vendor, method, reason, asOf }] }
    let sources: Vec<Value> = degradation
        .iter()
        .map(|d| {
            json!({
                "vendor": d.vendor,
                "method": d.method,
                "reason": d.reason,
                "asOf": d.as_of,
            })
        })
        .collect();
    bb.insert(
        "degraded".into(),
        json!({
            "count": degradation.len(),
            "sources": sources,
        }),
    );
    // 注入原始节点输出（以 _raw.{nodeId} 为键），供 rerun_decision 等工具读取
    // 上游节点使用原始 nodeId 而非被黑板上 key remapping 改写后的键。
    for (node_id, raw_output) in results {
        bb.insert(format!("_raw.{node_id}"), raw_output.clone());
    }
    bb
}

fn build_meta(as_of_ctx: Option<&AsOfContext>) -> Value {
    let mut meta = Map::new();
    meta.insert(
        "mode".into(),
        json!(if as_of_ctx.is_some() {
            "replay"
        } else {
            "live"
        }),
    );
    meta.insert(
        "built_at".into(),
        json!(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
    );
    if let Some(ctx) = as_of_ctx {
        meta.insert("as_of_date".into(), json!(ctx.as_string()));
        meta.insert("source".into(), json!(ctx.source.to_string()));
    }
    Value::Object(meta)
}

/// 每轮 token / 缓存汇总（落在 `_meta.cost`）。
///
/// 只记**计量事实**，不折算金额：单价随模型与峰谷档变化，写死一个价格等于在快照里
/// 埋一个必然腐烂的数。查询侧按当时单价乘即可。
///
/// `cache_reported = false` 表示该供应商根本不返回缓存统计（两格为 0 而输入 > 0），
/// 与「返回了但零命中」必须分开 —— 后者命中率是 0，前者是拿不到（判据与
/// `agent_executor` 落 `cache` 两格时的注释同源）。
fn build_cost_summary(results: &HashMap<String, Value>) -> Option<Value> {
    let mut nodes: u64 = 0;
    let mut input: u64 = 0;
    let mut output: u64 = 0;
    let mut hit: u64 = 0;
    let mut miss: u64 = 0;
    for v in results.values() {
        let Some(usage) = v.get("usage") else {
            continue;
        };
        nodes += 1;
        input += usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        output += usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
        hit += v.pointer("/cache/hit_tokens").and_then(Value::as_u64).unwrap_or(0);
        miss += v.pointer("/cache/miss_tokens").and_then(Value::as_u64).unwrap_or(0);
    }
    if nodes == 0 {
        return None;
    }
    let cache_total = hit + miss;
    let hit_pct = if cache_total > 0 {
        (1000.0 * hit as f64 / cache_total as f64).round() / 10.0
    } else {
        0.0
    };
    Some(json!({
        "nodes": nodes,
        "input_tokens": input,
        "output_tokens": output,
        "cache_hit_tokens": hit,
        "cache_miss_tokens": miss,
        "cache_hit_pct": hit_pct,
        "cache_reported": cache_total > 0,
    }))
}

/// 把 `{report, verdict}` 产物重构成前端识别的 `report<!-- VERDICT: {...} -->` 文本。
///
/// `__verdict_only` 是 agent_executor 在「LLM 只回结论标签、没有正文」时打的标记：那句中文占位
/// 陈述是给下游 LLM 读的（「本维度没有分析」），不是分析师/辩手写的正文。快照是呈现数据源 ⇒
/// 丢弃该句、只留标签，并把标记并进 verdict JSON，让卡片按标记渲染本地化的「只有结论」提示。
/// （不能反过来让前端比对那句中文 —— 展示文案当协议值，非中文界面会恒判不命中。）
fn reconstruct_verdict_text(verdict_only: Option<&Value>, report: &str, verdict: &Value) -> String {
    // 2026-10-23（审计 F-3）：标量 verdict（`"看多"`）原样拼进标签会产出**JSON 标量**，
    // 前端 `tryParseVerdictFormat` 拿到的对象里没有 `verdict` 键 ⇒ 结构化通道整体失效，
    // 退化成对整篇文本做关键词扫描（快照路径与实时路径结论可能相反）。
    // 统一包成 `{"verdict": <标量>}`，与 TS 侧 `reconstructVerdictTag` 同形。
    let payload = verdict_tag_payload(verdict);
    if verdict_only != Some(&Value::Bool(true)) {
        return format!("{report}<!-- VERDICT: {payload} -->");
    }
    let mut tagged = payload;
    if let Some(map) = tagged.as_object_mut() {
        map.insert("__verdict_only".into(), Value::Bool(true));
    }
    format!("<!-- VERDICT: {tagged} -->")
}

/// `<!-- VERDICT: … -->` 的载荷：对象原样；标量包成 `{"verdict": <标量>}`。
///
/// 为什么不放行裸标量：`<!-- VERDICT: 看多 -->` 不是合法 JSON；
/// `<!-- VERDICT: "看多" -->` 虽是合法 JSON，但**没有对象字段** —— 前端两条结构化通道
/// 都取不到方向，只能回落到关键词扫描（`classifySentiment` 的显式 verdict 被正文措辞覆盖）。
fn verdict_tag_payload(verdict: &Value) -> Value {
    if verdict.is_object() {
        verdict.clone()
    } else {
        serde_json::json!({ "verdict": verdict })
    }
}

fn extract_node_text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(obj) = v.as_object() {
        for k in ["output", "text", "content", "message", "result", "data"] {
            if let Some(s) = obj.get(k).and_then(|v| v.as_str()) {
                return s.to_string();
            }
        }
        if let Some(nested) = obj.get("output").and_then(|o| o.as_object()) {
            for k in ["text", "content", "message", "result"] {
                if let Some(s) = nested.get(k).and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
    }
    v.to_string()
}

/// `data-quality` 节点在快照里的键（与 [`build_blackboard_snapshot`] 的 `result.{node_id}` 同族）。
const DATA_QUALITY_RESULT_KEY: &str = "result.data-quality";

/// 归一两形态后的 `data-quality` 产物对象；不是对象（或解不出来）时返回 `None`。
///
/// 本模块内**唯一**接触 `DATA_QUALITY_RESULT_KEY` 的地方 —— grade 读法与反馈写侧共用它，
/// 于是「两种形态都要吃」这条纪律只有一份实现（写两份迟早一边漏掉字符串形态，
/// 症状是「有诊断却一行都不写」，与 2026-10-25 查明的那批静默落空同型）。
fn data_quality_object(snapshot: &HashMap<String, Value>) -> Option<Value> {
    let raw = snapshot.get(DATA_QUALITY_RESULT_KEY)?;
    match raw {
        Value::Object(_) => Some(raw.clone()),
        Value::String(s) => serde_json::from_str::<Value>(s).ok().filter(Value::is_object),
        _ => None,
    }
}

/// 从快照里取本轮 `data-quality` 的**完整诊断报告**（`analyst_feedbacks` 写侧的唯一读法）。
///
/// 与 [`data_quality_grade`] 同款容忍，且同样要求顶层带 `grade` —— 没有 `grade` 就说明
/// 这个键装的不是 data-quality 的输出（旧快照 / 别的节点串位），此时宁可返回 `None`
/// 让调用侧记「本轮无诊断」，也不要按半截结构去凑反馈行。
pub fn data_quality_report(snapshot: &HashMap<String, Value>) -> Option<Value> {
    let report = data_quality_object(snapshot)?;
    report.get("grade").and_then(Value::as_str)?;
    Some(report)
}

/// 从 blackboard 快照里取本轮 `data-quality` 的**字母等级**（#8 P5 观测落库的唯一读法）。
///
/// 放在本模块而不是 dao/命令层，是为了让「快照的键形状」只有一个知情人 —— 写入形状在这里，
/// 读法若散到调用侧，改键名就会出现「写侧改了、读侧静默拿 None」。
///
/// 容忍**两种形态**：CodeNode 的 `.result` 通常是对象；走 `extract_node_text` 那条形变时是
/// 一段被再次编码的 JSON 字符串。只认一种就会把「有 grade、形态不同」读成「没跑成」⇒
/// 观测被静默丢掉，而 dao 侧 `grade_is_abnormal(None)` 按异常处理 —— 那会变成误熔断。
///
/// 返回 `None` 的三种成因（节点没跑成 / 快照无该键 / 没有 grade 字段）在本函数里**不区分**，
/// 因为调用侧的处置相同（记一行异常）。要区分的人应先改这里，而不是在调用侧猜。
pub fn data_quality_grade(snapshot: &HashMap<String, Value>) -> Option<String> {
    data_quality_report(snapshot)?
        .get("grade")
        .and_then(|g| g.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_astock_data::as_of::AsOfSource;
    use chrono::NaiveDate;

    #[test]
    fn live_meta_block_has_mode_live_and_no_asof() {
        let bb = build_blackboard_snapshot(&HashMap::new(), None, &[]);
        let meta = bb.get("_meta").expect("must have _meta");
        assert_eq!(meta["mode"], json!("live"));
        assert!(meta.get("as_of_date").is_none());
        assert!(meta["built_at"].is_string());
    }

    #[test]
    fn replay_meta_block_includes_asof_date_and_source() {
        let ctx =
            AsOfContext::new(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(), AsOfSource::UserReplay)
                .unwrap();
        let bb = build_blackboard_snapshot(&HashMap::new(), Some(&ctx), &[]);
        let meta = bb.get("_meta").expect("must have _meta");
        assert_eq!(meta["mode"], json!("replay"));
        assert_eq!(meta["as_of_date"], json!("2026-06-01"));
        assert_eq!(meta["source"], json!("user_replay"));
    }

    #[test]
    fn backtest_sweep_source_serialized() {
        let ctx = AsOfContext::new(
            NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            AsOfSource::BacktestSweep,
        )
        .unwrap();
        let bb = build_blackboard_snapshot(&HashMap::new(), Some(&ctx), &[]);
        assert_eq!(bb["_meta"]["source"], json!("backtest_sweep"));
    }

    #[test]
    fn meta_block_does_not_clobber_existing_node_keys() {
        let mut results = HashMap::new();
        results.insert("a-fundamentals".into(), json!("text"));
        results.insert("trader".into(), json!("trader text"));
        let bb = build_blackboard_snapshot(&results, None, &[]);
        assert_eq!(bb["report.a-fundamentals"], json!("text"));
        assert_eq!(bb["report.investment-plan"], json!("trader text"));
        assert!(bb.contains_key("_meta"));
    }

    #[test]
    fn node_id_with_dash_prefix_uses_report_key() {
        let mut results = HashMap::new();
        results.insert("a-policy".into(), json!("policy text"));
        let bb = build_blackboard_snapshot(&results, None, &[]);
        assert!(bb.contains_key("report.a-policy"));
    }

    #[test]
    fn unknown_node_id_stored_verbatim() {
        let mut results = HashMap::new();
        results.insert("debate-bull-bear".into(), json!("debate"));
        let bb = build_blackboard_snapshot(&results, None, &[]);
        assert_eq!(bb["debate-bull-bear"], json!("debate"));
    }

    #[test]
    fn degraded_block_is_empty_in_live_mode() {
        let bb = build_blackboard_snapshot(&HashMap::new(), None, &[]);
        assert_eq!(bb["degraded"]["count"], json!(0));
        assert_eq!(bb["degraded"]["sources"], json!([]));
    }

    #[test]
    fn degraded_block_records_entries_with_asof_string() {
        let entries = vec![DegradationEntry {
            vendor: "eastmoney".into(),
            method: "get_industry_ranking".into(),
            reason: "as-of 截断后,排名无 N 日前对比语义".into(),
            as_of: "2026-06-01".into(),
            mode: axagent_astock_data::as_of::DEGRADATION_MODE_ASOF.into(),
            // 该维度对回放本就不适用 ⇒ 不进「真故障」档
            kind: axagent_astock_data::as_of::DegradationKind::StructuralGap,
        }];
        let bb = build_blackboard_snapshot(&HashMap::new(), None, &entries);
        assert_eq!(bb["degraded"]["count"], json!(1));
        assert_eq!(bb["degraded"]["sources"][0]["vendor"], json!("eastmoney"));
        assert_eq!(bb["degraded"]["sources"][0]["asOf"], json!("2026-06-01"));
    }

    #[test]
    fn extract_node_text_handles_nested_output() {
        let v = json!({"output": {"text": "hello"}});
        assert_eq!(extract_node_text(&v), "hello");
        let v2 = json!({"text": "x"});
        assert_eq!(extract_node_text(&v2), "x");
        let v3 = json!("raw string");
        assert_eq!(extract_node_text(&v3), "raw string");
    }

    /// 2026-09-28 实证（300285 / a-sector）：LLM 只回 VERDICT 标签、没有正文时，
    /// agent_executor 会往 `report` 里塞一句中文占位陈述（面向下游 LLM）并打 `__verdict_only`。
    /// 快照是呈现数据源 ⇒ 那句不得进快照，标记要并进标签，否则 11 语言界面都显示中文。
    #[test]
    fn verdict_only_placeholder_stays_out_of_snapshot_and_tags_the_marker() {
        let inner = json!({
            "report": "(该节点仅给出结论标签，未提供分析正文)",
            "verdict": {"verdict": "偏空", "bull_score": 30, "bear_score": 70},
            "__verdict_only": true,
        });
        let mut results = HashMap::new();
        results.insert("a-sector".into(), json!({"content": inner.to_string()}));
        let bb = build_blackboard_snapshot(&results, None, &[]);
        let text = bb["report.a-sector"].as_str().expect("a-sector 应为文本");
        assert!(!text.contains("该节点仅给出结论标签"), "占位陈述不得进快照: {text}");
        assert!(text.starts_with("<!-- VERDICT: "), "无正文时应只剩标签: {text}");
        // 按语义比对而非字符串等值：serde_json 的 Map 不保序，键序不是契约
        let meta: serde_json::Value = serde_json::from_str(
            text.trim_start_matches("<!-- VERDICT: ").trim_end_matches(" -->"),
        )
        .expect("标签内必须是合法 JSON");
        assert_eq!(meta["__verdict_only"], json!(true));
        assert_eq!(meta["verdict"], json!("偏空"));
        assert_eq!(meta["bull_score"], json!(30));
    }

    /// 2026-10-23（审计 F-3）：标量 verdict（`"看多"`）必须被包成对象再进标签。
    ///
    /// 病态：原实现产出 `<!-- VERDICT: "看多" -->`（合法 JSON 但无对象字段）；TS 侧同场景
    /// 产出 `<!-- VERDICT: 看多 -->`（**非法 JSON**）⇒ 快照路径与实时路径的解析结果不一致。
    #[test]
    fn scalar_verdict_is_wrapped_into_object_payload() {
        let text = reconstruct_verdict_text(None, "正文", &json!("看多"));
        assert_eq!(text, r#"正文<!-- VERDICT: {"verdict":"看多"} -->"#);
        let meta: serde_json::Value = serde_json::from_str(
            text.trim_start_matches("正文<!-- VERDICT: ").trim_end_matches(" -->"),
        )
        .expect("标签内必须是合法 JSON 对象");
        assert_eq!(meta["verdict"], json!("看多"), "方向必须落在 verdict 键上: {text}");
    }

    /// 反向对照：正常有正文的分析师产物必须原样带出报告，且不被打上标记。
    #[test]
    fn normal_report_keeps_prose_and_has_no_marker() {
        let inner = json!({
            "report": "行业景气度回升，龙头份额集中。",
            "verdict": {"verdict": "偏多"},
        });
        let mut results = HashMap::new();
        results.insert("a-sector".into(), json!({"content": inner.to_string()}));
        let bb = build_blackboard_snapshot(&results, None, &[]);
        let text = bb["report.a-sector"].as_str().expect("a-sector 应为文本");
        assert!(text.starts_with("行业景气度回升，龙头份额集中。"), "{text}");
        assert!(!text.contains("__verdict_only"), "{text}");
    }

    /// #8 P5：快照里 grade 的两种形态都必须读得到 —— 读不到会被当「没跑成」记异常 ⇒ 误熔断。
    #[test]
    fn data_quality_grade_reads_object_and_double_encoded_string() {
        let mut snap: HashMap<String, Value> = HashMap::new();
        snap.insert("result.data-quality".to_string(), json!({ "grade": "B", "score": 71.5 }));
        assert_eq!(data_quality_grade(&snap).as_deref(), Some("B"), "对象形态");

        snap.insert(
            "result.data-quality".to_string(),
            json!("{\"grade\": \"A\", \"score\": 88.0}"),
        );
        assert_eq!(data_quality_grade(&snap).as_deref(), Some("A"), "再次编码成字符串的形态");

        // 读不到就返回 None（不猜等级、不把坏 JSON 折成 "A"）
        snap.insert("result.data-quality".to_string(), json!("{坏 JSON"));
        assert_eq!(data_quality_grade(&snap), None);
        snap.insert("result.data-quality".to_string(), json!({ "score": 1.0 }));
        assert_eq!(data_quality_grade(&snap), None, "缺 grade 字段");
        snap.insert("result.data-quality".to_string(), json!({ "grade": "  " }));
        assert_eq!(data_quality_grade(&snap), None, "空白等级不算 A 也不算某个字母");
        let empty: HashMap<String, Value> = HashMap::new();
        assert_eq!(data_quality_grade(&empty), None);
    }

    /// P3：`_meta.cost` 必须把计量事实原样汇总，并区分「零命中」与「供应商不返回统计」。
    #[test]
    fn meta_cost_aggregates_usage_and_cache() {
        let mut results = HashMap::new();
        results.insert(
            "a-sentiment".to_string(),
            json!({
                "usage": {"input_tokens": 100, "output_tokens": 10},
                "cache": {"hit_tokens": 80, "miss_tokens": 20},
            }),
        );
        results.insert(
            "a-policy".to_string(),
            json!({
                "usage": {"input_tokens": 50, "output_tokens": 5},
                "cache": {"hit_tokens": 0, "miss_tokens": 0},
            }),
        );
        // 无 usage 的节点（Code / 容器）不得计入节点数
        results.insert("portfolio-mgr".to_string(), json!({"result": {"ok": true}}));

        let bb = build_blackboard_snapshot(&results, None, &[]);
        let cost = &bb["_meta"]["cost"];
        assert_eq!(cost["nodes"], json!(2), "只数有 usage 的节点");
        assert_eq!(cost["input_tokens"], json!(150));
        assert_eq!(cost["output_tokens"], json!(15));
        assert_eq!(cost["cache_hit_tokens"], json!(80));
        assert_eq!(cost["cache_miss_tokens"], json!(20));
        assert_eq!(cost["cache_hit_pct"], json!(80.0));
        assert_eq!(cost["cache_reported"], json!(true));

        // 供应商完全不返回缓存统计：input > 0 但两格为 0 ⇒ 必须标「拿不到」而非「零命中」
        let mut only_usage = HashMap::new();
        only_usage.insert(
            "a-research".to_string(),
            json!({"usage": {"input_tokens": 7, "output_tokens": 1}}),
        );
        let bb2 = build_blackboard_snapshot(&only_usage, None, &[]);
        let c2 = &bb2["_meta"]["cost"];
        assert_eq!(c2["cache_reported"], json!(false));
        assert_eq!(c2["cache_hit_pct"], json!(0.0));
        assert_eq!(c2["input_tokens"], json!(7));
    }
}
