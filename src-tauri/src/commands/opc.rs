// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 业务领域 Tauri 命令层
//!
//! 将 opc-dao 的 Service 暴露给前端 React 调用。

use axagent_agent_macro::agent_command;

use crate::AppState;
use sea_orm::ActiveModelTrait;
use tauri::State;

use axagent_company_runtime::opc::{ContentAsset, ContentAssetService, DefaultSiteService};

use axagent_company_runtime::opc::capability_pack::{
    CAPABILITY_PACKS_DIR, CapabilityMatchPredicate, CapabilityPackManifest, export_capability_pack,
    import_capability_pack, list_capability_pack_enabled_map, resolve_capability_packs_dir,
    set_capability_pack_enabled,
};

// ── Invoice / Customer Commands（本段六条已随 #264 批 7 刀 B2 删除）──────────
//
// `opc_get_invoice` / `opc_update_invoice` / `opc_get_customer` /
// `opc_find_customer_by_email` / `opc_get_investment_advice` 与同批的
// `opc_record_kpi` / `opc_list_revenue` / `opc_import_talent_library` / `opc_sync_fleet` /
// `opc_get_publish_schedule` / `opc_update_publish_schedule` 十一条宿主命令已删：
// 插件 `axagent-plugins/crates/plugin-opc` 逐条声明同名命令并走 `opc.store` 接缝
// （宿主实现 `src-tauri/src/init/opc_store_service.rs` 的 `HostOpcStore`）。
// 删前经穷举取证：前端 0 处 `invoke`、`axagent-runtime/crates/tools` 0 处直连
// （工具面走 `call_opc_command` 插件命令桥，fail-closed 不回落直连）、宿主内部 0 处转调。

// ── Project Commands ──────────────────────────────────────────────
//
// #32 批 B：`opc_create_project` / `opc_list_projects` / `opc_add_milestone` 三条已删 ——
// 前端 `ProjectsTab` 与智能体面都走插件命令 `opc_create_project` / `opc_list_projects` /
// `opc_add_milestone`（`axagent-plugins/crates/plugin-opc`）。⚠ 删 `opc_create_project` 同时
// 带走了本文件唯一的轨迹记录点（`record_opc_success`）：插件进程不持 trajectory 存储，
// 该记录在插件通道上不可达；要恢复须在宿主分派层另开记录位，不在本刀范围。

// ── Project Commands（本段四条已随 #266 删除）────────────────────────
//
// `opc_get_project` / `opc_update_project` / `opc_delete_project` /
// `opc_complete_milestone` 四条宿主命令已删：插件 `axagent-plugins/crates/plugin-opc`
// 逐条声明同名命令并走 `opc.store` 接缝（`#32` 批 B 实现），本文件侧经穷举取证确认
// 零消费 —— 前端 `src/**` 0 处、`axagent-runtime/crates/tools` 0 处（tools 侧改走
// `call_opc_command` → 插件命令桥，fail-closed 不回落直连）、`agency_experts` 0 处、
// 宿主内部 0 处转调。

// ── Site / Landing Page Commands ────────────────────────────────────
//
// #36 D-4 刀：落地页 / 联系人的六条宿主命令（`opc_create_landing_page` /
// `opc_list_landing_pages` / `opc_publish_landing_page` / `opc_delete_landing_page` /
// `opc_list_contacts` / `opc_mark_contact_read`）已删除 —— 接缝补齐 `publish_landing_page` /
// `delete_landing_page` / `mark_contact_read` 三条写 op 后，前端经
// `opc_list_landing_pages` / `opc_list_contacts` / `opc_create_landing_page` 等插件命令
// 走 worker 通道（先例 #144 的双声明收口：宿主侧随接缝补齐同批删除）。

// ── Analytics Commands（本段两条已随 #264 批 7 刀 B2 删除）──────────────
//
// `opc_record_kpi` / `opc_list_revenue` 两条宿主命令已删：插件
// `axagent-plugins/crates/plugin-opc` 声明同名命令，读 / 写分别走 `opc.store` 的
// `record_kpi`（写）与 `list_revenue`（读）两条 op。`domain_pack_id` 的必填与
// 「空值 / 空白即拒」两条判据原样留在域服务（`CreateKpiInput` 无 `#[serde(default)]` +
// `opc::analytics::canonical_domain_pack_id`），接缝层不复制。

// `opc_list_kpis` 已随 #266 删除：插件侧同名命令走 `opc.store` 的 KPI 列表读 op
// （`axagent-plugins/crates/plugin-opc`），宿主本文件侧零消费；`axagent-runtime/crates/tools`
// 的 KPI 工具改走 `call_opc_command` 插件命令桥（fail-closed，不回落直连）。
// 域包范围内的 KPI 仍走 `opc_get_capability_pack_dashboard` 链路，不受本条删除影响。

// `opc_get_dashboard_summary` 已随 #267 删除：插件侧同名命令走 `opc.store` 的
// `dashboard_summary` 读 op（宿主按整表聚合，见 `init/opc_store_service.rs`），宿主侧
// 唯一消费方 `DashboardTab.tsx` 已切 `invokePluginCommand`。域包范围内的摘要仍走
// `opc_get_capability_pack_dashboard`，不受本条删除影响。

// ── Finance Commands（本段一条已随 #264 批 7 刀 B2 删除）────────────────
//
// `opc_get_financial_report` 已随 #266 删除：插件侧同名命令走 `opc.store` 只读接缝
// （`axagent-plugins/crates/plugin-opc`），宿主侧零消费（`axagent-runtime/crates/tools`
// 的财务报表工具走 `call_opc_command` 插件命令桥）。
// `opc_get_investment_advice` 已随 #264 批 7 刀 B2 删除：插件侧同名命令走 `opc.store` 的
// `get_investment_advice` 读 op（「先出报表、再据报表出建议」两步仍在一处，见
// `init/opc_store_service.rs` 的同名方法）。

// ── Domain Pack .opcip 导出/导入 ──────────────────────────────

/// 导出域包为 .opcip 归档（打包 manifest + workflows）。
/// out_dir 为前端选择的保存目录（通过对话框）。
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "导出域包")]
#[tauri::command]
pub async fn opc_export_capability_pack(
    state: State<'_, AppState>,
    id: String,
    out_dir: String,
) -> Result<String, String> {
    // 包源：app_dir/config/opc/domain_packs（生产）或仓库根（开发）
    let app_dir = &state.app_data_dir;
    let base = resolve_capability_packs_dir(Some(app_dir));
    let out_path = std::path::PathBuf::from(&out_dir);
    export_capability_pack(&base, &id, &out_path).await
}

/// 导入 .opcip 域包：解包到 app_dir/config/opc/domain_packs/ 并注册 seed。
/// archive_path 为前端选择的 .opcip 文件路径。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "导入域包")]
#[tauri::command]
pub async fn opc_import_capability_pack(
    state: State<'_, AppState>,
    archive_path: String,
) -> Result<String, String> {
    let app_dir = &state.app_data_dir;
    let archive = std::path::PathBuf::from(&archive_path);
    let result = import_capability_pack(&state.harness.db().clone(), app_dir, &archive).await;
    let id = match result {
        Ok(id) => id,
        Err(e) => return Err(e),
    };
    // 期一·3 能力封闭校验：导入成功后全量对账一次（非阻塞审计，坏只 warning）。
    // 期一·4：把刚导入域包的审计摘要（missing / domain_proposals）随成功信息带回前端展示。
    let base = state.pack_registry.resolve_packs_dir(Some(app_dir));
    crate::init::state::audit_and_log_capability_packs_capability(
        state.pack_registry.as_ref(),
        &*state.capability_indexer,
        &base,
    )
    .await;
    let reports =
        state.pack_registry.audit_capability_closure(&*state.capability_indexer, &base).await;
    if let Some(report) = reports.into_iter().find(|r| r.domain_pack_id == id) {
        if !report.closed {
            let missing_detail = report
                .missing
                .iter()
                .map(|m| format!("{}:{}", m.source.as_str(), m.capability_type))
                .collect::<Vec<_>>()
                .join("、");
            let proposal_detail = report
                .domain_proposals
                .iter()
                .map(|p| format!("{}（{}）", p.domain_id, p.suggestion))
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(format!(
                "域包 {id} 能力闭合校验未通过：缺失 {} 项（{}）{}",
                report.missing.len(),
                missing_detail,
                if proposal_detail.is_empty() {
                    String::new()
                } else {
                    format!("；建议增加域：{proposal_detail}")
                },
            ));
        }
    }
    Ok(format!("{id}：能力闭合校验通过"))
}

/// 启用/停用域包 —— 写 DB `opc_capability_packs.enabled`（权威存储，重启后生效）。
/// pack_id 为域包 id（manifest.id）；enabled 为布尔开关。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "启用停用域包")]
#[tauri::command]
pub async fn opc_set_capability_pack_enabled(
    state: State<'_, AppState>,
    pack_id: String,
    enabled: bool,
) -> Result<(), String> {
    let db = state.harness.db();
    set_capability_pack_enabled(db, &pack_id, enabled).await
}

// ── Company Runtime：看板投影 + 阻塞升级链（P3-3/4）──────────────

/// 创建工作项（修复 P0-2：此前 opc_work_items 无生产创建入口，看板/自改进/质量门全无数据源）。
/// id 强制 `wi-` 前缀（与 self_improving::OpcWorkItemRound 的 task 契约一致）。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "创建工作项")]
#[tauri::command]
pub async fn opc_create_work_item(
    state: State<'_, AppState>,
    title: String,
    owner_role_id: Option<String>,
) -> Result<serde_json::Value, String> {
    use axagent_company_runtime::WorkItemService;
    use axagent_company_runtime::work_item_service::NewWorkItem;

    if title.trim().is_empty() {
        return Err("工作项标题不能为空".to_string());
    }
    let db = state.harness.db().clone();
    let svc = WorkItemService::new(&db);
    let id = format!("wi-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let mut input = NewWorkItem::new(id, title.trim().to_string());
    if let Some(owner) = owner_role_id.filter(|s| !s.trim().is_empty()) {
        input = input.owner(owner.trim().to_string());
    }
    let model = svc.create(input).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    serde_json::to_value(&model).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

// ── Content Asset Commands ──────────────────────────────────────────

#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取内容资产详情")]
#[tauri::command]
pub async fn opc_get_content_asset(
    state: State<'_, AppState>,
    id: String,
) -> Result<ContentAsset, String> {
    let svc = DefaultSiteService::new(state.harness.db().clone());
    svc.get_content_asset(&id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 看板投影：按 phase 列聚合 work items（Kanban）。
/// 返回 {列名: [item...]}，列为 待办/进行中/阻塞/评审/已完成/终止。
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取看板块")]
#[tauri::command]
pub async fn opc_kanban_board(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    use axagent_company_runtime::WorkItemService;
    use axagent_company_runtime::work_item::Phase;
    use std::collections::BTreeMap;

    let db = state.harness.db().clone();
    let svc = WorkItemService::new(&db);
    let items = svc.list_by_phase(None).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let mut board: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for it in &items {
        let phase = it.phase.parse::<Phase>().unwrap_or(Phase::Queued);
        let col = phase.kanban_column().to_string();
        let entry = serde_json::json!({
            "id": it.id,
            "title": it.title,
            "phase": it.phase,
            "owner_role_id": it.owner_role_id,
            "assignee_agent_id": it.assignee_agent_id,
            "manager_role_id": it.manager_role_id,
            "last_error": it.last_error,
            "deps": serde_json::from_str::<Vec<String>>(&it.deps_json).unwrap_or_default(),
            "created_at": it.created_at,
            "updated_at": it.updated_at,
        });
        board.entry(col).or_default().push(entry);
    }
    serde_json::to_value(board).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 认领 work item（Start）。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "认领工作项")]
#[tauri::command]
pub async fn opc_work_item_start(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    use axagent_company_runtime::WorkItemService;
    let db = state.harness.db().clone();
    let svc = WorkItemService::new(&db);
    let model = svc.start(&id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    serde_json::to_value(&model).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 提交评审（质量门前置，方案 A）：先跑一轮自改进评估，质量达标才允许进 REVIEW。
///
/// 流程：执行一轮 OpcWorkItemRound → 5 维规则评估 → 评估经 QualityGateService
/// 落经验（归因+信号）→ score >= 0.80 才 apply(SubmitForReview)；未达标返回
/// 缺口清单（前端展示原因），产出无法进入评审流。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "提交工作项评审")]
#[tauri::command]
pub async fn opc_work_item_review(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    use axagent_company_runtime::work_item::Transition;
    use axagent_company_runtime::{OpcWorkItemRound, QualityGateService, WorkItemService};
    use axagent_harness::self_improving_loop::SelfImprovingRound;

    // P1-3：准入线 0.70——允许无历史经验（无 playbook）的首次执行也进入评审；
    // 收敛判定（decide_next 0.85 Accept）语义不同，保留在自改进循环内。
    const QUALITY_THRESHOLD: f64 = 0.70;

    let db = state.harness.db().clone();
    let svc = WorkItemService::new(&db);

    // 1. 加载 work item（拿 owner 做归因）
    let item = svc.get(&id).await.map_err(|e| format!("加载 work item 失败: {e}"))?;

    // 2. 质量门前置：跑一轮自改进评估
    let mut round = OpcWorkItemRound::new(db.clone());
    let result =
        round.execute_round(&id, None).await.map_err(|e| format!("自改进评估失败: {e}"))?;
    let evaluation =
        round.evaluate_round(&id, &result).await.map_err(|e| format!("自改进评估失败: {e}"))?;

    // 3. 评估落经验（归因铁律：评估者=owner，写入 owner 档案）
    if let Some(owner) = &item.owner_role_id {
        if !owner.is_empty() {
            let gate = QualityGateService::new(&db);
            let _ = gate.apply(owner, &id, owner, &evaluation, QUALITY_THRESHOLD).await;
        }
    }

    // 4. 质量门判定
    if evaluation.score < QUALITY_THRESHOLD {
        let pct = evaluation.score * 100.0;
        return Err(format!(
            "质量门未通过（{pct:.0}% < {}%）。缺口：{}",
            QUALITY_THRESHOLD * 100.0,
            if evaluation.gaps.is_empty() {
                "综合质量不足".to_string()
            } else {
                evaluation.gaps.join("；")
            }
        ));
    }

    // 5. 达标 → 进入 REVIEW
    let model = svc.apply(&id, Transition::SubmitForReview).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    serde_json::to_value(&model).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 阻塞升级链：置 BLOCKED + 记录 last_error（原因），通知 manager。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "升级阻塞工作项")]
#[tauri::command]
pub async fn opc_escalate_work_item(
    state: State<'_, AppState>,
    id: String,
    reason: String,
) -> Result<serde_json::Value, String> {
    use axagent_company_runtime::WorkItemService;
    use axagent_company_runtime::work_item::{Phase, Transition};
    let db = state.harness.db().clone();
    let svc = WorkItemService::new(&db);

    // 1. 状态机置 BLOCKED
    let model = svc.apply(&id, Transition::Block).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // 2. 记录 last_error（升级原因）
    let mut am: axagent_entities::opc_work_items::ActiveModel = model.clone().into();
    am.last_error = sea_orm::Set(Some(reason.clone()));
    am.phase = sea_orm::Set(Phase::Blocked.as_str().to_string());
    am.updated_at = sea_orm::Set(chrono::Utc::now().timestamp());
    let updated = am.update(&db).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // 3. 通知 manager（rt-messaging 渠道；无 manager 则跳过并记日志）
    if let Some(mgr) = &updated.manager_role_id {
        tracing::info!("[opc-escalate] {} 升级给 {}: {}", updated.id, mgr, reason);
    } else {
        tracing::warn!("[opc-escalate] {} 无 manager_role_id，升级仅记录", updated.id);
    }

    serde_json::to_value(&updated).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 解除阻塞（Unblock）。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "解除工作项阻塞")]
#[tauri::command]
pub async fn opc_work_item_unblock(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    use axagent_company_runtime::WorkItemService;
    use axagent_company_runtime::work_item::Transition;
    let db = state.harness.db().clone();
    let svc = WorkItemService::new(&db);
    let model = svc.apply(&id, Transition::Unblock).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    serde_json::to_value(&model).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

// ── P4-1：人才库导入 + 市场包 ─────────────────────────────────

// `opc_import_talent_library` 已随 #264 批 7 刀 B2 删除：插件侧同名命令走 `opc.store` 的
// `import_talent_library` 写 op（正文逐字搬入 `HostOpcStore`，含它曾用到的私有辅助
// `parse_frontmatter_brief` —— 展示名的解析判据只有那一份）。

/// 市场包列表：扫描内置域包目录 + app_dir 已装状态。
/// 返回 [{id, name, icon, version, enabled, installed, path, domain, capabilityCount}]
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "列出市场域包")]
#[tauri::command]
pub async fn opc_market_list(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let app_dir = &state.app_data_dir;
    let builtin = resolve_capability_packs_dir(Some(app_dir));
    let installed_root = app_dir.join(CAPABILITY_PACKS_DIR);

    // P1-5：用户手动启用/停用状态以 DB `opc_capability_packs.enabled` 为准（manifest 仅首装生效）。
    // 故已注册包取 DB 值，未注册（未 seed）的包退回 manifest 默认。
    // DB 访问下沉到 analysis-engine（list_capability_pack_enabled_map），不在 command 层直连 entity。
    let db_enabled = list_capability_pack_enabled_map(state.harness.db()).await.unwrap_or_default();

    let mut items = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&builtin) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let manifest_path = dir.join("manifest.yaml");
            let Ok(raw) = std::fs::read_to_string(&manifest_path) else { continue };
            let Ok(manifest) = serde_yaml::from_str::<CapabilityPackManifest>(&raw) else {
                continue;
            };
            let installed = installed_root.join(&manifest.id).is_dir();
            items.push(serde_json::json!({
                "id": manifest.id,
                "name": manifest.name,
                "icon": manifest.icon,
                "version": manifest.version,
                "enabled": db_enabled.get(&manifest.id).copied().unwrap_or(manifest.enabled),
                "installed": installed,
                "path": dir.display().to_string(),
                // 期一·4：卡片展示归属本体域 + 能力承诺数
                // domain 缺失（旧 manifest）回退默认本体域 Automation，保证 UI 恒有值
                "domain": manifest
                    .domain
                    .as_ref()
                    .map(|d| d.as_str().to_string())
                    .unwrap_or_else(|| axagent_harness::CapabilityDomain::Automation.as_str().to_string()),
                "capabilityCount": manifest.capabilities.len(),
                // 期一·5：卡片展示具体承诺能力（predicate 解析出的可读 id 列表）
                "capabilities": manifest
                    .capabilities
                    .iter()
                    .filter_map(|c| match &c.predicate {
                        CapabilityMatchPredicate::IdExact { id } => Some(id.clone()),
                        CapabilityMatchPredicate::IdPrefix { prefix } => {
                            Some(format!("id_prefix:{prefix}"))
                        },
                        CapabilityMatchPredicate::All => None,
                    })
                    .collect::<Vec<String>>(),
            }));
        }
    }
    serde_json::to_value(items).map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

// ── OPC 自改进循环（对接上游 Loop Engineering，参照 stock_analysis）──

/// 自改进 WorkItem 循环：OPC 领域实现 OpcWorkItemRound（company-runtime）
/// 通过上游 harness::SelfImprovingRound trait + agent::SelfImprovementExecutor
/// 跑"执行 → 自评估 → 收敛/改进"回合制闭环。返回最终产出 + 评估分 + 轮次。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "自改进 OPC 工作项循环")]
#[tauri::command]
pub async fn run_self_improving_opc_work_item(
    state: State<'_, AppState>,
    task: String,
    max_rounds: Option<u32>,
) -> Result<serde_json::Value, String> {
    use axagent_agent::self_improvement_executor::{
        SelfImprovementConfig, SelfImprovementExecutor,
    };

    let db = state.harness.db().clone();
    let round = axagent_company_runtime::OpcWorkItemRound::new(db);
    let config = SelfImprovementConfig::new(
        max_rounds.unwrap_or(3),
        0.80, // 收敛阈值：评估分高于此值直接 Accept
        3,    // 连续无进展多少次后 Escalate
    );
    let mut executor = SelfImprovementExecutor::new(Box::new(round), config);

    match executor.run(&task).await {
        Ok(output) => Ok(serde_json::json!({
            "text": output.text,
            "totalRounds": output.total_rounds,
            "finalScore": output.final_evaluation.score,
            "confidence": output.final_evaluation.confidence,
            "strengths": output.final_evaluation.strengths,
            "gaps": output.final_evaluation.gaps,
        })),
        Err(e) => Err(e.to_string()),
    }
}

// ── 方案 B：OPC 角色进 Fleet（本条已随 #264 批 7 刀 B2 删除）────────────
//
// `opc_sync_fleet` 已删：插件侧同名命令走 `opc.store` 的 `sync_fleet` 写 op。正文（找/建默认
// 舰队 → 扫 active 员工 → 按 `agent_slug` 判重 → 成员状态由最新 work item phase 驱动）逐字
// 搬入 `init/opc_store_service.rs`；两段直连 sea_orm 的查询（员工扫描、工作项状态聚合）按
// 分层纪律下沉 dao（`axagent-infra/crates/dao/src/repo/opc_org.rs`），私有辅助
// `opc_batch_role_status` 随之下沉、本文件不再有第二份。

// ── 发布计划 Tauri 命令（本段两条已随 #264 批 7 刀 B2 删除）────────────────
//
// `opc_get_publish_schedule` / `opc_update_publish_schedule` 两条宿主命令已删：插件侧
// 同名命令走 `opc.store` 的 `get_publish_schedule`（读）/ `update_publish_schedule`（写）
// 两条 op，状态机合法值与时间戳量纲的判据仍在域服务（`PublishScheduleService`）。

// `opc_process_due_schedules` 已随 #266 删除：插件侧同名命令（写 op，`axagent-plugins/crates/plugin-opc`）
// 是唯一通道；宿主侧零消费 —— `axagent-runtime/crates/tools` 的到期处理工具走
// `call_opc_command` 插件命令桥（fail-closed，不回落直连）。
