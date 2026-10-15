import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// 组件用 antd 的 `App.useApp()` 取 message。这里只替 message —— 包真 `<App>` 反而会让
// 本文件的渲染产出为空（实测 container 拿到空 div）。message 记下调用参数供断言。
const { messageErrorMock, isTauriMock } = vi.hoisted(() => ({
  messageErrorMock: vi.fn(),
  isTauriMock: vi.fn(() => true),
}));

vi.mock("antd", async (importOriginal) => {
  const actual = await importOriginal<typeof import("antd")>();
  return {
    ...actual,
    App: Object.assign(actual.App, {
      useApp: () => ({
        message: { error: messageErrorMock, success: () => {}, warning: () => {}, info: () => {} },
      }),
    }),
  };
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  isTauri: () => isTauriMock(),
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue("/tmp/trades.csv"),
}));

import { TradeImportPanel } from "../TradeImportPanel";

// 后端 ImportRow 的真实形态（crates/analysis-engine/src/trade_import.rs）
const row = {
  row: 2,
  stockCode: "600519",
  stockName: "贵州茅台",
  direction: "buy",
  price: 1680.5,
  quantity: 100,
  tradeDate: "2026-10-14",
  tradeTime: "10:30",
  fee: 5,
  notes: null,
  errors: [],
};

const renderPanel = () => render(<TradeImportPanel />);

/**
 * 收尾：等挂载副作用（数据请求）落地。
 *
 * 为什么必须有：页面挂载后会异步拉数据，测试同步结束 ⇒ setState 落在测试之外 ⇒
 * React 报 "not wrapped in act"。为什么必须放在**断言之后**：这些用例断言的是
 * **初始渲染态**，提前 flush 会把组件推进"加载完成"态，断言随之失效（实测会让 3 条用例变红）。
 * 为什么不能用 afterEach：React 是在 setState 那一刻检查的，而那一刻在测试函数返回后、
 * afterEach 之前 —— 收尾钩子永远太晚。
 */
const settle = async () => {
  await act(async () => {});
};

/** 走完「选文件 → 确认导入」两步，最后一步由 importResult 决定 import_trades 的返回。 */
async function runImport(container: HTMLElement, importResult: unknown) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "parse_trades_csv") { return Promise.resolve([row]); }
    if (cmd === "import_trades") { return Promise.resolve(importResult); }
    return Promise.resolve(null);
  });

  const buttons = [...container.querySelectorAll("button")];
  // 为什么包在 act 里: raw .click() 里同步的 setLoading(true) 会在离散事件 flushSync 中
  // 于 act 外提交 ⇒ 每次点击刷 "not wrapped in act" 警告。act 同步作用域只影响提交时机,
  // 点击处理函数自身的异步链(对话框/解析)不受影响,后面照旧 waitFor。
  act(() => {
    buttons.find((b) => b.textContent?.includes("trade.import.selectFile"))?.click();
  });
  await waitFor(() => {
    expect(container.textContent).toContain("trade.import.selectedFile");
  });

  const confirm = [...container.querySelectorAll("button")]
    .find((b) => b.textContent?.includes("trade.import.confirm"));
  act(() => {
    confirm?.click();
  });
  await waitFor(() => {
    expect(invokeMock).toHaveBeenCalledWith("import_trades", expect.anything());
  });
}

describe("TradeImportPanel 的失败明细", () => {
  beforeEach(() => {
    isTauriMock.mockReturnValue(true);
    messageErrorMock.mockClear();
  });

  // 导入成功后预览表会被清空 —— 失败明细不在这里列出来，用户就再也追不到是哪几行、为什么失败。
  it("列出每条失败的行号与原因", async () => {
    const { container } = renderPanel();
    await runImport(container, {
      total: 3,
      valid: 1,
      skipped: 1,
      failed: 1,
      errors: [[4, "价格为空"]],
      preview: [],
    });
    await waitFor(() => {
      expect(container.textContent).toContain("价格为空");
    });
    // 行号也要有，否则只知道「有一条失败」却不知道是哪一行
    expect(container.textContent).toContain("4");
    expect(container.textContent).toContain("trade.import.columns.errors");
    await settle();
  });

  // 负控：没有失败行时不得渲染那块明细（否则上一条就成了恒真门）。
  // 断言锁的是那块明细自己的标题键 —— 它只在 errors 非空时出现。
  it("负控：无失败行时不渲染失败明细", async () => {
    const { container } = renderPanel();
    await runImport(container, {
      total: 2,
      valid: 2,
      skipped: 0,
      failed: 0,
      errors: [],
      preview: [],
    });
    await waitFor(() => {
      expect(container.textContent).toContain("trade.import.summaryTotal");
    });
    expect(container.textContent).not.toContain("trade.import.columns.errors");
    await settle();
  });

  // 环境判断改用 isTauri()，不再靠错误文案里有没有 "dialog"/"Tauri" 字样嗅探 ——
  // 文案一变嗅探就失效，会把「插件不可用」误报成「解析失败」（压根没走到解析这一步）。
  it("浏览器模式下选文件失败，提示开发模式而不是解析失败", async () => {
    isTauriMock.mockReturnValue(false);
    const dialog = await import("@tauri-apps/plugin-dialog");
    vi.mocked(dialog.open).mockRejectedValueOnce(new Error("plugin not available"));

    const { container } = renderPanel();
    act(() => {
      [...container.querySelectorAll("button")]
        .find((b) => b.textContent?.includes("trade.import.selectFile"))
        ?.click();
    });

    await waitFor(() => {
      expect(messageErrorMock).toHaveBeenCalledWith("trade.import.devModeHint");
    });
    await settle();
  });

  // 负控：同样是 dialog 插件报错，但在 Tauri 环境里要报「解析失败」——
  // 证明上一条不是「任何错误都提示开发模式」。
  it("负控：Tauri 环境下同一错误报解析失败", async () => {
    isTauriMock.mockReturnValue(true);
    const dialog = await import("@tauri-apps/plugin-dialog");
    vi.mocked(dialog.open).mockRejectedValueOnce(new Error("plugin not available"));

    const { container } = renderPanel();
    act(() => {
      [...container.querySelectorAll("button")]
        .find((b) => b.textContent?.includes("trade.import.selectFile"))
        ?.click();
    });

    await waitFor(() => {
      expect(messageErrorMock).toHaveBeenCalledWith(
        expect.stringContaining("trade.import.parseFailed"),
      );
    });
    await settle();
  });
});
