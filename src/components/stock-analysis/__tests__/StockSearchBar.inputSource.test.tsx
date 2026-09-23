/**
 * 回归：搜索框显示 A、却分析了 B（「输入 688315 跑出 300642」）
 *
 * 根因（2026-09-22 DB 实证）：输入框文本 searchKeyword 与待分析标的 store.stockCode
 * 是两套独立状态，同步点只有「点下拉项」和「回车且 NL 解析成功」。
 * 用户键入代码后直接点「开始分析」，按钮用的是**上一次**的 stockCode
 * ⇒ stock_analyses 里 688315 零记录，而当天该次点击后多出一条 300642。
 *
 * 本组测试锁住三条：①输入框与标的不一致时以输入框为准 ②一致时行为不变（正对照）
 * ③输入框无法识别为股票时**拒绝启动**（而不是退回上一次的标的）
 */
import i18n from "@/i18n";
import { useStockAnalysisStore } from "@/stores";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { StockSearchBar } from "../StockSearchBar";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function renderBar() {
  return render(
    <I18nextProvider i18n={i18n}>
      <App>
        <StockSearchBar />
      </App>
    </I18nextProvider>,
  );
}

/** 点击「开始分析」 */
async function clickStart() {
  const btn = screen.getByRole("button", { name: new RegExp(i18n.t("stockAnalysis.startAnalysis")) });
  await userEvent.click(btn);
}

let startAnalysisSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
  useStockAnalysisStore.getState().reset();
  startAnalysisSpy = vi.fn().mockResolvedValue(undefined);
  useStockAnalysisStore.setState({
    startAnalysis: startAnalysisSpy as never,
    getStockQuote: vi.fn().mockResolvedValue(undefined) as never,
    getStockKline: vi.fn().mockResolvedValue(undefined) as never,
  });
});

describe("StockSearchBar — 「开始分析」必须以输入框为准", () => {
  it("输入框是 688315、上次标的是 300642 ⇒ 必须分析 688315（修复前分析的是 300642）", async () => {
    useStockAnalysisStore.setState({
      stockCode: "300642",
      stockName: "透景生命",
      searchKeyword: "诺禾致源科技 (688315)",
    });

    renderBar();
    await clickStart();

    expect(startAnalysisSpy).toHaveBeenCalledTimes(1);
    expect(startAnalysisSpy.mock.calls[0][0]).toBe("688315");
    // 关键区分力断言：绝不能再出现"分析上一次标的"
    expect(startAnalysisSpy.mock.calls[0][0]).not.toBe("300642");
  });

  it("纯代码输入（无下拉选中）也对齐输入框", async () => {
    useStockAnalysisStore.setState({
      stockCode: "300642",
      stockName: "透景生命",
      searchKeyword: "688315",
    });

    renderBar();
    await clickStart();

    expect(startAnalysisSpy.mock.calls[0][0]).toBe("688315");
  });

  it("输入框与标的本来就一致时，行为不变（正对照，防误伤既有路径）", async () => {
    useStockAnalysisStore.setState({
      stockCode: "300642",
      stockName: "透景生命",
      searchKeyword: "透景生命 (300642)",
    });

    renderBar();
    await clickStart();

    expect(startAnalysisSpy).toHaveBeenCalledTimes(1);
    expect(startAnalysisSpy.mock.calls[0][0]).toBe("300642");
  });

  it("输入框无法识别为股票时拒绝启动（而不是退回上一次的标的）", async () => {
    // search_stock 返回空 ⇒ 解析失败
    invokeMock.mockResolvedValue([]);
    useStockAnalysisStore.setState({
      stockCode: "300642",
      stockName: "透景生命",
      searchKeyword: "这个不是股票名",
    });

    renderBar();
    await clickStart();

    expect(startAnalysisSpy).not.toHaveBeenCalled();
  });
});
