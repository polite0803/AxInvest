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
    useStockAnalysisStore.setState({
      searchKeyword: "",
      searchResults: [],
      analysisId: null,
      workflowId: null,
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
    invokeMock.mockResolvedValue({
      analysisId: "test-id",
      workflowId: "wf-1",
      stockCode: "600519",
      stockName: "茅台",
    });
    listenMock.mockResolvedValue(unlistenMock);
  });

  describe("startAnalysis", () => {
    it("starts analysis and sets loading then running status", async () => {
      await useStockAnalysisStore.getState().startAnalysis("600519");

      const state = useStockAnalysisStore.getState();
      expect(state.status).toBe("running");
      expect(state.stockCode).toBe("600519");
      expect(state.analysisId).toBe("test-id");
      expect(invokeMock).toHaveBeenCalledWith("get_workflow_template", {
        id: "stock-analysis",
      });
      expect(invokeMock).toHaveBeenCalledWith("run_stock_workflow", {
        stockCode: "600519",
        dryRun: false,
        asOfDate: null,
      });
    });

    it("ignores duplicate start when status is loading", async () => {
      useStockAnalysisStore.setState({ status: "loading" });
      await useStockAnalysisStore.getState().startAnalysis("600519");

      expect(invokeMock).not.toHaveBeenCalled();
    });

    it("ignores duplicate start when status is running", async () => {
      useStockAnalysisStore.setState({ status: "running" });
      await useStockAnalysisStore.getState().startAnalysis("000001");

      expect(invokeMock).not.toHaveBeenCalled();
    });

    it("resets analysis fields before starting", async () => {
      useStockAnalysisStore.setState({
        decision: { action: "买入", confidence: 80 } as any,
        analystReports: { "test-analyst": "old report" },
        error: "previous error",
      });

      await useStockAnalysisStore.getState().startAnalysis("600519");

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
    it("registers event listeners once", async () => {
      await useStockAnalysisStore.getState().setupEventListener();

      // 7 个监听 = 4 个 workflow 生命周期 + 节点内流式增量 + 监控重跑请求 + 仿真结果。
      // 数量的意义在于「面板需要感知的事件集合」，故同时逐个断言事件名 ——
      // 只断言数字会让下一次新增/删除监听时无从判断该改哪边。
      //
      // 历史值 5 少了 `workflow-step-start`：该监听是 T-1 P1(2026-09-12) 有意补齐的
      // （见 stockAnalysisStore.ts 中该 listen 上方注释）。补齐前只有 executionStore
      // 订阅它，本面板在整个执行期间拿不到「当前节点」，长 LLM 节点（1-5 分钟）里
      // 进度条静止 —— 所以这是「该改断言」而不是「该删监听」。
      //
      // 历史值 6 少了 `simulation-ready`：该监听是「仿真接入工作流」落点乙
      // (2026-09-14) 有意新增的。仿真在后端于决策落库后异步执行，结果只经该事件
      // 推送 —— 在 `workflow-completed` 里拉取会拿到空值（此时仿真尚未跑完）。
      // 与前一处同理：该改断言，不是该删监听。
      expect(listenMock).toHaveBeenCalledTimes(7);
      expect(listenMock).toHaveBeenCalledWith("workflow-step-start", expect.any(Function));
      expect(listenMock).toHaveBeenCalledWith("workflow-step-done", expect.any(Function));
      expect(listenMock).toHaveBeenCalledWith("workflow-step-delta", expect.any(Function));
      expect(listenMock).toHaveBeenCalledWith("workflow-completed", expect.any(Function));
      expect(listenMock).toHaveBeenCalledWith("workflow-error", expect.any(Function));
      expect(listenMock).toHaveBeenCalledWith("stock-monitor-t0-rerun-requested", expect.any(Function));
      expect(listenMock).toHaveBeenCalledWith("simulation-ready", expect.any(Function));

      await useStockAnalysisStore.getState().setupEventListener();
      expect(listenMock).toHaveBeenCalledTimes(7); // 已注册则第二次调用为 no-op
    });

    it("handles workflow-completed event with AgentExecutor JSON results", async () => {
      let completeHandler: (
        event: { payload: { workflowId: string; results: Record<string, { role: string; content: string }> } },
      ) => void = () => {};
      listenMock.mockImplementation((event: string, handler) => {
        if (event === "workflow-completed") { completeHandler = handler; }
        return Promise.resolve(unlistenMock);
      });

      await useStockAnalysisStore.getState().setupEventListener();

      completeHandler({
        payload: {
          workflowId: "wf-1",
          results: {
            "a-market-analyst": { role: "market-analyst", content: "技术面看好" },
            "bull-r1": { role: "bull-researcher", content: "多方看涨" },
            "bear-r1": { role: "bear-researcher", content: "空方谨慎" },
            "risk-agg": { role: "aggressive-debator", content: "高风险" },
            "trader": { role: "trader", content: "建议轻仓" },
            "portfolio-mgr": {
              role: "portfolio-manager",
              content: JSON.stringify({ action: "BUY", confidence: 85 }),
            },
          },
        },
      });

      const state = useStockAnalysisStore.getState();
      expect(state.status).toBe("completed");
      expect(state.analystReports["market-analyst"]).toBe("技术面看好");
      expect(state.debateRounds).toHaveLength(1);
      expect(state.debateRounds[0]).toEqual({ round: 1, bull: "多方看涨", bear: "空方谨慎" });
    });

    it("handles workflow-error event", async () => {
      let errorHandler: (event: { payload: { workflowId: string; error: string } }) => void = () => {};
      listenMock.mockImplementation((event: string, handler) => {
        if (event === "workflow-error") { errorHandler = handler; }
        return Promise.resolve(unlistenMock);
      });

      await useStockAnalysisStore.getState().setupEventListener();

      errorHandler({
        payload: { workflowId: "wf-1", error: "网络超时" },
      });

      const state = useStockAnalysisStore.getState();
      expect(state.error).toBe("网络超时");
      expect(state.status).toBe("error");
    });
  });

  describe("cancelAnalysis", () => {
    it("cancels and resets status to idle", async () => {
      useStockAnalysisStore.setState({ workflowId: "wf-1", status: "running" });
      invokeMock.mockResolvedValue(undefined);

      await useStockAnalysisStore.getState().cancelAnalysis();

      expect(invokeMock).toHaveBeenCalledWith("cancel_stock_workflow", { workflowId: "wf-1" });
      expect(useStockAnalysisStore.getState().status).toBe("cancelled");
    });
  });
});
