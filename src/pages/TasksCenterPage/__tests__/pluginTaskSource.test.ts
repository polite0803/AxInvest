// SPDX-License-Identifier: AGPL-3.0-only

import type { PluginTaskRunRowDto, PluginTaskSourceDto, PluginTaskSourcesInfoDto } from "@/types";
import { cleanup, render, waitFor } from "@testing-library/react";
import { createElement, type ReactElement } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPluginTaskSourceAdapter } from "../adapters/pluginTaskSource.adapter";
import { getAdapter, getAllAdapters, registerPluginTaskSources } from "../adapters/registry";

const { invokeMock, schemaCardSpecs } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  /** 详情槽渲染时 `PluginSchemaCard` 实际收到的 spec（逐次追加）。 */
  schemaCardSpecs: [] as Array<{ schemaId: string; dataContext?: unknown; title?: string }>,
}));

vi.mock("@/lib/invoke", () => ({ invoke: invokeMock }));
vi.mock("@/i18n", () => ({ default: { t: (k: string) => k } }));

// 详情槽只验「规格递对了没有」：卡片本体（动态 UI 库 / 渲染链）有自己的测试，
// 这里桩掉它，既不用起 store，也不会因 schema 不存在而落 Alert 干扰断言。
vi.mock("@/components/chat/PluginSchemaCard", () => ({
  PluginSchemaCard: (props: { spec: { schemaId: string; dataContext?: unknown; title?: string } }) => {
    schemaCardSpecs.push(props.spec);
    return createElement("div", { "data-testid": "unit-plugin-schema-card" });
  },
}));

function makeSource(over: Partial<PluginTaskSourceDto> = {}): PluginTaskSourceDto {
  return {
    id: "unit-plugin-runs",
    title: "单元测试插件执行",
    command: "list_unit_runs",
    color: "gold",
    ...over,
  };
}

function makeRow(over: Partial<PluginTaskRunRowDto> = {}): PluginTaskRunRowDto {
  return {
    nativeId: "r-1",
    name: "示例 · r-1",
    state: "running",
    ...over,
  };
}

function makeInfo(pluginId: string, taskSources: PluginTaskSourceDto[], enabled = true): PluginTaskSourcesInfoDto {
  return {
    id: pluginId,
    name: pluginId,
    version: "1.0.0",
    description: "",
    author: null,
    enabled,
    taskSources,
  };
}

/** 让下一次 `plugin_execute_command` 回给定行数组。 */
function stubRows(rows: unknown) {
  invokeMock.mockResolvedValue(JSON.stringify(rows));
}

beforeEach(() => {
  invokeMock.mockReset();
  schemaCardSpecs.length = 0;
});

describe("插件任务源泛型适配器", () => {
  it("取数走 plugin_execute_command，并把行映射成 UnifiedRun", async () => {
    stubRows([makeRow({ startedAt: 1_000, finishedAt: 2_500 })]);
    const adapter = createPluginTaskSourceAdapter("unit-plugin", makeSource());

    const runs = await adapter.list();

    expect(invokeMock).toHaveBeenCalledWith("plugin_execute_command", {
      pluginId: "unit-plugin",
      name: "list_unit_runs",
      input: {},
    });
    expect(runs).toHaveLength(1);
    const run = runs[0];
    // run id 由宿主拼：插件只给源内原生 id（带前缀会破坏全局唯一）
    expect(run.id).toBe("unit-plugin-runs:r-1");
    expect(run.source).toBe("unit-plugin-runs");
    expect(run.nativeId).toBe("r-1");
    expect(run.state).toBe("running");
    expect(run.trigger).toBe("unknown");
    expect(run.durationMs).toBe(1500);
  });

  it("源名用声明的纯文本、nameKey 留空（插件文案不占宿主 i18n key）", () => {
    const adapter = createPluginTaskSourceAdapter("unit-plugin", makeSource({ color: undefined }));
    expect(adapter.name).toBe("单元测试插件执行");
    expect(adapter.nameKey).toBe("");
    // color 缺省兜底成 antd 的 default，而不是 undefined（Tag 会拿到 undefined 色）
    expect(adapter.color).toBe("default");
  });

  it("表外状态落 waiting，不猜成 running 或 failed", async () => {
    stubRows([makeRow({ nativeId: "x", state: "partially_completed" })]);
    const adapter = createPluginTaskSourceAdapter("unit-plugin", makeSource());
    const [run] = await adapter.list();
    expect(run.state).toBe("waiting");
  });

  it("durationMs 缺省时由两端点相减；端点缺失或倒挂则留空，不编一个数", async () => {
    stubRows([
      makeRow({ nativeId: "a", startedAt: 1_000, finishedAt: 3_000 }),
      makeRow({ nativeId: "b", startedAt: 5_000, finishedAt: 4_000 }),
      makeRow({ nativeId: "c", startedAt: 5_000 }),
      makeRow({ nativeId: "d", durationMs: 700, startedAt: 1, finishedAt: 9 }),
    ]);
    const adapter = createPluginTaskSourceAdapter("unit-plugin", makeSource());
    const runs = await adapter.list();
    expect(runs.find((r) => r.nativeId === "a")?.durationMs).toBe(2000);
    expect(runs.find((r) => r.nativeId === "b")?.durationMs).toBeUndefined();
    expect(runs.find((r) => r.nativeId === "c")?.durationMs).toBeUndefined();
    // 显式 durationMs 优先于端点相减
    expect(runs.find((r) => r.nativeId === "d")?.durationMs).toBe(700);
  });

  it("只有给了 deepLink 才算可打开；raw 缺省时回退整行", async () => {
    stubRows([
      makeRow({ nativeId: "no-link" }),
      makeRow({ nativeId: "with-link", deepLink: "/invest/600519", raw: { k: 1 } }),
    ]);
    const adapter = createPluginTaskSourceAdapter("unit-plugin", makeSource());
    const runs = await adapter.list();
    const [noLink, withLink] = runs;
    expect(noLink.deepLink).toBeUndefined();
    expect(withLink.raw).toEqual({ k: 1 });
  });

  it("命令返回非数组 ⇒ 抛错（由 aggregateRuns 收成单源错误），不静默空列表", async () => {
    stubRows({ executions: [] });
    const adapter = createPluginTaskSourceAdapter("unit-plugin", makeSource());
    await expect(adapter.list()).rejects.toThrow("应返回行数组");
  });

  it("命令失败 ⇒ 直接向上抛，不吞成空列表", async () => {
    // ⚠ `cmd === undefined` 不是业务分支：vitest 4 会在 mockImplementation() 当场拿空参试调一次实现
    //   （实证：把实现换成会 reject、而用例里根本不调 mock —— 该用例仍被判失败，报的就是实现里新建的
    //   rejection）。那一次试调没人 await ⇒ 记成用例级未处理异常，属假红。真实调用永远带命令名，
    //   所以按「有没有实参」把试调摘出去，rejection 只在真实调用时产生。
    invokeMock.mockImplementation((cmd?: unknown) =>
      cmd === undefined ? Promise.resolve("[]") : Promise.reject(new Error("worker 已停止"))
    );
    const adapter = createPluginTaskSourceAdapter("unit-plugin", makeSource());
    let caught: unknown;
    try {
      await adapter.list();
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(Error);
    expect((caught as Error).message).toBe("worker 已停止");
  });
});

describe("插件任务源详情槽（detailSchema，任务 #149 第 2 步）", () => {
  it("声明 detailSchema ⇒ 挂 renderDetail，渲染后把拼全的 schemaId / run.raw 原样递给 PluginSchemaCard", async () => {
    stubRows([makeRow({ nativeId: "r-9", name: "示例 · r-9", raw: { rows: [1, 2] } })]);
    const adapter = createPluginTaskSourceAdapter(
      "unit-plugin",
      makeSource({ detailSchema: "order-form" }),
    );
    const renderDetail = adapter.renderDetail;
    if (!renderDetail) {
      throw new Error("声明了 detailSchema 的源必须挂 renderDetail");
    }

    const [run] = await adapter.list();
    const close = vi.fn();
    render(renderDetail({ run, close }) as ReactElement);

    // Drawer 的子树经 portal 挂载，可能晚一帧进 DOM ⇒ 等卡片被渲染出来再断言
    await waitFor(() => expect(schemaCardSpecs).toHaveLength(1));
    // 后端只给 ui.json 的 key，行 id 由宿主拼（PLAN §112.4：拼全 id 的活儿归宿主）
    expect(schemaCardSpecs[0].schemaId).toBe("plugin.unit-plugin.order-form");
    // 同一份对象引用：dataContext 与 schema 节点 id 的对齐由插件负责，宿主不复制也不改键
    expect(schemaCardSpecs[0].dataContext).toBe(run.raw);
    expect(schemaCardSpecs[0].title).toBe(run.name);
    cleanup();
  });

  it("没有 detailSchema ⇒ 不挂 renderDetail（点开一行走中心空抽屉兜底）", () => {
    const adapter = createPluginTaskSourceAdapter("unit-plugin", makeSource());
    expect(adapter.renderDetail).toBeUndefined();
  });
});

describe("registerPluginTaskSources", () => {
  it("只挂 enabled 的插件源，挂上后 getAllAdapters / getAdapter 都能拿到", () => {
    const id = `unit-enabled-${Date.now()}`;
    const before = getAllAdapters().length;
    const added = registerPluginTaskSources([
      makeInfo("p-on", [makeSource({ id })]),
      makeInfo("p-off", [makeSource({ id: `${id}-off` })], false),
    ]);

    expect(added).toBe(1);
    expect(getAllAdapters().length).toBe(before + 1);
    expect(getAdapter(id)?.name).toBe("单元测试插件执行");
    expect(getAdapter(`${id}-off`)).toBeUndefined();
  });

  it("与宿主源重名 ⇒ 宿主优先，插件源被忽略并告警", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    try {
      const added = registerPluginTaskSources([makeInfo("p-clash", [makeSource({ id: "stock-analysis" })])]);
      expect(added).toBe(0);
      // 被忽略的必须是宿主那份适配器（nameKey 有值、没有纯文本 name）
      expect(getAdapter("stock-analysis")?.name).toBeUndefined();
      expect(warn).toHaveBeenCalled();
    } finally {
      warn.mockRestore();
    }
  });

  it("空清单 ⇒ 不追加、不 bump（注册表长度不变）", () => {
    const before = getAllAdapters().length;
    expect(registerPluginTaskSources([])).toBe(0);
    expect(registerPluginTaskSources([makeInfo("p-none", [])])).toBe(0);
    expect(getAllAdapters().length).toBe(before);
  });

  it("聚合把插件源失败隔离成一条 SourceFetchError，不影响宿主源", async () => {
    const id = `unit-fail-${Date.now()}`;
    registerPluginTaskSources([makeInfo("p-fail", [makeSource({ id })])]);
    // 其余源（含真实宿主源）全部成功，只有插件源抛错
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "plugin_execute_command") {
        return Promise.reject(new Error("插件没起来"));
      }
      return Promise.resolve("[]");
    });

    const { aggregateRuns } = await import("../adapters/registry");
    const { runs, errors } = await aggregateRuns({ sources: [id] });

    expect(runs).toEqual([]);
    expect(errors).toHaveLength(1);
    expect(errors[0].source).toBe(id);
    expect(errors[0].message).toContain("插件没起来");
  });
});
