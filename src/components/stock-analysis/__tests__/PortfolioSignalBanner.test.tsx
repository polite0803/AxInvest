// 运行门：组合级信号必须真有一条上屏的去处（PLAN 阶段 G 的 G-6）。
//
// 回归对象：跨股聚合器早就在产 `PortfolioSignal`，但发射点发的是裸字面量
// `"portfolio-signal"`，事件名不在权威表、前端零监听 ⇒ 「聚合器已注入但产出没人收」。
// 本文件锁的是消费侧：来一条就出一条、参数带全、未知方向不许兜成「中性」。
//
// 事件名两头一致 + payload 键集合逐项相等由 `scripts/check-monitor-event-authority.mjs`
// 的 R14/R15 管，不在这里重复。

import { fireEvent, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: unknown) => opts && typeof opts === "object" ? `${key}|${JSON.stringify(opts)}` : key,
    i18n: { language: "zh-CN" },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const handlers = new Map<string, (event: { payload: unknown }) => void>();
const invokeMock = vi.fn();

vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    handlers.set(name, handler);
    return Promise.resolve(() => {});
  },
  isTauri: () => true,
}));

import type { PortfolioSignalPayload } from "@/types";
import { directionLabel, PortfolioSignalBanner } from "../PortfolioDashboard";

/** 后端 `PortfolioSignal`（`rename_all = "camelCase"`）序列化出的整份形状。 */
function signalPayload(overrides: Record<string, unknown> = {}): PortfolioSignalPayload {
  return {
    direction: "bearish",
    stocks: ["601899", "300750", "600519"],
    dominantSignal: "support_break",
    strength: 0.7,
    snapshot: { positionCount: 3, topConcentrationPct: 41.2, maxSectorPct: 55.0, riskLevel: null },
    suggestedAction: "组合集中度风险升高，建议减仓 20%",
    timestamp: 1_760_000_000,
    ...overrides,
  } as PortfolioSignalPayload;
}

const body = () => document.body.textContent ?? "";

describe("PortfolioSignalBanner —— 组合级信号上屏", () => {
  it("还没收到信号时不占位（不得凭空出一条「无信号」）", () => {
    handlers.clear();
    render(<PortfolioSignalBanner />);
    expect(body()).not.toContain("stockAnalysis.portfolio.signalBody");
  });

  it("挂载即订阅权威事件名 portfolio-signal", async () => {
    handlers.clear();
    render(<PortfolioSignalBanner />);
    await waitFor(() => expect(handlers.has("portfolio-signal")).toBe(true));
  });

  it("收到看空信号 ⇒ 只数、方向、强度、后端建议一起进文案", async () => {
    handlers.clear();
    render(<PortfolioSignalBanner />);
    const handler = handlers.get("portfolio-signal");
    if (!handler) { throw new Error("监听器未注册"); }
    handler({ payload: signalPayload() });

    await waitFor(() => expect(body()).toContain("stockAnalysis.portfolio.signalBody"));
    expect(body()).toContain("stockAnalysis.portfolio.signalTitle");
    expect(body()).toContain('"count":3');
    expect(body()).toContain('"direction":"stockAnalysis.portfolio.signalBearish"');
    expect(body()).toContain('"pct":"70"');
    expect(body()).toContain('"action":"组合集中度风险升高，建议减仓 20%"');
  });

  it("看多信号与看空用不同色型（信息级 vs 警示级），文案仍各占一句", async () => {
    handlers.clear();
    const { container } = render(<PortfolioSignalBanner />);
    const handler = handlers.get("portfolio-signal");
    if (!handler) { throw new Error("监听器未注册"); }
    handler({ payload: signalPayload({ direction: "bullish" }) });

    await waitFor(() => expect(body()).toContain('"direction":"stockAnalysis.portfolio.signalBullish"'));
    expect(container.querySelector(".ant-alert-info")).not.toBeNull();
  });

  it("未见过的方向值原样显示，不兜成「中性」那一档", async () => {
    handlers.clear();
    render(<PortfolioSignalBanner />);
    const handler = handlers.get("portfolio-signal");
    if (!handler) { throw new Error("监听器未注册"); }
    handler({ payload: signalPayload({ direction: "mixed" }) });

    await waitFor(() => expect(body()).toContain('"direction":"mixed"'));
    expect(body()).not.toContain("stockAnalysis.portfolio.signalNeutral");
  });

  it("关掉横条后不再占据面板", async () => {
    handlers.clear();
    const { container } = render(<PortfolioSignalBanner />);
    const handler = handlers.get("portfolio-signal");
    if (!handler) { throw new Error("监听器未注册"); }
    handler({ payload: signalPayload() });
    await waitFor(() => expect(body()).toContain("stockAnalysis.portfolio.signalBody"));

    const close = container.querySelector(".ant-alert-close-icon");
    if (!close) { throw new Error("横条没给关闭入口：用户只能一直看着上一条过期信号"); }
    fireEvent.click(close);
    await waitFor(() => expect(body()).not.toContain("stockAnalysis.portfolio.signalBody"));
  });
});

describe("directionLabel —— 方向三档与未知兜底", () => {
  const t = ((key: string) => key) as never;
  it("三档各有自己的键", () => {
    expect(directionLabel(t, "bullish")).toBe("stockAnalysis.portfolio.signalBullish");
    expect(directionLabel(t, "bearish")).toBe("stockAnalysis.portfolio.signalBearish");
    expect(directionLabel(t, "neutral")).toBe("stockAnalysis.portfolio.signalNeutral");
  });

  it("未知值原样返回，不塞进已知那一档", () => {
    expect(directionLabel(t, "flat")).toBe("flat");
  });
});
