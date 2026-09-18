// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

// 活动信号 mock 成受控对象：本文件只验证「切换栏怎么渲染活动点」，
// 「信号是否真的接到 store」由 useWorkspaceTabActivity.test.ts 单独覆盖。
const hoisted = vi.hoisted(() => ({ activity: {} as Record<string, boolean> }));
vi.mock("@/hooks/useWorkspaceTabActivity", () => ({
  useWorkspaceTabActivity: () => hoisted.activity,
}));

import { WorkspaceSwitcher } from "@/components/layout/WorkspaceSwitcher";
import type { WorkspaceTab } from "@/lib/workspaceTabs";
import { WORKSPACE_TABS } from "@/lib/workspaceTabs";
import { useWorkspaceTabStore } from "@/stores";

/** 切换栏里按 WORKSPACE_TABS 顺序取第 N 个按钮 */
function tabButton(index: number): HTMLButtonElement {
  const buttons = document.querySelectorAll<HTMLButtonElement>(".ax-workspace-switcher button");
  const el = buttons[index];
  if (!el) {
    throw new Error(`切换栏第 ${index} 个按钮不存在（实际 ${buttons.length} 个）`);
  }
  return el;
}

function renderSwitcher(activeTab: WorkspaceTab) {
  act(() => {
    useWorkspaceTabStore.setState({ activeTab });
  });
  return render(
    <MemoryRouter initialEntries={["/chat"]}>
      <WorkspaceSwitcher />
    </MemoryRouter>,
  );
}

describe("WorkspaceSwitcher — 活动指示点", () => {
  beforeEach(() => {
    hoisted.activity = {};
  });

  it("非活跃 Tab 有工作进行中时显示活动点", () => {
    hoisted.activity = { chat: true };
    renderSwitcher("terminal");
    expect(screen.getByTestId("ws-activity-chat")).toBeInTheDocument();
  });

  it("活跃 Tab 即使有信号也不显示活动点（用户正看着，提示是噪声）", () => {
    hoisted.activity = { chat: true };
    renderSwitcher("chat");
    expect(screen.queryByTestId("ws-activity-chat")).not.toBeInTheDocument();
  });

  it("信号为 false 的 Tab 不显示活动点", () => {
    hoisted.activity = { chat: false, workflow: true };
    renderSwitcher("terminal");
    expect(screen.queryByTestId("ws-activity-chat")).not.toBeInTheDocument();
    expect(screen.getByTestId("ws-activity-workflow")).toBeInTheDocument();
  });

  it("4 个有信号的 Tab 都能各自点亮（chat / workflow / multiAgent / devtools）", () => {
    hoisted.activity = { chat: true, workflow: true, multiAgent: true, devtools: true };
    renderSwitcher("files");
    for (const key of ["chat", "workflow", "multiAgent", "devtools"]) {
      expect(screen.getByTestId(`ws-activity-${key}`), `${key} 活动点未渲染`).toBeInTheDocument();
    }
  });

  it("活动点只在有信号的 Tab 上出现 —— 无信号 Tab 一个点都不该有", () => {
    hoisted.activity = { chat: true };
    renderSwitcher("terminal");
    // terminal / files / knowledge / dashboard 无信号源（见 useWorkspaceTabActivity 文件头反例）
    for (const key of ["terminal", "files", "knowledge", "dashboard"]) {
      expect(screen.queryByTestId(`ws-activity-${key}`)).not.toBeInTheDocument();
    }
  });

  it("活动信息并入按钮 aria-label（不能只用一个色点传达状态）", () => {
    hoisted.activity = { chat: false };
    const quiet = renderSwitcher("terminal");
    const quietLabel = tabButton(0).getAttribute("aria-label");
    quiet.unmount();

    hoisted.activity = { chat: true };
    renderSwitcher("terminal");
    const busyLabel = tabButton(0).getAttribute("aria-label");

    // 不硬编码译文：只断言「忙碌态是无活动态的前缀 + 后缀」，与 i18n 语言无关
    expect(quietLabel).toBeTruthy();
    expect(busyLabel).not.toBe(quietLabel);
    expect(busyLabel?.startsWith(quietLabel!)).toBe(true);
    expect(busyLabel?.length ?? 0).toBeGreaterThan(quietLabel?.length ?? 0);
  });

  it("切换栏仍渲染全部 8 个 Tab（活动点改造不得影响 Tab 清单）", () => {
    renderSwitcher("chat");
    expect(document.querySelectorAll(".ax-workspace-switcher button")).toHaveLength(
      WORKSPACE_TABS.length,
    );
  });
});
