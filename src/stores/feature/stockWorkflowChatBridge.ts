import { makeWorkflowContent, type WorkflowCardData } from "@/components/chat/WorkflowAgentCard";
import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
import { useConversationStore } from "@/stores/domain/conversationStore";
import type { Message } from "@/types";
import i18next from "i18next";

const activeBridges = new Map<string, UnlistenFn[]>();

const ANALYST_NODE_TO_NAME: Record<string, string> = {
  "a-market-analyst": "market-analyst",
  "a-sentiment": "sentiment-analyst",
  "a-news": "news-analyst",
  "a-fundamentals": "fundamentals-analyst",
  "a-policy": "policy-analyst",
  "a-hot-money": "hot-money-tracker",
  "a-lockup": "lockup-watcher",
  "a-research": "research-analyst",
  "a-sector": "sector-analyst",
};

const TOOL_NODE_TO_LABEL: Record<string, string> = {
  "t-market-data": "市场行情",
  "t-kline-data": "K线数据",
  "t-financial-data": "财务数据",
  "t-news-data": "新闻资讯",
  "t-money-flow": "资金流向",
  "t-sentiment-data": "市场情绪",
  "t-policy-data": "政策数据",
  "t-hot-money-data": "游资数据",
  "t-lockup-data": "解禁数据",
  "t-research-data": "研报数据",
  "t-sector-data": "板块数据",
  "t-scoring": "技术评分",
  "t-valuation": "估值计算",
  "t-portfolio-risk": "组合风险",
  "t-peers-data": "同业可比",
  "t-option-data": "期权数据",
  "t-index-data": "大盘指数",
  "t-announcement-data": "公司公告",
  "t-northbound-data": "北向资金",
  "t-dragon-tiger-data": "龙虎榜",
  "t-cls-flash-data": "财联社快讯",
  "t-block-trade-data": "大宗交易",
  "t-institutional-data": "机构调研",
  "t-consensus-data": "一致预期",
  "t-concept-data": "概念板块",
};

function wf(type: string, data: Record<string, unknown>, fallback: string): string {
  return makeWorkflowContent(type, data, fallback);
}

function appendMessageToStore(conversationId: string, msg: Message) {
  const store = useConversationStore.getState();
  if (store.activeConversationId !== conversationId) {
    return;
  }
  const exists = store.messages.some((m) => m.id === msg.id);
  if (exists) {
    return;
  }
  useConversationStore.setState((s) => ({
    messages: [...s.messages, msg],
  }));
}

function updateMessageInStore(messageId: string, content: string) {
  const store = useConversationStore.getState();
  const exists = store.messages.some((m) => m.id === messageId);
  if (!exists) {
    return;
  }
  useConversationStore.setState((s) => ({
    messages: s.messages.map((m) => m.id === messageId ? { ...m, content } : m),
  }));
}

function extractContent(output: unknown): string {
  if (output == null) { return ""; }
  if (typeof output === "string") { return output; }
  try {
    return JSON.stringify(output, null, 2);
  } catch {
    return String(output);
  }
}

function summarizeToolResult(result: unknown): string {
  if (result == null) { return ""; }
  if (typeof result === "string") {
    try {
      return summarizeParsed(JSON.parse(result));
    } catch {
      return result.length > 120 ? result.slice(0, 120) + "..." : result;
    }
  }
  return summarizeParsed(result);
}

function summarizeParsed(v: unknown): string {
  if (Array.isArray(v)) {
    if (v.length === 0) { return "0 条记录"; }
    const first = v[0];
    if (first && typeof first === "object") {
      const keys = Object.keys(first as Record<string, unknown>);
      return `${v.length} 条记录 (${keys.slice(0, 3).join(", ")}${keys.length > 3 ? "..." : ""})`;
    }
    return `${v.length} 条记录`;
  }
  if (v && typeof v === "object") {
    const obj = v as Record<string, unknown>;
    if (obj.stockName && obj.price) { return `${obj.stockName} ¥${obj.price}`; }
    if (obj.stockCode && obj.totalScore) { return `评分 ${obj.totalScore}`; }
    if (obj.stockCode && obj.dcf) { return `DCF ¥${(obj.dcf as Record<string, unknown>)?.intrinsicValue ?? "N/A"}`; }
    if (obj.items && Array.isArray(obj.items)) { return `${(obj.items as unknown[]).length} 条记录`; }
    if (obj.news && Array.isArray(obj.news)) { return `${(obj.news as unknown[]).length} 条新闻`; }
    if (obj.data && Array.isArray(obj.data)) { return `${(obj.data as unknown[]).length} 条数据`; }
    const keys = Object.keys(obj);
    if (keys.length <= 4) { return keys.map((k) => `${k}: ${JSON.stringify(obj[k]).slice(0, 30)}`).join(", "); }
    return `${keys.length} 个字段`;
  }
  return String(v).slice(0, 80);
}

interface DataSourceEntry {
  nodeId: string;
  toolName: string;
  label: string;
  status: "pending" | "fetching" | "success" | "failed";
  error?: string;
  summary?: string;
}

export async function startStockWorkflowChatBridge(conversationId: string): Promise<void> {
  stopStockWorkflowChatBridge(conversationId);

  const unlisteners: UnlistenFn[] = [];
  let aggregateMsgId: string | null = null;

  const analystsMap = new Map(
    Object.entries(ANALYST_NODE_TO_NAME).map(([nodeId, name]) => [
      nodeId,
      {
        nodeId,
        name,
        status: "pending" as "pending" | "running" | "done" | "failed",
        report: undefined as string | undefined,
      },
    ]),
  );

  const debatesMap = new Map([
    ["bull-researcher", {
      key: "bull-researcher",
      label: "多方研究员",
      status: "pending" as "pending" | "running" | "done" | "failed",
      rounds: [] as string[],
    }],
    ["bear-researcher", {
      key: "bear-researcher",
      label: "空方研究员",
      status: "pending" as "pending" | "running" | "done" | "failed",
      rounds: [] as string[],
    }],
  ]);

  const risksMap = new Map([
    ["risk-agg", {
      key: "risk-agg",
      label: "激进评估",
      status: "pending" as "pending" | "running" | "done" | "failed",
      content: undefined as string | undefined,
    }],
    ["risk-con", {
      key: "risk-con",
      label: "保守评估",
      status: "pending" as "pending" | "running" | "done" | "failed",
      content: undefined as string | undefined,
    }],
    ["risk-neu", {
      key: "risk-neu",
      label: "中性评估",
      status: "pending" as "pending" | "running" | "done" | "failed",
      content: undefined as string | undefined,
    }],
  ]);

  const extraNodesMap = new Map([
    ["agg-risk", {
      key: "agg-risk",
      label: "风险聚合",
      status: "pending" as "pending" | "running" | "done" | "failed",
    }],
    ["cls-risk-level", {
      key: "cls-risk-level",
      label: "风险分级",
      status: "pending" as "pending" | "running" | "done" | "failed",
    }],
    ["notify-result", {
      key: "notify-result",
      label: "结果通知",
      status: "pending" as "pending" | "running" | "done" | "failed",
    }],
  ]);

  const dataSourcesMap = new Map<string, DataSourceEntry>();
  const completedNodes = new Set<string>();
  let finalDecision: WorkflowCardData | null = null;

  const buildAggregateContent = (
    phase: string,
    status: "running" | "done" | "error",
    totalNodes: number,
    error?: string,
  ) => {
    const analysts = Array.from(analystsMap.values());
    const debates = Array.from(debatesMap.values());
    const risks = Array.from(risksMap.values());
    const dataSources = Array.from(dataSourcesMap.values());
    const extraNodes = Array.from(extraNodesMap.values());
    const count = completedNodes.size;
    return wf(
      "aggregate",
      {
        phase,
        completed: count,
        total: totalNodes,
        analysts,
        debates,
        risks,
        extraNodes,
        dataSources,
        status,
        error,
        decision: finalDecision,
      },
      `${i18next.t("stockAnalysis.workflow.title")} (${count}/${totalNodes})`,
    );
  };

  try {
    const m = await invoke<Message>("send_system_message", {
      conversationId,
      content: buildAggregateContent("trigger", "running", 30),
    });
    aggregateMsgId = m.id;
    appendMessageToStore(conversationId, m);
  } catch { /* silent */ }

  const u1 = await listen<{
    workflowId: string;
    nodeId: string;
    status: string;
    totalNodes: number;
    completedNodes: number;
    output?: unknown;
  }>("workflow-step-done", async (event) => {
    const { nodeId, totalNodes, status, output } = event.payload;
    if (status === "completed" || status === "failed") {
      completedNodes.add(nodeId);
    }

    // ── Tool 节点 (t- 前缀)：数据源信息 ──
    if (nodeId.startsWith("t-")) {
      if (!dataSourcesMap.has(nodeId)) {
        let toolName = "";
        let label = TOOL_NODE_TO_LABEL[nodeId] ?? nodeId.slice(2);

        if (output != null && typeof output === "object") {
          const out = output as Record<string, unknown>;
          if (out.tool_name && typeof out.tool_name === "string") {
            toolName = out.tool_name;
          }
        }

        dataSourcesMap.set(nodeId, { nodeId, toolName, label, status: "pending" });
      }

      const ds = dataSourcesMap.get(nodeId)!;
      if (status === "running") {
        ds.status = "fetching";
      } else if (status === "completed") {
        ds.status = "success";
        if (output != null && typeof output === "object") {
          const out = output as Record<string, unknown>;
          if (out.tool_name && typeof out.tool_name === "string" && !ds.toolName) {
            ds.toolName = out.tool_name;
          }
          if (out.result != null) {
            ds.summary = summarizeToolResult(out.result);
          } else {
            ds.summary = summarizeToolResult(output);
          }
        } else if (output != null) {
          ds.summary = summarizeToolResult(output);
        }
      } else if (status === "failed") {
        ds.status = "failed";
        if (output != null) {
          ds.error = typeof output === "string" ? output : JSON.stringify(output);
        } else {
          ds.error = "数据获取失败";
        }
      }
    }

    // ── Agent 节点 (a- 前缀)：分析师报告 ──
    if (analystsMap.has(nodeId)) {
      const analyst = analystsMap.get(nodeId)!;
      if (status === "running") {
        analyst.status = "running";
      } else if (status === "completed") {
        analyst.status = "done";
        if (output != null) {
          analyst.report = extractContent(output);
        }
      } else if (status === "failed") {
        analyst.status = "failed";
      }
    }

    // ── 辩论节点 (DebateNode 子节点: bull-researcher / bear-researcher) ──
    if (debatesMap.has(nodeId)) {
      const debater = debatesMap.get(nodeId)!;
      if (status === "running") {
        debater.status = "running";
      } else if (status === "completed") {
        debater.status = "done";
        if (output != null) {
          debater.rounds.push(extractContent(output));
        }
      } else if (status === "failed") {
        debater.status = "failed";
      }
    }

    // ── 风险评估节点 (ParallelNode 子节点) ──
    if (risksMap.has(nodeId)) {
      const risk = risksMap.get(nodeId)!;
      if (status === "running") {
        risk.status = "running";
      } else if (status === "completed") {
        risk.status = "done";
        if (output != null) {
          risk.content = extractContent(output);
        }
      } else if (status === "failed") {
        risk.status = "failed";
      }
    }

    // ── 新增节点 (Aggregator / LlmClassifier / Notification) ──
    if (extraNodesMap.has(nodeId)) {
      const node = extraNodesMap.get(nodeId)!;
      if (status === "running") {
        node.status = "running";
      } else if (status === "completed") {
        node.status = "done";
      } else if (status === "failed") {
        node.status = "failed";
      }
    }

    // ── 更新聚合卡片 ──
    if (aggregateMsgId) {
      const newContent = buildAggregateContent(nodeId, "running", totalNodes);
      invoke("update_message_content", {
        id: aggregateMsgId,
        content: newContent,
      }).catch(() => {});
      updateMessageInStore(aggregateMsgId, newContent);
    }
  });
  unlisteners.push(u1);

  const u2 = await listen<{
    workflowId: string;
    results: Record<string, unknown>;
    output?: Record<string, unknown> | null;
  }>("workflow-completed", async (event) => {
    const { output } = event.payload;

    if (output && typeof output === "object") {
      finalDecision = {
        type: "decision",
        action: String(output.action ?? "N/A"),
        positionPct: Number(output.positionPct ?? 0),
        targetPrice: Number(output.targetPrice ?? 0),
        stopLoss: Number(output.stopLoss ?? 0),
        reasoning: String(output.reasoning ?? ""),
        riskLevel: String(output.riskLevel ?? "N/A"),
        confidence: Number(output.confidence ?? 0),
      };
    }

    if (aggregateMsgId) {
      const newContent = buildAggregateContent("done", "done", completedNodes.size);
      invoke("update_message_content", {
        id: aggregateMsgId,
        content: newContent,
      }).catch(() => {});
      updateMessageInStore(aggregateMsgId, newContent);
    }

    stopStockWorkflowChatBridge(conversationId);
  });
  unlisteners.push(u2);

  const u3 = await listen<{ workflowId: string; error: string }>(
    "workflow-error",
    async (event) => {
      if (aggregateMsgId) {
        const newContent = buildAggregateContent("error", "error", completedNodes.size, event.payload.error);
        invoke("update_message_content", {
          id: aggregateMsgId,
          content: newContent,
        }).catch(() => {});
        updateMessageInStore(aggregateMsgId, newContent);
      }
      stopStockWorkflowChatBridge(conversationId);
    },
  );
  unlisteners.push(u3);

  activeBridges.set(conversationId, unlisteners);
}

export function stopStockWorkflowChatBridge(conversationId: string): void {
  const unlisteners = activeBridges.get(conversationId);
  if (unlisteners) {
    for (const u of unlisteners) {
      try {
        u();
      } catch { /* silent */ }
    }
    activeBridges.delete(conversationId);
  }
}
