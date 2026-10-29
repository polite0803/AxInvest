// SPDX-License-Identifier: AGPL-3.0-only

import type {
  AnalysisSummary,
  BackgroundTaskInfo,
  MoverRecallBatchState,
  PipelineRun,
  ScheduledTask,
  TimeTravelBatchSummary,
} from "@/types";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backgroundTasksAdapter, mapBackgroundTask } from "../adapters/backgroundTasks.adapter";
import { cronAdapter, mapExecutionRecord, mapLastRun, mapRunningTask } from "../adapters/cron.adapter";
import { mapBatchSummary as mapMoverBatchSummary } from "../adapters/moverRecallAnalysis.adapter";
import { mapPipelineRun } from "../adapters/pipeline.adapter";
import { aggregateRuns } from "../adapters/registry";
import { mapStockAnalysisRun, stockAnalysisAdapter } from "../adapters/stockAnalysis.adapter";
import { mapBatchSummary } from "../adapters/timeTravel.adapter";
import { isNextRunStale } from "../model";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("@/i18n", () => ({ default: { t: (k: string) => k } }));

function makeTask(over: Partial<ScheduledTask>): ScheduledTask {
  return {
    id: "t1",
    name: "定时任务一",
    description: "",
    taskType: "custom",
    cronExpression: "0 9 * * *",
    intervalSeconds: null,
    nextRunAt: null,
    lastRunAt: "2026-01-01T00:00:00.000Z",
    lastResult: {
      success: true,
      output: null,
      error: null,
      durationMs: 1200,
      skipped: false,
    },
    status: "active",
    config: {
      timeoutSeconds: 300,
      retryOnFailure: false,
      maxRetries: 0,
      retryDelaySeconds: 30,
      notificationEnabled: true,
      runOnStartup: false,
    },
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    workflowId: null,
    ...over,
  };
}

beforeEach(() => invokeMock.mockReset());

describe("cron.mapLastRun", () => {
  it("lastTrigger 如实映射；缺失（旧记录）→ unknown", () => {
    expect(mapLastRun(makeTask({ lastTrigger: "manual" })).trigger).toBe("manual");
    expect(mapLastRun(makeTask({ lastTrigger: "schedule" })).trigger).toBe("schedule");
    expect(mapLastRun(makeTask({})).trigger).toBe("unknown");
  });

  it("runningSince → running 行（不再只看 lastResult）", () => {
    const run = mapRunningTask(
      makeTask({ runningSince: 1_700_000_000_000, runningTrigger: "manual" }),
    );
    expect(run.state).toBe("running");
    expect(run.startedAt).toBe(1_700_000_000_000);
    expect(run.trigger).toBe("manual");
    expect(run.finishedAt).toBeUndefined();
  });

  it("成功映射 succeeded + 耗时回推开始时间", () => {
    const run = mapLastRun(makeTask({}));
    expect(run.state).toBe("succeeded");
    expect(run.source).toBe("scheduled-cron");
    expect(run.capabilities.retry).toBe(false);
    expect(run.durationMs).toBe(1200);
    expect(run.startedAt).toBe(run.finishedAt! - 1200);
  });

  it("失败结果映射 failed + error", () => {
    const run = mapLastRun(
      makeTask({
        lastResult: {
          success: false,
          output: null,
          error: "执行失败",
          durationMs: 10,
        },
      }),
    );
    expect(run.state).toBe("failed");
    expect(run.error?.message).toBe("执行失败");
  });

  it("skipped 映射 succeeded + skipped badge", () => {
    const run = mapLastRun(
      makeTask({
        lastResult: {
          success: true,
          output: null,
          error: null,
          durationMs: 1,
          skipped: true,
        },
      }),
    );
    expect(run.state).toBe("succeeded");
    expect(run.badges).toEqual(["skipped"]);
  });

  it("partial（部分完成）不落 failed，主状态 succeeded + partial badge", () => {
    const run = mapLastRun(
      makeTask({
        lastResult: {
          success: false,
          output: "候选池扫描完成: 12 只入队（达到单轮墙钟预算提前收尾，属部分完成）",
          error: null,
          durationMs: 3,
          partial: true,
        },
      }),
    );
    expect(run.state).toBe("succeeded");
    expect(run.badges).toEqual(["partial"]);
  });

  it("绑定 workflow 的任务开放 retry", () => {
    const run = mapLastRun(
      makeTask({ taskType: "custom", workflowId: "opc-demand-discovery" }),
    );
    expect(run.capabilities.retry).toBe(true);
  });

  it("list 只返回有最近结果的任务", async () => {
    invokeMock.mockResolvedValue([
      makeTask({ id: "ran" }),
      makeTask({ id: "never", lastRunAt: null, lastResult: null }),
    ]);
    const runs = await cronAdapter.list!();
    expect(runs.map((r) => r.nativeId)).toEqual(["ran"]);
  });
});

describe("cron.mapExecutionRecord（兜底轮区分）", () => {
  const rec = (
    over: Partial<Parameters<typeof mapExecutionRecord>[1]>,
  ): Parameters<typeof mapExecutionRecord>[1] => ({
    id: "h1",
    task_id: "t1",
    started_at: 1,
    completed_at: 2,
    success: true,
    output: null,
    error: null,
    duration_ms: 1,
    ...over,
  });

  it("output 带 [系统兜底轮 前缀 → trigger=system", () => {
    const run = mapExecutionRecord(
      "t1",
      rec({ output: "[系统兜底轮·每6h] 完成 3 条" }),
      makeTask({}),
    );
    expect(run.trigger).toBe("system");
  });

  it("历史行自带 trigger=schedule → schedule", () => {
    const run = mapExecutionRecord(
      "t1",
      rec({ output: "候选池扫描完成: 3 只入队", trigger: "schedule" }),
      makeTask({}),
    );
    expect(run.trigger).toBe("schedule");
  });

  it("旧行缺 trigger → unknown（不再一律猜成定时）", () => {
    const run = mapExecutionRecord(
      "t1",
      rec({ output: "候选池扫描完成: 3 只入队" }),
      makeTask({}),
    );
    expect(run.trigger).toBe("unknown");
  });
});

describe("isNextRunStale（next_run 停滞标记）", () => {
  const NOW = Date.parse("2026-10-16T10:00:00+08:00");

  it("档期过期且距上次运行超 24h → stale", () => {
    expect(
      isNextRunStale(
        makeTask({
          nextRunAt: "2026-10-14T02:00:00+08:00",
          lastRunAt: "2026-10-14T02:00:00+08:00",
        }),
        NOW,
      ),
    ).toBe(true);
  });

  it("next_run_at=0 序列化成的 1970 形态同样命中", () => {
    expect(
      isNextRunStale(
        makeTask({
          nextRunAt: "1970-01-01T00:00:00+00:00",
          lastRunAt: "2026-10-10T02:00:00+08:00",
        }),
        NOW,
      ),
    ).toBe(true);
  });

  it("未来档期 / 禁用 / 从未运行 不标", () => {
    expect(
      isNextRunStale(
        makeTask({
          nextRunAt: "2026-10-17T02:00:00+08:00",
          lastRunAt: "2026-10-15T02:00:00+08:00",
        }),
        NOW,
      ),
    ).toBe(false);
    expect(
      isNextRunStale(
        makeTask({ status: "paused", nextRunAt: "1970-01-01T00:00:00+00:00" }),
        NOW,
      ),
    ).toBe(false);
    expect(
      isNextRunStale(
        makeTask({ lastRunAt: null, nextRunAt: "1970-01-01T00:00:00+00:00" }),
        NOW,
      ),
    ).toBe(false);
  });
});

describe("pipeline.mapPipelineRun", () => {
  const base: PipelineRun = {
    id: "r1",
    runDate: "2026-01-02",
    asOfDate: null,
    status: "running",
    startedAt: 1000,
    completedAt: null,
    errorMessage: null,
    summary: null,
  };

  it("running 状态", () => {
    const run = mapPipelineRun(base);
    expect(run.state).toBe("running");
    expect(run.source).toBe("stock-pipeline");
    expect(run.capabilities.viewIO).toBe(true);
  });

  it("completed 无失败 → succeeded", () => {
    const run = mapPipelineRun({
      ...base,
      status: "completed",
      completedAt: 2000,
      summary: {
        pipelineDate: "2026-01-02",
        discovery: { candidatesFound: 0 },
        analysis: {
          newAnalyzed: 1,
          newFailed: 0,
          reassessed: 0,
          reassessFailed: 0,
        },
        decisions: { buy: 0, hold: 0, watch: 0, sell: 0 },
        reflectionScheduled: 0,
        note: "",
      },
    });
    expect(run.state).toBe("succeeded");
    expect(run.badges).toBeUndefined();
    expect(run.durationMs).toBe(1000);
  });

  it("completed 但有失败 → succeeded + partial badge", () => {
    const run = mapPipelineRun({
      ...base,
      status: "completed",
      completedAt: 2000,
      summary: {
        pipelineDate: "2026-01-02",
        discovery: { candidatesFound: 0 },
        analysis: {
          newAnalyzed: 2,
          newFailed: 1,
          reassessed: 0,
          reassessFailed: 0,
        },
        decisions: { buy: 0, hold: 0, watch: 0, sell: 0 },
        reflectionScheduled: 0,
        note: "",
      },
    });
    expect(run.state).toBe("succeeded");
    expect(run.badges).toEqual(["partial"]);
  });

  it("failed + errorMessage", () => {
    const run = mapPipelineRun({ ...base, status: "failed", errorMessage: "炸了" });
    expect(run.state).toBe("failed");
    expect(run.error?.message).toBe("炸了");
  });

  it("interrupted → canceled + interrupted badge（进程退出遗留行）", () => {
    const run = mapPipelineRun({
      ...base,
      status: "interrupted",
      errorMessage: "应用重启：管道运行中进程退出",
    });
    expect(run.state).toBe("canceled");
    expect(run.badges).toEqual(["interrupted"]);
    expect(run.error?.message).toContain("应用重启");
  });

  it("未知状态不再默认成 running", () => {
    const run = mapPipelineRun({ ...base, status: "weird" });
    expect(run.state).toBe("failed");
  });
});

describe("stockAnalysis.mapStockAnalysisRun", () => {
  const base: AnalysisSummary = {
    id: "a1",
    stockCode: "300308",
    stockName: "中际旭创",
    analysisDate: "2026-10-21",
    status: "running",
    decisionAction: null,
    decisionPositionPct: null,
    decisionPositionState: null,
    decisionJson: null,
    createdAt: 1_000,
    updatedAt: 1_000,
    completedAt: null,
    analysisKind: "live",
    asOfDate: null,
    parentAnalysisId: null,
    templateId: "stock-analysis",
  };

  it("running：有开始时间、无结束时刻", () => {
    const run = mapStockAnalysisRun(base);
    expect(run.state).toBe("running");
    expect(run.source).toBe("stock-analysis");
    expect(run.startedAt).toBe(1_000);
    expect(run.finishedAt).toBeUndefined();
    expect(run.durationMs).toBeUndefined();
    // 旧 `/stock-analysis/:id` 已于 2026-10-28 从路由表删除 ⇒ 深链必须指向投资工作区，
    // 否则点「打开」会命中 `path="*"` 的 404 页（用户报的正是这一条）。
    // 断言按**参数值**取，不钉查询串的拼接顺序（顺序不是契约，参数集合才是）。
    expect(run.deepLink).not.toContain("/stock-analysis");
    const link = new URL(run.deepLink!, "http://x");
    expect(link.pathname).toBe("/invest");
    expect(link.searchParams.get("tab")).toBe("workspace");
    expect(link.searchParams.get("view")).toBe("analysis");
    expect(link.searchParams.get("stockCode")).toBe("300308");
    expect(link.searchParams.get("stockName")).toBe("中际旭创");
    expect(link.searchParams.get("analysisId")).toBe("a1");
  });

  it("completed：用 completedAt 作为结束时刻", () => {
    const run = mapStockAnalysisRun({
      ...base,
      status: "completed",
      updatedAt: 4_000,
      completedAt: 4_000,
    });
    expect(run.state).toBe("succeeded");
    expect(run.durationMs).toBe(3_000);
  });

  it("completed 但 completedAt 为空：不拿 updatedAt 冒充完成时刻", () => {
    // 现网 65% 的 completed 行 updated_at 被后续写库顶掉（可达数天）；
    // 没有 completedAt 时必须显示「-」，不能把编辑时刻算成耗时。
    const run = mapStockAnalysisRun({
      ...base,
      status: "completed",
      updatedAt: 9_999_999,
      completedAt: null,
    });
    expect(run.finishedAt).toBeUndefined();
    expect(run.durationMs).toBeUndefined();
  });

  it("interrupted：已取消 + 徽章，但不拿回收时刻伪造耗时", () => {
    const run = mapStockAnalysisRun({ ...base, status: "interrupted", updatedAt: 9_999_999 });
    expect(run.state).toBe("canceled");
    expect(run.badges).toEqual(["interrupted"]);
    expect(run.durationMs).toBeUndefined();
  });

  it("未知状态不默认成 running", () => {
    expect(mapStockAnalysisRun({ ...base, status: "weird" }).state).toBe("failed");
  });

  it("list 只取根记录（replay 子行由 time-travel-batch 代表）", async () => {
    invokeMock.mockResolvedValue([
      { ...base, id: "root" },
      { ...base, id: "child", parentAnalysisId: "root", analysisKind: "replay" },
    ]);
    const rows = await stockAnalysisAdapter.list();
    expect(rows.map((r) => r.nativeId)).toEqual(["root"]);
  });
});

describe("timeTravel.mapBatchSummary", () => {
  function batch(over: Partial<TimeTravelBatchSummary>): TimeTravelBatchSummary {
    return {
      batchId: "b1",
      asOf: "2026-01-02",
      tradingDaysAgo: 1,
      status: "running",
      phase: "analysis",
      message: null,
      total: 10,
      ok: 4,
      fail: 0,
      recoPicks: 0,
      serenityPicks: 0,
      serenityError: null,
      errorCode: null,
      errorDetail: null,
      createdAtMs: 1000,
      updatedAtMs: 1000,
      finishedAtMs: null,
      ...over,
    };
  }

  it("running：进度由 ok/total 计算，阶段本地化", () => {
    const run = mapBatchSummary(batch({}));
    expect(run.state).toBe("running");
    expect(run.progress).toBe(40);
    expect(run.stage).toBe("replayWorkbench.batch.phaseAnalysis");
  });

  it("partial → succeeded + partial badge", () => {
    const run = mapBatchSummary(
      batch({ status: "partial", finishedAtMs: 2000, phase: "reflection" }),
    );
    expect(run.state).toBe("succeeded");
    expect(run.badges).toEqual(["partial"]);
    expect(run.stage).toBe("replayWorkbench.batch.phaseReflection");
  });

  it("interrupted → canceled + interrupted badge", () => {
    const run = mapBatchSummary(batch({ status: "interrupted" }));
    expect(run.state).toBe("canceled");
    expect(run.badges).toEqual(["interrupted"]);
  });

  it("failed 携带 errorDetail", () => {
    const run = mapBatchSummary(
      batch({
        status: "failed",
        errorCode: "E1",
        errorDetail: "批次失败",
        finishedAtMs: 3000,
      }),
    );
    expect(run.state).toBe("failed");
    expect(run.error).toMatchObject({ code: "E1", message: "批次失败" });
    expect(run.durationMs).toBe(2000);
  });
});

describe("moverRecall.mapBatchSummary", () => {
  function batch(over: Partial<MoverRecallBatchState>): MoverRecallBatchState {
    return {
      batchId: "b1",
      status: "running",
      message: null,
      total: 10,
      ok: 4,
      fail: 0,
      items: [],
      errorDetail: null,
      createdAtMs: 1_000,
      updatedAtMs: 1_000,
      finishedAtMs: null,
      trigger: null,
      ...over,
    };
  }

  it("trigger 如实映射；旧批次缺失 → unknown", () => {
    expect(mapMoverBatchSummary(batch({ trigger: "schedule" })).trigger).toBe("schedule");
    expect(mapMoverBatchSummary(batch({ trigger: "manual" })).trigger).toBe("manual");
    expect(mapMoverBatchSummary(batch({})).trigger).toBe("unknown");
  });
});

describe("aggregateRuns 源失败隔离", () => {
  it("单源 reject 只产生 SourceFetchError，其余源正常", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_pipeline_history") {
        return Promise.reject(new Error("backend down"));
      }
      return Promise.resolve([]);
    });
    const agg = await aggregateRuns();
    expect(agg.runs).toEqual([]);
    expect(agg.errors).toEqual([
      { source: "stock-pipeline", message: "backend down" },
    ]);
  });
});

describe("backgroundTasks.mapBackgroundTask", () => {
  function task(over: Partial<BackgroundTaskInfo>): BackgroundTaskInfo {
    return {
      id: "t-1",
      title: "采集收盘快照 2026-10-23",
      description: "",
      taskType: "market-close-sweep",
      status: "running",
      output: "",
      attempt: 0,
      createdAt: 1_000,
      updatedAt: 1_000,
      ...over,
    };
  }

  it("running：开始=创建时刻，无结束、无耗时", () => {
    const run = mapBackgroundTask(task({}));
    expect(run.state).toBe("running");
    expect(run.source).toBe("background-tasks");
    expect(run.startedAt).toBe(1_000);
    expect(run.finishedAt).toBeUndefined();
    expect(run.durationMs).toBeUndefined();
  });

  it("completed：终态带结束时刻与耗时", () => {
    const run = mapBackgroundTask(
      task({ status: "completed", exitCode: 0, output: "落库 5,215 行", finishedAt: 4_000 }),
    );
    expect(run.state).toBe("succeeded");
    expect(run.durationMs).toBe(3_000);
    expect(run.error).toBeUndefined();
  });

  it("failed：output 落 error.message（失败原因要能看见）", () => {
    const run = mapBackgroundTask(
      task({ status: "failed", exitCode: 1, output: "引擎报错了", finishedAt: 2_000 }),
    );
    expect(run.state).toBe("failed");
    expect(run.error?.message).toBe("引擎报错了");
  });

  it("pending → queued；stopped → canceled", () => {
    expect(mapBackgroundTask(task({ status: "pending" })).state).toBe("queued");
    expect(mapBackgroundTask(task({ status: "stopped" })).state).toBe("canceled");
  });

  it("未知状态不默认成 running", () => {
    expect(mapBackgroundTask(task({ status: "weird" })).state).toBe("failed");
  });

  it("行上无触发来源 ⇒ trigger=unknown（不猜成 manual）", () => {
    expect(mapBackgroundTask(task({})).trigger).toBe("unknown");
  });

  it("list 非数组（后端异常形态）⇒ 空列表而非炸页", async () => {
    invokeMock.mockResolvedValue(undefined);
    expect(await backgroundTasksAdapter.list()).toEqual([]);
  });

  it("list 正常透传映射", async () => {
    invokeMock.mockResolvedValue([task({ id: "t-9" })]);
    const runs = await backgroundTasksAdapter.list();
    expect(runs.map((r) => r.nativeId)).toEqual(["t-9"]);
  });
});
