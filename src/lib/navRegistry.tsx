// SPDX-License-Identifier: AGPL-3.0-only
// ! 内置侧栏导航项的唯一权威来源
//
// 所有内置导航项集中在此声明，Sidebar 与 DomainHubPage（域聚合页）共同复用，
// 禁止在别处重复定义导航项数组。
// 导航以「能力域」为组织轴：每个导航项通过 NAV_ITEM_DOMAIN_MAP（见 domainMeta）
// 归入唯一标准域。
//
// 图标一律用 @ant-design/icons 本地组件（确定性渲染），
// 禁止用 Iconify 运行时图标——名字无效或网络不通时渲染空白且无报错（09-06 事故）。

import { domainForNavKey } from "@/lib/domainMeta";
import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import type { CapabilityDomain } from "@/types/capability";
import {
  AimOutlined,
  ApartmentOutlined,
  ApiOutlined,
  AppstoreFilled,
  BgColorsOutlined,
  BookFilled,
  BookOutlined,
  CalculatorFilled,
  CarryOutFilled,
  CodeFilled,
  CodeOutlined,
  DatabaseOutlined,
  DollarCircleFilled,
  EnvironmentFilled,
  FileTextOutlined,
  FolderOpenOutlined,
  HistoryOutlined,
  MessageFilled,
  NodeIndexOutlined,
  PlaySquareFilled,
  RiseOutlined,
  RobotFilled,
  SafetyCertificateFilled,
  ScheduleOutlined,
  ShopOutlined,
  ShoppingFilled,
  TeamOutlined,
  VideoCameraFilled,
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

/** 内置导航项 */
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
  // ── 金融域（finance）：OPC 投顾经营域包 ──
  {
    key: "finance-analysis",
    icon: navIcon(TeamOutlined),
    labelKey: "nav.financeAnalysis",
    path: BUILTIN_PAGE_PATH["finance-analysis"],
    isPlugin: false,
  },
  // ── 金融域（finance）：财务会计行业包 ──
  {
    key: "finance-accounting",
    icon: navIcon(CalculatorFilled),
    labelKey: "nav.financeAccounting",
    path: BUILTIN_PAGE_PATH["finance-accounting"],
    isPlugin: false,
  },
  // ── 自动化域（automation）：OPC 需求发现 ──
  {
    key: "demand-discovery",
    icon: navIcon(AimOutlined),
    labelKey: "opc.demand.pageTitle",
    path: BUILTIN_PAGE_PATH["demand-discovery"],
    isPlugin: false,
  },
  // ── 自动化域（automation）：OPC 一人公司管理面板 ──
  {
    key: "automation-operations",
    icon: navIcon(AppstoreFilled),
    labelKey: "nav.automationOperations",
    path: BUILTIN_PAGE_PATH["automation-operations"],
    isPlugin: false,
  },
  // ── 自动化域（automation）：销售增长行业包 ──
  {
    key: "automation-sales",
    icon: navIcon(RiseOutlined),
    labelKey: "nav.automationSales",
    path: BUILTIN_PAGE_PATH["automation-sales"],
    isPlugin: false,
  },
  // ── 自动化域（automation）：项目管理行业包 ──
  {
    key: "automation-projects",
    icon: navIcon(CarryOutFilled),
    labelKey: "nav.automationProjects",
    path: BUILTIN_PAGE_PATH["automation-projects"],
    isPlugin: false,
  },
  // ── 自动化域（automation）：行业咨询包 ──
  {
    key: "automation-consulting",
    icon: navIcon(TeamOutlined),
    labelKey: "nav.automationConsulting",
    path: BUILTIN_PAGE_PATH["automation-consulting"],
    isPlugin: false,
  },
  // ── 自动化域（automation）：电商运营行业包 ──
  {
    key: "automation-ecommerce",
    icon: navIcon(ShoppingFilled),
    labelKey: "nav.automationEcommerce",
    path: BUILTIN_PAGE_PATH["automation-ecommerce"],
    isPlugin: false,
  },
  // ── 运维域（devops）：软件开发行业包 ──
  {
    key: "devops-software",
    icon: navIcon(CodeFilled),
    labelKey: "nav.devopsSoftware",
    path: BUILTIN_PAGE_PATH["devops-software"],
    isPlugin: false,
  },
  // ── 运维域（devops）：安全运维行业包 ──
  {
    key: "devops-security",
    icon: navIcon(SafetyCertificateFilled),
    labelKey: "nav.devopsSecurity",
    path: BUILTIN_PAGE_PATH["devops-security"],
    isPlugin: false,
  },
  // ── 数据分析域（data_analysis）：地理空间行业包 ──
  {
    key: "data-geospatial",
    icon: navIcon(EnvironmentFilled),
    labelKey: "nav.dataGeospatial",
    path: BUILTIN_PAGE_PATH["data-geospatial"],
    isPlugin: false,
  },
  // ── 数据分析域（data_analysis）：AI 研究行业包 ──
  {
    key: "data-ai-research",
    icon: navIcon(RobotFilled),
    labelKey: "nav.dataAiResearch",
    path: BUILTIN_PAGE_PATH["data-ai-research"],
    isPlugin: false,
  },
  // ── 内容创作域（content_creation）：内容媒体行业包 ──
  {
    key: "content-media",
    icon: navIcon(VideoCameraFilled),
    labelKey: "nav.contentMedia",
    path: BUILTIN_PAGE_PATH["content-media"],
    isPlugin: false,
  },
  // ── 内容创作域（content_creation）：视觉设计行业包 ──
  {
    key: "content-design",
    icon: navIcon(BgColorsOutlined),
    labelKey: "nav.contentDesign",
    path: BUILTIN_PAGE_PATH["content-design"],
    isPlugin: false,
  },
  // ── 内容创作域（content_creation）：教育内容行业包 ──
  {
    key: "content-education",
    icon: navIcon(BookFilled),
    labelKey: "nav.contentEducation",
    path: BUILTIN_PAGE_PATH["content-education"],
    isPlugin: false,
  },
  // ── AI 媒体域（ai_media）：游戏开发行业包 ──
  {
    key: "ai-media-game",
    icon: navIcon(PlaySquareFilled),
    labelKey: "nav.aiMediaGame",
    path: BUILTIN_PAGE_PATH["ai-media-game"],
    isPlugin: false,
  },
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

/** 按标准域过滤内置导航项 */
export function navItemsByDomain(domain: CapabilityDomain): NavItem[] {
  return builtinNavItems.filter((n) => domainForNavKey(n.key) === domain);
}
