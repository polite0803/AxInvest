// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求能力匹配（v133，需求全链路审计 P3）
//!
//! 需求「响应」环节的回答机器：给定一条线索，用现有能力库（CapabilityPassport，
//! 100+ 实现，混合检索 = 向量 + BM25 + 标签）判定**能不能接、缺什么**。
//!
//! - `opc_match_lead_capabilities`：单条线索的能力匹配结论
//!
//! ## 判定口径
//!
//! - **ready**：最高检索分 ≥ 0.65 且必需能力域全覆盖 → 现有能力直接能接
//! - **partial**：最高检索分 ≥ 0.40，或分数够但有必需域未覆盖 → 能接一部分
//! - **missing**：最高检索分 < 0.40 → 能力库基本没有对应能力，需要补齐
//!
//! 分数门槛是经验值，与 `CapabilityRetriever` 的综合分口径
//! （`semantic*0.6 + keyword*0.2 + tag*0.2`）配套，调整时两个文件一起看。
//!
//! ## 为什么「缺什么」用能力域而不是让 LLM 现编
//!
//! LLM 自由生成缺口描述会编造不存在的工具名（与 chain-decomposer 曾出现的问题同源）。
//! 这里用**需求类型 → 必需能力域**的静态映射 + 检索命中的域做差集，结论确定、
//! 可单测、可解释；`gap_hint` 只做描述性拼接，不发明新能力。

use crate::AppState;
use axagent_agent_macro::agent_command;
use axagent_harness::capability::CapabilityDomain;
use axagent_harness::capability_retriever::CapabilityQuery;
use axagent_harness::types::{CapabilityMatchItem, DemandLeadDto, LeadCapabilityMatch};
use axagent_harness::util_fns::truncate_to_char_boundary;
use axagent_tools::tools::marketplace_scanner::DemandType;
use tauri::State;

/// 判定为「可直接接」的综合分门槛
const READY_SCORE: f64 = 0.65;
/// 判定为「部分覆盖」的综合分门槛
const PARTIAL_SCORE: f64 = 0.40;
/// 检索默认召回条数
const DEFAULT_TOP_K: usize = 8;
/// 检索召回条数上限（避免大 top_k 拖慢混合检索）
const MAX_TOP_K: usize = 20;
/// 参与匹配的需求文本上限（字节，UTF-8 安全截断）
const MATCH_TEXT_BYTES: usize = 1200;

/// 单条线索的能力匹配
///
/// 从能力库检索与该需求最匹配的能力，输出「能接/部分/缺失」结论与缺失的能力域。
/// 检索失败不报错 —— 能力库不可用时应降级为「未知」（missing），而不是让「响应」
/// 环节整体崩掉；错误只记日志。
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "为需求匹配能力")]
#[tauri::command]
pub async fn opc_match_lead_capabilities(
    state: State<'_, AppState>,
    lead_id: String,
    top_k: Option<usize>,
) -> Result<LeadCapabilityMatch, String> {
    let lead =
        axagent_dao::repo::opc_demand::get_lead(state.harness.db(), &lead_id).await.map_err(err)?;

    // 复用 DemandType 枚举（marketplace_scanner 落库同源），识别不了的类型不猜缺口
    let demand_type: DemandType = lead.demand_type.parse().unwrap_or(DemandType::Unknown);
    let required: Vec<String> =
        required_domains_for(&demand_type).iter().map(|d| (*d).to_string()).collect();
    let query_text = build_match_text(&lead);

    let query = CapabilityQuery {
        user_input: query_text,
        top_k: top_k.unwrap_or(DEFAULT_TOP_K).clamp(1, MAX_TOP_K),
        ..Default::default()
    };

    let candidates = match state.capability_router.retriever.retrieve(&query).await {
        Ok(result) => result.candidates,
        Err(e) => {
            tracing::warn!(
                lead_id = %lead_id,
                error = %e,
                "[opc_demand] 能力检索失败，按无命中降级"
            );
            Vec::new()
        },
    };

    // 系统域是编排器内部能力，不应对外暴露；负面命中的能力直接剔除
    let mut matches: Vec<CapabilityMatchItem> = candidates
        .into_iter()
        .filter(|c| !c.negative_hit && c.domain != CapabilityDomain::System)
        .map(|c| CapabilityMatchItem {
            capability_id: c.capability_id,
            name: c.name,
            kind: c.kind.as_str().to_string(),
            domain: c.domain.as_str().to_string(),
            retrieval_score: c.retrieval_score,
            summary: c.passport.summary.clone(),
        })
        .collect();
    // 检索结果已按分降序，这里再兜一次（过滤后顺序不变，保险起见）
    matches.sort_by(|a, b| {
        b.retrieval_score.partial_cmp(&a.retrieval_score).unwrap_or(std::cmp::Ordering::Equal)
    });

    let best_score = matches.first().map(|m| m.retrieval_score).unwrap_or(0.0);
    let covered: Vec<&str> = matches.iter().map(|m| m.domain.as_str()).collect();
    let missing_domains = missing_domains(&required, &covered);

    let verdict = verdict_of(best_score, !missing_domains.is_empty());
    let gap_hint = if missing_domains.is_empty() {
        None
    } else {
        Some(format!(
            "需求类型 {} 需要以下能力域，当前能力库未覆盖：{}",
            lead.demand_type,
            missing_domains.join("、")
        ))
    };

    tracing::info!(
        lead_id = %lead_id,
        demand_type = %lead.demand_type,
        best_score,
        verdict,
        matched = matches.len(),
        "[opc_demand] 能力匹配完成"
    );

    Ok(LeadCapabilityMatch {
        lead_id,
        verdict: verdict.to_string(),
        best_score,
        matches,
        required_domains: required,
        missing_domains,
        gap_hint,
    })
}

/// 构造参与匹配的文本：标题 + 需求类型 + 描述
///
/// 需求类型单独拼一行：它是 DB 里的 snake_case 标识，与能力域名的语义相近，
/// 能提高关键词（BM25）侧的召回。
fn build_match_text(lead: &DemandLeadDto) -> String {
    let desc = truncate_to_char_boundary(&lead.description, MATCH_TEXT_BYTES);
    format!("{}\n需求类型: {}\n{}", lead.title, lead.demand_type, desc)
}

/// 需求类型 → 必需能力域（`CapabilityDomain::as_str()`）
///
/// 接收 `DemandType` 枚举而非字符串：match 必须穷尽所有 variant，
/// 新增需求类型时编译器会强制补映射，避免静默返回空导致「缺什么」永远为真。
/// `Unknown` 返回空 —— 不做无依据的猜测，避免把「不知道」报成「缺能力」。
fn required_domains_for(demand_type: &DemandType) -> Vec<&'static str> {
    match demand_type {
        DemandType::ToolSoftware => vec!["general", "automation"],
        DemandType::ContentCreation => vec!["content_creation", "ai_media"],
        DemandType::Design => vec!["content_creation", "ai_media"],
        DemandType::Development => vec!["devops", "general"],
        DemandType::Operations => vec!["automation", "devops"],
        DemandType::Marketing => vec!["content_creation", "communication"],
        DemandType::Education => vec!["content_creation"],
        DemandType::EnterpriseService => vec!["automation", "data_analysis"],
        DemandType::Outsourcing => vec!["general", "automation"],
        DemandType::Consulting => vec!["data_analysis", "general"],
        DemandType::Unknown => Vec::new(),
    }
}

/// 必需域中未被命中的部分（保留 required 的顺序，便于前端稳定展示）
fn missing_domains(required: &[String], covered: &[&str]) -> Vec<String> {
    required.iter().filter(|d| !covered.contains(&d.as_str())).cloned().collect()
}

/// 判定结论：分数门槛 + 必需域覆盖度
///
/// 有必需域未覆盖时最高只能判 `partial` —— 分数高但关键域缺失，
/// 直接报 ready 会让「响应」环节误判为可直接接单。
fn verdict_of(best_score: f64, has_missing: bool) -> &'static str {
    if best_score >= READY_SCORE && !has_missing {
        "ready"
    } else if best_score >= PARTIAL_SCORE || (!has_missing && best_score > 0.0) {
        "partial"
    } else {
        "missing"
    }
}

/// DAO 错误 → 命令层错误串（走错误码映射层）
fn err(e: axagent_harness::core_error::AxAgentError) -> String {
    String::from(crate::commands::error::ErrorResponse::from_error(
        e,
        crate::commands::error::ErrorCategory::Unrecoverable,
    ))
}

#[cfg(test)]
mod tests {
    use super::{missing_domains, required_domains_for, verdict_of};
    use axagent_tools::tools::marketplace_scanner::DemandType;

    /// 通过 FromStr 解析，与 DB 落库的 snake_case 字符串保持同源
    fn dt(s: &str) -> DemandType {
        s.parse().unwrap_or(DemandType::Unknown)
    }

    #[test]
    fn required_domains_cover_known_demand_types() {
        assert_eq!(required_domains_for(&dt("tool_software")), vec!["general", "automation"]);
        assert_eq!(
            required_domains_for(&dt("content_creation")),
            vec!["content_creation", "ai_media"]
        );
        assert_eq!(required_domains_for(&dt("development")), vec!["devops", "general"]);
        // 未知类型不猜
        assert!(required_domains_for(&DemandType::Unknown).is_empty());
        assert!(required_domains_for(&dt("nonexistent_type")).is_empty());
    }

    #[test]
    fn required_domains_only_use_legal_capability_domains() {
        // route_path 铁律同款风险：这里出现拼错的域名会让「缺失」永远为真
        const LEGAL: [&str; 8] = [
            "general",
            "devops",
            "ai_media",
            "data_analysis",
            "content_creation",
            "communication",
            "finance",
            "automation",
        ];
        for d_t in [
            DemandType::ToolSoftware,
            DemandType::ContentCreation,
            DemandType::Design,
            DemandType::Development,
            DemandType::Operations,
            DemandType::Marketing,
            DemandType::Education,
            DemandType::EnterpriseService,
            DemandType::Outsourcing,
            DemandType::Consulting,
        ] {
            for d in required_domains_for(&d_t) {
                assert!(LEGAL.contains(&d), "需求类型 {} 映射出非法能力域: {d}", d_t.as_str());
            }
        }
    }

    #[test]
    fn missing_domains_is_set_difference_preserving_order() {
        let required = vec!["general".to_string(), "automation".to_string()];
        assert_eq!(missing_domains(&required, &["general", "automation"]), Vec::<String>::new());
        assert_eq!(missing_domains(&required, &["general"]), vec!["automation".to_string()]);
        assert_eq!(
            missing_domains(&required, &[]),
            vec!["general".to_string(), "automation".to_string()]
        );
        // 命中不在 required 里的域不影响差集
        assert_eq!(
            missing_domains(&required, &["finance", "general"]),
            vec!["automation".to_string()]
        );
    }

    #[test]
    fn verdict_thresholds() {
        // 高分且全覆盖 → ready
        assert_eq!(verdict_of(0.9, false), "ready");
        assert_eq!(verdict_of(0.65, false), "ready");
        // 高分但有必需域缺口 → 只能 partial
        assert_eq!(verdict_of(0.9, true), "partial");
        // 中等分 → partial
        assert_eq!(verdict_of(0.5, false), "partial");
        assert_eq!(verdict_of(0.4, false), "partial");
        // 低分但必需域全覆盖（有真实命中）→ 仍是 partial：能力库能覆盖该类型，只是匹配弱
        assert_eq!(verdict_of(0.39, false), "partial");
        // 完全无命中 → missing
        assert_eq!(verdict_of(0.0, false), "missing");
        // 低分且有必需域缺口 → missing（分数不够也没有覆盖）
        assert_eq!(verdict_of(0.39, true), "missing");
        // 无检索命中（0 分）但有 required 缺口 → missing
        assert_eq!(verdict_of(0.0, true), "missing");
    }
}
