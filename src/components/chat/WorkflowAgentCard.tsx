/**
 * 工作流 Agent 卡片 — 在对话页中渲染工作流执行状态。
 *
 * 三种卡片类型：
 * - progress: 进度条卡片（当前阶段 + 完成数/总数）
 * - analyst:  分析师完成卡片（报告摘要）
 * - decision: 最终决策卡片（action/仓位/目标价/止损）
 */
import { Card, Progress, Tag } from "antd";
import { TrendingUp } from "lucide-react";

// ── 卡片数据类型 ──

export interface WorkflowCardData {
  type: "progress" | "analyst" | "decision";
  // progress
  phase?: string;
  completed?: number;
  total?: number;
  // analyst
  analystName?: string;
  analystReport?: string;
  // decision
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

/** 构建 workflow 标记内容 */
export function makeWorkflowContent(type: string, data: Record<string, unknown>, fallback: string): string {
  return `<!-- workflow-${type}:${JSON.stringify(data)} -->${fallback}`;
}

// ── 分析师名称映射 ──

const ANALYST_LABELS: Record<string, string> = {
  "market-analyst": "技术面分析师",
  "sentiment-analyst": "情绪面分析师",
  "news-analyst": "消息面分析师",
  "fundamentals-analyst": "基本面分析师",
  "policy-analyst": "政策面分析师",
  "hot-money-tracker": "资金面追踪师",
  "lockup-watcher": "限售观察师",
  "research-analyst": "研报分析师",
  "sector-analyst": "板块分析师",
};

// ── 阶段名称映射 ──

const PHASE_LABELS: Record<string, string> = {
  "trigger": "启动工作流",
  "t-market-data": "获取K线数据",
  "t-sentiment-data": "获取新闻数据",
  "t-news-data": "获取新闻数据",
  "t-fundamentals-data": "获取财务数据",
  "t-policy-data": "获取新闻数据",
  "t-hotmoney-data": "获取资金流向",
  "t-lockup-data": "获取财务数据",
  "t-research-data": "获取新闻数据",
  "t-sector-data": "获取行情数据",
  "a-market-analyst": "技术面分析",
  "a-sentiment": "情绪面分析",
  "a-news": "消息面分析",
  "a-fundamentals": "基本面分析",
  "a-policy": "政策面分析",
  "a-hot-money": "资金面分析",
  "a-lockup": "限售分析",
  "a-research": "研报分析",
  "a-sector": "板块分析",
  "bull-r1": "多方辩论 R1",
  "bear-r1": "空方辩论 R1",
  "bull-r2": "多方辩论 R2",
  "bear-r2": "空方辩论 R2",
  "bull-r3": "多方辩论 R3",
  "bear-r3": "空方辩论 R3",
  "risk-agg": "激进风险评估",
  "risk-con": "保守风险评估",
  "risk-neu": "中性风险评估",
  "t-scoring": "技术评分算法",
  "t-valuation": "估值算法",
  "t-risk": "组合风险评估",
  "t-quality": "质量门控",
  "research-mgr": "研究经理综合",
  "trader": "交易方案制定",
  "portfolio-mgr": "最终决策",
};

// ── 组件 ──

export function WorkflowAgentCard({ data }: { data: WorkflowCardData }) {
  if (data.type === "progress") {
    const pct = data.total && data.total > 0
      ? Math.round(((data.completed ?? 0) / data.total) * 100)
      : 0;
    const phaseLabel = data.phase ? (PHASE_LABELS[data.phase] || data.phase) : "初始化";
    return (
      <div className="workflow-card" style={{ padding: "12px 16px" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
          <TrendingUp size={16} style={{ color: "var(--accent)" }} />
          <span style={{ fontSize: 13, fontWeight: 600 }}>
            🔍 A股多维度分析
          </span>
          <Tag color="processing" style={{ fontSize: 11 }}>进行中</Tag>
        </div>
        <Progress percent={pct} size="small" status="active" />
        <div style={{ fontSize: 12, color: "var(--muted)", marginTop: 4 }}>
          当前: {phaseLabel} ({data.completed ?? 0}/{data.total ?? "?"})
        </div>
      </div>
    );
  }

  if (data.type === "analyst") {
    const label = ANALYST_LABELS[data.analystName ?? ""] || data.analystName || "分析师";
    const brief = data.analystReport
      ? data.analystReport.slice(0, 200) + (data.analystReport.length > 200 ? "..." : "")
      : "分析完毕，请在分析页查看详细报告";
    return (
      <div className="workflow-card" style={{ padding: "10px 14px" }}>
        <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 4 }}>📊 {label}</div>
        <div style={{ fontSize: 12, color: "var(--muted)", lineHeight: 1.6 }}>{brief}</div>
      </div>
    );
  }

  if (data.type === "decision") {
    const isBull = data.action === "买入" || data.action === "增持";
    return (
      <Card
        size="small"
        style={{ borderColor: isBull ? "var(--sa-red)" : "var(--sa-green)" }}
        title={
          <span>
            {isBull ? "🟢" : data.action === "卖出" || data.action === "减持" ? "🔴" : "🟡"} 最终决策：{data.action}
          </span>
        }
      >
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "4px 16px", fontSize: 12 }}>
          <span>
            仓位: <b>{data.positionPct ?? "N/A"}%</b>
          </span>
          <span>
            目标价: <b>¥{data.targetPrice ?? "N/A"}</b>
          </span>
          <span>
            止损价: <b>¥{data.stopLoss ?? "N/A"}</b>
          </span>
          <span>
            置信度: <b>{data.confidence ?? "N/A"}%</b>
          </span>
          <span style={{ gridColumn: "1 / -1" }}>
            风险:{" "}
            <Tag color={data.riskLevel === "高" ? "red" : data.riskLevel === "中" ? "orange" : "green"}>
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
