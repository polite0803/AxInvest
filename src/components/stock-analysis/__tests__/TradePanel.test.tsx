import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// ⚠ mock 必须提供 `initReactI18next`：本文件的组件 `TradePanel.tsx:3` import 了
// `@/lib/errorI18n`，而后者（2026-09-13 起）在**顶层** `import i18n from "@/i18n"`
// ⇒ 加载链会执行 `src/i18n/index.ts:22` 的 `i18n.use(initReactI18next)`。
// 若 mock 只给 `useTranslation`，这一步拿到 undefined 并抛
// 「No "initReactI18next" export is defined on the "react-i18next" mock」——
// 报错发生在模块加载期 ⇒ 该文件 **0 个用例执行**、整文件被标记 FAIL。
// 形态照抄仓库主流写法（如 DecisionTimelinePanel.test.tsx:7，共 20 处先例）。
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const storeState: Record<string, unknown> = {
  stockCode: "",
  stockName: "",
  decision: null,
};

vi.mock("@/stores", () => ({
  useStockAnalysisStore: (selector: (s: typeof storeState) => unknown) => selector(storeState),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  isTauri: () => false,
}));

import { invoke } from "@/lib/invoke";
import { TradePanel } from "../TradePanel";

const invokeMock = vi.mocked(invoke);

/** 按命令名分派的替身：开关回读返回布尔，其余列表类返回空数组。 */
function stubInvoke(tradingEnabled: boolean) {
  invokeMock.mockImplementation(
    ((cmd: string) => {
      if (cmd === "get_trading_enabled") { return Promise.resolve(tradingEnabled); }
      return Promise.resolve([]);
    }) as typeof invoke,
  );
}

describe("TradePanel", () => {
  beforeEach(() => {
    stubInvoke(false);
  });

  it("renders without crashing", async () => {
    storeState.stockCode = "";
    storeState.stockName = "";
    storeState.decision = null;

    const { container } = render(<TradePanel />);
    expect(container).toBeTruthy();
  });

  it("renders trade panel heading", () => {
    storeState.stockCode = "600519";
    storeState.stockName = "茅台";
    storeState.decision = null;

    const { container } = render(<TradePanel />);
    expect(container.textContent).toBeTruthy();
  });

  it("renders stock code and name from store", async () => {
    storeState.stockCode = "600519";
    storeState.stockName = "贵州茅台";
    storeState.decision = {
      action: "买入",
      confidence: 85,
      targetPrice: 1850,
      positionPct: 15,
    };

    const { container } = render(<TradePanel />);
    // 组件应渲染，不崩溃
    expect(container).toBeTruthy();
    // 挂载回读与「同步分析页代码」的预填都是异步落地：必须在**测试内**等它们完成，
    // 否则更新发生在测试函数返回之后、afterEach 之前 ⇒ React 报 not wrapped in act。
    await act(async () => {});
  });

  // 开关此前是纯本地 useState(false)，只有写口没有读口 ⇒ 后端仍启用时界面显示关闭。
  // 默认态就是 false，所以「变成开」只能来自挂载回读这一条新路径。
  it("挂载回读后端为启用时，开关显示为已开启", async () => {
    stubInvoke(true);
    const { container } = render(<TradePanel />);
    await waitFor(() => {
      expect(container.querySelector(".ant-switch-checked")).toBeTruthy();
    });
  });

  // 负控：后端说未启用时不得被打开（证明上一条不是无条件置 true）
  it("负控：后端返回未启用时开关保持关闭", async () => {
    stubInvoke(false);
    const { container } = render(<TradePanel />);
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_trading_enabled");
    });
    expect(container.querySelector(".ant-switch-checked")).toBeNull();
  });

  // 读取失败必须显式告知：此前 catch 是空的，三块数据整体消失后看起来像「本来就没有交易记录」。
  it("读取失败时显示提示，而不是静默留空", async () => {
    invokeMock.mockImplementation(
      ((cmd: string) => {
        if (cmd === "get_trading_enabled") { return Promise.resolve(true); }
        return Promise.reject(new Error("boom"));
      }) as typeof invoke,
    );
    const { container } = render(<TradePanel />);
    await waitFor(() => {
      expect(container.textContent).toContain("stockAnalysis.settings.vendor.connectionFailed");
    });
  });

  // 负控：读取成功（哪怕是空数据）不得出现失败提示 —— 否则第一段就成了恒真门。
  it("负控：读取成功时不显示失败提示", async () => {
    stubInvoke(true);
    const { container } = render(<TradePanel />);
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_trading_enabled");
    });
    await waitFor(() => {
      expect(container.querySelector(".ant-switch-checked")).toBeTruthy();
    });
    expect(container.textContent).not.toContain("stockAnalysis.settings.vendor.connectionFailed");
  });

  // 交易表此前没有日期列：批量导入的历史成交按入库时刻（created_at=now）倒序排，
  // 看不到成交日就无从分辨哪条是哪天的。
  it("交易列表带成交日期列", async () => {
    invokeMock.mockImplementation(
      ((cmd: string) => {
        if (cmd === "get_trading_enabled") { return Promise.resolve(true); }
        if (cmd === "list_trades") {
          return Promise.resolve([{
            id: "t1",
            stockCode: "600519",
            stockName: "贵州茅台",
            direction: "buy",
            price: 1680.5,
            quantity: 100,
            tradeDate: "2026-10-14",
            tradeTime: "10:30",
            realizedPnl: null,
            notes: null,
            createdAt: 1,
          }]);
        }
        return Promise.resolve([]);
      }) as typeof invoke,
    );

    const { container } = render(<TradePanel />);
    await waitFor(() => expect(container.textContent).toContain("600519"));
    // 表头与成交日都要在 —— 只断表头会被「列存在但渲染空」蒙混过去
    expect(container.textContent).toContain("trade.import.columns.tradeDate");
    expect(container.textContent).toContain("2026-10-14");
  });
});
