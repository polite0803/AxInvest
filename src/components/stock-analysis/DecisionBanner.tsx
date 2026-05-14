import { useStockAnalysisStore } from "@/stores";
import { Alert, Tag } from "antd";
import { useTranslation } from "react-i18next";

const ACTION_COLORS: Record<string, "success" | "warning" | "error" | "info"> = {
  "买入": "success",
  "增持": "success",
  "持有": "info",
  "减持": "warning",
  "卖出": "error",
};

export function DecisionBanner() {
  const { t } = useTranslation();
  const decision = useStockAnalysisStore((s) => s.decision);

  if (!decision) return null;

  const color = ACTION_COLORS[decision.action] || "info";

  return (
    <Alert
      type={color as "success" | "warning" | "error" | "info"}
      showIcon
      message={
        <div>
          <span className="font-semibold">{t("stockAnalysis.finalDecision")}: </span>
          <Tag color={color === "success" ? "green" : color === "error" ? "red" : "blue"}>
            {decision.action}
          </Tag>
          <span> {t("stockAnalysis.position")}: {decision.positionPct}%</span>
        </div>
      }
      description={
        <div className="text-xs" style={{ whiteSpace: "pre-wrap" }}>
          {decision.reasoning}
          <div className="mt-1">
            {decision.targetPrice && (
              <span>{t("stockAnalysis.targetPrice")}: {decision.targetPrice} &nbsp;</span>
            )}
            {decision.stopLoss && (
              <span>{t("stockAnalysis.stopLoss")}: {decision.stopLoss} &nbsp;</span>
            )}
            <Tag>{t("stockAnalysis.riskLevel")}: {decision.riskLevel}</Tag>
            <Tag>{t("stockAnalysis.confidence")}: {(decision.confidence * 100).toFixed(0)}%</Tag>
          </div>
        </div>
      }
    />
  );
}
