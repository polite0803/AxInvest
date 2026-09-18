// SPDX-License-Identifier: AGPL-3.0-only

import { AppHeader } from "@/components/layout/AppHeader";
import { IpcReconnectBanner } from "@/components/layout/IpcReconnectBanner";
import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { PageContextProvider } from "@/components/shared/PageContextProvider";
import { useIpcHealth } from "@/hooks/useIpcHealth";
import { DEVTOOLS_SUB_PARAM, DEVTOOLS_SUB_PATHS, DEVTOOLS_SUBS, type DevToolsSub } from "@/lib/devtoolsSubTabs";
import { CAPABILITY_DOMAIN_META } from "@/lib/domainMeta";
import { BUILTIN_PAGE_PATH, DEFAULT_HOME } from "@/lib/pageRegistry";
import { WORKSPACE_TAB_PARAM } from "@/lib/workspaceTabs";

import { Button, Result, Spin } from "antd";
import { lazy, memo, Suspense } from "react";
import { useTranslation } from "react-i18next";
import { Navigate, Route, Routes, useLocation, useNavigate, useParams } from "react-router-dom";

// ── 页面 lazy 导入 ──
const LazyWorkspaceHub = lazy(() =>
  import("@/components/layout/WorkspaceHub").then((m) => ({ default: m.WorkspaceHub }))
);
const LazyMemoryPage = lazy(() => import("@/pages/MemoryPage").then((m) => ({ default: m.MemoryPage })));
const LazyGatewayLinkPage = lazy(() =>
  import("@/pages/GatewayLinkPage").then((m) => ({
    default: m.GatewayLinkPage,
  }))
);
const LazySettingsPage = lazy(() => import("@/pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));
const LazyIngestPage = lazy(() => import("@/pages/IngestPage").then((m) => ({ default: m.IngestPage })));
const LazyWikiGraphPage = lazy(() => import("@/pages/WikiGraphPage").then((m) => ({ default: m.WikiGraphPage })));
const LazyWikiEditPage = lazy(() => import("@/pages/WikiEditPage").then((m) => ({ default: m.WikiEditPage })));
const LazyQuickBarPage = lazy(() => import("@/pages/QuickBarPage").then((m) => ({ default: m.QuickBarPage })));
const LazyLearningGraphPage = lazy(() =>
  import("@/pages/LearningGraphPage").then((m) => ({ default: m.LearningGraphPage }))
);
const LazyDynamicPageViewer = lazy(() =>
  import("@/pages/DynamicPageViewer").then((m) => ({ default: m.DynamicPageViewer }))
);
const LazyDomainHubPage = lazy(() => import("@/pages/DomainHubPage").then((m) => ({ default: m.DomainHubPage })));
const LazyDemandDiscoveryPage = lazy(() =>
  import("@/pages/DemandDiscoveryPage").then((m) => ({ default: m.DemandDiscoveryPage }))
);
const LazyInvestPage = lazy(() => import("@/pages/InvestPage").then((m) => ({ default: m.InvestPage })));
const LazyOpcPage = lazy(() => import("@/pages/OpcPage").then((m) => ({ default: m.OpcPage })));
// ── 域包页（OPC 域包，2026-09-06 按能力域恢复接线；2026-09-15 「行业」→「域」概念统一） ──
const LazyFinanceInvestDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.FinanceInvestPage }))
);
const LazyAccountingDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.AccountingPage }))
);
const LazySalesGrowthDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.SalesGrowthPage }))
);
const LazyProjectManagementDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.ProjectManagementPage }))
);
const LazyConsultingDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.ConsultingPage }))
);
const LazyEcommerceDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.EcommercePage }))
);
const LazySoftwareDevDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.SoftwareDevPage }))
);
const LazySecurityDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.SecurityPage }))
);
const LazyGeospatialDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.GeospatialPage }))
);
const LazyAiResearchDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.AiResearchPage }))
);
const LazyContentMediaDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.ContentMediaPage }))
);
const LazyDesignDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.DesignPage }))
);
const LazyEducationDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.EducationPage }))
);
const LazyGameDevDomainPage = lazy(() =>
  import("@/pages/opc/domains/DomainPages").then((m) => ({ default: m.GameDevPage }))
);

/** 域包页路由表：[BuiltinPageKey（即能力域导航 key）, 页面组件] */
const DOMAIN_ROUTES: ReadonlyArray<[string, React.LazyExoticComponent<React.ComponentType>]> = [
  ["finance-analysis", LazyFinanceInvestDomainPage],
  ["finance-accounting", LazyAccountingDomainPage],
  ["automation-sales", LazySalesGrowthDomainPage],
  ["automation-projects", LazyProjectManagementDomainPage],
  ["automation-consulting", LazyConsultingDomainPage],
  ["automation-ecommerce", LazyEcommerceDomainPage],
  ["devops-software", LazySoftwareDevDomainPage],
  ["devops-security", LazySecurityDomainPage],
  ["data-geospatial", LazyGeospatialDomainPage],
  ["data-ai-research", LazyAiResearchDomainPage],
  ["content-media", LazyContentMediaDomainPage],
  ["content-design", LazyDesignDomainPage],
  ["content-education", LazyEducationDomainPage],
  ["ai-media-game", LazyGameDevDomainPage],
];

/**
 * 旧链兼容：`/opc/industry/:packId` → `/opc/domain/:packId`。
 *
 * 2026-09-15「行业」→「域」概念统一迁移后 pack 路径前缀改为 `/opc/domain/`，
 * 此单段路由兜住历史书签与外部链接。查询串（`?tab=` 等）原样保留，
 * 用 `replace` 避免在浏览历史里留下中间态。
 */
function LegacyIndustryPathRedirect() {
  const location = useLocation();
  return (
    <Navigate
      to={location.pathname.replace("/opc/industry/", "/opc/domain/") + location.search}
      replace
    />
  );
}

function PageLoader() {
  return (
    <div
      className="flex items-center justify-center h-full w-full"
      style={{ minHeight: 200 }}
    >
      <Spin size="large" />
    </div>
  );
}

function SafeLazyPage({ Page }: { Page: React.LazyExoticComponent<React.ComponentType> }) {
  const { t } = useTranslation();
  return (
    <PageErrorBoundary title={t("error.page")}>
      <Suspense fallback={<PageLoader />}>
        <Page />
      </Suspense>
    </PageErrorBoundary>
  );
}

function NotFoundRoute() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <div style={{ padding: 48, textAlign: "center" }}>
      <Result
        status="404"
        title="404"
        subTitle={t("error.pageNotFound")}
        extra={
          <Button type="primary" onClick={() => navigate(DEFAULT_HOME)}>
            {t("common.back")}
          </Button>
        }
      />
    </div>
  );
}

/** 旧路由重定向到 /chat，通过 location.state.tab 传递目标功能 Tab。 */
function redirectToChat(tab: string) {
  return <Navigate to={BUILTIN_PAGE_PATH.chat} replace state={{ tab }} />;
}

/** 开发工具旧子路由 → `/chat?ws=devtools&sub=<子页>`。
 *
 * 刻意**不走** redirectToChat 的 `location.state` 机制：WorkspaceHub 的 state→URL 归一化
 * 只认 `tab`，`sub` 会在那一刻丢失（这正是此前 6 条子路由全部落到首个 Tab 的原因）。
 * 直接产出 URL，子页选择即可深链、可刷新恢复。`sub` 缺省时只写 `ws=devtools`，
 * 由 DevToolsPage 回落到默认子页。 */
function redirectToDevTools(sub?: DevToolsSub) {
  const params = new URLSearchParams();
  params.set(WORKSPACE_TAB_PARAM, "devtools");
  if (sub) {
    params.set(DEVTOOLS_SUB_PARAM, sub);
  }
  return <Navigate to={{ pathname: BUILTIN_PAGE_PATH.chat, search: params.toString() }} replace />;
}

/** 重定向到 /chat 并保留当前 URL 的查询参数（如 template=xxx）。
 * 用于 /workflow/new?template=xxx 等需要透传查询参数的场景。
 * 必须是真正的组件，因为 useLocation 只能在组件渲染时调用，
 * 而路由的 element 属性在路由定义时就被求值。 */
function RedirectToChatWithParams({ tab }: { tab: string }) {
  const location = useLocation();
  const qs = location.search;
  const to = qs ? `${BUILTIN_PAGE_PATH.chat}${qs}` : BUILTIN_PAGE_PATH.chat;
  return <Navigate to={to} replace state={{ tab }} />;
}

/** 旧股票业务路由 → 投资中心 /invest?tab=…。
 * 分析/交易/回测等独立页已内嵌进 StockWorkspaceShell 或 InvestHub Tab，
 * 旧路由已移除，但全项目仍有大量调用方（历史记录按钮 / 复盘面板 / 行情表格 /
 * 时间旅行工作台等）在跳旧地址，不重定向会落入 * 通配路由报 404。
 * 参数映射（query 全量保留）：
 *   - code → stockCode（工作区切换器同步选中该股票）
 *   - from="id"：/stock-analysis/:id 的 :id → analysisId + view=analysis
 *     （与 StockAnalysisPage 既有的工作区入口参数约定一致）
 *   - from="stockCode"：/workspace/:stockCode 的 :stockCode → stockCode */
function RedirectToInvest({ tab, view, from }: { tab: string; view?: string; from?: "id" | "stockCode" }) {
  const location = useLocation();
  const params = useParams<{ id?: string; stockCode?: string }>();
  const qs = new URLSearchParams(location.search);
  const code = (from === "stockCode" ? params.stockCode : undefined) ?? qs.get("code");
  if (code && !qs.get("stockCode")) {
    qs.set("stockCode", code);
  }
  if (from === "id" && params.id) {
    qs.set("view", "analysis");
    qs.set("analysisId", params.id);
  }
  if (view) {
    qs.set("view", view);
  }
  qs.set("tab", tab);
  return <Navigate to={`/invest?${qs.toString()}`} replace />;
}

export const ContentArea = memo(function ContentArea() {
  const { ipcHealthy } = useIpcHealth();

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", minWidth: 0 }}>
      <IpcReconnectBanner healthy={ipcHealthy} />
      <AppHeader />
      <div
        style={{ flex: 1, minHeight: 0, overflow: "hidden", display: "flex", flexDirection: "column", minWidth: 0 }}
      >
        <Routes>
          <Route path="/" element={<Navigate to={DEFAULT_HOME} replace />} />

          {/* ── 能力域聚合入口（8 个业务域） ── */}
          {CAPABILITY_DOMAIN_META.map((domain) => (
            <Route
              key={domain.id}
              path={domain.path}
              element={
                <PageContextProvider page={domain.id}>
                  <SafeLazyPage Page={LazyDomainHubPage} />
                </PageContextProvider>
              }
            />
          ))}

          {/* ── 通用功能 ── */}
          <Route
            path={BUILTIN_PAGE_PATH.chat}
            element={
              <PageContextProvider page="chat">
                <SafeLazyPage Page={LazyWorkspaceHub} />
              </PageContextProvider>
            }
          />
          <Route path={BUILTIN_PAGE_PATH.dashboard} element={redirectToChat("dashboard")} />
          <Route path={BUILTIN_PAGE_PATH.workflow} element={redirectToChat("workflow")} />
          <Route path={`${BUILTIN_PAGE_PATH.workflow}/new`} element={<RedirectToChatWithParams tab="workflow" />} />
          <Route path={BUILTIN_PAGE_PATH.terminal} element={redirectToChat("terminal")} />
          <Route path={BUILTIN_PAGE_PATH.files} element={redirectToChat("files")} />
          <Route path={BUILTIN_PAGE_PATH.knowledge} element={redirectToChat("knowledge")} />
          <Route path={BUILTIN_PAGE_PATH.multiAgent} element={redirectToChat("multiAgent")} />
          <Route path={BUILTIN_PAGE_PATH.marketplace} element={redirectToChat("workflow")} />
          <Route
            path={BUILTIN_PAGE_PATH["finance-investment"]}
            element={
              <PageContextProvider page="finance-investment">
                <SafeLazyPage Page={LazyInvestPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH["automation-operations"]}/:tab`}
            element={
              <PageContextProvider page="automation-operations">
                <SafeLazyPage Page={LazyOpcPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={BUILTIN_PAGE_PATH["automation-operations"]}
            element={
              <PageContextProvider page="automation-operations">
                <SafeLazyPage Page={LazyOpcPage} />
              </PageContextProvider>
            }
          />
          {/* ── AxInvest 域包页（OPC 域包，路径 /opc/domain/:id，按能力域归位） ── */}
          {DOMAIN_ROUTES.map(([pageKey, Page]) => (
            <Route
              key={pageKey}
              path={BUILTIN_PAGE_PATH[pageKey]}
              element={
                <PageContextProvider page={pageKey}>
                  <SafeLazyPage Page={Page} />
                </PageContextProvider>
              }
            />
          ))}
          {/* ── 旧链兼容：/opc/industry/:packId → /opc/domain/:packId ── */}
          <Route path="/opc/industry/:packId" element={<LegacyIndustryPathRedirect />} />
          <Route
            path={BUILTIN_PAGE_PATH["demand-discovery"]}
            element={
              <PageContextProvider page="demand-discovery">
                <SafeLazyPage Page={LazyDemandDiscoveryPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={BUILTIN_PAGE_PATH.memory}
            element={
              <PageContextProvider page="memory">
                <SafeLazyPage Page={LazyMemoryPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={BUILTIN_PAGE_PATH.link}
            element={
              <PageContextProvider page="link">
                <SafeLazyPage Page={LazyGatewayLinkPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={BUILTIN_PAGE_PATH.gateway}
            element={
              <PageContextProvider page="gateway">
                <SafeLazyPage Page={LazyGatewayLinkPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH.settings}/*`}
            element={
              <PageContextProvider page="settings">
                <SafeLazyPage Page={LazySettingsPage} />
              </PageContextProvider>
            }
          />
          <Route path={BUILTIN_PAGE_PATH.llmWiki} element={redirectToChat("knowledge")} />
          <Route
            path={`${BUILTIN_PAGE_PATH.llmWiki}/:wikiId/graph`}
            element={
              <PageContextProvider page="wiki">
                <SafeLazyPage Page={LazyWikiGraphPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH.llmWiki}/:wikiId/ingest`}
            element={
              <PageContextProvider page="wiki">
                <SafeLazyPage Page={LazyIngestPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH.llmWiki}/:wikiId/edit/:noteId`}
            element={
              <PageContextProvider page="wiki">
                <SafeLazyPage Page={LazyWikiEditPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={BUILTIN_PAGE_PATH.quickbar}
            element={
              <PageContextProvider page="quickbar">
                <SafeLazyPage Page={LazyQuickBarPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH["dynamic-ui"]}/:schemaId`}
            element={
              <PageContextProvider page="dynamic-ui">
                <SafeLazyPage Page={LazyDynamicPageViewer} />
              </PageContextProvider>
            }
          />
          <Route
            path={BUILTIN_PAGE_PATH["dynamic-ui"]}
            element={<Navigate to={BUILTIN_PAGE_PATH.settings} replace />}
          />

          {/* 开发工具旧路由 → 重定向到 /chat */}
          {
            /* 开发工具旧路由 → /chat?ws=devtools&sub=<子页>。
              子页信息必须保留（此前全部压平到首个 Tab，见 lib/devtoolsSubTabs.ts）。 */
          }
          <Route path={BUILTIN_PAGE_PATH.devtools} element={redirectToDevTools()} />
          {DEVTOOLS_SUBS.map((sub) => (
            <Route key={sub} path={DEVTOOLS_SUB_PATHS[sub]} element={redirectToDevTools(sub)} />
          ))}

          {/* 学习图 */}
          <Route
            path={BUILTIN_PAGE_PATH.learningGraph}
            element={
              <PageContextProvider page="learning-graph">
                <SafeLazyPage Page={LazyLearningGraphPage} />
              </PageContextProvider>
            }
          />

          {/* 旧股票业务路由 → 投资中心（历史记录/复盘/行情表格/时间旅行等调用方仍指向旧地址） */}
          <Route path="/stock-analysis" element={<RedirectToInvest tab="workspace" />} />
          <Route path="/stock-analysis/:id" element={<RedirectToInvest tab="workspace" from="id" />} />
          <Route path="/trade" element={<RedirectToInvest tab="workspace" view="trade" />} />
          <Route path="/watchlist" element={<RedirectToInvest tab="workspace" view="monitor" />} />
          <Route path="/backtest" element={<RedirectToInvest tab="workspace" view="backtest" />} />
          <Route path="/compare" element={<RedirectToInvest tab="workspace" view="compare" />} />
          <Route path="/screener" element={<RedirectToInvest tab="screener" />} />
          <Route path="/quant" element={<RedirectToInvest tab="quant" />} />
          <Route path="/workspace/:stockCode" element={<RedirectToInvest tab="workspace" from="stockCode" />} />

          <Route path="*" element={<NotFoundRoute />} />
        </Routes>
      </div>
    </div>
  );
});
