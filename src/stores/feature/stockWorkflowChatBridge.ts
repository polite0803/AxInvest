import { makeWorkflowContent } from "@/components/chat/WorkflowAgentCard";
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

export async function startStockWorkflowChatBridge(conversationId: string): Promise<void> {
  stopStockWorkflowChatBridge(conversationId);

  const unlisteners: UnlistenFn[] = [];
  let progressMsgId: string | null = null;
  const completedNodes = new Set<string>();

  try {
    const m = await invoke<Message>("send_system_message", {
      conversationId,
      content: wf(
        "progress",
        { phase: "trigger", completed: 0, total: 30 },
        i18next.t("stockAnalysis.workflow.title") + "...",
      ),
    });
    progressMsgId = m.id;
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
    const count = completedNodes.size;

    if (progressMsgId) {
      const newContent = wf(
        "progress",
        { phase: nodeId, completed: count, total: totalNodes },
        `${i18next.t("stockAnalysis.workflow.inProgress")} (${count}/${totalNodes})`,
      );
      invoke("update_message_content", {
        id: progressMsgId,
        content: newContent,
      }).catch(() => {});
      updateMessageInStore(progressMsgId, newContent);
    }

    if (status === "completed" && nodeId.startsWith("a-") && !nodeId.includes("bull") && !nodeId.includes("bear")) {
      const analystName = ANALYST_NODE_TO_NAME[nodeId] || nodeId.replace("a-", "");
      const reportText = output != null
        ? (typeof output === "string" ? output : JSON.stringify(output, null, 2))
        : "";
      const msg = await invoke<Message>("send_system_message", {
        conversationId,
        content: wf(
          "analyst",
          { analystName, analystReport: reportText, nodeId },
          "📊 " + i18next.t("stockAnalysis.workflow.analystComplete"),
        ),
      }).catch(() => null);
      if (msg) {
        appendMessageToStore(conversationId, msg);
      }
    }
  });
  unlisteners.push(u1);

  const u2 = await listen<{
    workflowId: string;
    results: Record<string, unknown>;
    output?: Record<string, unknown> | null;
  }>("workflow-completed", async (event) => {
    const { output } = event.payload;

    if (progressMsgId) {
      const newContent = wf(
        "progress",
        { phase: "done", completed: completedNodes.size, total: completedNodes.size },
        `✅ ${i18next.t("stockAnalysis.workflow.phase.done")}`,
      );
      invoke("update_message_content", {
        id: progressMsgId,
        content: newContent,
      }).catch(() => {});
      updateMessageInStore(progressMsgId, newContent);
    }

    if (output && typeof output === "object") {
      const msg = await invoke<Message>("send_system_message", {
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
      }).catch(() => null);
      if (msg) {
        appendMessageToStore(conversationId, msg);
      }
    }

    stopStockWorkflowChatBridge(conversationId);
  });
  unlisteners.push(u2);

  const u3 = await listen<{ workflowId: string; error: string }>(
    "workflow-error",
    async (event) => {
      if (progressMsgId) {
        const newContent = wf(
          "progress",
          { phase: "error", completed: completedNodes.size, total: completedNodes.size },
          `❌ ${i18next.t("stockAnalysis.workflow.phase.error")}: ${event.payload.error}`,
        );
        invoke("update_message_content", {
          id: progressMsgId,
          content: newContent,
        }).catch(() => {});
        updateMessageInStore(progressMsgId, newContent);
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
