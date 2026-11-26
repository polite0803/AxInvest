// SPDX-License-Identifier: AGPL-3.0-only

/**
 * P3/T8 前端收口：入口可达 + 关键 create→run→steps→回执流程。
 *
 * - 入口：/bots/workflows 路由可直达页面；导航注册表暴露 Bot 编排入口（沿用 P1/P2 的 Bot 页模式）。
 * - 流程：编辑器创建编排 → 运行编排 → run 列表出现步骤进度 → outputs 回执可展开 → 委派链面板可见。
 *
 * store / invoke / router 全部替身化：本文件钉前端组合层接线，不跑 e2e、不碰后端。
 */

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { App } from "antd";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { NAV_GROUPS } from "@/lib/navGroups";
import { builtinNavItems, getNavItems } from "@/lib/navRegistry";
import { BUILTIN_PAGE_PATH } from "@/lib/pageRegistry";
import type { Bot, BotAssignment, BotDepartment, BotWorkflow, BotWorkflowRun, CreateBotWorkflowInput } from "@/types";

const h = vi.hoisted(() => {
  const fn = () => vi.fn();
  const state = {
    bots: [] as Bot[],
    loading: false,
    saving: false,
    deleting: false,
    error: null as string | null,
    activeBotId: null as string | null,
    workflows: [] as BotWorkflow[],
    workflowRuns: [] as BotWorkflowRun[],
    workflowLoading: false,
    workflowSaving: false,
    loadBots: fn(),
    loadWorkflows: fn(),
    createWorkflow: fn(),
    updateWorkflow: fn(),
    runWorkflow: fn(),
    loadWorkflowRuns: fn(),
    cancelWorkflowRun: fn(),
    createWorkflowSchedule: fn(),
    listBotDepartments: fn(),
    listBotAssignments: fn(),
    cancelBotAssignment: fn(),
  };
  // 替身 store 必须像 zustand 一样在 set 之后通知订阅者，见下方 vi.mock("@/stores")。
  const listeners = new Set<() => void>();
  const conversationState: { activeConversationId: string | null } = {
    activeConversationId: "conv-1",
  };
  const botState = new Proxy(state, {
    set(target, prop, value) {
      Reflect.set(target, prop, value);
      for (const listener of listeners) {
        listener();
      }
      return true;
    },
  });
  return { botState, conversationState, listeners };
});

const { botState } = h;

function makeBot(overrides: Partial<Bot> = {}): Bot {
  return {
    id: "bot-a",
    name: "Bot A",
    title: null,
    description: null,
    soul: "",
    soulHash: "",
    soulDrift: false,
    icon: "🤖",
    color: null,
    section: "",
    modelId: null,
    providerId: null,
    effort: null,
    workdir: null,
    approvalMode: "full",
    agentProfileId: null,
    isEnabled: true,
    sortOrder: 0,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

function makeWorkflow(id: string, name: string): BotWorkflow {
  return {
    id,
    name,
    description: "desc",
    ownerScope: "instance",
    departmentId: null,
    steps: [
      {
        stepKey: "step1",
        botId: "bot-a",
        promptTemplate: "do {{inputs.topic}}",
        dependsOn: [],
        timeoutSecs: 300,
        onFailure: "abort",
      },
    ],
    maxConcurrency: 2,
    totalTimeoutSecs: 900,
    createdAt: 1,
    updatedAt: 1,
  };
}

function makeRun(workflow: BotWorkflow, status: BotWorkflowRun["status"]): BotWorkflowRun {
  return {
    id: "run-1",
    workflowId: workflow.id,
    workflowName: workflow.name,
    status,
    origin: "person",
    requesterBotId: null,
    requesterConversationId: "conv-1",
    anchorMessageId: "anchor-1",
    context: {
      inputs: { topic: "AI" },
      outputs: { step1: { assignmentId: "as-1", resultText: "步骤回执内容" } },
      skipped: [],
    },
    completedSteps: status === "completed" ? 1 : 0,
    totalSteps: 1,
    errorCode: null,
    errorDetail: null,
    startedAt: 1,
    finishedAt: status === "completed" ? 2 : null,
    createdAt: 1,
    updatedAt: 2,
  };
}

function makeAssignment(): BotAssignment {
  return {
    id: "as-1",
    rootId: "as-1",
    parentId: null,
    depth: 1,
    origin: "workflow",
    workflowRunId: "run-1",
    stepKey: "step1",
    requesterBotId: null,
    requesterConversationId: "conv-1",
    assigneeBotId: "bot-a",
    assigneeName: "Bot A",
    mode: "dispatch",
    taskText: "do AI",
    status: "completed",
    resultText: "步骤回执内容",
    errorCode: null,
    errorDetail: null,
    startedAt: 1,
    finishedAt: 2,
    createdAt: 1,
    updatedAt: 2,
  };
}

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({ t: (key: string) => key }),
}));

// 真实 zustand store 在 set() 后会让订阅者重渲染；替身不同构时「run 落库 → 运行面板仍空白」
// 这类假阴性只会挂在测试替身上（2026-11-24 实测：只有切 tab 才刷新 → 关键流程用例假失败）。
vi.mock("@/stores", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const subscribe = (listener: () => void) => {
    h.listeners.add(listener);
    return () => {
      h.listeners.delete(listener);
    };
  };
  return {
    useBotStore: (selector: (s: typeof h.botState) => unknown) =>
      React.useSyncExternalStore(subscribe, () => selector(h.botState)),
    useConversationStore: (selector: (s: { activeConversationId: string | null }) => unknown) =>
      React.useSyncExternalStore(subscribe, () => selector(h.conversationState)),
  };
});

vi.mock("@/lib/invoke", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/errorI18n", () => ({
  backendErrorText: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

import { BotWorkflowsPage } from "../BotWorkflowsPage";

function renderPage() {
  return render(
    <MemoryRouter initialEntries={["/bots/workflows"]}>
      <App>
        <Routes>
          <Route path={BUILTIN_PAGE_PATH.botWorkflows} element={<BotWorkflowsPage />} />
        </Routes>
      </App>
    </MemoryRouter>,
  );
}

describe("BotWorkflowsPage (P3/T8)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    botState.bots = [makeBot()];
    botState.workflows = [];
    botState.workflowRuns = [];
    botState.loading = false;
    botState.workflowLoading = false;
    botState.workflowSaving = false;
    botState.error = null;
    botState.loadBots.mockResolvedValue(botState.bots);
    botState.loadWorkflows.mockImplementation(async () => botState.workflows);
    botState.loadWorkflowRuns.mockImplementation(async () => botState.workflowRuns);
    botState.listBotDepartments.mockResolvedValue([] as BotDepartment[]);
    botState.listBotAssignments.mockResolvedValue([makeAssignment()]);
    botState.createWorkflow.mockImplementation(async (input: CreateBotWorkflowInput) => {
      const wf = makeWorkflow("wf-1", input.name);
      botState.workflows = [wf];
      return wf;
    });
    botState.runWorkflow.mockImplementation(async () => {
      const run = makeRun(botState.workflows[0], "completed");
      botState.workflowRuns = [run];
      return run;
    });
    botState.cancelWorkflowRun.mockResolvedValue(undefined);
    botState.createWorkflowSchedule.mockResolvedValue({} as never);
  });

  it("入口：导航注册表与路由表都暴露 /bots/workflows（侧栏入口 + 独立路由）", () => {
    expect(BUILTIN_PAGE_PATH.botWorkflows).toBe("/bots/workflows");
    expect(builtinNavItems.some((i) => i.key === "bot-workflows" && i.path === "/bots/workflows")).toBe(true);
    expect(NAV_GROUPS.find((g) => g.key === "chat")?.itemKeys).toContain("bot-workflows");
    expect(getNavItems().some((i) => i.path === "/bots/workflows")).toBe(true);
  });

  it("空态：无编排时显示空态与创建入口（不落空白页）", async () => {
    renderPage();
    expect(await screen.findByTestId("bot-workflows-page")).toBeInTheDocument();
    expect(await screen.findByTestId("bot-workflow-empty")).toBeInTheDocument();
    expect(screen.getByTestId("bot-workflow-create-first")).toBeEnabled();
  });

  it("空态：无 Bot 时创建入口禁用（无 Bot 空态不丢）", async () => {
    botState.bots = [];
    renderPage();
    expect(await screen.findByTestId("bot-workflow-empty")).toBeInTheDocument();
    expect(screen.getByTestId("bot-workflow-create-first")).toBeDisabled();
  });

  it("关键流程：创建编排 → 运行 → steps 进度 → outputs 回执 → 委派链可见", async () => {
    const user = userEvent.setup();
    renderPage();

    expect(await screen.findByTestId("bot-workflow-empty")).toBeInTheDocument();

    // ① 新建编排（编辑器 = 真实组件，走与产品一致的表单）
    await user.click(screen.getByTestId("bot-workflow-new"));
    await user.type(await screen.findByTestId("bot-workflow-name"), "日报编排");
    const stepCard = screen.getByTestId("bot-workflow-step-card");
    const botSelect = within(stepCard).getByTestId("bot-workflow-step-bot");
    await user.click((botSelect.querySelector(".ant-select-selector") as HTMLElement | null) ?? botSelect);
    const dropdown = await waitFor(() => {
      const el = document.querySelector(".ant-select-dropdown:not(.ant-select-dropdown-hidden)");
      if (!el) {
        throw new Error("bot dropdown 未展开");
      }
      return el as HTMLElement;
    });
    await user.click(within(dropdown).getByText("Bot A"));
    await user.type(within(stepCard).getByTestId("bot-workflow-step-prompt"), "do {{inputs.topic}}");
    await user.click(screen.getByTestId("bot-workflow-save"));

    await waitFor(() => expect(botState.createWorkflow).toHaveBeenCalledTimes(1));
    expect(botState.createWorkflow.mock.calls[0][0]).toMatchObject({
      name: "日报编排",
      ownerScope: "instance",
      steps: [expect.objectContaining({ stepKey: "step1", botId: "bot-a" })],
    });

    // ② 保存后回到定义列表并出现该编排
    expect(await screen.findByTestId("bot-workflow-item")).toBeInTheDocument();

    // ③ 切到运行面板 → 手工运行（带当前线程 id）
    await user.click(screen.getByRole("tab", { name: "bot.workflowRun.title" }));
    botState.loadWorkflowRuns.mockResolvedValue([makeRun(botState.workflows[0], "completed")]);
    await user.click(await screen.findByTestId("bot-workflow-run"));
    await waitFor(() =>
      expect(botState.runWorkflow).toHaveBeenCalledWith({
        workflowId: "wf-1",
        requesterConversationId: "conv-1",
      })
    );

    // ④ steps 进度与回执（outputs）可见：reloadRuns() 异步写完 store 后由事件/tab 切换刷新
    const runItem = await waitFor(() => screen.getByTestId("bot-workflow-run-item"), { timeout: 3000 });
    expect(runItem).toHaveAttribute("data-status", "completed");
    expect(within(runItem).getByTestId("bot-workflow-run-progress")).toBeInTheDocument();
    await user.click(within(runItem).getByTestId("bot-workflow-run-outputs-toggle"));
    expect(await screen.findByTestId("bot-workflow-run-outputs")).toHaveTextContent("步骤回执内容");

    // ⑤ 委派链组（workflow 回执行按 rootId 归属）
    await user.click(screen.getByRole("tab", { name: "bot.assignment.chain" }));
    expect(await screen.findByTestId("bot-delegation-chain-panel")).toBeInTheDocument();
    expect(screen.getAllByTestId("bot-assignment-item").length).toBeGreaterThan(0);
  });
});
