import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn().mockResolvedValue([]),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

import { ScreenerPage } from "../ScreenerPage";

const renderWithRouter = () => render(<ScreenerPage />, { wrapper: MemoryRouter });

describe("ScreenerPage", () => {
  it("renders without crashing", () => {
    const { container } = renderWithRouter();
    expect(container).toBeTruthy();
  });

  it("renders back button with chat label", () => {
    const { container } = renderWithRouter();
    const backButton = container.querySelector(".sa-header-back");
    expect(backButton).toBeTruthy();
    expect(backButton?.textContent).toContain("nav.chat");
  });

  it("renders the page title via i18n key", () => {
    const { container } = renderWithRouter();
    const title = container.querySelector(".sa-header-title");
    expect(title).toBeTruthy();
    expect(title?.textContent).toBe("screener.title");
  });

  it("renders a 3-column grid containing HotStocks/LimitUp/DragonTiger", () => {
    const { container } = renderWithRouter();
    const grid = container.querySelector(".grid");
    expect(grid).toBeTruthy();
    expect(grid?.children.length).toBe(3);
  });
});
