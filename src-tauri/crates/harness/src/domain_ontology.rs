// SPDX-License-Identifier: AGPL-3.0-only

//! **finance 域业务本体**（瓶颈掘金）—— 「类 / 关系 / 公理」在**本域内**的唯一权威源。
//!
//! ## ⚠ 先读这一节：本模块**不是**整个系统的本体，也**不包含「能力域」**
//!
//! 2026-09-15 正名。此前模块头自称「瓶颈掘金领域本体 —— 唯一权威源」，读起来像
//! 「全系统的本体」；实测**两项事实与之不符**（判据 #218：核「自称覆盖范围」是否等于
//! 「实际覆盖范围」）：
//!
//! 1. **本模块只覆盖 9 个能力域里的 `finance` 一格。** `CLASSES` 全部 11 个类的
//!    `evidence` **无一例外**指向 `agency_experts/stock-analysis/*.md` 或
//!    `commands/bottleneck-calc.rhai`；顶层类 `IndustryElement`（产业要素）与
//!    `BottleneckForce`（瓶颈力）都是产业链 / 瓶颈掘金概念。另外 8 个域
//!    （`devops` / `ai_media` / `data_analysis` / …）**在本模块里没有任何本体内容**。
//! 2. **「能力域」这一层不在本模块**，见下。
//!
//! ### 域层在哪里（三个模块，各管一段，勿互抄）
//!
//! | 层 | 模块 | 内容 | 本模块与它的关系 |
//! |---|---|---|---|
//! | **L1 能力域**（9 值：8 业务域 + `system`） | [`crate::capability`] · `CapabilityDomain` | 枚举（唯一权威分类轴） | 本模块**只引它当注册表 key**（[`ONTOLOGY_DOMAINS`]，P1b §4.3.1 已批准的例外） |
//! | **L1 域元数据**（路径 / 顺序 / 别名） | [`crate::domain_registry`] | `DomainNode` 声明 | 同上 |
//! | **L2 功能集群**（27 簇） | [`crate::capability_clusters`] | `CapabilityCluster` 声明 | 同上 |
//! | **finance 域业务本体** | **本模块** | 类 / 关系 / 公理 / 权重常量 | —— |
//!
//! ⇒ 正确的三层理解：**能力域是「能力怎么归类」的轴**，本模块是**其中一个域的业务语义**。
//! 二者不是「两套并列词汇表」，是「上位结构 vs 某分支的内容」。
//! **不要把能力域并进本模块** —— `page_type.rs` 已记录 2026-09-14 因此造出并列词汇表的事故
//! （详见 `PLAN-domain-single-source.md` §9）。
//!
//! ## 与 [`crate::domain_semantics`] 的分工
//!
//! | 模块 | 层次 | 回答的问题 |
//! |---|---|---|
//! | [`crate::domain_semantics`] | 字段层（横向） | 「`confidence` 这个名字此刻指哪个概念、值域多少」 |
//! | **本模块** | 领域层（纵向） | 「这个领域里有哪些**类**、类之间允许什么**关系**、哪些**公理**成立」 |
//!
//! ## 为什么需要它（实证，非假设）
//!
//! 「瓶颈三力」（供给刚性 / 需求弹性 / 不可替代性）的定义式、权重与分档阈值
//! 在仓库里散成 **7 处**，其中两处**互相矛盾** —— 而这些矛盾此前**没有任何机制看得见**：
//!
//! | # | 位置 | 内容 |
//! |---|---|---|
//! | 1 | `src-tauri/src/commands/bottleneck-calc.rhai:114` | 顶层权重 fallback `0.35/0.35/0.30` |
//! | 2 | `src-tauri/src/commands/bottleneck-calc.rhai:121` | `composite = Σ wᵢ·fᵢ`（**加权**，经 Rust 权威函数 bottleneck_node_score） |
//! | 3 | `src-tauri/src/commands/strategy-scorer.rhai:212-214` | 顶层权重 fallback（**第二份脚本，独立一份**） |
//! | 4 | `src-tauri/src/commands/strategy-scorer.rhai:237` | `composite = Σ wᵢ·fᵢ` |
//! | 5 | `src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs:1344-1364` | 模板变量默认值 + 描述文本 |
//! | 6 | `src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:101` | `composite = 三力 / 3`（**等权**）← **与 2/4 矛盾** |
//! | 7 | `src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:102` | 分档 `80 / 60` ← **与 `bottleneck-calc.rhai:148` 的 `band_for_score` 分档逻辑矛盾** |
//!
//! ## 两条不同的处置（刻意的，勿"顺手统一"）
//!
//! - **数值副本**（1/3/5）＝ 同一口径被抄了多份 ⇒ **硬拦**：由
//!   `scripts/check-ontology-consistency.mjs` 比对全部副本，不一致即 CI 红。
//! - **口径分歧**（6/7）＝ 两个**不同定义**同时存在，消除它需要产品裁决
//!   ⇒ **只登记**（[`OPEN_DIVERGENCES`]），脚本报告但不拦。
//!   擅自统一其中一侧 = 替业务做语义决策。
//!
//! ## 定位
//!
//! 纯声明 + 纯查询，**无运行时状态、无 IO、无阻塞**（符合本 crate「零业务逻辑」约束；
//! 权重与阈值是**常量声明**，不是计算逻辑 —— 计算仍在 `.rhai` 侧）。
//!
//! ## ⚠ 当前接线状态（2026-09-15 实证复核）
//!
//! **本模块的「权威源」地位目前只对本模块自身成立。** 读之前先看这张表，
//! 不要把「声明齐备 + 测试全绿」误读成「已被生产消费」：
//!
//! **「门禁覆盖」列的含义是「数值漂移 / 引用腐烂会不会让 CI 红」，不是「有人消费」**——
//! 两者极易混读，故分成两列。
//!
//! | 声明 | 生产消费方 | 门禁覆盖 |
//! |---|---|---|
//! | `BOTTLENECK_WEIGHTS` | **1**（`seed_serenity.rs` 的 `let bw = …` 绑定处） | ✅ `SITES`（顶层权重兜底） |
//! | `format_weight` | **3**（同文件三处调用） | ✅ `WIRING` |
//! | `READINESS_BANDS` | **0**（真在分档的是 `.rhai`，本表只是比对基准） | ✅ `SITES`（分档阈值） |
//! | `*_PARTS` ×3 | **0** | ✅ `SITES`（**2026-09-15 补齐**，`rhai-force-parts` 站点） |
//! | `AXIOMS`（内容） | **0** | ✅ `enforced_by` 指向校验（守护者必须真实存在） |
//! | `CLASSES` / `OBJECT_PROPERTIES` / `METRICS`（内容） | **0** | ✅ 其 `evidence` 指针的文件存在性 |
//! | 查询接口（`class_by_id` / `subclasses_of` / `band_for_score` / …） | **0** | ❌ 无 |
//!
//! 四条推论（勿跳过）：
//!
//! 1. 真正算分的是 `.rhai`；本模块对分档**只是副本的比对基准**，无生产代码查询
//!    `band_for_score`；
//! 2. `*_PARTS` 的 3 组共 8 个权重，在 `.rhai` 里另有 **6 处 / 16 个**独立字面量
//!    （`bottleneck-calc.rhai` 的三力合成分 `supply_rigidity_score = …` /
//!    `demand_elasticity_score = …` / `irreplaceability_score = …`，以及
//!    `strategy-scorer.rhai` 的 `srs = …` / `des = …` / `irs = …`）。
//!    这 6 处**现已在门禁覆盖内**（改动任一侧而不改本模块 ⇒ CI 红）；
//! 3. [`AXIOMS`] 的 `enforced_by` 指向的**全是本模块的单元测试**，不是生产消费者 ⇒
//!    公理只保证「常量表自洽」，**不保证 `.rhai` 真按公理计算**。例如 A4 声明
//!    「composite 加权非等权」，若把 `bottleneck-calc.rhai` 的
//!    `let bottleneck_composite = …` 改成等权平均，A4 **不红、门禁也不红**
//!    （门禁只比各力的**权重数字**副本，而顶层权重数字原样未动）。
//!    `enforced_by` 的指向校验只保证「守护者没被改名/删除」，**不扩大**这条公理的覆盖范围；
//! 4. 门禁**只验到文件级、不验行号**：`evidence` 的 `:行号` 尾巴会被剥掉再判存在性。
//!    行号必然随重构漂移（判据 #186–#189）⇒ 只保证「文件还在」，**不保证锚点行还对**。
//!
//! 出处：`AUDIT-ontology-vs-palantir-2026-09-15.md` §2.2 / §2.4.1 / §4。
//!
//! ## 变更治理
//!
//! 改动 [`BOTTLENECK_WEIGHTS`] / [`READINESS_BANDS`] / `*_PARTS` 后，
//! `scripts/check-ontology-consistency.mjs` 会比对全部副本与新值；不同步即红。
//!
//! 具体站点（改名前先看这里，改名后须同步脚本的 `FORCE_LHS_ALIASES`）：
//! 顶层权重兜底在 `bottleneck-calc.rhai` 的 `let ws = if w_supply != () {…} else {…}`
//! 三连与 `strategy-scorer.rhai` 的 `to_f64(w_supply, …)` 三连；
//! **内部**权重在 `bottleneck-calc.rhai` 的 `let supply_rigidity_score = …` /
//! `let demand_elasticity_score = …` / `let irreplaceability_score = …`，
//! 以及 `strategy-scorer.rhai` 的 `let srs = …` / `let des = …` / `let irs = …`。
//! 分档阈值在两者的 `readiness_signal` 链。改了 `.rhai` 里的变量名 ⇒ 门禁报
//! 「抓不到站点」而非静默通过（站点清单是契约）。
//!
//! 本模块的测试另外强制「权重和 = 1」「分档无缝无洞」「类层级无环」「关系端点已声明」；
//! 门禁另查「`enforced_by` 指向的 `#[test] fn` 真实存在」「`evidence` / `site`
//! 指向的文件真实存在」（后两类 Rust 测试查不了，故放在 JS 门禁里）。

use crate::capability::CapabilityDomain;

/// 一个**类**（本体里的一种实体类型）。
///
/// **不变量**（由本模块测试锁定）：
/// - [`id`](Self::id) 全局唯一；
/// - [`parent`](Self::parent) 若存在，必须指向另一个**已声明**的类，且层级**无环**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OntoClass {
    /// 类 ID（PascalCase，如 `Chokepoint`）。
    pub id: &'static str,
    /// 中文标签。
    pub label: &'static str,
    /// 父类（`None` = 顶层）。构成 `subClassOf` 层级。
    pub parent: Option<&'static str>,
    /// 人读语义（一句话）。
    pub meaning: &'static str,
    /// 权威依据 `<仓库根相对路径>:<行号>`，多个用 ` | ` 分隔。
    pub evidence: &'static str,
}

/// 一个**对象属性**（类与类之间的关系），带 `domain` / `range`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectProperty {
    /// 属性 ID（snake_case，如 `occupies`）。
    pub id: &'static str,
    /// 中文标签。
    pub label: &'static str,
    /// 定义域：主体所属类 ID。
    pub domain: &'static str,
    /// 值域：客体所属类 ID。
    pub range: &'static str,
    /// 语义说明。
    pub meaning: &'static str,
    /// 权威依据（`路径:行`，多锚点用 ` | ` 分隔）。
    ///
    /// ⚠ 门禁 `scripts/check-ontology-consistency.mjs` **只校验文件存在，不校验行号**
    /// —— 行号必然随重构漂移（判据 #186–#189），锁行号会把门禁变成噪声源。
    pub evidence: &'static str,
}

/// 一个**度量**（数据属性）：某类身上的一个数值刻度 + 值域。
///
/// ⚠ 不派生 `Eq` —— 含 `f64` 字段，`f64` 不满足 `Eq`（只有 `PartialEq`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metric {
    /// 度量 ID。
    pub id: &'static str,
    /// 中文标签。
    pub label: &'static str,
    /// 归属类 ID（= 数据属性的 domain）。
    pub owner: &'static str,
    /// 量纲标签（人读）。
    pub unit: &'static str,
    /// 值域下界（含）。
    pub min: f64,
    /// 值域上界（含）。
    pub max: f64,
    /// 权威依据（`路径:行`，多锚点用 ` | ` 分隔）。
    ///
    /// ⚠ 门禁 `scripts/check-ontology-consistency.mjs` **只校验文件存在，不校验行号**
    /// —— 行号必然随重构漂移（判据 #186–#189），锁行号会把门禁变成噪声源。
    pub evidence: &'static str,
}

/// 一条**公理 / 约束**的声明（机器可校验的那部分由测试强制）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Axiom {
    /// 公理 ID。
    pub id: &'static str,
    /// 人读表述。
    pub statement: &'static str,
    /// 由哪个测试强制（**必须是本文件里真实存在的 `#[test] fn` 名**）。
    ///
    /// ⚠ 本字段是自由文本，Rust 侧只断言非空 ⇒ 改名/删除测试会让公理**静默失去守护者**。
    /// 故由 `scripts/check-ontology-consistency.mjs` 的 `checkEnforcedByTargets` 机器校验
    /// 「指向的 `#[test] fn` 真实存在」；改名而不改此处 ⇒ CI 红。
    /// 注意这只保证**守护者还在**，不扩大公理本身的覆盖范围（守护者可能只测常量表）。
    pub enforced_by: &'static str,
}

/// 一条**已登记的口径分歧**：权威口径 vs 某处仍在用的另一种口径。
///
/// 处置纪律：**只登记，不擅自统一**。两侧都可能"对"，取决于产品语义；
/// 统一任一侧等于替业务做决策（见模块头「两条不同的处置」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    /// 分歧 ID。
    pub id: &'static str,
    /// 涉及的概念（表面名）。
    pub concept: &'static str,
    /// 权威口径（本模块声明的那一侧）。
    pub authority: &'static str,
    /// 分歧现场（`路径:行`）。门禁只校验**文件**存在，行号会随重构漂移。
    pub site: &'static str,
    /// 该现场声称的口径。
    pub claim: &'static str,
    /// 状态。
    pub status: &'static str,
}

// ─────────────────────────── 类的层级 ───────────────────────────

/// 已声明类的全量清单 —— **本表的唯一权威源**。
///
/// 登记标准：每条必须有可核对的源码锚点（[`OntoClass::evidence`]）。
pub const CLASSES: &[OntoClass] = &[
    OntoClass {
        id: "IndustryElement",
        label: "产业要素",
        parent: None,
        meaning: "本体的顶层类：产业链语境下的一切要素",
        evidence: "src-tauri/agency_experts/stock-analysis/chain-decomposer.md:1",
    },
    OntoClass {
        id: "Industry",
        label: "行业",
        parent: Some("IndustryElement"),
        meaning: "券商/行情数据口径下的行业分类",
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:159",
    },
    OntoClass {
        id: "IndustryChain",
        label: "产业链",
        parent: Some("IndustryElement"),
        meaning: "一条产业趋势所展开的上下游链条",
        evidence: "src-tauri/agency_experts/stock-analysis/chain-decomposer.md:1",
    },
    OntoClass {
        id: "ChainStage",
        label: "产业链环节",
        parent: Some("IndustryChain"),
        meaning: "链条上的一个环节（节点），瓶颈判定的作用对象",
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:143",
    },
    OntoClass {
        id: "Chokepoint",
        label: "瓶颈环节",
        parent: Some("ChainStage"),
        meaning: "满足三力公理（供给刚性 ∧ 需求弹性 ∧ 不可替代性）的环节，是 ChainStage 的受限子类",
        evidence: "src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:15",
    },
    OntoClass {
        id: "Company",
        label: "公司",
        parent: Some("IndustryElement"),
        meaning: "A 股上市公司",
        evidence: "src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:76",
    },
    OntoClass {
        id: "Product",
        label: "产品",
        parent: Some("IndustryElement"),
        meaning: "公司产出物，产业链上下游依赖的载体",
        evidence: "src-tauri/agency_experts/stock-analysis/chain-decomposer.md:1",
    },
    OntoClass {
        id: "BottleneckForce",
        label: "瓶颈力",
        parent: None,
        meaning: "顶层类之一（与 IndustryElement 并列）；瓶颈判定的抽象度量维度（三力的共同父类）",
        evidence: "src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:15",
    },
    OntoClass {
        id: "SupplyRigidity",
        label: "供给刚性",
        parent: Some("BottleneckForce"),
        meaning: "供给端难以扩张的程度（集中度 / 技术壁垒 / 扩产周期）",
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:122",
    },
    OntoClass {
        id: "DemandElasticity",
        label: "需求弹性",
        parent: Some("BottleneckForce"),
        meaning: "下游需求真实且不可逆的程度（订单可见性 / 需求确定性）",
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:123",
    },
    OntoClass {
        id: "Irreplaceability",
        label: "不可替代性",
        parent: Some("BottleneckForce"),
        meaning: "替代方案缺失的程度（供应商数量 / 技术护城河）",
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:124",
    },
];

// ─────────────────────────── 关系（含 domain / range）───────────────────────────

/// 已声明对象属性的全量清单。
///
/// `domain` / `range` 必须指向 [`CLASSES`] 里**已声明**的类（由测试强制）。
pub const OBJECT_PROPERTIES: &[ObjectProperty] = &[
    ObjectProperty {
        id: "occupies",
        label: "处于",
        domain: "Company",
        range: "ChainStage",
        meaning: "某公司在产业链的某个环节上",
        evidence: "src-tauri/agency_experts/stock-analysis/candidate-mapper.md:17",
    },
    ObjectProperty {
        id: "produces",
        label: "生产",
        domain: "Company",
        range: "Product",
        meaning: "某公司生产某产品",
        evidence: "src-tauri/agency_experts/stock-analysis/chain-decomposer.md:1",
    },
    ObjectProperty {
        id: "depends_on",
        label: "上游依赖",
        domain: "Product",
        range: "Product",
        meaning: "产品对上游产品的依赖 —— 由此串成产业链的顺序",
        evidence: "src-tauri/agency_experts/stock-analysis/chain-decomposer.md:1",
    },
    ObjectProperty {
        id: "measured_by",
        label: "由…度量",
        domain: "Chokepoint",
        range: "BottleneckForce",
        meaning: "瓶颈由三力度量（恰好三条：供给刚性 / 需求弹性 / 不可替代性）",
        evidence: "src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:15",
    },
];

// ─────────────────────────── 度量 ───────────────────────────

/// 已声明度量的全量清单。
pub const METRICS: &[Metric] = &[
    Metric {
        id: "supply_rigidity_score",
        label: "供给刚性分",
        owner: "SupplyRigidity",
        unit: "0-100 分",
        min: 0.0,
        max: 100.0,
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:122 | src-tauri/src/commands/bottleneck-calc.rhai:148",
    },
    Metric {
        id: "demand_elasticity_score",
        label: "需求弹性分",
        owner: "DemandElasticity",
        unit: "0-100 分",
        min: 0.0,
        max: 100.0,
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:123 | src-tauri/src/commands/bottleneck-calc.rhai:148",
    },
    Metric {
        id: "irreplaceability_score",
        label: "不可替代性分",
        owner: "Irreplaceability",
        unit: "0-100 分",
        min: 0.0,
        max: 100.0,
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:124 | src-tauri/src/commands/bottleneck-calc.rhai:148",
    },
    Metric {
        id: "bottleneck_composite",
        label: "瓶颈综合分",
        owner: "Chokepoint",
        unit: "0-100 分",
        min: 0.0,
        max: 100.0,
        evidence: "src-tauri/src/commands/bottleneck-calc.rhai:125 | src-tauri/src/commands/bottleneck-calc.rhai:148",
    },
];

// ─────────────────────────── 公理 ───────────────────────────

/// 已声明公理（每条都由同名的测试强制，见 [`Axiom::enforced_by`]）。
pub const AXIOMS: &[Axiom] = &[
    Axiom {
        id: "A1",
        statement: "三力度量的值域均为 [0, 100] 且彼此一致",
        enforced_by: "test_metric_ranges_are_valid",
    },
    Axiom {
        id: "A2",
        statement: "三力顶层权重之和 = 1",
        enforced_by: "test_top_level_weights_sum_to_one",
    },
    Axiom {
        id: "A3",
        statement: "每一力的内部权重之和 = 1",
        enforced_by: "test_force_part_weights_sum_to_one",
    },
    Axiom {
        id: "A4",
        statement: "composite = Σ 顶层权重 × 对应力分（加权，非等权平均）",
        enforced_by: "test_composite_is_weighted_not_mean",
    },
    Axiom {
        id: "A5",
        statement: "分档阈值把 [0, 100] 切成无缝无洞的若干档，最低档下界 = 0",
        enforced_by: "test_readiness_bands_are_total_and_gapfree",
    },
    Axiom {
        id: "A6",
        statement: "Chokepoint 是 ChainStage 的子类（瓶颈首先必须是一个环节）",
        enforced_by: "test_chokepoint_is_subclass_of_chain_stage",
    },
    Axiom {
        id: "A7",
        statement: "瓶颈恰好由 3 条力度量；每条力各有且仅有 1 个度量、且顶层权重 > 0",
        enforced_by: "test_exactly_three_forces_match_weights",
    },
];

// ─────────────────────────── 权威数值：权重 ───────────────────────────

/// 三力顶层权重（**唯一权威源**）。
///
/// ⚠ 改动此处后必须同步 `bottleneck-calc.rhai` / `strategy-scorer.rhai` /
/// `seed_serenity.rs` 的副本，否则 `scripts/check-ontology-consistency.mjs` 会红。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BottleneckWeights {
    /// 供给刚性权重。
    pub supply: f64,
    /// 需求弹性权重。
    pub demand: f64,
    /// 不可替代性权重。
    pub irreplaceability: f64,
}

impl BottleneckWeights {
    /// 顶层权重之和（公理 A2：必须 = 1）。
    pub fn total(&self) -> f64 {
        self.supply + self.demand + self.irreplaceability
    }
}

/// 顶层权重的权威取值。**这是全仓唯一一处可以改这些数字的地方。**
///
/// 解析契约：`scripts/check-ontology-consistency.mjs` 以正则读取本常量，
/// 故书写形式（**单行**、字段顺序 `supply / demand / irreplaceability`）不得随意改动。
pub const BOTTLENECK_WEIGHTS: BottleneckWeights =
    BottleneckWeights { supply: 0.35, demand: 0.35, irreplaceability: 0.30 };

/// 权重的**规范文本渲染**：固定 2 位小数。
///
/// ## 为什么必须固定
///
/// `format!("{}", 0.30)` 得到的是 **`"0.3"`**（`f64` 的 `Display` 不保留尾零）。
/// 而 `seed_serenity.rs` 会把权重写进模板变量的**描述文本**（"…（默认0.30，c-scorer 引用）"）
/// ⇒ 直接用 `{}` 渲染会让**种子产物静默变化**（"默认0.30" → "默认0.3"），
/// 进而要求升 `TEMPLATE_VERSION` 重种子，而**没有任何东西会提示你是渲染造成的**。
///
/// ⇒ 本函数是权重数字进入**文本**的唯一通道；`scripts/check-ontology-consistency.mjs`
/// 在 JS 侧用 `toFixed(2)` 做同一约定，两侧必须一致。
pub fn format_weight(v: f64) -> String {
    format!("{v:.2}")
}

/// 某一力的**内部**权重（`(分项名, 权重)` 列表）。
pub type ForceParts = &'static [(&'static str, f64)];

/// 供给刚性的内部分项权重（集中度 / 技术壁垒 / 扩产周期）。
pub const SUPPLY_RIGIDITY_PARTS: ForceParts =
    &[("concentration", 0.30), ("barrier", 0.40), ("expansion_cycle", 0.30)];

/// 需求弹性的内部分项权重（证据强度 / 需求确定性）。
pub const DEMAND_ELASTICITY_PARTS: ForceParts = &[("evidence", 0.60), ("certainty", 0.40)];

/// 不可替代性的内部分项权重（供应商数 / 技术护城河 / 技术壁垒）。
pub const IRREPLACEABILITY_PARTS: ForceParts =
    &[("supplier", 0.30), ("tech_moat", 0.40), ("barrier", 0.30)];

// ─────────────────────────── 权威数值：分档 ───────────────────────────

/// 一个分档：覆盖 `[min, 上一档的 min)`。
///
/// ⚠ 不派生 `Eq` —— 含 `f64` 字段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// 分档 ID（即为 `.rhai` 里输出的标签）。
    pub id: &'static str,
    /// 本档下界（含）。降序排列。
    pub min: f64,
    /// 中文含义。
    pub label: &'static str,
}

/// 瓶颈综合分的分档阈值（**唯一权威源**）。
///
/// 权威侧取**被执行的那一侧**（`.rhai`），而非 prompt 里的散文口径。
/// 与 `chokepoint-identifier.md:102` 的 `80 / 60` 存在分歧 ⇒ 见 [`OPEN_DIVERGENCES`]。
pub const READINESS_BANDS: &[Band] = &[
    Band { id: "strong_bottleneck", min: 75.0, label: "强瓶颈信号" },
    Band { id: "potential_bottleneck", min: 55.0, label: "潜在瓶颈信号" },
    Band { id: "weak_signal", min: 35.0, label: "弱信号" },
    Band { id: "no_signal", min: 0.0, label: "无信号" },
];

// ─────────────────────────── 已登记的口径分歧 ───────────────────────────

/// 已登记的口径分歧（**只登记，不擅自统一**）。
///
/// `scripts/check-ontology-consistency.mjs` 会把本表打印出来，但不据此判失败：
/// 消除分歧需要产品决策（哪一侧才是业务口径），不属于机械修复。
///
/// ⚠ 唯一的例外是「**存在性**」：本表每条的 [`Divergence::site`] 指向的文件必须真实存在
/// （由门禁的 `checkPointerFilesExist` 校验）—— 现场文件被改名/删除 ⇒ 登记项变成僵尸，
/// 这在门禁看来必须红。**校验的是存在性，不是口径**。
pub const OPEN_DIVERGENCES: &[Divergence] = &[
    Divergence {
        id: "D1",
        concept: "bottleneck_composite 的定义式",
        authority: "加权：Σ 顶层权重 × 对应力分（BOTTLENECK_WEIGHTS）",
        site: "src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:101",
        claim: "等权：(supply_rigidity + demand_elasticity + irreplaceability) / 3",
        status: "待裁决 —— 若按等权，BOTTLENECK_WEIGHTS 应全为 1/3；若按加权，该行须改",
    },
    Divergence {
        id: "D2",
        concept: "瓶颈分档阈值",
        authority: "75 / 55 / 35（READINESS_BANDS）",
        site: "src-tauri/agency_experts/stock-analysis/chokepoint-identifier.md:102",
        claim: "80 / 60",
        status: "待裁决 —— 已登记未统一；统一前须先确认两侧是否对同一个量分档",
    },
    Divergence {
        id: "D3",
        concept: "分档档名（_signal vs _bottleneck）",
        authority: "本体档名 strong_bottleneck / potential_bottleneck（band_for_score）",
        site: "src-tauri/src/commands/strategy-scorer.rhai:64",
        claim: "强 / 潜在信号档沿用 _signal 后缀（strong_signal / potential_signal），映射自本体同阈值",
        status: "已统一阈值、保留档名 —— 按 §7-a b1 收敛：阈值来自 band_for_score，仅档名回映射到 _signal；weak_signal / no_signal 两侧档名本就一致",
    },
];

// ─────────────────────────── 域 → 本体 注册表（P1b §4.3.1）───────────────────────────

/// 一个能力域所关联的本体内容（类 / 关系 / 度量 / 分档 / 顶层权重）。
///
/// 「域 × 本体表」的静态绑定：**新增能力域本体 = 在 [`ONTOLOGY_DOMAINS`] 加一行**，
/// 查询工具（`ontology_query`）按 `domain` 路由即可自动可见，零消费代码改动。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OntologyDomain {
    /// 权威 key（[`CapabilityDomain::as_str`] 即协议 id）。
    pub domain: CapabilityDomain,
    /// 该域的类层级表。
    pub classes: &'static [OntoClass],
    /// 该域的对象属性表。
    pub object_properties: &'static [ObjectProperty],
    /// 该域的度量表。
    pub metrics: &'static [Metric],
    /// 该域的分档表（降序）。
    pub bands: &'static [Band],
    /// 该域的顶层权重（无则不参与权重评分）。
    pub weights: Option<BottleneckWeights>,
}

/// 已登记的能力域本体表。finance 现成的 6 张表原样塞入第一条（**不重排、不改字段**，
/// 种子产物逐字节不变，不升 `TEMPLATE_VERSION`）；其它域缺省为空表，由后续按需补充。
pub const ONTOLOGY_DOMAINS: &[OntologyDomain] = &[OntologyDomain {
    domain: CapabilityDomain::Finance,
    classes: CLASSES,
    object_properties: OBJECT_PROPERTIES,
    metrics: METRICS,
    bands: READINESS_BANDS,
    weights: Some(BOTTLENECK_WEIGHTS),
}];

/// 按域解析本体；未登记域返回 `None`（**不 panic** —— 消费侧据此返回 `DOMAIN_UNREGISTERED`）。
pub fn ontology_of(domain: CapabilityDomain) -> Option<&'static OntologyDomain> {
    ONTOLOGY_DOMAINS.iter().find(|d| d.domain == domain)
}

/// 域内按类 ID 定位类。
pub fn class_by_id_in(domain: CapabilityDomain, id: &str) -> Option<&'static OntoClass> {
    ontology_of(domain).and_then(|d| d.classes.iter().find(|c| c.id == id))
}

// ─────────────────────────── 查询接口（finance 全局便捷壳）───────────────────────────
//
// 下列 finance 全局函数保持签名不变（保护 `seed_serenity.rs:1268` 与门禁 `SITES`），
// 内部改为经注册表（`ontology_of(Finance)`）查找 —— 消除双份数据，消费方无感。

/// 按类 ID 查找（finance 域）。
pub fn class_by_id(id: &str) -> Option<&'static OntoClass> {
    class_by_id_in(CapabilityDomain::Finance, id)
}

/// 某类的直接子类（finance 域）。
pub fn subclasses_of(parent_id: &str) -> Vec<&'static OntoClass> {
    match ontology_of(CapabilityDomain::Finance) {
        Some(d) => d.classes.iter().filter(|c| c.parent == Some(parent_id)).collect(),
        None => Vec::new(),
    }
}

/// 沿 `parent` 逐级上溯（含起点自身）。层级有环时最多走 `CLASSES.len() + 1` 步后停止。
pub fn superclass_chain(id: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut cur = class_by_id(id).map(|c| c.id);
    let mut steps = 0usize;
    while let Some(cid) = cur {
        if steps > CLASSES.len() {
            break;
        }
        out.push(cid);
        cur = class_by_id(cid).and_then(|c| c.parent);
        steps += 1;
    }
    out
}

/// `sub` 是否为 `ancestor` 的后代（**严格**，不含自身）。
pub fn is_subclass_of(sub: &str, ancestor: &str) -> bool {
    superclass_chain(sub).iter().skip(1).any(|c| *c == ancestor)
}

/// 按综合分给出分档 ID；越界输入钳制到 `[0, 100]`（与 `.rhai` 的 `clamp` 一致）。
pub fn band_for_score(score: f64) -> &'static str {
    // 权威：finance 域的本体分档表（经注册表索引，P1b）—— 与 `class_by_id` / `subclasses_of` 同一路由。
    let Some(domain) = ontology_of(CapabilityDomain::Finance) else {
        // 登记表缺 finance 不该发生（固有一条）；兜底以免未来改表后 panic。
        return "no_signal";
    };
    let s = if score.is_nan() {
        0.0
    } else {
        score.clamp(0.0, 100.0)
    };
    for b in domain.bands {
        if s >= b.min {
            return b.id;
        }
    }
    // 末档 min = 0.0（由 A5 强制）⇒ 此分支不可达；
    // 保留兜底以免未来改表后 panic（不可达 ≠ 可删：删了就是未来改表的隐雷）。
    "no_signal"
}

// ─────────────────────────── 纯校验函数（供测试做正负对照）───────────────────────────

/// 权重和是否 = 1（容差 `1e-9`）。抽出来是为了能用**畸形输入**做负向对照。
pub fn weights_sum_to_one(w: &BottleneckWeights) -> bool {
    (w.total() - 1.0).abs() < 1e-9
}

/// 分项权重和是否 = 1。空表判失败（否则「把分项删空」会静默通过）。
pub fn parts_sum_to_one(parts: ForceParts) -> bool {
    if parts.is_empty() {
        return false;
    }
    let sum: f64 = parts.iter().map(|(_, w)| *w).sum();
    (sum - 1.0).abs() < 1e-9
}

/// 分档表是否「严格降序 + 末档下界 = 0」⇒ 在 `[0, metric_max]` 上无缝无洞。
pub fn bands_are_total_and_gapfree(bands: &[Band], metric_max: f64) -> bool {
    if bands.is_empty() {
        return false;
    }
    if bands[0].min > metric_max {
        return false;
    }
    for w in bands.windows(2) {
        if w[0].min <= w[1].min {
            return false;
        }
    }
    // 末档下界必须落到 0：否则 [0, min) 是空洞（分数落进去没有任何档位接住）
    bands[bands.len() - 1].min.abs() < 1e-12
}

/// 类层级是否无环，且无悬空 `parent`。
pub fn hierarchy_is_acyclic(classes: &[OntoClass]) -> bool {
    for c in classes {
        let mut cur = c.parent;
        let mut steps = 0usize;
        while let Some(pid) = cur {
            steps += 1;
            if steps > classes.len() {
                return false;
            }
            cur = match classes.iter().find(|x| x.id == pid) {
                Some(p) => p.parent,
                // 悬空 parent 也算层级不成立（另有测试单独报更清晰的错误）
                None => return false,
            };
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // ── 结构一致性 ──

    /// 类 ID 全局唯一
    #[test]
    fn test_class_ids_unique() {
        let mut seen = HashSet::new();
        for c in CLASSES {
            assert!(seen.insert(c.id), "类 ID 重复：{}", c.id);
        }
    }

    /// 每条声明都必须有可核对的源码锚点（`路径:行` 或 `路径:行-行`）
    #[test]
    fn test_every_decl_cites_evidence() {
        let anchor_ok = |a: &str| {
            let (_, tail) = a.rsplit_once(':').unwrap_or(("", ""));
            !tail.is_empty() && tail.split(['-', ',']).all(|n| n.trim().parse::<u32>().is_ok())
        };
        let check = |id: &str, ev: &str| {
            assert!(!ev.is_empty(), "{} 缺少依据锚点", id);
            for anchor in ev.split('|') {
                let a = anchor.trim();
                assert!(anchor_ok(a), "{} 的依据锚点格式不对：{}", id, a);
            }
        };
        for c in CLASSES {
            check(c.id, c.evidence);
        }
        for p in OBJECT_PROPERTIES {
            check(p.id, p.evidence);
        }
        for m in METRICS {
            check(m.id, m.evidence);
        }
    }

    /// 层级无环 —— 正例 + **负向对照**（自环、互指）
    #[test]
    fn test_class_hierarchy_acyclic_with_negative_control() {
        assert!(hierarchy_is_acyclic(CLASSES), "真实类层级存在环");

        let bad_self = [OntoClass { id: "A", parent: Some("A"), ..CLASSES[0] }];
        assert!(!hierarchy_is_acyclic(&bad_self), "自环未被检出 ⇒ 校验函数无效");

        let bad_pair = [
            OntoClass { id: "A", parent: Some("B"), ..CLASSES[0] },
            OntoClass { id: "B", parent: Some("A"), ..CLASSES[0] },
        ];
        assert!(!hierarchy_is_acyclic(&bad_pair), "互指成环未被检出 ⇒ 校验函数无效");
    }

    /// 每个 `parent` 必须指向已声明的类
    #[test]
    fn test_parent_references_declared_class() {
        for c in CLASSES {
            if let Some(p) = c.parent {
                assert!(class_by_id(p).is_some(), "{} 的父类 `{}` 未声明", c.id, p);
                assert_ne!(p, c.id, "{} 的父类是自己", c.id);
            }
        }
    }

    /// 对象属性的 `domain` / `range` 必须指向已声明的类
    #[test]
    fn test_object_property_endpoints_declared() {
        for p in OBJECT_PROPERTIES {
            assert!(class_by_id(p.domain).is_some(), "{} 的 domain `{}` 未声明", p.id, p.domain);
            assert!(class_by_id(p.range).is_some(), "{} 的 range `{}` 未声明", p.id, p.range);
        }
        let mut seen = HashSet::new();
        for p in OBJECT_PROPERTIES {
            assert!(seen.insert(p.id), "对象属性 ID 重复：{}", p.id);
        }
    }

    /// 度量的归属类已声明，且值域合法（公理 A1）
    #[test]
    fn test_metric_ranges_are_valid() {
        for m in METRICS {
            assert!(class_by_id(m.owner).is_some(), "{} 的归属类 `{}` 未声明", m.id, m.owner);
            assert!(m.min <= m.max, "{} 值域倒置：{} > {}", m.id, m.min, m.max);
        }
        // 三力度量的值域必须一致 —— 它们被同一个 composite 公式线性组合
        let forces: Vec<&Metric> = METRICS
            .iter()
            .filter(|m| {
                class_by_id(m.owner).is_some_and(|c| is_subclass_of(c.id, "BottleneckForce"))
            })
            .collect();
        assert_eq!(forces.len(), 3, "瓶颈力度量应恰好 3 条，实际 {}", forces.len());
        let (lo, hi) = (forces[0].min, forces[0].max);
        for f in &forces {
            assert_eq!((f.min, f.max), (lo, hi), "{} 与同类量纲不一致（三力被同一公式组合）", f.id);
        }
    }

    // ── 公理 ──

    /// A2：顶层权重和 = 1 —— 正例 + **负向对照**
    #[test]
    fn test_top_level_weights_sum_to_one() {
        assert!(
            weights_sum_to_one(&BOTTLENECK_WEIGHTS),
            "顶层权重和 ≠ 1：{}",
            BOTTLENECK_WEIGHTS.total()
        );

        let drifted = BottleneckWeights { supply: 0.40, demand: 0.40, irreplaceability: 0.40 };
        assert!(!weights_sum_to_one(&drifted), "和为 1.2 仍判通过 ⇒ 校验函数恒真，A2 无人守");
    }

    /// `format_weight` 必须保留尾零 —— 守「种子描述文本不因渲染而静默变化」
    #[test]
    fn test_format_weight_keeps_trailing_zero() {
        // 这两条是本测试存在的全部理由：Display 会给 "0.3"，而描述文本里写的是 "0.30"
        assert_eq!(format_weight(0.30), "0.30", "尾零丢失 ⇒ 种子产物会静默变化并要求升版");
        assert_eq!(format_weight(0.35), "0.35");
        assert_eq!(format_weight(1.0 / 3.0), "0.33");
        // 负向对照：证明「用 Display」确实会丢尾零（否则本测试可能是恒真的）
        assert_ne!(
            format!("{}", 0.30_f64),
            "0.30",
            "若 Display 已保留尾零，本函数的必要性需重新评估"
        );
    }

    /// A3：每力内部分项权重和 = 1 —— 正例 + **负向对照**
    #[test]
    fn test_force_part_weights_sum_to_one() {
        for (name, parts) in [
            ("SupplyRigidity", SUPPLY_RIGIDITY_PARTS),
            ("DemandElasticity", DEMAND_ELASTICITY_PARTS),
            ("Irreplaceability", IRREPLACEABILITY_PARTS),
        ] {
            assert!(parts_sum_to_one(parts), "{} 分项权重和 ≠ 1", name);
        }
        assert!(!parts_sum_to_one(&[("a", 0.5), ("b", 0.4)]), "和为 0.9 被判成 1 ⇒ 校验函数无效");
        assert!(!parts_sum_to_one(&[]), "空分项表应判失败（否则删空即静默通过）");
    }

    /// A4：composite 是**加权**而非等权平均 —— 本公理存在的意义就是钉死 D1 的权威侧
    #[test]
    fn test_composite_is_weighted_not_mean() {
        let w = BOTTLENECK_WEIGHTS;
        // 顶层权重不得退化成等权 1/3，否则与 chokepoint-identifier.md:101 的 /3 口径不再可区分
        let all_equal_third = (w.supply - 1.0 / 3.0).abs() < 1e-9
            && (w.demand - 1.0 / 3.0).abs() < 1e-9
            && (w.irreplaceability - 1.0 / 3.0).abs() < 1e-9;
        assert!(!all_equal_third, "顶层权重退化成等权 1/3 ⇒ A4 失去意义，须改 D1 登记");

        // 三档权重不得全部相同
        let all_same =
            (w.supply - w.demand).abs() < 1e-12 && (w.demand - w.irreplaceability).abs() < 1e-12;
        assert!(
            !all_same,
            "顶层权重全部相等 ⇒ composite 实际等价于等权平均，与 A4 宣称的「加权」不符"
        );

        // 复算：同一组力分上，加权与等权必须给出**不同**结果（否则用例无判别力）
        let (f1, f2, f3) = (80.0_f64, 20.0, 100.0);
        let weighted = f1 * w.supply + f2 * w.demand + f3 * w.irreplaceability;
        let mean = (f1 + f2 + f3) / 3.0;
        assert!(
            (weighted - mean).abs() > 1.0,
            "该组力分下加权({weighted})与等权({mean})几乎相同 ⇒ 用例失去判别力"
        );
    }

    /// A5：分档无缝无洞 —— 正例 + **负向对照**（底部空洞、乱序、空表）
    #[test]
    fn test_readiness_bands_are_total_and_gapfree() {
        let m = METRICS
            .iter()
            .find(|x| x.id == "bottleneck_composite")
            .expect("bottleneck_composite 未声明");
        assert!(
            bands_are_total_and_gapfree(READINESS_BANDS, m.max),
            "分档表不满足「降序 + 末档下界 = 0」"
        );

        let gap_at_bottom = [
            Band { id: "strong", min: 75.0, label: "" },
            Band { id: "none", min: 10.0, label: "" },
        ];
        assert!(
            !bands_are_total_and_gapfree(&gap_at_bottom, 100.0),
            "末档下界 10 留出 [0,10) 空洞却判通过 ⇒ 校验函数无效"
        );

        let misordered =
            [Band { id: "a", min: 35.0, label: "" }, Band { id: "b", min: 75.0, label: "" }];
        assert!(!bands_are_total_and_gapfree(&misordered, 100.0), "乱序分档未被检出");
        assert!(!bands_are_total_and_gapfree(&[], 100.0), "空分档表应判失败");
    }

    /// A6：Chokepoint ⊂ ChainStage（并验传递性、反方向、严格性）
    #[test]
    fn test_chokepoint_is_subclass_of_chain_stage() {
        assert!(is_subclass_of("Chokepoint", "ChainStage"), "Chokepoint 不是 ChainStage 的子类");
        assert!(
            is_subclass_of("Chokepoint", "IndustryElement"),
            "传递性不成立：Chokepoint 应经由 ChainStage/IndustryChain 继承 IndustryElement"
        );
        for f in ["SupplyRigidity", "DemandElasticity", "Irreplaceability"] {
            assert!(is_subclass_of(f, "BottleneckForce"), "{} 不是 BottleneckForce 的子类", f);
        }
        // 反方向不得成立（防 parent 写反）
        assert!(!is_subclass_of("ChainStage", "Chokepoint"), "父子关系被写反");
    }

    /// A7：恰好 3 条力；每条力恰有 1 个度量；顶层权重不得为 0
    #[test]
    fn test_exactly_three_forces_match_weights() {
        let forces = subclasses_of("BottleneckForce");
        assert_eq!(forces.len(), 3, "瓶颈力应恰好 3 条，实际 {}", forces.len());
        let ids: HashSet<&str> = forces.iter().map(|c| c.id).collect();
        for expected in ["SupplyRigidity", "DemandElasticity", "Irreplaceability"] {
            assert!(ids.contains(expected), "缺少力：{expected}");
        }
        for f in &forces {
            let n = METRICS.iter().filter(|m| m.owner == f.id).count();
            assert_eq!(n, 1, "{} 的度量数应为 1，实际 {}", f.id, n);
        }
        // 权重为 0 ⇒ 该力被算出来却永不参与 composite（静默的无效计算）
        let w = BOTTLENECK_WEIGHTS;
        for (name, v) in
            [("supply", w.supply), ("demand", w.demand), ("irreplaceability", w.irreplaceability)]
        {
            assert!(v > 0.0, "顶层权重 {name} = {v} ⇒ 对应之力被计算却永不生效（静默失效）");
        }
    }

    // ── 查询接口 ──

    /// 查询接口的负向对照（防过滤器写反导致「查谁都命中」）
    #[test]
    fn test_lookup_negative_control() {
        assert!(class_by_id("Chokepoint").is_some());
        assert!(class_by_id("不存在的类").is_none());
        assert!(!CLASSES.is_empty(), "类表为空 ⇒ 所有查询都会静默返回空");
        assert!(subclasses_of("不存在的类").is_empty());
        assert!(!is_subclass_of("Chokepoint", "不存在的类"));
        assert!(!is_subclass_of("Chokepoint", "Chokepoint"), "is_subclass_of 应为严格关系");
    }

    /// 分档函数与阈值表一致，边界取「含下界」
    #[test]
    fn test_band_for_score_boundaries() {
        assert_eq!(band_for_score(100.0), "strong_bottleneck");
        assert_eq!(band_for_score(75.0), "strong_bottleneck", "边界应含下界");
        assert_eq!(band_for_score(74.99), "potential_bottleneck");
        assert_eq!(band_for_score(55.0), "potential_bottleneck");
        assert_eq!(band_for_score(35.0), "weak_signal");
        assert_eq!(band_for_score(34.99), "no_signal");
        assert_eq!(band_for_score(0.0), "no_signal");
        // 越界与 NaN 的钳制（须与 .rhai 的 clamp 语义一致）
        assert_eq!(band_for_score(-5.0), "no_signal");
        assert_eq!(band_for_score(1e9), "strong_bottleneck");
        assert_eq!(band_for_score(f64::NAN), "no_signal");
    }

    /// 登记的分歧**不得为空** —— 空表意味着「分歧被悄悄解决了」或「漏登记」
    #[test]
    fn test_open_divergences_are_declared_not_empty() {
        assert!(!OPEN_DIVERGENCES.is_empty(), "口径分歧表为空：确认是已统一，还是漏登记");
        let mut seen = HashSet::new();
        for d in OPEN_DIVERGENCES {
            assert!(seen.insert(d.id), "分歧 ID 重复：{}", d.id);
            assert!(!d.authority.is_empty() && !d.claim.is_empty(), "{} 缺少任一侧口径", d.id);
            let (_, tail) = d.site.rsplit_once(':').expect("分歧现场必须带行号");
            assert!(tail.parse::<u32>().is_ok(), "{} 的现场行号不可解析：{}", d.id, tail);
        }
    }

    /// 每条公理都必须声明「谁在守它」
    #[test]
    fn test_axioms_declare_enforcer() {
        assert!(!AXIOMS.is_empty());
        let mut seen = HashSet::new();
        for a in AXIOMS {
            assert!(seen.insert(a.id), "公理 ID 重复：{}", a.id);
            assert!(!a.enforced_by.is_empty(), "{} 未声明守护测试", a.id);
        }
    }
}
