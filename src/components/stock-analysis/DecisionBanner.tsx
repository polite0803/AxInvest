import { invoke } from "@/lib/invoke";
import { useStockAnalysisStore } from "@/stores";
import { Alert, Button, message, Tag } from "antd";
import { useState } from "react";
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
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const [watchlisted, setWatchlisted] = useState(false);
  const [adding, setAdding] = useState(false);

  if (!decision) { return null; }

  const color = ACTION_COLORS[decision.action] || "info";
  const actionLabel: Record<string, string> = {
    "买入": t("stockAnalysis.actionBuy"),
    "增持": t("stockAnalysis.actionIncrease"),
    "持有": t("stockAnalysis.actionHold"),
    "减持": t("stockAnalysis.actionReduce"),
    "卖出": t("stockAnalysis.actionSell"),
  };

  const addToWatchlist = async () => {
    if (!stockCode || !stockName) { return; }
    setAdding(true);
    try {
      await invoke("add_to_watchlist", { stockCode, stockName });
      setWatchlisted(true);
      message.success(t("stockAnalysis.addedToWatchlist"));
    } catch {
      message.error(t("stockAnalysis.addFailed"));
    }
    setAdding(false);
  };

  return (
    <Alert
      type={color as "success" | "warning" | "error" | "info"}
      showIcon
      message={
        <div>
          <span className="font-semibold">{t("stockAnalysis.finalDecision")}:</span>
          <Tag color={color === "success" ? "green" : color === "error" ? "red" : "blue"}>
            {actionLabel[decision.action] || decision.action}
          </Tag>
          <span>{t("stockAnalysis.position")}: {decision.positionPct}%</span>
        </div>
      }
      description={
        <div className="text-xs">
          <div style={{ whiteSpace: "pre-wrap", marginBottom: 8 }}>{decision.reasoning}</div>
          <div className="flex gap-2 flex-wrap items-center">
            {decision.targetPrice && <Tag>{t("stockAnalysis.targetPrice")}: ¥{decision.targetPrice}</Tag>}
            {decision.stopLoss && <Tag>{t("stockAnalysis.stopLoss")}: ¥{decision.stopLoss}</Tag>}
            <Tag>{t("stockAnalysis.riskLevel")}: {decision.riskLevel}</Tag>
            <Tag>{t("stockAnalysis.confidence")}: {(decision.confidence * 100).toFixed(0)}%</Tag>
            {stockCode && !watchlisted && (
              <Button size="small" type="dashed" loading={adding} onClick={addToWatchlist}>
                ⭐ {t("stockAnalysis.addToWatchlist")}
              </Button>
            )}
            {watchlisted && <Tag color="gold">⭐ {t("stockAnalysis.inWatchlist")}</Tag>}
          </div>
        </div>
      }
    />
  );
}
