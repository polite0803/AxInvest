// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 跨市场面板的失败态呈现约束
 *
 * 钉死一条不变式：上游取数失败**不得**显示成「暂无数据」。
 * 本机对 push2* 数据源存在累积性 RST，「拿不到」是常态而非异常路径，
 * 若塌成空态就会被读成「这个市场没有行情」。
 */

import { invoke } from "@/lib/invoke";
import { useCrossMarketStore } from "@/stores";
import type { KLine } from "@/types";
import { render, screen } from "@testing-library/react";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CrossMarketDashboard } from "../CrossMarketDashboard";

// 只替 invoke：@/stores 桶里其它 store 在模块初始化期就用 listen/isTauri，
// 整体 mock 会让套件起不来（实测两次报错换了符号名）。
vi.mock(import("@/lib/invoke"), async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, invoke: vi.fn() };
});

vi.mock("@/components/stock-analysis/KLineChart", () => ({
  KLineChart: () => <div data-testid="kline-chart" />,
}));

const ZH: Record<string, string> = {
  "crossMarket.title": "跨市场行情",
  "crossMarket.subtitle": "外部市场参照",
  "crossMarket.codePlaceholder": "代码",
  "crossMarket.fetchQuote": "取行情",
  "crossMarket.noQuotes": "暂无行情记录",
  "crossMarket.klineTitle": "{{code}} K 线",
  "crossMarket.noKline": "暂无 K 线",
  "crossMarket.benchmarkTitle": "基准指数对比",
  "crossMarket.noBenchmark": "暂无基准指数数据",
  "crossMarket.forexTitle": "外汇",
  "crossMarket.noForex": "暂无外汇数据",
  "crossMarket.latestClose": "最新收盘",
  "crossMarket.klineCount": "K 线根数",
  "crossMarket.loading": "加载中",
  "crossMarket.refresh": "刷新",
  "crossMarket.code": "代码",
  "crossMarket.name": "名称",
  "crossMarket.price": "价格",
  "crossMarket.changePct": "涨跌幅",
  "crossMarket.totalMv": "市值",
  "error.loadFailed": "加载失败，请重试",
};

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => ZH[key] ?? key }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const BENCH = "SPX:daily:120";

function resetStore() {
  useCrossMarketStore.setState({
    intlQuotes: {},
    intlKlines: {},
    benchmarkKlines: {},
    forexKlines: {},
    loadingQuote: false,
    loadingKline: false,
    loadingBenchmark: false,
    loadingForex: false,
    errors: { quote: null, kline: null, benchmark: null, forex: null },
  });
}

function renderPanel() {
  return render(
    <App>
      <CrossMarketDashboard />
    </App>,
  );
}

describe("CrossMarketDashboard 取数失败态", () => {
  beforeEach(() => {
    resetStore();
    vi.mocked(invoke).mockReset();
  });

  it("上游被断时显示失败态，且不冒充「暂无数据」", async () => {
    vi.mocked(invoke).mockRejectedValue("schannel: server closed abruptly");
    renderPanel();

    const alerts = await screen.findAllByText("加载失败，请重试");
    expect(alerts.length).toBeGreaterThan(0);
    // 后端原文（无结构化码时按裸串兜底）必须仍可见，不能被通用文案吃掉；
    // 挂载时基准 + 外汇两区各自预取 ⇒ 两条独立失败提示
    expect(await screen.findAllByText("schannel: server closed abruptly")).toHaveLength(2);
    expect(screen.queryByText("暂无基准指数数据")).toBeNull();
    expect(screen.queryByText("暂无外汇数据")).toBeNull();
  });

  it("取到空数组时才是空态，且不得出现失败提示", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    renderPanel();

    expect(await screen.findByText("暂无基准指数数据")).toBeTruthy();
    expect(screen.getByText("暂无外汇数据")).toBeTruthy();
    expect(screen.queryByText("加载失败，请重试")).toBeNull();
  });

  it("取到数据时正常渲染图表，两种缺席态都不出现", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "get_benchmark_kline" || cmd === "get_forex_kline"
        ? ([{ close: 1 }, { close: 2 }] as unknown as KLine[])
        : []
    );
    renderPanel();

    await screen.findAllByTestId("kline-chart");
    expect(useCrossMarketStore.getState().benchmarkKlines[BENCH]).toHaveLength(2);
    expect(screen.queryByText("加载失败，请重试")).toBeNull();
    expect(screen.queryByText("暂无基准指数数据")).toBeNull();
  });

  it("一区失败不得覆盖另一区的状态", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "get_benchmark_kline") {
        throw "benchmark upstream reset";
      }
      return [{ close: 7 }] as unknown as KLine[];
    });
    renderPanel();

    expect(await screen.findByText("benchmark upstream reset")).toBeTruthy();
    // 外汇区成功：既不出现失败提示，也不该被 benchmark 的失败连带打空
    expect(useCrossMarketStore.getState().errors.forex).toBeNull();
    expect(useCrossMarketStore.getState().errors.benchmark).toBe("benchmark upstream reset");
    expect(screen.queryByText("暂无外汇数据")).toBeNull();
  });
});
