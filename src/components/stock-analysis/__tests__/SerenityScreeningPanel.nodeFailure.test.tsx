import i18n from "@/i18n";
import { type StepLog, useSerenityStore } from "@/stores/feature/serenityStore";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { SerenityScreeningPanel } from "../SerenityScreeningPanel";

/**
 * 节点失败**展示层**的防回归测试。
 *
 * 背景：`StepLog.error` 是后端 `NodeError::Display` 的**自由文本**（形如
 * `"EXECUTION_CANCELLED: 节点执行已取消"`），此前被面板**直接渲染** ⇒ 中文文案漏给所有语言用户。
 * 本轮起 `errorCode` 参与展示（`lib/errorI18n.ts::translateFailureText`）：
 *   ① **折叠摘要**（主文案）优先用 11 语言译文，无码 / 码未收录时回退原文；
 *   ② **展开态的原文详情保持原样** —— 本地化不得以「丢原因」为代价
 *      （detail 里有 LLM 报错正文 / IO 详情，没有对应译文）；
 *   ③ 展开块**不重复渲染**主文案（摘要已承担），否则界面出现两行同样的话。
 */
const { invokeMock, listenHandlers } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  /** `listen(事件名, handler)` 的 handler 捕获表 —— 让测试能直接投递后端事件。 */
  listenHandlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    listenHandlers.set(name, handler);
    return Promise.resolve(() => {});
  },
  isTauri: () => false,
  TimeoutError: class MockTimeoutError extends Error {},
}));

const CANCEL_TEXT = "EXECUTION_CANCELLED: 节点执行已取消";
const CANCEL_LOCALIZED = "分析节点因分析被取消而中止";

function setSteps(steps: StepLog[]) {
  act(() => {
    useSerenityStore.setState({ steps });
  });
}

function failedStep(error: string, errorCode: string | null | undefined): StepLog {
  return {
    nodeId: "c-bottleneck-trend1",
    status: "failed",
    error,
    errorCode,
    timestamp: 1,
  };
}

/** 渲染并冲掉挂载期的异步副作用（`invoke` 的 promise 回调会在渲染后才 setState）。 */
async function renderPanel() {
  const result = render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <SerenityScreeningPanel />
      </I18nextProvider>
    </MemoryRouter>,
  );
  await act(async () => {});
  return result;
}

beforeAll(async () => {
  await i18n.changeLanguage("zh-CN");
});

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  useSerenityStore.setState({ steps: [], running: false, error: null, errorDetail: null });
});

describe("SerenityScreeningPanel · 节点失败展示（errorCode 结构化）", () => {
  it("有码 ⇒ 折叠摘要用本地化译文，原文不出现在折叠态", async () => {
    setSteps([failedStep(CANCEL_TEXT, "STOCK_WORKFLOW_STEP_CANCELLED")]);
    await renderPanel();

    // 主文案 = 译文
    expect(screen.getByText(CANCEL_LOCALIZED)).toBeTruthy();
    // 自由文本（含中文）不再直接出现在折叠态
    expect(screen.queryByText(CANCEL_TEXT)).toBeNull();
  });

  it("有码 + 展开 ⇒ 摘要=译文、详情=原文，两者分工不重复", async () => {
    setSteps([failedStep(CANCEL_TEXT, "STOCK_WORKFLOW_STEP_CANCELLED")]);
    await renderPanel();

    fireEvent.click(screen.getByText(CANCEL_LOCALIZED));

    // 主文案只由折叠摘要承担 ⇒ 展开块**不得**再渲染一次（否则界面出现重复文案）
    expect(screen.getAllByText(CANCEL_LOCALIZED)).toHaveLength(1);
    // 详情 = 原文全文
    expect(screen.getByText(CANCEL_TEXT)).toBeTruthy();
  });

  it("无码（null = 后端明示无失败）⇒ 原文原样展示，零回归", async () => {
    setSteps([failedStep("boom failure", null)]);
    await renderPanel();

    expect(screen.getByText("boom failure")).toBeTruthy();
  });

  it("无码 + 展开 ⇒ 原文详情可见（与改动前行为一致）", async () => {
    setSteps([failedStep("boom failure", null)]);
    await renderPanel();

    fireEvent.click(screen.getByText("boom failure"));

    // 摘要 + 详情同源 ⇒ 共 2 处；这是改动前就有的行为，非本轮引入
    expect(screen.getAllByText("boom failure")).toHaveLength(2);
  });

  it("timeout 也走同一路径（与 failed 同属失败类）", async () => {
    setSteps([failedStep("TIMEOUT: 节点执行超时", "STOCK_WORKFLOW_TIMEOUT")]);
    await renderPanel();

    expect(screen.getByText("分析超时，请稍后重试")).toBeTruthy();
  });
});

describe("SerenityScreeningPanel · 工作流级失败展示（completed 通道的结构化码）", () => {
  it("有码 ⇒ 按码本地化整句，不再把含中文的自由文本直接渲染", async () => {
    await renderPanel();
    const handler = listenHandlers.get("serenity-screening-completed");
    expect(handler).toBeTruthy();

    act(() => {
      handler!({
        payload: {
          status: "failed",
          // 真实形态：`format!("Serenity 筛选工作流失败: {e}")`，且 `WorkflowError::Display`
          // 的 LifecycleHookFailed 变体本身自带中文 ⇒ 两层中文叠加
          error: "Serenity 筛选工作流失败: 生命周期钩子 'data-quality-precheck' 阻断执行: 数据缺失",
          code: "STOCK_WORKFLOW_HOOK_BLOCKED",
        },
      });
    });

    expect(screen.getByText("前置检查未通过，分析已中止")).toBeTruthy();
  });

  it("无码（旧载荷）⇒ 回退原文，零回归", async () => {
    await renderPanel();
    const handler = listenHandlers.get("serenity-screening-completed");
    expect(handler).toBeTruthy();

    act(() => {
      handler!({ payload: { status: "failed", error: "legacy failure text" } });
    });

    expect(screen.getByText("legacy failure text")).toBeTruthy();
  });
});

/**
 * `partial_failure`（候选已产出、部分落库失败）这一通道此前把
 * `format!("写入 {} 失败: {e}")` 的**中文整句**直接渲染 —— 与节点级失败同型。
 * 本轮起产出端给「码 + params(股票代码) + detail(DB 原文)」三元组，
 * 展示端主文案取译文、详情行留原文。
 */
describe("SerenityScreeningPanel · 落库部分失败展示（partial_failure 的结构化码）", () => {
  const PERSIST_TEXT = "筛选已完成，但结果未能保存到历史记录";
  const DB_ERR = "error returned from database: deadlock detected";
  /** 旧载荷形态：中文整句 + 无码（迁移前 `format!` 的产物）。 */
  const LEGACY_TEXT = `写入 300567 失败: ${DB_ERR}`;

  function emitCompleted(payload: Record<string, unknown>) {
    const handler = listenHandlers.get("serenity-screening-completed");
    expect(handler).toBeTruthy();
    act(() => {
      handler!({ payload });
    });
  }

  it("有码 ⇒ 主文案本地化，且详情行补上「哪只」（DB 原文不丢）", async () => {
    await renderPanel();
    emitCompleted({
      status: "partial_failure",
      persistenceCode: "STOCK_WORKFLOW_PERSIST_FAILED",
      persistenceStockCode: "300567",
      persistenceError: DB_ERR,
    });

    expect(screen.getByText(PERSIST_TEXT)).toBeTruthy();
    // 本地化不得以「丢原因」为代价：DB 原文 + 股票代码仍在
    expect(screen.getByText(`300567: ${DB_ERR}`)).toBeTruthy();
  });

  it("有码但缺 persistenceStockCode ⇒ 详情行退化为纯 DB 原文，不留 `: ` 前缀", async () => {
    await renderPanel();
    emitCompleted({
      status: "partial_failure",
      persistenceCode: "STOCK_WORKFLOW_PERSIST_FAILED",
      persistenceStockCode: null,
      persistenceError: DB_ERR,
    });

    expect(screen.getByText(PERSIST_TEXT)).toBeTruthy();
    expect(screen.getByText(DB_ERR)).toBeTruthy();
    expect(screen.queryByText(`: ${DB_ERR}`)).toBeNull();
  });

  it("旧载荷（无码）⇒ 原文整句原样展示，且**不重复**补股票代码行", async () => {
    await renderPanel();
    emitCompleted({
      status: "partial_failure",
      persistenceError: LEGACY_TEXT,
    });

    // 零回归：无码时主文案即原文
    expect(screen.getByText(LEGACY_TEXT)).toBeTruthy();
    // 关键：原文整句里已含代码 ⇒ 详情行必须为 null，否则同一句话出现两遍
    expect(screen.getAllByText(LEGACY_TEXT)).toHaveLength(1);
  });

  it("候选不受影响：落库失败只是提示，候选项照常渲染", async () => {
    await renderPanel();
    emitCompleted({
      status: "partial_failure",
      candidates: [{ stockCode: "300567", stock_name: "麦捷科技", serenityScore: 88 }],
      persistenceCode: "STOCK_WORKFLOW_PERSIST_FAILED",
      persistenceStockCode: "300567",
      persistenceError: DB_ERR,
    });

    expect(screen.getByText("麦捷科技")).toBeTruthy();
    expect(screen.getByText(PERSIST_TEXT)).toBeTruthy();
  });
});
