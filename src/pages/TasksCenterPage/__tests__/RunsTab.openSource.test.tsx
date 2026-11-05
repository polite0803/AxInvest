// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { useTaskCenterStore } from "@/stores";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter, useLocation } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { UnifiedRun } from "../model";
import { RunsTab } from "../RunsTab";

/**
 * 「打开」按钮 = `deepLink` 的宿主侧消费点（任务 #152）。
 *
 * 缺陷形态：`cronAdapter` 每一行都写了 `deepLink: "/tasks?tab=schedules"` 并置
 * `capabilities.openSource: true`，而改造前**没有任何组件读这两个字段** ⇒ 能力声明了却点不到。
 * 修法是在 Runs 表操作列加一个按钮，因此这里必须钉住两条：
 * 1. 有 `deepLink` 才有按钮，跳的地址就是 run 自带的（中心不重拼路由）；
 * 2. 点击**不能**顺带打开详情抽屉 —— 行上的 onClick 会冒泡触发，两个动作同时发生就是回归。
 * 插件贡献的任务源没有详情抽屉（§93 禁第三方 React 进主 origin）⇒ 这个按钮是它们
 * 回到原页面的唯一路径，所以第 1 条对插件源是功能而不是装饰。
 */

const invokeMock = vi.fn();

// 适配器/注册表会经 `@/stores` barrel 拉到 cacheStore，它会调用 `isTauri()` 等导出
// ⇒ 整模块替身不够用（同 PLAN §100.3 的模块环结论），保留原模块只换 invoke。
vi.mock("@/lib/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: (...args: unknown[]) => invokeMock(...args) };
});

function makeRun(over: Partial<UnifiedRun> = {}): UnifiedRun {
  return {
    id: "unit-open:r-1",
    source: "unit-open",
    nativeId: "r-1",
    name: "单元用 run",
    trigger: "manual",
    state: "succeeded",
    attempts: 0,
    capabilities: {},
    ...over,
  };
}

/** 记录当前路由，用来断言 `navigate(run.deepLink)` 真的发生了。 */
function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="loc">{loc.pathname + loc.search}</div>;
}

function renderTab(runs: UnifiedRun[]) {
  act(() => {
    useTaskCenterStore.setState({
      runs,
      selectedRun: null,
      loading: false,
      refreshing: false,
      sourceErrors: [],
      // 挂载时的聚合取数在这里没有意义：本用例只验操作列，直接替换掉避免真打 IPC。
      loadRuns: async () => {},
    });
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <App>
        <MemoryRouter initialEntries={["/tasks"]}>
          <RunsTab />
          <LocationProbe />
        </MemoryRouter>
      </App>
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  act(() => {
    useTaskCenterStore.setState({ runs: [], selectedRun: null });
  });
});

describe("Runs 表的「打开」动作", () => {
  it("run 带 deepLink ⇒ 出「打开」，点击跳到该地址且不打开详情抽屉", () => {
    renderTab([makeRun({ deepLink: "/invest?tab=workspace&view=analysis&analysisId=a-1" })]);

    const open = screen.getByRole("button", { name: i18n.t("tasksCenter.action.open") });
    fireEvent.click(open);

    expect(screen.getByTestId("loc").textContent).toBe(
      "/invest?tab=workspace&view=analysis&analysisId=a-1",
    );
    expect(useTaskCenterStore.getState().selectedRun).toBeNull();
  });

  it("run 无 deepLink ⇒ 只有「详情」，不出「打开」", () => {
    renderTab([makeRun()]);

    expect(screen.getByRole("button", { name: i18n.t("tasksCenter.action.detail") }))
      .toBeInTheDocument();
    expect(screen.queryByRole("button", { name: i18n.t("tasksCenter.action.open") })).toBeNull();
  });

  it("多行时只有带 deepLink 的那行出「打开」", () => {
    renderTab([
      makeRun({ id: "unit-open:with", nativeId: "with", deepLink: "/tasks?tab=schedules" }),
      makeRun({ id: "unit-open:without", nativeId: "without", name: "无链接 run" }),
    ]);

    expect(screen.getAllByRole("button", { name: i18n.t("tasksCenter.action.open") })).toHaveLength(1);
  });
});
