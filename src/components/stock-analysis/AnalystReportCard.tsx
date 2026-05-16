import { ANALYST_NAMES } from "@/types";
import { Card, Tag, Typography } from "antd";

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
  const name = ANALYST_NAMES[expertId] || expertId;
  const parsed = tryParse(report);

  if (!parsed) {
    return (
      <Card size="small" title={name}>
        <Typography.Paragraph ellipsis={{ rows: 5, expandable: true }}>
          {report}
        </Typography.Paragraph>
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
    >
      {/* 摘要 */}
      {(summary || argument) && (
        <Typography.Paragraph ellipsis={{ rows: 4, expandable: true }} className="text-xs">
          {summary || argument}
        </Typography.Paragraph>
      )}

      {/* 关键论点 */}
      {key_points && key_points.length > 0 && (
        <ul className="text-xs list-disc pl-4 mb-1" style={{ color: "var(--color-text-secondary)" }}>
          {key_points.map((p, i) => <li key={i}>{p}</li>)}
        </ul>
      )}

      {/* 信号标签 */}
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

      {/* 风险标记 */}
      {risk_flags && risk_flags.length > 0 && (
        <div className="flex gap-1 flex-wrap mt-1">
          {risk_flags.map((r, i) => <Tag key={i} color="orange">{r}</Tag>)}
        </div>
      )}

      {/* 置信度 */}
      {confidence != null && (
        <div className="text-xs mt-1" style={{ color: "var(--color-text-tertiary)" }}>
          置信度: {(confidence * 100).toFixed(0)}%
        </div>
      )}
    </Card>
  );
}
