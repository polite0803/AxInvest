// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 行业数据管理 Hook — 提供行业数据加载和操作
 *
 * 注意：所有 invoke 调用使用 camelCase 参数名（Tauri v2 IPC 默认 rename_all=camelCase）
 */

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
 * @param domainPackId 行业 ID
 * @returns 行业数据和操作方法
 */
export function useDomainData(domainPackId: string | null): UseDomainDataReturn {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [manifest, setManifest] = useState<DomainManifest | null>(null);
  const [learningConfig, setLearningConfig] = useState<DomainLearningConfig | null>(null);
  const [learningLoading, setLearningLoading] = useState(false);
  const [dashboard, setDashboard] = useState<DomainDashboard | null>(null);
  const [dashboardLoading, setDashboardLoading] = useState(false);
  const [workflowSteps, setWorkflowSteps] = useState<WorkflowStepInfo[]>([]);
  const [stepsLoading, setStepsLoading] = useState(false);
  const [automationRules, setAutomationRules] = useState<AutomationRuleInfo[]>([]);
  const [rulesLoading, setRulesLoading] = useState(false);
  const [rulesRunning, setRulesRunning] = useState(false);
  const [kpiTimeRange, setKpiTimeRange] = useState<"7" | "30" | "90">("30");
  const [decision, setDecision] = useState<OpcDomainDecision | null>(null);
  const [decisionLoading, setDecisionLoading] = useState(false);
  const [decisionDays, setDecisionDays] = useState(30);
  const [workflowResult, setWorkflowResult] = useState<WorkflowExecutionResult | null>(null);
  const [workflowExecuting, setWorkflowExecuting] = useState(false);
  const [learningMetrics, setLearningMetrics] = useState<DomainLearningMetrics | null>(null);
  const [metricsLoading, setMetricsLoading] = useState(false);

  // 加载行业清单
  useEffect(() => {
    if (!domainPackId) {
      setLoading(false);
      return;
    }

    const loadDomain = async () => {
      setLoading(true);
      try {
        const result = await invoke<{ manifest: DomainManifest }>(
          "opc_get_domain_pack",
          { domainPackId },
        );
        setManifest(result.manifest);
      } catch (e) {
        console.error("[useDomainData] load failed:", e);
      } finally {
        setLoading(false);
      }
    };

    loadDomain();
  }, [domainPackId]);

  // 加载仪表盘
  const loadDashboard = useCallback(async () => {
    if (!domainPackId) {
      return;
    }
    setDashboardLoading(true);
    try {
      const days = Number(kpiTimeRange);
      // 后端返回信封 { domainPackId, dashboard }，仪表盘本体在 dashboard 字段内
      const result = await invoke<DomainDashboardResponse>(
        "opc_get_domain_pack_dashboard",
        { domainPackId, days },
      );
      setDashboard(result?.dashboard ?? null);
    } catch (e) {
      console.error("[useDomainData] load dashboard failed:", e);
    } finally {
      setDashboardLoading(false);
    }
  }, [domainPackId, kpiTimeRange]);

  // 加载工作流步骤
  const loadWorkflowSteps = useCallback(async () => {
    if (!domainPackId) {
      return;
    }
    setStepsLoading(true);
    try {
      const result = await invoke<{ steps: WorkflowStepInfo[] }>(
        "opc_get_domain_pack_workflow_steps",
        { domainPackId },
      );
      setWorkflowSteps(result.steps || []);
    } catch (e) {
      console.error("[useDomainData] load workflow steps failed:", e);
      setWorkflowSteps([]);
    } finally {
      setStepsLoading(false);
    }
  }, [domainPackId]);

  // 加载自动化规则
  const loadAutomationRules = useCallback(async () => {
    if (!domainPackId) {
      return;
    }
    setRulesLoading(true);
    try {
      const result = await invoke<{ rules: AutomationRuleInfo[] }>(
        "opc_get_domain_pack_automation_rules",
        { domainPackId },
      );
      setAutomationRules(result.rules || []);
    } catch (e) {
      console.error("[useDomainData] load automation rules failed:", e);
      setAutomationRules([]);
    } finally {
      setRulesLoading(false);
    }
  }, [domainPackId]);

  // 加载决策（使用 opc_execute_analysis 命令）
  const loadDecision = useCallback(async () => {
    if (!domainPackId) {
      return;
    }
    setDecisionLoading(true);
    try {
      const result = await invoke<OpcDomainDecision>("opc_execute_analysis", {
        domainPackId,
        days: decisionDays,
      });
      setDecision(result);
    } catch (e) {
      console.error("[useDomainData] load decision failed:", e);
    } finally {
      setDecisionLoading(false);
    }
  }, [domainPackId, decisionDays]);

  // 加载学习指标（使用 opc_get_learning_metrics 命令）
  const loadLearningMetrics = useCallback(async () => {
    if (!domainPackId) {
      return;
    }
    setMetricsLoading(true);
    try {
      const result = await invoke<DomainLearningMetrics>(
        "opc_get_learning_metrics",
        { domainPackId },
      );
      setLearningMetrics(result);
    } catch (e) {
      console.error("[useDomainData] load learning metrics failed:", e);
    } finally {
      setMetricsLoading(false);
    }
  }, [domainPackId]);

  // 加载学习配置（使用 opc_get_learning_config 命令）
  const loadLearningConfig = useCallback(async () => {
    if (!domainPackId) {
      return;
    }
    setLearningLoading(true);
    try {
      const result = await invoke<DomainLearningConfig>(
        "opc_get_learning_config",
        { domainPackId },
      );
      setLearningConfig(result);
    } catch (e) {
      console.error("[useDomainData] load learning config failed:", e);
    } finally {
      setLearningLoading(false);
    }
  }, [domainPackId]);

  // 执行自动化规则
  const runAutomationRules = useCallback(async (): Promise<string[]> => {
    if (!domainPackId) {
      return [];
    }
    setRulesRunning(true);
    try {
      const triggered = await invoke<string[]>("opc_run_automation_rules", {
        domainPackId,
        entityType: "customer",
        entityId: "manual_trigger",
      });
      return triggered;
    } catch (e) {
      console.error("[useDomainData] run automation rules failed:", e);
      return [];
    } finally {
      setRulesRunning(false);
    }
  }, [domainPackId]);

  // 执行工作流（使用 opc_execute_workflow 命令，传递 workflow_id + domain_pack_id + days + userInput）
  const executeWorkflow = useCallback(
    async (workflowId: string, userInput?: Record<string, unknown>): Promise<WorkflowExecutionResult> => {
      setWorkflowExecuting(true);
      try {
        const result = await invoke<WorkflowExecutionResult>("opc_execute_workflow", {
          domainPackId,
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
    [domainPackId],
  );

  // 反思（使用 opc_reflect_on_workflow 命令，需要 workflow_id + workflow_result）
  const reflectOnWorkflow = useCallback(
    async (workflowId?: string) => {
      if (!domainPackId) {
        return;
      }
      try {
        const wfId = workflowId || `default_${domainPackId}`;
        const wfResult = workflowResult || { status: "completed", steps_completed: 0, steps_total: 0 };
        await invoke("opc_reflect_on_workflow", {
          domainPackId,
          workflowId: wfId,
          workflowResult: wfResult,
        });
        await loadLearningMetrics();
      } catch (e) {
        console.error("[useDomainData] reflect failed:", e);
      }
    },
    [domainPackId, workflowResult, loadLearningMetrics],
  );

  // 进化（使用 opc_evolve_workflow 命令，需要 workflow_id + reason）
  const evolveWorkflow = useCallback(
    async (workflowId?: string, reason?: string) => {
      if (!domainPackId) {
        return;
      }
      try {
        const wfId = workflowId || `default_${domainPackId}`;
        const reasonText = reason || t("opc.domain.learning.evolution.defaultReason");
        await invoke("opc_evolve_workflow", {
          domainPackId,
          workflowId: wfId,
          reason: reasonText,
        });
        await loadLearningMetrics();
      } catch (e) {
        console.error("[useDomainData] evolve failed:", e);
      }
    },
    [domainPackId, loadLearningMetrics, t],
  );

  // 自我改进（使用 opc_run_self_improvement 命令，需要 target）
  const runSelfImprovement = useCallback(
    async (target?: string) => {
      if (!domainPackId) {
        return;
      }
      try {
        const targetText = target || "all";
        await invoke("opc_run_self_improvement", {
          domainPackId,
          target: targetText,
        });
        await loadLearningMetrics();
      } catch (e) {
        console.error("[useDomainData] self improve failed:", e);
      }
    },
    [domainPackId, loadLearningMetrics],
  );

  // 初始化加载
  useEffect(() => {
    if (!domainPackId) {
      return;
    }
    loadDashboard();
    loadWorkflowSteps();
    loadAutomationRules();
    loadLearningMetrics();
    loadLearningConfig();
  }, [domainPackId, loadDashboard, loadWorkflowSteps, loadAutomationRules, loadLearningMetrics, loadLearningConfig]);

  // KPI 时间范围变化时刷新
  useEffect(() => {
    if (!domainPackId) {
      return;
    }
    loadDashboard();
  }, [domainPackId, kpiTimeRange, loadDashboard]);

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
