// SPDX-License-Identifier: AGPL-3.0-only

/**
 * `useDomainData` 的「诚实性」不变式
 *
 * 1. **首帧标志是「待答」而非「答案是没有」** —— 取数标志初值为 `false` 时，
 *    首帧会在加载 effect 跑起来之前渲染出 `Empty`（「暂无数据」），把「还没问」
 *    显示成「后端说没有」；
 * 2. **失败落进 `errors`，不塌进空数据** —— 组件据此渲染错误态而非空态；
 * 3. **规则执行失败必须抛出** —— 原实现在 catch 里 `return []`，
 *    把「根本没跑成」伪装成「跑完没有规则命中」。
 */

import { invoke } from "@/lib/invoke";
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useDomainData } from "../useDomainData";

vi.mock(import("@/lib/invoke"), async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, invoke: vi.fn() };
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

/** 把挂载时发起的 5 条并行取数的落定收进 act，避免污染输出的 act 警告 */
const flushPendingLoads = () =>
  act(async () => {
    await Promise.resolve();
  });

describe("useDomainData", () => {
  it("首帧：取数标志为「待答」，决策保持未触发", () => {
    // 永不落定：把首帧状态冻在「尚未得到答案」
    vi.mocked(invoke).mockImplementation(() => new Promise(() => {}));

    const { result } = renderHook(() => useDomainData("demo-pack"));

    expect(result.current.dashboardLoading).toBe(true);
    expect(result.current.stepsLoading).toBe(true);
    expect(result.current.rulesLoading).toBe(true);
    expect(result.current.metricsLoading).toBe(true);
    expect(result.current.learningLoading).toBe(true);
    // 分析决策是手动触发项：未点「执行分析」前不得处于取数中、也不得有结论
    expect(result.current.decisionLoading).toBe(false);
    expect(result.current.decision).toBeNull();
  });

  it("取数失败：落进 errors 且结束等待，不塌成空数据", async () => {
    vi.mocked(invoke).mockRejectedValue("backend down");

    const { result } = renderHook(() => useDomainData("demo-pack"));

    await waitFor(() => expect(result.current.errors.dashboard).toBeTruthy());
    expect(result.current.errors.dashboard).toContain("backend down");
    expect(result.current.errors.steps).toContain("backend down");
    expect(result.current.errors.rules).toContain("backend down");
    expect(result.current.errors.metrics).toContain("backend down");
    expect(result.current.errors.learningConfig).toContain("backend down");
    expect(result.current.dashboardLoading).toBe(false);
    expect(result.current.learningLoading).toBe(false);
    // 失败态下不得同时给出「空数据」这种正常结论
    expect(result.current.dashboard).toBeNull();
    expect(result.current.learningMetrics).toBeNull();

    await flushPendingLoads();
  });

  it("规则执行失败必须抛出，不得伪装成「没有规则被触发」", async () => {
    vi.mocked(invoke).mockRejectedValue("rule engine down");

    const { result } = renderHook(() => useDomainData("demo-pack"));
    await flushPendingLoads();

    // 拒绝本身要让 finally 清 rulesRunning ⇒ 状态更新须收进 act
    await act(async () => {
      await expect(result.current.runAutomationRules()).rejects.toBe("rule engine down");
    });
    expect(result.current.rulesRunning).toBe(false);
  });
});
