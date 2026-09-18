// SPDX-License-Identifier: AGPL-3.0-only

import { act, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, useNavigate } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

// ── 页面组件 mock：仅验证 WorkspaceHub 的 Tab 渲染分发 ──
vi.mock("@/pages/ChatPage", () => ({ ChatPage: () => <div data-testid="page-chat" /> }));
vi.mock("@/pages/DashboardPage", () => ({ DashboardPage: () => <div data-testid="page-dashboard" /> }));
vi.mock("@/pages/DevTools/DevToolsPage", () => ({ DevToolsPage: () => <div data-testid="page-devtools" /> }));
vi.mock("@/pages/FilesPage", () => ({ FilesPage: () => <div data-testid="page-files" /> }));
vi.mock("@/pages/KnowledgeHubPage", () => ({ KnowledgeHubPage: () => <div data-testid="page-knowledge" /> }));
vi.mock("@/pages/MultiAgentPage", () => ({ MultiAgentPage: () => <div data-testid="page-multiagent" /> }));
vi.mock("@/pages/TerminalPage", () => ({ TerminalPage: () => <div data-testid="page-terminal" /> }));
vi.mock("@/pages/WorkflowPage", () => ({ WorkflowPage: () => <div data-testid="page-workflow" /> }));

import { WorkspaceHub } from "@/components/layout/WorkspaceHub";
import { WORKSPACE_TAB_PARAM, WORKSPACE_TABS } from "@/lib/workspaceTabs";
import { useWorkspaceTabStore } from "@/stores";

/**
 * 当前处于活跃（可见）状态的 Tab key。
 *
 * ⚠ 保活改造后，「切到哪个 Tab 了」**不能再**用 `queryByTestId(...)` 是否存在于文档来判断
 * —— 切走的 Tab 仍然留在 DOM 里（`display: none`）。唯一有区分力的判据是哪个 pane 是 active。
 */
function activePane(): string | undefined {
  const pane = document.querySelector<HTMLElement>('[data-workspace-pane][data-active="true"]');
  return pane?.dataset.workspacePane;
}

/** 某 Tab 的 pane 是否在 DOM 中 */
function paneExists(tab: string): boolean {
  return document.querySelector(`[data-workspace-pane="${tab}"]`) !== null;
}

/** 某 Tab 的 pane 的 display 值（保活页切走后应为 "none"，活跃时为 "contents"） */
function paneDisplay(tab: string): string | undefined {
  const pane = document.querySelector<HTMLElement>(`[data-workspace-pane="${tab}"]`);
  return pane?.style.display;
}

/** 模拟旧路由重定向：navigate("/chat", { state: { tab } }) 产生新的 state 对象引用 */
function RedirectProbe({ tab }: { tab: string }) {
  const navigate = useNavigate();
  return (
    <button onClick={() => navigate("/chat", { state: { tab } })}>
      redirect-{tab}
    </button>
  );
}

function renderWithState(state?: { tab: string }) {
  return render(
    <MemoryRouter initialEntries={[{ pathname: "/chat", state }]}>
      <WorkspaceHub />
    </MemoryRouter>,
  );
}

describe("WorkspaceHub 重定向 Tab 消费", () => {
  beforeEach(() => {
    // 重置全局 store，避免用例间泄漏
    useWorkspaceTabStore.setState({ activeTab: "chat" });
  });

  it("从旧路由重定向进入时消费 state.tab 并切到对应 Tab", async () => {
    renderWithState({ tab: "workflow" });
    expect(await screen.findByTestId("page-workflow")).toBeInTheDocument();
    expect(activePane()).toBe("workflow");
  });

  it("消费重定向后手动切换 Tab 不再回弹", async () => {
    renderWithState({ tab: "workflow" });
    expect(await screen.findByTestId("page-workflow")).toBeInTheDocument();

    // 手动切换 Tab（顶部 WorkspaceSwitcher 走 setActiveTab，不触发路由变化）
    act(() => {
      useWorkspaceTabStore.getState().setActiveTab("chat");
    });

    // 修复前：effect 依赖 activeTab 重跑，读到残留 state.tab="workflow" 回弹 → 仍显示 workflow
    // 修复后：保持手动切换的结果（workflow 因保活仍在 DOM，但已不是活跃 pane）
    expect(activePane()).toBe("chat");
    expect(paneDisplay("workflow")).toBe("none");
  });

  it("同值二次导航（state 为新对象）仍会消费，不会因 ref 值相同漏消费", async () => {
    render(
      <MemoryRouter initialEntries={[{ pathname: "/chat", state: { tab: "workflow" } }]}>
        <RedirectProbe tab="workflow" />
        <WorkspaceHub />
      </MemoryRouter>,
    );
    expect(await screen.findByTestId("page-workflow")).toBeInTheDocument();

    // 手动切走
    act(() => {
      useWorkspaceTabStore.getState().setActiveTab("chat");
    });
    expect(activePane()).toBe("chat");

    // 再次通过旧路由重定向进入（tab 值相同但 state 对象引用全新）
    fireEvent.click(screen.getByRole("button"));
    expect(await screen.findByTestId("page-workflow")).toBeInTheDocument();
    expect(activePane()).toBe("workflow");
  });

  it("直接访问 /chat（无 state）保持默认 Tab", async () => {
    renderWithState();
    expect(await screen.findByTestId("page-chat")).toBeInTheDocument();
    expect(activePane()).toBe("chat");
  });
});

/** 渲染指定 URL 的 Hub（不带重定向 state） */
function renderAtUrl(search: string) {
  return render(
    <MemoryRouter initialEntries={[{ pathname: "/chat", search }]}>
      <WorkspaceHub />
    </MemoryRouter>,
  );
}

describe("WorkspaceHub — ?ws= URL 通路（Tab 可深链 / 可刷新恢复）", () => {
  beforeEach(() => {
    useWorkspaceTabStore.setState({ activeTab: "chat" });
  });

  it("深链 ?ws=terminal 直接落到终端 Tab（无需经过点击）", async () => {
    renderAtUrl(`?${WORKSPACE_TAB_PARAM}=terminal`);
    expect(await screen.findByTestId("page-terminal")).toBeInTheDocument();
    expect(activePane()).toBe("terminal");
  });

  it("无 ?ws= 时保留 store 现值 —— 「回工作台恢复上次位置」的判据（不得重置为对话）", async () => {
    useWorkspaceTabStore.setState({ activeTab: "files" });
    renderWithState();
    expect(await screen.findByTestId("page-files")).toBeInTheDocument();
    expect(activePane()).toBe("files");
  });

  it("state.tab 优先于 ?ws=（旧路由重定向语义不被 URL 覆盖）", async () => {
    render(
      <MemoryRouter
        initialEntries={[{
          pathname: "/chat",
          search: `?${WORKSPACE_TAB_PARAM}=terminal`,
          state: { tab: "workflow" },
        }]}
      >
        <WorkspaceHub />
      </MemoryRouter>,
    );
    expect(await screen.findByTestId("page-workflow")).toBeInTheDocument();
    expect(activePane()).toBe("workflow");
  });

  it("非法 ?ws= 不改变 Tab（URL 手改 / 旧链接脏值防线）", async () => {
    useWorkspaceTabStore.setState({ activeTab: "knowledge" });
    renderAtUrl(`?${WORKSPACE_TAB_PARAM}=__bogus__`);
    expect(await screen.findByTestId("page-knowledge")).toBeInTheDocument();
    expect(activePane()).toBe("knowledge");
  });

  it("chat 专属参数仍强制对话 Tab（既有语义无回归）", async () => {
    useWorkspaceTabStore.setState({ activeTab: "terminal" });
    renderAtUrl("?conversationId=abc");
    expect(await screen.findByTestId("page-chat")).toBeInTheDocument();
    expect(activePane()).toBe("chat");
  });

  it("WORKSPACE_TABS 每个 Tab 都能渲染出页面（防「新增 Tab 忘加 switch 分支」）", async () => {
    const expectedTestId: Record<string, string> = {
      chat: "page-chat",
      dashboard: "page-dashboard",
      workflow: "page-workflow",
      terminal: "page-terminal",
      files: "page-files",
      knowledge: "page-knowledge",
      multiAgent: "page-multiagent",
      devtools: "page-devtools",
    };
    for (const tab of WORKSPACE_TABS) {
      const testId = expectedTestId[tab.key];
      // Tab 表新增 key 但此处未登记 ⇒ 直接失败，而不是静默跳过
      expect(testId, `Tab "${tab.key}" 未在本用例登记期望页面`).toBeDefined();
      const { unmount } = renderAtUrl(`?${WORKSPACE_TAB_PARAM}=${tab.key}`);
      expect(await screen.findByTestId(testId)).toBeInTheDocument();
      unmount();
      useWorkspaceTabStore.setState({ activeTab: "chat" });
    }
  });
});

describe("WorkspaceHub — Tab 保活（切走不卸载）", () => {
  beforeEach(() => {
    useWorkspaceTabStore.setState({ activeTab: "chat" });
  });

  it("切走的 Tab 保留在 DOM 中并被隐藏，而非卸载", async () => {
    renderAtUrl(`?${WORKSPACE_TAB_PARAM}=files`);
    expect(await screen.findByTestId("page-files")).toBeInTheDocument();
    expect(paneDisplay("files")).toBe("contents");

    act(() => {
      useWorkspaceTabStore.getState().setActiveTab("chat");
    });

    // 保活：仍在文档里，只是隐藏（对照：保活改造前这里会被卸载）
    expect(paneExists("files")).toBe(true);
    expect(paneDisplay("files")).toBe("none");
    expect(activePane()).toBe("chat");
  });

  it("切走再切回拿到的是同一个 DOM 节点 —— 证明未卸载重建（保活的核心价值）", async () => {
    renderAtUrl(`?${WORKSPACE_TAB_PARAM}=files`);
    const before = await screen.findByTestId("page-files");

    act(() => {
      useWorkspaceTabStore.getState().setActiveTab("chat");
    });
    act(() => {
      useWorkspaceTabStore.getState().setActiveTab("files");
    });

    // 若中间被卸载过，React 会创建全新节点 ⇒ 引用不相等
    expect(screen.getByTestId("page-files")).toBe(before);
  });

  it("从未访问过的 Tab 不进 DOM（保活 = 首次访问后才挂载，不是首屏全挂）", async () => {
    renderAtUrl(`?${WORKSPACE_TAB_PARAM}=chat`);
    await screen.findByTestId("page-chat");

    expect(paneExists("terminal")).toBe(false);
    expect(paneExists("workflow")).toBe(false);
    expect(paneExists("devtools")).toBe(false);
  });

  it("保活不破坏互斥可见性：任意时刻有且只有一个 active pane", async () => {
    renderAtUrl(`?${WORKSPACE_TAB_PARAM}=chat`);
    await screen.findByTestId("page-chat");

    act(() => {
      useWorkspaceTabStore.getState().setActiveTab("workflow");
    });
    act(() => {
      useWorkspaceTabStore.getState().setActiveTab("terminal");
    });

    const actives = document.querySelectorAll('[data-workspace-pane][data-active="true"]');
    expect(actives).toHaveLength(1);
    expect(activePane()).toBe("terminal");
  });
});
