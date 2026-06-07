import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { Filter } from "lucide-react";

/**
 * ScreenerPage — 选股中心
 * 完整内容将在 Phase 3 填充（StockScreener + HotStocks + LimitUp + DragonTiger + ConceptBlocks）
 */
export function ScreenerPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <PageErrorBoundary title="Screener">
      <div className="flex h-full flex-col">
        <header className="sa-header">
          <button type="button" className="sa-header-back" onClick={() => navigate("/")}>
            ‹ {t("nav.chat")}
          </button>
          <h2 className="sa-header-title">{t("screener.title")}</h2>
          <span className="sa-header-meta">{t("common.comingSoon")}</span>
        </header>
        <div className="flex-1 flex items-center justify-center p-8">
          <div className="text-center max-w-md space-y-4">
            <Filter size={48} className="mx-auto opacity-30" />
            <h3 className="text-lg font-medium">{t("screener.title")}</h3>
            <p className="text-sm opacity-60">
              {t("screener.placeholder")}
            </p>
          </div>
        </div>
      </div>
    </PageErrorBoundary>
  );
}
