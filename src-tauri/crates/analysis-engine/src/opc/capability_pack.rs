// SPDX-License-Identifier: AGPL-3.0-only

//! 域包数据资产包（Domain Pack）引擎
//!
//! 域包 = 数据资产包，非代码。每个域包一个独立目录：
//! `config/opc/domain_packs/{domain_pack_id}/`
//!   ├── manifest.yaml     # id / name / icon / version / enabled
//!   ├── roles.yaml        # 域包角色映射（opc-cfo 等 → 专家/工具白名单）
//!   └── workflows/*.yaml  # 工作流模板（纯数据，节点/边/prompt）
//!
//! 启动扫描注册到 `opc_capability_packs` 表，支持单独启用/禁用/导出/导入。
//! 域包级版本号取代全局 OPC_TEMPLATE_VERSION，域包间互不影响。

use axagent_harness::util_fns::now_ts;
use axagent_harness::CapabilityDomain;
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 域包根目录（相对仓库根）
pub const CAPABILITY_PACKS_DIR: &str = "config/opc/domain_packs";

/// **旧**域包根目录（2026-09-15 更名前）。
///
/// 仅用于 [`resolve_capability_packs_dir`] 的**存量回退读取**：
/// 升级前安装到 app_dir 的 `.opcip` 包仍在旧路径下，不设回退会静默读不到。
/// ⚠ 不要在任何「写」路径上使用本常量 —— 写入一律走 [`CAPABILITY_PACKS_DIR`]。
///
/// ⚠ **刻意写成 `concat!` 拼接**：本仓的批量更名脚本按「字符串字面量 `config/opc/industries`」
/// 做替换，若此处直接写完整字面量，脚本会把**回退目标**一并改成新目录，
/// 使回退静默退化为「指向同一个目录」（实测已发生过一次）。
pub const LEGACY_CAPABILITY_PACKS_DIR: &str = concat!("config/opc/", "industries");

// ── manifest.yaml schema ──────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CapabilityPackManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_version")]
    pub version: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 分析配置文件（P0-4 四件套之一），缺省 "analysis.yaml"，None 表示无分析配置
    /// 供 `analysis_schema::load_capability_pack_analysis` 读取（capability_pack.rs:286）
    #[serde(default = "default_analysis_file")]
    pub analysis: String,
    /// 学习配置文件（P0-4 四件套之一），缺省 "learning.yaml"；读取见
    /// `opc_capability_pack_actions::capability_pack_learning_config_path`
    /// （`capability_pack_learning_config_path` 经 `read_manifest` 消费本字段决定实际文件名）
    #[serde(default = "default_learning_file")]
    pub learning: String,
    /// 运行时配置文件（P0-4 四件套之一），缺省 "runtime.yaml"。
    /// KPI **元数据**（key/name/unit/metric_type）与展示卡片的权威来源，
    /// 读取见 `runtime_schema::load_capability_pack_runtime`。
    #[serde(default = "default_runtime_file")]
    pub runtime: String,
    /// 该域包工作流模板 ID 的前缀（**不含** `workflow-`），缺省为
    /// `id` 的下划线转连字符形式（`content_media` → `content-media`）。
    ///
    /// 存在的原因：个别域包的模板 ID 用的是缩写（content_media 的模板是
    /// `workflow-cm-*`），与域包 id 不构成前缀关系，靠字符串推导不出 ——
    /// 显式声明，避免在代码里写死映射。
    #[serde(default)]
    pub template_prefix: Option<String>,
    /// 该域包归属的**本体域**（`CapabilityDomain` 用户域子集，见
    /// `PLAN-domain-pack-consolidation.md` §3.1）。缺省 `None` 表示未声明 ——
    /// 兼容旧 manifest，回退现行为（`Automation`）。值必须 ⊆ `DOMAIN_NODES` 用户域，
    /// 校验器 `assert_capability_pack_capability_closed` 保证。
    #[serde(default)]
    pub domain: Option<CapabilityDomain>,
    /// **承诺能力契约**（能力集封闭校验的承诺集合）。缺省空 = 未声明（不参与校验）。
    /// 每个承诺描述「归属哪个来源 + 哪类能力 + 用何种谓词匹配」，非裸 id 白名单。
    #[serde(default)]
    pub capabilities: Vec<CapabilityPackCapabilityClaim>,
    /// 办公室「建房即成队」声明段（`PLAN-office-auto-provision.md` 阶段 3）。
    /// 缺省 `None` = 未声明，消费方回退 Rust 专家名册（`domain_pack_roster`）。
    #[serde(default)]
    pub office: Option<CapabilityPackOffice>,
}

/// manifest 的 `office` 段 —— 域包办公室声明（数据驱动，新增域包无需改代码）。
#[derive(Debug, Clone, Deserialize)]
pub struct CapabilityPackOffice {
    /// 建房自动入房的成员名册。非空即**整表覆盖** Rust 名册推导的默认名单。
    #[serde(default)]
    pub seed_members: Vec<CapabilityPackSeedMember>,
}

/// `office.seed_members` 元素 —— `expert` 是专家 key（profile id = `opc-<expert>`），
/// `room` 显式指定 Phaser 房间站位；缺省 = 前端按 defaultRoomId 优先轮转分配。
#[derive(Debug, Clone, Deserialize)]
pub struct CapabilityPackSeedMember {
    pub expert: String,
    #[serde(default)]
    pub room: Option<String>,
}

fn default_analysis_file() -> String {
    "analysis.yaml".into()
}

// ── 承诺能力契约（能力集封闭） ──────────────────────────────────────

/// 域包承诺能力条目 —— manifest 的 `capabilities` 元素。
///
/// 语义：域包声明「我应当提供这些能力」，与能力索引的**实际归属**对账。
/// 承诺 ⊆ 实际：不满足则提示缺失；实际 ⊄ 承诺：触发「提示增加域」。
#[derive(Debug, Clone, Deserialize)]
pub struct CapabilityPackCapabilityClaim {
    /// 能力**来源**（Tool / Skill / McpTool / Workflow / Agent），复用 `opc::capability::CapabilitySource`。
    pub source: super::capability::CapabilitySource,
    /// 能力类型谓词（`capability_type` 匹配目标；配合 `predicate` 判定——如
    /// `l1_classifier` / `analysis_source`，具体取值由域包业务自定）。
    pub capability_type: String,
    /// 匹配谓词：声明「承诺覆盖该身份子集的哪个范围」。
    pub predicate: CapabilityMatchPredicate,
}

/// 匹配谓词 —— 声明一条承诺覆盖的实际能力范围。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CapabilityMatchPredicate {
    /// 该来源下的**全部**能力（能力包整来源封闭）。
    All,
    /// 承诺 id 以该前缀开头的全部能力。
    IdPrefix { prefix: String },
    /// 承诺单个精确 id 的能力。
    IdExact { id: String },
}
fn default_learning_file() -> String {
    "learning.yaml".into()
}
fn default_runtime_file() -> String {
    "runtime.yaml".into()
}

fn default_icon() -> String {
    "🏢".into()
}
fn default_version() -> i32 {
    1
}
fn default_true() -> bool {
    true
}

// ── 包加载 ────────────────────────────────────────────────────────

/// 读取单个域包目录的 manifest（解析失败返回 None，调用方自行兜底）。
pub fn read_manifest(capability_pack_dir: &Path) -> Option<CapabilityPackManifest> {
    let raw = std::fs::read_to_string(capability_pack_dir.join("manifest.yaml")).ok()?;
    serde_yaml::from_str::<CapabilityPackManifest>(&raw).ok()
}

/// 扫描域包目录，返回所有 manifest（含是否启用）。
pub fn scan_capability_packs(base_dir: &Path) -> Vec<CapabilityPackManifest> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(base_dir) else { return out };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join("manifest.yaml");
        let Ok(raw) = std::fs::read_to_string(&manifest_path) else { continue };
        match serde_yaml::from_str::<CapabilityPackManifest>(&raw) {
            Ok(m) => {
                out.push(m);
            },
            Err(e) => {
                tracing::warn!("[domain-pack] {} manifest 解析失败: {e}", dir.display());
            },
        }
    }
    out
}

// P0-4 定义 schema；Phase 1 数据接入层已消费 data_sources/quality_precheck。
// strategies/risk 字段由 P2（分析策略维度）消费，当前未被读取的字段交给编译器逐项报告。
pub mod analysis_schema {
    use serde::Deserialize;
    use std::path::{Path, PathBuf};

    use super::CapabilityPackManifest;

    /// 域包分析配置（`analysis.yaml`，由 manifest.analysis 字段引用，缺省同名文件）。
    ///
    /// 供数据接入层（OpCapabilityPackVendor 路由）、分析层（策略维度）与
    /// 质量预检（QualityPrecheck 源清单）消费。P0-4 先定义 schema 与加载，
    /// 执行逻辑在 P1/P2 接入。
    // analysis.yaml 是配置契约（yaml 文件字段齐全），P2 分析策略维度接入后消费
    // strategies/risk/quality_precheck，届时移除豁免。
    #[derive(Debug, Clone, Default, Deserialize)]
    #[allow(dead_code)]
    pub struct CapabilityPackAnalysisConfig {
        #[serde(default)]
        pub version: u32,
        #[serde(default)]
        pub domain_pack_id: String,
        /// 数据源声明（vendor 链按优先级）
        #[serde(default)]
        pub data_sources: Vec<AnalysisDataSource>,
        /// 分析策略（域包专属分析维度）
        #[serde(default)]
        pub strategies: Vec<AnalysisStrategy>,
        /// 风控参数（对齐 position_limits 的域包版）
        #[serde(default)]
        pub risk: AnalysisRisk,
        /// 质量预检源清单（对齐 stock QualityPrecheck 的域包版）
        #[serde(default)]
        pub quality_precheck: Vec<String>,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct AnalysisDataSource {
        pub id: String,
        /// vendor 链（按优先级：db / cache / web / file / astock）
        #[serde(default)]
        pub chain: Vec<String>,
        /// 是否纳入质量预检
        #[serde(default)]
        pub quality_precheck: bool,
    }

    /// P2（分析策略维度）接入后消费
    #[derive(Debug, Clone, Deserialize)]
    #[allow(dead_code)]
    pub struct AnalysisStrategy {
        pub id: String,
        #[serde(default)]
        pub name: String,
        /// 分析维度（如 cash_flow_health / tax_risk）
        #[serde(default)]
        pub dimensions: Vec<String>,
    }

    /// P2（风控参数维度）接入后消费
    #[derive(Debug, Clone, Default, Deserialize)]
    #[allow(dead_code)]
    pub struct AnalysisRisk {
        /// 超阈值告警线（0-1）
        #[serde(default)]
        pub max_kpi_warning_pct: f64,
        /// 关键 KPI 清单（越界触发风控拦截）
        #[serde(default)]
        pub critical_kpis: Vec<String>,
    }

    /// 读取域包内分析配置（`{manifest.analysis}`，缺省 analysis.yaml）。
    /// 文件缺失返回 None（向后兼容：旧域包无分析配置）。
    pub fn load_capability_pack_analysis(
        capability_pack_dir: &Path,
        manifest: &CapabilityPackManifest,
    ) -> Option<CapabilityPackAnalysisConfig> {
        let path = capability_pack_dir.join(&manifest.analysis);
        let raw = std::fs::read_to_string(&path).ok()?;
        match serde_yaml::from_str(&raw) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                tracing::warn!("[domain-pack] {} analysis 解析失败: {e}", path.display());
                None
            },
        }
    }

    /// 域包完整资产（P0-4：manifest + analysis；workflows 已迁移至手动 seed 文件）
    /// analysis/pack_dir 当前仅 load_capability_pack 装配，由 P1 数据接入层完整消费后移除豁免。
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    pub struct CapabilityPackBundle {
        pub manifest: CapabilityPackManifest,
        pub analysis: Option<CapabilityPackAnalysisConfig>,
        /// runtime.yaml 的 KPI 元数据（key/name/unit/metric_type + dashboard_cards）。
        /// 这是 KPI 元数据的**权威来源**；缺失时消费方回退既有配置。
        pub runtime: Option<super::runtime_schema::RuntimeKpiConfig>,
        /// 学习配置在 `{capability_pack_dir}/{manifest.learning}`（P4-3 已迁入域包），
        /// 此处不重复解析，读取走 `opc_capability_pack_actions::capability_pack_learning_config_path`
        pub pack_dir: PathBuf,
    }

    /// 加载单个域包目录的完整资产（manifest 解析失败返回 None）。
    pub fn load_capability_pack(dir: &Path) -> Option<CapabilityPackBundle> {
        let manifest_path = dir.join("manifest.yaml");
        let raw = std::fs::read_to_string(&manifest_path).ok()?;
        let manifest: CapabilityPackManifest = serde_yaml::from_str(&raw).ok()?;
        let analysis = load_capability_pack_analysis(dir, &manifest);
        let runtime = super::runtime_schema::load_capability_pack_runtime(dir, &manifest);
        Some(CapabilityPackBundle { manifest, analysis, runtime, pack_dir: dir.to_path_buf() })
    }
}

// ── runtime.yaml schema（KPI 元数据权威来源）──────────────────────

/// `runtime.yaml` 的 KPI 元数据段（`kpi_definitions` / `dashboard_cards`）。
///
/// ## 权威边界（本次改造的核心约定）
///
/// - **YAML 管元数据**：KPI 的 `key` / `name` / `unit` / `metric_type` 与展示卡片。
/// - **Rust 管计算**：按 `key` 注册计算器（查哪张表、怎么聚合），见
///   `analysis_engine::opc::capability_pack_kpi_service`。
/// - 契约只有一个：`key`。
///
/// `kpi_sources` 段为**废弃配置**（历史遗留：试图把「怎么算」声明化，属于错误
/// 抽象，全项目零解析器）。段本身可保留在 YAML 中，不解析即不生效；
/// **不要为其新增解释器**。
pub mod runtime_schema {
    use std::path::Path;

    use serde::Deserialize;

    use super::super::automation::CapabilityPackAutomationRule;
    use super::super::workflow::{KpiCalculationDef, WorkflowInputField};
    use super::CapabilityPackManifest;

    /// runtime.yaml 中的单条 KPI 元数据。
    #[derive(Debug, Clone, Deserialize)]
    pub struct RuntimeKpiDefinition {
        pub key: String,
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub unit: Option<String>,
        /// 指标类型字符串（count / percentage / currency / duration …）。
        /// 仅作元数据，不参与计算；未知取值原样保留，不做白名单校验。
        #[serde(default)]
        pub metric_type: Option<String>,
        /// 该 KPI 的**风控阈值声明**（可选）。
        ///
        /// `None`、或段内 `min`/`max` 都缺 ⇒ 该键**不参与风控**
        /// （未声明 = 不启用；用默认值兜底等于把硬编码换个位置）。
        /// 判定逻辑在 `analysis_engine::opc::analysis::OpcRiskGate`，
        /// 由命令层经 `opc_capability_pack_runtime::declared_kpi_thresholds` 注入。
        #[serde(default)]
        pub risk: Option<RuntimeKpiRiskRule>,
    }

    /// 单条 KPI 的风控阈值。至少声明 `min` / `max` 之一才有意义。
    #[derive(Debug, Clone, Deserialize)]
    pub struct RuntimeKpiRiskRule {
        /// 下限：KPI 值**低于**它即违规。
        #[serde(default)]
        pub min: Option<f64>,
        /// 上限：KPI 值**高于**它即违规。
        #[serde(default)]
        pub max: Option<f64>,
    }

    /// runtime.yaml 中的展示卡片。
    #[derive(Debug, Clone, Deserialize)]
    pub struct RuntimeDashboardCard {
        pub id: String,
        #[serde(default)]
        pub title: String,
        #[serde(default)]
        pub kpi_key: String,
        #[serde(default)]
        pub display_value: Option<String>,
    }

    /// runtime.yaml 校验规则段（`validations`）——**独立的段结构**。
    ///
    /// ⚠ 形状与 `CapabilityPackConfig.validations`（`ValidationDef{field,type,error_message}`）
    /// **不同源**：此处是 `{entity_type,field,operator,value,message}`，贴近
    /// `capability_pack_validator` 语义。两套校验概念不得误合并，消费由 A3 接入 validator。
    #[derive(Debug, Clone, Deserialize)]
    pub struct RuntimeValidation {
        #[serde(default)]
        pub entity_type: String,
        #[serde(default)]
        pub field: String,
        #[serde(default)]
        pub operator: String,
        /// 校验阈值/期望值，可为 null / 数字 / 字符串。
        #[serde(default)]
        pub value: serde_json::Value,
        #[serde(default)]
        pub message: String,
    }

    /// runtime.yaml 工作流步骤段（`workflow_steps`）——**独立的段结构**。
    ///
    /// 形状 `{id,name,description,order}`，与 `WorkflowStepDef`（含 prompt/tools）
    /// **不同**，故单独建 struct。A2 组装 `CapabilityPackConfig.workflow_steps` 时做映射。
    #[derive(Debug, Clone, Deserialize)]
    pub struct RuntimeWorkflowStep {
        #[serde(default)]
        pub id: String,
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub description: String,
        #[serde(default)]
        pub order: u32,
    }

    /// runtime.yaml 的完整配置子集。
    ///
    /// **只解析静态配置段**；`kpi_sources` 为废弃配置（见模块头部注释），不解析。
    /// 段字段全部 `#[serde(default)]`：缺段 / 旧文件不崩，缺失段由 A2 回退既有逻辑。
    #[derive(Debug, Clone, Default, Deserialize)]
    pub struct RuntimeKpiConfig {
        #[serde(default)]
        pub domain_pack_id: String,
        #[serde(default)]
        pub kpi_definitions: Vec<RuntimeKpiDefinition>,
        #[serde(default)]
        pub dashboard_cards: Vec<RuntimeDashboardCard>,
        /// 自动化规则（复用 `CapabilityPackAutomationRule`，直接对接规则引擎）
        #[serde(default)]
        pub automation_rules: Vec<CapabilityPackAutomationRule>,
        /// 实体类型（白名单、用于 kind 判定）
        #[serde(default)]
        pub entity_types: Vec<String>,
        /// KPI 计算定义（复用 `KpiCalculationDef`）
        #[serde(default)]
        pub kpi_calculations: Vec<KpiCalculationDef>,
        /// 工作流用户输入字段（复用 `WorkflowInputField`）
        #[serde(default)]
        pub input_fields: Vec<WorkflowInputField>,
        /// 是否需要审批
        #[serde(default)]
        pub requires_approval: bool,
        /// 工作流步骤（独立段结构，含 id）
        #[serde(default)]
        pub workflow_steps: Vec<RuntimeWorkflowStep>,
        /// 校验规则（独立段结构，非 `ValidationDef`）
        #[serde(default)]
        pub validations: Vec<RuntimeValidation>,
    }

    impl RuntimeKpiConfig {
        /// KPI 元数据是否可用（空 ⇒ 调用方退回既有配置，避免把面板清空）。
        pub fn has_kpi_metadata(&self) -> bool {
            !self.kpi_definitions.is_empty()
        }

        /// 按 key 查元数据。
        pub fn definition(&self, key: &str) -> Option<&RuntimeKpiDefinition> {
            self.kpi_definitions.iter().find(|d| d.key == key)
        }
    }

    /// 读取域包内 runtime 配置（`{manifest.runtime}`，缺省 runtime.yaml）。
    ///
    /// 失败**不静默**：文件缺失 / 解析失败一律 `warn` 留痕（约束类输入失效必须
    /// 可观测），返回 `None` 由调用方兜底。
    pub fn load_capability_pack_runtime(
        capability_pack_dir: &Path,
        manifest: &CapabilityPackManifest,
    ) -> Option<RuntimeKpiConfig> {
        let path = capability_pack_dir.join(&manifest.runtime);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                tracing::warn!(
                    "[domain-pack] {} runtime 配置不可读（KPI 元数据回退既有配置）: {e}",
                    path.display()
                );
                return None;
            },
        };
        match serde_yaml::from_str::<RuntimeKpiConfig>(&raw) {
            Ok(cfg) => {
                // 完整性自检：runtime.yaml 的 domain_pack_id 与 manifest.id 不一致 ⇒
                // 元数据可能被错配到别的域包（面板会显示别人的 KPI 名称）。
                if !cfg.domain_pack_id.is_empty() && cfg.domain_pack_id != manifest.id {
                    tracing::warn!(
                        "[domain-pack] {} 的 domain_pack_id={} 与 manifest.id={} 不一致，请核对",
                        path.display(),
                        cfg.domain_pack_id,
                        manifest.id
                    );
                }
                Some(cfg)
            },
            Err(e) => {
                tracing::warn!("[domain-pack] {} runtime 解析失败: {e}", path.display());
                None
            },
        }
    }
}

// ── 注册与 seed ───────────────────────────────────────────────────

/// 将域包注册进 opc_capability_packs 表（存在则按 version 判断是否升级）。
pub async fn upsert_capability_pack_registry(
    db: &DatabaseConnection,
    m: &CapabilityPackManifest,
) -> Result<(), String> {
    use axagent_entities::opc_capability_packs;
    use sea_orm::*;

    let now = now_ts();
    // P1-5：保留用户手动禁用状态——DB 已有记录时以 DB enabled 为准，
    // manifest.enabled 仅首次插入生效（否则重启会把用户禁用的域包自动重新启用）。
    let existing = opc_capability_packs::Entity::find_by_id(&m.id).one(db).await.ok().flatten();
    let effective_enabled = existing.map(|e| e.enabled != 0).unwrap_or(m.enabled);
    let am = opc_capability_packs::ActiveModel {
        id: Set(m.id.clone()),
        name: Set(m.name.clone()),
        icon: Set(m.icon.clone()),
        description: Set(m.description.clone()),
        version: Set(m.version),
        enabled: Set(effective_enabled as i32),
        pack_path: Set(format!("{CAPABILITY_PACKS_DIR}/{}", m.id)),
        installed_at: Set(now),
        updated_at: Set(now),
    };
    opc_capability_packs::Entity::insert(am)
        .on_conflict(
            sea_query::OnConflict::column(opc_capability_packs::Column::Id)
                .update_column(opc_capability_packs::Column::Name)
                .update_column(opc_capability_packs::Column::Icon)
                .update_column(opc_capability_packs::Column::Description)
                .update_column(opc_capability_packs::Column::Version)
                .update_column(opc_capability_packs::Column::Enabled)
                .update_column(opc_capability_packs::Column::PackPath)
                .update_column(opc_capability_packs::Column::UpdatedAt)
                .to_owned(),
        )
        .exec_without_returning(db)
        .await
        .map_err(|e| format!("upsert capability_pack: {e}"))?;
    Ok(())
}

/// 设置域包启用/停用状态 —— 写入 DB `opc_capability_packs.enabled`。
///
/// 权威存储是 DB（P1-5：manifest.enabled 仅首装生效，seed 后用户手动状态以 DB 为准），
/// 因此开关落盘这里，而不是改 manifest.yaml。
pub async fn set_capability_pack_enabled(
    db: &DatabaseConnection,
    id: &str,
    enabled: bool,
) -> Result<(), String> {
    use axagent_entities::opc_capability_packs;
    use sea_orm::{EntityTrait, Set};

    let now = now_ts();
    let am = opc_capability_packs::ActiveModel {
        id: Set(id.to_string()),
        enabled: Set(enabled as i32),
        updated_at: Set(now),
        ..Default::default()
    };
    opc_capability_packs::Entity::update(am)
        .exec(db)
        .await
        .map_err(|e| format!("set capability_pack enabled(id={id}): {e}"))?;
    Ok(())
}

/// 读取全部域包的启用状态（id → enabled）。
///
/// 供 market_list 等 command 层复用：避免 command 直接访问 `opc_capability_packs` 实体，
/// 保持「command 只经 analysis-engine / dao 访问 DB」的分层纪律（对齐 `commands-no-direct-db`）。
/// 不存在于 DB 的域包由调用方回退 manifest 默认。
pub async fn list_capability_pack_enabled_map(
    db: &DatabaseConnection,
) -> Result<std::collections::HashMap<String, bool>, String> {
    use axagent_entities::opc_capability_packs;
    use sea_orm::EntityTrait;

    let rows = opc_capability_packs::Entity::find()
        .all(db)
        .await
        .map_err(|e| format!("list capability_pack enabled map: {e}"))?;
    Ok(rows.into_iter().map(|r| (r.id, r.enabled != 0)).collect())
}

/// 域包完整 seed：扫描目录 → 注册表（opc_capability_packs）。
///
/// ⚠️ 架构变更：域包工作流已迁移至手动定义的 seed 文件（见 mod.rs `seed_opc_capability_packs_from_seed_files`），
/// 本函数仅负责 manifest 注册（opc_capability_packs 表），不再从 YAML 加载工作流。
///
/// 返回 seed 的域包 id 列表。
pub async fn ensure_opc_capability_packs_seeded(
    db: &DatabaseConnection,
    base_dir: &Path,
) -> Result<Vec<String>, String> {
    use axagent_entities::opc_capability_packs;
    use sea_orm::EntityTrait;

    let manifests = scan_capability_packs(base_dir);
    let mut seeded = Vec::new();

    for m in manifests {
        // 版本判断：读 DB 现有记录（seed 前，避免 registry upsert 自引用）
        let existing = opc_capability_packs::Entity::find_by_id(&m.id).one(db).await.ok().flatten();
        // P1-5：生效 enabled 以 DB 为准（用户手动禁用优先于 manifest），manifest 仅首装生效
        let effective_enabled = existing.as_ref().map(|e| e.enabled != 0).unwrap_or(m.enabled);
        let already_seeded = existing.as_ref().map(|e| e.version >= m.version).unwrap_or(false);

        // 注册表 upsert（记录当前包状态，enabled 保留 DB 用户状态）
        upsert_capability_pack_registry(db, &m).await?;

        if already_seeded {
            seeded.push(m.id.clone());
            continue;
        }

        if !effective_enabled {
            tracing::info!("[domain-pack] {} 已禁用，跳过注册", m.id);
            continue;
        }

        // 域包工作流已由手动定义的 seed 文件生成（seed_opc_capability_packs_from_seed_files），
        // 此处仅注册 manifest 到 opc_capability_packs 表。
        tracing::info!(
            "[domain-pack] {} manifest 注册完成（v{}，工作流由手动 seed 文件提供）",
            m.id,
            m.version
        );
        seeded.push(m.id.clone());
    }
    Ok(seeded)
}

/// 供测试/工具使用：给定域包 id 的包目录路径。
pub fn capability_pack_dir(base_dir: &Path, id: &str) -> PathBuf {
    base_dir.join(id)
}

// ── .opcip 导出/导入 ─────────────────────────────────────────────
//
// .opcip = Domain Pack 的 zip 归档（manifest.yaml + workflows/*.yaml）。
// 导出：打包域包目录 → zip 文件；导入：解包 → 注册 → seed。

/// 导出域包为 .opcip 归档。
/// 返回生成的文件路径。
pub async fn export_capability_pack(
    base_dir: &Path,
    id: &str,
    out_dir: &Path,
) -> Result<String, String> {
    let src = capability_pack_dir(base_dir, id);
    if !src.is_dir() {
        return Err(format!("域包不存在: {}", src.display()));
    }

    let file_path = out_dir.join(format!("{id}.opcip"));
    let file = std::fs::File::create(&file_path).map_err(|e| format!("创建归档失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 递归打包目录（zip 内部用正斜杠相对路径）
    fn add_dir(
        zip: &mut zip::ZipWriter<std::fs::File>,
        opts: &zip::write::SimpleFileOptions,
        _base: &Path,
        dir: &Path,
        prefix: &str,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let zip_name = format!("{prefix}{name}");
            if path.is_dir() {
                add_dir(zip, opts, _base, &path, &format!("{zip_name}/"))?;
            } else {
                let content = std::fs::read(&path).map_err(|e| format!("读取文件失败: {e}"))?;
                zip.start_file(zip_name, *opts).map_err(|e| format!("写入归档失败: {e}"))?;
                zip.write_all(&content).map_err(|e| format!("写入归档失败: {e}"))?;
            }
        }
        Ok(())
    }

    // 打包：zip 内路径以 {id}/ 为前缀（如 "finance_invest/manifest.yaml"），
    // 保证导入时能识别单一顶层域包目录。
    add_dir(&mut zip, &opts, &src, &src, &format!("{id}/"))
        .map_err(|e| format!("打包失败: {e}"))?;
    zip.finish().map_err(|e| format!("归档完成失败: {e}"))?;
    tracing::info!("[domain-pack] 导出 {id} → {}", file_path.display());
    Ok(file_path.to_string_lossy().to_string())
}

/// 导入 .opcip 域包：解包到 app_dir/config/opc/domain_packs/{id}/ 并注册 seed。
/// 返回导入的域包 id。
pub async fn import_capability_pack(
    db: &DatabaseConnection,
    app_dir: &Path,
    archive_path: &Path,
) -> Result<String, String> {
    // P1-12：兼容目录导入（市场页把域包目录路径当归档传）。
    // 目录内应含 manifest.yaml（或其子目录含），直接拷贝到 industries/ 并 seed。
    if archive_path.is_dir() {
        let Some(id) = archive_path.file_name().map(|s| s.to_string_lossy().to_string()) else {
            return Err("无法从目录名确定域包 id".to_string());
        };
        let industries_root = app_dir.join(CAPABILITY_PACKS_DIR);
        let target = industries_root.join(&id);
        // 源目录可能是 {id}/（含 manifest）或 {id}/workflows 的父目录，先探测 manifest 位置
        let manifest_candidate = if archive_path.join("manifest.yaml").is_file() {
            archive_path.to_path_buf()
        } else if archive_path.parent().map(|p| p.join("manifest.yaml").is_file()).unwrap_or(false)
        {
            archive_path.parent().unwrap().to_path_buf()
        } else {
            return Err(format!(
                "{} 目录内未找到 manifest.yaml，不是有效的域包目录",
                archive_path.display()
            ));
        };
        copy_dir_recursive(&manifest_candidate, &target)
            .map_err(|e| format!("拷贝域包目录失败: {e}"))?;
        tracing::info!("[domain-pack] 目录导入 {id} → {}", target.display());
        let seeded = ensure_opc_capability_packs_seeded(db, &industries_root).await?;
        if !seeded.contains(&id) {
            tracing::info!("[domain-pack] {id} 已存在（版本一致），视为导入成功");
        }
        return Ok(id);
    }

    let file = std::fs::File::open(archive_path).map_err(|e| format!("打开归档失败: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("解析归档失败: {e}"))?;

    // 目标目录：app_dir/config/opc/domain_packs/{id}
    let target_root = app_dir.join(CAPABILITY_PACKS_DIR);
    std::fs::create_dir_all(&target_root).map_err(|e| format!("创建目录失败: {e}"))?;

    // 解包所有条目，记录顶层目录（域包 id，通常只有一个）
    let mut top_dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut has_manifest = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("读取条目失败: {e}"))?;
        let entry_name = entry.name().to_string();
        // P2-3：zip-slip 防护——拒绝绝对路径与 `..` 穿越（恶意 .opcip 可写任意目录）
        let normalized = entry_name.replace('\\', "/");
        if std::path::Path::new(&normalized).is_absolute()
            || normalized.split('/').any(|c| c == "..")
        {
            return Err(format!("归档内存在非法路径，已拒绝解包: {entry_name}"));
        }
        if entry.is_dir() {
            continue;
        }
        // 顶层目录 = 域包 id（zip_name 形如 "finance_invest/manifest.yaml"）
        let top = entry_name.split('/').next().unwrap_or("").to_string();
        if top.is_empty() {
            continue;
        }
        top_dirs.insert(top.clone());
        if entry_name.ends_with("manifest.yaml") {
            has_manifest = true;
        }
        let out_path = target_root.join(&entry_name);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        let mut out = std::fs::File::create(&out_path).map_err(|e| format!("创建文件失败: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("解包失败: {e}"))?;
    }

    if !has_manifest {
        return Err("归档内未找到 manifest.yaml，不是有效的 .opcip 域包".to_string());
    }
    if top_dirs.len() != 1 {
        return Err(format!("归档应只含一个域包目录，实际 {} 个: {top_dirs:?}", top_dirs.len()));
    }
    let id = top_dirs.into_iter().next().unwrap();
    tracing::info!("[domain-pack] 导入 {id} → {}", target_root.display());

    // 注册 + seed（域包工作流现已由 Rust 代码生成，仅注册 manifest）
    let seeded = ensure_opc_capability_packs_seeded(db, &target_root).await?;
    if !seeded.contains(&id) {
        tracing::info!("[domain-pack] {id} 已存在，跳过 seed");
    }
    Ok(id)
}

// ── 目录解析与递归拷贝（随域包引擎一并下沉；原 `commands/opc_workflows/mod.rs`）──

/// 域包根目录（相对仓库根，由调用方拼接）
pub fn capability_packs_base_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(CAPABILITY_PACKS_DIR)
}

/// 域包目录解析：app_dir/CAPABILITY_PACKS_DIR → 旧目录 → 仓库根 fallback。
///
/// # 存量兼容（2026-09-15 目录更名）
///
/// 包根目录已由 [`LEGACY_CAPABILITY_PACKS_DIR`] 更名为 [`CAPABILITY_PACKS_DIR`]。
/// 已安装的 `.opcip` 包可能仍落在**旧目录**，故此处显式回退读取旧目录 ——
/// 否则升级后表现为「包凭空消失」，且**不报任何错**（判据：目录改名必须配存量回退）。
///
/// ⚠ 本条注释**不得复写旧路径字面量**：一旦写全，重跑更名脚本会把「旧目录」也改成新目录，
/// 回退分支从此永不执行（已实测发生过一次，回退静默退化为指向同一目录）。
pub fn resolve_capability_packs_dir(app_dir: Option<&std::path::Path>) -> std::path::PathBuf {
    if let Some(dir) = app_dir {
        let candidate = dir.join(CAPABILITY_PACKS_DIR);
        if candidate.is_dir() {
            return candidate;
        }
        let legacy = dir.join(LEGACY_CAPABILITY_PACKS_DIR);
        if legacy.is_dir() {
            tracing::warn!(
                "[opc-workflows] 旧包目录 {} 仍在使用（{} 不存在），已自动回退。\
                 建议把包迁移到新目录，该回退将在后续版本移除。",
                legacy.display(),
                candidate.display()
            );
            return legacy;
        }
    }
    capability_packs_base_dir()
}

/// 递归拷贝目录（仅文件与子目录，保持结构）
pub fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

// ── 能力集封闭校验（承诺 ⊆ 实际）+ 「提示增加域」 ──────────────────────
//
// 期一·3：对账「域包 manifest 承诺能力」与「该域包实际能力」，产出结构化报告。
// 校验器是**纯函数**（不依赖 capability_indexer trait / 数据库），实际侧数据由
// wiring 层（seed / import / 索引重建）准备好后灌入，保证单一职责 + 可单测。
// 本文件为权威 schema 所在（CapabilityPackManifest / CapabilityPackCapabilityClaim /
// CapabilityMatchPredicate），校验器就近放置，避免跨 crate 重复类型。

use axagent_harness::capability::CapabilityPassportDto;

/// 能力封闭校验报告——一次对账的完整结果（供前端展示 + 日志）。
/// `closed == true` 表示「承诺 ⊆ 实际」成立且无待注册域提案。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityPackCapabilityReport {
    /// 被校验的域包 id
    pub domain_pack_id: String,
    /// 域包声明的本体域归属
    pub domain: Option<CapabilityDomain>,
    /// 是否闭合：承诺 ⊆ 实际且无未落本体域的实际能力
    pub closed: bool,
    /// 承诺了但没有实际能力承接的清单（承诺 ⊄ 实际，缺口）
    pub missing: Vec<MissingCapability>,
    /// 该域包实际存在、但未被任何承诺覆盖的能力（实际 ⊄ 承诺）
    pub uncovered: Vec<UncoveredCapability>,
    /// 「提示增加域」提案：uncovered 能力若归属域未落入现存本体域，
    /// 汇总为结构化「待注册域」提案（引导走新增全局域流程，非硬失败）。
    pub domain_proposals: Vec<DomainRegistrationProposal>,
}

/// 承诺缺口：域包声明了能力，但在该域包实际能力集里找不到承接。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingCapability {
    /// 承诺的能力来源（tool / skill / mcp_tool / workflow / agent）
    pub source: super::capability::CapabilitySource,
    /// 承诺的能力类型（业务自定义标签，如 `opc_tool` / `analysis_source`）
    pub capability_type: String,
    /// 匹配谓词的人类可读描述（All / id_prefix:xxx / id_exact:xxx）
    pub predicate_desc: String,
    /// 在「承诺 ⊆ 实际」对账中未命中的原因提示
    pub note: String,
}

/// 未承诺的实际能力：域包实际能力集里没有被任何承诺覆盖的项。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UncoveredCapability {
    /// 能力护照 id（`{kind}:{id}`）
    pub capability_id: String,
    pub name: String,
    pub description: String,
    /// 该护照归属的能力域（若未落入现存本体域 → 触发「提示增加域」）
    pub domain: CapabilityDomain,
}

/// 「提示增加域」提案——动态扩展口，非硬失败。
/// 当出现未落现存本体域的实际能力时，据此引导注册全局域并挂靠域包。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainRegistrationProposal {
    /// 建议注册的域 id（本质域 slug，从护照归属域派生）
    pub domain_id: String,
    /// 该提案来源的能力来源标签（触发能力护照 id，去重、排序后的稳定串）
    pub capability_sources: Vec<String>,
    /// 触发的实际能力示例（能力名，去重）
    pub example_scenarios: Vec<String>,
    /// 建议的归属说明（当前护照归属域 → 建议域）
    pub suggestion: String,
}

/// 校验器：对账「manifest 承诺能力」⊆「该域包实际能力」，并产出「提示增加域」提案。
///
/// # 参数
/// - `manifest`：被校验的域包 manifest（含 `domain` + `capabilities` 承诺集合）。
/// - `actual_passports`：该域包的真实能力护照。来源约定（wiring 层灌入）：
///   - workflow 承诺：`capability_indexer` 中该域包反指的 workflow 护照
///     （护照 `domain_pack_id` 反向指针，期一·2 已注入）；
///   - tool 承诺：该域包 harness workflow 的 `tool_defs` 收敛成的工具护照
///     （工具共享、不背单指针，故按「承诺 id 在全局护照索引存在性」对账）。
/// - `existing_user_domains`：现存允许挂靠的本体域用户子集。
///
/// # 返回
/// 结构化 `CapabilityPackCapabilityReport`：
/// - `missing`：承诺但实际未命中的能力（承诺 ⊄ 实际，缺口，不静默）；
/// - `uncovered`：实际存在但未被承诺覆盖的能力（实际 ⊄ 承诺）；
/// - `domain_proposals`：其中归属域未落现存本体域的 → 「提示增加域」提案
///   （非硬失败，动态扩展口）。
pub fn assert_capability_pack_capability_closed(
    manifest: &CapabilityPackManifest,
    actual_passports: &[CapabilityPassportDto],
    existing_user_domains: &[CapabilityDomain],
) -> CapabilityPackCapabilityReport {
    // 1) 承诺 ⊆ 实际：逐条 claim 匹配实际护照
    let mut missing = Vec::new();
    for claim in &manifest.capabilities {
        let matched = match &claim.predicate {
            CapabilityMatchPredicate::All => {
                actual_passports.iter().any(|p| source_matches(p, &claim.source))
            },
            CapabilityMatchPredicate::IdPrefix { prefix } => actual_passports.iter().any(|p| {
                source_matches(p, &claim.source) && predicate_bare(prefix, p, &claim.source)
            }),
            CapabilityMatchPredicate::IdExact { id } => actual_passports
                .iter()
                .any(|p| source_matches(p, &claim.source) && p.capability_id == *id),
        };
        if !matched {
            let desc = claim_id_of(&claim.predicate);
            let note = if desc.is_empty() {
                "承诺覆盖该来源全部能力，但域包实际护照中无对应来源".to_string()
            } else {
                format!("承诺 `{desc}` 在域包实际护照中未命中")
            };
            missing.push(MissingCapability {
                source: claim.source.clone(),
                capability_type: claim.capability_type.clone(),
                predicate_desc: desc,
                note,
            });
        }
    }

    // 2) 实际 ⊄ 承诺：找出该域包「实际存在但未被任何承诺覆盖」的能力
    let mut uncovered = Vec::new();
    for passport in actual_passports {
        let covered = manifest.capabilities.iter().any(|claim| match &claim.predicate {
            CapabilityMatchPredicate::All => source_matches(passport, &claim.source),
            CapabilityMatchPredicate::IdPrefix { prefix } => {
                source_matches(passport, &claim.source)
                    && predicate_bare(prefix, passport, &claim.source)
            },
            CapabilityMatchPredicate::IdExact { id } => {
                source_matches(passport, &claim.source) && &passport.capability_id == id
            },
        });
        // uncovered 对账排除两类：
        // ① 已有属地本体（domain_pack_id 反指本域包）——期一·2 已注入，视为「已归属」不重复提示；
        // ② Tool 共享来源——工具不背单指针，其收敛由「承诺 id 在全局索引存在性」对账承担，
        //    不触发「提示增加域」（否则全局工具护照会成片制造域名提案噪音）。
        let pack_owned =
            passport.domain_pack_id.as_deref().is_some_and(|id| id == manifest.id.as_str());
        let is_shared_tool = source_matches(passport, &super::capability::CapabilitySource::Tool);
        if !covered && !passport.visibility.is_system_only() && !pack_owned && !is_shared_tool {
            uncovered.push(UncoveredCapability {
                capability_id: passport.capability_id.clone(),
                name: passport.name.clone(),
                description: passport.description.clone(),
                domain: passport.domain,
            });
        }
    }

    // 3) 「提示增加域」：uncovered 中归属域未落现存本体域的能力 → 聚合提案
    let mut domain_proposals: Vec<DomainRegistrationProposal> = Vec::new();
    let mut idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for cap in &uncovered {
        if existing_user_domains.contains(&cap.domain) {
            continue;
        }
        let domain_id = format!(
            "{}_{}",
            manifest.domain.map(|d| d.as_str()).unwrap_or("general"),
            cap.domain.as_str()
        );
        let pos = *idx.entry(domain_id.clone()).or_insert_with(|| {
            domain_proposals.push(DomainRegistrationProposal {
                domain_id: domain_id.clone(),
                capability_sources: Vec::new(),
                example_scenarios: Vec::new(),
                suggestion: format!(
                    "{}/{}：能力归属域 `{}` 未落现存本体域，建议注册全局域后挂靠",
                    manifest.id,
                    cap.capability_id,
                    cap.domain.as_str()
                ),
            });
            domain_proposals.len() - 1
        });
        if !domain_proposals[pos].capability_sources.contains(&cap.capability_id) {
            domain_proposals[pos].capability_sources.push(cap.capability_id.clone());
        }
        if !domain_proposals[pos].example_scenarios.contains(&cap.name) {
            domain_proposals[pos].example_scenarios.push(cap.name.clone());
        }
    }
    // 确定性输出：源 id 与示例均排序
    for proposal in &mut domain_proposals {
        proposal.capability_sources.sort();
        proposal.example_scenarios.sort();
    }
    domain_proposals.sort_by(|a, b| a.domain_id.cmp(&b.domain_id));

    // 4) 汇总：承诺 ⊆ 实际成立 且 无待注册域提案 ⇒ closed
    let closed = missing.is_empty() && domain_proposals.is_empty();

    CapabilityPackCapabilityReport {
        domain_pack_id: manifest.id.clone(),
        domain: manifest.domain,
        closed,
        missing,
        uncovered,
        domain_proposals,
    }
}

/// 将匹配谓词渲染为人可读的承诺 id 描述（用于报告）。
fn claim_id_of(predicate: &CapabilityMatchPredicate) -> String {
    match predicate {
        CapabilityMatchPredicate::All => String::new(),
        CapabilityMatchPredicate::IdPrefix { prefix } => format!("id_prefix:{prefix}"),
        CapabilityMatchPredicate::IdExact { id } => format!("id_exact:{id}"),
    }
}

/// 该护照是否属于某能力来源（按护照 id 前缀判定）。
fn source_matches(
    passport: &CapabilityPassportDto,
    source: &super::capability::CapabilitySource,
) -> bool {
    match source {
        super::capability::CapabilitySource::Tool => passport.capability_id.starts_with("tool:"),
        super::capability::CapabilitySource::Skill => passport.capability_id.starts_with("skill:"),
        super::capability::CapabilitySource::McpTool => passport.capability_id.starts_with("mcp:"),
        super::capability::CapabilitySource::Workflow => {
            passport.capability_id.starts_with("workflow:")
        },
        super::capability::CapabilitySource::Agent => {
            passport.capability_id.starts_with("agent:")
                || passport.capability_id.starts_with("agent_role:")
        },
    }
}

/// 承诺前缀与护照 id 的裸名匹配：剥离 `{kind}:` 前缀后做 `ends_with` 判定。
fn predicate_bare(
    prefix: &str,
    passport: &CapabilityPassportDto,
    source: &super::capability::CapabilitySource,
) -> bool {
    let source_prefix = match source {
        super::capability::CapabilitySource::McpTool => "mcp:",
        super::capability::CapabilitySource::Skill => "skill:",
        super::capability::CapabilitySource::Workflow => "workflow:",
        super::capability::CapabilitySource::Agent => "agent:",
        super::capability::CapabilitySource::Tool => "tool:",
    };
    passport
        .capability_id
        .strip_prefix(source_prefix)
        .map(|id| id.ends_with(prefix))
        .unwrap_or(false)
}

/// 装配助手（统一入口）—— 枚举域包目录全部 manifest，结合全局护照跑能力封闭校验。
///
/// wiring 层（索引重建 `register_all_capabilities` / `opc_import_capability_pack` /
/// `ensure_opc_workflows_seeded` 三处）在护照收集完成后调用，共用本函数避免重复接线。
/// 对账口径与 `assert_capability_pack_capability_closed` 一致：工具共享、不背单指针，
/// 故实际护照取「全局工具护照 ∪ 反指本域包的动态（workflow/skill/agent）护照」。
/// 校验为**审计性质**（非硬失败）：报告用于日志/前端提示，不阻断任何流程。
pub async fn audit_capability_packs_capability<I: axagent_harness::CapabilityIndexer>(
    indexer: &I,
    base_dir: &std::path::Path,
) -> Vec<CapabilityPackCapabilityReport> {
    let all = indexer.list_passports().await;
    let user_domains: Vec<CapabilityDomain> = axagent_harness::domain_registry::DOMAIN_NODES
        .iter()
        .filter(|n| !n.domain.is_system())
        .map(|n| n.domain)
        .collect();
    scan_capability_packs(base_dir)
        .into_iter()
        .filter(|m| m.enabled)
        .map(|manifest| {
            let actual: Vec<CapabilityPassportDto> = all
                .iter()
                .filter(|p| {
                    p.capability_id.starts_with("tool:")
                        || p.domain_pack_id.as_deref() == Some(manifest.id.as_str())
                })
                .cloned()
                .collect();
            assert_capability_pack_capability_closed(&manifest, &actual, &user_domains)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! 期一·3 能力集封闭校验：manifest capabilities 反序列化 + 承诺 ⊆ 实际闭合判定。
    //! 测试必须追加在文件末尾（禁 `clippy::items_after_test_module`）。

    use std::path::PathBuf;

    use crate::opc::capability::CapabilitySource;
    use axagent_harness::Visibility;

    use super::*;

    /// 定位仓库根的域包目录。
    ///
    /// 优先用生产装配路径 `resolve_capability_packs_dir(None)`；当它以相对路径行不被命中
    /// （如 `cargo test -p axagent-analysis-engine` 在 `src-tauri` 作 cwd 运行时，
    /// 相对路径解析到 `src-tauri/config/opc/domain_packs` 而该目录不存在）——
    /// 则从当前 cwd 向上回溯仓库根，凑 `config/opc/domain_packs`。
    fn capability_packs_base_for_test() -> PathBuf {
        let resolved = super::resolve_capability_packs_dir(None);
        if resolved.is_dir() {
            return resolved;
        }
        let cwd = std::env::current_dir().expect("测试进程当前目录不可读");
        let mut dir = cwd.as_path();
        loop {
            let candidate = dir.join(super::CAPABILITY_PACKS_DIR);
            if candidate.is_dir() {
                return candidate;
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => break,
            }
        }
        panic!("找不到 {}（仓库根 config/opc/domain_packs）", super::CAPABILITY_PACKS_DIR)
    }

    /// 验收：14 个内置域包 manifest 的 `capabilities` 承诺能正确从 YAML 反序列化，
    /// 且每条 predicate 都是 `IdExact`（本轮改动的关键验收点）。
    #[test]
    fn scan_builtin_capability_packs_parse_capabilities() {
        let base = capability_packs_base_for_test();
        let manifests = scan_capability_packs(&base);

        assert!(
            manifests.len() >= 14,
            "预期至少读到 14 个内置域包 manifest，实际 {}（base={}）",
            manifests.len(),
            base.display()
        );

        for manifest in &manifests {
            assert!(
                !manifest.capabilities.is_empty(),
                "域包 {} 的 capabilities 承诺不能为空",
                manifest.id
            );
            for claim in &manifest.capabilities {
                assert!(
                    matches!(claim.predicate, CapabilityMatchPredicate::IdExact { .. }),
                    "域包 {} 承诺应全部为 IdExact 精确匹配，实际 predicate={:?}",
                    manifest.id,
                    claim.predicate
                );
            }
        }
    }

    /// 验收：承诺 ⊆ 实际时校验器闭合。构造一条 IdExact tool 承诺 + 一个精确命中的
    /// 公共护照（归属用户域），断言 `missing` 为空且 `closed == true`。
    #[test]
    fn assert_closed_when_claim_satisfied() {
        let manifest = CapabilityPackManifest {
            id: "mock_pack".to_string(),
            name: "Mock Pack".to_string(),
            icon: "🐾".to_string(),
            description: String::new(),
            version: 1,
            enabled: true,
            analysis: "analysis.yaml".to_string(),
            learning: "learning.yaml".to_string(),
            runtime: "runtime.yaml".to_string(),
            template_prefix: None,
            domain: Some(CapabilityDomain::Finance),
            capabilities: vec![CapabilityPackCapabilityClaim {
                source: CapabilitySource::Tool,
                capability_type: "opc_tool".to_string(),
                predicate: CapabilityMatchPredicate::IdExact {
                    id: "tool:OpcListInvoices".to_string(),
                },
            }],
            office: None,
        };

        let actual = vec![CapabilityPassportDto {
            capability_id: "tool:OpcListInvoices".to_string(),
            domain: CapabilityDomain::Finance, // 用户域，避免触发「提示增加域」
            visibility: Visibility::Public,    // 公共，避免被 system_only 排除逻辑干扰
            ..Default::default()
        }];
        let user_domains = vec![CapabilityDomain::Finance];

        let report = assert_capability_pack_capability_closed(&manifest, &actual, &user_domains);

        assert!(report.missing.is_empty(), "承诺应被实际护照命中，missing={:?}", report.missing);
        assert!(report.closed, "承诺 ⊆ 实际 时应闭合，report={report:?}");
    }

    /// 验收（阶段 A1）：runtime.yaml 已落盘但曾被 serde 忽略的段（以 accounting 为锚）
    /// 现在能被 `RuntimeKpiConfig` 读取，不再静默丢弃。
    #[test]
    fn runtime_yaml_ignored_segments_now_parsed() {
        let base = capability_packs_base_for_test();
        let raw = std::fs::read_to_string(base.join("accounting/runtime.yaml"))
            .expect("读 accounting/runtime.yaml");
        let cfg: super::runtime_schema::RuntimeKpiConfig = serde_yaml::from_str(&raw)
            .expect("accounting/runtime.yaml 应能被 RuntimeKpiConfig 反序列化");

        // automation_rules：直接复用 CapabilityPackAutomationRule（与 accounting 段对齐）
        assert_eq!(cfg.automation_rules.len(), 2, "accounting 应有 2 条自动化规则");
        assert_eq!(cfg.automation_rules[0].id, "accounting_overdue_alert");
        assert!(cfg.automation_rules[0].enabled);

        // validations：独立段结构（entity_type/field/operator/value/message）
        // ⚠ 2026-09-19 同步：原硬编码 validate_accounting（金额为正、状态枚举）已迁入 YAML
        // （见 accounting/runtime.yaml 第 17 行注释）⇒ 由 2 条扩为 4 条。
        assert_eq!(cfg.validations.len(), 4);
        assert_eq!(cfg.validations[0].entity_type, "invoice");
        assert_eq!(cfg.validations[0].operator, "ge");
        assert_eq!(cfg.validations[0].value, serde_json::json!(0));

        // workflow_steps：独立段结构（含 id）
        assert_eq!(cfg.workflow_steps.len(), 4);
        assert_eq!(cfg.workflow_steps[0].id, "a-create");
        assert_eq!(cfg.workflow_steps[0].name, "创建发票");

        // 未落盘段 ⇒ default 空位（供 A2 缺失回退）
        assert!(cfg.entity_types.is_empty());
        assert!(cfg.kpi_calculations.is_empty());
        assert!(cfg.input_fields.is_empty());
        assert!(!cfg.requires_approval);

        // 既有段不受影响（防回归）
        assert_eq!(cfg.kpi_definitions.len(), 4);
        assert_eq!(cfg.dashboard_cards.len(), 3);
    }
}
