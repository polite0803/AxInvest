// SPDX-License-Identifier: AGPL-3.0-only

//! 域包装配/查询契约 —— manifest schema 家族 + `CapabilityPackRegistry` 窄 trait。
//!
//! 与 [`super::capability_pack_orchestration`]（运行时编排：分解/反思/进化）不同面：
//! 本模块只管「域包目录 → manifest 读数 → 能力封闭审计」的装配/查询，不碰编排。
//!
//! # DTO 上收（铁律 4）
//! `CapabilityPackManifest` 家族与能力封闭审计报告家族原住 `company-runtime`
//! （#159 搬迁后的域内实现），主壳（`src-tauri`）的 fleet 装配与能力索引初始化
//! 需要消费它们 ⇒ 按「共享 DTO 权威在 harness」上收，`company-runtime` 经
//! `pub use` 零 From 引用，主壳经 trait 对象消费（Cargo 边随 #36 终局拔除）。
//!
//! # `CapabilityCarrier` 消歧
//! harness [`super::capability::CapabilitySource`] 是「护照注册来源」
//! （Builtin/Plugin）；本模块的 `CapabilityCarrier` 是「承诺能力的承载形态」
//! （Tool/Skill/McpTool/Workflow/Agent，原 `company-runtime` 的 `CapabilitySource`）。
//! 语义不同，禁止合并。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::capability_indexer::CapabilityIndexer;

// ── 承诺能力的承载形态（原 company-runtime::opc::capability::CapabilitySource）──

/// 承诺能力的承载形态 —— manifest `capabilities[].source` 与审计缺口报告共用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityCarrier {
    Tool,
    Skill,
    McpTool,
    Workflow,
    Agent,
}

impl CapabilityCarrier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Skill => "skill",
            Self::McpTool => "mcp_tool",
            Self::Workflow => "workflow",
            Self::Agent => "agent",
        }
    }
}

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
    /// 供 `analysis_schema::load_capability_pack_analysis` 读取（capability_pack.rs:261）
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
    pub domain: Option<crate::capability::CapabilityDomain>,
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

/// 域包承诺能力条目 —— manifest 的 `capabilities` 元素。
///
/// 语义：域包声明「我应当提供这些能力」，与能力索引的**实际归属**对账。
/// 承诺 ⊆ 实际：不满足则提示缺失；实际 ⊄ 承诺：触发「提示增加域」。
#[derive(Debug, Clone, Deserialize)]
pub struct CapabilityPackCapabilityClaim {
    /// 能力**来源**（Tool / Skill / McpTool / Workflow / Agent）
    pub source: CapabilityCarrier,
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

fn default_analysis_file() -> String {
    "analysis.yaml".into()
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

// ── 能力集封闭校验报告（承诺 ⊆ 实际）+ 「提示增加域」 ────────────────

/// 能力封闭校验报告——一次对账的完整结果（供前端展示 + 日志）。
/// `closed == true` 表示「承诺 ⊆ 实际」成立且无待注册域提案。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityPackCapabilityReport {
    /// 被校验的域包 id
    pub domain_pack_id: String,
    /// 域包声明的本体域归属
    pub domain: Option<crate::capability::CapabilityDomain>,
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingCapability {
    /// 承诺的能力来源（tool / skill / mcp_tool / workflow / agent）
    pub source: CapabilityCarrier,
    /// 承诺的能力类型（业务自定义标签，如 `opc_tool` / `analysis_source`）
    pub capability_type: String,
    /// 匹配谓词的人类可读描述（All / id_prefix:xxx / id_exact:xxx）
    pub predicate_desc: String,
    /// 在「承诺 ⊆ 实际」对账中未命中的原因提示
    pub note: String,
}

/// 未承诺的实际能力：域包实际能力集里没有被任何承诺覆盖的项。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UncoveredCapability {
    /// 能力护照 id（`{kind}:{id}`）
    pub capability_id: String,
    pub name: String,
    pub description: String,
    /// 该护照归属的能力域（若未落入现存本体域 → 触发「提示增加域」）
    pub domain: crate::capability::CapabilityDomain,
}

/// 「提示增加域」提案——动态扩展口，非硬失败。
/// 当出现未落现存本体域的实际能力时，据此引导注册全局域并挂靠域包。
#[derive(Debug, Clone, Serialize)]
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

// ── 装配/查询窄 trait ─────────────────────────────────────────────

/// 域包装配/查询契约 —— 主壳消费域包 manifest 的唯一入口。
///
/// 实现方：`company-runtime`（域包读写的权威实现住它那里，本 trait 只收窄
/// 主壳消费面）；消费方：`src-tauri` 的 fleet 命令与能力索引初始化。
#[async_trait]
pub trait CapabilityPackRegistry: Send + Sync {
    /// 解析域包根目录：优先 `app_dir/CAPABILITY_PACKS_DIR`，回退 legacy 目录与仓库根。
    fn resolve_packs_dir(&self, app_dir: Option<&std::path::Path>) -> std::path::PathBuf;

    /// 读取单个域包目录的 manifest（解析失败返回 None，调用方自行兜底）。
    fn read_manifest(&self, pack_dir: &std::path::Path) -> Option<CapabilityPackManifest>;

    /// 扫描域包目录，返回所有 manifest（含是否启用；解析失败 warn 后跳过）。
    fn scan_manifests(&self, base_dir: &std::path::Path) -> Vec<CapabilityPackManifest>;

    /// 对全部启用域包跑能力封闭校验（承诺 ⊆ 实际），产出结构化报告（审计性质，不阻断）。
    async fn audit_capability_closure(
        &self,
        indexer: &dyn CapabilityIndexer,
        base_dir: &std::path::Path,
    ) -> Vec<CapabilityPackCapabilityReport>;
}
