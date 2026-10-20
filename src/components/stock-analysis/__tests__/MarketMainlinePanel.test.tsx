// SPDX-License-Identifier: AGPL-3.0-only
//
// D7 的呈现层门：市场主线读侧的三种缺席各占一句，且**解析失败不得冒充零标的**。
// 立门理由：`market_mainlines` 现网 0 行不是「今天没主线」，而是写入全靠模型自愿调工具、
// 实测近 8 次一次没调（A3 已把它改成确定性节点）。读侧若把「空表」「命令失败」「标的字段
// 存的不是 JSON 数组」压成同一句，界面就会替一个尚未成立的结论说话。
import i18n from "@/i18n";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MarketMainlinePanel } from "../MarketMainlinePanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

const row = (over: Record<string, unknown> = {}) => ({
  id: "m1",
  mainlineDate: "2026-10-20",
  theme: "AI 算力",
  themeCategory: "科技",
  narrative: "隔夜海外链走强",
  representativeSymbols: JSON.stringify(["002230", "688256"]),
  strengthScore: 85,
  persistence: "1w",
  status: "active",
  sourceWorkflowExecutionId: "exec-abcdef123456",
  ...over,
});

function renderPanel() {
  return render(
    <I18nextProvider i18n={i18n}>
      <MarketMainlinePanel />
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("MarketMainlinePanel 的缺席分列", () => {
  it("① 有行：出表，代表标的按 JSON 数组数出来，来源批次原样可见", async () => {
    invokeMock.mockResolvedValue([row()]);
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mainline-symbols").textContent).toBe("2"));
    expect(screen.queryByTestId("mainlines-empty")).not.toBeInTheDocument();
    expect(screen.queryByTestId("mainlines-failed")).not.toBeInTheDocument();
    expect(screen.getByTestId("mainline-source").textContent).toBe("exec-abcdef123456");
  });

  it("② 库里零行：报「写入节点是新加的、需重启并等 18:00」，不报失败", async () => {
    invokeMock.mockResolvedValue([]);
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mainlines-empty")).toBeInTheDocument());
    expect(screen.getByTestId("mainlines-empty").textContent).toContain(
      i18n.t("backtest.mainlines.stateNoRows"),
    );
    expect(screen.queryByTestId("mainlines-failed")).not.toBeInTheDocument();
  });

  it("③ 命令失败：报失败并带上原因，绝不显示成「没有主线」", async () => {
    invokeMock.mockRejectedValue(new Error("db down"));
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mainlines-failed")).toBeInTheDocument());
    expect(screen.getByTestId("mainlines-failed").textContent).toContain("db down");
    expect(screen.queryByTestId("mainlines-empty")).not.toBeInTheDocument();
  });

  it("④ 代表标的存的不是 JSON 数组 ⇒ 出「—」而不是 0（空数组与坏形态是两回事）", async () => {
    invokeMock.mockResolvedValue([row({ representativeSymbols: "{not json" })]);
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("mainline-symbols").textContent).toBe("—"));
  });
});
