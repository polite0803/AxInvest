import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { Card, Collapse, Tag } from "antd";
import NodeRenderer from "markstream-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { cleanToolCallTags } from "./utils";

export function DebatePanel() {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
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
      <div className="mb-3">
        <div className="flex justify-between text-xs mb-1">
          <span style={{ color: "var(--sa-green)" }}>{t("stockAnalysis.bear")}</span>
          <span className="font-semibold" style={{ fontSize: 13 }}>{sentimentText}</span>
          <span style={{ color: "var(--sa-red)" }}>{t("stockAnalysis.bull")}</span>
        </div>
        <div
          className="relative"
          style={{
            height: 18,
            borderRadius: 9,
            background: "linear-gradient(to right, var(--sa-green), var(--sa-amber), var(--sa-red))",
            overflow: "hidden",
          }}
        >
          <div
            style={{
              position: "absolute",
              left: pointerLeft,
              top: 0,
              width: 2,
              height: 18,
              background: "var(--surface)",
              boxShadow: "0 0 4px rgba(0,0,0,0.4)",
              transform: "translateX(-1px)",
              zIndex: 2,
            }}
          />
        </div>
        <div className="flex justify-between text-xs mt-1">
          <span style={{ color: "var(--muted)" }}>
            🐻 {bearCount}
            {t("stockAnalysis.views")}
          </span>
          <span className="font-mono" style={{ color: "var(--muted)" }}>
            {pct}:{100 - pct}
          </span>
          <span style={{ color: "var(--muted)" }}>
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
            <div className="flex flex-col sm:flex-row gap-2" style={{ maxHeight: 360, overflow: "auto" }}>
              <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid var(--sa-red)" }}>
                <Tag color="red">{t("stockAnalysis.bull")}</Tag>
                <div className="sa-markdown-content text-xs mt-1">
                  <NodeRenderer content={cleanToolCallTags(r.bull)} isDark={isDark} />
                </div>
              </div>
              <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid var(--sa-green)" }}>
                <Tag color="green">{t("stockAnalysis.bear")}</Tag>
                <div className="sa-markdown-content text-xs mt-1">
                  <NodeRenderer content={cleanToolCallTags(r.bear)} isDark={isDark} />
                </div>
              </div>
            </div>
          ),
        }))}
      />
      {debateRounds.length > 4 && (
        <div className="text-center text-xs mt-1" style={{ color: "var(--muted)" }}>
          {t("stockAnalysis.moreRounds", { count: debateRounds.length - 4 })}
        </div>
      )}
    </Card>
  );
}
