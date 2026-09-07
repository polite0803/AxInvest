// SPDX-License-Identifier: AGPL-3.0-only

import { listen } from "@/lib/invoke";
import type {
  CognitiveCandidateSummary,
  CognitiveExecutionMode,
  CognitiveQueryResponse,
  CognitiveRouteEventPayload,
  CognitiveRouteStageView,
  CognitiveSelectedAgentProfile,
} from "@/types";
import { create } from "zustand";

/**
 * 认知编排路由观测状态
 *
 * 保存最近一次认知编排请求的路由决策结果（stageRecords / routePath /
 * circuitBroken / totalElapsedMs / isLlmFallback 等），供右侧边栏「路由观测」
 * 面板展示。仅保留最近一次，切换会话/再次发送时覆盖。
 *
 * 两条数据通道：
 * 1. 返回值通道（向后兼容）：cognitive_query 同步返回后 recordObservation 全量写入；
 * 2. 事件通道（T6）：`cognitive-route-event` 三时点推送 —— route_decision（决策即达，
 *    不再阻塞等执行完成）、dispatch（执行模式分派）、completed/failed（错误路径也覆盖）。
 *    由 ensureRouteEventListening 幂等订阅，面板挂载时启动。
 */
export interface CognitiveRouteObservation {
  /** 所属会话 */
  conversationId: string;
  /** 三层路由地址（确定性路径），如 "invest/stock_analysis/tech" */
  routePath: string;
  /** 业务域 */
  domain: string;
  /** 功能集群 */
  cluster: string;
  /** 具体能力/工作流 ID */
  capabilityId: string;
  /** 路由置信度（0.0 - 1.0） */
  confidence: number;
  /** 是否通过 LLM 兜底 */
  isLlmFallback: boolean;
  /** 是否触发熔断 */
  circuitBroken: boolean;
  /** 熔断原因 */
  circuitBreakReason: string | null;
  /** 备选路径 */
  fallbackPath: string | null;
  /** 候选列表（Top-K） */
  candidates: string[];
  /** 候选能力详情（含名称/描述/置信度/种类，供前端展示） */
  candidateDetails: CognitiveCandidateSummary[];
  /** 熔断过滤数量（RAR 原始候选数 - 最终候选数，0 表示无过滤） */
  filteredCount: number;
  /** 执行模式 */
  executionMode: CognitiveExecutionMode;
  /** 选中工作流的可读名称（未命中工作流时为 null） */
  selectedWorkflowName: string | null;
  /** 选中的执行专家（Agent 执行路径；未走 Agent 路径时为 null） */
  selectedAgentProfile: CognitiveSelectedAgentProfile | null;
  /** 各阶段执行记录 */
  stageRecords: CognitiveRouteStageView[];
  /** 总耗时（毫秒） */
  totalElapsedMs: number;
  /** 记录时间戳 */
  recordedAt: number;
  /** 最近事件阶段（事件通道独有；返回值通道写入时为 undefined） */
  phase?: CognitiveRouteEventPayload["phase"];
  /** 执行是否进行中（route_decision 后为 true，completed/failed 后为 false） */
  executing?: boolean;
  /** 本次编排是否失败（failed 事件写入） */
  failed?: boolean;
  /** 失败错误码 */
  errorCode?: string | null;
  /** 失败错误详情 */
  errorDetail?: string | null;
}

/** 由 route_decision 事件构造全量观测（缺失字段用安全默认值填充，保证面板兼容） */
function observationFromRouteDecision(
  payload: CognitiveRouteEventPayload,
): CognitiveRouteObservation {
  return {
    conversationId: payload.conversationId ?? "",
    routePath: payload.routePath ?? "",
    domain: payload.domain ?? "",
    cluster: payload.cluster ?? "",
    capabilityId: payload.capabilityId ?? "",
    confidence: payload.confidence ?? 0,
    isLlmFallback: payload.isLlmFallback ?? false,
    circuitBroken: false,
    circuitBreakReason: null,
    fallbackPath: null,
    candidates: [],
    candidateDetails: [],
    filteredCount: 0,
    executionMode: payload.executionMode ?? "act",
    selectedWorkflowName: null,
    selectedAgentProfile: null,
    stageRecords: payload.stageRecords ?? [],
    totalElapsedMs: 0,
    recordedAt: Date.now(),
    phase: "route_decision",
    executing: true,
    failed: false,
    errorCode: null,
    errorDetail: null,
  };
}

interface CognitiveRouteState {
  /** 最近一次认知路由观测（null 表示尚无观测） */
  observation: CognitiveRouteObservation | null;
  /** 记录一次观测（覆盖旧值；返回值通道） */
  recordObservation: (
    conversationId: string,
    response: CognitiveQueryResponse,
  ) => void;
  /** 记录一条路由事件（事件通道；按 phase 合并/覆盖） */
  recordRouteEvent: (payload: CognitiveRouteEventPayload) => void;
  /** 清空观测 */
  reset: () => void;
}

export const useCognitiveRouteStore = create<CognitiveRouteState>(
  (set, get) => ({
    observation: null,

    recordObservation: (conversationId, response) => {
      set({
        observation: {
          conversationId,
          routePath: response.routePath,
          domain: response.domain,
          cluster: response.cluster,
          capabilityId: response.capabilityId,
          confidence: response.confidence,
          isLlmFallback: response.isLlmFallback,
          circuitBroken: response.circuitBroken,
          circuitBreakReason: response.circuitBreakReason ?? null,
          fallbackPath: response.fallbackPath ?? null,
          candidates: response.candidates ?? [],
          candidateDetails: response.candidateDetails ?? [],
          filteredCount: response.filteredCount ?? 0,
          executionMode: response.executionMode,
          selectedWorkflowName: response.selectedWorkflowName ?? null,
          selectedAgentProfile: response.selectedAgentProfile ?? null,
          stageRecords: response.stageRecords ?? [],
          totalElapsedMs: response.totalElapsedMs,
          recordedAt: Date.now(),
          executing: false,
          failed: false,
        },
      });
    },

    recordRouteEvent: (payload) => {
      const prev = get().observation;
      switch (payload.phase) {
        case "route_decision": {
          // 决策即达：全量覆盖（执行进行中）
          set({ observation: observationFromRouteDecision(payload) });
          return;
        }
        case "dispatch": {
          // route_decision 丢失（面板晚挂载/事件乱序）时先补最小记录
          if (!prev) {
            set({ observation: observationFromRouteDecision(payload) });
            return;
          }
          set({
            observation: {
              ...prev,
              executionMode: payload.executionMode ?? prev.executionMode,
              phase: "dispatch",
            },
          });
          return;
        }
        case "completed": {
          if (!prev) { return; }
          set({
            observation: {
              ...prev,
              phase: "completed",
              executing: false,
              failed: false,
            },
          });
          return;
        }
        case "failed": {
          if (!prev) {
            // 失败且无前置决策：构造最小失败记录（错误路径观测不再丢失）
            set({
              observation: {
                ...observationFromRouteDecision(payload),
                executing: false,
                failed: true,
                errorCode: payload.errorCode ?? null,
                errorDetail: payload.errorDetail ?? null,
              },
            });
            return;
          }
          set({
            observation: {
              ...prev,
              phase: "failed",
              executing: false,
              failed: true,
              errorCode: payload.errorCode ?? null,
              errorDetail: payload.errorDetail ?? null,
            },
          });
        }
      }
    },

    reset: () => set({ observation: null }),
  }),
);

// ── 事件订阅（T6 事件通道）──

let routeEventUnlisten: (() => void) | null = null;

/**
 * 幂等启动 `cognitive-route-event` 订阅（面板挂载时调用）。
 *
 * 模块级缓存 unlisten：面板卸载不退订（全局单实例面板 + 事件轻量），
 * React 严格模式双挂载依赖幂等守卫，不会重复注册。
 */
export async function ensureRouteEventListening(): Promise<void> {
  if (routeEventUnlisten) { return; }
  routeEventUnlisten = await listen<CognitiveRouteEventPayload>(
    "cognitive-route-event",
    (event) => {
      useCognitiveRouteStore.getState().recordRouteEvent(event.payload);
    },
  );
}
