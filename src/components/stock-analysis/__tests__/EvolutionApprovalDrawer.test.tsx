import i18n from "@/i18n";
import type { EvolutionHistoryEntry } from "@/stores/feature/stockAnalysisStore";
import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EvolutionApprovalDrawer } from "../EvolutionApprovalDrawer";

/** 让 invoke 的 Promise 链在 act 内 flush 干净。 */
const flushPromises = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

const SAMPLES = {
  reflections: ["rf-1"],
  validations: [],
  moverEvents: ["mv-1"],
  blockedReflections: ["rf-bad-1"],
  blockedValidations: [],
  blockedMoverEvents: ["mv-bad-1"],
};

function makePending(over: Partial<EvolutionHistoryEntry> = {}): EvolutionHistoryEntry {
  return {
    id: "evo-1",
    planId: "plan-1",
    trigger: JSON.stringify({ type: "manual_trigger", reason: "晚间聚合触发" }),
    evolutionType: "weight_decay",
    qualityBefore: 40,
    status: "pending_approval",
    improvementSummary: "建议上调 EWMA 平滑系数",
    triggerSampleJson: JSON.stringify(SAMPLES),
    createdAt: 1_790_000_000_000,
    ...over,
  };
}

const ACCEPTED = makePending({
  id: "evo-acc",
  status: "accepted",
  reviewer: "local-user",
  reviewedAt: 1_790_000_100_000,
  rollbackRef: "evo-0",
  parameterResultJson: JSON.stringify({
    defaultConfig: { lookbackDays: 30, ewmaAlpha: 0.3, sampleSaturation: 20 },
    bestConfig: { lookbackDays: 25, ewmaAlpha: 0.4, sampleSaturation: 18 },
    evolutionStats: {},
  }),
});

const FIRST_GEN_ACCEPTED = makePending({
  id: "evo-first",
  status: "accepted",
  rollbackRef: null,
});

function renderDrawer(selected: EvolutionHistoryEntry | null) {
  return render(
    <I18nextProvider i18n={i18n}>
      <App>
        <EvolutionApprovalDrawer
          open
          selected={selected}
          onSelect={() => {}}
          onClose={() => {}}
        />
      </App>
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  useStockAnalysisStore.getState().reset();
});

afterEach(() => {
  cleanup();
});

describe("EvolutionApprovalDrawer P4 计划-审批", () => {
  it("pending 行：触发样本分组展示，blocked 组红色标签 + 展开可见样本 id", async () => {
    invokeMock.mockResolvedValue([makePending()]);
    renderDrawer(makePending());

    // 有效组与数据不可达组标签同时出现（blocked 组 Tag 带 ⛔ 前缀，用正则匹配）
    expect(await screen.findByText(/^反思样本$/)).toBeTruthy();
    expect(screen.getByText(/反思样本（数据不可达）/)).toBeTruthy();
    expect(screen.getByText(/调仓事件（数据不可达）/)).toBeTruthy();
    // 列表 Tag + 详情状态 Tag 均为「待审批」
    expect(screen.getAllByText("待审批").length).toBeGreaterThan(0);

    // 展开全部 id 折叠面板（4 个非空组各一个同名 header），断言 blocked id 落盘可见
    for (const header of screen.getAllByText("查看 id 列表")) {
      fireEvent.click(header);
    }
    expect(screen.getByText("rf-1")).toBeTruthy();
    expect(screen.getByText("rf-bad-1")).toBeTruthy();
    expect(screen.getByText("mv-bad-1")).toBeTruthy();
  });

  it("接受：Popconfirm 确认后以 camelCase 载荷调 approve_evolution_plan", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "approve_evolution_plan") {
        return Promise.resolve({ ...makePending(), status: "accepted" });
      }
      return Promise.resolve([makePending()]);
    });
    renderDrawer(makePending());
    expect(await screen.findByText("进化计划审批")).toBeTruthy();

    // 触发按钮与 Popconfirm 浮层确认按钮同名（按钮 accessible name 含图标名，
    // 故按文本节点定位再上溯 button）：先点触发器，再点浮层中的确认按钮
    const triggerBtn = screen.getAllByText("接受并执行")[0].closest("button")!;
    fireEvent.click(triggerBtn);
    await waitFor(() => expect(screen.getAllByText("接受并执行").length).toBeGreaterThan(1));
    const confirmBtn = screen.getAllByText("接受并执行")[1].closest("button")!;
    await act(async () => {
      fireEvent.click(confirmBtn);
      await flushPromises();
    });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("approve_evolution_plan", {
        historyId: "evo-1",
        reviewer: null,
      })
    );
  });

  it("拒绝：原因为空不发命令；填写后确认才调 reject_evolution_plan", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "reject_evolution_plan") {
        return Promise.resolve({ ...makePending(), status: "rejected" });
      }
      return Promise.resolve([makePending()]);
    });
    renderDrawer(makePending());
    expect(await screen.findByText("进化计划审批")).toBeTruthy();

    fireEvent.click(screen.getByText("拒绝").closest("button")!);
    const reasonInput = await screen.findByPlaceholderText(/请填写拒绝原因/);

    // 空原因确认：拦截，不发命令
    await act(async () => {
      fireEvent.click(screen.getByText("确认拒绝").closest("button")!);
      await flushPromises();
    });
    expect(invokeMock).not.toHaveBeenCalledWith(
      "reject_evolution_plan",
      expect.anything(),
    );

    // 填原因后确认：命令携带 trim 后原文
    fireEvent.change(reasonInput, { target: { value: "样本不足，暂不调整" } });
    await act(async () => {
      fireEvent.click(screen.getByText("确认拒绝").closest("button")!);
      await flushPromises();
    });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("reject_evolution_plan", {
        historyId: "evo-1",
        reason: "样本不足，暂不调整",
        reviewer: null,
      })
    );
  });

  it("accepted 行：展示 default → best 参数 diff，可回滚到 rollbackRef", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "rollback_evolution") {
        return Promise.resolve({ ...ACCEPTED, id: "evo-rollback-9" });
      }
      return Promise.resolve([makePending()]);
    });
    renderDrawer(ACCEPTED);
    expect(await screen.findByText("已接受")).toBeTruthy();

    // 三组参数 diff（文本混排，按容器 textContent 断言）。
    // antd 6 的 <Text strong> 外多包一层 span，故用 closest 上溯到块级 div。
    const diffBox = screen.getByText("参数变更预览（回测最优 vs 默认）").closest("div")!;
    expect(diffBox.textContent).toContain("0.3");
    expect(diffBox.textContent).toContain("0.4");
    expect(diffBox.textContent).toContain("30");
    expect(diffBox.textContent).toContain("25");
    expect(diffBox.textContent).toContain("20");
    expect(diffBox.textContent).toContain("18");

    // 回滚 Popconfirm（按文本节点上溯 button，避开图标 accessible name）。
    // 触发按钮含图标文本为「回滚」；浮层 ok 按钮无图标，antd 对两字汉字按钮自动
    // 插字距，accessible name 实际渲染为「回 滚」——用锚定空白容错正则精确区分两者。
    const rollbackTrigger = screen.getByText("回滚").closest("button")!;
    expect(rollbackTrigger).not.toBeDisabled();
    fireEvent.click(rollbackTrigger);
    const confirmBtn = await screen.findByRole("button", { name: /^\s*回\s*滚\s*$/ });
    await act(async () => {
      fireEvent.click(confirmBtn);
      await flushPromises();
    });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("rollback_evolution", {
        historyId: "evo-acc",
        reviewer: null,
      })
    );
  });

  it("第一代 accepted（无 rollbackRef）：回滚按钮禁用并给出提示", async () => {
    invokeMock.mockResolvedValue([]);
    renderDrawer(FIRST_GEN_ACCEPTED);
    expect(await screen.findByText("已接受")).toBeTruthy();
    const rollbackBtn = screen.getByText("回滚").closest("button")!;
    expect(rollbackBtn).toBeDisabled();
  });
});
