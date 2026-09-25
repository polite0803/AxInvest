// SPDX-License-Identifier: AGPL-3.0-only
//
// 批次 4 放行条件（`PLAN-stock-reflection-four-horizon.md` 阶段 D）的定向回归：
//   ① UI 能区分 mature / immature / unavailable / legacy；
//   ② null 不得显示为 0（既不能显示 0.00%，也不能把未判定显示成「正确/错误」）；
//   ③ 按 horizon 切换不会串用其它周期的 action 或收益；
//   ④ 空周期（该周期无条目）不生成 Tab —— 从源头避免「空周期误报」。
//
// 断言方式说明：antd Tabs 的**非激活**面板切走后仍留在 DOM 里（只是 hidden），
// 因此「不串线」类断言**必须限定在 `.ant-tabs-tabpane-active` 内**取文本；
// 若直接用 `container.textContent`，四个周期的值全都在 ⇒ 恒通过，等于没测。

import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

import type { HorizonResultsMap } from "@/types";
import { ExpandedReflectionRow } from "../ReflectionPanel";

// 用 importOriginal 保留真实模块的其余导出：`@/stores` 初始化链路会调用 `isTauri()`，
// 全量替换 mock 会让它抛 "No isTauri export is defined"（实测踩到）。
vi.mock("@/lib/invoke", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/invoke")>();
  return { ...actual, invoke: vi.fn(async () => null), listen: vi.fn(async () => () => {}) };
});

// 被测组件本身不读 store，但 `@/stores` 的 barrel 会拉起整棵 store 树（重且在 jsdom 下易崩），
// 故按最小面替换为普通对象。
vi.mock("@/stores", () => ({
  useStockAnalysisStore: (selector: (s: { stockCode: string | null }) => unknown) => selector({ stockCode: "600519" }),
}));

/** t 替身：直接回显 key（插值对象拼成 `key|k=v`），断言即可定位到具体文案 key */
const t = (key: string, opts?: object): string =>
  opts ? `${key}|${Object.entries(opts).map(([k, v]) => `${k}=${v}`).join(",")}` : key;

function makeRow(horizonResults?: HorizonResultsMap | null) {
  return {
    id: "r-1",
    stockCode: "600519",
    stockName: "贵州茅台",
    asOfDate: "2026-08-01",
    hindsightDate: "2026-09-01",
    actualOutcome: "outcome:up",
    whatWentWrong: null,
    missedSignals: null,
    fixForFuture: null,
    reflectionDepth: "light",
    minConfidenceThreshold: 60,
    status: "done",
    createdAt: 1_750_000_000_000,
    horizonResults,
  };
}

function renderRow(horizonResults?: HorizonResultsMap | null) {
  return render(
    <MemoryRouter>
      <ExpandedReflectionRow row={makeRow(horizonResults)} t={t} />
    </MemoryRouter>,
  );
}

/**
 * 仅取**当前激活**面板的文本 —— 见文件头「断言方式说明」。
 *
 * 用 `role="tabpanel"` + `aria-hidden` 定位，不依赖 class 名：当前 antd 的 TabPane
 * 激活类名是 `.ant-tabs-content-active`（rc-tabs 已把 `-pane-active` 改成 content 前缀），
 * 写死类名会随版本静默失效。
 */
function activePaneText(): string {
  const active = Array.from(document.querySelectorAll<HTMLElement>('[role="tabpanel"]')).filter(
    (p) => p.getAttribute("aria-hidden") !== "true",
  );
  if (active.length !== 1) {
    throw new Error(`激活面板数应为 1，实际 ${active.length}（面板总数见 DOM）`);
  }
  return active[0].textContent ?? "";
}

/**
 * 点击某个周期的 Tab。
 *
 * 不依赖 `data-node-key`（antd 内部属性，版本间可能变），改为按 Tab 文案里的
 * i18n key 定位（`horizonUltraShort` / `horizonShort` / `horizonMid` / `horizonLong`
 * 四个 key 互不为子串，故唯一）。
 */
function clickHorizonTab(key: string): void {
  const tabs = Array.from(document.querySelectorAll(".ant-tabs-nav .ant-tabs-tab"));
  const hit = tabs.find((el) => el.textContent?.includes(key));
  if (!hit) {
    throw new Error(
      `未找到周期 Tab「${key}」，实际 Tab 文案：${JSON.stringify(tabs.map((e) => e.textContent))}`,
    );
  }
  fireEvent.click(hit.querySelector('[role="tab"]') ?? hit);
}

function navTabCount(): number {
  return document.querySelectorAll(".ant-tabs-nav .ant-tabs-tab").length;
}

const LABEL = "stockAnalysis.reflection.horizon";

describe("ExpandedReflectionRow 四周期 Tab（批次 4）", () => {
  it("四个周期各自成熟时，切 Tab 只显示该周期自己的 action 与收益（不串线）", () => {
    renderRow({
      ultra_short: {
        status: "mature",
        expectedHoldingDays: 3,
        decision: { action: "BUY_ULTRA" },
        market: { returnPct: 3.5 },
        evaluation: { wasCorrect: 1 },
      },
      short: {
        status: "mature",
        expectedHoldingDays: 10,
        decision: { action: "SELL_SHORT" },
        market: { returnPct: -2.25 },
        evaluation: { wasCorrect: 0 },
      },
      mid: {
        status: "mature",
        expectedHoldingDays: 40,
        decision: { action: "HOLD_MID" },
        market: { returnPct: 1.0 },
        evaluation: { wasCorrect: 1 },
      },
      long: {
        status: "mature",
        expectedHoldingDays: 120,
        decision: { action: "BUY_LONG" },
        market: { returnPct: 12.75 },
        evaluation: { wasCorrect: 1 },
      },
    });

    // 默认激活首周期
    expect(activePaneText()).toContain("BUY_ULTRA");
    expect(activePaneText()).toContain("+3.50%");

    clickHorizonTab(`${LABEL}Short`);
    expect(activePaneText()).toContain("SELL_SHORT");
    expect(activePaneText()).toContain("-2.25%");
    // 反向断言：不得出现上一周期的判定与收益
    expect(activePaneText()).not.toContain("BUY_ULTRA");
    expect(activePaneText()).not.toContain("+3.50%");
    expect(activePaneText()).toContain("stockAnalysis.reflection.horizonVerdictWrong");

    clickHorizonTab(`${LABEL}Long`);
    expect(activePaneText()).toContain("BUY_LONG");
    expect(activePaneText()).toContain("+12.75%");
    expect(activePaneText()).not.toContain("HOLD_MID");
  });

  it("immature：显示样本不足说明与状态标签，不显示「正确 / 错误」，null 收益不显示为 0", () => {
    renderRow({
      short: {
        status: "immature",
        expectedHoldingDays: 10,
        decision: { action: "BUY_SHORT" },
        market: { returnPct: null, alphaPct: null, targetReached: null },
        evaluation: { wasCorrect: null, directionMatch: null, targetHit: null },
      },
    });

    const text = activePaneText();
    expect(text).toContain("stockAnalysis.reflection.horizonStatusImmature");
    expect(text).toContain("stockAnalysis.reflection.horizonImmatureNote");
    // 未判定不得被渲染成结论
    expect(text).not.toContain("stockAnalysis.reflection.horizonVerdictCorrect");
    expect(text).not.toContain("stockAnalysis.reflection.horizonVerdictWrong");
    // null 收益必须是「—」，不得伪报 0
    expect(text).toContain("—");
    expect(text).not.toContain("0.00%");
  });

  it("unavailable：显示行情不可用说明，且不渲染行情事实区块", () => {
    renderRow({
      mid: {
        status: "unavailable",
        expectedHoldingDays: 40,
        decision: { action: "HOLD_MID" },
        market: null,
        evaluation: { wasCorrect: null },
      },
    });

    const text = activePaneText();
    expect(text).toContain("stockAnalysis.reflection.horizonStatusUnavailable");
    expect(text).toContain("stockAnalysis.reflection.horizonUnavailableNote");
    expect(text).not.toContain("stockAnalysis.reflection.horizonMarketTitle");
    expect(text).not.toContain("stockAnalysis.reflection.horizonVerdictWrong");
  });

  it("legacy：显示 legacy 状态标签与 legacy 说明（四种状态文案互不混淆）", () => {
    renderRow({
      long: {
        status: "legacy",
        expectedHoldingDays: 120,
        decision: { action: "BUY_LONG" },
        market: { returnPct: 8.0 },
        evaluation: { wasCorrect: 1 },
      },
    });

    const text = activePaneText();
    expect(text).toContain("stockAnalysis.reflection.horizonStatusLegacy");
    expect(text).toContain("stockAnalysis.reflection.horizonLegacyNote");
    expect(text).not.toContain("stockAnalysis.reflection.horizonStatusMature");
    expect(text).not.toContain("stockAnalysis.reflection.horizonImmatureNote");
  });

  it("缺失周期不生成 Tab（空周期不可能被误报为正确 / 错误）", () => {
    renderRow({
      mid: {
        status: "mature",
        expectedHoldingDays: 40,
        decision: { action: "HOLD_MID" },
        market: { returnPct: 1.0 },
        evaluation: { wasCorrect: 1 },
      },
    });

    expect(navTabCount()).toBe(1);
    expect(activePaneText()).toContain("HOLD_MID");
    // 另外三个周期既不出现 Tab，也不出现「无该周期结果」占位串
    expect(screen.queryByText(new RegExp(`${LABEL}UltraShort`))).toBeNull();
    expect(screen.queryByText(new RegExp(`${LABEL}Long`))).toBeNull();
    expect(document.body.textContent).not.toContain("stockAnalysis.reflection.horizonNoEntry");
  });

  it("旧记录（无 horizonResults）回退到 legacy 三段文本，不渲染四周期 Tab", () => {
    renderRow(null);

    const body = document.body.textContent ?? "";
    expect(body).toContain("stockAnalysis.reflection.causeLabel");
    expect(body).toContain("stockAnalysis.reflection.signalsLabel");
    expect(body).toContain("stockAnalysis.reflection.improveLabel");
    expect(document.querySelector(".ant-tabs-nav")).toBeNull();
    expect(body).not.toContain("stockAnalysis.reflection.horizonNoEntry");
  });
});
