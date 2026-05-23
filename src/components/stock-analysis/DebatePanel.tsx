import { useStockAnalysisStore } from "@/stores";
import { Card, Collapse, Tag } from "antd";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

export function DebatePanel() {
  const { t } = useTranslation();
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);

  const [sentimentRatio, bullCount, bearCount] = useMemo(() => {
    if (debateRounds.length === 0) { return [0.5, 0, 0]; }
    const bulls = debateRounds.map((r) => r.bull);
    const bears = debateRounds.map((r) => r.bear);
    const bs = bulls.join("").length;
    const bs2 = bears.join("").length;
    const total = bs + bs2;
    return [
      total > 0 ? bs / total : 0.5,
      debateRounds.length,
      debateRounds.length,
    ];
  }, [debateRounds]);

  const sentimentText = sentimentRatio > 0.55
    ? t("stockAnalysis.sentimentBullish")
    : sentimentRatio < 0.45
    ? t("stockAnalysis.sentimentBearish")
    : t("stockAnalysis.sentimentNeutral");
  const pct = Math.round(sentimentRatio * 100);
  const pointerLeft = `${sentimentRatio * 100}%`;

  if (debateRounds.length === 0) { return null; }

  return (
    <Card size="small" title={t("stockAnalysis.debate")}>
      {/* 情绪仪表 */}
      <div className="mb-3">
        <div className="flex justify-between text-xs mb-1">
          <span style={{ color: "#3fb950" }}>{t("stockAnalysis.bear")}</span>
          <span className="font-semibold" style={{ fontSize: 13 }}>{sentimentText}</span>
          <span style={{ color: "#cf1322" }}>{t("stockAnalysis.bull")}</span>
        </div>
        {/* 渐变色条 */}
        <div
          className="relative"
          style={{
            height: 18,
            borderRadius: 9,
            background: "linear-gradient(to right, #3fb950, #fadb14, #cf1322)",
            overflow: "hidden",
          }}
        >
          {/* 指针 */}
          <div
            style={{
              position: "absolute",
              left: pointerLeft,
              top: 0,
              width: 2,
              height: 18,
              background: "#fff",
              boxShadow: "0 0 4px rgba(0,0,0,0.4)",
              transform: "translateX(-1px)",
              zIndex: 2,
            }}
          />
          {/* 无数据时半透明遮罩 */}
        </div>
        {/* 统计数据 */}
        <div className="flex justify-between text-xs mt-1">
          <span style={{ color: "var(--color-text-secondary)" }}>
            🐻 {bearCount}
            {t("stockAnalysis.views")}
          </span>
          <span className="font-mono" style={{ color: "var(--color-text-secondary)" }}>
            {pct}:{100 - pct}
          </span>
          <span style={{ color: "var(--color-text-secondary)" }}>
            🐂 {bullCount}
            {t("stockAnalysis.views")}
          </span>
        </div>
      </div>

      <Collapse
        size="small"
        items={debateRounds.slice(0, 4).map((r, i) => ({
          key: i,
          label: <span>{t("stockAnalysis.debateRound", { round: r.round })}</span>,
          children: (
            <div className="flex flex-col sm:flex-row gap-2" style={{ maxHeight: 260, overflow: "auto" }}>
              <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid #cf1322" }}>
                <Tag color="red">{t("stockAnalysis.bull")}</Tag>
                <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap" }}>
                  {r.bull}
                </p>
              </div>
              <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid #3f8600" }}>
                <Tag color="green">{t("stockAnalysis.bear")}</Tag>
                <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap" }}>
                  {r.bear}
                </p>
              </div>
            </div>
          ),
        }))}
      />
      {debateRounds.length > 4 && (
        <div className="text-center text-xs mt-1" style={{ color: "var(--color-text-secondary)" }}>
          {t("stockAnalysis.moreRounds", { count: debateRounds.length - 4 })}
        </div>
      )}
    </Card>
  );
}
