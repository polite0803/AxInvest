// SPDX-License-Identifier: AGPL-3.0-only

import i18n from "@/i18n";
import { useTaskCenterStore } from "@/stores";
import { act, cleanup, render, screen } from "@testing-library/react";
import { App } from "antd";
import { createElement } from "react";
import { I18nextProvider } from "react-i18next";
import { afterEach, describe, expect, it } from "vitest";
import { getAdapter, registerAdapter } from "../adapters/registry";
import type { TaskSourceAdapter } from "../adapters/types";
import type { UnifiedRun } from "../model";
import { RunDetailDrawer } from "../RunDetailDrawer";

/**
 * 详情面分派的契约测试（任务 #151 / PLAN §100）。
 *
 * 为什么值得为「一个 if」写测试：改造前这里是五条 `selected.source === "…"` 字面量判断，
 * 中心必须知道每个源怎么渲染；改造后中心只认注册表。这两件事都只有渲染一遍才能证明：
 * 1. 源声明了 `renderDetail` ⇒ 用的就是它（不是兜底空抽屉）；
 * 2. 源**没**声明（插件贡献的源今天全部如此）或该源已不在注册表里（插件被禁用）⇒
 *    落空抽屉兜底，不崩、不白屏。
 * 第 2 条是 §98 那条「点开一条插件 run 没有抽屉」的现形路径，之前没有任何测试钉过。
 */

const MARKER = "unit-render-detail-marker";

function makeRun(source: string, nativeId = "n-1"): UnifiedRun {
  return {
    id: `${source}:${nativeId}`,
    source,
    nativeId,
    name: "单元用 run",
    trigger: "manual",
    state: "succeeded",
    attempts: 0,
    capabilities: {},
  };
}

const withDetail: TaskSourceAdapter = {
  id: "unit-with-detail",
  nameKey: "",
  color: "blue",
  async list() {
    return [];
  },
  renderDetail: ({ run }) => createElement("div", { "data-testid": MARKER }, run.nativeId),
};

const withoutDetail: TaskSourceAdapter = {
  id: "unit-without-detail",
  nameKey: "",
  color: "blue",
  async list() {
    return [];
  },
};

function selectAndRender(source: string, nativeId = "n-1") {
  act(() => {
    useTaskCenterStore.setState({ selectedRun: makeRun(source, nativeId) });
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <App>
        <RunDetailDrawer />
      </App>
    </I18nextProvider>,
  );
}

afterEach(() => {
  cleanup();
  act(() => {
    useTaskCenterStore.setState({ selectedRun: null });
  });
});

describe("RunDetailDrawer 按注册表分派详情面", () => {
  it("源声明 renderDetail ⇒ 渲染该源的详情形态，不走兜底", () => {
    registerAdapter(withDetail);
    expect(getAdapter("unit-with-detail")).toBe(withDetail);

    selectAndRender("unit-with-detail", "exec-42");

    expect(screen.getByTestId(MARKER).textContent).toBe("exec-42");
    expect(screen.queryByText(i18n.t("tasksCenter.runs.detailUnavailable"))).toBeNull();
  });

  it("源未声明 renderDetail ⇒ 落空抽屉兜底（与改造前一致）", () => {
    registerAdapter(withoutDetail);

    selectAndRender("unit-without-detail");

    expect(screen.queryByTestId(MARKER)).toBeNull();
    expect(screen.getByText(i18n.t("tasksCenter.runs.detailUnavailable"))).toBeInTheDocument();
  });

  it("run 的源不在注册表里（插件被禁用等）⇒ 仍出兜底抽屉，不抛错", () => {
    expect(getAdapter("unit-not-registered")).toBeUndefined();

    selectAndRender("unit-not-registered");

    expect(screen.getByText(i18n.t("tasksCenter.runs.detailUnavailable"))).toBeInTheDocument();
  });
});
