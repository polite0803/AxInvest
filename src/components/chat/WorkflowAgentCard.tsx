/**
 * 工作流 Agent 卡片 — 在对话页中渲染工作流执行状态。
 */
import { Card, Progress, Tag } from "antd";
import { TrendingUp } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

// ── 卡片数据类型 ──

export interface WorkflowCardData {
  type: "progress" | "analyst" | "decision";
  phase?: string;
  completed?: number;
  total?: number;
  analystName?: string;
  analystReport?: string;
  action?: string;
  positionPct?: number;
  targetPrice?: number;
  stopLoss?: number;
  reasoning?: string;
  riskLevel?: string;
  confidence?: number;
}

// ── 解析消息内容中的 workflow 标记 ──

export function parseWorkflowCard(content: string): WorkflowCardData | null {
  const match = content.match(/^<!-- workflow-(progress|analyst|decision):(.*?) -->/);
  if (!match) { return null; }
  try {
    return { type: match[1] as WorkflowCardData["type"], ...JSON.parse(match[2]) };
  } catch {
    return null;
  }
}

export function makeWorkflowContent(type: string, data: Record<string, unknown>, fallback: string): string {
  return `<!-- workflow-${type}:${JSON.stringify(data)} -->${fallback}`;
}

// ── 组件 ──

export function WorkflowAgentCard({ data }: { data: WorkflowCardData }) {
  const { t } = useTranslation();

  const phaseLabel = useMemo(() => {
    if (!data.phase) { return t("stockAnalysis.workflow.initializing"); }
    return t(`stockAnalysis.workflow.phase.${data.phase}`, data.phase);
  }, [data.phase, t]);

  const analystLabel = useMemo(() => {
    if (!data.analystName) { return t("stockAnalysis.workflow.analyst"); }
    return t(`stockAnalysis.workflow.analyst.${data.analystName}`, data.analystName);
  }, [data.analystName, t]);

  if (data.type === "progress") {
    const pct = data.total && data.total > 0
      ? Math.round(((data.completed ?? 0) / data.total) * 100)
      : 0;
    return (
      <div className="workflow-card" style={{ padding: "12px 16px" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
          <TrendingUp size={16} style={{ color: "var(--accent)" }} />
          <span style={{ fontSize: 13, fontWeight: 600 }}>
            {t("stockAnalysis.workflow.title")}
          </span>
          <Tag color="processing" style={{ fontSize: 11 }}>{t("stockAnalysis.workflow.inProgress")}</Tag>
        </div>
        <Progress percent={pct} size="small" status="active" />
        <div style={{ fontSize: 12, color: "var(--muted)", marginTop: 4 }}>
          {t("stockAnalysis.workflow.currentPhase")}: {phaseLabel} ({data.completed ?? 0}/{data.total ?? "?"})
        </div>
      </div>
    );
  }

  if (data.type === "analyst") {
    const brief = data.analystReport
      ? data.analystReport.slice(0, 200) + (data.analystReport.length > 200 ? "..." : "")
      : t("stockAnalysis.workflow.analystComplete");
    return (
      <div className="workflow-card" style={{ padding: "10px 14px" }}>
        <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 4 }}>📊 {analystLabel}</div>
        <div style={{ fontSize: 12, color: "var(--muted)", lineHeight: 1.6 }}>{brief}</div>
      </div>
    );
  }

  if (data.type === "decision") {
    const isBull = data.action === t("stockAnalysis.actionBuy") || data.action === t("stockAnalysis.actionIncrease");
    const isBear = data.action === t("stockAnalysis.actionSell") || data.action === t("stockAnalysis.actionReduce");
    return (
      <Card
        size="small"
        style={{ borderColor: isBull ? "var(--sa-red)" : isBear ? "var(--sa-green)" : undefined }}
        title={
          <span>
            {isBull ? "🟢" : isBear ? "🔴" : "🟡"} {t("stockAnalysis.workflow.decisionTitle")}：{data.action}
          </span>
        }
      >
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "4px 16px", fontSize: 12 }}>
          <span>
            {t("stockAnalysis.workflow.positionPct")}: <b>{data.positionPct ?? "N/A"}%</b>
          </span>
          <span>
            {t("stockAnalysis.workflow.targetPrice")}: <b>¥{data.targetPrice ?? "N/A"}</b>
          </span>
          <span>
            {t("stockAnalysis.workflow.stopLoss")}: <b>¥{data.stopLoss ?? "N/A"}</b>
          </span>
          <span>
            {t("stockAnalysis.workflow.confidence")}: <b>{data.confidence ?? "N/A"}%</b>
          </span>
          <span style={{ gridColumn: "1 / -1" }}>
            {t("stockAnalysis.workflow.riskLevel")}:{" "}
            <Tag
              color={data.riskLevel === t("stockAnalysis.risk.high")
                ? "red"
                : data.riskLevel === t("stockAnalysis.risk.medium")
                ? "orange"
                : "green"}
            >
              {data.riskLevel ?? "N/A"}
            </Tag>
          </span>
        </div>
        {data.reasoning && (
          <div style={{ fontSize: 12, color: "var(--muted)", marginTop: 6, lineHeight: 1.5 }}>
            {data.reasoning}
          </div>
        )}
      </Card>
    );
  }

  return null;
}
