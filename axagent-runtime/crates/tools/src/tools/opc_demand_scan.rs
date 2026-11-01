// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现扫描核心 + 需求发现工具节点
//!
//! **单一权威来源**（AGENTS.md 禁区 12）：「装配扫描器 → 并发扫描 → 评估 →
//! 按去重窗口入库 → 回写平台状态」的完整管线只在本模块实现一份，三个消费方：
//!
//! 1. 命令层 `opc_discover_and_evaluate_leads`（手动单关键词扫描）—— 委托本模块
//! 2. 订阅定时扫描 `commands::opc_demand_subscription` —— 经命令层委托本模块
//! 3. 工作流工具节点 `OpcDiscoverLeads`（demand-discovery 模板 Loop 体）—— 直接调用
//!
//! 本模块位于 tools crate 的原因：工作流 ToolNode 经 ToolRegistry 解析工具名，
//! 工具必须注册在 `register_all()`；而 tools crate 已依赖 axagent-dao 与
//! axagent-harness，扫描所需的全部构件（MarketplaceScanner / ScanPolicy /
//! opc_demand repo）均在此层可达，命令层反而是唯一不可达工具注册表的层。

use crate::tools::marketplace_scanner::AggregateMarketplaceScanner;

use crate::tools::demand_llm;
use crate::tools::scan_policy::{SCAN_POLICY_SETTING_KEY, ScanPolicy};
use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolResult};
use axagent_dao::repo::{opc_demand, settings};
use axagent_harness::core_error::AxAgentError;
use axagent_harness::types::{DemandPlatform, DiscoverLeadsSummary};
use sea_orm::DatabaseConnection;
use serde_json::Value;

/// 高价值门槛（与 opportunity_level 的 "high" 档对齐）
pub const HIGH_VALUE_THRESHOLD: f64 = 60.0;
/// 摘要中返回的高价值线索明细上限
pub const SUMMARY_LEADS_LIMIT: usize = 20;

/// 从通用设置表读取扫描策略；缺失或解析失败时返回默认策略
///
/// 命令层的 `opc_get_scan_policy` / `opc_save_scan_policy` / 手动补录共用，
/// 避免策略读取逻辑在命令层与本模块各写一份。
pub async fn load_scan_policy(db: &DatabaseConnection) -> Result<ScanPolicy, AxAgentError> {
    match settings::get_setting(db, SCAN_POLICY_SETTING_KEY).await? {
        Some(json) => Ok(ScanPolicy::from_json(&json)),
        None => Ok(ScanPolicy::default()),
    }
}

/// 评估结果 → DAO 写入行（扫描入库与命令层手动补录共用同一字段映射，避免漂移）
pub fn evaluated_to_row(
    evaluated: &crate::tools::marketplace_scanner::EvaluatedDemandLead,
) -> opc_demand::NewLeadRow {
    let lead = &evaluated.lead;
    let evaluation = &evaluated.evaluation;
    opc_demand::NewLeadRow {
        id: lead.id.clone(),
        platform: lead.platform.clone(),
        title: lead.title.clone(),
        description: lead.description.clone(),
        budget_min: lead.budget_min,
        budget_max: lead.budget_max,
        budget_currency: lead.budget_currency.clone(),
        contact_name: lead.contact_name.clone(),
        contact_email: lead.contact_email.clone(),
        contact_phone: lead.contact_phone.clone(),
        source_url: lead.source_url.clone(),
        content_fingerprint: lead.content_fingerprint.clone(),
        raw_snapshot: lead.raw_snapshot.clone(),
        confidence: evaluation.confidence(),
        pain_score: evaluation.pain_score(),
        market_gap_score: evaluation.market_gap_score(),
        commercial_value_score: evaluation.commercial_value_score(),
        demand_type: evaluation.demand_type().as_str().to_string(),
    }
}

/// 扫描核心：装配扫描器 → 并发扫描 → 评估 → 入库 → 回写平台状态
///
/// 供手动扫描命令、订阅定时扫描与工作流工具节点共用。`platform_filter`
/// 非空时只装配这些平台（订阅可限定平台），为空则装配全部启用平台。
pub async fn run_discovery_scan(
    db: &DatabaseConnection,
    query: &str,
    platform_filter: &[String],
) -> Result<DiscoverLeadsSummary, AxAgentError> {
    let policy = load_scan_policy(db).await?;
    let dedup_window_secs = policy.dedup_window_secs();
    let max_leads = policy.max_leads_per_scan;

    opc_demand::seed_default_platforms_if_empty(db).await?;
    let all_platforms = opc_demand::list_enabled_platforms(db).await?;
    // 订阅限定了平台时只装配这些平台（过滤掉未启用的，避免绕过全局开关）
    let platforms: Vec<DemandPlatform> = if platform_filter.is_empty() {
        all_platforms
    } else {
        all_platforms.into_iter().filter(|p| platform_filter.contains(&p.id)).collect()
    };

    // 装配扫描器：无配置行时回退默认（全部内置扫描器）
    let mut scanner = AggregateMarketplaceScanner::with_policy(policy.clone());
    if platforms.is_empty() {
        let mut default_scanner = AggregateMarketplaceScanner::default();
        default_scanner.set_policy(policy.clone());
        scanner = default_scanner;
    } else {
        for p in &platforms {
            let base_url = p.base_url.as_deref();
            let config = &p.config;
            // DemandPlatform.config 是 Value；add_platform 接受引用。
            // 空配置需绑定到具名变量 —— 直接内联 &Value::Object(..) 是临时值，语句结束即 drop（E0716）。
            let empty_config = serde_json::Value::Object(Default::default());
            let cfg = if config.is_null() {
                &empty_config
            } else {
                config
            };
            scanner.add_platform(&p.id, &p.platform_type, base_url, cfg);
        }
    }

    let results = scanner.scan_and_evaluate_platforms(query).await;
    let mut summary = DiscoverLeadsSummary::default();
    // 逐平台的同步状态：platform → (成功?, 合规跳过?, 失败原因)
    let mut platform_status: Vec<(String, bool, bool, Option<String>)> = Vec::new();
    // 本轮实际评估到的线索 ID（用于回填 round_leads，供订阅按 min_score 推送）
    let mut round_ids: Vec<String> = Vec::new();

    'outer: for result in results {
        platform_status.push((
            result.platform.clone(),
            result.error.is_none(),
            result.compliance_skipped,
            result.error.clone(),
        ));

        if let Some(e) = &result.error {
            tracing::warn!(
                platform = result.platform,
                attempts = result.attempts,
                error = %e,
                "[opc_demand] 平台扫描失败"
            );
        }

        for evaluated in result.leads {
            // max_leads 截断必须终止整轮扫描：旧实现只 break 单平台循环，
            // 后续平台照扫照耗请求配额（P1-5）
            if summary.total_scanned as usize >= max_leads {
                break 'outer;
            }
            summary.total_scanned += 1;
            // 计数口径（P1-5）：Skipped（窗口内重复）不算评估产出 —— 否则
            // total_evaluated ≫ 实际入库量，摘要失真；round_leads 只含真实
            // 入库/刷新的线索，订阅推送也不会把窗口内重复再推一遍。
            let row = evaluated_to_row(&evaluated);
            match opc_demand::upsert_lead_within_window(db, row, dedup_window_secs).await? {
                opc_demand::LeadWriteOutcome::Inserted => {
                    summary.total_saved += 1;
                    summary.total_evaluated += 1;
                    round_ids.push(evaluated.lead.id.clone());
                    if evaluated.value_score() >= HIGH_VALUE_THRESHOLD {
                        summary.high_value_count += 1;
                    }
                },
                opc_demand::LeadWriteOutcome::Refreshed => {
                    summary.total_refreshed += 1;
                    summary.total_evaluated += 1;
                    round_ids.push(evaluated.lead.id.clone());
                    if evaluated.value_score() >= HIGH_VALUE_THRESHOLD {
                        summary.high_value_count += 1;
                    }
                },
                opc_demand::LeadWriteOutcome::Skipped => {},
            }
        }
    }

    // 本轮线索明细（一次查询回填，供订阅扫描按 min_score 过滤推送）
    summary.round_leads = if round_ids.is_empty() {
        Vec::new()
    } else {
        opc_demand::list_leads_by_ids(db, &round_ids).await?
    };

    // LLM 精评（增强，非依赖）：对通过预筛的候选批量重打分并回写 DB。
    // 未配置 bridge / 调用失败 / 解析为空时静默跳过，规则评分兜底。
    // 有精评产出时重读明细，让高价值统计与摘要反映精评后的分数。
    let refined = demand_llm::refine_round_leads(db, &summary.round_leads, &policy).await;
    if refined > 0 && !round_ids.is_empty() {
        summary.round_leads = opc_demand::list_leads_by_ids(db, &round_ids).await?;
        summary.high_value_count = summary
            .round_leads
            .iter()
            .filter(|l| l.commercial_value_score >= HIGH_VALUE_THRESHOLD)
            .count() as u32;
    }

    // 高价值明细（P1-6 语义修正）：旧实现回填**全局历史**高价值榜（全表 ≥60
    // 分查询），本轮 0 命中时摘要也会显示一堆历史线索，误导"本轮扫描很成功"。
    // 现在直接从本轮 round_leads 过滤，口径与 high_value_count 一致。
    summary.leads = summary
        .round_leads
        .iter()
        .filter(|l| l.commercial_value_score >= HIGH_VALUE_THRESHOLD)
        .take(SUMMARY_LEADS_LIMIT)
        .cloned()
        .collect();

    // 回写平台同步状态（单平台失败不阻断整体结果）
    // 失败原因持久化到 last_error，供前端平台配置页查看异常原因；
    // 合规跳过单独置 skipped —— 它是配置状态，伪装成 ok 会掩盖真实情况。
    for (platform_id, ok, compliance_skipped, error) in &platform_status {
        let outcome = if *compliance_skipped {
            opc_demand::PlatformScanOutcome::Skipped
        } else if *ok {
            opc_demand::PlatformScanOutcome::Ok
        } else {
            opc_demand::PlatformScanOutcome::Error(error.clone())
        };
        if let Err(e) = opc_demand::mark_platform_result(db, platform_id, outcome).await {
            tracing::warn!(platform = platform_id, error = %e, "[opc_demand] 更新同步状态失败");
        }
    }

    tracing::info!(
        scanned = summary.total_scanned,
        saved = summary.total_saved,
        refreshed = summary.total_refreshed,
        high_value = summary.high_value_count,
        "[opc_demand] 扫描评估完成"
    );
    Ok(summary)
}

// ═══════════════════════════════════════════════════════════════════
// 工作流工具节点：OpcDiscoverLeads
// ═══════════════════════════════════════════════════════════════════

/// 按关键词扫描全部启用平台并评估入库（demand-discovery 工作流模板 Loop 体）
///
/// 与命令层 `opc_discover_and_evaluate_leads` 完全同一条管线（见模块注释），
/// 只是入口从 Tauri 命令换成 ToolRegistry 工具 —— Loop 逐关键词迭代调用。
pub struct OpcDiscoverLeadsTool;

#[async_trait::async_trait]
impl Tool for OpcDiscoverLeadsTool {
    fn name(&self) -> &str {
        "OpcDiscoverLeads"
    }

    fn description(&self) -> &str {
        "按关键词扫描已启用的需求平台（闲鱼/猪八戒/Reddit 等），评估商业价值并去重入库。\
         返回扫描摘要（scanned/saved/high_value 等）。与 OPC 需求发现页的手动扫描为同一条管线。"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "搜索关键词（不能为空）"
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Integration
    }

    fn domain(&self) -> crate::ToolDomain {
        crate::ToolDomain::Automation
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::execution_failed("query 不能为空".to_string()))?
            .to_string();

        let db = crate::global_state::get_sea_db()
            .ok_or_else(|| ToolError::execution_failed("OPC 数据库未初始化".to_string()))?;

        let summary = run_discovery_scan(&db, &query, &[])
            .await
            .map_err(|e| ToolError::execution_failed(format!("需求扫描失败: {e}")))?;

        // 摘要序列化为 JSON 字符串作为工具结果（下游节点 / 投递文本可直接消费）
        let content = serde_json::to_string(&summary)
            .map_err(|e| ToolError::execution_failed(format!("序列化扫描摘要失败: {e}")))?;
        Ok(ToolResult::success(content))
    }
}
