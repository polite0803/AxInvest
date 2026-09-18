// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 域包命令 — 直读域包配置和服务（与股票业务同架构）
//!
//! 所有业务逻辑通过独立的 Service 实现，不再依赖域包适配器。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axagent_agent_macro::agent_command;
use tauri::State;

use axagent_analysis_engine::opc::*;
use axagent_dao::db::DatabaseConnection;

use crate::AppState;
use crate::commands::opc_capability_pack_logic;
use axagent_analysis_engine::opc::capability_pack;

/// 定位域包目录：`{industries_dir}/{domain_pack_id}`
///
/// 仅用于 Phase 1 数据接入（读取域包 `analysis.yaml` 数据源配置）。
fn capability_pack_dir(app_dir: Option<&Path>, domain_pack_id: &str) -> Result<PathBuf, String> {
    let base = axagent_analysis_engine::opc::resolve_capability_packs_dir(app_dir);
    let dir = base.join(domain_pack_id);
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(format!("域包不存在: {domain_pack_id}"))
    }
}

/// 读取域包 `runtime.yaml` 的 KPI **元数据**（key/name/unit + dashboard_cards）。
///
/// 这是 KPI 元数据的**唯一权威来源**。返回 `None` 表示包内没有可用 KPI 元数据
/// （文件缺失/解析失败已由 loader 以 `warn` 留痕，不静默），调用方回退既有配置。
///
/// 目录名双轨：manifest.id 是下划线（`content_media`），前端也可能传连字符形式。
fn load_runtime_kpi_meta(
    app_dir: Option<&Path>,
    domain_pack_id: &str,
) -> Option<capability_pack::runtime_schema::RuntimeKpiConfig> {
    let base = axagent_analysis_engine::opc::resolve_capability_packs_dir(app_dir);
    let candidates = [domain_pack_id.to_string(), domain_pack_id.replace('-', "_")];
    for dir_name in candidates {
        let dir = base.join(&dir_name);
        if !dir.is_dir() {
            continue;
        }
        if let Some(bundle) = capability_pack::analysis_schema::load_capability_pack(&dir) {
            if let Some(cfg) = bundle.runtime.filter(|c| c.has_kpi_metadata()) {
                return Some(cfg);
            }
        }
    }
    None
}

/// 从**已加载**的 runtime 元数据里取风控阈值声明。
///
/// 与 [`declared_kpi_thresholds`] 分开的唯一理由是**复用一次文件读取**：
/// `get_dashboard` 已经为 KPI 卡片加载过同一份元数据，再独立加载一次既浪费又会
/// 造成「卡片来自第 N 次读、阈值来自第 N+1 次读」的撕裂可能。
pub(crate) fn declared_kpi_thresholds_from(
    meta: Option<&capability_pack::runtime_schema::RuntimeKpiConfig>,
    domain_pack_id: &str,
) -> Vec<DeclaredKpiThreshold> {
    let Some(meta) = meta else {
        return Vec::new();
    };
    let scoped = risk_scoped_kpi_keys(domain_pack_id);
    meta.kpi_definitions
        .iter()
        .filter(|d| scoped.contains(&d.key.as_str()))
        .filter_map(|d| {
            let rule = d.risk.as_ref()?;
            let threshold =
                DeclaredKpiThreshold { key: d.key.clone(), min: rule.min, max: rule.max };
            threshold.is_effective().then_some(threshold)
        })
        .collect()
}

/// 读取域包 `runtime.yaml` 声明的 KPI 风控阈值，供 `OpcRiskGate` 注入。
///
/// 只下发两类条目：
/// 1. 属于本域包**受管键集**的键（`analysis::risk_scoped_kpi_keys`）——键名跨域包
///    重名（如 `completion_rate` 在 7 个域包中都存在），不按键名裸匹配；
/// 2. `min` / `max` **至少声明一个**的条目 —— 两个都缺视同未声明。
///
/// 未声明一律不下发（**未声明 = 不启用**）；「受管但未声明」由 `OpcRiskGate`
/// 构造时 `warn` 留痕，不在此处静默兜底。
pub(crate) fn declared_kpi_thresholds(
    app_dir: Option<&Path>,
    domain_pack_id: &str,
) -> Vec<DeclaredKpiThreshold> {
    declared_kpi_thresholds_from(
        load_runtime_kpi_meta(app_dir, domain_pack_id).as_ref(),
        domain_pack_id,
    )
}

/// 用 runtime.yaml 元数据补全 KPI 列表（YAML 管元数据，Rust 管计算）。
///
/// 规则：
/// 1. `id` 缺省 = `key`（前端拿它做 React key）；
/// 2. `name` / `unit` 以 YAML 为准（YAML 未声明 unit 时保留计算侧单位）；
/// 3. **YAML 声明但计算侧未产出的 key ⇒ 追加一条显式不可用记录**（`availability`
///    标明 `no_data_source`/`empty`，值仅为占位）——未知必须显式，不得静默消失；
/// 4. 计算侧产出但 YAML 未声明的 key 一律保留（避免已有真实值从面板消失），
///    `name` 回退为 `key`。
fn apply_kpi_metadata(
    kpis: &mut Vec<KpiValue>,
    domain_pack_id: &str,
    meta: Option<&capability_pack::runtime_schema::RuntimeKpiConfig>,
) {
    let Some(meta) = meta else {
        // 无元数据：至少保证 id 不为空，前端 key 稳定。
        for k in kpis.iter_mut() {
            if k.id.is_empty() {
                k.id = k.key.clone();
            }
        }
        return;
    };

    let now = chrono::Utc::now().timestamp();
    let mut produced: std::collections::HashMap<String, KpiValue> =
        kpis.drain(..).map(|k| (k.key.clone(), k)).collect();

    let mut ordered: Vec<KpiValue> =
        Vec::with_capacity(meta.kpi_definitions.len() + produced.len());
    for def in &meta.kpi_definitions {
        match produced.remove(&def.key) {
            Some(mut k) => {
                if k.id.is_empty() {
                    k.id = def.key.clone();
                }
                k.name = if def.name.is_empty() {
                    k.key.clone()
                } else {
                    def.name.clone()
                };
                if def.unit.is_some() {
                    k.unit = def.unit.clone();
                }
                ordered.push(k);
            },
            None => {
                let (availability, note) =
                    match capability_pack_kpi_service::kpi_source(domain_pack_id, &def.key) {
                        Some(capability_pack_kpi_service::KpiSource::NoDataSource(reason)) => {
                            (KpiAvailability::NoDataSource, Some(reason.to_string()))
                        },
                        Some(_) => (
                            KpiAvailability::Empty,
                            Some("计算源已注册，但本次未产出值".to_string()),
                        ),
                        None => (
                            KpiAvailability::NoDataSource,
                            Some(
                                "该 KPI 尚未在 Rust 侧注册计算器（元数据来自 runtime.yaml）"
                                    .to_string(),
                            ),
                        ),
                    };
                ordered.push(KpiValue {
                    key: def.key.clone(),
                    id: def.key.clone(),
                    name: if def.name.is_empty() {
                        def.key.clone()
                    } else {
                        def.name.clone()
                    },
                    value: 0.0,
                    target: None,
                    unit: def.unit.clone(),
                    timestamp: now,
                    availability,
                    note,
                });
            },
        }
    }

    let mut rest: Vec<KpiValue> = produced.into_values().collect();
    rest.sort_by(|a, b| a.key.cmp(&b.key));
    for mut k in rest {
        if k.id.is_empty() {
            k.id = k.key.clone();
        }
        if k.name.is_empty() {
            k.name = k.key.clone();
        }
        ordered.push(k);
    }
    *kpis = ordered;
}

// ── 公共 API（内部函数） ───────────────────────────────────────

/// 验证域包实体
pub async fn validate_entity(
    db: &DatabaseConnection,
    app_dir: Option<&Path>,
    domain_pack_id: &str,
    entity_type: &str,
    entity_data: &serde_json::Value,
) -> Result<Vec<ValidationError>, String> {
    let errors = capability_pack_validator::validate_entity(
        domain_pack_id,
        entity_type,
        entity_data,
        app_dir,
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    let _ = db; // 参数保留，未来可能用于数据库验证
    Ok(errors)
}

/// 批量验证域包实体
pub async fn validate_batch(
    db: &DatabaseConnection,
    app_dir: Option<&Path>,
    domain_pack_id: &str,
    entities: &[(String, serde_json::Value)],
) -> Result<Vec<(String, Vec<ValidationError>)>, String> {
    let results = capability_pack_validator::validate_batch(domain_pack_id, entities, app_dir)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let _ = db;
    Ok(results)
}

/// 计算域包 KPI 指标
pub async fn compute_kpis(
    db: &DatabaseConnection,
    domain_pack_id: &str,
    time_range: TimeRange,
) -> Result<Vec<KpiValue>, String> {
    let data_service: Arc<dyn OpcDataService> = Arc::new(DefaultDataService::new(db.clone()));
    capability_pack_kpi_service::compute_kpis(domain_pack_id, &data_service, &time_range)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

/// 获取域包 KPI 定义列表
///
/// 元数据权威来源：域包 `runtime.yaml` 的 `kpi_definitions`；包内无元数据时
/// 回退 `capability_pack_config`（旧域包/未迁域包）。
pub fn get_kpi_definitions(
    app_dir: Option<&Path>,
    domain_pack_id: &str,
) -> Result<Vec<KpiDefinition>, String> {
    let config = capability_pack_config::get_config(domain_pack_id, app_dir)
        .ok_or_else(|| format!("域包配置不存在: {}", domain_pack_id))?;
    if let Some(meta) = load_runtime_kpi_meta(app_dir, domain_pack_id) {
        return Ok(meta
            .kpi_definitions
            .iter()
            .map(|d| KpiDefinition {
                key: d.key.clone(),
                name: if d.name.is_empty() {
                    d.key.clone()
                } else {
                    d.name.clone()
                },
                description: String::new(),
                metric_type: d
                    .metric_type
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default(),
                target: None,
                unit: d.unit.clone(),
                formula: None,
                calculation_rule: None,
            })
            .collect());
    }
    Ok(config.kpi_definitions)
}

/// 获取域包工作流步骤（从配置获取）
pub fn get_workflow_steps(domain_pack_id: &str) -> Result<Vec<WorkflowStep>, String> {
    // 从 seed 文件定义的工作流节点获取，不再使用适配器
    // 这里返回空列表，实际工作流步骤由前端从 template 表加载
    let _ = domain_pack_id;
    Ok(Vec::new())
}

/// 获取域包启用的自动化规则
pub fn get_enabled_rules(
    app_dir: Option<&Path>,
    domain_pack_id: &str,
) -> Result<Vec<CapabilityPackAutomationRule>, String> {
    let config = capability_pack_config::get_config(domain_pack_id, app_dir)
        .ok_or_else(|| format!("域包配置不存在: {}", domain_pack_id))?;
    Ok(config.automation_rules.into_iter().filter(|r| r.enabled).collect())
}

/// 运行域包自动化规则（通用条件求值 + 动作执行）
pub async fn run_automation_rules(
    db: &DatabaseConnection,
    app_dir: Option<&Path>,
    domain_pack_id: &str,
    context: RuleContext,
) -> Result<Vec<String>, String> {
    let config = capability_pack_config::get_config(domain_pack_id, app_dir)
        .ok_or_else(|| format!("域包配置不存在: {}", domain_pack_id))?;
    let rules = config.automation_rules.into_iter().filter(|r| r.enabled).collect::<Vec<_>>();
    let ctx_map = opc_capability_pack_logic::context_to_hashmap(&context);
    let data_service: Arc<dyn OpcDataService> = Arc::new(DefaultDataService::new(db.clone()));
    let mut triggered = Vec::new();
    for rule in &rules {
        if opc_capability_pack_logic::evaluate_conditions(&rule.conditions, &ctx_map) {
            opc_capability_pack_logic::execute_rule_actions(Some(&data_service), rule, &context)
                .await
                .map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;
            triggered.push(rule.id.clone());
        }
    }
    Ok(triggered)
}

/// 获取域包仪表盘数据
///
/// KPI 值由 Rust 计算（按 key 查计算器注册表），KPI 的 id/name/unit 与展示卡片
/// 由域包 `runtime.yaml` 权威提供；风控等级复用 `OpcRiskGate`（与
/// `opc_execute_analysis` 同一判据）。
pub async fn get_dashboard(
    app_dir: Option<&Path>,
    db: &DatabaseConnection,
    domain_pack_id: &str,
    time_range: TimeRange,
) -> Result<CapabilityPackDashboard, String> {
    let config = capability_pack_config::get_config(domain_pack_id, app_dir)
        .ok_or_else(|| format!("域包配置不存在: {}", domain_pack_id))?;
    let meta = load_runtime_kpi_meta(app_dir, domain_pack_id);
    let mut kpis = compute_kpis(db, domain_pack_id, time_range).await?;
    apply_kpi_metadata(&mut kpis, domain_pack_id, meta.as_ref());

    // 风控判定**就在上面这份已装配的 KPI 列表上做**：
    // - `availability` 是链路上本来就有的（`compute_kpis` 逐项给，`apply_kpi_metadata`
    //   只会把「YAML 声明但未产出」的键标成 `empty`/`no_data_source`，**永远不会**
    //   伪造成 `available`）⇒ 门控的 `Available` 前置守卫真实生效，无需另取数据；
    // - 阈值同样来自域包（`declared_kpi_thresholds`），与 `opc_execute_analysis`
    //   走同一个注入点 ⇒ 两条链路的等级不会打架；
    // - **不新增任何 DB 查询**：判定只读内存里已有的 `kpis`。
    let declared = declared_kpi_thresholds_from(meta.as_ref(), domain_pack_id);
    let risk = OpcRiskGate::with_declared_thresholds(domain_pack_id, declared)
        .check(&kpis)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let cards = match meta.as_ref().map(|m| m.dashboard_cards.as_slice()) {
        Some(cards) if !cards.is_empty() => cards
            .iter()
            .map(|c| {
                DashboardCard::new(
                    &c.id,
                    &c.title,
                    &c.kpi_key,
                    c.display_value.clone().unwrap_or_default(),
                )
            })
            .collect(),
        _ => config
            .dashboard_cards
            .into_iter()
            .map(|c| DashboardCard::new(&c.id, &c.title, &c.kpi_key, ""))
            .collect(),
    };

    Ok(CapabilityPackDashboard {
        domain_pack_id: domain_pack_id.to_string(),
        kpis,
        cards,
        summary: None,
        // 已判定（`Some`）；未判定语义见 `CapabilityPackDashboard::risk_level` 的文档。
        risk_level: Some(risk.risk_level),
        violations: risk.violations,
    })
}

/// 列出全部内建域包（从配置获取）
pub fn list_industries() -> Vec<(String, String)> {
    capability_pack_config::list_industries()
}

/// 检查域包是否存在（从配置获取）
pub fn has_capability_pack(app_dir: Option<&Path>, domain_pack_id: &str) -> bool {
    capability_pack_config::get_config(domain_pack_id, app_dir).is_some()
}

// ── Tauri 命令（签名保持前端契约；app_state 由 Tauri 自动注入） ──

/// 验证域包实体（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "验证域包实体")]
#[tauri::command]
pub async fn opc_validate_entity(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
    entity_type: String,
    entity_data: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let db = app_state.harness.db();
    let errors = validate_entity(
        db,
        Some(&app_state.app_data_dir),
        &capability_pack_id,
        &entity_type,
        &entity_data,
    )
    .await?;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "entityType": entity_type,
        "valid": errors.is_empty(),
        "errors": errors,
    }))
}

/// 计算域包 KPI（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "计算域包KPI")]
#[tauri::command]
pub async fn opc_compute_kpis(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
    days: Option<i64>,
) -> Result<serde_json::Value, String> {
    let range = match days {
        Some(d) => TimeRange::days(d),
        None => TimeRange::days(30),
    };
    let db = app_state.harness.db();
    let kpis = compute_kpis(db, &capability_pack_id, range).await?;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "kpis": kpis,
    }))
}

/// 运行域包自动化规则（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "运行域包自动化规则")]
#[tauri::command]
pub async fn opc_run_automation_rules(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
    entity_type: String,
    entity_id: String,
    status: Option<String>,
    overdue_days: Option<u32>,
    created_days: Option<u32>,
    fields: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut ctx = RuleContext::new(&entity_type, &entity_id);
    if let Some(s) = status {
        ctx = ctx.with_status(s);
    }
    if let Some(d) = overdue_days {
        ctx = ctx.with_overdue_days(d);
    }
    if let Some(d) = created_days {
        ctx = ctx.with_created_days(d);
    }
    if let Some(f) = fields {
        ctx.fields = f;
    }
    let db = app_state.harness.db();
    let triggered =
        run_automation_rules(db, Some(&app_state.app_data_dir), &capability_pack_id, ctx).await?;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "triggeredRules": triggered,
        "triggeredCount": triggered.len(),
    }))
}

/// 获取域包仪表盘（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取域包仪表盘")]
#[tauri::command]
pub async fn opc_get_capability_pack_dashboard(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
    days: Option<i64>,
) -> Result<serde_json::Value, String> {
    let range = match days {
        Some(d) => TimeRange::days(d),
        None => TimeRange::days(30),
    };
    let db = app_state.harness.db();
    let dashboard =
        get_dashboard(Some(&app_state.app_data_dir), db, &capability_pack_id, range).await?;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "dashboard": dashboard,
    }))
}

/// 列出全部域包（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateOnly, description = "列出全部域包")]
#[tauri::command]
pub async fn opc_list_runtime_industries(
    _app_state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let industries = list_industries();
    Ok(serde_json::json!({
        "count": industries.len(),
        "industries": industries.into_iter().map(|(id, name)| {
            serde_json::json!({ "id": id, "name": name })
        }).collect::<Vec<_>>(),
    }))
}

/// 获取域包工作流步骤（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取域包工作流步骤")]
#[tauri::command]
pub async fn opc_get_capability_pack_workflow_steps(
    _app_state: State<'_, AppState>,
    capability_pack_id: String,
) -> Result<serde_json::Value, String> {
    let steps = get_workflow_steps(&capability_pack_id)?;
    let step_infos: Vec<serde_json::Value> = steps
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "name": s.name,
                "description": s.description,
                "order": s.order,
                "status": "pending",
            })
        })
        .collect();
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "steps": step_infos,
    }))
}

/// 获取域包自动化规则（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取域包自动化规则")]
#[tauri::command]
pub async fn opc_get_capability_pack_automation_rules(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
) -> Result<serde_json::Value, String> {
    let rules = get_enabled_rules(Some(&app_state.app_data_dir), &capability_pack_id)?;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "rules": rules,
    }))
}

/// 批量验证域包实体（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "批量验证域包实体")]
#[tauri::command]
pub async fn opc_batch_validate_entities(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
    entities: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let pairs: Vec<(String, serde_json::Value)> = entities
        .into_iter()
        .filter_map(|e| {
            let t = e.get("entityType").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let d = e.get("data").cloned().unwrap_or(serde_json::Value::Null);
            if t.is_empty() { None } else { Some((t, d)) }
        })
        .collect();
    let db = app_state.harness.db();
    let results =
        validate_batch(db, Some(&app_state.app_data_dir), &capability_pack_id, &pairs).await?;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "results": results.into_iter().map(|(t, errs)| {
            serde_json::json!({ "entityType": t, "valid": errs.is_empty(), "errors": errs })
        }).collect::<Vec<_>>(),
    }))
}

/// 获取域包 KPI 定义（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取域包KPI定义")]
#[tauri::command]
pub async fn opc_get_kpi_definitions(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
) -> Result<serde_json::Value, String> {
    let definitions = get_kpi_definitions(Some(&app_state.app_data_dir), &capability_pack_id)?;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "definitions": definitions,
    }))
}

/// 检查域包是否存在（Tauri 命令）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "检查域包是否存在")]
#[tauri::command]
pub async fn opc_has_capability_pack(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
) -> Result<serde_json::Value, String> {
    let exists = has_capability_pack(Some(&app_state.app_data_dir), &capability_pack_id);
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "exists": exists,
    }))
}

// ── Phase 1 数据接入命令（OpCapabilityPackClient 直读域包 analysis.yaml） ──

/// 构造域包数据客户端（db/cache/web/file 内建 vendor，无容器）
fn build_data_client(
    app_state: &AppState,
    domain_pack_id: &str,
) -> Result<OpCapabilityPackClient, String> {
    let dir = capability_pack_dir(Some(&app_state.app_data_dir), domain_pack_id)?;
    let config = crate::commands::opc_data::load_analysis_config(&dir)?;
    let db = app_state.harness.db();
    let mut vendors: std::collections::HashMap<String, std::sync::Arc<dyn OpCapabilityPackVendor>> =
        std::collections::HashMap::new();
    let db_vendor = DbVendor::new(std::sync::Arc::new(DefaultDataService::new(db.clone())));
    vendors.insert("db".to_string(), std::sync::Arc::new(db_vendor));
    let cache_vendor = CacheVendor::new(app_state.app_data_dir.join("opc-cache"));
    vendors.insert("cache".to_string(), std::sync::Arc::new(cache_vendor));
    vendors.insert("web".to_string(), std::sync::Arc::new(WebVendor));
    vendors.insert("file".to_string(), std::sync::Arc::new(FileVendor));

    let sources: Vec<AnalysisDataSource> = config
        .data_sources
        .iter()
        .map(|s| AnalysisDataSource {
            id: s.id.clone(),
            chain: s.chain.clone(),
            quality_precheck: s.quality_precheck,
        })
        .collect();

    Ok(OpCapabilityPackClient::new(domain_pack_id.to_string(), sources, vendors))
}

/// 获取域包数据（Phase 1：按 analysis.yaml data_sources 路由 + 降级）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取域包数据")]
#[tauri::command]
pub async fn opc_get_capability_pack_data(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
    source_id: String,
    data_domain: String,
    query: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let client = build_data_client(&app_state, &capability_pack_id)?;
    let data = client
        .fetch(&source_id, &data_domain, &query.unwrap_or(serde_json::json!({})))
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "sourceId": source_id,
        "data": data,
    }))
}

/// 获取域包数据质量预检（Phase 1：quality_precheck 源清单探测）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取域包数据质量预检")]
#[tauri::command]
pub async fn opc_get_capability_pack_precheck(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
) -> Result<serde_json::Value, String> {
    let client = build_data_client(&app_state, &capability_pack_id)?;
    let precheck = client.precheck().await;
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "precheck": precheck,
    }))
}

/// 获取域包数据源健康状态（Phase 1：vendor 降级可观测）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取域包数据源健康状态")]
#[tauri::command]
pub async fn opc_get_capability_pack_health(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
) -> Result<serde_json::Value, String> {
    let client = build_data_client(&app_state, &capability_pack_id)?;
    let health = client.health_snapshot();
    Ok(serde_json::json!({
        "domainPackId": capability_pack_id,
        "health": health,
    }))
}

/// 动态工作流执行（兼容旧接口）
///
/// 新架构下所有工作流均通过种子化到 DB → WorkEngine 执行，
/// 此函数将旧的动态执行请求转发到标准执行通道。
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "动态工作流执行（兼容旧接口）")]
#[tauri::command]
pub async fn opc_execute_dynamic_workflow(
    app_state: State<'_, AppState>,
    capability_pack_id: String,
    workflow_id: Option<String>,
    days: Option<u32>,
    user_input: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    crate::commands::opc_capability_pack_actions::opc_execute_workflow(
        app_state,
        capability_pack_id,
        workflow_id,
        days,
        user_input,
    )
    .await
}
