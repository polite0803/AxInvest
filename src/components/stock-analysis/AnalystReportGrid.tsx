import { useStockAnalysisStore } from "@/stores";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { AnalystReportCard } from "./AnalystReportCard";
import { cleanToolCallTags } from "./utils";

export function AnalystReportGrid() {
  const { t } = useTranslation();
  const analystReports = useStockAnalysisStore((s) => s.analystReports);

  // Aggregate sentiment from reports (clean tool_call tags first)
  const sentiment = useMemo(() => {
    const entries = Object.values(analystReports);
    let bullish = 0;
    let bearish = 0;
    let neutral = 0;
    for (const rawReport of entries) {
      const report = cleanToolCallTags(rawReport);
      const lower = report.toLowerCase();
      if (lower.includes("买入") || lower.includes("增持") || lower.includes("看多") || lower.includes("推荐")) {
        bullish++;
      } else if (lower.includes("卖出") || lower.includes("减持") || lower.includes("看空") || lower.includes("回避")) {
        bearish++;
      } else {
        neutral++;
      }
    }
    const total = bullish + bearish + neutral;
    return { bullish, bearish, neutral, total };
  }, [analystReports]);

  if (Object.keys(analystReports).length === 0) { return null; }

  return (
    <div>
      {/* 舆情摘要 bar */}
      {sentiment.total > 0 && (
        <div className="mb-2 p-2 rounded text-xs" style={{ background: "var(--surface)" }}>
          <div className="flex justify-between mb-1">
            <span style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.tab.analysts")} · {sentiment.total} {t("stockAnalysis.views")}
            </span>
            <span>
              <span style={{ color: "var(--sa-red)" }}>📈 {sentiment.bullish}</span>
              {" / "}
              <span style={{ color: "var(--sa-green)" }}>📉 {sentiment.bearish}</span>
              {" / "}
              <span style={{ color: "var(--muted)" }}>➖ {sentiment.neutral}</span>
            </span>
          </div>
          <div className="flex h-2 rounded overflow-hidden">
            {sentiment.bullish > 0 && (
              <div style={{ width: `${(sentiment.bullish / sentiment.total) * 100}%`, background: "var(--sa-red)" }} />
            )}
            {sentiment.neutral > 0 && (
              <div style={{ width: `${(sentiment.neutral / sentiment.total) * 100}%`, background: "var(--muted)" }} />
            )}
            {sentiment.bearish > 0 && (
              <div
                style={{ width: `${(sentiment.bearish / sentiment.total) * 100}%`, background: "var(--sa-green)" }}
              />
            )}
          </div>
        </div>
      )}
      <div
        className="grid gap-2"
        style={{ gridTemplateColumns: "repeat(auto-fill, minmax(min(240px, 100%), 1fr))" }}
      >
        {Object.entries(analystReports).map(([expertId, report]) => (
          <AnalystReportCard key={expertId} expertId={expertId} report={report} />
        ))}
      </div>
    </div>
  );
}
