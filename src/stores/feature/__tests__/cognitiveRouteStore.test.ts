// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it } from "vitest";
import { useCognitiveRouteStore } from "../cognitiveRouteStore";

import type { CognitiveRouteEventPayload } from "@/types";

/** 构造 route_decision 事件 payload */
function decisionPayload(): CognitiveRouteEventPayload {
  return {
    phase: "route_decision",
    emittedAtMs: 1,
    conversationId: "conv-1",
    capabilityId: "workflow:stock-analysis",
    executionMode: "workflow",
    routePath: "finance/stock-analysis/tech",
    domain: "finance",
    cluster: "stock-analysis",
    confidence: 0.92,
    isLlmFallback: false,
    stageRecords: [{ stage: "L1Domain", success: true, confidence: 0.9, elapsedMs: 3, summary: "" }],
  };
}

describe("cognitiveRouteStore.recordRouteEvent", () => {
  beforeEach(() => {
    useCognitiveRouteStore.getState().reset();
  });

  it("route_decision 全量覆盖且标记执行中", () => {
    useCognitiveRouteStore.getState().recordRouteEvent(decisionPayload());
    const obs = useCognitiveRouteStore.getState().observation;
    expect(obs).not.toBeNull();
    expect(obs?.capabilityId).toBe("workflow:stock-analysis");
    expect(obs?.executionMode).toBe("workflow");
    expect(obs?.stageRecords).toHaveLength(1);
    expect(obs?.phase).toBe("route_decision");
    expect(obs?.executing).toBe(true);
    expect(obs?.failed).toBe(false);
  });

  it("dispatch 合并更新执行模式", () => {
    useCognitiveRouteStore.getState().recordRouteEvent(decisionPayload());
    useCognitiveRouteStore.getState().recordRouteEvent({
      phase: "dispatch",
      capabilityId: "workflow:stock-analysis",
      executionMode: "delegate",
    });
    const obs = useCognitiveRouteStore.getState().observation;
    expect(obs?.executionMode).toBe("delegate");
    expect(obs?.phase).toBe("dispatch");
    expect(obs?.executing).toBe(true);
  });

  it("completed 清除执行中标记", () => {
    useCognitiveRouteStore.getState().recordRouteEvent(decisionPayload());
    useCognitiveRouteStore.getState().recordRouteEvent({
      phase: "completed",
      capabilityId: "workflow:stock-analysis",
      executionMode: "workflow",
    });
    const obs = useCognitiveRouteStore.getState().observation;
    expect(obs?.phase).toBe("completed");
    expect(obs?.executing).toBe(false);
    expect(obs?.failed).toBe(false);
  });

  it("failed 写入错误信息（错误路径观测不再丢失）", () => {
    useCognitiveRouteStore.getState().recordRouteEvent(decisionPayload());
    useCognitiveRouteStore.getState().recordRouteEvent({
      phase: "failed",
      errorCode: "WORKFLOW_NOT_FOUND",
      errorCategory: "Workflow",
      errorDetail: "模板不存在",
    });
    const obs = useCognitiveRouteStore.getState().observation;
    expect(obs?.phase).toBe("failed");
    expect(obs?.executing).toBe(false);
    expect(obs?.failed).toBe(true);
    expect(obs?.errorCode).toBe("WORKFLOW_NOT_FOUND");
    expect(obs?.errorDetail).toBe("模板不存在");
  });

  it("无前置决策时 failed 构造最小失败记录", () => {
    useCognitiveRouteStore.getState().recordRouteEvent({
      phase: "failed",
      errorCode: "ROUTE_FAILED",
      errorCategory: "Router",
      errorDetail: "boom",
    });
    const obs = useCognitiveRouteStore.getState().observation;
    expect(obs).not.toBeNull();
    expect(obs?.failed).toBe(true);
    expect(obs?.errorCode).toBe("ROUTE_FAILED");
  });

  it("无前置决策时 dispatch/completed 忽略", () => {
    useCognitiveRouteStore.getState().recordRouteEvent({ phase: "completed" });
    expect(useCognitiveRouteStore.getState().observation).toBeNull();
  });
});
