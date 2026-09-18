import i18n from "@/i18n";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RecommendationPanel } from "../RecommendationPanel";

/**
 * 兜底合成候选（synthetic）的「可见性」防回归测试。
 *
 * 背景：面板过滤 `p.synthetic` 本身是正确的（占位数据会污染判断），
 * 但 2026-09-12 之前是**静默过滤** ——
 *   ① 汇总条仅在 `real === 0` 时出现，于是当资本策略擦边产出 10 条真实、
 *      另外 100 条兜底被隐藏时，条件不成立、面板毫无提示；
 *   ② 空风格一律渲染「暂无推荐数据」，无法区分「真无信号」与「仅兜底被隐藏」。
 * 两者叠加使「上游数据链断裂」看起来只是「风格没选到票」，故障极难定位。
 *
 * 本测试锁定修复后的三条契约：
 *   A. 兜底候选不进入列表；
 *   B. 只要存在被隐藏的兜底候选，汇总条就必须出现（不再要求 real === 0）；
 *   C. 仅有兜底的风格，空状态文案与「真的没信号」必须不同。
 */
const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

type Pick = Record<string, unknown>;

function makePick(code: string, name: string, synthetic: boolean): Pick {
  return {
    stockCode: code,
    stockName: name,
    style: "trend",
    period: "short",
    price: 10,
    entryLow: 9.8,
    entryHigh: 10.2,
    stopLoss: 9.5,
    targetPrice: 11,
    positionPct: 5,
    holdingDays: 5,
    confidence: 70,
    reasons: [],
    riskNotes: [],
    synthetic,
  };
}

function mockResponse(picks: Record<string, Pick[]>) {
  invokeMock.mockResolvedValue({
    period: "short",
    picks,
    disabledStyles: [],
    degradedStyles: [],
    generatedAt: Date.now(),
    rawSeedPoolSize: 90,
    mode: "live",
  });
}

function renderPanel() {
  return render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <RecommendationPanel />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  useTimeAnchorStore.setState({
    asOfDate: null,
    mode: "live",
    tourSeen: true,
    pendingLiveConfirm: false,
  });
});

describe("RecommendationPanel — 兜底候选可见性", () => {
  it("A. 兜底候选不进入列表，仅真实候选渲染", async () => {
    mockResponse({
      trend: [
        makePick("600001", "真实甲", false),
        makePick("600002", "兜底乙", true),
        makePick("600003", "兜底丙", true),
      ],
    });
    renderPanel();

    await waitFor(() => expect(screen.getAllByText("真实甲").length).toBeGreaterThan(0));
    expect(screen.queryByText("兜底乙")).toBeNull();
    expect(screen.queryByText("兜底丙")).toBeNull();
  });

  it("B. 只要有兜底被隐藏，汇总条就出现（real > 0 时同样出现）", async () => {
    mockResponse({
      trend: [makePick("600001", "真实甲", false)],
      capital: [
        makePick("600002", "兜底乙", true),
        makePick("600003", "兜底丙", true),
        makePick("600004", "兜底丁", true),
      ],
    });
    renderPanel();

    // 关键：real = 1（> 0），旧实现下汇总条不会出现
    await waitFor(() => expect(screen.getByText(/条真实候选 · 3 条兜底候选已隐藏/)).toBeTruthy());
  });

  it("C. 仅含兜底的风格，空状态文案区别于「真的没信号」", async () => {
    mockResponse({
      trend: [makePick("600001", "真实甲", false)],
      watchlist: [
        makePick("600005", "兜底戊", true),
        makePick("600006", "兜底己", true),
      ],
    });
    renderPanel();

    await waitFor(() => expect(screen.getAllByText("真实甲").length).toBeGreaterThan(0));

    // watchlist 分组默认不展开（无真实产出），手动展开以读取空状态文案
    const header = screen.getByText(/自选/);
    fireEvent.click(header);

    await waitFor(() => expect(screen.getByText(/该风格暂无真实命中（2 条兜底候选已隐藏）/)).toBeTruthy());
  });

  it("D. 完全没有兜底候选时，汇总条不应出现", async () => {
    mockResponse({
      trend: [makePick("600001", "真实甲", false), makePick("600007", "真实庚", false)],
    });
    renderPanel();

    await waitFor(() => expect(screen.getAllByText("真实甲").length).toBeGreaterThan(0));
    expect(screen.queryByText(/兜底候选已隐藏/)).toBeNull();
  });
});
