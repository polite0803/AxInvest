import { PageErrorBoundary } from "@/components/shared/ErrorBoundary";
import { ArrowLeftRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

/**
 * TradePage — 交易与回放
 * 完整内容将在 Phase 4 填充（TradePanel + ExecutionReplayPanel）
 */
export function TradePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  return (
    <PageErrorBoundary title="Trade">
      <div className="flex h-full flex-col">
        <header className="sa-header">
          <button type="button" className="sa-header-back" onClick={() => navigate("/")}>
            ‹ {t("nav.chat")}
          </button>
          <h2 className="sa-header-title">{t("trade.title")}</h2>
          <span className="sa-header-meta">{t("common.comingSoon")}</span>
        </header>
        <div className="flex-1 flex items-center justify-center p-8">
          <div className="text-center max-w-md space-y-4">
            <ArrowLeftRight size={48} className="mx-auto opacity-30" />
            <h3 className="text-lg font-medium">{t("trade.title")}</h3>
            <p className="text-sm opacity-60">
              {t("trade.placeholder")}
            </p>
          </div>
        </div>
      </div>
    </PageErrorBoundary>
  );
}
