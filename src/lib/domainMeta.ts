// SPDX-License-Identifier: AGPL-3.0-only
// ! 前端能力域（8+1）唯一权威映射
//
// 对齐后端 axagent_harness::CapabilityDomain（8 个业务功能域 + System 内部域）。
// 本文件是前端「域」概念的单一真相源：
//   - 侧栏导航按域分组
//   - 页面/行业按业务本质归入唯一标准域
//   - 大导航用域作为一级组织轴（能力发现也以域一级过滤）
//
// 设计约束（与后端 capability.rs 一致）：
//   - 只允许 8 个业务域 + System 内部域，禁止引入自定义/产品线域。
//   - 业务线（如股票投研/一人公司）通过标签表达，不占域轴。
//   - General 是唯一兜底域。
//   - System 仅配合 SystemOnly，永不进入检索与导航。
//
// ## 结构（2026-09-15 P1-④ 收敛 —— 改本文件前先读这段）
//
//   域 id 集合  →  `DOMAIN_PRESENTATION` 的 **key 集合**（**唯一**手写点；`tsc` 强制穷尽）
//   协议顺序    →  `DOMAIN_PRESENTATION` 的 **key 声明顺序**（派生，见 PROTOCOL_ORDER）
//   path/color  →  `DOMAIN_PRESENTATION` 的 **值**
//   导航顺序    →  `NAV_ORDER`（同一批 id 的**另一种排列**，无法派生，故为第二处手写）
//
//   ⇒ 三个导出视图（`CAPABILITY_DOMAIN_META` / `CAPABILITY_DOMAIN_PROTOCOL_ORDER` /
//     `CAPABILITY_DOMAIN_OPTIONS`）**全部是派生**，自身不再手抄任何 id。
//
//   收敛前的形态是「两个数组各自手写同一批 8 个 id」——集合维度有两个独立手写点，
//   且数组漏一个元素仍是合法数组（`tsc` 拦不住），只有门禁能守。
//   现在集合维度只有一个点，且**漏一个域 `tsc` 直接报错**。

import type { CapabilityDomain } from "@/types/capability";

// ── 域元数据 ──────────────────────────────────────

/**
 * 业务域 = 全部域中除**内部域**外的部分（内部域永不进入导航与检索）。
 *
 * ⚠ 此处出现的 `"system"` 字面量**不是**手抄副本，而是「本表不含内部域」这条断言的
 * 表达方式。若后端把内部域改名，`Exclude` 会失效 ⇒ 下面的 `DOMAIN_PRESENTATION`
 * 会因缺少该 key 而**编译失败** —— 这正是期望的方向（宁可编译不过，也不静默漏域）。
 * 运行时层面另有门禁逐值比对，二者互为兜底。
 */
type BusinessDomain = Exclude<CapabilityDomain, "system">;

export interface CapabilityDomainMeta {
  /** 域 id（与后端 CapabilityDomain 完全一致，snake_case） */
  id: CapabilityDomain;
  /** 域一处聚合入口路径（用户选择的「域路径」导航目标） */
  path: string;
  /** 主题色（用于图标/标签高亮）——**纯前端呈现决策**：后端没有任何渲染路径读它 */
  color: string;
  // ⚠ 此处曾有 `order: number`（导航分组顺序）。2026-09-15 **删除**：
  //   它与**数组下标**逐项相等（0..7），是冗余副本 —— 挪动条目却忘改 `order`
  //   就会静默乱序；且实测全仓**零读取端**（下面那张表的消费点只读 id / path / color）。
  //   导航顺序现在由**数组顺序**唯一表达，门禁按**下标**与后端 `DOMAIN_NODES.nav_order` 比对。
  // ⚠ 此处曾有 `labelKey: string`（取值形如 `domain.<camelCase>`）。
  //   2026-09-15 收敛：显示名一律由 `domainLabelKey(id)` 派生，不再手抄 ——
  //   原因见下方 `domainLabelKey` 的说明。
}

/**
 * 域的 i18n 显示名 key —— **唯一派生入口**。
 *
 * 历史背景（勿重新引入手抄）：本仓曾有**两套**域标签命名空间表达同一批域 ——
 * `capabilityDomain.<snake_case>`（与域 id 同名）与 `domain.<camelCase>`
 * （`domain.dataAnalysis` / `domain.aiMedia` …）。两张表在 11 个 locale 里各存一份，
 * 增删一个域要改两处、且改名极易漏侧。2026-09-15 收敛为单套：
 * `domain.*` 只保留该页专属的 `description` / `empty`，标签全部由本函数派生。
 *
 * 出处：`PLAN-domain-single-source.md` §4 / §9.2（P1-③）；巡检报告见
 * `scripts/check-domain-single-source.mjs`：
 *   · **硬拦**本函数的命名空间与 Rust 侧 `DomainNode::label_key()`
 *     （`crates/harness/src/domain_registry.rs`）不一致 —— 两侧是**同一公式**：
 *     后端持有声明端、前端持有调用端。只改一侧时界面显示裸 key（i18n 查不到该 key），
 *     而 `tsc` / `cargo` **双双沉默**（两侧各自都是合法代码）⇒ 只能由门禁守。
 *   · 报告档：`domain.*` 下重新出现非 `description`/`empty` 的 key 时会点名。
 *
 * ⚠ 本函数**被门禁以正则读取**（契约形态：`return \`<命名空间>.${参数名}\`;`）——
 *   改成 `switch`、拆成多段拼接、把命名空间挪进常量、或让插值变成复合表达式
 *   （如 `${globalThis.x}`），门禁会报「公式解析失败」/「插值不可信」并**硬拦**。
 */
export function domainLabelKey(id: CapabilityDomain): string {
  return `capabilityDomain.${id}`;
}

/**
 * 域的呈现元数据 —— 前端**唯一**持有域 id 集合的地方。
 *
 * ## 为什么是 `Record` 而不是数组
 * `Record<BusinessDomain, …>` 的 **key 集合被 `tsc` 强制穷尽**：后端新增一个域而这里
 * 漏了 ⇒ `tsc` 报 `Property 'x' is missing`。数组形态做不到这件事（漏一个元素仍是
 * 合法数组），只能靠门禁事后守。
 *
 * ## 声明顺序 = 协议（枚举）顺序
 * 下面 `CAPABILITY_DOMAIN_PROTOCOL_ORDER` 由 `Object.keys()` **派生**本表，
 * 故**本表的条目顺序就是协议顺序**。协议顺序是一条跨语言硬契约（逐项等于
 * `crates/harness/src/domain_registry.rs` 的 `DOMAIN_NODES` 声明顺序）。
 * ⇒ **不要为了对齐 color / 排版而重排本表**：那会静默改掉协议顺序
 * （门禁会红，但改动意图与后果毫不相干）。
 *
 * ## 导航分组顺序是**另一件事**
 * 见 `NAV_ORDER` —— 侧栏分组用的是另一套排列，两者都合法但用途不同。
 *
 * ## color 是纯前端呈现决策
 * 后端没有任何渲染路径读它，故不上报为声明字段（否则造出「零读取端字段」）。
 */
const DOMAIN_PRESENTATION = {
  general: { path: "/general", color: "#8c8c8c" },
  devops: { path: "/devops", color: "#13c2c2" },
  ai_media: { path: "/ai-media", color: "#fa8c16" },
  data_analysis: { path: "/data-analysis", color: "#2f54eb" },
  content_creation: { path: "/content-creation", color: "#eb2f96" },
  communication: { path: "/communication", color: "#52c41a" },
  finance: { path: "/finance", color: "#d4380d" },
  automation: { path: "/automation", color: "#722ed1" },
} as const satisfies Record<BusinessDomain, { readonly path: string; readonly color: string }>;

/**
 * 导航分组顺序（侧栏域分区顺序，下标 0..7）。
 *
 * ⚠ 这是本文件**唯一**仍需手写 id 的地方：它是同一批域的**另一种排列**，
 * 无法从 `DOMAIN_PRESENTATION` 派生（信息论上没有互相派生的可能）。
 * 能做到的防护是把「拼错」交给 `tsc`（元素类型 `BusinessDomain`），
 * 把「不重不漏 + 与后端 `nav_order` 一致」交给门禁**硬拦**
 * （集合比对 + 逐项比对 `DOMAIN_NODES.nav_order`）。
 *
 * ⚠ 不要再引入 `order` 字段（2026-09-15 已删）：它与**数组下标**逐项相等（冗余），
 * 且全仓零读取端 —— 挪动条目却忘改 `order` 就会静默乱序。
 */
const NAV_ORDER = [
  "general",
  "finance",
  "automation",
  "devops",
  "data_analysis",
  "content_creation",
  "ai_media",
  "communication",
] as const satisfies readonly BusinessDomain[];

/**
 * 8 个业务功能域（不含 System，System 永不进入导航）—— **派生视图**。
 *
 * `id` 取自 `NAV_ORDER`（导航顺序），`path` / `color` 取自 `DOMAIN_PRESENTATION`。
 * ⇒ 本表自身不手抄任何 id / path / color（P1-④）；手抄点收敛为上面两处。
 */
export const CAPABILITY_DOMAIN_META: readonly CapabilityDomainMeta[] = NAV_ORDER.map((id) => ({
  id,
  path: DOMAIN_PRESENTATION[id].path,
  color: DOMAIN_PRESENTATION[id].color,
}));

// ── 域勾选/筛选的协议顺序与选项 ────────────────────
//
// ⚠ 为什么不复用上面 `CAPABILITY_DOMAIN_META` 的顺序：
//   那张表的顺序是**导航分组顺序**（general→finance→automation→…），
//   而 L1 路由、工具域过滤、prompt 域清单用的是**协议（枚举）顺序**
//   （general→devops→ai_media→…）。两者都合法但用途不同 —— 混用会让同一下拉项
//   在两次重构之间来回跳（此前 `AgentProfileManager` / `ExpertSelector` 各自手抄过
//   一份协议顺序，属于重复定义）。此处把协议顺序显式钉住，下拉一律由它派生。
//
// 2026-09-15：本文件原有的 `CAPABILITY_DOMAIN_BY_ID`（id → 元数据）与
// `CAPABILITY_DOMAIN_IDS`（id 集合）**已删除** —— 实测全仓**零消费**（含测试、e2e），
// 属「零读取端导出」；域 id 集合的权威比对由门禁负责，不需要运行时副本。

/**
 * 域在协议（枚举）中的顺序 —— **派生**自 `DOMAIN_PRESENTATION` 的 key 声明顺序，不含 `system`。
 *
 * 用途：L1 路由候选、工具域过滤、prompt 域清单、勾选/筛选下拉（经 `CAPABILITY_DOMAIN_OPTIONS`）。
 *
 * ⚠ 它不是可自由调整的产品决策：逐项等于后端
 * `crates/harness/src/domain_registry.rs` 的 `DOMAIN_NODES` 声明顺序，
 * 门禁 `scripts/check-domain-single-source.mjs` **硬拦**（集合 + **顺序**双向校验）。
 * 要改顺序，先改后端声明 —— 改单侧即红。
 *
 * ⚠ 顺序**在此之前只是报告档**，理由是「手抄副本的顺序可能是有意的产品决策，
 * 硬拦会逼人改判据来灭红灯」（判据 #147）。P1-② 把协议顺序**定义为**声明顺序的镜像后，
 * 该理由不再成立（已无自由决策空间，两侧不一致就是缺陷，且症状静默：
 * 下拉项在两次重构之间来回跳、prompt 候选序与枚举序错位），故升为硬拦。
 * ⇒ 不要把这条降回报告档：那等于把「顺序漂移」重新变成无人守的静默故障。
 *
 * ⚠ **不要再写回字面量数组**（P1-④）：本常量此前是 8 个手抄 id，与
 * `CAPABILITY_DOMAIN_META` 各自独立手写同一批域 —— 集合维度因此有两个会各自腐烂的
 * 手写点。现在集合只在 `DOMAIN_PRESENTATION` 出现一次（且受 `tsc` 穷尽检查），
 * 本常量只是它的读取视图。门禁**硬拦**「改回字面量数组」这一形态。
 */
export const CAPABILITY_DOMAIN_PROTOCOL_ORDER: readonly CapabilityDomain[] = Object.keys(
  DOMAIN_PRESENTATION,
) as BusinessDomain[];

/** 域选项（用于勾选/筛选控件）。`labelKey` 由 `domainLabelKey(id)` 派生，**禁止在组件里手抄本表**。 */
export const CAPABILITY_DOMAIN_OPTIONS: readonly {
  value: CapabilityDomain;
  labelKey: string;
}[] = CAPABILITY_DOMAIN_PROTOCOL_ORDER.map((id) => ({
  value: id,
  labelKey: domainLabelKey(id),
}));

// ── 导航项归域表 ──────────────────────────────────
//
// 将侧栏内置导航项（NavItem.key）按业务本质归入唯一标准域。
// 这是「导航以域为标准」的权威归域来源。

/** 导航项 key → 标准域 id */
export const NAV_ITEM_DOMAIN_MAP: Readonly<Record<string, CapabilityDomain>> = {
  // 通用域
  chat: "general",
  // 金融域
  "finance-investment": "finance",
  "finance-analysis": "finance",
  "finance-accounting": "finance",
  // 自动化域
  "demand-discovery": "automation",
  "automation-operations": "automation",
  "automation-sales": "automation",
  "automation-projects": "automation",
  "automation-consulting": "automation",
  "automation-ecommerce": "automation",
  // 运维域
  "devops-software": "devops",
  "devops-security": "devops",
  // 数据分析域
  "data-geospatial": "data_analysis",
  "data-ai-research": "data_analysis",
  // 内容创作域
  "content-media": "content_creation",
  "content-design": "content_creation",
  "content-education": "content_creation",
  // AI 媒体域
  "ai-media-game": "ai_media",
  // 通信域
  "communication-message": "communication",
};

/** 根据导航项 key 解析其所属标准域；未知项兜底 general */
export function domainForNavKey(key: string): CapabilityDomain {
  return NAV_ITEM_DOMAIN_MAP[key] ?? "general";
}
