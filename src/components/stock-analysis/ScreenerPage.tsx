import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { Collapse, Grid } from "antd";
import { useTranslation } from "react-i18next";
import { PageHeader } from "./_shared/PageHeader";
import { DragonTigerPanel } from "./DragonTigerPanel";
import { HotStocksPanel } from "./HotStocksPanel";
import { LimitUpPanel } from "./LimitUpPanel";
import { StockScreenerPanel } from "./StockScreenerPanel";

const { useBreakpoint } = Grid;

/**
 * ScreenerPage — 选股中心
 *
 * 顶部双块:今日荐股 (discover) + 我的筛选 (screen) — 横向并列,等高
 * 底部折叠:HotStocks / LimitUp / DragonTiger — 默认收起的折叠面板
 * 响应式:desktop 2 列 / tablet 2 列紧凑 / mobile 单列
 */
export function ScreenerPage() {
  const { t } = useTranslation();
  const screens = useBreakpoint();
  const isMobile = !screens.md;

  return (
    <PageErrorBoundary title="Screener">
      <div className="flex h-full flex-col">
        <PageHeader titleKey="screener.title" backTo="/stock-analysis" />
        <div className={["flex-1 overflow-auto space-y-4", isMobile ? "p-2" : "p-4"].join(" ")}>
          {/* 顶部:双块荐股面板 */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <StockScreenerPanel mode="discover" />
            <StockScreenerPanel mode="screen" />
          </div>

          {/* 底部:三个市场氛围面板(手风琴,默认全部收起) */}
          <Collapse
            accordion
            defaultActiveKey={[]}
            bordered={false}
            size={isMobile ? "small" : "middle"}
            items={[
              {
                key: "hot",
                label: (
                  <span className="text-sm font-medium">
                    🔥 {t("stockAnalysis.settings.panels.hotStocks")}
                  </span>
                ),
                children: <HotStocksPanel bordered={false} />,
              },
              {
                key: "limitup",
                label: (
                  <span className="text-sm font-medium">
                    🏆 {t("stockAnalysis.settings.panels.limitUp")}
                  </span>
                ),
                children: <LimitUpPanel bordered={false} />,
              },
              {
                key: "dragontiger",
                label: (
                  <span className="text-sm font-medium">
                    🐉 {t("stockAnalysis.settings.panels.dragonTiger")}
                  </span>
                ),
                children: <DragonTigerPanel bordered={false} />,
              },
            ]}
          />
        </div>
      </div>
    </PageErrorBoundary>
  );
}
