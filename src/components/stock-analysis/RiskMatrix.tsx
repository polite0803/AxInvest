import { useStockAnalysisStore } from "@/stores";
import { Card, Tag } from "antd";
import { useTranslation } from "react-i18next";

const RISK_LABELS: Record<string, string> = {
  aggressive: "risk.aggressive",
  conservative: "risk.conservative",
  neutral: "risk.neutral",
};

const RISK_COLORS: Record<string, string> = {
  aggressive: "red",
  conservative: "green",
  neutral: "blue",
};

export function RiskMatrix() {
  const { t } = useTranslation();
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);

  if (Object.keys(riskAssessments).length === 0) return null;

  return (
    <Card size="small" title={t("stockAnalysis.riskAssessment")}>
      <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        {Object.entries(riskAssessments).map(([type, report]) => (
          <div
            key={type}
            className="p-2 rounded"
            style={{ background: "var(--color-bg-elevated)" }}
          >
            <Tag color={RISK_COLORS[type]}>
              {t(`stockAnalysis.${RISK_LABELS[type]}`)}
            </Tag>
            <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap" }}>{report}</p>
          </div>
        ))}
      </div>
    </Card>
  );
}
