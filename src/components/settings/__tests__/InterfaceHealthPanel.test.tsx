// SPDX-License-Identifier: AGPL-3.0-only
//
// 呈现层门（2026-10-28）：日级接口健康的三种缺席必须各占一句。
//
//   · `storeEnabled=false` ⇒ 归档没启用、结果没留存（不是「检查过且全好」）
//   · `days=[]`           ⇒ 还没跑过（每个交易日收盘后自动检查）
//   · 载荷形状不认识       ⇒ 本环境不提供（浏览器 mock），不是「没有记录」
//
// 有记录时：正常日显示「正常」，异常日逐条列出 `方法 · 状态`，且故障/改版/时段性空
// 三种文案互不包含。
import i18n from "@/i18n";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { InterfaceHealthPanel } from "../InterfaceHealthPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

function renderPanel() {
  return render(
    <I18nextProvider i18n={i18n}>
      <InterfaceHealthPanel />
    </I18nextProvider>,
  );
}

const row = (method: string, status: string, detail?: string) => ({
  method,
  status,
  detail,
  elapsedMs: 12,
  rows: status === "ok" ? 3 : 0,
});

beforeEach(() => {
  invokeMock.mockReset();
});

describe("InterfaceHealthPanel 三种缺席 + 有记录", () => {
  it("归档未启用 ⇒ 明确说「没留存」", async () => {
    invokeMock.mockResolvedValue({ storeEnabled: false, days: [] });
    renderPanel();
    await waitFor(() => expect(screen.getByText(/每日归档未启用/)).toBeTruthy());
    expect(screen.queryByText(/尚无日检记录/)).toBeNull();
  });

  it("归档已启用但无记录 ⇒ 明确说「还没跑过」", async () => {
    invokeMock.mockResolvedValue({ storeEnabled: true, days: [] });
    renderPanel();
    await waitFor(() => expect(screen.getByText(/尚无日检记录/)).toBeTruthy());
    expect(screen.queryByText(/每日归档未启用/)).toBeNull();
  });

  it("载荷形状不认识（浏览器 mock）⇒ 本环境不提供", async () => {
    invokeMock.mockResolvedValue({});
    renderPanel();
    await waitFor(() => expect(screen.getByText(/本环境不提供日级健康记录/)).toBeTruthy());
  });

  it("正常日只显示「正常」，不列出异常项", async () => {
    invokeMock.mockResolvedValue({
      storeEnabled: true,
      days: [
        {
          checkedAt: "2026-10-27T16:05:00+08:00",
          anchorDate: "2026-10-27",
          probeCode: "600519",
          rows: [row("get_quote", "ok"), row("get_index_quotes", "ok")],
        },
      ],
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText("2026-10-27")).toBeTruthy());
    expect(screen.getByText(/2\/2 正常/)).toBeTruthy();
    expect(screen.queryByText(/数据源不可用/)).toBeNull();
  });

  it("异常日逐条点名方法与状态（故障与改版文案互斥）", async () => {
    invokeMock.mockResolvedValue({
      storeEnabled: true,
      days: [
        {
          checkedAt: "2026-10-27T16:05:00+08:00",
          anchorDate: "2026-10-27",
          probeCode: "600519",
          rows: [
            row("get_quote", "ok"),
            row("get_index_quotes", "source_failed", "eastmoney_push2=source_failed"),
            row("get_financials", "schema_changed", "缺 f57"),
            row("search_news", "empty_by_time"),
          ],
        },
      ],
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText(/get_index_quotes · 数据源不可用/)).toBeTruthy());
    expect(screen.getByText(/get_financials · 接口结构变更/)).toBeTruthy();
    expect(screen.getByText(/search_news · 时段性无数据/)).toBeTruthy();
    expect(screen.getByText(/1\/4 正常/)).toBeTruthy();
  });
});
