// SPDX-License-Identifier: AGPL-3.0-only
//
// 呈现层门（2026-10-28）：`get_index_quotes` 的四态必须在 UI 上**各占一句**。
//
// 立门理由：改前后端在三条通道全挂时也返回裸 `[]`，面板一律显示「暂无指数数据」——
// 「今天没行情」与「取数通道故障」在屏幕上长得一模一样。这里锁住：
//   ok → 出表；empty_by_time → 暂无数据；source_failed → 数据源不可用；schema_changed → 接口结构变更。
// 每个破坏样本只让对应那一条红（四种文案互不包含）。
import i18n from "@/i18n";
import { render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { IndexQuotesPanel } from "../IndexQuotesPanel";

const invokeMock = vi.fn();
vi.mock("@/lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: vi.fn().mockResolvedValue(() => {}),
  isTauri: () => false,
}));

// PanelEmpty 只读 mode（live 时 noData 不升级成 replayDegraded），按最小面替身。
const anchor = { mode: "live", asOfDate: null as string | null };
vi.mock("@/stores/feature/timeAnchorStore", () => ({
  useTimeAnchorStore: (selector: (s: { mode: string; asOfDate: string | null }) => unknown) => selector(anchor),
}));

const QUOTE = {
  code: "sh000001",
  name: "上证指数",
  price: 3888.11,
  preClose: 3860,
  changePct: 0.73,
  volume: 1,
  amount: 1,
};

function renderPanel() {
  return render(
    <I18nextProvider i18n={i18n}>
      <IndexQuotesPanel />
    </I18nextProvider>,
  );
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("IndexQuotesPanel 四态呈现", () => {
  it("status=ok 时出表", async () => {
    invokeMock.mockResolvedValue({ status: "ok", data: [QUOTE] });
    renderPanel();
    await waitFor(() => expect(screen.getByText("上证指数")).toBeTruthy());
    expect(screen.queryByText("暂无指数数据")).toBeNull();
  });

  it("empty_by_time 显示「暂无指数数据」（不是故障文案）", async () => {
    invokeMock.mockResolvedValue({
      status: "empty_by_time",
      data: [],
      detail: "eastmoney_push2=empty_by_time",
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText(/暂无指数数据/)).toBeTruthy());
    expect(screen.queryByText(/数据源不可用/)).toBeNull();
    expect(screen.queryByText(/接口结构变更/)).toBeNull();
  });

  it("source_failed 显示「数据源不可用」并带逐源证据", async () => {
    invokeMock.mockResolvedValue({
      status: "source_failed",
      data: [],
      detail: "eastmoney_push2=source_failed(HTTP 429)",
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText(/数据源不可用.*HTTP 429/s)).toBeTruthy());
    expect(screen.queryByText(/暂无指数数据/)).toBeNull();
    expect(screen.queryByText(/接口结构变更/)).toBeNull();
  });

  it("schema_changed 显示「接口结构变更」", async () => {
    invokeMock.mockResolvedValue({
      status: "schema_changed",
      data: [],
      detail: "缺 f57(code)",
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText(/接口结构变更.*缺 f57/s)).toBeTruthy());
    expect(screen.queryByText(/暂无指数数据/)).toBeNull();
    expect(screen.queryByText(/数据源不可用/)).toBeNull();
  });

  it("历史裸数组载荷仍可渲染（向后兼容）", async () => {
    invokeMock.mockResolvedValue([QUOTE]);
    renderPanel();
    await waitFor(() => expect(screen.getByText("上证指数")).toBeTruthy());
  });

  it("命令抛错显示「连接失败」，不退化成「暂无数据」", async () => {
    invokeMock.mockRejectedValue(new Error("ipc down"));
    renderPanel();
    await waitFor(() => expect(screen.getByText(/连接失败/)).toBeTruthy());
    expect(screen.queryByText(/暂无指数数据/)).toBeNull();
  });
});
