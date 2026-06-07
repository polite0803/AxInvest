import { useRightPanel } from "@/hooks/useRightPanel";
import type { EvidenceRef, TimelineNode } from "@/types";
import { ChevronDown, ChevronRight, Send } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";

interface TimelineNodeCardProps {
  node: TimelineNode;
}

/** 紫底 + agent 名首字母徽章(按 plan 8.3 要求) */
function NodeBadge({ name }: { name: string }) {
  const initial = name.trim().charAt(0).toUpperCase() || "?";
  return (
    <div
      className="flex items-center justify-center rounded-full text-[10px] font-bold text-white shrink-0"
      style={{
        width: 22,
        height: 22,
        background: "var(--accent, #7c3aed)",
        flexShrink: 0,
      }}
      title={name}
    >
      {initial}
    </div>
  );
}

/** 证据 chip:点击 → useRightPanel.navigateTo */
function EvidenceChip({ ref: ev }: { ref: EvidenceRef }) {
  const { navigateTo } = useRightPanel();
  return (
    <button
      type="button"
      className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded cursor-pointer border"
      style={{
        background: "var(--surface)",
        borderColor: "var(--border)",
        color: "var(--color-text)",
      }}
      onClick={() => navigateTo(ev.tabKey, ev.panelKey, ev.anchor)}
      title={`${ev.tabKey} / ${ev.panelKey}`}
    >
      🔗 {ev.snippet}
    </button>
  );
}

/** 节点卡片:紧凑态 1 行 / 展开态 3-5 行 */
export function TimelineNodeCard({ node }: TimelineNodeCardProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [expanded, setExpanded] = useState(false);

  const statusColor = node.status === "done"
    ? "var(--sa-green)"
    : node.status === "failed"
    ? "var(--sa-red)"
    : node.status === "running"
    ? "var(--accent)"
    : "var(--muted)";

  const statusLabel = node.status === "done"
    ? "✓"
    : node.status === "failed"
    ? "✕"
    : node.status === "running"
    ? "⟳"
    : "·";

  return (
    <div
      className="rounded border text-[11px] relative"
      style={{
        background: "var(--surface)",
        borderColor: node.status === "failed" ? "var(--sa-red)" : "var(--border)",
        borderLeft: `3px solid ${statusColor}`,
      }}
    >
      <button
        type="button"
        className="flex items-center gap-2 w-full text-left p-1.5"
        onClick={() => setExpanded(!expanded)}
        style={{ color: "var(--color-text)" }}
      >
        {expanded
          ? <ChevronDown size={11} style={{ flexShrink: 0 }} />
          : <ChevronRight size={11} style={{ flexShrink: 0 }} />}
        <NodeBadge name={node.agentName} />
        <span className="font-medium truncate flex-1" title={node.title}>
          {node.title}
        </span>
        <span style={{ color: statusColor, fontWeight: 600, flexShrink: 0 }}>
          {statusLabel}
        </span>
      </button>

      {expanded && (
        <div className="px-2 pb-2 space-y-1.5">
          {node.summary && (
            <div
              className="text-[11px] leading-relaxed"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {node.summary}
            </div>
          )}

          {node.confidence > 0 && (
            <div className="flex items-center gap-1.5 text-[10px]">
              <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.confidence")}</span>
              <div
                className="flex-1 rounded-full overflow-hidden"
                style={{ height: 4, background: "var(--border)" }}
              >
                <div
                  style={{
                    width: `${Math.round(node.confidence * 100)}%`,
                    height: "100%",
                    background: statusColor,
                  }}
                />
              </div>
              <span className="font-mono">{Math.round(node.confidence * 100)}%</span>
            </div>
          )}

          {node.evidenceRefs.length > 0 && (
            <div className="flex flex-wrap gap-1">
              {node.evidenceRefs.map((ev, i) => <EvidenceChip key={i} ref={ev} />)}
            </div>
          )}

          <div className="flex gap-1 pt-0.5">
            <button
              type="button"
              className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded border"
              style={{
                background: "transparent",
                borderColor: "var(--border)",
                color: "var(--color-text-secondary)",
              }}
              title={t("stockAnalysis.timeline.sendToChat")}
              onClick={() => {
                // 简化：跳转到对话页并附 query
                navigate(`/?refTimeline=${encodeURIComponent(node.id)}`);
              }}
            >
              <Send size={9} />
              {t("stockAnalysis.timeline.sendToChat")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
