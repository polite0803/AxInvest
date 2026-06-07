/**
 * CompactAnalystSummary — AnalystReportGrid 在 chat 中的紧凑版本
 * 输入:分析师报告 { expertId: report }
 * 输出:多空条 + Top 3 报告摘要
 */
import { useMemo } from "react";

interface CompactAnalystSummaryProps {
  data: Record<string, string> | unknown;
}

function cleanToolCallTags(text: string): string {
  return (text ?? "").replace(/<tool_call[\s\S]*?<\/tool_call>/g, "").trim();
}

function classifySentiment(text: string): "bullish" | "bearish" | "neutral" {
  const cleaned = cleanToolCallTags(text);
  const bullWords = [
    "buy",
    "买入",
    "增持",
    "推荐",
    "看好",
    "上涨",
    "看多",
    "推荐买入",
    "outperform",
    "overweight",
    "strong buy",
  ];
  const bearWords = ["sell", "卖出", "减持", "看空", "下跌", "看淡", "看低", "underperform", "underweight", "reduce"];
  let bull = 0;
  let bear = 0;
  for (const w of bullWords) { bull += (cleaned.match(new RegExp(w, "gi")) || []).length; }
  for (const w of bearWords) { bear += (cleaned.match(new RegExp(w, "gi")) || []).length; }
  if (bull > bear && bull > 0) { return "bullish"; }
  if (bear > bull && bear > 0) { return "bearish"; }
  return "neutral";
}

function normalizeMap(data: CompactAnalystSummaryProps["data"]): Record<string, string> {
  if (data && typeof data === "object" && !Array.isArray(data)) {
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(data as Record<string, unknown>)) {
      if (typeof v === "string") { out[k] = v; }
    }
    return out;
  }
  return {};
}

function getAgentShortName(id: string): string {
  // 形如 "a-value-investor" → "Value"
  // 形如 "a-momentum-trader" → "Momentum"
  const m = id.replace(/^a-/, "").replace(/-/g, " ");
  const parts = m.split(" ");
  // 取前 1-2 词首字母
  if (parts.length === 1) { return parts[0].slice(0, 8); }
  return (parts[0][0] + (parts[1]?.[0] ?? "")).toUpperCase();
}

export function CompactAnalystSummary({ data }: CompactAnalystSummaryProps) {
  const reports = useMemo(() => normalizeMap(data), [data]);

  const summary = useMemo(() => {
    const entries = Object.entries(reports);
    if (entries.length === 0) { return null; }
    let bullish = 0;
    let bearish = 0;
    let neutral = 0;
    const details: Array<{ id: string; name: string; sentiment: "bullish" | "bearish" | "neutral"; snippet: string }> =
      [];
    for (const [id, rawReport] of entries) {
      const cleaned = cleanToolCallTags(rawReport);
      const sentiment = classifySentiment(cleaned);
      if (sentiment === "bullish") { bullish++; }
      else if (sentiment === "bearish") { bearish++; }
      else { neutral++; }
      details.push({
        id,
        name: getAgentShortName(id),
        sentiment,
        snippet: cleaned.slice(0, 80).replace(/\s+/g, " ").trim(),
      });
    }
    const total = bullish + bearish + neutral;
    return { bullish, bearish, neutral, total, details };
  }, [reports]);

  if (!summary) {
    return (
      <div className="text-[12px] italic" style={{ color: "var(--muted)" }}>
        暂无分析师报告
      </div>
    );
  }

  return (
    <div className="space-y-1 text-[12px]">
      <div className="flex items-baseline gap-2 flex-wrap">
        <span style={{ color: "var(--muted)" }}>{summary.total} 份</span>
        <span style={{ color: "var(--sa-red, #dc2626)" }}>📈 {summary.bullish}</span>
        <span style={{ color: "var(--sa-green, #16a34a)" }}>📉 {summary.bearish}</span>
        <span style={{ color: "var(--muted)" }}>➖ {summary.neutral}</span>
      </div>
      {summary.total > 0 && (
        <div className="flex h-1.5 rounded overflow-hidden" style={{ background: "var(--muted-bg, #e5e7eb)" }}>
          {summary.bullish > 0 && (
            <div
              style={{ width: `${(summary.bullish / summary.total) * 100}%`, background: "var(--sa-red, #dc2626)" }}
            />
          )}
          {summary.neutral > 0 && (
            <div
              style={{ width: `${(summary.neutral / summary.total) * 100}%`, background: "var(--muted, #6b7280)" }}
            />
          )}
          {summary.bearish > 0 && (
            <div
              style={{ width: `${(summary.bearish / summary.total) * 100}%`, background: "var(--sa-green, #16a34a)" }}
            />
          )}
        </div>
      )}
      <div className="space-y-0.5 mt-1">
        {summary.details.slice(0, 3).map((d) => (
          <div key={d.id} className="flex items-start gap-1.5 text-[11px]">
            <span
              className="text-[9px] font-mono px-1 rounded shrink-0"
              style={{
                background: d.sentiment === "bullish"
                  ? "var(--sa-red-bg, #fee2e2)"
                  : d.sentiment === "bearish"
                  ? "var(--sa-green-bg, #dcfce7)"
                  : "var(--muted-bg, #e5e7eb)",
                color: d.sentiment === "bullish"
                  ? "var(--sa-red, #dc2626)"
                  : d.sentiment === "bearish"
                  ? "var(--sa-green, #16a34a)"
                  : "var(--muted, #6b7280)",
              }}
            >
              {d.name}
            </span>
            <span className="flex-1 truncate leading-snug" style={{ color: "var(--color-text-secondary)" }}>
              {d.snippet}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
