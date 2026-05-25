/**
 * 工作流→对话消息桥接。
 *
 * 当用户输入 @股票代码 启动分析后，工作流事件双端消费：
 * - 对话页：通过本模块将事件转换为 system 消息（标记为 workflow-* 卡片）
 * - 分析页：stockAnalysisStore 独立监听同一事件（现有逻辑）
 */
import { invoke, listen } from "@/lib/invoke";
import type { UnlistenFn } from "@/lib/invoke";

const activeBridges = new Map<string, UnlistenFn[]>();

function makeMarkdown(type: string, data: Record<string, unknown>, fallback: string): string {
  return `<!-- workflow-${type}:${JSON.stringify(data)} -->${fallback}`;
}

/** 启动桥接 */
export async function startStockWorkflowChatBridge(conversationId: string): Promise<void> {
  stopStockWorkflowChatBridge(conversationId);

  const unlisteners: UnlistenFn[] = [];
  let progressMsgId: string | null = null;
  const completedNodes = new Set<string>();

  // 插入初始进度消息
  try {
    const m = await invoke<{ id: string }>("send_system_message", {
      conversationId,
      content: makeMarkdown("progress", {
        phase: "trigger",
        completed: 0,
        total: 30,
      }, "🔍 正在启动 A 股多维度分析..."),
    });
    progressMsgId = m.id;
  } catch { /* 静默 */ }

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
        content: makeMarkdown("progress", {
          phase: nodeId,
          completed: count,
          total: totalNodes,
        }, `🔍 分析进行中 (${count}/${totalNodes})`),
      }).catch(() => {});
    }

    if (nodeId.startsWith("a-") && !nodeId.includes("bull") && !nodeId.includes("bear")) {
      const name = nodeId.replace("a-", "");
      invoke("send_system_message", {
        conversationId,
        content: makeMarkdown("analyst", { analystName: name, analystReport: "" }, `📊 ${name} 分析完成`),
      }).catch(() => {});
    }
  });
  unlisteners.push(u1);

  // 监听工作流完成
  const u2 = await listen<{
    workflowId: string;
    results: Record<string, unknown>;
    output?: Record<string, unknown> | null;
  }>("workflow-completed", async (event) => {
    const output = event.payload.output;

    if (progressMsgId) {
      invoke("update_message_content", {
        id: progressMsgId,
        content: makeMarkdown("progress", {
          phase: "done",
          completed: completedNodes.size,
          total: completedNodes.size,
        }, "✅ 分析完成"),
      }).catch(() => {});
    }

    if (output && typeof output === "object") {
      invoke("send_system_message", {
        conversationId,
        content: makeMarkdown("decision", {
          action: String(output.action ?? "N/A"),
          positionPct: Number(output.positionPct ?? 0),
          targetPrice: Number(output.targetPrice ?? 0),
          stopLoss: Number(output.stopLoss ?? 0),
          reasoning: String(output.reasoning ?? ""),
          riskLevel: String(output.riskLevel ?? "N/A"),
          confidence: Number(output.confidence ?? 0),
        }, `最终决策：${output.action} | 仓位${output.positionPct}%`),
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
          content: `## ❌ 分析失败\n\n> ${event.payload.error}`,
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
      } catch { /* 静默 */ }
    }
    activeBridges.delete(conversationId);
  }
}
