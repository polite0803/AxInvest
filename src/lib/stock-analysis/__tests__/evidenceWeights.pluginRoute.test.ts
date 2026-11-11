// SPDX-License-Identifier: AGPL-3.0-only

/**
 * B9-1 回归：`computeEvidenceWeights` 走**插件命令**（宿主同名 Tauri 命令进入退役期）。
 *
 * 钉住三条：
 * ① 命令名先经 `resolvePluginForCommand` 解析（不写死插件 id）；
 * ② 命中后以 `plugin_execute_command` 调用，`input` 原样透传（host 端不再包一层 `request`）；
 * ③ 插件未装/未启用 ⇒ 抛错，让 `computeStockConsensus` 走既有离线兜底（不静默返回空权重）。
 */
import { computeEvidenceWeights, type EvidenceWeightRequest } from "@/lib/stock-analysis-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
const resolveMock = vi.fn();

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@/lib/pluginCommandResolve", () => ({
  resolvePluginForCommand: (...args: unknown[]) => resolveMock(...args),
}));

const REQUEST: EvidenceWeightRequest = {
  marketRegime: {
    regime: "bull",
    confidence: 0.8,
    volatility: "normal",
    description: "单测固定环境",
  },
  timeHorizon: "mid",
  analysts: [{ analystId: "a-market-analyst", stance: "看多", bullScore: 70, bearScore: 30 }],
};

const REPORT = {
  marketRegime: REQUEST.marketRegime,
  timeHorizon: "mid",
  analystWeights: [],
  consensus: {},
  holdGate: {},
  recommendedAction: "BUY",
  recommendedPositionPct: 20,
  overallConfidence: 60,
};

beforeEach(() => {
  invokeMock.mockReset();
  resolveMock.mockReset();
});

describe("computeEvidenceWeights（插件通道）", () => {
  it("解析到插件后以 plugin_execute_command 调用，入参原样透传并解析回执", async () => {
    resolveMock.mockResolvedValue("stock@bundled");
    invokeMock.mockResolvedValue(JSON.stringify(REPORT));

    const result = await computeEvidenceWeights({ ...REQUEST });

    expect(resolveMock).toHaveBeenCalledWith("compute_evidence_weights");
    expect(invokeMock).toHaveBeenCalledWith("plugin_execute_command", {
      pluginId: "stock@bundled",
      name: "compute_evidence_weights",
      input: REQUEST,
    });
    expect(result.recommendedAction).toBe("BUY");
    expect(result.timeHorizon).toBe("mid");
  });

  it("没有启用中的插件时抛错（调用方走离线兜底，不假装算出了权重）", async () => {
    resolveMock.mockResolvedValue(null);

    await expect(computeEvidenceWeights({ ...REQUEST })).rejects.toThrow(
      /no enabled plugin provides command/,
    );
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("插件命令失败时错误向上抛（不吞成空报告）", async () => {
    resolveMock.mockResolvedValue("stock@bundled");
    invokeMock.mockRejectedValue(new Error("PLUGIN_COMMAND_TIMEOUT"));

    await expect(computeEvidenceWeights({ ...REQUEST })).rejects.toThrow("PLUGIN_COMMAND_TIMEOUT");
  });
});
