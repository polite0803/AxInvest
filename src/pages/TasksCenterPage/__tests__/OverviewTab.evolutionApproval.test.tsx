import i18n from "@/i18n";
import type { EvolutionHistoryEntry } from "@/stores/feature/stockAnalysisStore";
import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OverviewTab } from "../OverviewTab";

const flushPromises = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", async (importOriginal) => {
  // @/stores barrel 会消费 isTauri / listen 等导出，保留原模块仅替换 invoke
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: (...args: unknown[]) => invokeMock(...args) };
});

const PENDING: EvolutionHistoryEntry = {
  id: "evo-overview-1",
  planId: "plan-overview-1",
  trigger: JSON.stringify({ type: "high_error_rate", error_count: 6 }),
  evolutionType: "weight_decay",
  qualityBefore: 35,
  status: "pending_approval",
  improvementSummary: "近窗口误差率超阈，建议下调回看天数",
  createdAt: 1_790_000_000_000,
};

function renderOverview() {
  return render(
    <I18nextProvider i18n={i18n}>
      <App>
        <OverviewTab onViewRuns={() => {}} />
      </App>
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "list_pending_evolution_plans") {
      return Promise.resolve([PENDING]);
    }
    return Promise.resolve([]);
  });
  useStockAnalysisStore.getState().reset();
});

afterEach(() => {
  cleanup();
});

describe("OverviewTab P4 待审批进化健康卡", () => {
  it("挂载即拉待办，第 5 张卡显示数量；点击打开审批抽屉并列出计划", async () => {
    renderOverview();

    // 健康卡（label + Statistic value=1）
    expect(await screen.findByText("待审批进化")).toBeTruthy();
    expect(screen.getByText("1")).toBeTruthy();

    // 点击卡片 → 抽屉打开（标题 + 抽屉内待办列表）
    await act(async () => {
      fireEvent.click(screen.getByText("待审批进化"));
      await flushPromises();
    });
    expect(await screen.findByText("进化计划审批")).toBeTruthy();
    expect(screen.getAllByText("weight_decay").length).toBeGreaterThan(0);
    // 自动选中第一条待办：列表 Tag + 详情状态 Tag 两处「待审批」
    expect(screen.getAllByText("待审批").length).toBeGreaterThanOrEqual(2);
  });
});
