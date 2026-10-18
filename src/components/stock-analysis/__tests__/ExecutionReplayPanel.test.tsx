import { render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

// workflowStatusLabel 走 @/i18n 实例（与 lib/errorI18n.ts 同形态），不经 useTranslation，
// 所以要单独替身；返回带前缀的串，才能证明组件确实查了译文键而不是直显后端原文。
vi.mock("@/i18n", () => ({
  default: { t: (key: string) => `T(${key})` },
}));

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

import { ExecutionReplayPanel } from "../ExecutionReplayPanel";

// 现网真实形态：created_at_ms 是**毫秒**（后端由秒换算而来）。
const MS = 1_792_026_000_000;

function row(over: Record<string, unknown>) {
  return {
    id: "30a89877-b865-4d55-87a5-6eb5f3f55339",
    workflowId: "stock-analysis",
    status: "failed",
    totalTimeMs: 0,
    errorDetail: null,
    createdAtMs: MS,
    ...over,
  };
}

/** 按 workflowId 分派的替身：面板现在会同时拉完整链与快速链。 */
function stubExecutions(byWorkflow: Record<string, ReturnType<typeof row>[]>) {
  invokeMock.mockImplementation((cmd: string, args?: { workflowId?: string }) => {
    if (cmd !== "list_workflow_executions") { return Promise.resolve(null); }
    return Promise.resolve(byWorkflow[args?.workflowId ?? ""] ?? []);
  });
}

describe("ExecutionReplayPanel", () => {
  it("状态走译文键而不是后端原文，且失败原因可见", async () => {
    stubExecutions({
      "stock-analysis": [row({ errorDetail: 'input_schema 校验失败："" is required' })],
    });
    const { container } = render(<ExecutionReplayPanel />);
    await waitFor(() => expect(container.textContent).toContain("30a89877"));

    // 后端原文 failed 不得直接上屏
    expect(container.textContent).toContain("T(chat.workflow.workflowStatus.failed)");
    // 失败原因要渲染出来 —— 此前只进日志，界面上只有一个 failed 标签
    expect(container.textContent).toContain("input_schema 校验失败");
  });

  it("毫秒时间戳渲染成日期，不是 Invalid Date", async () => {
    stubExecutions({ "stock-analysis": [row({})] });
    const { container } = render(<ExecutionReplayPanel />);
    await waitFor(() => expect(container.textContent).toContain("30a89877"));
    expect(container.textContent).not.toContain("Invalid Date");
  });

  // 坏样本：时间戳缺失（正是后端输出 snake_case created_at、前端读 createdAtMs 得
  // undefined 时的形态）。这条若恒绿，上面那条「不是 Invalid Date」就没有电。
  it("坏样本自证：时间戳缺失时确实会显示 Invalid Date", async () => {
    stubExecutions({ "stock-analysis": [row({ createdAtMs: undefined })] });
    const { container } = render(<ExecutionReplayPanel />);
    await waitFor(() => expect(container.textContent).toContain("30a89877"));
    expect(container.textContent).toContain("Invalid Date");
  });

  // 只锁「errorDetail 为 null 时不渲染错误行」这一格：状态译文已由第一条覆盖，
  // 在这里重复断言会让破坏状态译文连带打红本条（实测破坏一次红两条）。
  it("无失败原因时不渲染错误行", async () => {
    stubExecutions({
      "stock-analysis": [row({ status: "completed", errorDetail: null })],
    });
    const { container } = render(<ExecutionReplayPanel />);
    await waitFor(() => expect(container.textContent).toContain("30a89877"));
    expect(container.textContent).not.toContain("input_schema");
  });

  // 快速分析链（stock-analysis-fast）的执行此前永不出现：面板只拉完整链的 id。
  it("快速分析链的执行也会出现在列表里", async () => {
    stubExecutions({
      "stock-analysis": [],
      "stock-analysis-fast": [row({ id: "feedface-1111-2222-3333-444455556666" })],
    });
    const { container } = render(<ExecutionReplayPanel />);
    await waitFor(() => expect(container.textContent).toContain("feedface"));
  });

  // 启动回收会把上一进程遗留的执行行写成 `interrupted`（`scheduler/restore.rs`）。
  // 本条只锁「该状态有译文键、且不再显示转圈的 running 图标」：`STATUS_I18N_KEY` 里
  // 漏加 interrupted 时，`workflowStatusLabel` 会回退成后端原文 `interrupted` ⇒ 本条红。
  it("回收写入的 interrupted 状态走译文键，不再是转圈态", async () => {
    stubExecutions({
      "stock-analysis": [row({ id: "aborted1-0000-0000-0000-000000000001", status: "interrupted" })],
    });
    const { container } = render(<ExecutionReplayPanel />);
    await waitFor(() => expect(container.textContent).toContain("aborted1"));
    expect(container.textContent).toContain("T(chat.workflow.workflowStatus.interrupted)");
    expect(container.querySelector(".animate-spin")).toBeNull();
  });
});
