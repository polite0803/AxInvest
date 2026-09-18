// SPDX-License-Identifier: AGPL-3.0-only

import { isTauri, listen, logIpcError } from "@/lib/invoke";
import { create } from "zustand";

export type BackendTaskStatus = "running" | "completed" | "failed";

export interface BackendTask {
  id: string;
  type: string;
  label: string;
  status: BackendTaskStatus;
  progress?: number;
  detail?: string;
  startedAt: number;
  completedAt?: number;
}

interface BackendStatusState {
  tasks: BackendTask[];
  agentRunning: Record<string, boolean>;

  upsertTask: (task: BackendTask) => void;
  removeTask: (id: string) => void;
  setAgentRunning: (conversationId: string, running: boolean) => void;
  clearCompleted: () => void;
}

export const useBackendStatusStore = create<BackendStatusState>((set) => ({
  tasks: [],
  agentRunning: {},

  upsertTask: (task) => {
    set((state) => {
      const idx = state.tasks.findIndex((t) => t.id === task.id);
      if (idx >= 0) {
        const tasks = [...state.tasks];
        tasks[idx] = task;
        return { tasks };
      }
      return { tasks: [task, ...state.tasks].slice(0, 50) };
    });
  },

  removeTask: (id) => {
    set((state) => ({
      tasks: state.tasks.filter((t) => t.id !== id),
    }));
  },

  setAgentRunning: (conversationId, running) => {
    set((state) => ({
      agentRunning: { ...state.agentRunning, [conversationId]: running },
    }));
  },

  clearCompleted: () => {
    set((state) => ({
      tasks: state.tasks.filter((t) => t.status !== "completed" && t.status !== "failed"),
    }));
  },
}));

let _initialized = false;

export function initBackendStatusListeners() {
  if (_initialized || !isTauri()) { return; }
  _initialized = true;

  // 注意（P1-D，2026-09-12）：这三个事件的后端 payload 结构体
  // （`AgentStatusPayload` / `AgentDonePayload`，`commands/agent/payloads.rs`）都写了
  // `#[serde(rename = "conversationId")]`，即发到前端的 JSON key 是 **camelCase**。
  // 此前后端读的是 `conversation_id`（全项目仅存于 Rust 字段名），解析结果恒为
  // `undefined` ⇒ `agentRunning` 表被写成 `{ undefined: true }`，运行中标记从未生效。
  // 段 G 只查「事件名有无发射」查不出这类**字段名不匹配**，故一并在此显式说明。
  listen<{ conversationId: string }>("agent-started", (event) => {
    useBackendStatusStore.getState().setAgentRunning(event.payload.conversationId, true);
  }).catch(logIpcError("listen:agent-started"));

  listen<{ conversationId: string }>("agent-done", (event) => {
    useBackendStatusStore.getState().setAgentRunning(event.payload.conversationId, false);
  }).catch(logIpcError("listen:agent-done"));

  // `agent-status` 的 payload 是 `{ conversationId, phase, message, code }` ——
  // **没有 `status` 字段**，`phase` 才是阶段名（init/setup/running/done/error）。
  listen<{ conversationId: string; phase: string }>("agent-status", (event) => {
    const { conversationId, phase } = event.payload;
    if (phase === "cancelled" || phase === "error") {
      useBackendStatusStore.getState().setAgentRunning(conversationId, false);
    }
  }).catch(logIpcError("listen:agent-status"));

  listen<{ knowledgeBaseId: string; status: string; progress?: number }>("knowledge-base-updated", (event) => {
    const { knowledgeBaseId, status, progress } = event.payload;
    const store = useBackendStatusStore.getState();
    if (status === "indexing") {
      store.upsertTask({
        id: `kb-index-${knowledgeBaseId}`,
        type: "knowledge-indexing",
        label: `Indexing knowledge base`,
        status: "running",
        progress,
        startedAt: Date.now(),
      });
    } else {
      store.upsertTask({
        id: `kb-index-${knowledgeBaseId}`,
        type: "knowledge-indexing",
        label: `Indexing knowledge base`,
        status: status === "error" ? "failed" : "completed",
        startedAt: Date.now(),
        completedAt: Date.now(),
      });
    }
  }).catch(logIpcError("listen:knowledge-base-updated"));

  // 类型声明与实际 payload 对齐（`{ namespaceId }` —— 由 `index_queue.rs`
  // `emit_container_settled` 发射；此前声明为 snake_case，与后端不符）。
  listen<{ namespaceId: string }>("memory-rebuild-complete", () => {
    useBackendStatusStore.getState().upsertTask({
      id: "memory-rebuild",
      type: "memory-rebuild",
      label: "Rebuilding memory",
      status: "completed",
      startedAt: Date.now(),
      completedAt: Date.now(),
    });
  }).catch(logIpcError("listen:memory-rebuild-complete"));

  // 同理：`wiki.rs:529` 发射的是 `{ wikiId }`。
  listen<{ wikiId: string }>("wiki-rebuild-complete", () => {
    useBackendStatusStore.getState().upsertTask({
      id: "wiki-rebuild",
      type: "wiki-rebuild",
      label: "Rebuilding wiki",
      status: "completed",
      startedAt: Date.now(),
      completedAt: Date.now(),
    });
  }).catch(logIpcError("listen:wiki-rebuild-complete"));

  listen<{ baseId: string }>("knowledge-rebuild-complete", () => {
    useBackendStatusStore.getState().upsertTask({
      id: "kb-rebuild",
      type: "knowledge-rebuild",
      label: "Rebuilding knowledge base",
      status: "completed",
      startedAt: Date.now(),
      completedAt: Date.now(),
    });
  }).catch(logIpcError("listen:knowledge-rebuild-complete"));

  listen<{ execution_id: string; node_id: string; status: string }>("workflow:node-status-changed", (event) => {
    const { node_id: nodeId, status } = event.payload;
    const store = useBackendStatusStore.getState();
    if (status === "running") {
      store.upsertTask({
        id: `wf-node-${nodeId}`,
        type: "workflow-node",
        label: `Workflow: ${nodeId}`,
        status: "running",
        startedAt: Date.now(),
      });
    } else if (status === "completed" || status === "skipped") {
      store.upsertTask({
        id: `wf-node-${nodeId}`,
        type: "workflow-node",
        label: `Workflow: ${nodeId}`,
        status: "completed",
        startedAt: Date.now(),
        completedAt: Date.now(),
      });
    } else if (status === "error" || status === "failed") {
      store.upsertTask({
        id: `wf-node-${nodeId}`,
        type: "workflow-node",
        label: `Workflow: ${nodeId}`,
        status: "failed",
        startedAt: Date.now(),
        completedAt: Date.now(),
      });
    }
  }).catch(logIpcError("listen:workflow:node-status-changed"));

  listen<{ execution_id: string; status: string }>("workflow:execution-completed", (event) => {
    const { execution_id: executionId, status } = event.payload;
    useBackendStatusStore.getState().upsertTask({
      id: `wf-exec-${executionId}`,
      type: "workflow-execution",
      label: "Workflow execution",
      status: status === "error" || status === "failed" ? "failed" : "completed",
      startedAt: Date.now(),
      completedAt: Date.now(),
    });
  }).catch(logIpcError("listen:workflow:execution-completed"));
}
