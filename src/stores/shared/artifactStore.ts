// SPDX-License-Identifier: AGPL-3.0-only

import type { ExecutionResult } from "@/lib/codeExecutor";
import { invoke, logIpcError } from "@/lib/invoke";
import type { Artifact, ArtifactPreviewMode, CreateArtifactInput, UpdateArtifactInput } from "@/types";
import { create } from "zustand";

interface ArtifactState {
  artifacts: Artifact[];
  loading: boolean;
  error: string | null;
  previewArtifact: Artifact | null;
  previewMode: ArtifactPreviewMode;
  executionResults: Record<string, ExecutionResult>;

  loadArtifacts: (conversationId: string) => Promise<void>;
  createArtifact: (input: CreateArtifactInput) => Promise<Artifact | null>;
  updateArtifact: (
    id: string,
    input: UpdateArtifactInput,
  ) => Promise<Artifact | null>;
  deleteArtifact: (id: string) => Promise<void>;
  pinArtifact: (id: string, pinned: boolean) => Promise<void>;
  clearArtifacts: () => void;
  setPreviewArtifact: (artifact: Artifact | null) => void;
  setPreviewMode: (mode: ArtifactPreviewMode) => void;
  executeCode: (artifactId: string) => Promise<ExecutionResult | null>;
  duplicateArtifact: (id: string) => Promise<Artifact | null>;
}

export const useArtifactStore = create<ArtifactState>((set, get) => ({
  artifacts: [],
  loading: false,
  error: null,
  previewArtifact: null,
  previewMode: "code",
  executionResults: {},

  loadArtifacts: async (conversationId) => {
    set({ loading: true, error: null });
    try {
      const artifacts = await invoke<Artifact[]>("list_artifacts", {
        conversationId: conversationId,
      });
      set({ artifacts, loading: false });
    } catch (e) {
      logIpcError("artifactStore: loadArtifacts failed")(e);
      set({ error: String(e), loading: false });
    }
  },

  createArtifact: async (input) => {
    try {
      // 后端 create_artifact(state, input: CreateArtifactInput)
      // （src-tauri/src/commands/artifacts.rs:36-38）⇒ 参数必须包在 `input` 键下。
      // 改前把载荷摊平到顶层，真机 IPC 会以 `missing required key input` 直接拒；
      // dev mock 侧则表现为「假成功 + 空字段孤儿产物」（#201 B 档改判）。
      const artifact = await invoke<Artifact>("create_artifact", { input });
      set((s) => ({ artifacts: [...s.artifacts, artifact] }));
      return artifact;
    } catch (e) {
      logIpcError("artifactStore: operation failed")(e);
      set({ error: String(e) });
      return null;
    }
  },

  updateArtifact: async (id, input) => {
    try {
      // 后端 update_artifact(state, id: String, input: UpdateArtifactInput)
      // （src-tauri/src/commands/artifacts.rs:55-59）⇒ id 与 input 是两个独立参数，
      // 载荷不能摊平进顶层（同 createArtifact，#201 B 档改判）。
      const updated = await invoke<Artifact>("update_artifact", {
        id,
        input,
      });
      set((s) => ({
        artifacts: s.artifacts.map((a) => (a.id === id ? updated : a)),
        previewArtifact: s.previewArtifact?.id === id ? updated : s.previewArtifact,
      }));
      return updated;
    } catch (e) {
      logIpcError("artifactStore: operation failed")(e);
      set({ error: String(e) });
      return null;
    }
  },

  deleteArtifact: async (id) => {
    try {
      await invoke("delete_artifact", { id });
      set((s) => ({
        artifacts: s.artifacts.filter((a) => a.id !== id),
        previewArtifact: s.previewArtifact?.id === id ? null : s.previewArtifact,
      }));
    } catch (e) {
      set({ error: String(e) });
    }
  },

  pinArtifact: async (id, pinned) => {
    const { updateArtifact } = get();
    await updateArtifact(id, { pinned });
  },

  clearArtifacts: () => set({ artifacts: [], error: null, previewArtifact: null }),

  setPreviewArtifact: (artifact) => set({ previewArtifact: artifact }),

  setPreviewMode: (mode) => set({ previewMode: mode }),

  executeCode: async (artifactId) => {
    const { artifacts } = get();
    const artifact = artifacts.find((a) => a.id === artifactId);
    if (!artifact) {
      return null;
    }

    const startTime = performance.now();
    try {
      const result = await invoke<ExecutionResult>("execute_sandbox", {
        code: artifact.content,
        // 后端 `Artifact`（harness rag_voice_etc.rs:628-637）没有 `language` 列，
        // 语言口径就是 `format`（#210 摘掉 TS 侧的幽灵字段后，这里不再读它）。
        language: artifact.format,
      });
      const executionResult = {
        ...result,
        duration_ms: performance.now() - startTime,
      };
      set((s) => ({
        executionResults: {
          ...s.executionResults,
          [artifactId]: executionResult,
        },
      }));
      return executionResult;
    } catch (e) {
      const errorResult: ExecutionResult = {
        stdout: "",
        stderr: String(e),
        exit_code: -1,
        duration_ms: performance.now() - startTime,
      };
      set((s) => ({
        executionResults: { ...s.executionResults, [artifactId]: errorResult },
      }));
      return errorResult;
    }
  },

  duplicateArtifact: async (id) => {
    const { artifacts, createArtifact } = get();
    const original = artifacts.find((a) => a.id === id);
    if (!original) {
      return null;
    }

    return createArtifact({
      conversationId: original.conversationId,
      kind: original.kind,
      title: `${original.title} (copy)`,
      content: original.content,
      format: original.format,
    });
  },
}));
