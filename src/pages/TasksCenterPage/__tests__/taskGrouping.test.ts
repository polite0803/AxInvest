// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { canExecuteNow, classifyScheduledTask, isBuiltinTask } from "../taskGrouping";

/** 分组矩阵（spec TR-6.1，≥8 组合） */
const CLASSIFY_CASES: ReadonlyArray<{
  name: string;
  taskType: string | null;
  workflowId: string | null;
  expected: ReturnType<typeof classifyScheduledTask>;
}> = [
  // ── 投资自动化：内置 handler 白名单 ──
  { name: "智能荐股", taskType: "stock-recommendation", workflowId: null, expected: "invest" },
  { name: "趋势智选(按 taskType)", taskType: "trend-screening", workflowId: null, expected: "invest" },
  { name: "股票池扫描", taskType: "pool-scan", workflowId: null, expected: "invest" },
  { name: "自选股扫描", taskType: "watchlist-scan", workflowId: null, expected: "invest" },
  { name: "个股分析", taskType: "stock-analysis", workflowId: null, expected: "invest" },
  { name: "批次反思", taskType: "batch-reflection", workflowId: null, expected: "invest" },
  { name: "决策校验", taskType: "validate-decisions", workflowId: null, expected: "invest" },
  { name: "决策回测", taskType: "decision-backtest", workflowId: null, expected: "invest" },
  // ── 投资自动化：serenity / trend-screening 工作流（taskType 为空或 workflow 兜底） ──
  { name: "serenity-fast 工作流", taskType: null, workflowId: "serenity-fast", expected: "invest" },
  {
    name: "custom 壳 + serenity 工作流仍归投资",
    taskType: "custom",
    workflowId: "serenity-deep-daily",
    expected: "invest",
  },
  {
    name: "trend-screening 工作流(按 workflowId)",
    taskType: "workflow",
    workflowId: "trend-screening",
    expected: "invest",
  },
  // ── OPC：taskType 前缀 opc（连字符 / 下划线两种线序） ──
  { name: "OPC 需求发现(连字符)", taskType: "opc-demand-discovery", workflowId: null, expected: "opc" },
  { name: "OPC 手动扫描(下划线)", taskType: "opc_demand_scan", workflowId: null, expected: "opc" },
  // ── OPC：workflowId 前缀 ──
  {
    name: "空 taskType + opc- 工作流",
    taskType: null,
    workflowId: "opc-demand-discovery",
    expected: "opc",
  },
  {
    name: "custom 壳 + opc- 工作流归 OPC 优先",
    taskType: "custom",
    workflowId: "opc-demand-discovery",
    expected: "opc",
  },
  // ── 系统维护 ──
  { name: "备份", taskType: "backup", workflowId: null, expected: "system" },
  { name: "清理", taskType: "cleanup", workflowId: null, expected: "system" },
  { name: "每日摘要", taskType: "daily_summary", workflowId: null, expected: "system" },
  { name: "知识库全量抓取", taskType: "knowledge_source_fetch_all", workflowId: null, expected: "system" },
  // ── 自定义 / 兜底 ──
  { name: "custom 空壳", taskType: "custom", workflowId: null, expected: "custom" },
  { name: "general 旧版任务", taskType: "general", workflowId: null, expected: "custom" },
  {
    name: "空 taskType 普通工作流归自定义",
    taskType: null,
    workflowId: "my-weekly-report",
    expected: "custom",
  },
  { name: "自由 platform 值兜底", taskType: "my-platform", workflowId: null, expected: "custom" },
];

describe("classifyScheduledTask", () => {
  for (const c of CLASSIFY_CASES) {
    it(`${c.expected} ← ${c.name}`, () => {
      expect(classifyScheduledTask({ taskType: c.taskType, workflowId: c.workflowId })).toBe(
        c.expected,
      );
    });
  }

  it("大小写不敏感（taskType / workflowId 大写混入）", () => {
    expect(
      classifyScheduledTask({ taskType: "OPC-DEMAND-DISCOVERY", workflowId: null }),
    ).toBe("opc");
    expect(
      classifyScheduledTask({ taskType: "Stock-Recommendation", workflowId: null }),
    ).toBe("invest");
  });
});

describe("canExecuteNow", () => {
  it("内置 handler 类型可执行", () => {
    expect(canExecuteNow({ taskType: "pool-scan", workflowId: null })).toBe(true);
    expect(canExecuteNow({ taskType: "opc_demand_scan", workflowId: null })).toBe(true);
    expect(canExecuteNow({ taskType: "knowledge_source_fetch_all", workflowId: null })).toBe(true);
  });

  it("绑定工作流即可执行（含自定义壳）", () => {
    expect(canExecuteNow({ taskType: "custom", workflowId: "my-wf" })).toBe(true);
    expect(canExecuteNow({ taskType: null, workflowId: "trend-screening" })).toBe(true);
  });

  it("裸 custom / general / 无 workflow 的系统空壳不可执行", () => {
    expect(canExecuteNow({ taskType: "custom", workflowId: null })).toBe(false);
    expect(canExecuteNow({ taskType: "general", workflowId: null })).toBe(false);
    // backup/cleanup/daily_summary 既无 workflow 又不在 BUILTIN_TASK_TYPES
    expect(canExecuteNow({ taskType: "backup", workflowId: null })).toBe(false);
    expect(canExecuteNow({ taskType: "daily_summary", workflowId: null })).toBe(false);
  });
});

describe("isBuiltinTask", () => {
  it("投资 / OPC / 系统三组均为内置种子任务", () => {
    expect(isBuiltinTask({ taskType: "stock-recommendation", workflowId: null })).toBe(true);
    expect(isBuiltinTask({ taskType: "opc-demand-discovery", workflowId: "opc-demand-discovery" })).toBe(true);
    expect(isBuiltinTask({ taskType: "backup", workflowId: null })).toBe(true);
  });

  it("自定义组不是内置任务", () => {
    expect(isBuiltinTask({ taskType: "custom", workflowId: null })).toBe(false);
    expect(isBuiltinTask({ taskType: "custom", workflowId: "my-wf" })).toBe(false);
  });
});
