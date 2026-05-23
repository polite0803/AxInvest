import { useStockAnalysisStore } from "@/stores";
import { Card, Collapse, Tag } from "antd";
import { useTranslation } from "react-i18next";

export function DebatePanel() {
  const { t } = useTranslation();
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);

  if (debateRounds.length === 0) { return null; }

  return (
    <Card size="small" title={t("stockAnalysis.debate")}>
      <Collapse
        size="small"
        items={debateRounds.map((r, i) => ({
          key: i,
          label: <span>{t("stockAnalysis.debateRound").replace("{round}", String(r.round))}</span>,
          children: (
            <div className="flex flex-col sm:flex-row gap-2">
              <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid #cf1322" }}>
                <Tag color="red">{t("stockAnalysis.bull")}</Tag>
                <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap", maxHeight: 300, overflow: "auto" }}>
                  {r.bull}
                </p>
              </div>
              <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid #3f8600" }}>
                <Tag color="green">{t("stockAnalysis.bear")}</Tag>
                <p className="text-xs mt-1" style={{ whiteSpace: "pre-wrap", maxHeight: 300, overflow: "auto" }}>
                  {r.bear}
                </p>
              </div>
            </div>
          ),
        }))}
      />
    </Card>
  );
}
