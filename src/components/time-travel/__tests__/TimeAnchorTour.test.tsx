import i18n from "@/i18n";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { act, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { TimeAnchorTour } from "../TimeAnchorTour";

beforeEach(() => {
  useTimeAnchorStore.setState({
    asOfDate: null,
    mode: "live",
    tourSeen: false,
    pendingLiveConfirm: false,
  });
});

afterEach(async () => {
  // 收尾钩子在 RTL 自动 cleanup（卸载）之前执行 ⇒ 这里直接改 store 会让仍挂载的订阅
  // 组件在 act 之外重渲染 ⇒ React 报 "not wrapped in act"。故须 act 包住。
  await act(async () => {
    useTimeAnchorStore.setState({ tourSeen: false });
  });
});

function renderWithI18n(ui: React.ReactNode) {
  return render(<I18nextProvider i18n={i18n}>{ui}</I18nextProvider>);
}

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

describe("TimeAnchorTour", () => {
  it("renders the tour bubble when tourSeen is false", () => {
    renderWithI18n(<TimeAnchorTour />);
    expect(screen.getByTestId("time-anchor-tour")).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toHaveAttribute(
      "aria-label",
      i18n.t("timeTravel.tour.title"),
    );
  });

  it("does not render when tourSeen is true", async () => {
    useTimeAnchorStore.setState({ tourSeen: true });
    renderWithI18n(<TimeAnchorTour />);
    expect(screen.queryByTestId("time-anchor-tour")).toBeNull();
    await settle();
  });

  it("renders the body text and a 'Got it' button", () => {
    renderWithI18n(<TimeAnchorTour />);
    expect(screen.getByText(i18n.t("timeTravel.tour.body"))).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: i18n.t("timeTravel.tour.gotIt") }),
    ).toBeInTheDocument();
  });

  it("renders the anchor hint pointing to the LIVE pill", () => {
    renderWithI18n(<TimeAnchorTour />);
    expect(
      screen.getByText(i18n.t("timeTravel.tour.stepAnchor")),
    ).toBeInTheDocument();
  });

  it("markTourSeen hides the bubble on next render", async () => {
    const { rerender } = renderWithI18n(<TimeAnchorTour />);
    expect(screen.getByTestId("time-anchor-tour")).toBeInTheDocument();
    // store 写入必须包在 act 内：此刻组件仍挂载且订阅 tourSeen，
    // act 外的写会在调度那一刻触发 "not wrapped in act" 警告（实测探针确认）。
    act(() => {
      useTimeAnchorStore.getState().markTourSeen();
    });
    rerender(
      <I18nextProvider i18n={i18n}>
        <TimeAnchorTour />
      </I18nextProvider>,
    );
    expect(screen.queryByTestId("time-anchor-tour")).toBeNull();
    await settle();
  });
});
