// SPDX-License-Identifier: AGPL-3.0-only

import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  listen: listenMock,
  isTauri: () => false,
}));

// evolutionStore 顶层 ENGINE_DEFINITIONS 调 t(...),mock 掉 i18n 依赖
vi.mock("@/lib/i18nStoreHelper", () => ({
  t: (key: string) => key,
}));

import { adaptAbTestReport, useEvolutionStore } from "@/stores/feature/evolutionStore";
import type { AbTestReportDTO } from "@/stores/feature/evolutionStore";

function makeReport(overrides?: Partial<AbTestReportDTO>): AbTestReportDTO {
  return {
    testId: "test-1",
    skillId: "skill-1",
    versionA: "v1",
    versionB: "v2",
    winner: "B",
    metrics: [{ name: "成功率", valueA: 100, valueB: 110, unit: "%" }],
    conclusion: "B 胜",
    ...overrides,
  };
}

describe("adaptAbTestReport(任务 #218 A/B 适配器口径)", () => {
  it("change% 方向为 (B-A)/A×100(B 相对 A 的提升),change>0 ⇒ B 胜", () => {
    const out = adaptAbTestReport(makeReport({ metrics: [{ name: "m", valueA: 100, valueB: 110, unit: "%" }] }));
    expect(out).toHaveLength(1);
    expect(out[0].change).toBeCloseTo(10, 6);
    expect(out[0].winner).toBe("B");
  });

  it("change<0 ⇒ A 胜", () => {
    const out = adaptAbTestReport(makeReport({ metrics: [{ name: "m", valueA: 110, valueB: 100, unit: "%" }] }));
    expect(out[0].change).toBeCloseTo(-9.090909, 4);
    expect(out[0].winner).toBe("A");
  });

  it("change=0 ⇒ 平局", () => {
    const out = adaptAbTestReport(makeReport({ metrics: [{ name: "m", valueA: 100, valueB: 100, unit: "%" }] }));
    expect(out[0].change).toBe(0);
    expect(out[0].winner).toBe("tie");
  });

  it("A=0 ⇒ change=null ⇒ 平局(除零保护)", () => {
    const out = adaptAbTestReport(makeReport({ metrics: [{ name: "m", valueA: 0, valueB: 50, unit: "%" }] }));
    expect(out[0].change).toBeNull();
    expect(out[0].winner).toBe("tie");
  });

  it("overall winner 原样透出,不与 per-metric winner 混算", () => {
    const out = adaptAbTestReport(
      makeReport({ winner: "B", metrics: [{ name: "m", valueA: 110, valueB: 100, unit: "%" }] }),
    );
    expect(out[0].winner).toBe("A"); // per-metric 按 change 正负推导
    expect(out[0].overallWinner).toBe("B"); // 后端整体 winner 原样透出
  });

  it("report 为 null 或缺 metrics ⇒ 空数组", () => {
    expect(adaptAbTestReport(null)).toEqual([]);
    expect(
      adaptAbTestReport(makeReport({ metrics: undefined as unknown as AbTestReportDTO["metrics"] })),
    ).toEqual([]);
  });
});

describe("evolutionStore getters(任务 #218 批一)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('getABTestResults("") 无技能绑定 ⇒ 短路空数组,不发起 invoke', async () => {
    const out = await useEvolutionStore.getState().getABTestResults("");
    expect(out).toEqual([]);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("传真 skillId ⇒ invoke evaluator_get_ab_results(camelCase 传参)并按适配器口径输出", async () => {
    invokeMock.mockResolvedValueOnce(makeReport());
    const out = await useEvolutionStore.getState().getABTestResults("skill-1");
    expect(invokeMock).toHaveBeenCalledWith("evaluator_get_ab_results", { skillId: "skill-1" });
    expect(out).toHaveLength(1);
    expect(out[0].winner).toBe("B");
    expect(out[0].change).toBeCloseTo(10, 6);
    expect(out[0].overallWinner).toBe("B");
  });

  it("invoke 失败 ⇒ 空数组(不回退假数据)", async () => {
    invokeMock.mockRejectedValueOnce(new Error("boom"));
    const out = await useEvolutionStore.getState().getABTestResults("skill-1");
    expect(out).toEqual([]);
  });

  it("getSkillEvolutionHistory 恒返回空数组(批二接持久化轨迹)", async () => {
    const out = await useEvolutionStore.getState().getSkillEvolutionHistory("skill-1");
    expect(out).toEqual([]);
  });
});
