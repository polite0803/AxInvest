import { render, screen } from "@testing-library/react";
import { type ReactElement } from "react";
import { describe, expect, it, vi } from "vitest";

// PageErrorBoundary 走 i18next.t（模块级，非 useTranslation），未 init 时取不到键，
// 故桩成「键即文案」——本测试断言的是抛出文本有没有上屏，与译文无关。
vi.mock("i18next", () => ({
  default: { t: (key: string) => key },
}));

import { ErrorBoundary, PageErrorBoundary } from "../ErrorBoundary";

function Thrower({ message }: { message: string }): ReactElement {
  throw new Error(message);
}

describe("ErrorBoundary", () => {
  it("PageErrorBoundary 把 error.message 渲染到 UI，而非只给固定串", () => {
    render(
      <PageErrorBoundary title="Watchlist">
        <Thrower message="Cannot read properties of null (reading 'toFixed')" />
      </PageErrorBoundary>,
    );

    expect(screen.getByText("Watchlist")).toBeTruthy();
    // 固定串与抛出文本并存：前者说明「哪一类失败」，后者说明「哪个表达式抛的」
    expect(screen.getByText(/errorBoundary\.pageError/).textContent)
      .toContain("Cannot read properties of null (reading 'toFixed')");
  });

  it("抛出的不是 Error 实例时不崩兜底，退回固定串", () => {
    function ThrowNonError(): ReactElement {
      throw { notAnError: true };
    }
    render(
      <PageErrorBoundary title="Watchlist">
        <ThrowNonError />
      </PageErrorBoundary>,
    );

    expect(screen.getByText(/errorBoundary\.pageError/).textContent).not.toContain("notAnError");
  });

  it("dev 下把抛出组件链一起上屏", () => {
    function SuspectPanel(): ReactElement {
      throw new Error("Rendered fewer hooks than expected");
    }
    render(
      <PageErrorBoundary title="StockAnalysis">
        <SuspectPanel />
      </PageErrorBoundary>,
    );

    expect(import.meta.env.DEV).toBe(true);
    const sub = screen.getByText(/errorBoundary\.pageError/).textContent ?? "";
    expect(sub).toContain("Rendered fewer hooks than expected");
    // 帧序「抛出者在前」⇒ 用户屏幕自己带出该查哪个组件、哪一行
    expect(sub).toContain("SuspectPanel @ ErrorBoundary.test.tsx:");
  });

  it("静态 fallback 优先级不变（SettingsPage 依赖此行为）", () => {
    render(
      <ErrorBoundary fallback={<span>static-fallback</span>}>
        <Thrower message="ignored" />
      </ErrorBoundary>,
    );

    expect(screen.getByText("static-fallback")).toBeTruthy();
    expect(screen.queryByText(/ignored/)).toBeNull();
  });
});
