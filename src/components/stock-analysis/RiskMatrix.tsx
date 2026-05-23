import { useStockAnalysisStore } from "@/stores";
import { Card, Tag } from "antd";
import { useTranslation } from "react-i18next";

/** 风险类型 → 颜色映射（匹配后端 risk_type 字段） */
const RISK_COLORS: Record<string, string> = {
  "aggressive-debator": "#f85149",
  "conservative-debator": "#3fb950",
  "neutral-debator": "#58a6ff",
  "research-manager": "#d29922",
  "comprehensive": "#a371f7",
};

/** 风险类型 → i18n key */
const RISK_LABEL_KEYS: Record<string, string> = {
  "aggressive-debator": "risk.aggressive",
  "conservative-debator": "risk.conservative",
  "neutral-debator": "risk.neutral",
  "research-manager": "risk.researchManager",
  "comprehensive": "risk.comprehensive",
};

export function RiskMatrix() {
  const { t } = useTranslation();
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);

  if (Object.keys(riskAssessments).length === 0) { return null; }

  return (
    <Card size="small" title={t("stockAnalysis.riskAssessment")} styles={{ body: { padding: "8px 12px" } }}>
      <div className="flex flex-col gap-2">
        {Object.entries(riskAssessments).map(([type, report]) => {
          const color = RISK_COLORS[type]
            || `hsl(${type.split("").reduce((a, c) => a + c.charCodeAt(0), 0) % 360}, 50%, 45%)`;
          const label = RISK_LABEL_KEYS[type]
            ? t(`stockAnalysis.${RISK_LABEL_KEYS[type]}`)
            : type;
          return (
            <div key={type} className="p-2 rounded" style={{ background: "var(--color-bg-elevated)" }}>
              <div className="text-sm font-medium mb-1">
                <Tag color={color}>{label}</Tag>
              </div>
              <p className="text-xs leading-relaxed" style={{ whiteSpace: "pre-wrap" }}>
                {report}
              </p>
            </div>
          );
        })}
      </div>
    </Card>
  );
}
