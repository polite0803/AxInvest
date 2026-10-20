import i18n from "@/i18n";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { RecoLoopPanel } from "../RecoLoopPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function renderPanel(asOfDate?: string | null) {
  return render(
    <App>
      <I18nextProvider i18n={i18n}>
        <RecoLoopPanel asOfDate={asOfDate} />
      </I18nextProvider>
    </App>,
  );
}

const LOOP = {
  gate: "on",
  lastRecalcAt: 1_792_389_861_032,
  cells: [
    {
      style: "trend",
      period: "short",
      status: "demoted_negative_ic",
      rankIc: -0.5,
      samples: 16,
      winRate: 0.72,
      oldWeight: 1,
      newWeight: 0.45,
      moverPenalty: 0.8,
      dqiPenalty: 1,
    },
    {
      style: "value",
      period: "short",
      status: "insufficient_samples",
      rankIc: null,
      samples: 3,
      winRate: null,
      oldWeight: 0.5,
      newWeight: 0.5,
    },
    {
      style: "reversion",
      period: "ultra_short",
      status: "not_in_matrix",
      rankIc: null,
      samples: 0,
      winRate: null,
      oldWeight: 1,
      newWeight: 1,
    },
  ],
};

function statsResponse(loop: unknown) {
  return {
    styles: [],
    totalCells: 0,
    usableCells: 0,
    totalSamples: 0,
    loop,
  };
}

describe("RecoLoopPanel", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reco_ic_stats") {
        return Promise.resolve(statsResponse(LOOP));
      }
      if (cmd === "recalc_reco_loop_weights") {
        return Promise.resolve({ written: 2 });
      }
      return Promise.resolve({});
    });
  });

  it("渲染闸状态、逐格样本数与降权/未校准两种状态", async () => {
    renderPanel();

    await waitFor(() => {
      expect(screen.getByText(/反思闭环已生效/)).toBeTruthy();
    });
    // 样本数是「已校准」与「只是样本很多但 IC 测不出」的分辨依据
    expect(screen.getAllByTestId("reco-loop-samples").map((n) => n.textContent)).toEqual([
      "16",
      "3",
    ]);
    expect(screen.getByText("负 IC 降权")).toBeTruthy();
    expect(screen.getByText("样本不足")).toBeTruthy();
    // 负控：未校准格不得被渲染成「已校准」
    expect(screen.queryByText("已校准")).toBeNull();
  });

  it("点击立即重算会调用 recalc_reco_loop_weights 并刷新观测面", async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByTestId("reco-loop-recalc")).toBeTruthy());

    await act(async () => {
      fireEvent.click(screen.getByTestId("reco-loop-recalc"));
    });

    await waitFor(() => {
      expect(invokeMock.mock.calls.some((c) => c[0] === "recalc_reco_loop_weights")).toBe(true);
    });
    const statsCalls = invokeMock.mock.calls.filter((c) => c[0] === "reco_ic_stats");
    expect(statsCalls.length).toBeGreaterThanOrEqual(2);
    expect(statsCalls.at(-1)?.[1]).toEqual({ generation: null });
    expect(invokeMock.mock.calls.find((c) => c[0] === "recalc_reco_loop_weights")?.[1]).toEqual({
      asOfDate: null,
    });
  });

  it("回放模式下禁用重算（该命令会写实盘权重留痕）", async () => {
    renderPanel("2026-09-28");
    await waitFor(() => {
      const btn = screen.getByTestId("reco-loop-recalc") as HTMLButtonElement;
      expect(btn.disabled).toBe(true);
    });
    expect(invokeMock.mock.calls.find((c) => c[0] === "reco_ic_stats")?.[1]).toEqual({
      generation: "2026-09-28",
    });
    // 回放代际与闭环样本不是同一分母：必须显式声明，不得让两套数看起来可比
    expect(screen.getByTestId("reco-loop-replay-note")).toBeTruthy();
  });

  it("观测面取不到时显示错误而不是空白", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reco_ic_stats") {
        return Promise.reject(new Error("boom"));
      }
      return Promise.resolve({});
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText(/读取闭环状态失败：boom/)).toBeTruthy());
  });
});
