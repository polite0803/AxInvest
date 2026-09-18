// SPDX-License-Identifier: AGPL-3.0-only

import { DevToolsPage } from "@/pages/DevTools/DevToolsPage";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

// 子面板各自依赖后端 invoke / store，本测试只关心「URL ↔ 活跃子页」的契约，
// 因此把它们替换成可断言的占位节点。
vi.mock("@/pages/DevTools/TraceExplorer", () => ({
  TraceExplorer: () => <div data-testid="panel-trace-explorer" />,
}));
vi.mock("@/pages/DevTools/BenchmarkRunner", () => ({
  BenchmarkRunner: () => <div data-testid="panel-benchmark" />,
}));
vi.mock("@/pages/DevTools/ToolRecommender", () => ({
  ToolRecommender: () => <div data-testid="panel-tool-recommender" />,
}));
vi.mock("@/pages/FineTunePage", () => ({
  FineTunePage: () => <div data-testid="panel-fine-tune" />,
}));
vi.mock("@/components/devtools/RLTrainingPanel", () => ({
  RLTrainingPanel: () => <div data-testid="panel-rl-training" />,
}));

function AddrProbe() {
  const location = useLocation();
  return <span data-testid="addr">{`${location.pathname}${location.search}`}</span>;
}

function renderAt(entry: string) {
  return render(
    <MemoryRouter initialEntries={[entry]}>
      <DevToolsPage />
      <AddrProbe />
    </MemoryRouter>,
  );
}

function addr(): string {
  return screen.getByTestId("addr").textContent ?? "";
}

describe("DevToolsPage — 子页真相源在 URL", () => {
  it("?sub=benchmark 落在「基准测试」（此前 6 条子路由被压平，永远落在首个 Tab）", () => {
    renderAt("/chat?ws=devtools&sub=benchmark");

    expect(screen.getByTestId("panel-benchmark")).toBeInTheDocument();
    expect(screen.queryByTestId("panel-trace-explorer")).not.toBeInTheDocument();
  });

  it("?sub=rl-training 落在末位子页（验证不是「只认第一个」的巧合）", () => {
    renderAt("/chat?ws=devtools&sub=rl-training");

    expect(screen.getByTestId("panel-rl-training")).toBeInTheDocument();
    expect(screen.queryByTestId("panel-trace-explorer")).not.toBeInTheDocument();
  });

  it("无 sub 时回落到默认子页", () => {
    renderAt("/chat?ws=devtools");

    expect(screen.getByTestId("panel-trace-explorer")).toBeInTheDocument();
  });

  it("脏 sub 回落到默认子页（不因手改 URL 而渲染空面板）", () => {
    renderAt("/chat?ws=devtools&sub=__bogus__");

    expect(screen.getByTestId("panel-trace-explorer")).toBeInTheDocument();
  });

  it("点子 Tab 写回 sub，且保留 ws=devtools（丢掉 ws 会让工作台落到别的 Tab）", () => {
    renderAt("/chat?ws=devtools&sub=trace-explorer");

    // 顺序与 DEVTOOLS_SUBS 一致：0=trace-explorer 1=benchmark
    const tabs = screen.getAllByRole("tab");
    fireEvent.click(tabs[1]);

    const params = new URLSearchParams(addr().split("?")[1] ?? "");
    expect(params.get("sub")).toBe("benchmark");
    expect(params.get("ws")).toBe("devtools");
    expect(screen.getByTestId("panel-benchmark")).toBeInTheDocument();
  });
});
