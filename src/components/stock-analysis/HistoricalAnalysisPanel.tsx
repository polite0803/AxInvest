import { invoke } from "@/lib/invoke";
import { Card, Collapse, Spin, Tag, Typography } from "antd";
import { useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  analysisId: string;
}

export function HistoricalAnalysisPanel({ analysisId }: Props) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [snapshot, setSnapshot] = useState<Record<string, string> | null>(null);

  const loadSnapshot = async () => {
    setLoading(true);
    try {
      const record = await invoke<{ blackboardSnapshot: string | null }>(
        "get_stock_analysis",
        { analysisId },
      );
      if (record.blackboardSnapshot) {
        setSnapshot(JSON.parse(record.blackboardSnapshot));
      }
    } catch (e) {
      console.error("Failed to load analysis snapshot:", e);
    }
    setLoading(false);
  };

  if (loading) { return <Spin />; }

  if (!snapshot) {
    return (
      <Card size="small" className="cursor-pointer" onClick={loadSnapshot}>
        <Typography.Text type="secondary">
          {t("stockAnalysis.loadHistory")}
        </Typography.Text>
      </Card>
    );
  }

  const reportEntries = Object.entries(snapshot).filter(
    ([k]) => k.startsWith("report."),
  );
  const debateEntries = Object.entries(snapshot).filter(
    ([k]) => k.startsWith("debate."),
  );

  return (
    <Card size="small" title={t("stockAnalysis.history")}>
      <Collapse
        size="small"
        items={[
          ...reportEntries.map(([key, value]) => ({
            key,
            label: (
              <span>
                {key.replace("report.", "")}
                <Tag style={{ marginLeft: 8 }}>
                  {value.length} 字
                </Tag>
              </span>
            ),
            children: (
              <pre
                className="text-xs"
                style={{
                  whiteSpace: "pre-wrap",
                  maxHeight: 300,
                  overflow: "auto",
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
                label: t("stockAnalysis.debateHistory"),
                children: (
                  <pre
                    className="text-xs"
                    style={{
                      whiteSpace: "pre-wrap",
                      maxHeight: 300,
                      overflow: "auto",
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
