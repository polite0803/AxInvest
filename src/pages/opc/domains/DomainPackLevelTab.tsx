// i18n-exempt: 组合组件，全部文案在被组合的组件内部
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * 能力包级面板 Tab —— 承载「属于整个能力包」而非某个业务阶段的数据。
 *
 * 为什么要单独成 tab：这些面板读的是 `useDomainData` 的**域级字段**
 * （仪表盘 / 分析决策 / 流程步骤 / 自动化规则 / 学习指标 / 学习配置），而各能力包的
 * 业务阶段 tab key 互不相同（`research` / `design` / `analysis` / `lead` …），
 * 没有任何一个 tab 是它们的共同落点。挂进任一业务阶段 tab 都会让「这个面板为什么
 * 出现在这一步」无从回答。
 *
 * 分析决策是**手动触发**：`loadDecision` 不在 `useDomainData` 的初始化 effect 里，
 * 只有点「执行分析」才发起 —— 该命令会真跑一轮分析，进页面即自动跑属越权开销。
 */

import {
  DomainAnalysisDecision,
  DomainAutomationRules,
  DomainDashboard,
  DomainLearningMetrics,
  DomainLearningPanel,
  DomainWorkflowSteps,
} from "./DomainComponents";
import type { UseDomainDataReturn } from "./useDomainData";

export function DomainPackLevelTab({ data }: { data: UseDomainDataReturn }) {
  return (
    <div style={{ padding: "16px 24px", height: "100%", overflow: "auto" }}>
      <DomainDashboard
        dashboard={data.dashboard}
        loading={data.dashboardLoading}
        error={data.errors.dashboard}
        kpiTimeRange={data.kpiTimeRange}
        onTimeRangeChange={data.setKpiTimeRange}
        onRefresh={data.loadDashboard}
      />
      <DomainAnalysisDecision
        decision={data.decision}
        loading={data.decisionLoading}
        error={data.errors.decision}
        decisionDays={data.decisionDays}
        onDaysChange={data.setDecisionDays}
        onExecute={data.loadDecision}
      />
      <DomainWorkflowSteps
        steps={data.workflowSteps}
        loading={data.stepsLoading}
        error={data.errors.steps}
      />
      <DomainAutomationRules
        rules={data.automationRules}
        loading={data.rulesLoading}
        error={data.errors.rules}
        running={data.rulesRunning}
        onRunAll={data.runAutomationRules}
      />
      <DomainLearningMetrics
        metrics={data.learningMetrics}
        loading={data.metricsLoading}
        error={data.errors.metrics}
        onRefresh={data.loadLearningMetrics}
      />
      <DomainLearningPanel
        learningConfig={data.learningConfig}
        loading={data.learningLoading}
        error={data.errors.learningConfig}
        onReflect={data.reflectOnWorkflow}
        onEvolve={data.evolveWorkflow}
        onSelfImprove={data.runSelfImprovement}
      />
    </div>
  );
}
