import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, listenMock, unlistenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}));

vi.mock("@/lib/invoke", () => ({
  invoke: invokeMock,
  listen: listenMock,
  isTauri: () => false,
}));

import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";

describe("stockAnalysisStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // 重置 store 状态
    useStockAnalysisStore.setState({
      searchKeyword: "",
      searchResults: [],
      analysisId: null,
      stockCode: "",
      stockName: "",
      analysisDate: "",
      status: "idle",
      quote: null,
      klineData: [],
      analystReports: {},
      debateRounds: [],
      riskAssessments: {},
      decision: null,
      error: null,
      history: [],
      currentStage: 0,
      llmStatus: "unknown",
      _unlisten: null,
      _searchTimer: null,
    });
    // 默认 mock invoke 返回成功
    invokeMock.mockResolvedValue({
      analysis_id: "test-id",
      stock_code: "600519",
      stock_name: "茅台",
      status: "running",
    });
    listenMock.mockResolvedValue(unlistenMock);
  });

  describe("startAnalysis", () => {
    it("starts analysis and sets loading then running status", async () => {
      await useStockAnalysisStore.getState().startAnalysis("600519", "2025-01-15", "provider-1");

      const state = useStockAnalysisStore.getState();
      expect(state.status).toBe("running");
      expect(state.stockCode).toBe("600519");
      expect(state.analysisId).toBe("test-id");
      expect(invokeMock).toHaveBeenCalledWith("start_stock_analysis", {
        stockCode: "600519",
        date: "2025-01-15",
        providerId: "provider-1",
      });
    });

    it("ignores duplicate start when status is loading", async () => {
      useStockAnalysisStore.setState({ status: "loading" });
      await useStockAnalysisStore.getState().startAnalysis("600519", "2025-01-15", "provider-1");

      expect(invokeMock).not.toHaveBeenCalled();
    });

    it("ignores duplicate start when status is running", async () => {
      useStockAnalysisStore.setState({ status: "running" });
      await useStockAnalysisStore.getState().startAnalysis("000001", "2025-01-15", "provider-1");

      expect(invokeMock).not.toHaveBeenCalled();
    });

    it("resets analysis fields before starting", async () => {
      useStockAnalysisStore.setState({
        decision: { action: "买入", confidence: 80 } as any,
        analystReports: { "test-analyst": "old report" },
        error: "previous error",
      });

      await useStockAnalysisStore.getState().startAnalysis("600519", "2025-01-15", "provider-1");

      const state = useStockAnalysisStore.getState();
      expect(state.decision).toBeNull();
      expect(state.analystReports).toEqual({});
      expect(state.error).toBeNull();
    });
  });

  describe("reset", () => {
    it("clears all state and calls unlisten", () => {
      useStockAnalysisStore.setState({
        analysisId: "test-id",
        stockCode: "600519",
        stockName: "茅台",
        status: "running",
        decision: { action: "买入", confidence: 80 } as any,
        _unlisten: unlistenMock,
      });

      useStockAnalysisStore.getState().reset();

      const state = useStockAnalysisStore.getState();
      expect(state.analysisId).toBeNull();
      expect(state.stockCode).toBe("");
      expect(state.status).toBe("idle");
      expect(state.decision).toBeNull();
      expect(unlistenMock).toHaveBeenCalled();
    });

    it("handles reset when no unlisten is registered", () => {
      useStockAnalysisStore.setState({ _unlisten: null });
      expect(() => useStockAnalysisStore.getState().reset()).not.toThrow();
    });
  });

  describe("setupEventListener", () => {
    it("registers event listener once", async () => {
      await useStockAnalysisStore.getState().setupEventListener();
      expect(listenMock).toHaveBeenCalledTimes(1);
      expect(listenMock).toHaveBeenCalledWith("stock-analysis-event", expect.any(Function));

      // 第二次调用不应重复注册
      await useStockAnalysisStore.getState().setupEventListener();
      expect(listenMock).toHaveBeenCalledTimes(1);
    });

    it("handles started event", async () => {
      let eventHandler: Function = () => {};
      listenMock.mockImplementation((_event: string, handler: Function) => {
        eventHandler = handler;
        return Promise.resolve(unlistenMock);
      });

      useStockAnalysisStore.setState({ status: "loading" });
      await useStockAnalysisStore.getState().setupEventListener();

      eventHandler({ payload: { type: "started", payload: {} } });
      expect(useStockAnalysisStore.getState().status).toBe("running");
    });

    it("handles analystReport event", async () => {
      let eventHandler: Function = () => {};
      listenMock.mockImplementation((_event: string, handler: Function) => {
        eventHandler = handler;
        return Promise.resolve(unlistenMock);
      });

      await useStockAnalysisStore.getState().setupEventListener();

      eventHandler({
        payload: {
          type: "analystReport",
          payload: { expertId: "market-analyst", reportText: "市场分析报告内容" },
        },
      });

      const state = useStockAnalysisStore.getState();
      expect(state.analystReports["market-analyst"]).toBe("市场分析报告内容");
    });

    it("handles error event with LLM fallback", async () => {
      let eventHandler: Function = () => {};
      listenMock.mockImplementation((_event: string, handler: Function) => {
        eventHandler = handler;
        return Promise.resolve(unlistenMock);
      });

      await useStockAnalysisStore.getState().setupEventListener();

      eventHandler({
        payload: { type: "error", payload: { message: "LLM 不可用" } },
      });

      const state = useStockAnalysisStore.getState();
      expect(state.error).toBe("LLM 不可用");
      expect(state.status).toBe("running"); // LLM错误时不中断，回退placeholder
    });

    it("handles decision event", async () => {
      let eventHandler: Function = () => {};
      listenMock.mockImplementation((_event: string, handler: Function) => {
        eventHandler = handler;
        return Promise.resolve(unlistenMock);
      });

      await useStockAnalysisStore.getState().setupEventListener();

      const decisionPayload = { action: "买入", confidence: 85 };
      eventHandler({ payload: { type: "decision", payload: decisionPayload } });

      const state = useStockAnalysisStore.getState();
      expect(state.decision).toEqual(decisionPayload);
      expect(state.status).toBe("completed");
    });
  });

  describe("inferStage", () => {
    // inferStage 是模块级私有函数，通过事件路由间接测试
    it("routes analyst events to stage 1", async () => {
      let eventHandler: Function = () => {};
      listenMock.mockImplementation((_event: string, handler: Function) => {
        eventHandler = handler;
        return Promise.resolve(unlistenMock);
      });

      await useStockAnalysisStore.getState().setupEventListener();
      eventHandler({
        payload: {
          type: "analystProgress",
          payload: { expertId: "market-analyst" },
        },
      });

      expect(useStockAnalysisStore.getState().currentStage).toBe(1);
    });

    it("routes debate event to stage 2", async () => {
      let eventHandler: Function = () => {};
      listenMock.mockImplementation((_event: string, handler: Function) => {
        eventHandler = handler;
        return Promise.resolve(unlistenMock);
      });

      await useStockAnalysisStore.getState().setupEventListener();
      eventHandler({
        payload: {
          type: "analystProgress",
          payload: { expertId: "debate" },
        },
      });

      expect(useStockAnalysisStore.getState().currentStage).toBe(2);
    });
  });

  describe("cancelAnalysis", () => {
    it("cancels and resets status to idle", async () => {
      useStockAnalysisStore.setState({ analysisId: "test-id", status: "running" });
      invokeMock.mockResolvedValue(undefined);

      await useStockAnalysisStore.getState().cancelAnalysis();

      expect(invokeMock).toHaveBeenCalledWith("cancel_stock_analysis", { analysisId: "test-id" });
      expect(useStockAnalysisStore.getState().status).toBe("idle");
    });
  });
});
