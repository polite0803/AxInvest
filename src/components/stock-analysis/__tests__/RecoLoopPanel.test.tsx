import i18n from "@/i18n";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { invalidateRecoIcStats } from "@/lib/recoIcStats";
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
  // 判据阈值随视图出（后端 `NegativeExpectancyGate`）；夹具给生产形态的值
  negativeExpectancy: { minSamples: 10, maxMeanReturnPct: -1 },
  cells: [
    {
      style: "trend",
      period: "short",
      status: "demoted_negative_ic",
      rankIc: -0.5,
      samples: 16,
      winRate: 0.72,
      meanReturnPct: 0.85,
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
      meanReturnPct: -0.4,
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
      meanReturnPct: null,
      oldWeight: 1,
      newWeight: 1,
    },
  ],
};

/** 硬停格的齐备态读数：均收益 -4.04%、12 单 ⇒ 与 2026-10-24 的生产实证同形。 */
const DISABLED_LOOP = {
  ...LOOP,
  cells: [
    {
      style: "capital",
      period: "mid",
      status: "disabled_negative_expectancy",
      rankIc: 0.01,
      samples: 12,
      winRate: 0.5,
      meanReturnPct: -4.04,
      oldWeight: 0.05,
      newWeight: 0,
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
    // 观测面取数有进程内缓存，用例之间必须清掉，否则后续用例拿不到新 mock 的响应
    invalidateRecoIcStats();
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
    // （观测面取数经共享 loader 多一层微任务，这里必须等到数据落位再断言）
    await waitFor(() => expect(screen.getByTestId("reco-loop-replay-note")).toBeTruthy());
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

  it("负期望硬停单列成一句，权重 0 不得渲染成「已校准」", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reco_ic_stats") {
        return Promise.resolve(statsResponse(DISABLED_LOOP));
      }
      return Promise.resolve({});
    });
    renderPanel();

    await waitFor(() => expect(screen.getByTestId("reco-loop-status-disabled")).toBeTruthy());
    // 状态列单列，且汇总里既不算「已校准」也不算「未校准」
    expect(screen.getByTestId("reco-loop-status-disabled").textContent).toBe("负期望硬停");
    expect(screen.queryByTestId("reco-loop-status-calibrated")).toBeNull();
    expect(screen.queryByTestId("reco-loop-status-insufficient")).toBeNull();
    expect(screen.getByText(/负期望硬停 1 格/)).toBeTruthy();
    expect(screen.getByText(/未校准 0 格/)).toBeTruthy();

    // 提示里必须给出「判据用的那个读数」，否则界面只在复述状态名
    fireEvent.mouseOver(screen.getByTestId("reco-loop-status-disabled"));
    await waitFor(() => expect(screen.getByText(/期窗均收益 -4\.04%/)).toBeTruthy());
    expect(screen.getByText(/≥10 单且均收益 ≤-1\.0%/)).toBeTruthy();
  });

  it("后端新增而本面板未跟上的状态露出原始码，不谎报成已知状态", async () => {
    const loop = {
      ...LOOP,
      cells: [
        { ...LOOP.cells[0], status: "brand_new_status_from_backend" },
      ],
    };
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reco_ic_stats") {
        return Promise.resolve(statsResponse(loop));
      }
      return Promise.resolve({});
    });
    renderPanel();

    await waitFor(() => expect(screen.getByTestId("reco-loop-status-unknown")).toBeTruthy());
    expect(screen.getByTestId("reco-loop-status-unknown").textContent).toBe(
      "未识别状态（brand_new_status_from_backend）",
    );
    expect(screen.queryByTestId("reco-loop-status-demoted")).toBeNull();
  });
});
