import i18n from "@/i18n";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ReplayWorkbench } from "../ReplayWorkbench";

const navigateMock = vi.fn();
vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return {
    ...actual,
    useNavigate: () => navigateMock,
  };
});

function renderWithProviders() {
  return render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <ReplayWorkbench />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  navigateMock.mockReset();
  useTimeAnchorStore.setState({
    asOfDate: null,
    mode: "live",
    tourSeen: true,
    pendingLiveConfirm: false,
  });
});

afterEach(async () => {
  // 收尾钩子在 RTL 自动 cleanup（卸载）之前执行 ⇒ 这里直接改 store 会让仍挂载的订阅
  // 组件在 act 之外重渲染 ⇒ React 报 "not wrapped in act"。故须 act 包住。
  await act(async () => {
    useTimeAnchorStore.setState({ asOfDate: null, mode: "live" });
  });
});

/**
 * 收尾：等挂载副作用（数据请求）落地。
 *
 * 为什么必须有：页面挂载后会异步拉数据，测试同步结束 ⇒ setState 落在测试之外 ⇒
 * React 报 "not wrapped in act"。为什么必须放在**断言之后**：这些用例断言的是
 * **初始渲染态**，提前 flush 会把组件推进"加载完成"态，断言随之失效。
 * 为什么不能用 afterEach：React 是在 setState 那一刻检查的，而那一刻在测试函数返回后、
 * afterEach 之前 —— 收尾钩子永远太晚。
 */
const settle = async () => {
  await act(async () => {});
};

describe("ReplayWorkbench", () => {
  it("renders the workbench title", () => {
    renderWithProviders();
    expect(screen.getByText(i18n.t("replayWorkbench.title"))).toBeInTheDocument();
  });

  it("keeps the live/as-of anchor on the standalone page header", () => {
    renderWithProviders();
    // 回放工作台是独立整页（不在 InvestHub 内），仍需自带世界观切换
    expect(document.querySelector('[data-testid="page-time-anchor"]')).toBeInTheDocument();
  });

  it("does NOT pre-fill the date picker (forces re-selection)", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-05-01", mode: "replay" });
    renderWithProviders();
    // 第一次进入 workbench 总是空白 picker，强迫用户重选
    expect(screen.getByText(i18n.t("replayWorkbench.step1.title"))).toBeInTheDocument();
    await settle();
  });

  it("shows the action card only after picking a date", () => {
    renderWithProviders();
    // action 区域没出现
    expect(screen.queryByTestId("goto-stock-analysis")).toBeNull();
  });

  it("hosts the time-travel batch replay card (historical verification, not live screening)", () => {
    renderWithProviders();
    // 批次复盘归属历史验证域，常驻本工作台，与 ReplaySweep 并列；
    // 不依赖上方锚点是否已选（卡片自带「N 个交易日前 / 指定日期」）
    expect(screen.getByTestId("ttb-start-btn")).toBeInTheDocument();
  });

  it("defaults the batch card to fixed-date mode seeded with a picked anchor", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-05-01", mode: "replay" });
    renderWithProviders();
    const pickerHost = document.querySelector('[data-testid="ttb-fixed-date"]');
    const pickerInput = pickerHost?.matches("input")
      ? pickerHost
      : pickerHost?.querySelector("input");
    expect(pickerInput?.getAttribute("value")).toBe("2026-05-01");
    await settle();
  });

  it("renders an exit panel with Switch-to-Live button when mode is replay", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-05-01", mode: "replay" });
    renderWithProviders();
    expect(screen.getByTestId("replay-exit-btn")).toBeInTheDocument();
    await settle();
  });

  it("exit button triggers enterLive", () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-05-01", mode: "replay" });
    renderWithProviders();
    act(() => {
      fireEvent.click(screen.getByTestId("replay-exit-btn"));
    });
    // enterLive(false) → 直接切到 live，asOfDate 清空
    expect(useTimeAnchorStore.getState().mode).toBe("live");
    expect(useTimeAnchorStore.getState().asOfDate).toBeNull();
  });
});
