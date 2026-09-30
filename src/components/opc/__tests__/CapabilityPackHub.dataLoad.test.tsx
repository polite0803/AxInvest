// SPDX-License-Identifier: AGPL-3.0-only

/**
 * CapabilityPackHub 的取数去重约束
 *
 * 原实现把 `useDomainData` 放在每个 `DomainTabContent` 内部，而 antd Tabs 会保留访问过的
 * 面板（`destroyOnHidden={false}`）⇒ 每多访问一个 tab 就多一份实例、多一轮域级 IPC。
 * 本测试把「域级数据只有一份实例」钉成不变式：挂载与切 tab 之后，
 * 五条域级命令各自**恰好一次**（`opc_get_capability_pack_dashboard` 亦不得因初始化与
 * KPI 区间两个 effect 各发一次而变成两次）。
 */

import { invoke } from "@/lib/invoke";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App } from "antd";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DomainConfig } from "@/pages/opc/domains/types";
import { CapabilityPackHub } from "../CapabilityPackHub";

vi.mock(import("@/lib/invoke"), async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, invoke: vi.fn() };
});

// 注意：**不 mock 业务阶段内容** —— 若把 `useDomainData` 塞回 `DomainTabContent`，
// 切 tab 会真的再发一轮域级取数，本测试才能抓住这次回归。mock 掉就抓不住了。

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

const CONFIG: DomainConfig = {
  tabs: [
    { key: "stage_a", label: "阶段A", actions: [], workflows: [] },
    { key: "stage_b", label: "阶段B", actions: [], workflows: [] },
  ],
};

/** 域级命令 → 期望调用次数 */
const DOMAIN_COMMANDS = [
  "opc_get_capability_pack_dashboard",
  "opc_get_capability_pack_workflow_steps",
  "opc_get_capability_pack_automation_rules",
  "opc_get_learning_metrics",
  "opc_get_learning_config",
] as const;

function countCalls(cmd: string): number {
  return vi.mocked(invoke).mock.calls.filter(([c]) => c === cmd).length;
}

function renderHub() {
  return render(
    <MemoryRouter>
      <App>
        <CapabilityPackHub
          capabilityPackId="demo-pack"
          config={CONFIG}
          domainTitle="演示能力包"
        />
      </App>
    </MemoryRouter>,
  );
}

describe("CapabilityPackHub 域级取数", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue({} as never);
  });

  it("挂载后五条域级命令各只调用一次（不含重复初始化）", async () => {
    renderHub();

    await waitFor(() => expect(countCalls("opc_get_learning_config")).toBe(1));
    for (const cmd of DOMAIN_COMMANDS) {
      expect(countCalls(cmd), `${cmd} 调用次数`).toBe(1);
    }
  });

  it("切换到另一个业务 tab 不产生新的域级取数", async () => {
    renderHub();
    await waitFor(() => expect(countCalls("opc_get_learning_config")).toBe(1));
    const before = DOMAIN_COMMANDS.map((cmd) => countCalls(cmd));

    fireEvent.click(screen.getByText("阶段B"));
    await waitFor(() => expect(screen.getByText("阶段B")).toBeTruthy());

    DOMAIN_COMMANDS.forEach((cmd, i) => {
      expect(countCalls(cmd), `${cmd} 切 tab 后调用次数`).toBe(before[i]);
    });
  });

  it("业务阶段 tab 之外还提供域级面板 tab", async () => {
    renderHub();

    expect(screen.getByText("opc.domain.tab.runtime")).toBeTruthy();

    fireEvent.click(screen.getByText("opc.domain.tab.runtime"));
    // 默认不自动跑分析：只有点了「执行分析」才发 opc_execute_analysis
    await waitFor(() => expect(countCalls("opc_execute_analysis")).toBe(0));
  });
});
