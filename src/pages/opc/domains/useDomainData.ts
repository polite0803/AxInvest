// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 行业数据管理 Hook — 提供行业数据加载和操作
 *
 * 注意：所有 invoke 调用使用 camelCase 参数名（Tauri v2 IPC 默认 rename_all=camelCase）
 */

import { translateBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import type { DomainLearningConfig } from "@/types";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  AutomationRuleInfo,
  DomainDashboard,
  DomainDashboardResponse,
  DomainLearningMetrics,
  DomainManifest,
  OpcDomainDecision,
  WorkflowExecutionResult,
  WorkflowStepInfo,
} from "./types";

/**
 * 域级取数分区。
 *
 * 每个分区独立记录错误：失败前先把该分区的 `errors[section]` 置为**用户可见文本**，
 * 组件据此渲染错误态而非空态 —— 「拿不到」与「确实没有」在 UI 上必须分开。
 */
export type DomainDataSection =
  | "dashboard"
  | "steps"
  | "rules"
  | "metrics"
  | "learningConfig"
  | "decision";

/** 行业数据 Hook 返回值 */
export interface UseDomainDataReturn {
  // 状态
  loading: boolean;
  manifest: DomainManifest | null;
  learningConfig: DomainLearningConfig | null;
  learningLoading: boolean;
  dashboard: DomainDashboard | null;
  dashboardLoading: boolean;
  workflowSteps: WorkflowStepInfo[];
  stepsLoading: boolean;
  automationRules: AutomationRuleInfo[];
  rulesLoading: boolean;
  rulesRunning: boolean;
  kpiTimeRange: "7" | "30" | "90";
  decision: OpcDomainDecision | null;
  decisionLoading: boolean;
  decisionDays: number;
  workflowResult: WorkflowExecutionResult | null;
  workflowExecuting: boolean;
  learningMetrics: DomainLearningMetrics | null;
  metricsLoading: boolean;
  /** 逐分区取数失败原因（已译）；键缺席 = 该分区本轮无错 */
  errors: Partial<Record<DomainDataSection, string>>;

  // 操作
  setKpiTimeRange: (range: "7" | "30" | "90") => void;
  setDecisionDays: (days: number) => void;
  loadDashboard: () => Promise<void>;
  loadWorkflowSteps: () => Promise<void>;
  loadAutomationRules: () => Promise<void>;
  loadDecision: () => Promise<void>;
  loadLearningMetrics: () => Promise<void>;
  loadLearningConfig: () => Promise<void>;
  runAutomationRules: () => Promise<string[]>;
  executeWorkflow: (workflowId: string, userInput?: Record<string, unknown>) => Promise<WorkflowExecutionResult>;
  reflectOnWorkflow: (workflowId?: string) => Promise<void>;
  evolveWorkflow: (workflowId?: string, reason?: string) => Promise<void>;
  runSelfImprovement: (target?: string) => Promise<void>;
}

/**
 * 行业数据管理 Hook
 * @param capabilityPackId 行业 ID
 * @returns 行业数据和操作方法
 */
export function useDomainData(capabilityPackId: string | null): UseDomainDataReturn {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [manifest, setManifest] = useState<DomainManifest | null>(null);
  const [learningConfig, setLearningConfig] = useState<DomainLearningConfig | null>(null);
  const [learningLoading, setLearningLoading] = useState(true);
  const [dashboard, setDashboard] = useState<DomainDashboard | null>(null);
  // ⚠ 取数标志初值为 true：「尚未得到答案」与「答案是没有」是两件事 —— 初值若为 false，
  // 首帧会在加载 effect 跑起来之前渲染出 `Empty`（「暂无数据」/「配置未找到」），
  // 即把「还没问」显示成「后端说没有」。
  const [dashboardLoading, setDashboardLoading] = useState(true);
  const [workflowSteps, setWorkflowSteps] = useState<WorkflowStepInfo[]>([]);
  const [stepsLoading, setStepsLoading] = useState(true);
  const [automationRules, setAutomationRules] = useState<AutomationRuleInfo[]>([]);
  const [rulesLoading, setRulesLoading] = useState(true);
  const [rulesRunning, setRulesRunning] = useState(false);
  const [kpiTimeRange, setKpiTimeRange] = useState<"7" | "30" | "90">("30");
  const [decision, setDecision] = useState<OpcDomainDecision | null>(null);
  const [decisionLoading, setDecisionLoading] = useState(false);
  const [decisionDays, setDecisionDays] = useState(30);
  const [workflowResult, setWorkflowResult] = useState<WorkflowExecutionResult | null>(null);
  const [workflowExecuting, setWorkflowExecuting] = useState(false);
  const [learningMetrics, setLearningMetrics] = useState<DomainLearningMetrics | null>(null);
  const [metricsLoading, setMetricsLoading] = useState(true);
  const [errors, setErrors] = useState<Partial<Record<DomainDataSection, string>>>({});

  const setSectionError = useCallback((section: DomainDataSection, reason: string | null) => {
    setErrors((prev) => {
      if (reason === null) {
        if (!(section in prev)) {
          return prev;
        }
        const next = { ...prev };
        delete next[section];
        return next;
      }
      return { ...prev, [section]: reason };
    });
  }, []);

  // 加载行业清单
  useEffect(() => {
    if (!capabilityPackId) {
      setLoading(false);
      return;
    }

    const loadDomain = async () => {
      setLoading(true);
      try {
        const result = await invoke<{ manifest: DomainManifest }>(
          "opc_get_capability_pack",
          { capabilityPackId },
        );
        setManifest(result.manifest);
      } catch (e) {
        console.error("[useDomainData] load failed:", e);
      } finally {
        setLoading(false);
      }
    };

    loadDomain();
  }, [capabilityPackId]);

  // 加载仪表盘
  const loadDashboard = useCallback(async () => {
    if (!capabilityPackId) {
      setDashboardLoading(false);
      return;
    }
    setDashboardLoading(true);
    try {
      const days = Number(kpiTimeRange);
      // 后端返回信封 { capabilityPackId, dashboard }，仪表盘本体在 dashboard 字段内
      const result = await invoke<DomainDashboardResponse>(
        "opc_get_capability_pack_dashboard",
        { capabilityPackId, days },
      );
      setDashboard(result?.dashboard ?? null);
      setSectionError("dashboard", null);
    } catch (e) {
      console.error("[useDomainData] load dashboard failed:", e);
      setSectionError("dashboard", translateBackendError(e));
    } finally {
      setDashboardLoading(false);
    }
  }, [capabilityPackId, kpiTimeRange, setSectionError]);

  // 加载工作流步骤
  const loadWorkflowSteps = useCallback(async () => {
    if (!capabilityPackId) {
      setStepsLoading(false);
      return;
    }
    setStepsLoading(true);
    try {
      const result = await invoke<{ steps: WorkflowStepInfo[] }>(
        "opc_get_capability_pack_workflow_steps",
        { capabilityPackId },
      );
      setWorkflowSteps(result.steps || []);
      setSectionError("steps", null);
    } catch (e) {
      console.error("[useDomainData] load workflow steps failed:", e);
      setWorkflowSteps([]);
      setSectionError("steps", translateBackendError(e));
    } finally {
      setStepsLoading(false);
    }
  }, [capabilityPackId, setSectionError]);

  // 加载自动化规则
  const loadAutomationRules = useCallback(async () => {
    if (!capabilityPackId) {
      setRulesLoading(false);
      return;
    }
    setRulesLoading(true);
    try {
      const result = await invoke<{ rules: AutomationRuleInfo[] }>(
        "opc_get_capability_pack_automation_rules",
        { capabilityPackId },
      );
      setAutomationRules(result.rules || []);
      setSectionError("rules", null);
    } catch (e) {
      console.error("[useDomainData] load automation rules failed:", e);
      setAutomationRules([]);
      setSectionError("rules", translateBackendError(e));
    } finally {
      setRulesLoading(false);
    }
  }, [capabilityPackId, setSectionError]);

  // 加载决策（使用 opc_execute_analysis 命令）
  const loadDecision = useCallback(async () => {
    if (!capabilityPackId) {
      return;
    }
    setDecisionLoading(true);
    try {
      const result = await invoke<OpcDomainDecision>("opc_execute_analysis", {
        capabilityPackId,
        days: decisionDays,
      });
      setDecision(result);
      setSectionError("decision", null);
    } catch (e) {
      console.error("[useDomainData] load decision failed:", e);
      setDecision(null);
      setSectionError("decision", translateBackendError(e));
    } finally {
      setDecisionLoading(false);
    }
  }, [capabilityPackId, decisionDays, setSectionError]);

  // 加载学习指标（使用 opc_get_learning_metrics 命令）
  const loadLearningMetrics = useCallback(async () => {
    if (!capabilityPackId) {
      setMetricsLoading(false);
      return;
    }
    setMetricsLoading(true);
    try {
      const result = await invoke<DomainLearningMetrics>(
        "opc_get_learning_metrics",
        { capabilityPackId },
      );
      setLearningMetrics(result);
      setSectionError("metrics", null);
    } catch (e) {
      console.error("[useDomainData] load learning metrics failed:", e);
      setLearningMetrics(null);
      setSectionError("metrics", translateBackendError(e));
    } finally {
      setMetricsLoading(false);
    }
  }, [capabilityPackId, setSectionError]);

  // 加载学习配置（使用 opc_get_learning_config 命令）
  const loadLearningConfig = useCallback(async () => {
    if (!capabilityPackId) {
      setLearningLoading(false);
      return;
    }
    setLearningLoading(true);
    try {
      const result = await invoke<DomainLearningConfig>(
        "opc_get_learning_config",
        { capabilityPackId },
      );
      setLearningConfig(result);
      setSectionError("learningConfig", null);
    } catch (e) {
      console.error("[useDomainData] load learning config failed:", e);
      setLearningConfig(null);
      setSectionError("learningConfig", translateBackendError(e));
    } finally {
      setLearningLoading(false);
    }
  }, [capabilityPackId, setSectionError]);

  /**
   * 执行自动化规则。
   *
   * **不吞异常**：调用方需要区分「跑完但没规则命中」与「根本没跑成」——
   * 原实现在 catch 里 `return []`，把失败伪装成「没有规则被触发」。
   */
  const runAutomationRules = useCallback(async (): Promise<string[]> => {
    if (!capabilityPackId) {
      return [];
    }
    setRulesRunning(true);
    try {
      return await invoke<string[]>("opc_run_automation_rules", {
        capabilityPackId,
        entityType: "customer",
        entityId: "manual_trigger",
      });
    } finally {
      setRulesRunning(false);
    }
  }, [capabilityPackId]);

  // 执行工作流（使用 opc_execute_workflow 命令，传递 workflow_id + capability_pack_id + days + userInput）
  const executeWorkflow = useCallback(
    async (workflowId: string, userInput?: Record<string, unknown>): Promise<WorkflowExecutionResult> => {
      setWorkflowExecuting(true);
      try {
        const result = await invoke<WorkflowExecutionResult>("opc_execute_workflow", {
          capabilityPackId,
          workflowId,
          days: 30,
          userInput: userInput ?? null,
        });
        setWorkflowResult(result);
        return result;
      } catch (e) {
        const errorResult: WorkflowExecutionResult = {
          workflow_id: workflowId,
          status: "failed",
          steps_completed: 0,
          steps_total: 0,
          error: String(e),
          duration_ms: 0,
        };
        setWorkflowResult(errorResult);
        return errorResult;
      } finally {
        setWorkflowExecuting(false);
      }
    },
    [capabilityPackId],
  );

  // 反思（使用 opc_reflect_on_workflow 命令，需要 workflow_id + workflow_result）
  const reflectOnWorkflow = useCallback(
    async (workflowId?: string) => {
      if (!capabilityPackId) {
        return;
      }
      try {
        const wfId = workflowId || `default_${capabilityPackId}`;
        const wfResult = workflowResult || { status: "completed", steps_completed: 0, steps_total: 0 };
        await invoke("opc_reflect_on_workflow", {
          capabilityPackId,
          workflowId: wfId,
          workflowResult: wfResult,
        });
        await loadLearningMetrics();
      } catch (e) {
        console.error("[useDomainData] reflect failed:", e);
      }
    },
    [capabilityPackId, workflowResult, loadLearningMetrics],
  );

  // 进化（使用 opc_evolve_workflow 命令，需要 workflow_id + reason）
  const evolveWorkflow = useCallback(
    async (workflowId?: string, reason?: string) => {
      if (!capabilityPackId) {
        return;
      }
      try {
        const wfId = workflowId || `default_${capabilityPackId}`;
        const reasonText = reason || t("opc.domain.learning.evolution.defaultReason");
        await invoke("opc_evolve_workflow", {
          capabilityPackId,
          workflowId: wfId,
          reason: reasonText,
        });
        await loadLearningMetrics();
      } catch (e) {
        console.error("[useDomainData] evolve failed:", e);
      }
    },
    [capabilityPackId, loadLearningMetrics, t],
  );

  // 自我改进（使用 opc_run_self_improvement 命令，需要 target）
  const runSelfImprovement = useCallback(
    async (target?: string) => {
      if (!capabilityPackId) {
        return;
      }
      try {
        const targetText = target || "all";
        await invoke("opc_run_self_improvement", {
          capabilityPackId,
          target: targetText,
        });
        await loadLearningMetrics();
      } catch (e) {
        console.error("[useDomainData] self improve failed:", e);
      }
    },
    [capabilityPackId, loadLearningMetrics],
  );

  // 初始化加载：只随能力包切换重跑。
  // ⚠ 不得把 loadDashboard 并入本 effect —— 它依赖 kpiTimeRange，一旦入列，
  // 切换 KPI 区间会连带把步骤/规则/指标/学习配置全部重拉一遍（4 次多余 IPC）。
  useEffect(() => {
    if (!capabilityPackId) {
      return;
    }
    loadWorkflowSteps();
    loadAutomationRules();
    loadLearningMetrics();
    loadLearningConfig();
  }, [
    capabilityPackId,
    loadWorkflowSteps,
    loadAutomationRules,
    loadLearningMetrics,
    loadLearningConfig,
  ]);

  // 仪表盘取数：挂载时与 KPI 区间变化时各一次（初始化 effect 不再重复发起）
  useEffect(() => {
    if (!capabilityPackId) {
      return;
    }
    loadDashboard();
  }, [capabilityPackId, kpiTimeRange, loadDashboard]);

  return {
    loading,
    manifest,
    learningConfig,
    learningLoading,
    dashboard,
    dashboardLoading,
    workflowSteps,
    stepsLoading,
    automationRules,
    rulesLoading,
    rulesRunning,
    kpiTimeRange,
    setKpiTimeRange,
    decision,
    decisionLoading,
    decisionDays,
    setDecisionDays,
    workflowResult,
    workflowExecuting,
    learningMetrics,
    metricsLoading,
    errors,
    loadDashboard,
    loadWorkflowSteps,
    loadAutomationRules,
    loadDecision,
    loadLearningMetrics,
    loadLearningConfig,
    runAutomationRules,
    executeWorkflow,
    reflectOnWorkflow,
    evolveWorkflow,
    runSelfImprovement,
  };
}
