import { useSettingsStore } from "@/stores";
import { ANALYST_NAMES } from "@/types";
import { Card, Tag } from "antd";
import NodeRenderer from "markstream-react";
import { useTranslation } from "react-i18next";
import { cleanToolCallTags } from "./utils";

interface Props {
  expertId: string;
  report: string;
}

interface ParsedReport {
  type?: string;
  summary?: string;
  signals?: string[];
  risk_flags?: string[];
  argument?: string;
  key_points?: string[];
  confidence?: number;
}

function tryParse(report: string): ParsedReport | null {
  try {
    const trimmed = report.trim();
    if (trimmed.startsWith("{")) { return JSON.parse(trimmed); }
    const m = trimmed.match(/```json\s*([\s\S]*?)\s*```/);
    if (m) { return JSON.parse(m[1]); }
  } catch { /* not JSON, show raw */ }
  return null;
}

export function AnalystReportCard({ expertId, report }: Props) {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const name = ANALYST_NAMES[expertId] || expertId;
  const cleanedReport = cleanToolCallTags(report);
  const parsed = tryParse(cleanedReport);

  if (!parsed) {
    return (
      <Card size="small" title={name} styles={{ body: { maxHeight: 320, overflow: "auto" } }}>
        <div className="sa-markdown-content">
          <NodeRenderer content={cleanedReport || report} isDark={isDark} />
        </div>
      </Card>
    );
  }

  const { type, summary, signals, risk_flags, argument, key_points, confidence } = parsed;

  return (
    <Card
      size="small"
      title={
        <span>
          {name}
          {type && <Tag style={{ marginLeft: 8 }}>{type}</Tag>}
        </span>
      }
      styles={{ body: { maxHeight: 400, overflow: "auto" } }}
    >
      {(summary || argument) && (
        <div className="sa-markdown-content text-xs">
          <NodeRenderer content={summary || argument || ""} isDark={isDark} />
        </div>
      )}

      {key_points && key_points.length > 0 && (
        <ul className="text-xs list-disc pl-4 mb-1" style={{ color: "var(--muted)" }}>
          {key_points.map((p, i) => <li key={i}>{p}</li>)}
        </ul>
      )}

      {signals && signals.length > 0 && (
        <div className="flex gap-1 flex-wrap mt-1">
          {signals.map((s, i) => (
            <Tag
              key={i}
              color={s.includes("买") || s.includes("多")
                ? "green"
                : s.includes("卖") || s.includes("空")
                ? "red"
                : "blue"}
            >
              {s}
            </Tag>
          ))}
        </div>
      )}

      {risk_flags && risk_flags.length > 0 && (
        <div className="flex gap-1 flex-wrap mt-1">
          {risk_flags.map((r, i) => <Tag key={i} color="orange">{r}</Tag>)}
        </div>
      )}

      {confidence != null && (
        <div className="text-xs mt-1" style={{ color: "var(--muted)" }}>
          {t("stockAnalysis.confidence")}: {(confidence * 100).toFixed(0)}%
        </div>
      )}
    </Card>
  );
}
