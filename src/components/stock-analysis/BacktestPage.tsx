import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { History } from "lucide-react";

/**
 * BacktestPage — 回测验证
 * 完整内容将在 Phase 5 填充（BacktestPanel + HistoricalAnalysisPanel）
 */
export function BacktestPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <PageErrorBoundary title="Backtest">
      <div className="flex h-full flex-col">
        <header className="sa-header">
          <button type="button" className="sa-header-back" onClick={() => navigate("/")}>
            ‹ {t("nav.chat")}
          </button>
          <h2 className="sa-header-title">{t("backtest.title")}</h2>
          <span className="sa-header-meta">{t("common.comingSoon")}</span>
        </header>
        <div className="flex-1 flex items-center justify-center p-8">
          <div className="text-center max-w-md space-y-4">
            <History size={48} className="mx-auto opacity-30" />
            <h3 className="text-lg font-medium">{t("backtest.title")}</h3>
            <p className="text-sm opacity-60">
              {t("backtest.placeholder")}
            </p>
          </div>
        </div>
      </div>
    </PageErrorBoundary>
  );
}
