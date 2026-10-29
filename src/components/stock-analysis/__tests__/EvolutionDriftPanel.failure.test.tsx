// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 进化漂移面板失败态的**可诊断性**约束。
 *
 * 背景（2026-10-29 实证）：`get_evolution_drift_dashboard` 曾在 IPC **序列化阶段**
 * 失败 —— `current_weights` 是 `HashMap<(String, String), f64>`，元组键让 `serde_json`
 * 直接报 `key must be a string`。这类失败在后端 tracing 里一行都不留；前端若再把原因
 * 吞掉（文案只有「加载失败」且没有 `{{msg}}` 占位符），故障就只剩一句无信息量的红字。
 *
 * 本文件钉死一条不变式：**错误详情必须出现在界面上**。
 */

import { useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { render, screen } from "@testing-library/react";
import { App } from "antd";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { EvolutionDriftPanel } from "../EvolutionDriftPanel";

// invoke 永不 settle：本用例只验证「已写入的失败态怎么渲染」，
// 不让面板挂载时的取数 effect 覆盖掉注入值（成功路径会清空 lastError）。
vi.mock("@/lib/invoke", () => ({
  invoke: vi.fn(() => new Promise(() => {})),
  listen: vi.fn(() => Promise.resolve(() => {})),
  isTauri: () => false,
}));

// 审批抽屉只在拿到 dashboard 之后才渲染；这里直接替掉，
// 避免它把图表一类的重依赖拖进失败态用例。
vi.mock("../EvolutionApprovalDrawer", () => ({ EvolutionApprovalDrawer: () => null }));

const ZH: Record<string, string> = {
  "stockAnalysis.evolutionDrift.loading": "加载中",
  "stockAnalysis.evolutionDrift.error": "加载失败：{{msg}}",
};

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => {
      const template = ZH[key] ?? key;
      return opts && typeof opts.msg === "string"
        ? template.replace("{{msg}}", opts.msg)
        : template;
    },
  }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

function renderPanel() {
  return render(
    <App>
      <EvolutionDriftPanel />
    </App>,
  );
}

describe("EvolutionDriftPanel 失败态", () => {
  beforeEach(() => {
    useStockAnalysisStore.setState({
      evolutionDashboard: null,
      evolutionLastError: null,
      evolutionRecalculating: false,
    });
  });

  it("把后端错误详情露在界面上（不得只剩「加载失败」四个字）", () => {
    useStockAnalysisStore.setState({
      evolutionLastError: "加载失败 — key must be a string",
    });

    renderPanel();

    expect(screen.getByText(/key must be a string/)).toBeTruthy();
  });

  it("负控：没有错误时不渲染失败红字（不得凭空造失败）", () => {
    renderPanel();

    expect(screen.queryByText(/加载失败/)).toBeNull();
  });
});
