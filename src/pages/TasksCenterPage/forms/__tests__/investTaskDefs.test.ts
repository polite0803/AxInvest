// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";
import { getInvestKindDef, INVEST_KIND_DEFS } from "../investTaskDefs";

/**
 * TR-7.1 数据层：7 类投资任务的「表单值 → 专用命令 + camelCase 载荷」映射。
 * 载荷键必须与后端各 create_*_cron 命令参数的 camelCase 形态逐字一致
 * （Tauri v2 命令宏默认 camelCase 反序列化）。
 */
describe("investTaskDefs 载荷构造", () => {
  it("7 个种类 id / 命令唯一且不重复", () => {
    expect(INVEST_KIND_DEFS).toHaveLength(7);
    expect(new Set(INVEST_KIND_DEFS.map((d) => d.kind)).size).toBe(7);
    expect(new Set(INVEST_KIND_DEFS.map((d) => d.command)).size).toBe(7);
  });

  it("stock-analysis：个股代码/名称/cron 三参数", () => {
    const def = getInvestKindDef("stock-analysis");
    expect(def.command).toBe("create_stock_cron");
    expect(
      def.buildArgs({
        stockCode: "600519",
        stockName: "贵州茅台",
        cronExpression: "0 9 * * *",
      }),
    ).toEqual({
      stockCode: "600519",
      stockName: "贵州茅台",
      cronExpression: "0 9 * * *",
    });
  });

  it("watchlist-scan：仅 cron + enabled=true", () => {
    const def = getInvestKindDef("watchlist-scan");
    expect(def.buildArgs({ cronExpression: "0 21 * * 1-5" })).toEqual({
      cronExpression: "0 21 * * 1-5",
      enabled: true,
    });
  });

  it("pool-scan：三个可选项缺省时不出现在载荷中（走后端默认）", () => {
    const def = getInvestKindDef("pool-scan");
    expect(def.buildArgs({ cronExpression: "0 17 * * 1-5" })).toEqual({
      cronExpression: "0 17 * * 1-5",
      enabled: true,
    });
  });

  it("pool-scan：可选项有值时按 camelCase 透传", () => {
    const def = getInvestKindDef("pool-scan");
    expect(
      def.buildArgs({
        cronExpression: "0 17 * * 1-5",
        lookbackDays: 30,
        maxStocks: 100,
        minConfidence: 60,
      }),
    ).toEqual({
      cronExpression: "0 17 * * 1-5",
      enabled: true,
      lookbackDays: 30,
      maxStocks: 100,
      minConfidence: 60,
    });
  });

  it("trend-screening：workflowId 取所选链 id（两链白名单值原样透传）", () => {
    const def = getInvestKindDef("trend-screening");
    expect(def.buildArgs({ chain: "serenity-screening-fast", cronExpression: "0 16 * * 1-5" })).toEqual({
      cronExpression: "0 16 * * 1-5",
      enabled: true,
      workflowId: "serenity-screening-fast",
    });
  });

  it("batch-reflection：period 透传 + dueOnly 固定 false", () => {
    const def = getInvestKindDef("batch-reflection");
    expect(def.buildArgs({ period: "mid", cronExpression: "0 18 * * *" })).toEqual({
      cronExpression: "0 18 * * *",
      period: "mid",
      dueOnly: false,
      enabled: true,
    });
  });

  it("decision-backtest：period=all 必须映射为 null（空串会零产物报成功）", () => {
    const def = getInvestKindDef("decision-backtest");
    const args = def.buildArgs({
      windows: [5, 20, 60],
      period: "all",
      maxPicks: 200,
      cronExpression: "0 7 * * *",
    });
    expect(args).toEqual({
      cronExpression: "0 7 * * *",
      periodFilter: null,
      tPlusNList: [5, 20, 60],
      maxPicks: 200,
      enabled: true,
    });
  });

  it("decision-backtest：具体周期原样进 periodFilter", () => {
    const def = getInvestKindDef("decision-backtest");
    expect(
      def.buildArgs({ windows: [20], period: "long", maxPicks: 50, cronExpression: "0 7 * * *" }),
    ).toMatchObject({ periodFilter: "long", tPlusNList: [20], maxPicks: 50 });
  });

  it("stock-recommendation：name/cron/periods/minConfidence/topN 五参数", () => {
    const def = getInvestKindDef("stock-recommendation");
    expect(
      def.buildArgs({
        name: "每日荐股",
        cronExpression: "30 15 * * *",
        periods: ["short", "mid"],
        minConfidence: 50,
        topN: 5,
      }),
    ).toEqual({
      name: "每日荐股",
      cronExpression: "30 15 * * *",
      periods: ["short", "mid"],
      minConfidence: 50,
      topN: 5,
    });
  });

  it("getInvestKindDef 遇未知种类抛错（防拼写漂移静默走错命令）", () => {
    // 内部 invariant 文案为英文（开发者错误，不进 UI，无需 i18n）
    expect(() => getInvestKindDef("nope" as never)).toThrow(/Unknown investment task kind/);
  });
});
