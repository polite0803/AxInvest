import i18n from "@/i18n";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RecommendationPanel } from "../RecommendationPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function renderWithProviders() {
  return render(
    <MemoryRouter>
      <I18nextProvider i18n={i18n}>
        <RecommendationPanel />
      </I18nextProvider>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue({
    period: "short",
    picks: {},
    disabledStyles: [],
    generatedAt: Date.now(),
    rawSeedPoolSize: 0,
  });
  useTimeAnchorStore.setState({
    asOfDate: null,
    mode: "live",
    tourSeen: true,
    pendingLiveConfirm: false,
  });
});

afterEach(() => {
  useTimeAnchorStore.setState({ asOfDate: null, mode: "live" });
});

describe("RecommendationPanel — as-of propagation", () => {
  it("does not pass asOfDate when in live mode", async () => {
    renderWithProviders();
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    const last = invokeMock.mock.calls[invokeMock.mock.calls.length - 1];
    expect(last[0]).toBe("recommend_stocks");
    expect(last[1]).toEqual({ period: "short", asOfDate: null });
  });

  it("passes asOfDate when in replay mode", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-06-01", mode: "replay" });
    renderWithProviders();
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    const last = invokeMock.mock.calls[invokeMock.mock.calls.length - 1];
    expect(last[0]).toBe("recommend_stocks");
    expect(last[1]).toEqual({ period: "short", asOfDate: "2026-06-01" });
  });

  it("shows a replay banner when in replay mode", async () => {
    useTimeAnchorStore.setState({ asOfDate: "2026-06-01", mode: "replay" });
    renderWithProviders();
    await waitFor(() => {
      expect(
        screen.getByText(
          i18n.t("timeTravel.recommendationBanner", { date: "2026-06-01" }),
        ),
      ).toBeInTheDocument();
    });
  });

  it("does NOT show a replay banner in live mode", async () => {
    renderWithProviders();
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(
      screen.queryByText(
        i18n.t("timeTravel.recommendationBanner", { date: "" }),
      ),
    ).toBeNull();
  });
});
