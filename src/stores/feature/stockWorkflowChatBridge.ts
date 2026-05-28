/**
 * 工作流→对话消息桥接。
 *
 * 当用户输入 $股票代码 启动分析后，工作流事件双端消费：
 * - 对话页：通过本模块将事件转换为 system 消息（标记为 workflow-* 卡片）
 * - 分析页：stockAnalysisStore 独立监听同一事件（现有逻辑）
 */
import { makeWorkflowContent } from "@/components/chat/WorkflowAgentCard";
import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";
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

function wf(type: string, data: Record<string, unknown>, fallback: string): string {
  return makeWorkflowContent(type, data, fallback);
}

/** 启动桥接 */
export async function startStockWorkflowChatBridge(conversationId: string): Promise<void> {
  stopStockWorkflowChatBridge(conversationId);

  const unlisteners: UnlistenFn[] = [];
  let progressMsgId: string | null = null;
  const completedNodes = new Set<string>();
  const pendingAnalystCards = new Map<string, string>(); // msg_id → nodeId

  // 插入初始进度消息
  try {
    const m = await invoke<{ id: string }>("send_system_message", {
      conversationId,
      content: wf(
        "progress",
        { phase: "trigger", completed: 0, total: 30 },
        i18next.t("stockAnalysis.workflow.title") + "...",
      ),
    });
    progressMsgId = m.id;
  } catch { /* silent */ }

  // 监听工作流步骤完成
  const u1 = await listen<{
    workflowId: string;
    nodeId: string;
    status: string;
    totalNodes: number;
    completedNodes: number;
  }>("workflow-step-done", async (event) => {
    const { nodeId, totalNodes } = event.payload;
    completedNodes.add(nodeId);
    const count = completedNodes.size;

    if (progressMsgId) {
      invoke("update_message_content", {
        id: progressMsgId,
        content: wf(
          "progress",
          { phase: nodeId, completed: count, total: totalNodes },
          `${i18next.t("stockAnalysis.workflow.inProgress")} (${count}/${totalNodes})`,
        ),
      }).catch(() => {});
    }

    if (nodeId.startsWith("a-") && !nodeId.includes("bull") && !nodeId.includes("bear")) {
      const analystName = ANALYST_NODE_TO_NAME[nodeId] || nodeId.replace("a-", "");
      // 先发占位消息，等工作流完成时在 results 中查找报告内容回填
      const msg = await invoke<{ id: string }>("send_system_message", {
        conversationId,
        content: wf(
          "analyst",
          { analystName, analystReport: "", nodeId },
          "📊 " + i18next.t("stockAnalysis.workflow.analystComplete"),
        ),
      }).catch(() => null);
      if (msg) {
        pendingAnalystCards.set(msg.id, nodeId);
      }
    }
  });
  unlisteners.push(u1);

  // 监听工作流完成
  const u2 = await listen<{
    workflowId: string;
    results: Record<string, unknown>;
    output?: Record<string, unknown> | null;
  }>("workflow-completed", async (event) => {
    const { results, output } = event.payload;

    if (progressMsgId) {
      invoke("update_message_content", {
        id: progressMsgId,
        content: wf(
          "progress",
          { phase: "done", completed: completedNodes.size, total: completedNodes.size },
          `✅ ${i18next.t("stockAnalysis.workflow.phase.done")}`,
        ),
      }).catch(() => {});
    }

    // 回填分析师报告内容：从 results 中提取 "report.xxx" 键
    for (const [msgId, nodeId] of pendingAnalystCards) {
      const analystName = ANALYST_NODE_TO_NAME[nodeId] || nodeId.replace("a-", "");
      const reportKey = `report.${analystName}`;
      const resultsObj: Record<string, unknown> = results ?? {};
      const reportText: string = (resultsObj[reportKey] as string)
        || (Object.entries(resultsObj).find(([k]) => k.includes(analystName)) ?? [])[1] as string
        || "";
      if (reportText) {
        invoke("update_message_content", {
          id: msgId,
          content: wf(
            "analyst",
            { analystName, analystReport: reportText, nodeId },
            "📊 " + i18next.t("stockAnalysis.workflow.analystComplete"),
          ),
        }).catch(() => {});
      }
    }
    pendingAnalystCards.clear();

    if (output && typeof output === "object") {
      invoke("send_system_message", {
        conversationId,
        content: wf(
          "decision",
          {
            action: String(output.action ?? "N/A"),
            positionPct: Number(output.positionPct ?? 0),
            targetPrice: Number(output.targetPrice ?? 0),
            stopLoss: Number(output.stopLoss ?? 0),
            reasoning: String(output.reasoning ?? ""),
            riskLevel: String(output.riskLevel ?? "N/A"),
            confidence: Number(output.confidence ?? 0),
          },
          `${i18next.t("stockAnalysis.workflow.decisionTitle")}: ${output.action} | ${
            i18next.t("stockAnalysis.workflow.positionPct")
          }${output.positionPct}%`,
        ),
      }).catch(() => {});
    }

    stopStockWorkflowChatBridge(conversationId);
  });
  unlisteners.push(u2);

  // 监听错误
  const u3 = await listen<{ workflowId: string; error: string }>(
    "workflow-error",
    async (event) => {
      if (progressMsgId) {
        invoke("update_message_content", {
          id: progressMsgId,
          content: wf(
            "progress",
            { phase: "error", completed: completedNodes.size, total: completedNodes.size },
            `❌ ${i18next.t("stockAnalysis.workflow.phase.error")}: ${event.payload.error}`,
          ),
        }).catch(() => {});
      }
      stopStockWorkflowChatBridge(conversationId);
    },
  );
  unlisteners.push(u3);

  activeBridges.set(conversationId, unlisteners);
}

/** 停止桥接 */
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
