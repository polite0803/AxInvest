import i18n from "@/i18n";
import { useStockAnalysisStore } from "@/stores";
import { render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SimulationTabContent } from "../SimulationTabContent";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function renderTab() {
  return render(
    <I18nextProvider i18n={i18n}>
      <SimulationTabContent />
    </I18nextProvider>,
  );
}

/** 取「市场微观结构」面板里的股票代码输入框（antd Form.Item 生成的 input） */
function codeInputValue(container: HTMLElement): string | undefined {
  return Array.from(container.querySelectorAll("input"))
    .map((el) => el.value)
    .find((v) => /^\d{6}$/.test(v));
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  useStockAnalysisStore.getState().reset();
});

describe("SimulationTabContent — 仿真标签", () => {
  it("把当前分析标的与现价带入上下文条，不必手填代码", () => {
    useStockAnalysisStore.setState({
      stockCode: "600519",
      stockName: "贵州茅台",
      quote: { code: "600519", name: "贵州茅台", price: 1680.5 } as never,
    });
    renderTab();
    expect(screen.getByText(/贵州茅台（600519）/)).toBeTruthy();
    // 现价以「元」回显（上下文条 + 面板参考价旁各一处，故用 getAllByText）
    expect(screen.getAllByText(/1680\.50/).length).toBeGreaterThan(0);
  });

  it("标的自动进入子面板，且现价按「元 → 分」换算", () => {
    useStockAnalysisStore.setState({
      stockCode: "600519",
      stockName: "贵州茅台",
      quote: { code: "600519", name: "贵州茅台", price: 1680.5 } as never,
    });
    const { container } = renderTab();
    // 代码带入：000001 之类不再需要手填
    expect(codeInputValue(container)).toBe("600519");
    // 1680.50 元 ⇒ 168050 分（后端单位是分）
    expect(screen.getByDisplayValue("168050")).toBeTruthy();
  });

  it("未选标的时给出明确指引，而不是留一个空面板（反向断言）", () => {
    const { container } = renderTab();
    expect(screen.getByText(/尚未选择标的/)).toBeTruthy();
    // 兜底标的仍存在（面板可用），但不是当前分析标的
    expect(codeInputValue(container)).toBe("000001");
  });

  it("三个仿真面板作为二级标签同时可达", () => {
    renderTab();
    // i18n 是真实加载的，用真实译文构造 accessible name（精确匹配，避免「仿真」与「量化仿真」互相命中）
    const expected = [
      `🏭 ${i18n.t("stockAnalysis.backtest.tabSimulation")}`,
      `🎲 ${i18n.t("stockAnalysis.backtest.tabMonteCarlo")}`,
      `🤖 ${i18n.t("stockAnalysis.backtest.tabQuantSim")}`,
    ];
    for (const name of expected) {
      expect(screen.getByRole("tab", { name })).toBeTruthy();
    }
  });
});
