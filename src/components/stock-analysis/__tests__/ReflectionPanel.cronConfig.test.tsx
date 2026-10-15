// SPDX-License-Identifier: AGPL-3.0-only
//
// PLAN-reflection-optimization-closure §六 的呈现层门：定时反思的三个控件必须
// **回读库里那条任务的真实配置**，并且把「读不到」显示成独立的缺席，而不是拿一个
// 出厂默认冒充当前生效值（旧形态：初值硬编码 "0 6 * * *"/0/"light"、从不回读，
// 且库里已有任务时开关只改状态 ⇒ 改完值再开关，编辑值零落库）。
//
// 四条测试各锁一格，破坏哪一格就红哪一条：
//   ① 有任务且 prompt 是配置 JSON ⇒ 显示库里的最低置信度，保存按钮可用；
//   ② 没有任何任务 ⇒ 三项显示「未配置」，保存按钮禁用（此时只能靠开关新建）；
//   ③ 有任务但 prompt 不是配置 JSON（早期把中文描述写进 prompt）⇒ 显式「读不到配置」，
//      不得把 0/light 显示成生效值；
//   ④ 点「保存配置」⇒ 必须以 camelCase 四项调用 update_validate_decisions_cron。
import i18n from "@/i18n";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ReflectionPanel } from "../ReflectionPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

// 被测面板读 stockCode；`@/stores` 的 barrel 会拉起整棵 store 树（jsdom 下重且易崩），
// 按最小面替身（同 ReflectionPanel.horizon.test.tsx 的处置）。
vi.mock("@/stores", () => ({
  useStockAnalysisStore: (selector: (s: { stockCode: string | null }) => unknown) => selector({ stockCode: "600519" }),
}));

function stubInvokes(cronJobs: unknown[]) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "list_reflections") { return Promise.resolve([]); }
    if (cmd === "list_validate_decisions_crons") { return Promise.resolve(cronJobs); }
    if (cmd === "reflection_stats") { return Promise.resolve({}); }
    return Promise.resolve(null);
  });
}

function renderPanel() {
  return render(
    <I18nextProvider i18n={i18n}>
      <ReflectionPanel />
    </I18nextProvider>,
  );
}

/** 数字输入框的实际显示值（用 DOM 值而不是文案，避免与语言包字面量耦合）。 */
function numberInputValues(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll('input[type="number"]')).map(
    (el) => (el as HTMLInputElement).value,
  );
}

const NOT_SET = i18n.t("stockAnalysis.reflection.configNotSet");
const NOT_READABLE = i18n.t("stockAnalysis.reflection.configNotReadable");
const SAVE_BTN = i18n.t("stockAnalysis.reflection.saveConfigBtn");

beforeEach(() => {
  invokeMock.mockReset();
});

describe("ReflectionPanel 定时配置回读", () => {
  it("回读库里任务配置：显示 prompt 里的最低置信度，并允许保存", async () => {
    stubInvokes([
      {
        id: "vldec-1",
        name: "决策校验",
        schedule: "0 18 * * *",
        status: "active",
        prompt: JSON.stringify({ minConfidence: 70, reflectionDepth: "deep" }),
      },
    ]);
    const { container } = renderPanel();
    await waitFor(() => {
      expect(numberInputValues(container)).toContain("70");
    });
    // 读到了配置 ⇒ 不该出现「未配置」占位
    expect(screen.queryAllByText(NOT_SET)).toHaveLength(0);
    expect(screen.getByRole("button", { name: SAVE_BTN })).toBeEnabled();
  });

  it("没有任何定时任务：三项显示「未配置」，保存按钮禁用", async () => {
    stubInvokes([]);
    const { container } = renderPanel();
    await waitFor(() => {
      // 两个 Select 的占位是文本节点；数字输入框的占位在属性上（antd 行为差异，别混判）
      expect(screen.getAllByText(NOT_SET).length).toBeGreaterThanOrEqual(2);
    });
    const num = container.querySelector('input[type="number"]') as HTMLInputElement | null;
    expect(num?.placeholder).toBe(NOT_SET);
    expect(num?.value).toBe("");
    expect(screen.getByRole("button", { name: SAVE_BTN })).toBeDisabled();
    // 「读不到配置」那句是另一种缺席（有任务但配置不可解析），无任务时不得同时出现
    expect(screen.queryByText(NOT_READABLE)).toBeNull();
  });

  it("任务的 prompt 不是配置 JSON：显式声明读不到，不把出厂值显示成生效值", async () => {
    stubInvokes([
      {
        id: "vldec-2",
        name: "决策校验（旧）",
        schedule: "0 6 * * *",
        status: "active",
        prompt: "决策校验 + 反思复盘", // 早期形态：prompt 里写的是中文描述
      },
    ]);
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(NOT_READABLE)).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: SAVE_BTN })).toBeDisabled();
  });

  it("点「保存配置」以 camelCase 四项调用 update_validate_decisions_cron", async () => {
    stubInvokes([
      {
        id: "vldec-3",
        name: "决策校验",
        schedule: "0 18 * * *",
        status: "active",
        prompt: JSON.stringify({ minConfidence: 70, reflectionDepth: "deep" }),
      },
    ]);
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "update_validate_decisions_cron") {
        return Promise.resolve({
          id: "vldec-3",
          name: "决策校验",
          schedule: "0 12 * * *",
          status: "active",
          prompt: JSON.stringify({ minConfidence: 55, reflectionDepth: "light" }),
        });
      }
      return stubInvokesResult(cmd);
    });
    const { container } = renderPanel();
    await waitFor(() => {
      expect(numberInputValues(container)).toContain("70");
    });

    const input = Array.from(container.querySelectorAll('input[type="number"]')).find(
      (el) => (el as HTMLInputElement).value === "70",
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "55" } });

    fireEvent.click(screen.getByRole("button", { name: SAVE_BTN }));
    await waitFor(() => {
      const call = invokeMock.mock.calls.find((c) => c[0] === "update_validate_decisions_cron");
      expect(call).toBeTruthy();
      expect(call?.[1]).toEqual({
        id: "vldec-3",
        cronExpression: "0 18 * * *",
        minConfidenceThreshold: 55,
        reflectionDepth: "deep",
      });
    });
  });
});

/** 第 ④ 条测试里除 update 之外仍要能列任务的返回值。 */
function stubInvokesResult(cmd: string): unknown {
  if (cmd === "list_reflections") { return []; }
  if (cmd === "list_validate_decisions_crons") {
    return [
      {
        id: "vldec-3",
        name: "决策校验",
        schedule: "0 18 * * *",
        status: "active",
        prompt: JSON.stringify({ minConfidence: 70, reflectionDepth: "deep" }),
      },
    ];
  }
  if (cmd === "reflection_stats") { return {}; }
  return null;
}
