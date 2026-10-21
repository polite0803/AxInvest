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
      expect(state.debateRounds[0]).toEqual({ tier: null, round: 1, bull: "多方看涨", bear: "空方谨慎" });
    });

    it("workflow-step-done：四档辩论按 (tier, round) 各自落库，不互相覆盖", async () => {
      let stepDoneHandler: (event: { payload: Record<string, unknown> }) => void = () => {};
      listenMock.mockImplementation((event: string, handler) => {
        if (event === "workflow-step-done") { stepDoneHandler = handler; }
        return Promise.resolve(unlistenMock);
      });

      await useStockAnalysisStore.getState().setupEventListener();

      const tiers = ["ultra_short", "short", "mid", "long"];
      for (const tier of tiers) {
        stepDoneHandler({
          payload: {
            nodeId: `bull-r1--${tier}`,
            status: "completed",
            output: `多方-${tier}`,
            totalNodes: 100,
            completedNodes: 1,
          },
        });
        stepDoneHandler({
          payload: {
            nodeId: `bear-r1--${tier}`,
            status: "completed",
            output: `空方-${tier}`,
            totalNodes: 100,
            completedNodes: 1,
          },
        });
      }

      const rounds = useStockAnalysisStore.getState().debateRounds;
      // 修复前：四档的 id 都被 parseInt 成 round=1，findIndex(round===1) 让后完成的档
      // 覆盖先完成的档 ⇒ 这里只会剩 1 条（而且是最后一次写入的那档）。
      expect(rounds.map((r) => [r.tier, r.round, r.bull, r.bear])).toEqual([
        ["ultra_short", 1, "多方-ultra_short", "空方-ultra_short"],
        ["short", 1, "多方-short", "空方-short"],
        ["mid", 1, "多方-mid", "空方-mid"],
        ["long", 1, "多方-long", "空方-long"],
      ]);
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

    it("后端取消失败也必须本地收口，否则「停止」对卡住的一轮永久失效", async () => {
      useStockAnalysisStore.setState({ workflowId: "wf-1", status: "running" });
      invokeMock.mockRejectedValue(new Error("WorkflowNotFound"));

      await expect(useStockAnalysisStore.getState().cancelAnalysis()).resolves.toBeUndefined();

      expect(useStockAnalysisStore.getState().status).toBe("cancelled");
      expect(useStockAnalysisStore.getState().workflowId).toBeNull();
    });
  });

  describe("错误退出后的状态收口（不得停在「分析中」）", () => {
    /** 捕获指定事件的处理器；同时让其它 invoke 走正常返回。 */
    function captureHandler(eventName: string) {
      const ref: { handler: (event: { payload: Record<string, unknown> }) => void } = {
        handler: () => {},
      };
      listenMock.mockImplementation((event: string, handler) => {
        if (event === eventName) { ref.handler = handler; }
        return Promise.resolve(unlistenMock);
      });
      return ref;
    }

    it("workflow-error 先于 run_stock_workflow 响应到达时，不被回写成 running", async () => {
      const captured = captureHandler("workflow-error");
      let resolveRun: (value: unknown) => void = () => {};
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === "run_stock_workflow") {
          return new Promise((resolve) => {
            resolveRun = resolve;
          });
        }
        if (cmd === "get_workflow_template") { return Promise.resolve({ variables: [] }); }
        return Promise.resolve(undefined);
      });

      const started = useStockAnalysisStore.getState().startAnalysis("600519");
      await vi.waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("run_stock_workflow", expect.anything());
      });

      captured.handler({
        payload: {
          workflowId: "wf-1",
          code: "STOCK_WORKFLOW_EXEC_FAILED",
          category: "unrecoverable",
          detail: "启动即失败",
        },
      });
      expect(useStockAnalysisStore.getState().status).toBe("error");

      // 后端在 spawn 之后才返回：响应晚于终态事件是真实时序
      resolveRun({ analysisId: "a-1", workflowId: "wf-1", stockCode: "600519", stockName: "茅台" });
      await started;

      const state = useStockAnalysisStore.getState();
      expect(state.status).toBe("error");
      expect(state.error).toBeTruthy();
    });

    it("workflow-error 的部分产物解析抛异常时，终态仍收口为 error", async () => {
      const captured = captureHandler("workflow-error");
      await useStockAnalysisStore.getState().setupEventListener();

      const poisoned: Record<string, unknown> = {};
      Object.defineProperty(poisoned, "content", {
        enumerable: true,
        get() {
          throw new Error("malformed node output");
        },
      });

      expect(() =>
        captured.handler({
          payload: {
            workflowId: "wf-1",
            code: "STOCK_WORKFLOW_EXEC_FAILED",
            category: "unrecoverable",
            results: { "a-market-analyst": poisoned },
          },
        })
      ).not.toThrow();
      expect(useStockAnalysisStore.getState().status).toBe("error");
    });

    it("workflow-completed 的部分产物解析抛异常时，终态仍收口为 completed", async () => {
      const captured = captureHandler("workflow-completed");
      await useStockAnalysisStore.getState().setupEventListener();

      const poisoned: Record<string, unknown> = {};
      Object.defineProperty(poisoned, "content", {
        enumerable: true,
        get() {
          throw new Error("malformed node output");
        },
      });

      expect(() =>
        captured.handler({
          payload: { workflowId: "wf-1", results: { "a-market-analyst": poisoned } },
        })
      ).not.toThrow();
      expect(useStockAnalysisStore.getState().status).toBe("completed");
    });
  });
});

// 重跑路径的「手工归一」历史上反复漏字段（2026-09-21 漏 dataGaps / weightAdjustments）。
// 后端 rerun_decision 会重新注入 crossCheck（decision.rs::inject_reco_crosscheck），
// store 若不整体透传 ⇒ 重跑后「趋势智选 vs 工作流」分歧报告与其归因整体消失。
describe("rerunDecision 透传跨系统互证", () => {
  it("保留 crossCheck.divergence 归因（缺此拷贝时本断言红）", async () => {
    useStockAnalysisStore.setState({ decision: null, status: "completed" });
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd !== "rerun_decision") { return {}; }
      return {
        analysis_id: "a-1",
        decision: {
          action: "观望",
          positionPct: 0,
          confidence: 38,
          riskLevel: "中风险",
          reasoning: "决策=观望 置信=38",
          data_gaps: ["催化剂评估(a-catalyst)"],
          crossCheck: {
            divergent: true,
            recoConfidence: 61,
            recoPositionPct: 4.925,
            decisionAction: "观望",
            decisionPositionPct: 0,
            divergence: {
              drivers: ["negative_legs", "below_hold_threshold", "dcf_leg_excluded", "data_gap"],
              legs: [{ name: "valuation", sigma: -0.485, weight: 0.21 }],
              posteriorRaw: 38,
              posteriorEffective: 38,
              holdThreshold: 48,
            },
          },
        },
        llm_decision_json: null,
      };
    });

    await useStockAnalysisStore.getState().rerunDecision("a-1");

    const d = useStockAnalysisStore.getState().decision;
    expect(d?.crossCheck?.divergent).toBe(true);
    expect(d?.crossCheck?.divergence?.legs?.[0]?.name).toBe("valuation");
    expect(d?.crossCheck?.divergence?.drivers).toContain("dcf_leg_excluded");
    expect(d?.dataGaps).toEqual(["催化剂评估(a-catalyst)"]);
  });
});
