// SPDX-License-Identifier: AGPL-3.0-only
// ! 侧栏导航项的宿主侧权威来源（宿主项 + 域贡献投影）
//
// **宿主内置项**在此声明；**域页面项**由 `src/pages/<域>/routes.tsx` 的 `nav` 字段声明，
// 经 `domainPageRegistry` 投影后由 `getNavItems()` / `navItemsByDomain()` 合并。
// Sidebar、AppHeader、DomainHubPage 一律经本文件取「全量导航项」，禁止在别处另拼数组。
// 宿主项按「能力域」组织：通过 NAV_ITEM_DOMAIN_MAP（见 domainMeta）归入唯一标准域；
// 域贡献项自带 `domain` 字段，不经该表。
//
// 图标一律用 @ant-design/icons 本地组件（确定性渲染），
// 禁止用 Iconify 运行时图标——名字无效或网络不通时渲染空白且无报错（09-06 事故）。

import { domainForNavKey } from "@/lib/domainMeta";
import { type ContributedNavEntry, getContributedNavItems } from "@/lib/domainPageRegistry";
import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import type { CapabilityDomain } from "@/types/capability";
import {
  ApartmentOutlined,
  ApiOutlined,
  BookOutlined,
  CodeOutlined,
  DatabaseOutlined,
  DollarCircleFilled,
  FileTextOutlined,
  FolderOpenOutlined,
  HistoryOutlined,
  MessageFilled,
  NodeIndexOutlined,
  ScheduleOutlined,
  ShopOutlined,
  TeamOutlined,
} from "@ant-design/icons";

const navIcon = (Icon: React.ComponentType<{ style?: React.CSSProperties }>) => <Icon style={{ fontSize: 17 }} />;

export interface NavItem {
  key: string;
  icon: React.ReactNode;
  labelKey: string;
  path: string;
  isPlugin: boolean;
  pluginName?: string;
}

/** 宿主内置导航项（域页面项由域模块贡献，见 `getNavItems`） */
export const builtinNavItems: NavItem[] = [
  // ── 通用域（general） ──
  // 侧栏首项语义 = 「回到工作台」：navigate("/chat") 不带 ?ws=，由 WorkspaceHub 解析为
  // 「恢复上次的功能 Tab」（持久化值）。这是业务页（/invest、/opc/*）回到工作台的唯一侧栏入口，
  // 因此文案用「工作台」而非「对话」—— 否则点「对话」却停在终端会让人以为导航坏了。
  // 需要**精确切到对话 Tab** 时用：命令面板 / Ctrl+1 / 显式 /chat?ws=chat。
  {
    key: "chat",
    icon: navIcon(MessageFilled),
    labelKey: "nav.workbench",
    path: BUILTIN_PAGE_PATH.chat,
    isPlugin: false,
  },
  // ── 通用域（general）：系统级任务中心（定时任务 / 手动作业统一入口） ──
  {
    key: "tasks-center",
    icon: navIcon(ScheduleOutlined),
    labelKey: "nav.tasksCenter",
    path: BUILTIN_PAGE_PATH["tasks-center"],
    isPlugin: false,
  },
  // ── 金融域（finance）：AxInvest 投资总览 ──
  {
    key: "finance-investment",
    icon: navIcon(DollarCircleFilled),
    labelKey: "nav.financeInvestment",
    path: BUILTIN_PAGE_PATH["finance-investment"],
    isPlugin: false,
  },
  // ── 金融域（finance）：OPC 投顾经营域包 / 财务会计行业包，自动化、运维、数据分析、
  // 内容创作、AI 媒体域的全部行业包，以及 OPC 经营面板与需求发现页 ——
  // 一律由 `src/pages/opc/routes.tsx` 的 `NAV_PAGE_DECLARATIONS` 贡献（#36 导航声明权半），
  // 宿主不再持有它们的 key / labelKey / icon / path。
  // ── 2026-10-28：补齐“有路由、无入口”的页面（阶段 1 可达性） ──
  // 这批页面此前只能靠 chat 内 tab、页面内跳转或深链到达。
  // 阶段 1 先补入口、沿用旧路径；阶段 2 再随 Shell 重构一次性切到 /chat/*、/knowledge/*。
  // 对应 `PLAN-frontend-information-architecture.md` §4.2。
  {
    key: "chat-workflow",
    icon: navIcon(ApartmentOutlined),
    labelKey: "nav.workflow",
    path: BUILTIN_PAGE_PATH.chatWorkflow,
    isPlugin: false,
  },
  {
    key: "chat-terminal",
    icon: navIcon(CodeOutlined),
    labelKey: "nav.terminal",
    path: BUILTIN_PAGE_PATH.chatTerminal,
    isPlugin: false,
  },
  {
    key: "chat-files",
    icon: navIcon(FolderOpenOutlined),
    labelKey: "nav.files",
    path: BUILTIN_PAGE_PATH.chatFiles,
    isPlugin: false,
  },
  {
    key: "chat-agents",
    icon: navIcon(TeamOutlined),
    labelKey: "nav.multiAgent",
    path: BUILTIN_PAGE_PATH.chatAgents,
    isPlugin: false,
  },
  {
    key: "chat-marketplace",
    icon: navIcon(ShopOutlined),
    labelKey: "nav.marketplace",
    path: BUILTIN_PAGE_PATH.chatMarketplace,
    isPlugin: false,
  },
  {
    key: "knowledge",
    icon: navIcon(BookOutlined),
    labelKey: "nav.knowledge",
    path: BUILTIN_PAGE_PATH.knowledge,
    isPlugin: false,
  },
  {
    key: "knowledge-memory",
    icon: navIcon(DatabaseOutlined),
    labelKey: "nav.memory",
    path: BUILTIN_PAGE_PATH.knowledgeMemory,
    isPlugin: false,
  },
  {
    key: "knowledge-wiki",
    icon: navIcon(FileTextOutlined),
    labelKey: "nav.wiki",
    path: BUILTIN_PAGE_PATH.knowledgeWiki,
    isPlugin: false,
  },
  {
    key: "knowledge-learning",
    icon: navIcon(NodeIndexOutlined),
    labelKey: "nav.learningGraph",
    path: BUILTIN_PAGE_PATH.knowledgeLearning,
    isPlugin: false,
  },
  {
    key: "invest-replay",
    icon: navIcon(HistoryOutlined),
    labelKey: "nav.replayWorkbench",
    path: BUILTIN_PAGE_PATH.investReplay,
    isPlugin: false,
  },
  // 网关配置页：此前 `/gateway` 与 `/link` 无任何侧栏入口。
  // 阶段 2 会并入设置页“连接”分组，这里先给一个可达入口。
  {
    key: "gateway",
    icon: navIcon(ApiOutlined),
    labelKey: "nav.gateway",
    path: BUILTIN_PAGE_PATH.gateway,
    isPlugin: false,
  },
];

/** 域贡献项 → 宿主渲染用的导航项（`path` 由所属路由派生）。 */
export function navItemFromContribution(
  entry: ContributedNavEntry,
): NavItem {
  return { key: entry.key, icon: entry.icon, labelKey: entry.labelKey, path: entry.path, isPlugin: false };
}

/** 全量导航项 = 宿主内置 + 域贡献（侧栏 / 页面标题共用）。 */
export function getNavItems(): NavItem[] {
  return [...builtinNavItems, ...getContributedNavItems().map(navItemFromContribution)];
}

/** 按标准域过滤导航项（宿主项查归域表，域贡献项自带 domain）。 */
export function navItemsByDomain(domain: CapabilityDomain): NavItem[] {
  return [
    ...builtinNavItems.filter((n) => domainForNavKey(n.key) === domain),
    ...getContributedNavItems().filter((n) => n.domain === domain).map(navItemFromContribution),
  ];
}
