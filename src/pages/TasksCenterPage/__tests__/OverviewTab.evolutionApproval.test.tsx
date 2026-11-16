import i18n from "@/i18n";
import { __resetCardRegistryForTests } from "@/lib/dynamicUI/pluginCardRegistry";
import type { PluginTaskCardsInfoDto } from "@/types";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OverviewTab } from "../OverviewTab";

// #216 批2 ③-B：OverviewTab 改读宿主卡片投影（不再 import 股票域 store / 抽屉）。
// 本测试镜像 browserMock 的 `task_card_host_cards` 投影，走**真实注册管线**
// （ensureCardContributionsLoaded → registerCardContributions），不直接塞注册表。

const flushPromises = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

const invokeMock = vi.fn();
const pluginInvokeMock = vi.hoisted(() => ({ fn: vi.fn() }));

vi.mock("@/lib/pluginCommand", () => ({
  invokePluginCommand: (...args: unknown[]) => pluginInvokeMock.fn(...args),
}));

vi.mock("@/lib/invoke", async (importOriginal) => {
  // @/stores barrel 会消费 isTauri / listen 等导出，保留原模块仅替换 invoke
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: (...args: unknown[]) => invokeMock(...args) };
});

const HOST_CARDS: PluginTaskCardsInfoDto[] = [
  {
    id: "host",
    name: "host",
    version: "0.0.0",
    description: "host-owned task cards",
    author: null,
    enabled: true,
    taskCards: [
      {
        id: "evolution-approval",
        title: "tasksCenter.health.pendingEvolution",
        surface: "task_card",
        summaryCommand: "stock_list_pending_evolution_plans",
        detailSchema: null,
        tone: "gold",
        order: 0,
        actions: [
          {
            id: "approve",
            title: "stockAnalysis.evolutionApproval.approve",
            command: "stock_approve_evolution_plan",
            mutating: true,
          },
          {
            id: "reject",
            title: "stockAnalysis.evolutionApproval.reject",
            command: "stock_reject_evolution_plan",
            mutating: true,
          },
        ],
      },
    ],
  },
];

const PENDING_ROWS = [
  { id: "evo-overview-1", improvementSummary: "近窗口误差率超阈，建议下调回看天数" },
  { id: "evo-overview-2", improvementSummary: "样本饱和度不足，建议扩采样窗口" },
];

function renderOverview() {
  return render(
    <I18nextProvider i18n={i18n}>
      <App>
        <OverviewTab onViewRuns={() => {}} />
      </App>
    </I18nextProvider>,
  );
}

/** 静态 Modal.confirm 渲染在 document.body，主按钮按结构定位（不依赖 antd locale 文案）。 */
function clickConfirmPrimary() {
  const btn = document.querySelector<HTMLElement>(".ant-modal-confirm-btns .ant-btn-primary");
  expect(btn).toBeTruthy();
  fireEvent.click(btn!);
}

beforeEach(() => {
  __resetCardRegistryForTests();
  invokeMock.mockReset();
  pluginInvokeMock.fn.mockReset();
  pluginInvokeMock.fn.mockImplementation((name: string, input?: unknown) => invokeMock(name, input ?? {}));
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "task_card_host_cards") {
      return Promise.resolve(HOST_CARDS);
    }
    if (cmd === "plugin_list_task_cards") {
      return Promise.resolve([]);
    }
    if (cmd === "plugin_list") {
      // 插件解析路由：声明 stock 插件提供该行集命令
      return Promise.resolve([
        {
          id: "stock",
          enabled: true,
          commands: [{ name: "stock_list_pending_evolution_plans" }],
        },
      ]);
    }
    if (cmd === "stock_list_pending_evolution_plans") {
      return Promise.resolve(PENDING_ROWS);
    }
    return Promise.resolve([]);
  });
});

afterEach(() => {
  cleanup();
  // 静态 Modal.confirm 渲染在 React 树外（document.body 直挂），cleanup() 卸不掉；
  // 不清会跨用例污染（下一个用例的 `.ant-modal-confirm` 查询命中上一用例的残留弹层）。
  document.body.innerHTML = "";
  __resetCardRegistryForTests();
});

describe("OverviewTab 待审批进化卡（宿主投影驱动）", () => {
  it("计数与行集同源：挂载读投影取行数，点击卡片打开行级审批弹层", async () => {
    renderOverview();

    // {count} 契约：计数 = summaryCommand 行数（2 行），不另发 count 读数
    expect(await screen.findByText("待审批进化")).toBeTruthy();
    expect(screen.getByText("2")).toBeTruthy();

    await act(async () => {
      fireEvent.click(screen.getByText("待审批进化"));
      await flushPromises();
    });
    // 弹层列出全部行（与计数同一次 invoke 的行集）与宿主动作按钮（t 渲染 i18n key）
    expect(await screen.findByText("近窗口误差率超阈，建议下调回看天数")).toBeTruthy();
    expect(screen.getByText("样本饱和度不足，建议扩采样窗口")).toBeTruthy();
    expect(screen.getAllByText("接受并执行").length).toBe(2);
    expect(screen.getAllByText("拒绝").length).toBe(2);
    // 走的是投影 summaryCommand，而非旧 store 通道
    // 插件解析多一跳（plugin_list → 行集），断言放到 waitFor 里
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "stock_list_pending_evolution_plans",
        expect.anything(),
      )
    );
  });

  it("审批动作带行 id：确认后 invoke approve_evolution_plan 并刷新行集", async () => {
    renderOverview();

    await act(async () => {
      fireEvent.click(await screen.findByText("待审批进化"));
      await flushPromises();
    });
    const approveButtons = await screen.findAllByText("接受并执行");
    await act(async () => {
      fireEvent.click(approveButtons[0]);
      await flushPromises();
    });
    await act(async () => {
      clickConfirmPrimary();
      await flushPromises();
    });
    expect(invokeMock).toHaveBeenCalledWith("stock_approve_evolution_plan", {
      historyId: "evo-overview-1",
      reviewer: null,
    });
    // 成功后按同源刷新（第二次 stock_list_pending_evolution_plans）
    const rowCalls = invokeMock.mock.calls.filter((c) => c[0] === "stock_list_pending_evolution_plans");
    expect(rowCalls.length).toBe(2);
  });

  it("拒绝必填原因：空原因按钮禁用不放行，填原因后 invoke reject_evolution_plan", async () => {
    renderOverview();

    await act(async () => {
      fireEvent.click(await screen.findByText("待审批进化"));
      await flushPromises();
    });
    const rejectButtons = await screen.findAllByText("拒绝");
    await act(async () => {
      fireEvent.click(rejectButtons[0]);
      await flushPromises();
    });
    // 负控：原因非空前确定按钮禁用 → 点击无效，不 invoke
    const okBtn = document.querySelector<HTMLButtonElement>(".ant-modal-footer .ant-btn-primary");
    expect(okBtn).toBeTruthy();
    expect(okBtn!.disabled).toBe(true);
    await act(async () => {
      fireEvent.click(okBtn!);
      await flushPromises();
    });
    expect(pluginInvokeMock.fn).not.toHaveBeenCalledWith("stock_reject_evolution_plan", expect.anything());
    // 正控：填原因 → 按钮解禁 → invoke 带上 reason
    await act(async () => {
      fireEvent.change(screen.getByRole("textbox"), { target: { value: "参数回退风险过高" } });
      await flushPromises();
    });
    expect(okBtn!.disabled).toBe(false);
    await act(async () => {
      fireEvent.click(okBtn!);
      await flushPromises();
    });
    expect(pluginInvokeMock.fn).toHaveBeenCalledWith("stock_reject_evolution_plan", {
      historyId: "evo-overview-1",
      reviewer: null,
      reason: "参数回退风险过高",
    });
  });
});
