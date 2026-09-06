// SPDX-License-Identifier: AGPL-3.0-only
// ! 内置侧栏导航项的唯一权威来源
//
// 所有内置导航项集中在此声明，Sidebar 与 DomainHub（域聚合页）共同复用，
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
  AppstoreFilled,
  BgColorsOutlined,
  BookFilled,
  CalculatorFilled,
  CarryOutFilled,
  CodeFilled,
  DollarCircleFilled,
  EnvironmentFilled,
  FundFilled,
  MessageFilled,
  PlaySquareFilled,
  RiseOutlined,
  RobotFilled,
  SafetyCertificateFilled,
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
  {
    key: "chat",
    icon: navIcon(MessageFilled),
    labelKey: "nav.chat",
    path: BUILTIN_PAGE_PATH.chat,
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
  // ── 金融域（finance）：金融投研行业包 ──
  {
    key: "finance-analysis",
    icon: navIcon(FundFilled),
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
];

/** 按标准域过滤内置导航项 */
export function navItemsByDomain(domain: CapabilityDomain): NavItem[] {
  return builtinNavItems.filter((n) => domainForNavKey(n.key) === domain);
}
