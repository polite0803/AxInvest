// SPDX-License-Identifier: AGPL-3.0-only

import { t } from "@/lib/i18nStoreHelper";
import { invoke, listen } from "@/lib/invoke";
import type { CapabilityGapProposal, EvolutionEvidenceView, RuntimeToolInfo } from "@/types";
import { create } from "zustand";

// ── Types ──

export interface EngineLog {
  timestamp: number;
  level: "info" | "warn" | "error";
  message: string;
}

export interface EngineStatus {
  name: string;
  displayName: string;
  description: string;
  category: "core" | "learning" | "safety" | "experimental";
  running: boolean;
  config: Record<string, unknown>;
  stats: Record<string, unknown>;
  logs: EngineLog[];
  lastActive?: number;
}

export interface SkillVersion {
  version: number;
  timestamp: number;
  summary: string;
  metrics: Record<string, { before: number; after: number }>;
  promptDiff?: { old: string; new: string };
}

export interface ABTestResult {
  metric: string;
  valueA: number;
  valueB: number;
  /** (B-A)/A×100(B 相对 A 的提升,与 winner 推导自洽);A=0 ⇒ null ⇒ 平局(除零保护) */
  change: number | null;
  /** per-metric winner,由 change 正负推导(>0 ⇒ B 胜,<0 ⇒ A 胜,=0/null ⇒ tie) */
  winner: "A" | "B" | "tie";
  /** 后端整体 winner(report.winner 字符串)原样透出,不与 per-metric 推导混算 */
  overallWinner?: string;
}

export interface EvolutionEvent {
  engine: string;
  timestamp: number;
  type: "started" | "stopped" | "evolved" | "config_changed" | "error";
  detail: string;
}

// ── Mock Data ──

const ENGINE_DEFINITIONS: Omit<EngineStatus, "running" | "config" | "stats" | "logs" | "lastActive">[] = [
  {
    name: "skill_evolution",
    displayName: t("evolution.engines.skill_evolution.name"),
    description: t("evolution.engines.skill_evolution.desc"),
    category: "core",
  },
  {
    name: "auto_tool_creator",
    displayName: t("evolution.engines.auto_tool_creator.name"),
    description: t("evolution.engines.auto_tool_creator.desc"),
    category: "core",
  },
  {
    name: "text_grad",
    displayName: t("evolution.engines.text_grad.name"),
    description: t("evolution.engines.text_grad.desc"),
    category: "core",
  },
  {
    name: "constitution",
    displayName: t("evolution.engines.constitution.name"),
    description: t("evolution.engines.constitution.desc"),
    category: "safety",
  },
  {
    name: "intrinsic_motivation",
    displayName: t("evolution.engines.intrinsic_motivation.name"),
    description: t("evolution.engines.intrinsic_motivation.desc"),
    category: "learning",
  },
  {
    name: "coevolution",
    displayName: t("evolution.engines.coevolution.name"),
    description: t("evolution.engines.coevolution.desc"),
    category: "learning",
  },
  {
    name: "dream_consolidator",
    displayName: t("evolution.engines.dream_consolidator.name"),
    description: t("evolution.engines.dream_consolidator.desc"),
    category: "learning",
  },
  {
    name: "process_reward",
    displayName: t("evolution.engines.process_reward.name"),
    description: t("evolution.engines.process_reward.desc"),
    category: "learning",
  },
  {
    name: "sandbox",
    displayName: t("evolution.engines.sandbox.name"),
    description: t("evolution.engines.sandbox.desc"),
    category: "safety",
  },
];

function buildDefaultConfig(engineName: string): Record<string, unknown> {
  const configs: Record<string, Record<string, unknown>> = {
    skill_evolution: {
      evolutionRate: 0.01,
      minImprovement: 0.05,
      maxVersions: 10,
      populationSize: 20,
      generations: 50,
      mutationRate: 0.1,
      crossoverRate: 0.7,
      autoRollback: true,
      requireApproval: true,
    },
    auto_tool_creator: {
      minPatternFrequency: 3,
      similarityThreshold: 0.8,
      maxToolsPerSession: 5,
      requireConfirmation: true,
      toolComplexityLimit: "medium",
    },
    text_grad: {
      learningRate: 0.01,
      momentum: 0.9,
      maxIterations: 100,
      convergenceThreshold: 0.001,
      batchSize: 8,
      optimizer: "adam",
    },
    constitution: {
      strictMode: true,
      allowOverrides: false,
      rulePriority: "high",
      auditLog: true,
      maxRuleCount: 50,
    },
    intrinsic_motivation: {
      curiosityWeight: 0.3,
      noveltyThreshold: 0.5,
      explorationDecay: 0.99,
      maxExplorationBudget: 1000,
    },
    coevolution: {
      maxConcurrentAgents: 5,
      knowledgeShareInterval: 60000,
      competitionRatio: 0.3,
      elitismCount: 2,
    },
    dream_consolidator: {
      consolidationInterval: 3600000,
      batchSize: 32,
      memoryRetention: 0.9,
      replayRatio: 0.2,
    },
    process_reward: {
      discountFactor: 0.95,
      stepPenalty: 0.01,
      successBonus: 1.0,
      failurePenalty: -0.5,
    },
    sandbox: {
      timeoutMs: 30000,
      maxMemoryMB: 512,
      networkAccess: false,
      fileSystemAccess: "readonly",
      allowedLanguages: ["python", "javascript"],
    },
  };
  return configs[engineName] ?? {};
}

function buildDefaultStats(engineName: string): Record<string, unknown> {
  const stats: Record<string, Record<string, unknown>> = {
    skill_evolution: {
      totalEvolutions: 42,
      activeSkills: 12,
      avgImprovement: "8.3%",
      lastEvolution: Date.now() - 3600000,
    },
    auto_tool_creator: {
      toolsCreated: 7,
      patternsDetected: 23,
      avgConfidence: "87%",
      lastCreated: Date.now() - 7200000,
    },
    text_grad: { nodes: 156, gradients: 1280, iterations: 5000, lossReduction: "34%" },
    constitution: { rules: 18, violations: 3, enforcementRate: "99.7%", lastViolation: Date.now() - 86400000 },
    intrinsic_motivation: { explorationScore: 0.72, noveltyCount: 45, activeDrives: 3, energyLevel: "85%" },
    coevolution: { activeTasks: 2, agentsInPool: 8, knowledgeTransfers: 156, avgFitness: 0.68 },
    dream_consolidator: { knowledgeEntries: 2048, lastConsolidation: Date.now() - 1800000, retentionRate: "94%" },
    process_reward: { accuracy: "82%", stepsEvaluated: 15000, avgStepScore: 0.65, activeModels: 2 },
    sandbox: { totalExecutions: 324, successRate: "96%", avgExecutionMs: 450, lastExecution: Date.now() - 600000 },
  };
  return stats[engineName] ?? {};
}

function buildDefaultLogs(engineName: string): EngineLog[] {
  const now = Date.now();
  return [
    { timestamp: now - 300000, level: "info", message: `[${engineName}] Engine initialized successfully` },
    { timestamp: now - 240000, level: "info", message: `[${engineName}] Configuration loaded` },
    { timestamp: now - 180000, level: "info", message: `[${engineName}] Starting background tasks` },
    { timestamp: now - 120000, level: "info", message: `[${engineName}] Health check passed` },
    { timestamp: now - 60000, level: "info", message: `[${engineName}] Idle, waiting for triggers` },
  ];
}

function buildMockEngines(): Record<string, EngineStatus> {
  const engines: Record<string, EngineStatus> = {};
  for (const def of ENGINE_DEFINITIONS) {
    engines[def.name] = {
      ...def,
      running: def.category === "core" || def.category === "safety",
      config: buildDefaultConfig(def.name),
      stats: buildDefaultStats(def.name),
      logs: buildDefaultLogs(def.name),
      lastActive: Date.now() - Math.floor(Math.random() * 3600000),
    };
  }
  return engines;
}

// ── A/B 测试结果(任务 #218 批一:接真 evaluator_get_ab_results,不再回退写死 mock) ──

/** 后端 AbTestReport DTO(权威定义:src-tauri/src/commands/evaluator.rs,serde camelCase 输出) */
export interface AbTestReportDTO {
  testId: string;
  skillId: string;
  versionA: string;
  versionB: string;
  /** 后端整体 winner("A"/"B" 字符串),前端原样透出,不参与 per-metric 推导 */
  winner: string;
  metrics: { name: string; valueA: number; valueB: number; unit: string }[];
  conclusion: string;
}

/**
 * A/B 适配器(任务 #218 裁定口径):
 * - change% = (B-A)/A×100(B 相对 A 的提升,与 winner 推导自洽;旧 mock 的 (A-B)/B×100 方向反,已废弃)
 * - A=0 ⇒ change=null ⇒ 平局
 * - per-metric winner 由 change 正负推导:>0 ⇒ B 胜,<0 ⇒ A 胜,=0/null ⇒ tie
 * - overall winner(后端 report.winner)原样透出,不与 per-metric 混算
 */
export function adaptAbTestReport(report: AbTestReportDTO | null): ABTestResult[] {
  if (!report || !Array.isArray(report.metrics)) {
    return [];
  }
  return report.metrics.map((m) => {
    const change = m.valueA === 0 ? null : ((m.valueB - m.valueA) / m.valueA) * 100;
    const winner: ABTestResult["winner"] = change === null || change === 0
      ? "tie"
      : change > 0
      ? "B"
      : "A";
    return {
      metric: m.name,
      valueA: m.valueA,
      valueB: m.valueB,
      change,
      winner,
      overallWinner: report.winner,
    };
  });
}

// ── Store ──

interface EvolutionState {
  engines: Record<string, EngineStatus>;
  evolutionHistory: EvolutionEvent[];
  loading: boolean;
  error: string | null;

  // ── 能力补齐/进化改进提议同意弹窗（T0.13） ──
  /** 挂起的提议（按 proposalId 索引），有值即弹窗可见 */
  pendingConsent: Record<string, CapabilityGapProposal>;

  /** 处理后端 evolution-consent-request 事件 */
  handleConsentRequest: (proposal: CapabilityGapProposal) => void;
  /** 用户同意/拒绝后调用，回传结果给后端 */
  respondConsent: (proposalId: string, approved: boolean) => Promise<void>;
  /** 清理已处理的提议 */
  clearConsent: (proposalId: string) => void;

  fetchAllEngineStatus: () => Promise<void>;
  startEngine: (name: string) => Promise<void>;
  stopEngine: (name: string) => Promise<void>;
  updateEngineConfig: (name: string, config: Record<string, unknown>) => Promise<void>;
  fetchEngineLogs: (name: string) => Promise<void>;
  getSkillEvolutionHistory: (skillId: string) => Promise<SkillVersion[]>;
  getABTestResults: (skillId: string) => Promise<ABTestResult[]>;
  triggerSkillEvolution: (skillId: string) => Promise<void>;
  addEvolutionEvent: (event: EvolutionEvent) => void;

  // ── 阶段二 T2.5：运行时动态工具管理 ──

  /** 运行时动态注册的工具列表（来源 runtime_evolution / system_evolution） */
  runtimeTools: RuntimeToolInfo[];
  /** 刷新运行时工具列表（调 list_runtime_tools） */
  listRuntimeTools: () => Promise<RuntimeToolInfo[]>;
  /** 卸载一个运行时动态注册的工具（调 unregister_runtime_tool） */
  unregisterRuntimeTool: (name: string) => Promise<void>;

  // ── T5A.4：进化证据视图（认知编排器决策标签流 → 贝叶斯后验） ──

  /** 最近一次拉取的进化证据视图（按会话缓存，null 表示未加载） */
  evolutionEvidence: EvolutionEvidenceView | null;
  /** 是否正在拉取进化证据 */
  evidenceLoading: boolean;
  /** 拉取指定会话的进化证据（调 cognitive_evolution_decision） */
  fetchEvolutionEvidence: (conversationId: string) => Promise<EvolutionEvidenceView | null>;
}

export const useEvolutionStore = create<EvolutionState>((set, get) => ({
  engines: buildMockEngines(),
  evolutionHistory: [],
  loading: false,
  error: null,
  pendingConsent: {},
  runtimeTools: [],
  evolutionEvidence: null,
  evidenceLoading: false,

  // ── 能力补齐/进化改进提议同意（T0.13） ──

  handleConsentRequest: (proposal) => {
    set((s) => ({
      pendingConsent: { ...s.pendingConsent, [proposal.id]: proposal },
    }));
  },

  respondConsent: async (proposalId, approved) => {
    try {
      await invoke("resolve_capability_gap", {
        proposalId,
        approved,
      });
    } catch (e) {
      console.warn("[evolutionStore] respondConsent failed", e);
    } finally {
      get().clearConsent(proposalId);
    }
  },

  clearConsent: (proposalId) => {
    set((s) => {
      const rest = { ...s.pendingConsent };
      delete rest[proposalId];
      return { pendingConsent: rest };
    });
  },

  fetchAllEngineStatus: async () => {
    set({ loading: true, error: null });
    try {
      const statuses = await invoke<Record<string, EngineStatus>>("get_all_engine_status");
      set({ engines: statuses, loading: false });
    } catch (err) {
      console.warn("[evolutionStore] fetchAllEngineStatus failed, using mock data", err);
      // Keep existing mock data, just mark loading done
      set({ loading: false });
    }
  },

  startEngine: async (name: string) => {
    try {
      await invoke("start_engine", { engineName: name });
      set((state) => ({
        engines: {
          ...state.engines,
          [name]: { ...state.engines[name], running: true, lastActive: Date.now() },
        },
      }));
      get().addEvolutionEvent({
        engine: name,
        timestamp: Date.now(),
        type: "started",
        detail: `Engine ${name} started`,
      });
    } catch (err) {
      console.warn("[evolutionStore] startEngine failed, using mock", err);
      set((state) => ({
        engines: {
          ...state.engines,
          [name]: { ...state.engines[name], running: true, lastActive: Date.now() },
        },
      }));
      get().addEvolutionEvent({
        engine: name,
        timestamp: Date.now(),
        type: "started",
        detail: `Engine ${name} started (mock)`,
      });
    }
  },

  stopEngine: async (name: string) => {
    try {
      await invoke("stop_engine", { engineName: name });
      set((state) => ({
        engines: {
          ...state.engines,
          [name]: { ...state.engines[name], running: false },
        },
      }));
      get().addEvolutionEvent({
        engine: name,
        timestamp: Date.now(),
        type: "stopped",
        detail: `Engine ${name} stopped`,
      });
    } catch (err) {
      console.warn("[evolutionStore] stopEngine failed, using mock", err);
      set((state) => ({
        engines: {
          ...state.engines,
          [name]: { ...state.engines[name], running: false },
        },
      }));
      get().addEvolutionEvent({
        engine: name,
        timestamp: Date.now(),
        type: "stopped",
        detail: `Engine ${name} stopped (mock)`,
      });
    }
  },

  updateEngineConfig: async (name: string, config: Record<string, unknown>) => {
    try {
      await invoke("update_engine_config", { engineName: name, config });
      set((state) => ({
        engines: {
          ...state.engines,
          [name]: { ...state.engines[name], config: { ...state.engines[name].config, ...config } },
        },
      }));
      get().addEvolutionEvent({
        engine: name,
        timestamp: Date.now(),
        type: "config_changed",
        detail: "Configuration updated",
      });
    } catch (err) {
      console.warn("[evolutionStore] updateEngineConfig failed, using mock", err);
      set((state) => ({
        engines: {
          ...state.engines,
          [name]: { ...state.engines[name], config: { ...state.engines[name].config, ...config } },
        },
      }));
    }
  },

  fetchEngineLogs: async (name: string) => {
    try {
      const logs = await invoke<EngineLog[]>("get_engine_logs", { engineName: name, limit: 50 });
      set((state) => ({
        engines: { ...state.engines, [name]: { ...state.engines[name], logs } },
      }));
    } catch (err) {
      console.warn("[evolutionStore] fetchEngineLogs failed, using mock", err);
      // Keep existing mock logs
    }
  },

  // 任务 #218 批一:getter 异步化,返回空数组,不回退写死 mock(空态是诚实行为)。
  getSkillEvolutionHistory: async (_skillId: string): Promise<SkillVersion[]> => {
    // 批二:history 持久化轨迹(引擎已捕获 old/new content 但命令层丢弃,需持久化表+命令)。
    return [];
  },

  // 任务 #218 批一:接真命令 evaluator_get_ab_results(写入键为 skill_id,来自 run_ab_test,
  // 查询键必须同键空间)。空串 = 无技能绑定(节点→技能绑定关系盘上不存在,已回报纠正,
  // 等 leader 改裁)⇒ 短路返回空态,不发注定为空的查询;绑定落地后调用方改传真 skillId 即生效。
  getABTestResults: async (skillId: string): Promise<ABTestResult[]> => {
    if (!skillId) {
      return [];
    }
    try {
      const report = await invoke<AbTestReportDTO | null>("evaluator_get_ab_results", { skillId });
      return adaptAbTestReport(report);
    } catch (e) {
      console.warn("[evolutionStore] getABTestResults failed", e);
      return []; // 失败 ⇒ 空态,不再回退假数据
    }
  },

  triggerSkillEvolution: async (_skillId: string) => {
    // 调用真进化命令 skill_evolution_start（调用 SkillEvolutionEngine::run 执行遗传算法）
    // 注意：trigger_skill_evolution 是伪触发（只写日志），已废弃删除
    try {
      const result = await invoke<{
        skill_id: string;
        improved: boolean;
        reason: string;
        confidence: number;
        quality_delta?: number;
      }>("skill_evolution_start", { skillId: _skillId });

      get().addEvolutionEvent({
        engine: "skill_evolution",
        timestamp: Date.now(),
        type: result.improved ? "evolved" : "error",
        detail: result.improved
          ? `Skill ${_skillId} evolved: ${result.reason} (confidence: ${(result.confidence * 100).toFixed(1)}%)`
          : `Skill ${_skillId} evolution skipped: ${result.reason}`,
      });
    } catch (err) {
      console.warn("[evolutionStore] triggerSkillEvolution failed", err);
      get().addEvolutionEvent({
        engine: "skill_evolution",
        timestamp: Date.now(),
        type: "error",
        detail: `Skill ${_skillId} evolution failed: ${err}`,
      });
    }
  },

  addEvolutionEvent: (event: EvolutionEvent) => {
    set((state) => ({
      evolutionHistory: [...state.evolutionHistory.slice(-199), event],
    }));
  },

  // ── 阶段二 T2.5：运行时动态工具管理 ──

  listRuntimeTools: async () => {
    try {
      const result = await invoke<{ success: boolean; count: number; tools: RuntimeToolInfo[] }>(
        "list_runtime_tools",
      );
      const tools = result?.tools ?? [];
      set({ runtimeTools: tools });
      return tools;
    } catch (e) {
      console.warn("[evolutionStore] listRuntimeTools failed", e);
      // 浏览器 mock 模式：展示自指工具静态列表
      const mock: RuntimeToolInfo[] = [
        { name: "system_evolution_inspect", source: "system_evolution" },
        { name: "system_evolution_define", source: "system_evolution" },
        { name: "system_evolution_deploy", source: "system_evolution" },
        { name: "system_evolution_undeploy", source: "system_evolution" },
      ];
      set({ runtimeTools: mock });
      return mock;
    }
  },

  unregisterRuntimeTool: async (name: string) => {
    try {
      await invoke("unregister_runtime_tool", { toolName: name });
      // 本地同步移除
      set((s) => ({ runtimeTools: s.runtimeTools.filter((tool) => tool.name !== name) }));
      get().addEvolutionEvent({
        engine: "auto_tool_creator",
        timestamp: Date.now(),
        type: "stopped",
        detail: `Runtime tool ${name} unregistered`,
      });
    } catch (e) {
      console.warn("[evolutionStore] unregisterRuntimeTool failed", e);
      throw e;
    }
  },

  // ── T5A.4：进化证据视图 ──

  fetchEvolutionEvidence: async (conversationId) => {
    set({ evidenceLoading: true });
    try {
      const view = await invoke<EvolutionEvidenceView>("cognitive_evolution_decision", {
        conversationId,
      });
      set({ evolutionEvidence: view, evidenceLoading: false });
      return view;
    } catch (e) {
      console.warn("[evolutionStore] fetchEvolutionEvidence failed", e);
      set({ evolutionEvidence: null, evidenceLoading: false });
      return null;
    }
  },
}));

// ── 事件监听（T0.13） ──
// 模块顶层监听能力补齐/进化改进提议事件（仿照 agentStore 顶层兜底模式），
// 避免依赖组件 useEffect 的挂载时机导致事件丢失。
// 后端 `await_user_consent`（cognitive.rs）emit 本事件后阻塞等待前端回传；
// 前端弹窗同意/拒绝后由 `respondConsent` → `capability_gap_consent` 命令回传。
listen<CapabilityGapProposal>("evolution-consent-request", (event) => {
  useEvolutionStore.getState().handleConsentRequest(event.payload);
});
