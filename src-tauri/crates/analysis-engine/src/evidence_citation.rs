//! 证据引用审计溯源 — 决策理由 → 分析师报告 → 原始数据的引用链
//!
//! ## 设计
//!
//! 每次股票分析完成后，从 `decision_json` 和 `blackboard_snapshot` 中提取
//! 证据引用关系，构建可审计的引用链：
//!
//! ```text
//! 决策: 买入 贵州茅台 (置信度 75%)
//!   ├─ 理由1: ROE 持续高于 20%
//!   │   ├─ 来源: fundamentals-analyst
//!   ���   └─ 数据: 2025年报 ROE=22.3% (eastmoney)
//!   ├─ 理由2: 白酒行业景气度上行
//!   │   ├─ 来源: sector-analyst
//!   │   └─ 数据: 行业营收同比+15% (industry_ranking)
//!   └─ 理由3: MA5 金叉 MA20
//!       ├─ 来源: market-analyst
//!       └─ 数据: 5日均价 1850 > 20日均价 1830 (tencent)
//! ```
//!
//! 引用提取算法：将 `decision_reasoning` 文本按句拆分，与各分析师报告的
//! 关键词做 Jaccard 相似度匹配，找到最可能的来源。

use axagent_harness::domain_semantics::{Percent100, Ratio01};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 文本相似度权重（Jaccard）。
///
/// ⚠️ **两项权重之和必须为 1**，且两项输入都必须先落在 `[0,1]`：
/// `jaccard ∈ [0,1]`；`数字重合数 / 声明去重字符数 ∈ [0,1]`。
/// 这样 `score` 才真在 `[0,1]`，与字段文档、`Ratio01` 类型、前端
/// `matchConfidence * 100` 的口径一致。
///
/// 2026-09-14 修复：第二项原为 `* 30.0`，使实际值域变成 `0–30.7`
/// （`3/20 × 30 = 4.5` 是常见值），而下游全部按 `0–1` 消费 ——
/// 后果是前端条形**恒满格**、颜色**恒绿**、Tooltip 可显示 `450%`。
/// 量纲登记见 `axagent_harness::domain_semantics`（`axinvest.evidence.match_confidence`）。
const W_JACCARD: f64 = 0.7;
const W_NUMBER: f64 = 0.3;

/// 匹配命中阈值（与 `score` 同量纲：`[0,1]`）。
///
/// 修复量纲后本阈值才真正起作用 —— 此前数字项一项就能把 `score` 顶到 4.5，
/// 阈值近乎恒真，`jaccard`（真正的文本相似度）几乎不参与判定。
const MATCH_THRESHOLD: f64 = 0.15;

/// 单条证据引用
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCitation {
    /// 决策中的一句话（理由）
    pub claim: String,
    /// 来源分析师 ID（如 "a-fundamentals", "a-technical"）
    pub source_analyst_id: String,
    /// 来源分析师显示名（如 "基本面分析师"）
    pub source_analyst_name: String,
    /// 匹配置信度，值域 **`[0, 1]`**
    ///
    /// 类型是 [`Ratio01`] 而非 `f64`：量纲由**编译器**归属。
    /// 构造即钳制，序列化仍是裸数字（`serde(transparent)`），
    /// 故前端 `matchConfidence * 100` 的既有契约不变。
    pub match_confidence: Ratio01,
    /// 分析师原文中匹配的片段
    pub source_snippet: String,
    /// 该理由是否在分析师报告中有数据支撑
    pub has_data_support: bool,
    /// 数据来源描述（如 "2025年报 ROE=22.3%"）
    pub data_source: Option<String>,
}

/// 完整引用报告
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationReport {
    pub stock_code: String,
    pub stock_name: String,
    pub analysis_date: String,
    pub decision_action: String,
    pub decision_confidence: f64,
    pub decision_reasoning: String,
    /// 所有证据引用（按说服力降序）
    pub citations: Vec<EvidenceCitation>,
    /// 有数据支撑的理由数
    pub supported_claims: usize,
    /// 总理由数
    pub total_claims: usize,
    /// 支撑率
    pub support_rate: f64,
    /// 参与的分析师数量
    pub analyst_count: usize,
    /// 本轮**决策未产出**（action 为 `Unavailable` 哨兵），故不存在可审计的决策理由。
    ///
    /// 为什么必须是独立标志而不是「`total_claims == 0`」或「`support_rate == 0`」：
    /// 降级路径会把异常诊断串（`portfolio-mgr Rhai 执行异常，已降级为保守决策: …`）
    /// 写进 `decision_reasoning`，那条串与任何分析师正文零重合 ⇒ 匹配必然全 miss
    /// ⇒ 支撑率读出 **0%**。而 0% 在展示层与「理由确有陈述、但没有数据支撑」**同形**
    /// ——「拿不到」被伪装成「查了且查出来是零分」。闸口按 `decision_action` 哨兵判，
    /// 不按 reasoning 文本猜（文本形态会变，哨兵是权威来源）。
    pub decision_degraded: bool,
}

/// 分析师 ID → 中文名的映射
fn analyst_display_name(id: &str) -> String {
    match id {
        "a-fundamentals" | "fundamentals-analyst" => "基本面分析师".into(),
        "a-technical" | "market-analyst" => "技术面分析师".into(),
        "a-sector" | "sector-analyst" => "行业分析师".into(),
        "a-macro" | "policy-analyst" => "宏观分析师".into(),
        "a-sentiment" | "sentiment-analyst" => "情绪分析师".into(),
        "a-news" | "news-analyst" => "新闻分析师".into(),
        "a-hot-money" | "hot-money-tracker" => "热钱追踪".into(),
        "value-investor" => "价值投资者".into(),
        "research-analyst" => "研报分析师".into(),
        "bull-researcher" | "bull-r2" | "bull-r3" => "多头研究员".into(),
        "bear-researcher" | "bear-r2" | "bear-r3" => "空头研究员".into(),
        "research-manager" | "research-mgr" => "研究经理".into(),
        "trader" => "交易员".into(),
        "catalyst-analyst" => "催化剂分析师".into(),
        "lockup-watcher" => "解禁观察".into(),
        "rule-checker" => "规则检查".into(),
        _ => id.to_string(),
    }
}

/// 从决策理由和黑板快照中提取证据引用
///
/// - `decision_reasoning`: 决策中的理由文本（由 trader 节点生成）
/// - `blackboard_snapshot`: 工作流结束时保存的黑板快照 JSON 字符串
/// - `decision_degraded`: 本轮决策未产出（action 为 `Unavailable` 哨兵）。为真时
///   **不做任何匹配**，直接返回带标志的空报告 —— 见 [`CitationReport::decision_degraded`]。
///
/// 返回按匹配置信度降序排列的引用列表。
pub fn extract_citations(
    decision_reasoning: &str,
    blackboard_snapshot: &str,
    decision_degraded: bool,
) -> CitationReport {
    let mut citations = Vec::new();

    if decision_degraded {
        return CitationReport {
            stock_code: String::new(),
            stock_name: String::new(),
            analysis_date: String::new(),
            decision_action: String::new(),
            decision_confidence: 0.0,
            decision_reasoning: decision_reasoning.to_string(),
            citations: Vec::new(),
            supported_claims: 0,
            total_claims: 0,
            support_rate: 0.0,
            analyst_count: 0,
            decision_degraded: true,
        };
    }

    // 1. 解析黑板快照
    let bb: HashMap<String, serde_json::Value> =
        serde_json::from_str(blackboard_snapshot).unwrap_or_default();

    // 2. 提取所有分析师报告（report.* 前缀的键）
    //    同时预抽每份报告的**数字 token 集合**：数字重合必须按 token 判，不能按子串
    //    （见 `text_has_number`，2026-09-28 假高分修复）。
    let mut analyst_reports: Vec<(String, String, std::collections::HashSet<u64>)> = Vec::new();
    for (key, value) in &bb {
        if key.starts_with("report.") {
            let analyst_id = key.strip_prefix("report.").unwrap_or(key).to_string();
            let text = match value {
                serde_json::Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            // f64 不满足 Hash/Eq ⇒ 以 `to_bits()` 入集：同源十进制字面量解析出的
            // 等值浮点，位型必然相同（本模块不产 NaN，extract_numbers 解析失败即丢弃）。
            let nums = extract_numbers(&text).into_iter().map(f64::to_bits).collect();
            analyst_reports.push((analyst_id, text, nums));
        }
    }

    // 3. 按句拆分理由文本
    //    ⚠ 分隔符**不加 `|`**（2026-09-28 实测否决并留档）：portfolio-mgr 的计算轨迹串以
    //    ` | ` 分段，按段拆后 3 条真实重跑行的 claim 由 1 变 5–8，但每段太短，字符集
    //    Jaccard 恒低于 MATCH_THRESHOLD ⇒ 逐段全落「未识别来源」，支撑率从饱和的 100%
    //    变成饱和的 0%（假高分换成假低分，不是改进）。公式轨迹与分析师叙述之间的
    //    引用关系不是文本相似度能量的量 —— 该口径问题另议，不靠调本表判据解决。
    let claims: Vec<&str> = decision_reasoning
        .split(&['。', '！', '？', '\n'][..])
        .map(|s| s.trim())
        .filter(|s| s.len() > 6)
        .collect();

    let mut supported = 0usize;

    for claim in &claims {
        let claim_chars: std::collections::HashSet<char> = claim.chars().collect();
        let claim_len = claim_chars.len() as f64;
        let claim_nums: Vec<f64> = extract_numbers(claim);

        let mut best_match: Option<(String, f64, String)> = None;

        for (analyst_id, report_text, report_nums) in &analyst_reports {
            // 用 Jaccard 相似度做简单匹配
            let report_chars: std::collections::HashSet<char> =
                report_text.chars().take(2000).collect();
            let intersection = claim_chars.intersection(&report_chars).count() as f64;
            let union = claim_chars.union(&report_chars).count() as f64;
            let jaccard = if union > 0.0 {
                intersection / union
            } else {
                0.0
            };

            // 也检查是否包含相同的财务数字模式。
            // ⚠ 必须按**数字 token** 判（`report_nums.contains(n)`），不能用
            //   `report_text.contains(&n.to_string())`：后者让 `0` 命中 `2025`、
            //   `20` 命中 `20.5` ⇒ 计算轨迹串里那些 `仓位=0.0`、`凯利=0.0` 的零值
            //   会在 11/11 份报告里「命中」，支撑率饱和到 100%（2026-09-28 假高分修复）。
            let number_overlap =
                claim_nums.iter().filter(|n| report_nums.contains(&n.to_bits())).count();

            let score =
                jaccard * W_JACCARD + (number_overlap as f64 / claim_len.max(1.0)) * W_NUMBER;

            if score > MATCH_THRESHOLD
                && (best_match.is_none() || score > best_match.as_ref().unwrap().1)
            {
                // 取匹配段（前后 60 字）
                let snippet = extract_snippet(report_text, claim, 60);
                best_match = Some((analyst_id.clone(), score, snippet));
            }
        }

        if let Some((analyst_id, score, snippet)) = best_match {
            let has_data = claim_nums
                .iter()
                .any(|n| analyst_reports.iter().any(|(_, _, nums)| nums.contains(&n.to_bits())));
            if has_data {
                supported += 1;
            }
            citations.push(EvidenceCitation {
                claim: claim.to_string(),
                source_analyst_id: analyst_id.clone(),
                source_analyst_name: analyst_display_name(&analyst_id),
                match_confidence: Ratio01::new(score),
                source_snippet: snippet,
                has_data_support: has_data,
                data_source: if has_data {
                    Some("分析师报告包含匹配数据".into())
                } else {
                    None
                },
            });
        } else {
            // 无匹配 → 标记为"无来源"
            citations.push(EvidenceCitation {
                claim: claim.to_string(),
                source_analyst_id: "unknown".into(),
                source_analyst_name: "未识别来源".into(),
                match_confidence: Ratio01::ZERO,
                source_snippet: String::new(),
                has_data_support: false,
                data_source: None,
            });
        }
    }

    // 按置信度降序
    citations.sort_by(|a, b| {
        b.match_confidence.partial_cmp(&a.match_confidence).unwrap_or(std::cmp::Ordering::Equal)
    });

    let total = citations.len();
    let support_rate = if total > 0 {
        supported as f64 / total as f64
    } else {
        0.0
    };

    // 分析师数量（在 citations 被 move 前计算）
    let analyst_ids: Vec<String> = citations.iter().map(|c| c.source_analyst_id.clone()).collect();
    let unique_count: std::collections::HashSet<&str> =
        analyst_ids.iter().map(|s| s.as_str()).collect();

    CitationReport {
        stock_code: String::new(),
        stock_name: String::new(),
        analysis_date: String::new(),
        decision_action: String::new(),
        decision_confidence: 0.0,
        decision_reasoning: decision_reasoning.to_string(),
        citations,
        supported_claims: supported,
        total_claims: total,
        support_rate,
        analyst_count: unique_count.len(),
        decision_degraded: false,
    }
}

/// 从文本中提取数字
fn extract_numbers(text: &str) -> Vec<f64> {
    let mut nums = Vec::new();
    let mut current = String::new();
    let mut has_dot = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if ch == '.' && !has_dot && !current.is_empty() {
            current.push(ch);
            has_dot = true;
        } else {
            if !current.is_empty() {
                if let Ok(n) = current.parse::<f64>() {
                    nums.push(n);
                }
                current.clear();
                has_dot = false;
            }
        }
    }
    if !current.is_empty() {
        if let Ok(n) = current.parse::<f64>() {
            nums.push(n);
        }
    }
    nums
}

/// 将字节位置向后调整到最近的 UTF-8 字符边界
fn floor_byte_pos(text: &str, byte_pos: usize) -> usize {
    if byte_pos >= text.len() {
        return text.len();
    }
    if text.is_char_boundary(byte_pos) {
        return byte_pos;
    }
    // 向前找到前一个字符的起始位置
    let mut i = byte_pos;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 将字节位置向前调整到最近的 UTF-8 字符边界
fn ceil_byte_pos(text: &str, byte_pos: usize) -> usize {
    if byte_pos >= text.len() {
        return text.len();
    }
    if text.is_char_boundary(byte_pos) {
        return byte_pos;
    }
    for i in byte_pos + 1..=text.len() {
        if text.is_char_boundary(i) {
            return i;
        }
    }
    text.len()
}

/// 安全地在 UTF-8 字符串字节范围切片（自动调整非字符边界的起始/结束位置）
fn safe_slice(text: &str, start_byte: usize, end_byte: usize) -> &str {
    let start = floor_byte_pos(text, start_byte);
    let end = ceil_byte_pos(text, end_byte);
    &text[start.min(end)..end]
}

/// 在原文中定位匹配文本段
fn extract_snippet(text: &str, query: &str, context_chars: usize) -> String {
    // 取 query 前 20 字节（但调整到 UTF-8 字符边界）
    let search_prefix_end = query.len().min(20);
    let search_prefix_end = floor_byte_pos(query, search_prefix_end);
    if let Some(pos) = text.find(&query[..search_prefix_end]) {
        let raw_start = pos.saturating_sub(context_chars);
        let raw_end = (pos + query.len() + context_chars).min(text.len());
        let snippet = safe_slice(text, raw_start, raw_end);
        if raw_start > 0 {
            format!("...{}...", snippet)
        } else {
            format!("{}...", snippet)
        }
    } else {
        // 用重叠词定位
        let words: Vec<&str> =
            query.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).collect();
        for w in words {
            if let Some(pos) = text.find(w) {
                let raw_start = pos.saturating_sub(context_chars);
                let raw_end = (pos + context_chars * 2).min(text.len());
                let snippet = safe_slice(text, raw_start, raw_end);
                return if raw_start > 0 {
                    format!("...{}...", snippet)
                } else {
                    format!("{}...", snippet)
                };
            }
        }
        String::new()
    }
}

/// 将 CitationsReport 渲染为可读的 Markdown 文本
pub fn citations_to_markdown(report: &CitationReport) -> String {
    let mut md = String::new();
    md.push_str("## 证据引用审计\n\n");
    md.push_str(&format!(
        "**决策**: {} (置信度 {:.0}%)\n\n",
        report.decision_action, report.decision_confidence
    ));
    md.push_str(&format!(
        "**数据支撑率**: {:.0}% ({}/{})\n\n",
        report.support_rate * 100.0,
        report.supported_claims,
        report.total_claims
    ));
    md.push_str(&format!("**参与分析师**: {} 个\n\n", report.analyst_count));

    for (i, citation) in report.citations.iter().enumerate() {
        // 档位仍按比率折算（算术与修复前逐位一致，只多了 `.get()`）
        let confidence_bar = match (citation.match_confidence.get() * 10.0) as usize {
            0..=2 => "🟡",
            3..=6 => "🟢",
            _ => "🔵",
        };
        // 展示用百分数走显式换算，单位不再靠读者猜
        let confidence_pct = Percent100::from(citation.match_confidence);
        md.push_str(&format!("{}. {}\n", i + 1, citation.claim));
        md.push_str(&format!(
            "   {} 来源: {} (匹配度 {:.0}%)\n",
            confidence_bar,
            citation.source_analyst_name,
            confidence_pct.get()
        ));
        if citation.has_data_support {
            md.push_str("   📊 有数据支撑\n");
        }
        if !citation.source_snippet.is_empty() {
            md.push_str(&format!("   > {}\n", citation.source_snippet));
        }
        md.push('\n');
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_numbers() {
        let nums = extract_numbers("ROE 22.3%, 营收 150亿, 增长 5.2%");
        assert!(!nums.is_empty());
        assert!(nums.contains(&22.3));
        assert!(nums.contains(&5.2));
    }

    #[test]
    fn test_empty_input() {
        let report = extract_citations("", "{}", false);
        assert_eq!(report.total_claims, 0);
        assert_eq!(report.analyst_count, 0);
    }

    /// 回归（2026-09-28）：**决策未产出**时不得读出「支撑率 0%」。
    ///
    /// 判据用降级路径的**真实形态**自证：`portfolio-mgr.rhai` 的 catch 块把异常诊断串
    /// 写进 `reasoning`（`决策=…` 之外的唯一 reasoning 生产者），而快照里分析师报告齐备。
    /// 走闸口前这条输入会拆出 1 条 claim、与任何中文报告零重合 ⇒ `support_rate = 0.0`
    /// ⇒ 面板显示 0%，与「理由无数据支撑」同形。
    #[test]
    fn degraded_decision_produces_no_zero_percent_reading() {
        let degraded_reasoning = "portfolio-mgr Rhai 执行异常，已降级为保守决策: \
            #{\"error\": \"ErrorVariableNotFound\", \"line\": 2817, \"variable\": \"rel\"}";
        let snapshot = r#"{
            "report.a-fundamentals": "该股ROE连续3年超过20%，盈利能力突出",
            "report.a-sector": "白酒行业整体营收增长15%，景气度较高"
        }"#;

        let report = extract_citations(degraded_reasoning, snapshot, true);
        assert!(report.decision_degraded, "降级输入必须带显式标志");
        assert_eq!(report.total_claims, 0, "降级态不得拆出 claim —— 诊断串不是决策理由");
        assert!(report.citations.is_empty(), "降级态不得产出引用条目");

        // 负对照：同一份输入未经闸口时**确实**产出 0%（证明本闸口拦住的正是该形态）。
        let unguarded = extract_citations(degraded_reasoning, snapshot, false);
        assert!(
            unguarded.total_claims > 0 && unguarded.support_rate == 0.0,
            "负对照失效：未经闸口的降级串本应读出 0%（实际 claims={}, rate={}）—— \
             匹配逻辑或输入形态已变，上面的断言失去区分力",
            unguarded.total_claims,
            unguarded.support_rate
        );
        assert!(!unguarded.decision_degraded, "闸口关闭时标志必须为 false");
    }

    #[test]
    fn test_basic_extraction() {
        let reasoning = "公司ROE持续高于20%,基本面优秀。行业景气度向上。";
        let snapshot = r#"{
            "report.a-fundamentals": "该股ROE连续3年超过20%，盈利能力突出",
            "report.a-sector": "白酒行业整体营收增长15%，景气度较高"
        }"#;
        let report = extract_citations(reasoning, snapshot, false);
        assert!(report.total_claims > 0);
        // 至少有一个理由匹配上了
        let matched = report.citations.iter().filter(|c| c.match_confidence.get() > 0.0).count();
        assert!(matched > 0, "应有至少一个理由匹配到分析师报告");
    }

    /// 回归（2026-09-28 假高分）：数字重合必须按**整个数字 token** 判。
    ///
    /// 修复前的失真（DB 实测：301269 / 300604 / 605376 三条正常重跑行全部读出 100%）：
    /// `report_text.contains(&n.to_string())` 让 `0.0` 渲染成 `"0"` 后命中 `20.0` / `2025`
    /// 这类**包含它**的其它数字 ⇒ 轨迹串里成片的零值（仓位/凯利/成本）恒被判「有数据支撑」。
    #[test]
    fn number_overlap_requires_whole_token() {
        // ① 边界：claim 的数字只有 `0.0`，报告里只有 `20.0` —— 子串判据会误判为重合。
        let reasoning_boundary = "低仓位 0.0 成本";
        let snapshot_boundary = r#"{"report.a-fundamentals":"低仓位 20.0 成本"}"#;
        let r1 = extract_citations(reasoning_boundary, snapshot_boundary, false);
        assert_eq!(r1.total_claims, 1, "该输入应拆出 1 条 claim");
        assert!(
            r1.citations[0].match_confidence.get() > 0.0,
            "文本高度重合，应命中来源（否则本用例测不到支撑判据）"
        );
        assert!(
            !r1.citations[0].has_data_support,
            "`0.0` 不得因报告里的 `20.0` 含字符 0 而被判为有数据支撑（修复前此处恒真）"
        );
        assert_eq!(r1.support_rate, 0.0, "无重合数字时支撑率应为 0，而不是被零值撑满");

        // ② 留档：按 `|` 逐段拆轨迹串的方案已实测否决（见上方 split 注释）——
        //    这里锁住「整串仍是一条 claim」这一既定口径，防止有人再靠拆段去救 100%。
        let trail = "决策=减持 置信=59.1 仓位=0.0% | 先验=0.5(sideways) 后验=0.42 生效后验=0.34";
        let snapshot_trail = r#"{
            "report.investment-plan": "建仓 59.1 万元，分 2 期打满，仓位上限 0.5"
        }"#;
        let r2 = extract_citations(trail, snapshot_trail, false);
        assert_eq!(r2.total_claims, 1, "轨迹串按现口径是 1 条 claim（拆段方案已否决并留档）");
    }

    /// 验证中文字符串不会因 UTF-8 字节切片导致 panic
    #[test]
    fn test_utf8_safe_slicing() {
        // 构造长中文文本 + 长中文 query（>20 字节），trigger 两个临界路径
        let text = "公司基本面稳健，ROE连续多年保持在20%以上，现金流充沛。".repeat(10);
        let query = "基本面稳健ROE连续多年保持在20%以上";
        // 主路径：query > 20 bytes
        let snippet = extract_snippet(&text, query, 30);
        assert!(!snippet.is_empty(), "主路径应能提取非空片段");

        // 回退路径：用重叠词定位（split 后保留多字节汉字）
        let short_text = "催化剂正在催化的效果明显。";
        let short_query = "催化剂";
        let snippet2 = extract_snippet(short_text, short_query, 10);
        assert!(!snippet2.is_empty(), "回退路径应能提取非空片段");

        // 边缘情况：context_chars 导致 start 落在中文字符中间
        let three_byte_text = format!("突破xx{}", "上".repeat(50));
        let snippet3 = extract_snippet(&three_byte_text, "xx", 5);
        assert!(snippet3.contains("x"), "边缘情况应包含查询词的内容");
    }

    /// 回归：`match_confidence` 的量纲必须真在 `[0,1]`（修复前实际可达 `0–30.7`）。
    ///
    /// **为什么不能只断言 `<= 1.0`** —— [`Ratio01`] 构造即钳制，越界会被压到 `1.0`，
    /// 于是「在范围内」恒真、断言等于没写。**量纲回归的可观测症状是「饱和到满值」**：
    /// 本用例下旧公式第二项 = `3/21 × 30 ≈ 4.29` ⇒ `match_confidence` 会正好 `== 1.0`。
    /// 所以判据写成「**命中但未饱和**」。
    #[test]
    fn test_match_confidence_is_ratio_not_saturated() {
        // 3 个数字 + 约 21 个去重字符 ⇒ 正是触发旧公式 `×30.0` 的形态
        let reasoning = "ROE 22.3% 且营收 150 亿且增长 5.2% 超预期。";
        let snapshot = r#"{"report.a-fundamentals":"ROE 22.3 营收 150 增长 5.2 均超预期"}"#;
        let report = extract_citations(reasoning, snapshot, false);

        assert!(report.total_claims > 0, "应至少拆出一句理由");
        let max = report.citations.iter().map(|c| c.match_confidence.get()).fold(0.0_f64, f64::max);

        assert!(max > 0.05, "该输入应能命中，实际最大匹配度 {}", max);
        assert!(
            max < 0.99,
            "最大匹配度 {} 已饱和到满值 —— 打分公式可能又漂回 0–30 量纲（旧值约 4.29）",
            max
        );
        for c in &report.citations {
            assert!(
                (0.0..=1.0).contains(&c.match_confidence.get()),
                "match_confidence 越界：{}（声明值域 0–1）",
                c.match_confidence.get()
            );
        }
    }
}
