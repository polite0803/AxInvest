import { useSettingsStore } from "@/stores";
import { ANALYST_NAMES } from "@/types";
import { getSignalColor } from "@/types/stock-analysis";
import { ExpandOutlined } from "@ant-design/icons";
import { Button, Card, Empty, Modal, Tag } from "antd";
import NodeRenderer from "markstream-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cleanToolCallTags, tryBeautifyJson } from "./utils";

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
  const beautified = tryBeautifyJson(cleanedReport);
  const parsed = tryParse(beautified);
  const displayContent = beautified || report;
  const [expanded, setExpanded] = useState(false);
  const hasContent = !!displayContent || !!parsed;

  const expandBtn = hasContent
    ? (
      <Button
        type="text"
        size="small"
        icon={<ExpandOutlined />}
        onClick={() => setExpanded(true)}
        title={t("stockAnalysis.expandView")}
      />
    )
    : null;

  const modalContent = parsed
    ? <ParsedReportModalContent parsed={parsed} isDark={isDark} t={t} />
    : (
      <div className="sa-markdown-content">
        {displayContent
          ? <NodeRenderer content={displayContent} isDark={isDark} />
          : <div style={{ color: "var(--muted)", textAlign: "center" }}>{t("stockAnalysis.noReport")}</div>}
      </div>
    );

  return (
    <>
      {!parsed
        ? (
          <Card
            size="small"
            title={name}
            extra={expandBtn}
            styles={{ body: { maxHeight: 320, overflow: "auto" } }}
          >
            {displayContent
              ? (
                <div className="sa-markdown-content">
                  <NodeRenderer content={displayContent} isDark={isDark} />
                </div>
              )
              : (
                <Empty
                  image={Empty.PRESENTED_IMAGE_SIMPLE}
                  description={
                    <span style={{ color: "var(--muted)", fontSize: 12 }}>
                      {t("stockAnalysis.noReport")}
                    </span>
                  }
                />
              )}
          </Card>
        )
        : (
          <Card
            size="small"
            title={
              <span>
                {name}
                {parsed.type && <Tag style={{ marginLeft: 8 }}>{parsed.type}</Tag>}
              </span>
            }
            extra={expandBtn}
            styles={{ body: { maxHeight: 400, overflow: "auto" } }}
          >
            {(parsed.summary || parsed.argument) && (
              <div className="sa-markdown-content text-xs">
                <NodeRenderer content={parsed.summary || parsed.argument || ""} isDark={isDark} />
              </div>
            )}
            {parsed.key_points && parsed.key_points.length > 0 && (
              <ul className="text-xs list-disc pl-4 mb-1" style={{ color: "var(--muted)" }}>
                {parsed.key_points.map((p, i) => <li key={i}>{p}</li>)}
              </ul>
            )}
            {parsed.signals && parsed.signals.length > 0 && (
              <div className="flex gap-1 flex-wrap mt-1">
                {parsed.signals.map((s, i) => (
                  <Tag
                    key={i}
                    color={getSignalColor(s)}
                  >
                    {s}
                  </Tag>
                ))}
              </div>
            )}
            {parsed.risk_flags && parsed.risk_flags.length > 0 && (
              <div className="flex gap-1 flex-wrap mt-1">
                {parsed.risk_flags.map((r, i) => <Tag key={i} color="orange">{r}</Tag>)}
              </div>
            )}
            {parsed.confidence != null && (
              <div className="text-xs mt-1" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.confidence")}: {(parsed.confidence * 100).toFixed(0)}%
              </div>
            )}
          </Card>
        )}
      <Modal
        title={name}
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={null}
        width="80vw"
        style={{ top: 20 }}
        styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
      >
        {modalContent}
      </Modal>
    </>
  );
}

function ParsedReportModalContent(
  { parsed, isDark, t }: { parsed: ParsedReport; isDark: boolean; t: (k: string) => string },
) {
  return (
    <div className="sa-markdown-content">
      {(parsed.summary || parsed.argument) && (
        <NodeRenderer content={parsed.summary || parsed.argument || ""} isDark={isDark} />
      )}
      {parsed.key_points && parsed.key_points.length > 0 && (
        <ul className="list-disc pl-6 my-2">
          {parsed.key_points.map((p, i) => <li key={i}>{p}</li>)}
        </ul>
      )}
      {parsed.signals && parsed.signals.length > 0 && (
        <div className="flex gap-1 flex-wrap my-2">
          {parsed.signals.map((s, i) => (
            <Tag
              key={i}
              color={getSignalColor(s)}
            >
              {s}
            </Tag>
          ))}
        </div>
      )}
      {parsed.risk_flags && parsed.risk_flags.length > 0 && (
        <div className="flex gap-1 flex-wrap my-2">
          {parsed.risk_flags.map((r, i) => <Tag key={i} color="orange">{r}</Tag>)}
        </div>
      )}
      {parsed.confidence != null && (
        <div className="text-sm mt-2" style={{ color: "var(--muted)" }}>
          {t("stockAnalysis.confidence")}: {(parsed.confidence * 100).toFixed(0)}%
        </div>
      )}
    </div>
  );
}
