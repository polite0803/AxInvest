// SPDX-License-Identifier: AGPL-3.0-only

import { invoke } from "@/lib/invoke";
import type {
  AutoLearningResult,
  DomainLearningConfig,
  DomainLearningConfigSummary,
  EvolveWorkflowParams,
  ExperiencePoolStats,
  ReflectOnWorkflowParams,
  RLPolicyUpdate,
  RunSelfImprovementParams,
  TriggerRLOptimizationParams,
} from "@/types";

/**
 * 获取指定行业的学习配置
 */
export async function getLearningConfig(
  domainPackId: string,
): Promise<DomainLearningConfig> {
  return invoke<DomainLearningConfig>("opc_get_learning_config", {
    domainPackId,
  });
}

/**
 * 获取所有行业的学习配置列表
 */
export async function listLearningConfigs(): Promise<DomainLearningConfigSummary[]> {
  return invoke<DomainLearningConfigSummary[]>("opc_list_learning_configs");
}

/**
 * 触发工作流反思
 * P1-7：返回类型对齐后端 ReflectionResult（camelCase）
 */
export async function reflectOnWorkflow(
  params: ReflectOnWorkflowParams,
): Promise<{
  success: boolean;
  domainPackId: string;
  workflowId: string;
  qualityScore: number;
  suggestions: string[];
  summary: string;
}> {
  return invoke("opc_reflect_on_workflow", {
    domainPackId: params.domainPackId,
    workflowId: params.workflowId,
    workflowResult: params.workflowResult,
  });
}

/**
 * 触发工作流进化
 * P1-7：返回类型对齐后端 EvolutionResult
 */
export async function evolveWorkflow(
  params: EvolveWorkflowParams,
): Promise<{
  success: boolean;
  domainPackId: string;
  workflowId: string;
  status: string;
  suggestedOptimizations: string[];
  message: string;
}> {
  return invoke("opc_evolve_workflow", {
    domainPackId: params.domainPackId,
    workflowId: params.workflowId,
    reason: params.reason,
  });
}

/**
 * 执行自我改进
 * P1-7：返回类型对齐后端 SelfImprovementResult
 */
export async function runSelfImprovement(
  params: RunSelfImprovementParams,
): Promise<{
  success: boolean;
  domainPackId: string;
  target: string;
  status: string;
  improvementsApplied: string[];
  message: string;
}> {
  return invoke("opc_run_self_improvement", {
    domainPackId: params.domainPackId,
    target: params.target,
  });
}

/**
 * 获取 RL 经验池统计
 */
export async function getRLStats(domainPackId?: string): Promise<ExperiencePoolStats> {
  return invoke("opc_get_rl_stats", {
    domainPackId,
  });
}

/**
 * 记录 RL 经验
 */
export async function recordRLExperience(
  params: {
    domainPackId: string;
    workflowId: string;
    qualityScore: number;
    workflowResult: Record<string, unknown>;
  },
): Promise<{ success: boolean; experienceId?: string; totalReward?: number; message?: string }> {
  return invoke("opc_record_rl_experience", {
    domainPackId: params.domainPackId,
    workflowId: params.workflowId,
    qualityScore: params.qualityScore,
    workflowResult: params.workflowResult,
  });
}

/**
 * 触发 RL 策略优化
 */
export async function triggerRLOptimization(
  params: TriggerRLOptimizationParams,
): Promise<RLPolicyUpdate> {
  return invoke("opc_trigger_rl_optimization", {
    domainPackId: params.domainPackId,
  });
}

/**
 * 触发自动学习闭环（反思→进化→自我改进→RL）
 */
export async function triggerAutoLearning(params: {
  domainPackId: string;
  workflowId: string;
  workflowResult: Record<string, unknown>;
}): Promise<AutoLearningResult> {
  return invoke("opc_trigger_domain_pack_learning", {
    domainPackId: params.domainPackId,
    workflowId: params.workflowId,
    workflowResult: params.workflowResult,
  });
}
