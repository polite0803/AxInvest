// SPDX-License-Identifier: AGPL-3.0-only
//
// B1 的呈现层门（`PLAN-chain-closure-remediation.md` §3）：智选后验的三种缺席必须各占一句。
//
// 立门理由：这条链的产物此前只进 tracing 日志，界面无任何读数；接上面板后最大的风险是
// 「库里没行」「有行但没判定完」「命令调用失败」被压成同一枚 0% 或同一个空态 ——
// 那等于用一个新的歧义换掉旧的看不见。故每个破坏样本**只**让对应那一条红。
import i18n from "@/i18n";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { SerenityValidationPanel } from "../SerenityValidationPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

// 时间锚点按最小面替身：被测组件只用 mode / asOfDate 两个选择器，
// 拉真 store 会把整棵 store 树带进 jsdom（重且易崩，同 ReflectionPanel.cronConfig.test.tsx 的处置）。
const anchor = { mode: "live", asOfDate: null as string | null };
vi.mock("@/stores/feature/timeAnchorStore", () => ({
  useTimeAnchorStore: (selector: (s: { mode: string; asOfDate: string | null }) => unknown) => selector(anchor),
}));

function renderPanel() {
  return render(
    <I18nextProvider i18n={i18n}>
      <SerenityValidationPanel />
    </I18nextProvider>,
  );
}

const summary = (over: Record<string, unknown>) => ({
  rows: 354,
  candidates: 81,
  judged: 354,
  outperform: 115,
  hitRate: 115 / 354,
  labelCounts: [
    { label: "excess_outperform", count: 44 },
    { label: "outperform", count: 71 },
    { label: "underperform", count: 103 },
    { label: "excess_underperform", count: 136 },
  ],
  latestValidatedAt: "2026-10-20T07:00:00+08:00",
  generation: null,
  inWeightLoop: false,
  ...over,
});

beforeEach(() => {
  invokeMock.mockReset();
  anchor.mode = "live";
  anchor.asOfDate = null;
});

describe("SerenityValidationPanel 的缺席分列", () => {
  it("① 有判定读数：显示命中率，并恒定挂上「不参与闭环」声明", async () => {
    invokeMock.mockResolvedValue(summary({}));
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("serenity-validation-hit-rate")).toBeInTheDocument());
    expect(screen.queryByTestId("serenity-validation-no-rows")).not.toBeInTheDocument();
    expect(screen.queryByTestId("serenity-validation-unjudged")).not.toBeInTheDocument();
    expect(screen.getByTestId("serenity-validation-loop-note").textContent).toContain(
      i18n.t("backtest.serenityValidation.loopNote"),
    );
    expect(screen.getByTestId("serenity-validation-rows").textContent).toBe("354");
    // 行数与候选数是两个计数域，不得并排显示成可比 —— 面板必须各自给标签
    expect(screen.getByTestId("serenity-validation-candidates").textContent).toBe("81");
  });

  it("② 库里零行：只报「从未被验证」，不得显示命中率", async () => {
    invokeMock.mockResolvedValue(
      summary({ rows: 0, candidates: 0, judged: 0, outperform: 0, hitRate: null, labelCounts: [] }),
    );
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("serenity-validation-no-rows")).toBeInTheDocument());
    expect(screen.getByTestId("serenity-validation-no-rows").textContent).toContain(
      i18n.t("backtest.serenityValidation.stateNoRows"),
    );
    expect(screen.queryByTestId("serenity-validation-hit-rate")).not.toBeInTheDocument();
    expect(screen.queryByTestId("serenity-validation-unjudged")).not.toBeInTheDocument();
    // 零行时不挂「不参与闭环」声明：那格会被读成「取数失败」的同义句
    expect(screen.queryByTestId("serenity-validation-loop-note")).not.toBeInTheDocument();
  });

  it("③ 有行但零判定：报「窗口未满/取数失败」，与②不同一句", async () => {
    invokeMock.mockResolvedValue(summary({ judged: 0, outperform: 0, hitRate: null, labelCounts: [] }));
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("serenity-validation-unjudged")).toBeInTheDocument());
    const unjudged = screen.getByTestId("serenity-validation-unjudged").textContent ?? "";
    expect(unjudged).toContain(i18n.t("backtest.serenityValidation.stateUnjudged"));
    expect(unjudged).not.toContain(i18n.t("backtest.serenityValidation.stateNoRows"));
    expect(screen.queryByTestId("serenity-validation-no-rows")).not.toBeInTheDocument();
    expect(screen.queryByTestId("serenity-validation-hit-rate")).not.toBeInTheDocument();
  });

  it("④ 命令调用失败：报失败，绝不显示成「库里没有数据」", async () => {
    invokeMock.mockRejectedValue(new Error("backend exploded"));
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("serenity-validation-failed")).toBeInTheDocument());
    expect(screen.getByTestId("serenity-validation-failed").textContent).toContain("backend exploded");
    expect(screen.queryByTestId("serenity-validation-no-rows")).not.toBeInTheDocument();
    expect(screen.queryByTestId("serenity-validation-unjudged")).not.toBeInTheDocument();
  });

  it("⑤ 回放态：必须以那批的截止日为 generation 取数（参数不是摆设）", async () => {
    anchor.mode = "replay";
    anchor.asOfDate = "2026-09-28";
    invokeMock.mockResolvedValue(summary({ generation: "2026-09-28" }));
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("serenity-validation-generation")).toBeInTheDocument());
    const call = invokeMock.mock.calls.find((c) => c[0] === "serenity_validation_summary");
    expect(call).toBeTruthy();
    expect(call?.[1]).toEqual({ generation: "2026-09-28" });
  });

  it("⑥ 实盘态：传 null 而不是省略键（与后端 Option<String> 的缺省口径一致）", async () => {
    invokeMock.mockResolvedValue(summary({}));
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("serenity-validation-hit-rate")).toBeInTheDocument());
    const call = invokeMock.mock.calls.find((c) => c[0] === "serenity_validation_summary");
    expect(call?.[1]).toEqual({ generation: null });
  });
});
