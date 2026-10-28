// SPDX-License-Identifier: AGPL-3.0-only

import { AppHeader } from "@/components/layout/AppHeader";
import { IpcReconnectBanner } from "@/components/layout/IpcReconnectBanner";
import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { PageContextProvider } from "@/components/shared/PageContextProvider";
import { useIpcHealth } from "@/hooks/useIpcHealth";
import { DEVTOOLS_SUB_PARAM, DEVTOOLS_SUB_PATHS, DEVTOOLS_SUBS, type DevToolsSub } from "@/lib/devtoolsSubTabs";
import { CAPABILITY_DOMAIN_META } from "@/lib/domainMeta";
import { BUILTIN_PAGE_PATH, DEFAULT_HOME } from "@/lib/pageRegistry";
import { workspaceTabPath } from "@/lib/workspaceTabs";

import { Button, Result, Spin } from "antd";
import { lazy, memo, Suspense } from "react";
import { useTranslation } from "react-i18next";
import { Navigate, Route, Routes, useLocation, useNavigate } from "react-router-dom";

// ── 页面 lazy 导入 ──
const LazyWorkspaceHub = lazy(() =>
  import("@/components/layout/WorkspaceHub").then((m) => ({ default: m.WorkspaceHub }))
);
const LazyMemoryPage = lazy(() => import("@/pages/MemoryPage").then((m) => ({ default: m.MemoryPage })));
const LazyKnowledgeHubPage = lazy(() =>
  import("@/pages/KnowledgeHubPage").then((m) => ({ default: m.KnowledgeHubPage }))
);
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
const LazyReplayWorkbenchPage = lazy(() =>
  import("@/pages/ReplayWorkbenchPage").then((m) => ({ default: m.ReplayWorkbenchPage }))
);
const LazyTasksCenterPage = lazy(() =>
  import("@/pages/TasksCenterPage/TasksCenterPage").then((m) => ({ default: m.TasksCenterPage }))
);
const LazyOpcPage = lazy(() => import("@/pages/OpcPage").then((m) => ({ default: m.OpcPage })));
// ── 能力包页（OPC 能力包，2026-09-06 按能力域恢复接线；2026-09-15 「行业」→「域」概念统一） ──
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

/** 能力包页路由表：[BuiltinPageKey（即能力域导航 key）, 页面组件] */
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

/** 旧路由重定向到 /chat，直接产出 URL 查询参数 `/chat?ws=<tab>`。
 *
 * 刻意**不走** `location.state` 机制：WorkspaceHub 的 state→URL 归一化只认 `tab`，
 * 且整页 goto（刷新 / 直链 / E2E 导航）时首次 navigate 的 history.state 未必能可靠
 * 落到 router 的 location.state，导致引擎回落到 store 持久化的上一个 Tab
 * （实测 14 个 /knowledge Chromium E2E 全量超时，App 本身可渲染）。直接写出
 * `?ws=` 后，WorkspaceHub 经 parseWorkspaceTab 首帧即命中，可深链、可刷新恢复。 */
/** 2026-10-28：`redirectToChat` 已删除（`/knowledge` 改为独立域，其余旧链均已移除）。 */

/** 开发工具旧子路由 → `/chat?ws=devtools&sub=<子页>`。
 *
 * 刻意**不走** redirectToChat 的 `location.state` 机制：WorkspaceHub 的 state→URL 归一化
 * 只认 `tab`，`sub` 会在那一刻丢失（这正是此前 6 条子路由全部落到首个 Tab 的原因）。
 * 直接产出 URL，子页选择即可深链、可刷新恢复。`sub` 缺省时只写 `ws=devtools`，
 * 由 DevToolsPage 回落到默认子页。 */
function redirectToDevTools(sub?: DevToolsSub) {
  const params = new URLSearchParams();
  if (sub) {
    params.set(DEVTOOLS_SUB_PARAM, sub);
  }
  return <Navigate to={{ pathname: workspaceTabPath("devtools"), search: params.toString() }} replace />;
}

/** 2026-10-28：`/workflow/new` 旧链已删除，`RedirectToChatWithParams` 随之移除。 */

/** 2026-10-28：股票旧业务路由（`/stock-analysis`、`/trade`、`/watchlist` 等）已删除，
 * 组件内的 `backTo` / `navigate` 已统一改为 `/invest`。 */

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

          {/* ── 能力域聚合页：2026-10-28 并入 OPC，统一由 /opc/domains/:domainId 承载 ── */}
          <Route
            path="/opc/domains/:domainId"
            element={
              <PageContextProvider page="automation-operations">
                <SafeLazyPage Page={LazyDomainHubPage} />
              </PageContextProvider>
            }
          />
          {/* 旧顶级域路径 → 新路径（保留一个版本） */}
          {CAPABILITY_DOMAIN_META.map((domain) => (
            <Route
              key={domain.id}
              path={domain.path}
              element={<Navigate to={`/opc/domains/${domain.id}`} replace />}
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
          {/* 工作台 Tab 的路径段（2026-10-28 起 URL 契约：/chat/<tab>） */}
          <Route
            path={`${BUILTIN_PAGE_PATH.chat}/:tab`}
            element={
              <PageContextProvider page="chat">
                <SafeLazyPage Page={LazyWorkspaceHub} />
              </PageContextProvider>
            }
          />
          {/* 独立于工作台 Tab 的 chat 子页：静态段优先于上面的 :tab */}
          <Route
            path={`${BUILTIN_PAGE_PATH.chat}/quickbar`}
            element={
              <PageContextProvider page="chat">
                <SafeLazyPage Page={LazyQuickBarPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH.chat}/marketplace`}
            element={<Navigate to={workspaceTabPath("workflow")} replace />}
          />
          {
            /* 2026-10-28：chat 类旧路由（/dashboard、/workflow、/terminal、/files、
              /multi-agent、/marketplace）已删除，统一走 /chat/<tab>。 */
          }
          {/* Knowledge 域：2026-10-28 起为独立域，不再重定向到 chat */}
          <Route
            path={BUILTIN_PAGE_PATH.knowledge}
            element={
              <PageContextProvider page="knowledge">
                <SafeLazyPage Page={LazyKnowledgeHubPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH.knowledge}/memory`}
            element={
              <PageContextProvider page="knowledge">
                <SafeLazyPage Page={LazyMemoryPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH.knowledge}/wiki`}
            element={
              <PageContextProvider page="knowledge">
                <SafeLazyPage Page={LazyKnowledgeHubPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={`${BUILTIN_PAGE_PATH.knowledge}/learning`}
            element={
              <PageContextProvider page="knowledge">
                <SafeLazyPage Page={LazyLearningGraphPage} />
              </PageContextProvider>
            }
          />
          <Route
            path={BUILTIN_PAGE_PATH["finance-investment"]}
            element={
              <PageContextProvider page="finance-investment">
                <SafeLazyPage Page={LazyInvestPage} />
              </PageContextProvider>
            }
          />
          {/* 回放工作台：2026-10-28 起归入投资域，旧 /replay-workbench 302 */}
          <Route
            path={`${BUILTIN_PAGE_PATH["finance-investment"]}/replay`}
            element={
              <PageContextProvider page="finance-investment">
                <SafeLazyPage Page={LazyReplayWorkbenchPage} />
              </PageContextProvider>
            }
          />
          {/* 2026-10-28：/replay-workbench 旧链已删除，统一走 /invest/replay。 */}
          <Route
            path={BUILTIN_PAGE_PATH["tasks-center"]}
            element={
              <PageContextProvider page="tasks-center">
                <SafeLazyPage Page={LazyTasksCenterPage} />
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
          {/* ── AxInvest 能力包页（OPC 能力包，路径 /opc/domain/:id，按能力域归位） ── */}
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
          <Route path={BUILTIN_PAGE_PATH.memory} element={<Navigate to="/knowledge/memory" replace />} />
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
          <Route path={BUILTIN_PAGE_PATH.llmWiki} element={<Navigate to="/knowledge/wiki" replace />} />
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
          {/* 2026-10-28：/quickbar 旧链已删除，统一走 /chat/quickbar。 */}
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
          <Route path={BUILTIN_PAGE_PATH.learningGraph} element={<Navigate to="/knowledge/learning" replace />} />

          {/* 旧股票业务路由 → 投资中心（历史记录/复盘/行情表格/时间旅行等调用方仍指向旧地址） */}
          {/* 2026-10-28：股票旧路由已全部删除，统一走 /invest（query 参数由 InvestPage 消费）。 */}

          <Route path="*" element={<NotFoundRoute />} />
        </Routes>
      </div>
    </div>
  );
});
