import { invoke } from "@/lib/invoke";
import { Card, Collapse, Tag } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  analysisId: string;
}

export function HistoricalAnalysisPanel({ analysisId }: Props) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [snapshot, setSnapshot] = useState<Record<string, string> | null>(null);

  useEffect(() => {
    if (!analysisId) { return; }
    setLoading(true);
    invoke<{ blackboardSnapshot: string | null }>("get_stock_analysis", { analysisId })
      .then((record) => {
        if (record.blackboardSnapshot) {
          setSnapshot(JSON.parse(record.blackboardSnapshot));
        }
      })
      .catch(() => {/* 静默 */})
      .finally(() => setLoading(false));
  }, [analysisId]);

  if (!analysisId || (!loading && !snapshot)) { return null; }

  if (loading) {
    return (
      <Card size="small" title={t("stockAnalysis.history")} styles={{ body: { padding: 8 } }}>
        <div className="ax-skeleton" style={{ height: 24, borderRadius: 4 }} />
      </Card>
    );
  }

  const reportEntries = Object.entries(snapshot ?? {}).filter(
    ([k]) => k.startsWith("report."),
  );
  const debateEntries = Object.entries(snapshot ?? {}).filter(
    ([k]) => k.startsWith("debate."),
  );

  if (reportEntries.length === 0 && debateEntries.length === 0) { return null; }

  return (
    <Card size="small" title={t("stockAnalysis.history")} styles={{ body: { padding: "6px 8px" } }}>
      <Collapse
        size="small"
        items={[
          ...reportEntries.slice(0, 6).map(([key, value]) => ({
            key,
            label: (
              <span className="text-xs">
                {key.replace("report.", "")}
                <Tag style={{ marginLeft: 6, fontSize: 10 }}>
                  {t("stockAnalysis.charCount", { count: value.length })}
                </Tag>
              </span>
            ),
            children: (
              <pre
                className="text-xs"
                style={{
                  whiteSpace: "pre-wrap",
                  maxHeight: 200,
                  overflow: "auto",
                  margin: 0,
                }}
              >
                {value}
              </pre>
            ),
          })),
          ...(debateEntries.length > 0
            ? [
              {
                key: "debates",
                label: <span className="text-xs">{t("stockAnalysis.debateHistory")}</span>,
                children: (
                  <pre
                    className="text-xs"
                    style={{
                      whiteSpace: "pre-wrap",
                      maxHeight: 200,
                      overflow: "auto",
                      margin: 0,
                    }}
                  >
                      {debateEntries
                        .map(([k, v]) => `### ${k}\n${v}`)
                        .join("\n\n")}
                  </pre>
                ),
              },
            ]
            : []),
        ]}
      />
    </Card>
  );
}
