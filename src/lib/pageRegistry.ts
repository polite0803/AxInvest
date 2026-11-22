// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Unified page registry — **宿主自有**内置路由路径的单一真相源。
 *
 * 所有宿主页面路径在此集中声明。以下位置必须从此处 import，禁止散写硬编码：
 *   - ContentArea 的 <Route path=...>
 *   - Sidebar 的 pathToPageKey / usePageRouting 的 path→key 映射
 *
 * 域页面（`src/pages/<域>/**`）的 path / pageKey 由域自己的 `routes.tsx` 声明，
 * 经 `domainPageRegistry` 投影 —— **不登记在本表**（2026-11-10 #36 导航声明权半：
 * 登记在这里等于主壳继续给域页面命名，域整棵迁出时还要回来改宿主）。
 *
 * 新增**宿主**内置页面时：
 *   1. 在 BUILTIN_PAGE_PATH 增加 key→path；
 *   2. 在 ContentArea 增加对应 <Route path={BUILTIN_PAGE_PATH.xxx}> 与懒加载组件；
 *   3. 在 Sidebar builtinNavItems 增加导航项（path 引用本表）。
 */

/** 应用冷启动后的默认首页路径（"/" 重定向目标）。
 *  仪表盘已合并到对话页的「工作台」Tab，默认进入对话页。 */
export const DEFAULT_HOME = "/chat";

/**
 * key→path 映射，覆盖所有内置页面。
 *
 * 路径按 8 个标准能力域组织：
 *   - 通用功能路径保持顶级（/chat, /terminal, /files, /gateway 等）
 *   - 业务域路径由 CAPABILITY_DOMAIN_META 自动展开（/finance, /automation 等）
 */
export const BUILTIN_PAGE_PATH: Record<string, string> = {
  // ── 能力域聚合入口（2026-10-28 起并入 OPC）──
  // `domainMeta.path` 仍是跨端协议值（须与后端 nav_path 一致），
  // 但不再注册为顶级路由；统一由 `/opc/domains/:domainId` 承载。
  // 该路由与 /opc、/opc/:tab、/demand-discovery、14 个能力包页一律由
  // `src/pages/opc/routes.tsx` 声明（#36 迁出期），宿主表不再登记。

  // ── AxInvest 本地业务页（2026-09-01 上游合并时路由被覆盖，09-06 按能力域恢复） ──
  "finance-investment": "/invest",
  investReplay: "/invest/replay",

  // ── 通用功能（general 域） ──
  chat: "/chat",
  /** Bot 归属层管理页（列表 / 创建 / 编辑 / SOUL / 记忆 / 线程） */
  bots: "/bots",
  chatWorkflow: "/chat/workflow",
  chatTerminal: "/chat/terminal",
  chatFiles: "/chat/files",
  chatAgents: "/chat/multiAgent",
  chatMarketplace: "/chat/marketplace",
  chatQuickbar: "/chat/quickbar",
  dashboard: "/chat/dashboard",
  knowledge: "/knowledge",
  knowledgeMemory: "/knowledge/memory",
  knowledgeWiki: "/knowledge/wiki",
  knowledgeLearning: "/knowledge/learning",
  memory: "/memory",
  link: "/link",
  settings: "/settings",
  "tasks-center": "/tasks",
  "dynamic-ui": "/dynamic-ui",

  // ── 运维域（devops）：路径保持顶级（/gateway） ──
  gateway: "/gateway",

  // ── 历史兼容入口 / devtools 等 ──
  llmWiki: "/llm-wiki",
  learningGraph: "/learning-graph",
  devtools: "/devtools",
  devtoolsTraceExplorer: "/devtools/trace-explorer",
  devtoolsBenchmark: "/devtools/benchmark",
  devtoolsToolRecommender: "/devtools/tool-recommender",
  devtoolsFineTune: "/devtools/fine-tune",
  devtoolsRlTraining: "/devtools/rl-training",
};
