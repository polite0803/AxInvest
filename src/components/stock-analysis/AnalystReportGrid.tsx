import { useStockAnalysisStore } from "@/stores";
import { classifySentiment } from "@/types/stock-analysis";
import { Tooltip } from "antd";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { AnalystReportCard } from "./AnalystReportCard";
import { cleanToolCallTags } from "./utils";

type Consensus = "bullish" | "bearish" | "neutral" | "divided";

/** 根据多空比例推共识：
 *  - 一方 > 65% → 共识看多 / 共识看空
 *  - 双方都有且 35-65% → 分歧
 *  - 全部中性 / 单边极小 → 中性
 */
function deriveConsensus(
  bullish: number,
  bearish: number,
  neutral: number,
): Consensus {
  const total = bullish + bearish + neutral;
  if (total === 0) { return "neutral"; }
  const bullRatio = bullish / total;
  const bearRatio = bearish / total;
  if (bullRatio > 0.65) { return "bullish"; }
  if (bearRatio > 0.65) { return "bearish"; }
  if (bullRatio > 0 && bearRatio > 0) { return "divided"; }
  return "neutral";
}

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
      const sentiment = classifySentiment(report);
      if (sentiment === "bullish") { bullish++; }
      else if (sentiment === "bearish") { bearish++; }
      else { neutral++; }
    }
    const total = bullish + bearish + neutral;
    return { bullish, bearish, neutral, total };
  }, [analystReports]);

  const consensus = useMemo(
    () => deriveConsensus(sentiment.bullish, sentiment.bearish, sentiment.neutral),
    [sentiment],
  );

  if (Object.keys(analystReports).length === 0) { return null; }

  const consensusConfig: Record<Consensus, { color: string; bg: string; labelKey: string; icon: string }> = {
    bullish: {
      color: "var(--sa-red, #dc2626)",
      bg: "var(--sa-red-bg, #fee2e2)",
      labelKey: "consensusBullish",
      icon: "📈",
    },
    bearish: {
      color: "var(--sa-green, #16a34a)",
      bg: "var(--sa-green-bg, #dcfce7)",
      labelKey: "consensusBearish",
      icon: "📉",
    },
    neutral: {
      color: "var(--muted, #6b7280)",
      bg: "var(--muted-bg, #e5e7eb)",
      labelKey: "consensusNeutral",
      icon: "➖",
    },
    divided: {
      color: "var(--ant-warning, #f59e0b)",
      bg: "var(--ant-warning-bg, #fef3c7)",
      labelKey: "consensusDivided",
      icon: "⚖️",
    },
  };
  const cc = consensusConfig[consensus];
  const total = sentiment.total;
  const bullPct = total > 0 ? Math.round((sentiment.bullish / total) * 100) : 0;
  const bearPct = total > 0 ? Math.round((sentiment.bearish / total) * 100) : 0;
  const neutPct = total > 0 ? Math.round((sentiment.neutral / total) * 100) : 0;

  return (
    <div>
      {/* 舆情摘要 bar — 醒目的共识卡片 + 大号数字 + 百分比 + 色彩条 */}
      {sentiment.total > 0 && (
        <div
          className="mb-3 p-3 rounded-md"
          style={{ background: "var(--surface)", border: "1px solid var(--border, #e5e7eb)" }}
        >
          {/* 共识标签 + 总数 */}
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2">
              <span
                className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-sm font-semibold"
                style={{ background: cc.bg, color: cc.color }}
              >
                <span>{cc.icon}</span>
                <span>{t(`stockAnalysis.recommendation.${cc.labelKey}`)}</span>
              </span>
              <span className="text-xs" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.tab.analysts")} · {total}
                {t("stockAnalysis.recommendation.reportCountSuffix")}
              </span>
            </div>
          </div>

          {/* 大号三色数字 */}
          <div className="grid grid-cols-3 gap-2 mb-2">
            <Tooltip title={t("stockAnalysis.recommendation.bullishTooltip")}>
              <div
                className="rounded p-2 text-center cursor-default"
                style={{ background: "var(--sa-red-bg, #fee2e2)" }}
              >
                <div className="text-2xl font-bold leading-none" style={{ color: "var(--sa-red, #dc2626)" }}>
                  {sentiment.bullish}
                </div>
                <div className="text-[10px] mt-1" style={{ color: "var(--sa-red, #dc2626)" }}>
                  📈 {t("stockAnalysis.recommendation.bullish")} {bullPct}%
                </div>
              </div>
            </Tooltip>
            <Tooltip title={t("stockAnalysis.recommendation.bearishTooltip")}>
              <div
                className="rounded p-2 text-center cursor-default"
                style={{ background: "var(--sa-green-bg, #dcfce7)" }}
              >
                <div
                  className="text-2xl font-bold leading-none"
                  style={{ color: "var(--sa-green, #16a34a)" }}
                >
                  {sentiment.bearish}
                </div>
                <div className="text-[10px] mt-1" style={{ color: "var(--sa-green, #16a34a)" }}>
                  📉 {t("stockAnalysis.recommendation.bearish")} {bearPct}%
                </div>
              </div>
            </Tooltip>
            <Tooltip title={t("stockAnalysis.recommendation.neutralTooltip")}>
              <div
                className="rounded p-2 text-center cursor-default"
                style={{ background: "var(--muted-bg, #e5e7eb)" }}
              >
                <div className="text-2xl font-bold leading-none" style={{ color: "var(--muted, #6b7280)" }}>
                  {sentiment.neutral}
                </div>
                <div className="text-[10px] mt-1" style={{ color: "var(--muted, #6b7280)" }}>
                  ➖ {t("stockAnalysis.recommendation.neutral")} {neutPct}%
                </div>
              </div>
            </Tooltip>
          </div>

          {/* 色彩条 */}
          <div className="flex h-2.5 rounded-full overflow-hidden" style={{ background: "var(--muted-bg, #e5e7eb)" }}>
            {sentiment.bullish > 0 && (
              <div
                style={{
                  width: `${bullPct}%`,
                  background: "var(--sa-red, #dc2626)",
                  transition: "width 0.3s ease",
                }}
              />
            )}
            {sentiment.neutral > 0 && (
              <div
                style={{
                  width: `${neutPct}%`,
                  background: "var(--muted, #6b7280)",
                  transition: "width 0.3s ease",
                }}
              />
            )}
            {sentiment.bearish > 0 && (
              <div
                style={{
                  width: `${bearPct}%`,
                  background: "var(--sa-green, #16a34a)",
                  transition: "width 0.3s ease",
                }}
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
