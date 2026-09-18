// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 旧链兼容回归：`/opc/industry/:packId` → `/opc/domain/:packId`
 *
 * 2026-09-15「行业」→「域」概念统一迁移后，域包路径前缀改为 `/opc/domain/`。
 * 本用例断言的是**真实路由表命中**（渲染 ContentArea 全表，而非单测重定向组件）：
 * 若重定向路由没接进 `<Routes>`，请求会落到 `path="*"` 的 404，
 * 探针路径不会变、`not-found` 会出现 —— 三种失败都判得出来。
 */

import { render, screen } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/components/layout/AppHeader", () => ({ AppHeader: () => null }));
vi.mock("@/hooks/useIpcHealth", () => ({ useIpcHealth: () => ({ ok: true }) }));
vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({ t: (k: string) => k }),
}));
vi.mock("antd", () => ({
  Button: ({ children }: { children?: React.ReactNode }) => <button>{children}</button>,
  Result: () => <div data-testid="not-found" />,
  Spin: () => <div data-testid="spin" />,
}));

// 域包页面本身不是本用例的被测对象，替换为桩，避免把整条 UI 依赖链拖进来
const DOMAIN_PAGE_EXPORTS = [
  "FinanceInvestPage",
  "AccountingPage",
  "SalesGrowthPage",
  "ProjectManagementPage",
  "ConsultingPage",
  "EcommercePage",
  "SoftwareDevPage",
  "SecurityPage",
  "GeospatialPage",
  "AiResearchPage",
  "ContentMediaPage",
  "DesignPage",
  "EducationPage",
  "GameDevPage",
] as const;

vi.mock("@/pages/opc/domains/DomainPages", () =>
  Object.fromEntries(
    DOMAIN_PAGE_EXPORTS.map((name) => [
      name,
      () => <div data-testid="domain-page">{name}</div>,
    ]),
  ));

function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="loc">{loc.pathname + loc.search}</div>;
}

describe("旧链兼容 /opc/industry/* → /opc/domain/*", () => {
  it("命中重定向，落到 /opc/domain/ 并保留查询串（不是 404）", async () => {
    const { ContentArea } = await import("../ContentArea");

    render(
      <MemoryRouter initialEntries={["/opc/industry/ai-research?tab=overview"]}>
        <LocationProbe />
        <ContentArea />
      </MemoryRouter>,
    );

    const loc = await screen.findByTestId("loc");
    expect(loc).toHaveTextContent("/opc/domain/ai-research?tab=overview");
    expect(screen.queryByTestId("not-found")).toBeNull();
  });

  it("新链 /opc/domain/* 直连无需重定向", async () => {
    const { ContentArea } = await import("../ContentArea");

    render(
      <MemoryRouter initialEntries={["/opc/domain/ai-research"]}>
        <LocationProbe />
        <ContentArea />
      </MemoryRouter>,
    );

    const loc = await screen.findByTestId("loc");
    expect(loc).toHaveTextContent("/opc/domain/ai-research");
    expect(await screen.findByTestId("domain-page")).toBeInTheDocument();
  });
});
