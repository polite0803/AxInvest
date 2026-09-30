// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 域级面板的「缺席」呈现约束
 *
 * 钉死两条不变式：
 * 1. **取数失败不得塌成空态** —— `error` 有值时渲染错误原文，不得再渲染 `Empty`。
 *    「拿不到」与「后端答了说没有」是两条不同结论，前者塌成后者就是把结构缺口伪装成正常结果；
 * 2. **步骤序号只认后端真实下发的 `stepOrder`** —— 原组件读幽灵字段 `step_order`，
 *    渲染出「步骤 undefined」；本测试同时守住「不出现 undefined」这一面。
 */

import { render, screen } from "@testing-library/react";
import { App } from "antd";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

import {
  DomainAutomationRules,
  DomainDashboard,
  DomainLearningMetrics,
  DomainLearningPanel,
  DomainWorkflowSteps,
} from "../DomainComponents";

// key 原样回显：断言直接钉在组件引用的 i18n key 上，避免再维护一份文案副本。
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
  initReactI18next: { type: "3rdParty", init: () => {} },
}));

function renderInApp(node: ReactNode) {
  return render(<App>{node}</App>);
}

const noopAsync = async () => {};

describe("域级面板：取数失败 vs 真·空", () => {
  it("工作流步骤：失败时显示原文，且不冒充「暂无」", () => {
    renderInApp(<DomainWorkflowSteps steps={[]} loading={false} error="后端连接被重置" />);

    expect(screen.getByText("后端连接被重置")).toBeTruthy();
    expect(screen.queryByText("opc.domain.workflowSteps.noData")).toBeNull();
  });

  it("工作流步骤：真·空列表才是空态", () => {
    renderInApp(<DomainWorkflowSteps steps={[]} loading={false} />);

    expect(screen.getByText("opc.domain.workflowSteps.noData")).toBeTruthy();
  });

  it("工作流步骤：序号取 stepOrder，不出现 undefined", () => {
    renderInApp(
      <DomainWorkflowSteps
        steps={[{ id: "s1", name: "资料收集", description: "desc", stepOrder: 3 }]}
        loading={false}
      />,
    );

    expect(screen.getByText(/opc\.domain\.workflowSteps\.step\s*3/)).toBeTruthy();
    expect(screen.queryByText(/undefined/)).toBeNull();
  });

  it("仪表盘：失败时不冒充「暂无数据」", () => {
    renderInApp(
      <DomainDashboard
        dashboard={null}
        loading={false}
        error="dashboard 取数失败"
        kpiTimeRange="30"
        onTimeRangeChange={() => {}}
        onRefresh={() => {}}
      />,
    );

    expect(screen.getByText("dashboard 取数失败")).toBeTruthy();
    expect(screen.queryByText("opc.domain.dashboard.noData")).toBeNull();
  });

  it("自动化规则：失败时不冒充「暂无规则」", () => {
    renderInApp(
      <DomainAutomationRules
        rules={[]}
        loading={false}
        error="rules 取数失败"
        running={false}
        onRunAll={async () => []}
      />,
    );

    expect(screen.getByText("rules 取数失败")).toBeTruthy();
    expect(screen.queryByText("opc.domain.rules.noData")).toBeNull();
  });

  it("学习指标：失败时不冒充「暂无指标」", () => {
    renderInApp(
      <DomainLearningMetrics
        metrics={null}
        loading={false}
        error="metrics 取数失败"
        onRefresh={noopAsync}
      />,
    );

    expect(screen.getByText("metrics 取数失败")).toBeTruthy();
    expect(screen.queryByText("opc.domain.metrics.noData")).toBeNull();
  });

  it("学习面板：配置取数失败时不冒充「配置未找到」", () => {
    renderInApp(
      <DomainLearningPanel
        learningConfig={null}
        loading={false}
        error="config 取数失败"
        onReflect={noopAsync}
        onEvolve={noopAsync}
        onSelfImprove={noopAsync}
      />,
    );

    expect(screen.getByText("config 取数失败")).toBeTruthy();
    expect(screen.queryByText("opc.domain.learning.actions.configNotFound")).toBeNull();
  });

  it("学习面板：确实没有配置（无错）时才报「配置未找到」", () => {
    renderInApp(
      <DomainLearningPanel
        learningConfig={null}
        loading={false}
        onReflect={noopAsync}
        onEvolve={noopAsync}
        onSelfImprove={noopAsync}
      />,
    );

    expect(screen.getByText("opc.domain.learning.actions.configNotFound")).toBeTruthy();
  });
});
