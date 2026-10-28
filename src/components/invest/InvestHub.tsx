// SPDX-License-Identifier: AGPL-3.0-only

import { PageTimeAnchor } from "@/components/time-travel/PageTimeAnchor";
import { Tabs, type TabsProps, Typography } from "antd";
import { LineChart } from "lucide-react";
import { lazy, Suspense, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSearchParams } from "react-router-dom";

import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";

const { Title } = Typography;

// 各业务子页面懒加载（复用现有页面入口）
const LazyScreener = lazy(() => import("@/pages/ScreenerPage").then((m) => ({ default: m.ScreenerPage })));
const LazyStockWorkspace = lazy(() =>
  import("@/pages/StockWorkspacePage").then((m) => ({ default: m.StockWorkspacePage }))
);
const LazyScreenshotDiagnosis = lazy(() =>
  import("@/pages/ScreenshotDiagnosisPage").then((m) => ({ default: m.ScreenshotDiagnosisPage }))
);
const LazyPaperPortfolio = lazy(() =>
  import("@/pages/PaperPortfolioPage").then((m) => ({ default: m.PaperPortfolioPage }))
);
const LazyQuantLab = lazy(() => import("@/pages/QuantLabPage").then((m) => ({ default: m.QuantLabPage })));
const LazyCrossMarket = lazy(() =>
  import("@/components/stock-analysis/CrossMarketDashboard").then((m) => ({ default: m.CrossMarketDashboard }))
);

/** 投资业务 tab key — 按操作逻辑排序：发现 → 单股深度 → 参照/导入 → 持仓跟踪 → 策略验证 */
export type InvestTabKey =
  | "screener"
  | "workspace"
  | "cross-market"
  | "screenshot-diagnosis"
  | "paper-portfolio"
  | "quant";

/** tab 默认值 */
const DEFAULT_TAB: InvestTabKey = "screener";

/** 合法 tab key 集合（用于校验 URL 参数；已收口的 pipeline 等旧值会回退默认 tab） */
const VALID_TABS: Set<InvestTabKey> = new Set([
  "screener",
  "workspace",
  "cross-market",
  "screenshot-diagnosis",
  "paper-portfolio",
  "quant",
]);

function TabLoader() {
  const { t } = useTranslation();
  return (
    <div className="flex items-center justify-center h-full w-full" style={{ minHeight: 200 }}>
      {t("common.loading")}
    </div>
  );
}

function SafeTab({ children }: { children: React.ReactNode }) {
  const { t } = useTranslation();
  return (
    <PageErrorBoundary title={t("error.page")}>
      <Suspense fallback={<TabLoader />}>{children}</Suspense>
    </PageErrorBoundary>
  );
}

/**
 * InvestHub — 投资业务统一入口。
 *
 * 6 个股票业务页面集成到一个页面内的 Tab 中，按业务操作逻辑排序：
 *   1. 选股（发现候选） → 2. 工作区（单股深度）
 *   → 3. 跨市场行情（外部市场参照） → 4. 截图诊断（外部导入） → 5. 模拟观察（持仓跟踪）
 *   → 6. 量化（策略验证）
 *
 * 已收口到系统级「任务中心」（/tasks）的能力不在本页保留入口：
 *   - 投资管道的执行/历史、时间旅行批次复盘、回放扫描 → 任务中心 → 手动作业；
 *   - 旧 URL `?tab=pipeline` 等非法/已废弃值统一回退到默认 tab（screener）。
 *
 * 世界观（live / as-of）是贯穿所有 tab 的顶层维度，不是业务 tab：
 *   - 标题栏右侧 PageTimeAnchor 是投资中心唯一的「实时分析 / 历史回放」切换，
 *     各业务 tab 不再各自渲染锚点（timeAnchorStore 是单一事实源，invoke 层
 *     自动向受控命令注入 as_of_date，切 tab 不丢世界观）。
 *
 * URL 参数：
 *   - ?tab=xxx — 当前激活的 tab（刷新保持）
 *   - ?stockCode=xxx — workspace tab 专用，驱动当前股票
 *   - ?view=xxx — workspace tab 专用，驱动当前视图
 */
export function InvestHub() {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();

  // 从 URL 读取当前 tab（非法值回退到默认）
  const currentTab = useMemo<InvestTabKey>(() => {
    const raw = searchParams.get("tab");
    if (raw && VALID_TABS.has(raw as InvestTabKey)) {
      return raw as InvestTabKey;
    }
    return DEFAULT_TAB;
  }, [searchParams]);

  // tab 切换 → 更新 URL（保留 stockCode/view 等其他参数）
  const handleTabChange = (key: string) => {
    const next = new URLSearchParams(searchParams);
    next.set("tab", key);
    // 切换到非 workspace tab 时清理 workspace 专属参数
    if (key !== "workspace") {
      next.delete("stockCode");
      // stockName 必须与 stockCode 同生共死：只留名称会让下次进入工作区
      // 时把上一只股票的名称贴到新股票上
      next.delete("stockName");
      next.delete("view");
    }
    setSearchParams(next, { replace: true });
  };

  const items: TabsProps["items"] = useMemo(
    () => [
      {
        key: "screener",
        label: t("invest.tab.screener"),
        children: (
          <SafeTab>
            <LazyScreener />
          </SafeTab>
        ),
      },
      {
        key: "workspace",
        label: t("invest.tab.workspace"),
        children: (
          <SafeTab>
            <LazyStockWorkspace />
          </SafeTab>
        ),
      },
      {
        // 标签复用 crossMarket.title（11 语言已收录），不新增 i18n 键
        key: "cross-market",
        label: t("crossMarket.title"),
        children: (
          <SafeTab>
            <LazyCrossMarket />
          </SafeTab>
        ),
      },
      {
        key: "screenshot-diagnosis",
        label: t("invest.tab.screenshotDiagnosis"),
        children: (
          <SafeTab>
            <LazyScreenshotDiagnosis />
          </SafeTab>
        ),
      },
      {
        key: "paper-portfolio",
        label: t("invest.tab.paperPortfolio"),
        children: (
          <SafeTab>
            <LazyPaperPortfolio />
          </SafeTab>
        ),
      },
      {
        key: "quant",
        label: t("invest.tab.quant"),
        children: (
          <SafeTab>
            <LazyQuantLab />
          </SafeTab>
        ),
      },
    ],
    [t],
  );

  return (
    <div className="flex flex-col h-full w-full min-h-0">
      {
        /* 顶层模式栏：投资中心标题 + 世界观切换(live/as-of)；
          批次复盘/管道/扫描等手动作业已统一收口到任务中心（/tasks） */
      }
      <div
        className="flex items-center justify-between gap-3 flex-wrap"
        style={{ padding: "12px 16px 0", background: "var(--color-bg-container)" }}
      >
        <Title level={3} style={{ margin: 0 }}>
          <LineChart size={20} style={{ marginRight: 8, verticalAlign: "middle" }} />
          {t("invest.title")}
        </Title>
        <PageTimeAnchor />
      </div>
      <Tabs
        activeKey={currentTab}
        onChange={handleTabChange}
        items={items}
        className="invest-hub-tabs ax-fill-tabs"
        tabBarStyle={{
          margin: 0,
          padding: "0 16px",
          background: "var(--color-bg-container)",
          borderBottom: "1px solid var(--border-light)",
        }}
        tabBarGutter={24}
        size="small"
        destroyOnHidden={false}
      />
    </div>
  );
}
