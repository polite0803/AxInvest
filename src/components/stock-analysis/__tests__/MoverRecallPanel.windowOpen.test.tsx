import i18n from "@/i18n";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { MoverRecallPanel } from "../MoverRecallPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

/** 构造与 Rust DTO 同形的响应；窗口天数取真实单源值（2/5/28/90）。 */
function makeView(collectedDays: number) {
  return {
    from: "2026-09-30",
    to: "2026-09-30",
    dataSince: "2026-09-30",
    collectedDays,
    universeSize: 46,
    universeConfirmed: 46,
    rules: [
      { period: "ultra_short", gainPct: 10, windowDays: 2, varName: "mover_gain_ultra_short" },
      { period: "short", gainPct: 20, windowDays: 5, varName: "mover_gain_short" },
      { period: "mid", gainPct: 30, windowDays: 28, varName: "mover_gain_mid" },
      { period: "long", gainPct: 40, windowDays: 90, varName: "mover_gain_long" },
    ],
    rates: { reachability: null, coverage: null, unexplainedShare: null, events: 0, misses: 0 },
    layers: [],
    misses: [],
  };
}

function renderWithProviders() {
  return render(
    <I18nextProvider i18n={i18n}>
      <MoverRecallPanel />
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
});

/**
 * 窗口未满 ≠ 无事件（`PLAN-mover-recall-attribution.md` §Phase 4-3 等待期修复）：
 * 0 事件只在**全部档位窗口已满**时才是「本区间无达标事件」的结论，
 * 未满时必须以「尚不可判定」说明并逐档标注——把「拿不到」说成「没有」即歧义。
 */
describe("MoverRecallPanel 窗口未满语义", () => {
  it("全部档位窗口未满：逐档标注 + 尚不可判定（不得显示「本区间无达标事件」）", async () => {
    invokeMock.mockResolvedValue(makeView(1));
    renderWithProviders();
    await waitFor(() => expect(screen.getByTestId("mover-recall-empty")).toBeTruthy());
    for (const p of ["ultra_short", "short", "mid", "long"]) {
      expect(screen.getByTestId(`mover-window-open-${p}`)).toBeTruthy();
    }
    const text = screen.getByTestId("mover-recall-empty").textContent ?? "";
    expect(text).toContain("窗口未满");
    expect(text).not.toContain("本区间无达标事件");
  });

  it("部分档位已满：仅未满档位标注，文案仍为尚不可判定", async () => {
    invokeMock.mockResolvedValue(makeView(3));
    renderWithProviders();
    await waitFor(() => expect(screen.getByTestId("mover-recall-empty")).toBeTruthy());
    expect(screen.queryByTestId("mover-window-open-ultra_short")).toBeNull();
    expect(screen.getByTestId("mover-window-open-short")).toBeTruthy();
    expect(screen.getByTestId("mover-window-open-mid")).toBeTruthy();
    expect(screen.getByTestId("mover-window-open-long")).toBeTruthy();
    expect(screen.getByTestId("mover-recall-empty").textContent ?? "").toContain("窗口未满");
  });

  it("全部档位窗口已满且 0 事件：才是「本区间无达标事件」结论", async () => {
    invokeMock.mockResolvedValue(makeView(90));
    renderWithProviders();
    await waitFor(() => expect(screen.getByTestId("mover-recall-empty")).toBeTruthy());
    expect(screen.queryByTestId("mover-window-open-ultra_short")).toBeNull();
    expect(screen.queryByTestId("mover-window-open-long")).toBeNull();
    expect(screen.getByTestId("mover-recall-empty").textContent ?? "").toContain("本区间无达标事件");
  });

  it("载荷缺 collectedDays ⇒ 形状守卫点名（契约漂移不得半渲染）", async () => {
    const view: Record<string, unknown> = { ...makeView(1) };
    delete view.collectedDays;
    invokeMock.mockResolvedValue(view);
    renderWithProviders();
    await waitFor(() => expect(screen.getByTestId("mover-recall-failed")).toBeTruthy());
    expect(screen.getByTestId("mover-recall-failed").textContent ?? "").toContain("collectedDays");
  });
});
