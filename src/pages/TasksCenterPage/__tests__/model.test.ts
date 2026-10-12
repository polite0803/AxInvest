// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import {
  activeRuns,
  attentionRuns,
  computeHealth,
  filterRuns,
  runIdOf,
  sortRuns,
  summarizeWindow,
  type UnifiedRun,
} from "../model";

const NOW = 1_700_000_000_000;

function makeRun(over: Partial<UnifiedRun> & Pick<UnifiedRun, "id" | "state">): UnifiedRun {
  return {
    source: "scheduled-cron",
    nativeId: over.id,
    name: `run-${over.id}`,
    trigger: "manual",
    attempts: 0,
    capabilities: {},
    ...over,
  };
}

const ALL: UnifiedRun[] = [
  makeRun({ id: "a", state: "running", startedAt: NOW - 5_000 }),
  makeRun({ id: "b", state: "queued", queuedAt: NOW - 1_000 }),
  makeRun({ id: "c", state: "waiting" }),
  makeRun({
    id: "d",
    state: "failed",
    finishedAt: NOW - 10_000,
    error: { message: "boom" },
  }),
  makeRun({
    id: "e",
    state: "succeeded",
    source: "stock-pipeline",
    name: "管道 Alpha",
    finishedAt: NOW - 60_000,
  }),
  makeRun({ id: "f", state: "canceled", finishedAt: NOW - 30_000 }),
];

describe("computeHealth", () => {
  it("四计数正确", () => {
    expect(computeHealth(ALL)).toEqual({
      running: 1,
      queued: 1,
      failed: 1,
      waiting: 1,
    });
  });
});

describe("attentionRuns", () => {
  it("失败 + 等待 + 僵死 badge", () => {
    const ids = attentionRuns(ALL).map((r) => r.id);
    expect(ids).toContain("c");
    expect(ids).toContain("d");
    const stale = makeRun({
      id: "z",
      state: "succeeded",
      badges: ["stale"],
    });
    expect(attentionRuns([stale]).map((r) => r.id)).toEqual(["z"]);
  });
});

describe("activeRuns", () => {
  it("running + queued", () => {
    expect(activeRuns(ALL).map((r) => r.id).sort()).toEqual(["a", "b"]);
  });
});

describe("filterRuns", () => {
  it("按源过滤", () => {
    expect(
      filterRuns(ALL, { sources: ["stock-pipeline"] }).map((r) => r.id),
    ).toEqual(["e"]);
  });

  it("按状态过滤", () => {
    expect(
      filterRuns(ALL, { states: ["succeeded", "canceled"] }).map((r) => r.id),
    ).toEqual(["e", "f"]);
  });

  it("按触发方式过滤", () => {
    expect(
      filterRuns(ALL, { triggers: ["schedule"] }),
    ).toEqual([]);
  });

  it("时间窗（开始/结束/排队锚点）", () => {
    const since = NOW - 20_000;
    expect(filterRuns(ALL, { sinceMs: since }).map((r) => r.id).sort()).toEqual(
      ["a", "b", "d"],
    );
  });

  it("全文搜索名称与错误", () => {
    expect(filterRuns(ALL, { search: "zzz-no-such" })).toHaveLength(0);
    expect(filterRuns(ALL, { search: "管道" }).map((r) => r.id)).toEqual(["e"]);
    expect(filterRuns(ALL, { search: "boom" }).map((r) => r.id)).toEqual(["d"]);
  });
});

describe("sortRuns", () => {
  it("运行中优先，其后按活动时间倒序", () => {
    const sorted = sortRuns(ALL).map((r) => r.id);
    expect(sorted.slice(0, 3)).toEqual(["a", "b", "c"]);
    expect(sorted.slice(3)).toEqual(["d", "f", "e"]);
  });
});

describe("summarizeWindow", () => {
  it("只统计窗口内终态", () => {
    const summary = summarizeWindow(ALL, NOW - 40_000, NOW + 1_000);
    expect(summary).toEqual({ succeeded: 0, failed: 1, canceled: 1 });
  });
});

describe("runIdOf", () => {
  it("源与原生 id 组合", () => {
    expect(runIdOf("stock-pipeline", "x1")).toBe("stock-pipeline:x1");
  });
});
