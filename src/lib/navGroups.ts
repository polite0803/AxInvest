// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 侧栏「一级导航组」—— 产品导航轴，**不是**能力域。
 *
 * ⚠ 与 `domainMeta.ts` 的分工：
 *   - `CapabilityDomain`（8 个）是后端协议概念，`domainMeta.ts` 是它的前端镜像，
 *     受 `check-domain-single-source.mjs` 硬拦，**不得**为产品导航新增/改名。
 *   - 本文件是**纯前端**的导航分组：面向用户把 30+ 个入口收成 6 组，
 *     不参与任何协议校验，也不进入 `NAV_ITEM_DOMAIN_MAP`。
 *
 * 为什么需要（2026-10-28）：此前侧栏按 8 个能力域分组，`general` 组承载了
 * chat/workflow/terminal/files/knowledge/memory/wiki/learning 等 12 项，
 * 用户找不到入口；同时 8 个域里有 4 个只挂 1–2 项。产品导航与能力域是两条轴，
 * 混用会让「域」既当协议又当菜单。
 *
 * 新增一项时：在 `navRegistry.builtinNavItems` 加 item，再把 key 放进对应组的
 * `itemKeys`；`check-route-reachability` 会校验每条路由仍有入口。
 */

export interface NavGroup {
  /** 组 key（稳定，仅用于 React key 与测试） */
  key: string;
  /** i18n key（复用既有 `nav.*`，不新增文案） */
  labelKey: string;
  /** 组标题圆点色（纯呈现） */
  color: string;
  /** 组标题点击路径；缺省表示不可点 */
  path?: string;
  /** 组内导航项 key，顺序即渲染顺序 */
  itemKeys: readonly string[];
}

/** OPC 行业包（14 个），单独成组以免挤占经营面板入口。 */
const PACK_ITEM_KEYS = [
  "finance-analysis",
  "finance-accounting",
  "automation-sales",
  "automation-projects",
  "automation-consulting",
  "automation-ecommerce",
  "devops-software",
  "devops-security",
  "data-geospatial",
  "data-ai-research",
  "content-media",
  "content-design",
  "content-education",
  "ai-media-game",
] as const;

export const NAV_GROUPS: readonly NavGroup[] = [
  {
    key: "chat",
    labelKey: "nav.workbench",
    color: "#3b82f6",
    path: "/chat",
    itemKeys: [
      "chat",
      "chat-workflow",
      "chat-terminal",
      "chat-files",
      "chat-agents",
      "chat-marketplace",
    ],
  },
  {
    key: "invest",
    labelKey: "nav.financeInvestment",
    color: "#d4380d",
    path: "/invest",
    itemKeys: ["finance-investment", "invest-replay"],
  },
  {
    key: "knowledge",
    labelKey: "nav.knowledge",
    color: "#10b981",
    path: "/knowledge",
    itemKeys: ["knowledge", "knowledge-memory", "knowledge-wiki", "knowledge-learning"],
  },
  {
    key: "opc",
    labelKey: "nav.automationOperations",
    color: "#722ed1",
    path: "/opc",
    itemKeys: ["automation-operations", "demand-discovery"],
  },
  {
    key: "packs",
    // `opc.nav.market` = “能力包”，避免与工作台组的“市场”重名，也不新增 11 语言文案。
    labelKey: "opc.nav.market",
    color: "#fa8c16",
    itemKeys: PACK_ITEM_KEYS,
  },
  {
    key: "tasks",
    labelKey: "nav.tasksCenter",
    color: "#8c8c8c",
    path: "/tasks",
    itemKeys: ["tasks-center"],
  },
  {
    key: "system",
    labelKey: "nav.settings",
    color: "#64748b",
    path: "/settings",
    itemKeys: ["gateway"],
  },
];
