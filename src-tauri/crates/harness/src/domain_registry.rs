// SPDX-License-Identifier: AGPL-3.0-only

//! L1 能力域元数据声明 —— 三层路由树第一层的「域节点」
//!
//! # 为什么需要它
//!
//! 「能力域」此前**只有枚举**（`crate::capability::CapabilityDomain`：9 个 id），
//! 而域的**元数据**（导航路径 / 导航顺序 / 历史别名）没有任何声明位置，只能散落成手抄副本：
//!
//! | 元数据 | 此前唯一载体 | 漏同步的后果 |
//! |---|---|---|
//! | 导航路径（`/finance`） | `src/lib/domainMeta.ts` 的 `path` | 域入口页 404 |
//! | 导航顺序 | 同文件的**数组下标**（2026-09-15 前是 `order` 字段） | 侧栏分组乱序 |
//! | 历史别名（18 条） | `capability.rs` 的 `FromStr` **扁平 match** | 存量标注静默解析失败 |
//!
//! 本模块给这些元数据一个**后端声明位置**：
//! 导航元数据由门禁 `check-domain-single-source.mjs` **逐值比对**前端副本，
//! 历史别名则由 [`CapabilityDomain::from_str`] **直接消费**（见下）。
//!
//! # 单一真相源的边界（⚠ 勿在本模块写 id 字符串）
//!
//! `DomainNode` 存的是 **`CapabilityDomain` 枚举变体**，不是字符串 id：
//!
//! - id 一律由 [`CapabilityDomain::as_str`] 派生（见 [`DomainNode::slug`]）
//!   ⇒ **本模块无法与权威源漂移**，因为它根本不复制那份数据。
//! - 因此本模块**不是**「第 N 套词汇表」：它没有自己的词汇，只有对既有词汇的**赋值**。
//!
//! 这条约束是本模块存在的**前提**，不是风格偏好 —— 2026-09-14 `page_type.rs` 曾因
//! 另立一套域字符串而造出并列词汇表（见 `PLAN-domain-single-source.md` §9）。
//!
//! # 与相邻模块的分工
//!
//! | 模块 | 层 | 内容 |
//! |---|---|---|
//! | [`crate::capability`] | L1 id | `CapabilityDomain` 枚举 + `as_str()`（唯一权威分类轴） |
//! | **本模块** | **L1 元数据** | `DOMAIN_NODES`：路径 / 顺序 / 历史别名 + 运行时覆盖层（启用状态 / 追加别名） |
//! | [`crate::capability_clusters`] | L2 | `CapabilityCluster`：27 簇 |
//! | [`crate::domain_ontology`] | 单域业务本体 | finance 域的类 / 关系 / 公理（**不含域层**） |
//!
//! # 运行时覆盖层（P2）：本模块同时是「内置默认」与「合并视图」
//!
//! `DOMAIN_NODES` 是**编译期内置默认**，不是最终值。P2 起多了一层**覆盖层**
//! （持久化在 `capability_domain_overrides` 表，见本模块 §运行时覆盖层）：
//!
//! ```text
//!   DOMAIN_NODES（编译期，本文件）
//!         ∪
//!   overrides（运行时，DB → apply_domain_overrides）
//!         ↓
//!   合并视图：is_domain_enabled / enabled_domains / effective_aliases
//!             / resolve_enabled_domain / l1_classifier_domain_list
//! ```
//!
//! 覆盖层只能改三件事的其中两件：**启用状态**与**追加别名**。
//! 路径 / 顺序 / 内置别名仍是编译期契约，**覆盖层碰不到**（这是刻意的：
//! 那三项被 DB 存量字符串与前端导航依赖，改了会 404 或乱序）。
//!
//! ⚠ 因此：**读域元数据一律走「合并视图」函数，不要直接读 `DOMAIN_NODES` 就下结论**。
//! 需要区分「内置默认」与「用户改过」时用 [`has_override`]。
//!
//! # 消费方（判据 #183：零读取端的字段不写）
//!
//! | 字段 | 消费方 |
//! |---|---|
//! | `domain` | 门禁（覆盖完整性比对）+ [`capability_clusters`] 单测 + [`node_of`] |
//! | `nav_path` / `nav_order` | 门禁（逐值比对 `domainMeta.ts` 的 `path` 与**数组下标**） |
//! | `aliases` | **[`CapabilityDomain::from_str`]**（生产路径：反序列化存量字符串 + 解析 LLM 输出） |
//! | [`DomainNode::label_key`]（方法） | 门禁（**跨语言公式比对** 前端 `domainLabelKey()`）+ 本模块单测 |
//! | 覆盖层 `enabled` | **[`enabled_domains`] → [`l1_classifier_domain_list`]（L1 分类器 prompt）** + **[`is_domain_enabled`]（能力过滤闸门）** + [`resolve_enabled_domain`] |
//! | 覆盖层 `extra_aliases` | **[`resolve_enabled_domain`]**（用户输入 / LLM 输出的解析入口）+ [`effective_aliases`]（UI 展示） |
//!
//! ⚠ **诚实边界**：导航两字段**没有 Rust 运行时代码消费者** —— 导航是前端行为。
//! 它们的价值在于「让前端副本有一个被 CI 强制比对的上游」，与本仓
//! `domain_ontology::READINESS_BANDS`（同样 0 生产消费、由门禁比对）是同一形态。
//! `aliases` 则相反：它是**运行时代码直接读的**（`from_str` 每次解析都要遍历）。
//! 覆盖层两个字段都是**真消费端**（见上表加粗项），不是「登记备查」。
//!
//! # 已知边界（自陈，勿高估本模块）
//!
//! 1. **新增枚举变体不会让本文件编译失败**（`as_str` 的 match 穷尽性只管枚举自身）。
//!    覆盖完整性由**两道**兜底：[`_variant_tripwire`] 的编译期穷尽 match（保证「改域」这件事
//!    必定把人引到本文件）+ 门禁的覆盖比对（保证漏声明即 CI 红）。
//! 2. 只声明**域层**元数据。域的业务语义（类 / 关系 / 公理）属 [`crate::domain_ontology`]，
//!    且目前只有 finance 一个域有内容。
//! 3. **覆盖层是进程内的**：`apply_domain_overrides` 只改内存。持久化由 DAO 负责
//!    （启动时读全表 → apply；写命令写库后 apply）。本模块**不碰 DB**（harness 零依赖）。
//! 4. **停用域不会让存量数据失效**：`CapabilityDomain::from_str` 与覆盖层无关，
//!    存量 `active_domains` / `route_path` 照常解析。只有「用户输入 / LLM 输出」入口
//!    （[`resolve_enabled_domain`]）会拒绝停用域。这两个语义分开是刻意的，见该函数文档。
//!
//! # 解析契约（改排版前必读）
//!
//! `scripts/check-domain-single-source.mjs` 以**正则**读取本文件，故每个条目必须是
//! `DomainNode { … }` **结构体字面量**，且**字段名逐字**为
//! `domain` / `nav_path` / `nav_order` / `aliases`，取值形态为
//! `CapabilityDomain::<变体>` / `Some("…")` / `None` / `Some(<数字>)` / `&[…]`。
//!
//! ⚠ **不要求单行或多行** —— `cargo fmt` 会按列宽自行折叠（实测同一数组里两种形态并存），
//! 抽取器对空白不敏感。但**不要**把条目改成函数调用、`const` 拼接或宏展开：
//! 那会让抽取器返回 `null`，门禁按「解析失败」**硬拦**（不按「0 违规」通过）。

use crate::capability::CapabilityDomain;

// ── 域节点声明 ────────────────────────────────────

/// L1 能力域节点 —— 域 id 之外的元数据宿主。
///
/// 字段全部 `&'static`，与 [`crate::capability_clusters::CapabilityCluster`] 同形态
/// （零堆分配，符合 harness foundation 层「纯声明 + 零运行时开销」约束）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainNode {
    /// 域（**唯一标识来源**：`domain.as_str()` 即协议 id；本模块刻意不写 id 字符串）
    pub domain: CapabilityDomain,
    /// 导航路径（`Some` = 侧栏 / 域聚合页入口；`None` = 不进入导航，即内部域 `system`）
    ///
    /// 不变量（PLAN-domain-single-source.md §7.3）：内部域**永不进入导航与检索**。
    pub nav_path: Option<&'static str>,
    /// 导航分组顺序（越小越靠前）—— **不等于**协议（枚举）顺序，两者用途不同：
    /// 导航顺序是产品决策（finance 提前），协议顺序是枚举声明顺序（L1 路由 / prompt 清单用）。
    ///
    /// `None` 与 `nav_path` 同步（不进入导航者无顺序）。
    pub nav_order: Option<u8>,
    /// **历史别名**（不含规范 id 本身）—— 兼容存量标注与 LLM 输出的近义写法。
    ///
    /// 唯一消费方是 [`CapabilityDomain::from_str`]；**不可删**：删了不会编译报错，
    /// 但 `active_domains` / `route_path` 等 DB 存量字符串会静默解析失败
    /// （PLAN-domain-single-source.md §7.2）。
    ///
    /// ⚠ 别名的**归属**在这里显式声明（此前在 `FromStr` 的扁平 match 里只能靠注释表达），
    /// 且由单测 [`tests::test_legacy_aliases_resolve`] 逐条钉住。
    pub aliases: &'static [&'static str],
}

/// 全部 L1 域节点（**按 `CapabilityDomain` 枚举声明顺序**排列）。
///
/// **排列约定**：`general → devops → ai_media → data_analysis → content_creation →
/// communication → finance → automation → system`，与 `CapabilityDomain::as_str()` 的输出顺序
/// 逐项一致。此约定被门禁读取（作为「协议顺序」的权威值），故**新增域必须插在对应位置**，
/// 不能追加到末尾。
///
/// `nav_order` 与本数组下标**无关** —— 它是导航顺序（`general → finance → automation → …`），
/// 与协议顺序是两个独立的序。
pub const DOMAIN_NODES: &[DomainNode] = &[
    DomainNode {
        domain: CapabilityDomain::General,
        nav_path: Some("/general"),
        nav_order: Some(0),
        // 历史「核心域」与一批命令域值：统一前的存量标注，全部收敛到通用域
        aliases: &[
            "core",
            "device",
            "dynamic_ui",
            "fine_tune",
            "cloud",
            "rl_training",
            "context",
            "conversation",
        ],
    },
    DomainNode {
        domain: CapabilityDomain::Devops,
        nav_path: Some("/devops"),
        nav_order: Some(3),
        aliases: &["pty", "db_config"],
    },
    DomainNode {
        domain: CapabilityDomain::AiMedia,
        nav_path: Some("/ai-media"),
        nav_order: Some(6),
        aliases: &[],
    },
    DomainNode {
        domain: CapabilityDomain::DataAnalysis,
        nav_path: Some("/data-analysis"),
        nav_order: Some(4),
        aliases: &[],
    },
    DomainNode {
        domain: CapabilityDomain::ContentCreation,
        nav_path: Some("/content-creation"),
        nav_order: Some(5),
        aliases: &[],
    },
    DomainNode {
        domain: CapabilityDomain::Communication,
        nav_path: Some("/communication"),
        nav_order: Some(7),
        aliases: &[],
    },
    DomainNode {
        domain: CapabilityDomain::Finance,
        nav_path: Some("/finance"),
        nav_order: Some(1),
        aliases: &["invest", "quant", "portfolio", "stock_analysis"],
    },
    DomainNode {
        domain: CapabilityDomain::Automation,
        nav_path: Some("/automation"),
        nav_order: Some(2),
        aliases: &["opc", "workflow"],
    },
    DomainNode {
        domain: CapabilityDomain::System,
        nav_path: None,
        nav_order: None,
        aliases: &["orchestrator", "agent"],
    },
];

// ── 派生访问 ──────────────────────────────────────

impl DomainNode {
    /// 协议 id —— **派生**自枚举，不是存下来的字符串。
    pub fn slug(&self) -> &'static str {
        self.domain.as_str()
    }

    /// 域的 i18n 显示名 key —— **派生**（`capabilityDomain.<slug>`）。
    ///
    /// # 为什么后端要持有这个公式（而不是让前端随便算）
    ///
    /// 它是一条**跨语言字符串契约**：前端 `src/lib/domainMeta.ts::domainLabelKey()`
    /// 必须算出同一个 key。两侧公式不一致的后果是**静默的** ——
    /// 界面把域名显示成裸 key（i18n 查不到该 key），而 `tsc` / `cargo` **都不报错**
    /// （两侧各自都是合法代码，只是算出来的字符串不同）。
    ///
    /// ⇒ 唯一防线是门禁 `scripts/check-domain-single-source.mjs` 的**公式比对**：
    /// 它分别抽取本函数与 `domainLabelKey()` 的命名空间字面量并断言相等。
    ///
    /// # 形态约束（解析契约）
    ///
    /// 门禁以正则读本函数，故**必须**保持 `format!("<命名空间>.{}", self.slug())` 这一形态。
    /// 改成 `push_str` 拼接、`concat!`、或把命名空间挪到常量里，抽取器会返回 `null`
    /// ⇒ 门禁按「解析失败」**硬拦**（不按「无违规」通过）。
    ///
    /// ⚠ 返回 `String`（拼接必然分配）—— 但它**不在任何热路径上**，
    /// 且当前**无 Rust 运行时消费方**（消费方是门禁 + 本模块单测），
    /// 故不违反本层「纯声明 + 零运行时开销」的约束。
    pub fn label_key(&self) -> String {
        format!("capabilityDomain.{}", self.slug())
    }
}

/// 按域查节点。
///
/// # Panics
/// 域未在 [`DOMAIN_NODES`] 声明时 panic —— 这是**刻意的**：漏声明是编译期挡不住的
/// 结构性缺陷（见模块头「已知边界」），静默返回 `None` 会让调用点各自兜底、把缺陷藏起来。
pub fn node_of(domain: CapabilityDomain) -> &'static DomainNode {
    DOMAIN_NODES
        .iter()
        .find(|n| n.domain == domain)
        .unwrap_or_else(|| panic!("域 {domain} 未在 DOMAIN_NODES 声明（domain_registry.rs）"))
}

/// 全部业务域（**不含**内部域 `system`），按协议（枚举）顺序。
///
/// 供 [`crate::capability_clusters`] 的单测派生使用 —— 此前该处硬编码
/// `const BUSINESS_DOMAINS: [CapabilityDomain; 8]`，新增域时测试会**静默少检一个域**。
pub fn business_domains() -> Vec<CapabilityDomain> {
    DOMAIN_NODES.iter().map(|n| n.domain).filter(|d| !d.is_system()).collect()
}

// ── 运行时覆盖层（P2：用户可改子集） ──────────────
//
// 本段回答一个问题：**域层在上次编译之后能不能变。**
//
// 覆盖前（P0/P1 阶段）的答案是不能：9 个 id、路径、顺序、别名全在编译期。
// 用户能做的只有「在自己的角色/专家里勾选 8 个域的子集」（`active_domains`），
// 而**域这一层自身**没有任何运行时可改的语义 —— 停用一个暂时不用的域做不到。
//
// 覆盖层刻意**不动** [`DOMAIN_NODES`]：内置默认永远是编译期常量，
// 覆盖是它上面的一层。这样做的理由有三条，都不是风格问题：
//
// 1. **可回退**：删表 / 清空覆盖即回到出厂状态，不需要重新编译；
// 2. **可对比**：任何时候都能回答「这个域是内置启用的，还是被人改过」（`has_override`）；
// 3. **门禁可测**：内置声明保持纯静态，门禁对它的正则比对（P0-④）依然是有效断言。
//
// ## 边界（明确不做）
//
// - **不开放 id 空间**：覆盖层不能新增/删除域，只能改「启用」与「追加别名」。
//   域的存在性由 `CapabilityDomain` 枚举决定（`capability.rs` 明令禁止自定义域）。
// - **不改 `DOMAIN_NODES`**：见上。
// - **不复活 `DomainRouter` 的 CRUD**（PLAN §6 已裁决只读化）：那是**路由规则**，
//   与本处的**域元数据**是两个对象，见 `domain_router.rs` 的防回归注释。

/// 单个域的覆盖项 —— 只含**可覆盖**字段（不含路径 / 顺序 / 内置别名，那些是编译期契约）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainOverride {
    /// 被覆盖的域。
    pub domain: CapabilityDomain,
    /// 是否启用。`false` ⇒ L1 分类器 prompt 不列它、L1 路由不返回它、能力过滤裁剪它。
    pub enabled: bool,
    /// **追加**别名（在内置 `DOMAIN_NODES[].aliases` 之上追加，不替换）。
    pub extra_aliases: Vec<String>,
}

impl DomainOverride {
    pub fn new(domain: CapabilityDomain, enabled: bool, extra_aliases: Vec<String>) -> Self {
        Self { domain, enabled, extra_aliases }
    }

    /// 只改启用状态（保留追加别名由调用方给出，此处为空）。
    pub fn enabled(domain: CapabilityDomain) -> Self {
        Self { domain, enabled: true, extra_aliases: Vec::new() }
    }

    /// 只改启用状态，指定值。
    pub fn with_enabled(domain: CapabilityDomain, enabled: bool) -> Self {
        Self { domain, enabled, extra_aliases: Vec::new() }
    }
}

/// 覆盖层存储。
///
/// ⚠ **为什么是 `std::sync::RwLock` 而不是 `tokio::sync::RwLock`**：
/// 本层的读点是**同步**函数（`is_domain_enabled` / L1 prompt 构造 / 过滤闸门），
/// 临界区内只做克隆、**不含 await**。用 tokio 版会强迫全部读点变 `async`，
/// 而 prompt 构造发生在 async 闭包里的同步片段中。AGENTS.md 铁律 8 禁止的是
/// 「`parking_lot::RwLock` **跨 await 持锁**」—— 本处不跨 await，且**不引入 `parking_lot`**。
///
/// 空 vec = 无任何覆盖 = 全部走内置默认（这正是首次启动的状态）。
// SAFETY: 此处 std::sync::RwLock 不跨 await 使用，临界区内仅同步操作。
#[allow(clippy::disallowed_types)]
static OVERRIDES: std::sync::OnceLock<std::sync::RwLock<Vec<DomainOverride>>> =
    std::sync::OnceLock::new();

// SAFETY: 此处 std::sync::RwLock 不跨 await 使用，临界区内仅同步操作。
#[allow(clippy::disallowed_types)]
fn overrides_lock() -> &'static std::sync::RwLock<Vec<DomainOverride>> {
    OVERRIDES.get_or_init(|| std::sync::RwLock::new(Vec::new()))
}

/// 取覆盖层快照（克隆）。
///
/// 中毒（写方 panic）时**不传染读路径**：覆盖层是「配置」而不是「不变量」，
/// 读到一个可能陈旧的快照远好于让整条路由 / 检索链路 panic。
/// 这也让「读侧永不 panic」成为可断言的性质。
fn overrides_snapshot() -> Vec<DomainOverride> {
    match overrides_lock().read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// 应用覆盖层（**整表替换**语义：传入即全部覆盖项，未列出的域回到内置默认）。
///
/// 调用方是 DAO（启动时从 `capability_domain_overrides` 读全表）与写命令（写库后刷新）。
pub fn apply_domain_overrides(overrides: Vec<DomainOverride>) {
    match overrides_lock().write() {
        Ok(mut guard) => *guard = overrides,
        Err(poisoned) => *poisoned.into_inner() = overrides,
    }
}

/// 清空覆盖层（回到全部内置默认）。
pub fn clear_domain_overrides() {
    apply_domain_overrides(Vec::new());
}

/// 该域是否有覆盖行（用于 UI 区分「内置默认」与「用户改过」）。
pub fn has_override(domain: CapabilityDomain) -> bool {
    has_override_with(&overrides_snapshot(), domain)
}

/// [`has_override`] 的**纯函数形态**（给定覆盖集，不读全局）。
pub fn has_override_with(overrides: &[DomainOverride], domain: CapabilityDomain) -> bool {
    overrides.iter().any(|o| o.domain == domain)
}

/// 该域是否**允许**被停用。
///
/// 两个例外，理由都是**结构性**的，不是偏好：
///
/// - **`General`**：唯一兜底域（PLAN §7.4）。`route()` 未命中时的返回值就是它，
///   `unknown()` 也硬编码返回它。允许停用 ⇒ 「未命中」会返回一个被停用的域，
///   与消费端②（路由不得返回停用域）**自相矛盾**，且没有任何合法兜底值可选。
/// - **`System`**：内部域（PLAN §7.3 永不进入导航与检索）。它承载
///   `Visibility::SystemOnly` 元能力的路由；停用会让元能力整条链路不可达，
///   而用户**无从观察**（它不出现在导航里，也不该出现在检索结果里）。
pub fn is_toggleable(domain: CapabilityDomain) -> bool {
    !matches!(domain, CapabilityDomain::General | CapabilityDomain::System)
}

/// 不可停用的**原因码**（`None` = 可停用）。
///
/// 刻意返回**码**而不是中文句子：前端按码查 i18n（11 语言），
/// 与错误码契约同一约定（见 `AGENTS.md` 的 i18n 门禁）。
pub fn toggle_block_reason(domain: CapabilityDomain) -> Option<&'static str> {
    match domain {
        CapabilityDomain::General => Some("general_is_fallback"),
        CapabilityDomain::System => Some("system_is_internal"),
        _ => None,
    }
}

/// 该域当前是否启用。
///
/// **无覆盖行 ⇒ 启用**（内置默认）。且**不可停用的域永远返回 `true`** ——
/// 这与写侧 [`is_toggleable`] 是**同一判据**，用于兜住「坏数据绕过命令层直接进库」
/// （手工改 DB / 旧版本写入）的情形：读侧一旦发现是 `general`/`system`，
/// 无论覆盖层怎么说都按启用处理，而不是把矛盾暴露给下游。
pub fn is_domain_enabled(domain: CapabilityDomain) -> bool {
    enabled_with(&overrides_snapshot(), domain)
}

/// [`is_domain_enabled`] 的**纯函数形态**。
///
/// 存在的理由不是「方便」，而是**单一计算路径**：命令层要把「库里的行」渲染成
/// 合并视图（尚未 apply 进全局，也不该为一次读命令去改动全局状态），
/// 若它自己重写一遍 `if 不可停用 { true } else { row.enabled }`，
/// 就有了第二处「启用语义」的实现 —— 正是本项目一直在消灭的副本形态。
/// 全局版本 [`is_domain_enabled`] 反过来调用本函数。
pub fn enabled_with(overrides: &[DomainOverride], domain: CapabilityDomain) -> bool {
    if !is_toggleable(domain) {
        return true;
    }
    overrides.iter().find(|o| o.domain == domain).map(|o| o.enabled).unwrap_or(true)
}

/// 当前启用的域，按**协议顺序**（= `DOMAIN_NODES` 声明顺序，含内部域）。
pub fn enabled_domains() -> Vec<CapabilityDomain> {
    enabled_domains_with(&overrides_snapshot())
}

/// [`enabled_domains`] 的**纯函数形态**。
pub fn enabled_domains_with(overrides: &[DomainOverride]) -> Vec<CapabilityDomain> {
    DOMAIN_NODES.iter().map(|n| n.domain).filter(|d| enabled_with(overrides, *d)).collect()
}

/// 当前启用的**业务**域，按协议顺序（不含内部域 `system`）。
pub fn enabled_business_domains() -> Vec<CapabilityDomain> {
    enabled_domains().into_iter().filter(|d| !d.is_system()).collect()
}

/// 该域当前的**追加**别名（覆盖层里的；无覆盖 = 空）。
pub fn extra_aliases(domain: CapabilityDomain) -> Vec<String> {
    extra_aliases_with(&overrides_snapshot(), domain)
}

/// [`extra_aliases`] 的**纯函数形态**。
pub fn extra_aliases_with(overrides: &[DomainOverride], domain: CapabilityDomain) -> Vec<String> {
    overrides
        .iter()
        .find(|o| o.domain == domain)
        .map(|o| o.extra_aliases.clone())
        .unwrap_or_default()
}

/// 该域当前的**有效别名** = 内置别名 ∪ 追加别名。
///
/// 顺序为先内置后追加（内置的 27 条存量兼容别名永不因覆盖层而消失）。
pub fn effective_aliases(domain: CapabilityDomain) -> Vec<String> {
    effective_aliases_with(&overrides_snapshot(), domain)
}

/// [`effective_aliases`] 的**纯函数形态**。
pub fn effective_aliases_with(
    overrides: &[DomainOverride],
    domain: CapabilityDomain,
) -> Vec<String> {
    let node = node_of(domain);
    let mut out: Vec<String> = node.aliases.iter().map(|a| (*a).to_string()).collect();
    for a in extra_aliases_with(overrides, domain) {
        if !out.iter().any(|x| x.eq_ignore_ascii_case(&a)) {
            out.push(a);
        }
    }
    out
}

/// 解析域标识 —— **运行时权威入口**：内置 id → 内置别名 → 追加别名，
/// 且**拒绝已停用的域**。
///
/// # 与 `CapabilityDomain::from_str` 的分工
///
/// `from_str` 是**编译期契约**的解析器（规范 id + 27 条存量别名），**不看覆盖层**：
/// 它被大量反序列化路径调用（DB 存量字符串、`serde`），那些路径**不应该**因为
/// 用户停用了一个域而解析失败（存量数据仍需可读）。
///
/// 本函数是**用户输入 / LLM 输出**的入口：这两处的语义是「这次请求要去哪个域」，
/// 停用的域在这里必须消失。把两个语义分开，才不会出现「停用后存量数据读不出来」。
pub fn resolve_enabled_domain(raw: &str) -> Option<CapabilityDomain> {
    let key = raw.trim().trim_matches('"').trim_matches('`').to_lowercase();
    if key.is_empty() {
        return None;
    }
    // 1) 内置：规范 id 与历史别名（由 DOMAIN_NODES 单点持有）
    if let Ok(d) = key.parse::<CapabilityDomain>() {
        return is_domain_enabled(d).then_some(d);
    }
    // 2) 追加别名（只有覆盖层才有；按协议顺序取第一个命中）
    let overrides = overrides_snapshot();
    for node in DOMAIN_NODES {
        let Some(o) = overrides.iter().find(|o| o.domain == node.domain) else {
            continue;
        };
        if o.extra_aliases.iter().any(|a| a.to_lowercase() == key) {
            return is_domain_enabled(node.domain).then_some(node.domain);
        }
    }
    None
}

/// L1 域分类器的**候选域清单**（逗号 + 空格分隔，按协议顺序，只含当前启用的域）。
///
/// # 为什么它必须在这里而不是在 prompt 里手抄
///
/// 这份清单是给 LLM 的**候选集**：**LLM 只认得这里列出的值**，
/// 漏一个 ⇒ 该域永远不会被模型兜底选中，且**不报错**（模型会安静地挑一个别域，
/// 或输出不在清单里的词然后被解析丢掉 ⇒ 落到 `General`）。
///
/// 此前它是 `init/state.rs` 里 `const SYS` 中手抄的 9 个 slug（第 N 份 id 副本），
/// 由门禁正则比对守。P2 起它由覆盖层**实时派生** —— 于是「停用一个域」
/// 这件事从「改代码 + 重编译」变成「写一行覆盖」，并且**停用的域自动从候选集消失**。
///
/// ⚠ 返回 `String`（拼接必然分配）；调用点在 LLM 兜底前的非热路径上。
pub fn l1_classifier_domain_list() -> String {
    enabled_domains().iter().map(|d| d.as_str()).collect::<Vec<_>>().join(", ")
}

// ── 编译期穷尽守卫 ────────────────────────────────

/// 编译期穷尽守卫 —— 让「新增一个域」**无法静默编译通过**。
///
/// `CapabilityDomain` 新增变体时，本 `match` 不再穷尽 ⇒ **编译失败**，报错点落在本文件，
/// 从而把改域者引到唯一需要同步的声明处（`DOMAIN_NODES`）。
///
/// 为什么不能靠 [`DOMAIN_NODES`] 自身兜底：数组长度不参与类型检查，
/// 漏一个变体只是「少一行」，编译与测试都不会红 —— 本仓 2026-09-15 实测确认过这一点。
///
/// 返回 `&'static DomainNode` 而非 `()`，是为了同时执行 [`node_of`] 的存在性检查，
/// 使「枚举有变体但没声明节点」在**任何**调用点都立刻暴露。
#[allow(dead_code)]
fn _variant_tripwire(domain: CapabilityDomain) -> &'static DomainNode {
    match domain {
        CapabilityDomain::General
        | CapabilityDomain::Devops
        | CapabilityDomain::AiMedia
        | CapabilityDomain::DataAnalysis
        | CapabilityDomain::ContentCreation
        | CapabilityDomain::Communication
        | CapabilityDomain::Finance
        | CapabilityDomain::Automation
        | CapabilityDomain::System => node_of(domain),
    }
}

/// 覆盖层用例的共享基建（`#[cfg(test)]`）。
///
/// # 为什么放在**模块级**而不是 `mod tests` 里
///
/// 覆盖层（`static OVERRIDES`）是**进程级全局**状态，而 `cargo test` 默认多线程
/// 并行跑同一二进制内的用例 ⇒ 触碰覆盖层的用例必须**串行**。
/// 而触碰它的**不止本模块**：`domain_router` 的「停用域不再被路由」用例同样要写覆盖层。
/// 若两个模块各持一把锁，就等于没锁 —— 症状是随机红的、与本机改动无关的失败
/// （一个用例的 `apply_domain_overrides` 污染另一个用例的路由断言）。
/// ⇒ 锁必须全 crate 唯一，故提升到模块级并 `pub(crate)`。
///
/// 只用「原子布尔 / 只读」的用例（`domain_registry` 里上面那些）不受影响，无需持锁。
#[cfg(test)]
pub(crate) mod test_support {
    /// 全 crate 唯一的覆盖层用例锁。
    ///
    /// # 为什么是 `tokio::sync::Mutex` 而不是 `std::sync::Mutex`
    ///
    /// 用例的临界区**必须**跨 `await`：断言对象是 `router.route(q).await` 的返回值，
    /// 而覆盖层在整个 `await` 期间都不许被别的用例改。std guard 跨 await 同时踩两条
    /// clippy 硬拦（都在 `-D warnings` 下算 error，不是风格建议）：
    ///   - `clippy::disallowed_types` —— 本仓 `clippy.toml` 禁用 `std::sync::{Mutex, …}`；
    ///   - `clippy::await_holding_lock` —— 原话「this `MutexGuard` is held across an await point」。
    ///
    /// 换异步锁后**两条都不再触发** ⇒ 本模块**不需要任何 `#[allow]` 豁免**，也就没有
    /// 「豁免注释与事实不符」的余地（前车之鉴：我写过一版「不跨 await」的豁免注释，
    /// 事实恰好相反，被 clippy 当场抓住）。
    /// 附带收益：tokio 锁无中毒语义 ⇒ 少一层 `unwrap_or_else(|p| p.into_inner())`。
    static OVERLAY_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// 取覆盖层用例锁（异步，无中毒语义）。给 `#[tokio::test]` 用例用。
    pub(crate) async fn overlay_guard() -> tokio::sync::MutexGuard<'static, ()> {
        OVERLAY_TEST_LOCK.lock().await
    }

    /// 覆盖层用例的统一入口（异步）：**前置清空** + 返回锁。
    ///
    /// **前置清空**是必须的：上一个用例若在断言处 panic 就没走到末尾的清理，
    /// 残留覆盖会让下一个用例看到别人的状态（假红）。
    pub(crate) async fn overlay_case() -> tokio::sync::MutexGuard<'static, ()> {
        let g = overlay_guard().await;
        super::clear_domain_overrides();
        g
    }

    /// 同步入口：给本模块里那批 `#[test]`（**不走运行时**）用。
    ///
    /// 为什么不直接让它们也 `await`：它们是 `#[test]` 而非 `#[tokio::test]`，
    /// 改成异步要动 8 个签名，而它们只调用同步 API，没有异步的必要。
    ///
    /// `blocking_lock()` 在**异步上下文里调用会 panic** —— 本函数成立的前提正是
    /// 「`#[test]` 用例体内没有进入 tokio 运行时」。若日后把某个同步用例改成
    /// `#[tokio::test]`，必须同时改走 [`overlay_case`]，否则会 panic 而不是编译报错。
    pub(crate) fn overlay_case_blocking() -> tokio::sync::MutexGuard<'static, ()> {
        let g = OVERLAY_TEST_LOCK.blocking_lock();
        super::clear_domain_overrides();
        g
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::overlay_case_blocking;
    use super::*;

    /// 规范 id = `general` 等 9 个；其余 18 条为历史别名。
    const LEGACY_ALIASES: [(&str, &str); 18] = [
        ("core", "general"),
        ("device", "general"),
        ("dynamic_ui", "general"),
        ("fine_tune", "general"),
        ("cloud", "general"),
        ("rl_training", "general"),
        ("context", "general"),
        ("conversation", "general"),
        ("pty", "devops"),
        ("db_config", "devops"),
        ("invest", "finance"),
        ("quant", "finance"),
        ("portfolio", "finance"),
        ("stock_analysis", "finance"),
        ("opc", "automation"),
        ("workflow", "automation"),
        ("orchestrator", "system"),
        ("agent", "system"),
    ];

    /// 声明必须**无重复、不遗漏**，且 id 可往返解析回自身。
    ///
    /// ⚠ 「不遗漏」在此只能以**数量 + 往返**近似断言（枚举变体无法在运行期枚举）。
    /// 真正的「不遗漏」由 [`_variant_tripwire`]（编译期）+ 门禁（逐值比对）保证。
    #[test]
    fn test_nodes_are_distinct_and_round_trip() {
        assert_eq!(DOMAIN_NODES.len(), 9, "域节点数应等于 CapabilityDomain 变体数（9）");
        let mut slugs: Vec<&'static str> = DOMAIN_NODES.iter().map(|n| n.slug()).collect();
        slugs.sort_unstable();
        let unique = {
            let mut s = slugs.clone();
            s.dedup();
            s
        };
        assert_eq!(slugs, unique, "存在重复域节点：{:?}", slugs);

        for node in DOMAIN_NODES {
            let parsed: CapabilityDomain = node
                .slug()
                .parse()
                .unwrap_or_else(|_| panic!("域 id {} 无法被 FromStr 解析回枚举", node.slug()));
            assert_eq!(parsed, node.domain, "域 {} 的 id 往返解析后不是自身", node.slug());
        }
    }

    /// 协议顺序（= 声明顺序）必须与 `as_str()` 的输出顺序逐项一致。
    ///
    /// 门禁把本数组顺序当作「协议顺序」的权威值喂给前端比对，
    /// 若声明顺序与枚举顺序不符，门禁会把**正确的前端**判成错 —— 故单独钉住。
    ///
    /// ⚠ 断言右侧的 `expected` 是**独立硬编码**值（本仓惯例，同
    /// `capability_clusters::test_cluster_count`）。**不得**改成从 `DOMAIN_NODES` 或
    /// `as_str()` 派生 —— 那样两边同源、断言恒真，就失去了判别力。
    #[test]
    fn test_declaration_order_matches_enum_order() {
        let declared: Vec<&'static str> = DOMAIN_NODES.iter().map(|n| n.slug()).collect();
        let expected = [
            "general",
            "devops",
            "ai_media",
            "data_analysis",
            "content_creation",
            "communication",
            "finance",
            "automation",
            "system",
        ];
        assert_eq!(
            declared, expected,
            "声明顺序必须等于 CapabilityDomain 的枚举声明顺序（协议顺序）"
        );
    }

    /// 内部域不进入导航；业务域必须有导航路径与唯一顺序。
    #[test]
    fn test_nav_invariants() {
        for node in DOMAIN_NODES {
            if node.domain.is_system() {
                assert!(node.nav_path.is_none(), "内部域 {} 不得有导航路径", node.slug());
                assert!(node.nav_order.is_none(), "内部域 {} 不得有导航顺序", node.slug());
            } else {
                let path =
                    node.nav_path.unwrap_or_else(|| panic!("业务域 {} 缺导航路径", node.slug()));
                assert!(path.starts_with('/'), "域 {} 的导航路径须以 / 开头：{path}", node.slug());
                assert!(node.nav_order.is_some(), "业务域 {} 缺导航顺序", node.slug());
            }
        }

        let mut orders: Vec<u8> =
            business_domains().iter().filter_map(|d| node_of(*d).nav_order).collect();
        assert_eq!(orders.len(), 8, "8 个业务域都应有导航顺序");
        orders.sort_unstable();
        assert_eq!(orders, (0..8).collect::<Vec<u8>>(), "导航顺序须为 0..8 连续无重复");
    }

    /// 导航路径唯一（重复会让侧栏两处指向同一页）。
    #[test]
    fn test_nav_paths_unique() {
        let mut paths: Vec<&'static str> = DOMAIN_NODES.iter().filter_map(|n| n.nav_path).collect();
        let total = paths.len();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), total, "存在重复导航路径");
    }

    /// `business_domains()` 必须排除内部域，且数量为 8。
    #[test]
    fn test_business_domains_excludes_system() {
        let business = business_domains();
        assert_eq!(business.len(), 8, "业务域应为 8 个");
        assert!(!business.contains(&CapabilityDomain::System), "内部域不得出现在业务域列表");
    }

    /// **别名迁移的等价性证据**（判据：等价改写必须给出行为级断言）。
    ///
    /// 把 18 条别名各自的**期望目标域**逐条写死并调用 `from_str` —— 迁移时漏掉任何一条，
    /// 或把某条挂到错误的目标域上，本用例即红。
    ///
    /// ⚠ 这张表是**独立硬编码**值（不是从 `DOMAIN_NODES` 派生）—— 派生版会与被测对象同源，
    /// 断言恒真、失去判别力（判据 #174）。
    #[test]
    fn test_legacy_aliases_resolve() {
        for (alias, expected) in LEGACY_ALIASES {
            let got: CapabilityDomain =
                alias.parse().unwrap_or_else(|_| panic!("历史别名 {alias} 解析失败"));
            assert_eq!(
                got.as_str(),
                expected,
                "别名 {alias} 应指向 {expected}，实际 {}",
                got.as_str()
            );
        }
    }

    /// 声明的别名集合必须与上面那张独立表**逐条一致**（双向）。
    ///
    /// 前一个用例保证「表里的都解析对」，本用例保证「声明的都在表里」——
    /// 只有两个方向都断言，才能证明别名表**没有悄悄增删**。
    #[test]
    fn test_declared_aliases_match_independent_table() {
        let mut declared: Vec<(&'static str, &'static str)> = Vec::new();
        for node in DOMAIN_NODES {
            for a in node.aliases {
                declared.push((*a, node.slug()));
            }
        }
        let mut expected: Vec<(&'static str, &'static str)> =
            LEGACY_ALIASES.iter().map(|(a, d)| (*a, *d)).collect();
        declared.sort_unstable();
        expected.sort_unstable();
        assert_eq!(declared, expected, "DOMAIN_NODES 声明的别名与独立表不一致（漏声明或多余声明）");
    }

    /// 别名不得与任何规范 id 同名 —— 否则解析会被**另一个域劫持**。
    ///
    /// 例：若给 `Finance` 加上别名 `automation`，则 `"automation".parse()` 可能先命中
    /// `Finance`（取决于遍历顺序）⇒ `automation` 这个域**永远解析不出来**，且不报错。
    #[test]
    fn test_aliases_do_not_shadow_canonical_ids() {
        let slugs: Vec<&'static str> = DOMAIN_NODES.iter().map(|n| n.slug()).collect();
        for node in DOMAIN_NODES {
            for a in node.aliases {
                assert!(
                    !slugs.contains(a),
                    "别名 {a} 与规范 id 同名 ⇒ 会遮蔽该域（声明在 {} 之下）",
                    node.slug()
                );
            }
        }
    }

    /// 别名必须在全局唯一（同一个词不能挂在两个域下）。
    #[test]
    fn test_aliases_globally_unique() {
        let mut seen: Vec<(&'static str, &'static str)> = Vec::new();
        for node in DOMAIN_NODES {
            for a in node.aliases {
                if let Some((_, owner)) = seen.iter().find(|(x, _)| x == a) {
                    panic!("别名 {a} 同时声明在 {owner} 与 {} 之下", node.slug());
                }
                seen.push((*a, node.slug()));
            }
        }
    }

    /// 每个别名都必须能解析回**它自己所属**的那个域（不只是「能解析」）。
    #[test]
    fn test_alias_resolves_to_own_domain() {
        for node in DOMAIN_NODES {
            for a in node.aliases {
                let got: CapabilityDomain =
                    a.parse().unwrap_or_else(|_| panic!("别名 {a} 解析失败"));
                assert_eq!(
                    got,
                    node.domain,
                    "别名 {a} 解析到了 {got}，但它声明在 {} 之下",
                    node.slug()
                );
            }
        }
    }

    /// [`DomainNode::label_key`] 的公式必须与前端 `domainLabelKey()` 一致。
    ///
    /// 这是一条**跨语言字符串契约**（Rust 算 key、TS 算 key、i18n 表存 key）：
    /// 两侧公式不同时，界面显示裸 key 而**两侧编译器都沉默**。故用两道断言钉住：
    /// ① 右侧是**独立硬编码**的完整 key（不是从 `slug()` 派生 —— 派生版与被测对象同源、
    ///    恒真、失去判别力，判据 #174）；② 遍历全部节点保证命名空间前缀统一
    ///    （防止只有某一个域被写成别的命名空间）。
    ///
    /// ⚠ 「两侧公式一致」这件事**单测测不到**（只能证明本侧）。跨侧一致由门禁
    /// `check-domain-single-source.mjs` 的真实注入用例负责。
    #[test]
    fn test_label_key_formula() {
        assert_eq!(node_of(CapabilityDomain::Finance).label_key(), "capabilityDomain.finance");
        assert_eq!(node_of(CapabilityDomain::AiMedia).label_key(), "capabilityDomain.ai_media");
        assert_eq!(node_of(CapabilityDomain::System).label_key(), "capabilityDomain.system");

        for node in DOMAIN_NODES {
            let key = node.label_key();
            let suffix = key.strip_prefix("capabilityDomain.").unwrap_or_else(|| {
                panic!(
                    "label_key 的命名空间已偏离 `capabilityDomain.`：{key}\n\
                     （前端 domainLabelKey() 会算出不同的 key ⇒ 界面显示裸 key，两侧编译器都不报错）"
                )
            });
            assert_eq!(suffix, node.slug(), "label_key 的后缀必须恰好是域 id 本身：{key}");
        }
    }

    // ── 运行时覆盖层（P2）────────────────────────────
    //
    // 用例统一走 `test_support::overlay_case_blocking()`（前置清空 + 取**全 crate 唯一**的锁）。
    // 本模块这批是 `#[test]`（不走运行时）⇒ 用 blocking 入口；
    // 异步用例（`domain_router` 的 route 用例）走 `overlay_case().await`。
    // 锁为什么不在本模块内定义、为什么不能只在本模块有效、为什么用 tokio 锁 ⇒ 见 `test_support` 文档。

    /// 无覆盖时：全部域启用，且**没有任何域被标为「有覆盖」**。
    #[test]
    fn test_overlay_defaults_to_all_enabled() {
        let _g = overlay_case_blocking();

        assert_eq!(enabled_domains().len(), 9, "无覆盖时 9 个域应全部启用");
        assert_eq!(enabled_business_domains().len(), 8, "无覆盖时 8 个业务域应全部启用");
        for node in DOMAIN_NODES {
            assert!(!has_override(node.domain), "{} 不应有覆盖行", node.slug());
            assert!(is_domain_enabled(node.domain), "{} 默认应启用", node.slug());
        }
    }

    /// **L1 分类器清单的完整性**（原由门禁正则守，P2 起由本用例守）。
    ///
    /// ⚠ 右侧是**独立硬编码**的 9 个 slug（不是从 `DOMAIN_NODES` 或 `enabled_domains()`
    /// 派生）—— 派生版与被测对象同源、断言恒真、失去判别力（判据 #174）。
    /// 新增域时本用例会红，正是要提醒「候选清单与顺序必须有人确认」。
    #[test]
    fn test_l1_classifier_domain_list_covers_all_when_enabled() {
        let _g = overlay_case_blocking();

        assert_eq!(
            l1_classifier_domain_list(),
            "general, devops, ai_media, data_analysis, content_creation, communication, \
             finance, automation, system"
        );
    }

    /// **消费端① 的核心断言**：停用一个域 ⇒ 它从 LLM 候选集里消失，其余不变。
    ///
    /// 这是「改前 / 改后行为不同」的直接证据：同一个函数、同一份代码，
    /// 只因覆盖层不同而返回不同的候选集。
    #[test]
    fn test_disabling_domain_drops_it_from_classifier_list() {
        let _g = overlay_case_blocking();

        let before = l1_classifier_domain_list();
        assert!(before.contains("finance"), "前置：启用时 finance 应在候选集里：{before}");

        apply_domain_overrides(vec![DomainOverride::with_enabled(
            CapabilityDomain::Finance,
            false,
        )]);

        let after = l1_classifier_domain_list();
        assert!(!after.contains("finance"), "停用后 finance 必须从候选集消失：{after}");
        assert!(after.contains("automation"), "只停用 finance，不应牵连 automation：{after}");
        assert_eq!(after.split(", ").count(), 8, "9 个域停用 1 个 ⇒ 候选集应剩 8 个：{after}");

        clear_domain_overrides();
        assert_eq!(l1_classifier_domain_list(), before, "清空覆盖后应完全还原");
    }

    /// `general` 与 `system` **不可停用**：覆盖层写进去也不生效（读侧兜底）。
    ///
    /// 断言的是**读侧**行为而不只是 [`is_toggleable`]：即使坏数据绕过命令层直接进库，
    /// 也不能让「唯一兜底域」变成停用态（否则 `route()` 未命中的返回值会自相矛盾）。
    #[test]
    fn test_general_and_system_cannot_be_disabled() {
        let _g = overlay_case_blocking();

        assert!(!is_toggleable(CapabilityDomain::General));
        assert!(!is_toggleable(CapabilityDomain::System));
        assert_eq!(toggle_block_reason(CapabilityDomain::General), Some("general_is_fallback"));
        assert_eq!(toggle_block_reason(CapabilityDomain::System), Some("system_is_internal"));
        assert_eq!(toggle_block_reason(CapabilityDomain::Finance), None);

        apply_domain_overrides(vec![
            DomainOverride::with_enabled(CapabilityDomain::General, false),
            DomainOverride::with_enabled(CapabilityDomain::System, false),
        ]);

        assert!(is_domain_enabled(CapabilityDomain::General), "general 读侧必须仍为启用");
        assert!(is_domain_enabled(CapabilityDomain::System), "system 读侧必须仍为启用");
        assert!(l1_classifier_domain_list().contains("general"));
        assert!(l1_classifier_domain_list().contains("system"));

        clear_domain_overrides();
    }

    /// **`resolve_enabled_domain` 与 `from_str` 的语义分工**（停用只影响入口，不影响存量读取）。
    ///
    /// 停用 `finance` 后：
    /// - 用户输入 / LLM 输出入口（`resolve_enabled_domain`）不再接受它 —— 也不接受它的别名；
    /// - 但 `from_str`（DB 存量字符串 / serde 反序列化路径）**仍然解析成功** ——
    ///   否则存量 `active_domains` / `route_path` 会静默解析失败（PLAN §7.2）。
    #[test]
    fn test_resolve_enabled_domain_rejects_disabled_but_from_str_still_parses() {
        let _g = overlay_case_blocking();

        assert_eq!(resolve_enabled_domain("finance"), Some(CapabilityDomain::Finance));
        assert_eq!(
            resolve_enabled_domain("  INVEST "),
            Some(CapabilityDomain::Finance),
            "内置别名应大小写/空白无关地解析"
        );

        apply_domain_overrides(vec![DomainOverride::with_enabled(
            CapabilityDomain::Finance,
            false,
        )]);

        assert_eq!(resolve_enabled_domain("finance"), None, "停用域不得从入口解析出来");
        assert_eq!(resolve_enabled_domain("invest"), None, "停用域的别名同样不得解析出来");
        assert_eq!(
            "finance".parse::<CapabilityDomain>().ok(),
            Some(CapabilityDomain::Finance),
            "from_str 是存量读取路径，必须无视覆盖层（否则存量数据读不出来）"
        );
        assert_eq!(
            resolve_enabled_domain("automation"),
            Some(CapabilityDomain::Automation),
            "未停用的域不受影响"
        );

        clear_domain_overrides();
    }

    /// 追加别名：能解析到目标域；目标域停用后随之一并失效。
    #[test]
    fn test_extra_aliases_resolve_and_respect_enabled() {
        let _g = overlay_case_blocking();

        assert_eq!(resolve_enabled_domain("股票分析"), None, "未追加时不应能解析");

        apply_domain_overrides(vec![DomainOverride::new(
            CapabilityDomain::Finance,
            true,
            vec!["股票分析".to_string()],
        )]);

        assert_eq!(
            resolve_enabled_domain("股票分析"),
            Some(CapabilityDomain::Finance),
            "追加别名应解析到声明它的域"
        );

        apply_domain_overrides(vec![DomainOverride::new(
            CapabilityDomain::Finance,
            false,
            vec!["股票分析".to_string()],
        )]);
        assert_eq!(
            resolve_enabled_domain("股票分析"),
            None,
            "域停用后其追加别名也不得解析出来（否则绕过了停用）"
        );

        clear_domain_overrides();
    }

    /// `effective_aliases` 是**并集**而非替换：内置 27 条永不因覆盖层消失。
    #[test]
    fn test_effective_aliases_is_union_not_replacement() {
        let _g = overlay_case_blocking();

        let builtin = node_of(CapabilityDomain::Finance).aliases.len();
        assert_eq!(
            effective_aliases(CapabilityDomain::Finance).len(),
            builtin,
            "无覆盖时应等于内置别名数"
        );

        apply_domain_overrides(vec![DomainOverride::new(
            CapabilityDomain::Finance,
            true,
            vec!["股票分析".to_string(), "invest".to_string()],
        )]);

        let got = effective_aliases(CapabilityDomain::Finance);
        assert_eq!(got.len(), builtin + 1, "追加 1 条新别名（invest 已存在应去重）：{got:?}");
        for a in node_of(CapabilityDomain::Finance).aliases {
            assert!(got.iter().any(|x| x == a), "内置别名 {a} 不得因覆盖层消失");
        }

        clear_domain_overrides();
    }

    /// `apply_domain_overrides` 是**整表替换**，不是合并。
    ///
    /// 若做成合并语义，「把某个域改回默认」就做不到（旧覆盖行会一直生效），
    /// 而 DAO 每次都是读全表调用它 ⇒ 合并会让已删除的行阴魂不散。
    #[test]
    fn test_apply_overrides_replaces_not_merges() {
        let _g = overlay_case_blocking();

        apply_domain_overrides(vec![DomainOverride::with_enabled(
            CapabilityDomain::Finance,
            false,
        )]);
        assert!(!is_domain_enabled(CapabilityDomain::Finance));

        apply_domain_overrides(vec![DomainOverride::with_enabled(
            CapabilityDomain::Automation,
            false,
        )]);
        assert!(
            is_domain_enabled(CapabilityDomain::Finance),
            "第二次 apply 是整表替换 ⇒ finance 应回到内置默认（启用）"
        );
        assert!(!is_domain_enabled(CapabilityDomain::Automation));

        clear_domain_overrides();
        assert!(is_domain_enabled(CapabilityDomain::Automation), "清空即回到出厂状态");
    }
}
