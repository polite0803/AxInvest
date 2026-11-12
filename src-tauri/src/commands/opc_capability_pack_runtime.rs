// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 域包命令 — 直读域包配置和服务（与股票业务同架构）
//!
//! 所有业务逻辑通过独立的 Service 实现，不再依赖域包适配器。

use std::path::Path;
use std::sync::Arc;

use axagent_company_runtime::opc::*;
use axagent_dao::db::DatabaseConnection;

use crate::commands::opc_capability_pack_logic;
use axagent_company_runtime::opc::capability_pack;

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
    let base = axagent_company_runtime::opc::resolve_capability_packs_dir(app_dir);
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
//
// #264 批 7 刀 B1 退役三个自由函数（`validate_entity` / `validate_batch` / `compute_kpis`）：
// 唯一调用面是同批已删的三条 `#[agent_command]` 包装器（`opc_validate_entity` /
// `opc_batch_validate_entities` / `opc_compute_kpis`，零前端调用迁 T2 插件通道）。真实
// 实现体在 company-runtime 的 `capability_pack_validator` 与 `capability_pack_kpi_service`，
// 现由 `HostOpcStore`（`src-tauri/src/init/opc_store_service.rs`）直调 —— 两条 validate 的
// `db` 形参本来就是死参（`let _ = db`），接缝侧已按此收敛为只带域包配置目录。

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
    // KPI 计算直调 company-runtime（#264 批 7 刀 B1 删宿主 `compute_kpis` 薄包装后
    // 改为原地装配 —— 装配形态与 `HostOpcStore::compute_kpis` 同链，错误映射沿用
    // `ErrorResponse::from_error` 的 Unrecoverable 档）。
    let data_service: Arc<dyn OpcDataService> = Arc::new(DefaultDataService::new(db.clone()));
    let mut kpis =
        capability_pack_kpi_service::compute_kpis(domain_pack_id, &data_service, &time_range)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
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
//
// #264 批 7 刀 A 删除三条只读包装器（`opc_list_runtime_industries` / `opc_get_kpi_definitions` /
// `opc_has_capability_pack`）：调用方走 plugin-opc 的同名命名命令，宿主侧经 `opc.store` 的三个新
// op 由 `HostOpcStore` 提供同一份域包配置读数（`app_dir` 由 `state.rs` 装配，未装配即 fail-closed）。
// ⚠ 本文件的 `list_industries`（磁盘 `runtime.yaml`/config 域包）与
// `opc_capability_pack_actions` 的注册表列表是**两个不同的数据源**，接缝侧分别落在
// `list_runtime_industries` 与 `list_industries` 两个 op 上，不得混同。

// #264 批 7 刀 B3 删除四条包装命令（`opc_run_automation_rules` /
// `opc_get_capability_pack_dashboard` / `opc_get_capability_pack_workflow_steps` /
// `opc_get_capability_pack_automation_rules`）：调用方走 plugin-opc 的同名命名命令，
// 宿主侧经 `opc.store` 的四个同名 op 由 `HostOpcStore` 转调本文件的四个 helper
// （`run_automation_rules` / `get_dashboard` / `get_workflow_steps` / `get_enabled_rules`，
// 签名不变）。信封逐字沿用迁移前命令体（`domainPackId` + 载荷）。

// ── Phase 1 数据接入命令（OpCapabilityPackClient 直读域包 analysis.yaml） ──
//
// #264 批 7 刀 B1 退役三条命令（`opc_get_capability_pack_data` / `opc_get_capability_pack_precheck`
// / `opc_get_capability_pack_health`，零前端调用迁 T2 插件通道）与它们的私有装配链
// （`build_data_client` + `capability_pack_dir`，唯一调用面就是这三条命令）：装配逻辑搬进
// `HostOpcStore::build_data_client`（`src-tauri/src/init/opc_store_service.rs`），`app_data_dir`
// 用已装配的 `app_dir`（未装配 fail-closed，不回退进程级默认根）。

// `opc_execute_dynamic_workflow` 已随 #264 批 7 刀 B2 删除：插件侧同名命令走 `opc.store` 的
// `execute_dynamic_workflow` 写 op。三级查找的正文只有一份 ——
// `commands::opc_capability_pack_actions::execute_workflow_core`（宿主命令 `opc_execute_workflow`
// 与接缝 provider 共用），本文件不再有第二条转调链。
